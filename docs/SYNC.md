# Storage & sync design

## Decision

Notes are **plain markdown files in a workspace folder**. That folder *is* the
sync mechanism: the user points their existing tool (Dropbox / iCloud /
Syncthing / git) at it, and that tool moves bytes between machines. No
accounts, no login, no server, and **no merge code of our own** — per-file
last-writer-wins and conflicted-copy handling are exactly what those tools
already do.

SQLite is kept, but **demoted to a disposable, local-only search index**. It
is rebuilt from the files, never synced, and never the source of truth. It can
be deleted at any time and regenerated. The earlier plan (DB as the store +
JSONL snapshots + hand-written LWW merge) is dropped: it duplicated, worse,
what the file-sync tool gives for free, and locked data in a DB file instead
of portable markdown. The multi-database idea is likewise dropped — the
database was never the hard part of sync.

## On-disk format

- One note = one `.md` file under `<workspace>/`.
- `folder` is the real subdirectory path (`work/meetings/note.md`); root notes
  sit at the workspace root.
- Filename is a slug of the title (Obsidian-style, editable in any tool). A
  stable `id` lives in YAML frontmatter, so a title change is a file rename
  without losing identity, and a half-synced rename can't duplicate a note.
- Frontmatter carries metadata that the filesystem can't be trusted to
  preserve across sync tools (mtime is unreliable on Dropbox/git):

  ```
  ---
  id: <uuid>
  created: <epoch-ms>
  updated: <epoch-ms>
  tags: a, b, c
  ---
  <markdown body>
  ```

- A file hand-created in the workspace without frontmatter is adopted on
  index: an `id`/timestamps block is written back, body preserved.

## The index

Local SQLite at the app data dir (NOT in the workspace, so it never syncs).
On startup the workspace is scanned and the index rebuilt: upsert by `id`,
drop rows whose file vanished. Saves are write-through — the `.md` file is
written atomically (temp + rename) and the index row updated in the same call.
Search/filter and ordering run against the index, so the query path is
unchanged from before; only its source of truth moved to the files.

## Conflicts

Delegated entirely to the user's sync tool. Editing the same note on two
offline devices yields the tool's normal outcome (a `... (conflicted copy).md`
for Dropbox/Syncthing, or a git merge conflict). We surface both copies as
notes rather than silently losing one — which is automatic, since each copy is
just another file the indexer picks up.

## Migration

First run after this change: if a legacy `notes.db` exists, every live note is
exported to `.md` files in the workspace, then the old DB is renamed aside
(`notes.db.pre-files`) rather than deleted — reversible if anything looks
wrong. Tombstones are not exported (a deleted note is simply an absent file).
