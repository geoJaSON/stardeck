use rusqlite::Connection;
use std::path::PathBuf;

/// A single note. Schema is sync-ready: `updated_at` drives last-write-wins
/// reconciliation against Postgres, and soft deletes leave a tombstone so a
/// delete on one machine isn't resurrected by a stale copy on another.
#[derive(Clone, Debug, Default)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    /// Slash-delimited path, e.g. "work/meetings". Empty = root.
    pub folder: String,
    /// Comma-separated tags, normalized lowercase on save.
    pub tags: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Tombstone marker. Written/queried in SQL for sync-readiness; not yet
    /// read from Rust until Postgres sync lands.
    #[allow(dead_code)]
    pub deleted_at: Option<i64>,
}

pub struct Db {
    conn: Connection,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Db {
    pub fn open() -> rusqlite::Result<Db> {
        let path = data_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notes (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                body        TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL,
                deleted_at  INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at);",
        )?;
        let db = Db { conn };
        db.ensure_column("folder")?;
        db.ensure_column("tags")?;
        Ok(db)
    }

    /// Add a `TEXT NOT NULL DEFAULT ''` column if it isn't there yet.
    /// SQLite has no `ADD COLUMN IF NOT EXISTS`, so probe first.
    fn ensure_column(&self, name: &str) -> rusqlite::Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(notes)")?;
        let exists = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(|c| c.ok())
            .any(|c| c == name);
        if !exists {
            self.conn.execute_batch(&format!(
                "ALTER TABLE notes ADD COLUMN {name} TEXT NOT NULL DEFAULT ''"
            ))?;
        }
        Ok(())
    }

    /// Live notes (tombstones excluded). `filter` is a case-insensitive
    /// substring match over title, body, tags and folder.
    pub fn list(&self, filter: &str) -> rusqlite::Result<Vec<Note>> {
        let like = format!("%{}%", filter);
        let mut stmt = self.conn.prepare(
            "SELECT id, title, body, folder, tags, created_at, updated_at, deleted_at
             FROM notes
             WHERE deleted_at IS NULL
               AND (?1 = ''
                    OR title  LIKE ?2
                    OR body   LIKE ?2
                    OR tags   LIKE ?2
                    OR folder LIKE ?2)
             ORDER BY folder COLLATE NOCASE, updated_at DESC",
        )?;
        let rows = stmt.query_map((filter, &like), |r| {
            Ok(Note {
                id: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                folder: r.get(3)?,
                tags: r.get(4)?,
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
                deleted_at: r.get(7)?,
            })
        })?;
        rows.collect()
    }

    pub fn create(&self) -> rusqlite::Result<Note> {
        let note = Note {
            id: uuid::Uuid::new_v4().to_string(),
            title: "untitled".to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            ..Default::default()
        };
        self.conn.execute(
            "INSERT INTO notes (id, title, body, folder, tags, created_at, updated_at, deleted_at)
             VALUES (?1, ?2, '', '', '', ?3, ?4, NULL)",
            (&note.id, &note.title, note.created_at, note.updated_at),
        )?;
        Ok(note)
    }

    /// Find today's journal note by (title, folder), creating it if absent.
    pub fn daily(&self, title: &str, folder: &str) -> rusqlite::Result<Note> {
        let existing = self.conn.query_row(
            "SELECT id, title, body, folder, tags, created_at, updated_at, deleted_at
             FROM notes
             WHERE title = ?1 AND folder = ?2 AND deleted_at IS NULL
             LIMIT 1",
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
                    deleted_at: r.get(7)?,
                })
            },
        );
        if let Ok(note) = existing {
            return Ok(note);
        }
        let note = Note {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            folder: folder.to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            ..Default::default()
        };
        self.conn.execute(
            "INSERT INTO notes (id, title, body, folder, tags, created_at, updated_at, deleted_at)
             VALUES (?1, ?2, '', ?3, '', ?4, ?5, NULL)",
            (
                &note.id,
                &note.title,
                &note.folder,
                note.created_at,
                note.updated_at,
            ),
        )?;
        Ok(note)
    }

    pub fn save(&self, note: &Note) -> rusqlite::Result<i64> {
        let ts = now_ms();
        self.conn.execute(
            "UPDATE notes
             SET title = ?1, body = ?2, folder = ?3, tags = ?4, updated_at = ?5
             WHERE id = ?6",
            (
                &note.title,
                &note.body,
                &note.folder,
                &note.tags,
                ts,
                &note.id,
            ),
        )?;
        Ok(ts)
    }

    /// Soft delete: leaves a tombstone row so the deletion can propagate during
    /// sync instead of the note reappearing from another device.
    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE notes SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
            (now_ms(), id),
        )?;
        Ok(())
    }
}

fn data_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("stardeck");
    p.push("notes.db");
    p
}
