use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Markdown files in the workspace are the source of truth; this is a thin
/// error alias so file I/O and SQLite errors share one `?`-able result.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A single note. Lives on disk as one `.md` file with a YAML frontmatter
/// header; `deleted_at` is vestigial (a deleted note is just an absent file)
/// but kept so call sites that build `Note { .. ..Default }` are untouched.
#[derive(Clone, Debug, Default)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    /// Slash-delimited path = real subdirectory under the workspace. Empty = root.
    pub folder: String,
    /// Comma-separated tags, normalized lowercase on save.
    pub tags: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[allow(dead_code)]
    pub deleted_at: Option<i64>,
}

pub struct Db {
    /// Local-only, rebuildable search index. Never synced, never canonical.
    conn: Connection,
    workspace: PathBuf,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Db {
    /// Open the workspace, run a one-time migration off any legacy `notes.db`,
    /// and (re)build the search index from the files on disk.
    pub fn open(workspace: &Path) -> Result<Db> {
        std::fs::create_dir_all(workspace)?;
        let conn = Connection::open(index_path(workspace))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notes (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                body        TEXT NOT NULL,
                folder      TEXT NOT NULL,
                tags        TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL,
                path        TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at);

            -- Ranked full-text search. External-content table mirroring `notes`,
            -- kept in sync by triggers so it survives upserts, deletes and the
            -- full reindex on every open without per-call bookkeeping.
            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                title, body, tags, folder,
                content='notes', content_rowid='rowid'
            );
            CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON notes BEGIN
                INSERT INTO notes_fts(rowid, title, body, tags, folder)
                VALUES (new.rowid, new.title, new.body, new.tags, new.folder);
            END;
            CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON notes BEGIN
                INSERT INTO notes_fts(notes_fts, rowid, title, body, tags, folder)
                VALUES ('delete', old.rowid, old.title, old.body, old.tags, old.folder);
            END;
            CREATE TRIGGER IF NOT EXISTS notes_au AFTER UPDATE ON notes BEGIN
                INSERT INTO notes_fts(notes_fts, rowid, title, body, tags, folder)
                VALUES ('delete', old.rowid, old.title, old.body, old.tags, old.folder);
                INSERT INTO notes_fts(rowid, title, body, tags, folder)
                VALUES (new.rowid, new.title, new.body, new.tags, new.folder);
            END;",
        )?;
        let db = Db {
            conn,
            workspace: workspace.to_path_buf(),
        };
        db.migrate_legacy_db();
        db.reindex()?;
        Ok(db)
    }

    /// Export a pre-files `notes.db` to markdown once, then move it aside so
    /// this never runs again. Best-effort: a bad row must not block startup.
    fn migrate_legacy_db(&self) {
        let legacy = stardeck_dir().join("notes.db");
        if !legacy.exists() {
            return;
        }
        if let Ok(old) = Connection::open(&legacy) {
            if let Ok(mut stmt) = old.prepare(
                "SELECT id, title, body, folder, tags, created_at, updated_at
                 FROM notes WHERE deleted_at IS NULL",
            ) {
                let rows = stmt.query_map([], |r| {
                    Ok(Note {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        body: r.get(2)?,
                        folder: r.get(3)?,
                        tags: r.get(4)?,
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                        deleted_at: None,
                    })
                });
                if let Ok(rows) = rows {
                    for note in rows.flatten() {
                        let _ = self.write_file(&note, None);
                    }
                }
            }
        }
        let _ = std::fs::rename(&legacy, stardeck_dir().join("notes.db.pre-files"));
    }

    /// Rebuild the index from scratch so files deleted or renamed outside the
    /// app (or by the sync tool) are reflected. Cheap at personal note counts.
    fn reindex(&self) -> Result<()> {
        self.conn.execute("DELETE FROM notes", [])?;
        let mut files = Vec::new();
        collect_md(&self.workspace, &mut files);
        for path in files {
            match self.read_file(&path) {
                Ok((note, adopted)) => {
                    if adopted {
                        // File had no id/frontmatter — write it back once so it
                        // gains a stable identity. Body is preserved.
                        let _ = self.write_file(&note, Some(&path));
                    }
                    self.index_upsert(&note, &path)?;
                }
                Err(_) => continue, // skip unreadable / non-note files
            }
        }
        Ok(())
    }

    fn index_upsert(&self, n: &Note, path: &Path) -> Result<()> {
        self.conn.execute(
            "INSERT INTO notes
               (id, title, body, folder, tags, created_at, updated_at, path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               title=excluded.title, body=excluded.body, folder=excluded.folder,
               tags=excluded.tags, created_at=excluded.created_at,
               updated_at=excluded.updated_at, path=excluded.path",
            (
                &n.id,
                &n.title,
                &n.body,
                &n.folder,
                &n.tags,
                n.created_at,
                n.updated_at,
                path.to_string_lossy().to_string(),
            ),
        )?;
        Ok(())
    }

    /// Notes matching `filter`. Empty filter returns everything grouped by
    /// folder (the tree view depends on that order). A non-empty filter runs a
    /// ranked FTS5 prefix query (best match first, then most-recent), so
    /// search-as-you-type stays useful before a word is finished.
    pub fn list(&self, filter: &str) -> Result<Vec<Note>> {
        let row = |r: &rusqlite::Row| {
            Ok(Note {
                id: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                folder: r.get(3)?,
                tags: r.get(4)?,
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
                deleted_at: None,
            })
        };
        let cols = "n.id, n.title, n.body, n.folder, n.tags, n.created_at, n.updated_at";

        match fts_query(filter) {
            None => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT {cols} FROM notes n
                     ORDER BY n.folder COLLATE NOCASE, n.updated_at DESC"
                ))?;
                let rows = stmt.query_map([], row)?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            }
            Some(q) => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT {cols} FROM notes n
                     JOIN notes_fts f ON f.rowid = n.rowid
                     WHERE notes_fts MATCH ?1
                     ORDER BY bm25(notes_fts), n.updated_at DESC"
                ))?;
                let rows = stmt.query_map((q,), row)?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            }
        }
    }

    pub fn create(&self) -> Result<Note> {
        let ts = now_ms();
        let note = Note {
            id: uuid::Uuid::new_v4().to_string(),
            title: "untitled".to_string(),
            created_at: ts,
            updated_at: ts,
            ..Default::default()
        };
        let path = self.write_file(&note, None)?;
        self.index_upsert(&note, &path)?;
        Ok(note)
    }

    /// Today's journal note by (title, folder), created if absent.
    pub fn daily(&self, title: &str, folder: &str) -> Result<Note> {
        let existing: Option<Note> = self
            .conn
            .query_row(
                "SELECT id, title, body, folder, tags, created_at, updated_at
                 FROM notes WHERE title = ?1 AND folder = ?2 LIMIT 1",
                (title, folder),
                |r| {
                    Ok(Note {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        body: r.get(2)?,
                        folder: r.get(3)?,
                        tags: r.get(4)?,
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                        deleted_at: None,
                    })
                },
            )
            .ok();
        if let Some(note) = existing {
            return Ok(note);
        }
        let ts = now_ms();
        let note = Note {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            folder: folder.to_string(),
            created_at: ts,
            updated_at: ts,
            ..Default::default()
        };
        let path = self.write_file(&note, None)?;
        self.index_upsert(&note, &path)?;
        Ok(note)
    }

    /// Read-only lookup by exact title (case-insensitive). Used to resolve
    /// `[[wiki links]]` without creating anything.
    pub fn by_title(&self, title: &str) -> Option<Note> {
        self.conn
            .query_row(
                "SELECT id, title, body, folder, tags, created_at, updated_at
                 FROM notes WHERE title = ?1 COLLATE NOCASE LIMIT 1",
                (title,),
                |r| {
                    Ok(Note {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        body: r.get(2)?,
                        folder: r.get(3)?,
                        tags: r.get(4)?,
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                        deleted_at: None,
                    })
                },
            )
            .ok()
    }

    /// Write-through save: rewrite the `.md` file (renaming it if the title or
    /// folder changed) and update the index row. If the title changed, every
    /// `[[old title]]` in other notes is rewritten to `[[new title]]` so the
    /// rename doesn't silently break inbound links. Returns new `updated_at`.
    pub fn save(&self, note: &Note) -> Result<i64> {
        let ts = now_ms();
        let prior: Option<(String, String, i64)> = self
            .conn
            .query_row(
                "SELECT path, title, created_at FROM notes WHERE id = ?1",
                (&note.id,),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let old_path = prior.as_ref().map(|p| p.0.clone());
        let old_title = prior.as_ref().map(|p| p.1.clone());
        let created_at = prior.as_ref().map(|p| p.2).unwrap_or(ts);

        let saved = Note {
            created_at,
            updated_at: ts,
            ..note.clone()
        };
        let old = old_path.as_deref().map(Path::new);
        let new_path = self.write_file(&saved, old)?;
        if let Some(old) = old {
            if old != new_path {
                let _ = std::fs::remove_file(old);
            }
        }
        self.index_upsert(&saved, &new_path)?;

        if let Some(old_title) = old_title {
            if !old_title.is_empty() && !old_title.eq_ignore_ascii_case(&saved.title) {
                self.rewrite_links(&old_title, &saved.title, &saved.id, ts)?;
            }
        }
        Ok(ts)
    }

    /// Repoint every `[[old]]` token (case-insensitive) to `[[new]]` across all
    /// notes except `skip_id`. Touched notes get a fresh `updated_at` since
    /// their content really did change.
    fn rewrite_links(&self, old: &str, new: &str, skip_id: &str, ts: i64) -> Result<()> {
        let rows: Vec<(String, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, path FROM notes WHERE id != ?1")?;
            let mapped = stmt.query_map((skip_id,), |r| Ok((r.get(0)?, r.get(1)?)))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in rows {
            let path = PathBuf::from(path);
            let Ok((note, _)) = self.read_file(&path) else {
                continue;
            };
            let Some(new_body) = replace_link(&note.body, old, new) else {
                continue;
            };
            let updated = Note {
                body: new_body,
                updated_at: ts,
                ..note
            };
            // Title/folder unchanged here, so the path is stable; reuse it.
            let p = self.write_file(&updated, Some(&path))?;
            self.index_upsert(&updated, &p)?;
            let _ = id; // (kept for clarity; lookup was by path)
        }
        Ok(())
    }

    /// Delete = remove the file (the sync tool propagates the absence) and
    /// drop the index row. No tombstone.
    pub fn delete(&self, id: &str) -> Result<()> {
        if let Ok(path) = self.conn.query_row(
            "SELECT path FROM notes WHERE id = ?1",
            (id,),
            |r| r.get::<_, String>(0),
        ) {
            let _ = std::fs::remove_file(path);
        }
        self.conn.execute("DELETE FROM notes WHERE id = ?1", (id,))?;
        Ok(())
    }

    /// Absolute path a note should live at, given its title/folder. If that
    /// path is already taken by a *different* note, disambiguate with a short
    /// id suffix so two same-titled notes never clobber each other.
    fn target_path(&self, note: &Note) -> PathBuf {
        let mut dir = self.workspace.clone();
        for seg in note.folder.split('/').filter(|s| !s.is_empty()) {
            dir.push(slug(seg));
        }
        let base = slug(&note.title);
        let candidate = dir.join(format!("{base}.md"));
        let owner: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM notes WHERE path = ?1",
                (candidate.to_string_lossy().to_string(),),
                |r| r.get(0),
            )
            .ok();
        match owner {
            Some(other) if other != note.id => {
                dir.join(format!("{base}-{}.md", &short_id(&note.id)))
            }
            _ => candidate,
        }
    }

    /// Serialize a note to `<frontmatter>\n<body>` and write it atomically
    /// (temp + rename). Reuses `at` (the file's current path) if given so an
    /// unchanged title overwrites in place instead of churning the filename.
    fn write_file(&self, note: &Note, at: Option<&Path>) -> Result<PathBuf> {
        let path = match at {
            Some(p) if title_matches_path(&self.workspace, p, note) => p.to_path_buf(),
            _ => self.target_path(note),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::new();
        out.push_str("---\n");
        out.push_str(&format!("id: {}\n", note.id));
        out.push_str(&format!("title: {}\n", note.title));
        out.push_str(&format!("created: {}\n", note.created_at));
        out.push_str(&format!("updated: {}\n", note.updated_at));
        out.push_str(&format!("tags: {}\n", note.tags));
        out.push_str("---\n");
        out.push_str(&note.body);

        let tmp = path.with_extension("md.tmp");
        std::fs::write(&tmp, out.as_bytes())?;
        std::fs::rename(&tmp, &path)?; // atomic; replaces on Windows too
        Ok(path)
    }

    /// Parse a `.md` file into a note. `adopted` is true when the file had no
    /// usable frontmatter id and we minted one (caller should write it back).
    fn read_file(&self, path: &Path) -> Result<(Note, bool)> {
        let raw = std::fs::read_to_string(path)?;
        let raw = raw.replace("\r\n", "\n");
        let (fm, body) = split_frontmatter(&raw);

        let mut id = String::new();
        let mut fm_title = String::new();
        let mut created = 0i64;
        let mut updated = 0i64;
        let mut tags = String::new();
        for line in fm.lines() {
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "id" => id = v.to_string(),
                "title" => fm_title = v.to_string(),
                "created" => created = v.parse().unwrap_or(0),
                "updated" => updated = v.parse().unwrap_or(0),
                "tags" => tags = v.to_string(),
                _ => {}
            }
        }

        let adopted = id.is_empty();
        if adopted {
            id = uuid::Uuid::new_v4().to_string();
        }
        let mtime = file_mtime_ms(path).unwrap_or_else(now_ms);
        if created == 0 {
            created = mtime;
        }
        if updated == 0 {
            updated = mtime;
        }

        let rel = path.strip_prefix(&self.workspace).unwrap_or(path);
        let folder = rel
            .parent()
            .map(|p| {
                p.components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        // Frontmatter title is authoritative (preserves casing/punctuation);
        // fall back to the filename for files made by hand in another editor.
        let title = if !fm_title.is_empty() {
            fm_title
        } else {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "untitled".to_string())
        };

        Ok((
            Note {
                id,
                title,
                body: body.to_string(),
                folder,
                tags,
                created_at: created,
                updated_at: updated,
                deleted_at: None,
            },
            adopted,
        ))
    }
}

/// Split a leading `---\n ... \n---\n` block off the rest. Returns
/// (frontmatter, body); frontmatter is empty when there is no header.
fn split_frontmatter(raw: &str) -> (&str, &str) {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return ("", raw);
    };
    match rest.find("\n---\n") {
        Some(end) => (&rest[..end], &rest[end + 5..]),
        None => ("", raw),
    }
}

