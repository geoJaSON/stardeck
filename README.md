# STARDECK

A terminal-themed, offline-first markdown note-taker. Notes are plain `.md`
files you own; syncing between machines is just pointing a folder-sync tool
(Dropbox, Syncthing, git, …) at your workspace. No accounts, no server, no
lock-in.

Built in Rust with [egui](https://github.com/emilk/egui)/eframe.

## Why

Most note apps trap your notes in a database or a proprietary cloud. STARDECK
keeps them as ordinary markdown files in a folder. SQLite is used only as a
**disposable, local search index** — rebuilt from the files, never synced,
never the source of truth. Delete it and it regenerates. See
[`docs/SYNC.md`](docs/SYNC.md) for the full rationale and on-disk format.

## Features

- **Plain markdown files** as the source of truth — one `.md` per note, with
  YAML frontmatter (`id`, `title`, `created`, `updated`, `tags`); folders are
  real directories.
- **Sync without accounts** — the workspace folder *is* the sync unit; your
  existing file-sync tool moves the bytes and handles conflicts.
- **Linked notes** — `[[wiki links]]` with a clickable links panel,
  create-on-unresolved, and a backlinks panel. Renaming a note rewrites
  inbound `[[links]]` so the graph doesn't break.
- **Ranked full-text search** (SQLite FTS5) with prefix matching as you type.
- **Daily notes** and **quick capture** — file a timestamped line to today's
  journal from anywhere without losing your place.
- **Task rollup** — `- [ ]` items aggregated across all notes.
- **Markdown editor** — split/preview, formatting shortcuts, auto-continuing
  lists and checkboxes, word/char/reading-time.
- **Command palette** for fast note jumping.
- **Theming** — configurable phosphor colors, CRT scanlines, radial glow.

## Build & run

Requires a recent stable Rust toolchain.

```sh
cargo run --release
```

`cargo test` runs the storage/search unit tests.

## Keyboard

| Shortcut          | Action                                  |
|-------------------|-----------------------------------------|
| `Ctrl+P`          | Jump to note (command palette)          |
| `Ctrl+Shift+I`    | Quick capture to today's journal        |
| `Ctrl+B/I/E/K`    | Bold / italic / code / link (in editor) |
| `Enter`           | Continue the current list/checkbox      |

Toolbar buttons cover `[+ NEW]`, `[DELETE]`, `[TODAY]`, `[CAPTURE]`,
`[TASKS]`, and `[CFG]`.

## Data locations

- **Notes (yours, canonical):** the workspace folder — defaults to
  `Documents/stardeck`, configurable in `[CFG]`. Point your sync tool here.
- **Search index (disposable):** per-workspace `index-*.db` in the app data
  dir (`%APPDATA%/stardeck` on Windows). Safe to delete; rebuilt on launch.
- **Config:** `stardeck/config.json` in the app data dir.

Migrating from an older build: a legacy `notes.db` is exported to markdown on
first run and moved aside as `notes.db.pre-files` (nothing is deleted).

## Status

v1.0 — usable. Storage, search, and the rename-link cascade are covered by
unit tests. Two-machine sync relies on your file-sync tool's conflict
handling (concurrent offline edits to the same note resolve to a
"conflicted copy" the indexer surfaces as a second note — see `docs/SYNC.md`).

## License

Not yet licensed — add a license before distributing.