/// Does `path`'s filename already match what `note` would slug to? Used to
/// avoid renaming a file on every keystroke when the title is unchanged.
fn title_matches_path(workspace: &Path, path: &Path, note: &Note) -> bool {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let base = slug(&note.title);
    let matches_title = stem == base || stem == format!("{base}-{}", short_id(&note.id));
    let rel = path.strip_prefix(workspace).unwrap_or(path);
    let folder = rel
        .parent()
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default();
    let want_folder = note
        .folder
        .split('/')
        .filter(|s| !s.is_empty())
        .map(slug)
        .collect::<Vec<_>>()
        .join("/");
    matches_title && folder == want_folder
}

/// Turn a raw search box string into an FTS5 prefix query, e.g.
/// `meet not` -> `"meet"* "not"*` (implicit AND, each term a prefix match).
/// Returns `None` when there is nothing searchable, so the caller can fall
/// back to listing everything instead of feeding FTS an empty/invalid MATCH.
fn fts_query(filter: &str) -> Option<String> {
    let terms: Vec<String> = filter
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.to_lowercase()))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// Replace every `[[old]]` (the title compared case-insensitively, brackets
/// literal) with `[[new]]`. Returns `None` when nothing matched so callers can
/// skip rewriting the file.
fn replace_link(body: &str, old: &str, new: &str) -> Option<String> {
    // ASCII-fold so `lower` keeps byte-for-byte the same indices as `body`
    // (Unicode lowercasing can change length and desync the two).
    let needle = format!("[[{}]]", old.to_ascii_lowercase());
    let lower = body.to_ascii_lowercase();
    if !lower.contains(&needle) {
        return None;
    }
    let with = format!("[[{new}]]");
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if lower[i..].starts_with(&needle) {
            out.push_str(&with);
            i += needle.len();
        } else {
            // Walk one char (not one byte) to stay on UTF-8 boundaries.
            let ch = body[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Some(out)
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Filesystem-safe slug: ascii alphanumerics kept, everything else collapses
/// to single dashes. Never empty.
fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}

fn file_mtime_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis() as i64)
}

/// Recursively collect `*.md` files, skipping our own temp files and dotdirs.
fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_md(&path, out);
        } else if name.ends_with(".md") && !name.ends_with(".md.tmp") {
            out.push(path);
        }
    }
}

/// Per-user app data directory (`.../stardeck`). Holds the local search index
/// and the moved-aside legacy DB — never the notes themselves.
pub fn stardeck_dir() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("stardeck");
    p
}

/// Per-workspace index file, living in the app data dir (never inside the
/// workspace, so it is never synced). Keyed by a hash of the workspace path so
/// different workspaces — and parallel tests — don't share one index.
fn index_path(workspace: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let dir = stardeck_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    workspace.to_string_lossy().hash(&mut h);
    dir.join(format!("index-{:016x}.db", h.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_round_trips() {
        let tmp = std::env::temp_dir().join(format!("sd-test-{}", uuid::Uuid::new_v4()));
        let db = Db::open(&tmp).unwrap();
        let mut n = db.create().unwrap();
        n.title = "Hello World".into();
        n.body = "line one\n\nline two".into();
        n.tags = "a,b".into();
        n.folder = "work/notes".into();
        db.save(&n).unwrap();

        // Fresh open must rebuild identical state purely from the files.
        let db2 = Db::open(&tmp).unwrap();
        let got = db2.list("").unwrap();
        let got = got.iter().find(|x| x.id == n.id).expect("note survived");
        assert_eq!(got.title, "Hello World"); // display title preserved in frontmatter
        assert_eq!(got.body, "line one\n\nline two");
        assert_eq!(got.tags, "a,b");
        assert_eq!(got.folder, "work/notes");

        db.delete(&n.id).unwrap();
        assert!(db.list("").unwrap().iter().all(|x| x.id != n.id));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn fts_prefix_and_ranking() {
        let tmp = std::env::temp_dir().join(format!("sd-fts-{}", uuid::Uuid::new_v4()));
        let db = Db::open(&tmp).unwrap();

        let mut a = db.create().unwrap();
        a.title = "Meeting notes".into();
        a.body = "discuss roadmap".into();
        db.save(&a).unwrap();

        let mut b = db.create().unwrap();
        b.title = "Groceries".into();
        b.body = "we should meet about milk".into();
        db.save(&b).unwrap();

        // Prefix: "meet" matches both (title "Meeting", body "meet").
        let hits = db.list("meet").unwrap();
        assert!(hits.iter().any(|n| n.id == a.id));
        assert!(hits.iter().any(|n| n.id == b.id));

        // A title hit should outrank a body-only hit.
        assert_eq!(hits.first().unwrap().id, a.id);

        // Unrelated term excludes non-matches.
        let hits = db.list("roadmap").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, a.id);

        // Deleting drops it from the FTS index too (trigger fired).
        db.delete(&a.id).unwrap();
        assert!(db.list("roadmap").unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
