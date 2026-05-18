#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod db;
mod theme;

use config::{Config, ViewMode};
use db::{Db, Note};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Procedurally drawn app icon — a phosphor `>_` prompt on near-black, in the
/// same palette as the UI. Generated in code so we pull in no image/PNG crate.
fn app_icon() -> egui::IconData {
    const N: i32 = 64;
    let bg = [6u8, 18, 10, 255]; // lifted black: visible on dark taskbars
    let border = [40u8, 200, 90, 255]; // theme accent
    let glyph = [51u8, 255, 102, 255]; // bright phosphor

    // Distance from point (px,py) to segment a->b, for fixed-width strokes.
    let dist_seg = |px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32| {
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let t = if len2 == 0.0 {
            0.0
        } else {
            (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
        };
        let (cx, cy) = (ax + t * dx, ay + t * dy);
        ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
    };

    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32, y as f32);
            // Inset frame.
            let frame = (x >= 4 && x < N - 4 && y >= 4 && y < N - 4)
                && !(x >= 7 && x < N - 7 && y >= 7 && y < N - 7);
            // `>` chevron.
            let chevron = dist_seg(fx, fy, 16.0, 18.0, 30.0, 32.0) < 3.0
                || dist_seg(fx, fy, 30.0, 32.0, 16.0, 46.0) < 3.0;
            // `_` cursor bar.
            let cursor = (34..=48).contains(&x) && (43..=46).contains(&y);

            let px = if chevron || cursor {
                glyph
            } else if frame {
                border
            } else {
                bg
            };
            rgba.extend_from_slice(&px);
        }
    }
    egui::IconData {
        rgba,
        width: N as u32,
        height: N as u32,
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(app_icon())
            .with_title("STARDECK"),
        ..Default::default()
    };
    eframe::run_native(
        "STARDECK",
        native_options,
        Box::new(|cc| Ok(Box::new(StarDeck::new(cc)))),
    )
}

#[derive(Clone, Copy)]
enum MdCmd {
    Bold,
    Italic,
    Code,
    Link,
}

/// Wrap the selected character range (or insert a placeholder) with markers.
fn md_wrap(body: &mut String, sel: (usize, usize), open: &str, close: &str, ph: &str) {
    let chars: Vec<char> = body.chars().collect();
    let (mut a, mut b) = (sel.0.min(chars.len()), sel.1.min(chars.len()));
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    let selected: String = chars[a..b].iter().collect();
    let inner = if selected.is_empty() {
        ph.to_string()
    } else {
        selected
    };
    let mut s: String = chars[..a].iter().collect();
    s.push_str(open);
    s.push_str(&inner);
    s.push_str(close);
    s.extend(chars[b..].iter());
    *body = s;
}

struct StarDeck {
    db: Db,
    cfg: Config,
    notes: Vec<Note>,
    selected: Option<String>,
    title: String,
    body: String,
    folder: String,
    tags: String,
    filter: String,
    active_tag: Option<String>,
    sel: Option<(usize, usize)>,
    dirty: bool,
    last_edit: Instant,
    md_cache: CommonMarkCache,
    boot: Instant,
    status: String,
    show_settings: bool,
    theme_dirty: bool,
    palette_open: bool,
    palette_query: String,
    palette_notes: Vec<Note>,
    palette_focus: bool,
    show_tasks: bool,
    capture_open: bool,
    capture_text: String,
    capture_focus: bool,
}

impl StarDeck {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load();
        theme::apply(&cc.egui_ctx, &cfg);
        let db = Db::open(&cfg.workspace_path()).expect("open workspace");
        let notes = db.list("").unwrap_or_default();
        let mut app = StarDeck {
            db,
            cfg,
            notes,
            selected: None,
            title: String::new(),
            body: String::new(),
            folder: String::new(),
            tags: String::new(),
            filter: String::new(),
            active_tag: None,
            sel: None,
            dirty: false,
            last_edit: Instant::now(),
            md_cache: CommonMarkCache::default(),
            boot: Instant::now(),
            status: "".to_string(),
            show_settings: false,
            theme_dirty: false,
            palette_open: false,
            palette_query: String::new(),
            palette_notes: Vec::new(),
            palette_focus: false,
            show_tasks: false,
            capture_open: false,
            capture_text: String::new(),
            capture_focus: false,
        };
        if let Some(first) = app.notes.first().cloned() {
            app.load(&first);
        }
        app
    }

    fn refresh(&mut self) {
        self.notes = self.db.list(&self.filter).unwrap_or_default();
    }

    fn load(&mut self, note: &Note) {
        self.selected = Some(note.id.clone());
        self.title = note.title.clone();
        self.body = note.body.clone();
        self.folder = note.folder.clone();
        self.tags = note.tags.clone();
        self.dirty = false;
    }

    fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(id) = self.selected.clone() {
            let note = Note {
                id,
                title: self.title.clone(),
                body: self.body.clone(),
                folder: self.folder.trim().trim_matches('/').to_string(),
                tags: normalize_tags(&self.tags),
                ..Default::default()
            };
            if self.db.save(&note).is_ok() {
                self.dirty = false;
                self.refresh();
            }
        }
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.last_edit = Instant::now();
    }

    fn apply_md(&mut self, cmd: MdCmd) {
        let sel = self.sel.unwrap_or((self.body.chars().count(), self.body.chars().count()));
        match cmd {
            MdCmd::Bold => md_wrap(&mut self.body, sel, "**", "**", "bold"),
            MdCmd::Italic => md_wrap(&mut self.body, sel, "*", "*", "italic"),
            MdCmd::Code => md_wrap(&mut self.body, sel, "`", "`", "code"),
            MdCmd::Link => md_wrap(&mut self.body, sel, "[", "](url)", "text"),
        }
        self.touch();
    }

    fn open_daily(&mut self) {
        self.flush();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if let Ok(n) = self.db.daily(&today, "journal") {
            self.refresh();
            self.load(&n);
            self.show_settings = false;
        }
    }

    fn open_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_notes = self.db.list("").unwrap_or_default();
        self.palette_focus = true;
    }

    /// Notes that link to the current one. Scans the full index (not the
    /// filtered view) and matches `[[title]]` case-insensitively.
    /// Append a timestamped line to today's journal note without disturbing
    /// whatever note is currently open. If the journal note *is* the one being
    /// edited, the editor is refreshed so the new line shows immediately.
    fn commit_capture(&mut self) {
        let text = self.capture_text.trim().to_string();
        self.capture_text.clear();
        self.capture_open = false;
        if text.is_empty() {
            return;
        }
        self.flush(); // don't lose in-progress edits to the current note
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if let Ok(mut n) = self.db.daily(&today, "journal") {
            let stamp = chrono::Local::now().format("%H:%M");
            if !n.body.is_empty() && !n.body.ends_with('\n') {
                n.body.push('\n');
            }
            n.body.push_str(&format!("- {stamp} {text}\n"));
            if self.db.save(&n).is_ok() {
                self.refresh();
                if self.selected.as_deref() == Some(n.id.as_str()) {
                    self.load(&n);
                }
                self.status = format!(" // captured to journal/{today}");
            }
        }
    }

    fn backlinks(&self) -> Vec<Note> {
        let title = self.title.trim();
        if title.is_empty() {
            return vec![];
        }
        let needle = format!("[[{}]]", title.to_ascii_lowercase());
        self.db
            .list("")
            .unwrap_or_default()
            .into_iter()
            .filter(|n| {
                Some(&n.id) != self.selected.as_ref()
                    && n.body.to_ascii_lowercase().contains(&needle)
            })
            .collect()
    }

    /// `[[targets]]` referenced by the current body, in first-seen order, each
    /// paired with the note it resolves to (None = unresolved).
    fn outgoing_links(&self) -> Vec<(String, Option<Note>)> {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for target in wiki_targets(&self.body) {
            let key = target.to_ascii_lowercase();
            if seen.insert(key) {
                let hit = self.db.by_title(&target);
                out.push((target, hit));
            }
        }
        out
    }

    fn all_tags(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = Default::default();
        for n in &self.notes {
            for t in note_tags(n) {
                set.insert(t);
            }
        }
        set.into_iter().collect()
    }

    fn tree_panel(&mut self, ui: &mut egui::Ui) {
        // Optional in-memory tag filter on top of the DB text search.
        let notes: Vec<Note> = self
            .notes
            .iter()
            .filter(|n| match &self.active_tag {
                Some(t) => note_tags(n).iter().any(|x| x == t),
                None => true,
            })
            .cloned()
            .collect();

        let mut groups: BTreeMap<String, Vec<Note>> = BTreeMap::new();
        for n in notes {
            groups.entry(n.folder.clone()).or_default().push(n);
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(root) = groups.get("") {
                for n in root {
                    self.note_row(ui, n);
                }
            }
            for (folder, items) in groups.iter().filter(|(k, _)| !k.is_empty()) {
                egui::CollapsingHeader::new(format!("▸ {folder}"))
                    .default_open(true)
                    .id_salt(folder)
                    .show(ui, |ui| {
                        for n in items {
                            self.note_row(ui, n);
                        }
                    });
            }
        });
    }

    fn note_row(&mut self, ui: &mut egui::Ui, n: &Note) {
        let sel = Some(&n.id) == self.selected.as_ref();
        let label = if n.title.trim().is_empty() {
            "untitled".to_string()
        } else {
            n.title.clone()
        };
        if ui.selectable_label(sel, format!("» {label}")).clicked() {
            self.flush();
            self.load(n);
            self.show_settings = false;
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("// CONSOLE CONFIG");
        ui.add_space(8.0);
        ui.label("DISPLAY");
        ui.horizontal(|ui| {
            ui.label("text color   ");
            if ui.color_edit_button_srgb(&mut self.cfg.text_color).changed() {
                self.theme_dirty = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("accent/lines ");
            if ui.color_edit_button_srgb(&mut self.cfg.accent_color).changed() {
                self.theme_dirty = true;
            }
        });
        ui.add(egui::Slider::new(&mut self.cfg.scanline_alpha, 0..=120).text("scanline opacity"));
        ui.add(egui::Slider::new(&mut self.cfg.scanline_gap, 2..=10).text("scanline spacing"));
        ui.add(egui::Slider::new(&mut self.cfg.glow_alpha, 0..=60).text("background glow"));

        ui.add_space(8.0);
        ui.label("DEFAULT VIEW");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.cfg.view_mode, ViewMode::Markdown, "[MD]");
            ui.selectable_value(&mut self.cfg.view_mode, ViewMode::Preview, "[PREVIEW]");
            ui.selectable_value(&mut self.cfg.view_mode, ViewMode::Split, "[SPLIT]");
        });

        ui.add_space(8.0);
        ui.label("FEATURES");
        ui.checkbox(&mut self.cfg.daily_notes, "daily notes — [TODAY] opens journal/<date>");

        ui.add_space(8.0);
        ui.label("WORKSPACE  (point a sync tool here to share across machines)");
        let ws_hint = self.cfg.workspace_path().to_string_lossy().into_owned();
        ui.add(
            egui::TextEdit::singleline(&mut self.cfg.workspace_dir)
                .desired_width(f32::INFINITY)
                .hint_text(ws_hint),
        );
        ui.small("Notes are .md files in this folder. Empty = default. Change takes effect on restart.");

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("[SAVE CONFIG]").clicked() {
                self.cfg.save();
                self.theme_dirty = true;
            }
            if ui.button("[CLOSE]").clicked() {
                self.cfg.save();
                self.show_settings = false;
            }
        });
        ui.add_space(6.0);
        ui.small("Config: %APPDATA%\\stardeck\\config.json");
    }

    fn tasks_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("// TASKS — open items");
        ui.add_space(6.0);
        let notes = self.db.list("").unwrap_or_default();
        let mut chosen: Option<Note> = None;
        let mut total_open = 0usize;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for n in &notes {
                let mut open: Vec<&str> = Vec::new();
                let mut done = 0usize;
                for line in n.body.lines() {
                    let l = line.trim_start();
                    if let Some(t) = task_text(l, false) {
                        open.push(t);
                    } else if task_text(l, true).is_some() {
                        done += 1;
                    }
                }
                if open.is_empty() {
                    continue;
                }
                total_open += open.len();
                let title = if n.title.trim().is_empty() {
                    "untitled"
                } else {
                    n.title.as_str()
                };
                let loc = if n.folder.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", n.folder)
                };
                ui.add_space(6.0);
                if ui
                    .selectable_label(
                        false,
                        format!("» {title}{loc}  [{}/{}]", open.len(), open.len() + done),
                    )
                    .clicked()
                {
                    chosen = Some(n.clone());
                }
                for t in open {
                    if ui.selectable_label(false, format!("    ☐ {t}")).clicked() {
                        chosen = Some(n.clone());
                    }
                }
            }
            if total_open == 0 {
                ui.label("// no open tasks — add `- [ ] something` to any note");
            }
        });

        if let Some(n) = chosen {
            self.flush();
            self.load(&n);
            self.show_tasks = false;
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        if self.selected.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("// NO NOTE SELECTED — [+ NEW] TO BEGIN");
            });
            return;
        }

        if ui
            .add(
                egui::TextEdit::singleline(&mut self.title)
                    .desired_width(f32::INFINITY)
                    .hint_text("title"),
            )
            .changed()
        {
            self.touch();
        }
        ui.horizontal(|ui| {
            ui.label("DIR");
            let f = ui.add(
                egui::TextEdit::singleline(&mut self.folder)
                    .desired_width(220.0)
                    .hint_text("work/meetings"),
            );
            ui.label("TAGS");
            let t = ui.add(
                egui::TextEdit::singleline(&mut self.tags)
                    .desired_width(f32::INFINITY)
                    .hint_text("comma, separated"),
            );
            if f.changed() || t.changed() {
                self.touch();
            }
        });

        ui.horizontal(|ui| {
            for (m, label) in [
                (ViewMode::Markdown, "[MD]"),
                (ViewMode::Preview, "[PREVIEW]"),
                (ViewMode::Split, "[SPLIT]"),
            ] {
                if ui.selectable_label(self.cfg.view_mode == m, label).clicked() {
                    self.cfg.view_mode = m;
                    self.cfg.save();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let words = self.body.split_whitespace().count();
                let mins = words.div_ceil(200).max(1); // ~200 wpm
                ui.small(format!(
                    "{} words · {} chars · ~{}m read",
                    words,
                    self.body.chars().count(),
                    mins
                ));
            });
        });
        ui.separator();

        let avail = ui.available_size();
        let pane_h = avail.y - 120.0;
        let show_editor = matches!(self.cfg.view_mode, ViewMode::Markdown | ViewMode::Split);
        let show_preview = matches!(self.cfg.view_mode, ViewMode::Preview | ViewMode::Split);
        let split = self.cfg.view_mode == ViewMode::Split;
        let mut editor_focused = false;

        ui.horizontal_top(|ui| {
            if show_editor {
                let w = if split { avail.x * 0.5 } else { avail.x };
                ui.allocate_ui(egui::vec2(w, pane_h), |ui| {
                    egui::ScrollArea::vertical().id_salt("editor").show(ui, |ui| {
                        let out = egui::TextEdit::multiline(&mut self.body)
                            .desired_width(f32::INFINITY)
                            .desired_rows(24)
                            .hint_text(
                                "# Heading   ## Subheading\n\
                                 **bold**  *italic*  `code`  ~~strike~~\n\
                                 > blockquote\n\
                                 - bullet      1. numbered      #tag\n\
                                 - [ ] task    - [x] done\n\
                                 ```\ncode block\n```\n\
                                 [label](https://url)   [[note title]] link\n\
                                 \n\
                                 ctrl+b bold · ctrl+i italic · ctrl+e code · ctrl+k link\n\
                                 ctrl+p jump to note · ctrl+shift+i quick capture\n\
                                 enter continues lists & checkboxes",
                            )
                            .show(ui);
                        if out.response.changed() {
                            self.touch();
                        }
                        if let Some(cr) = out.cursor_range {
                            self.sel = Some((
                                cr.primary.ccursor.index,
                                cr.secondary.ccursor.index,
                            ));
                        }
                        editor_focused = out.response.has_focus();

                        // Auto-continue lists: after TextEdit inserted the
                        // newline, repeat the bullet/checkbox (or end the list
                        // if the item was empty), then reposition the caret.
                        let resp_id = out.response.id;
                        let mut state = out.state;
                        let enter = ui
                            .input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                        if editor_focused && enter {
                            if let Some(cr) = out.cursor_range {
                                let ci = cr.primary.ccursor.index;
                                let collapsed = cr.primary.ccursor.index
                                    == cr.secondary.ccursor.index;
                                let chars: Vec<char> = self.body.chars().collect();
                                if collapsed && ci >= 1 && ci <= chars.len()
                                    && chars[ci - 1] == '\n'
                                {
                                    let nl = ci - 1;
                                    let prev_start = chars[..nl]
                                        .iter()
                                        .rposition(|&c| c == '\n')
                                        .map(|p| p + 1)
                                        .unwrap_or(0);
                                    let prev: String =
                                        chars[prev_start..nl].iter().collect();
                                    let mut v = chars;
                                    let new_cur = match list_continuation(&prev) {
                                        Some(ListEdit::Continue(s)) => {
                                            let ins: Vec<char> = s.chars().collect();
                                            let add = ins.len();
                                            v.splice(ci..ci, ins);
                                            Some(ci + add)
                                        }
                                        Some(ListEdit::ClearPrev) => {
                                            let removed = nl - prev_start;
                                            v.drain(prev_start..nl);
                                            Some(ci - removed)
                                        }
                                        None => None,
                                    };
                                    if let Some(cur) = new_cur {
                                        self.body = v.into_iter().collect();
                                        state.cursor.set_char_range(Some(
                                            egui::text::CCursorRange::one(
                                                egui::text::CCursor::new(cur),
                                            ),
                                        ));
                                        state.store(ui.ctx(), resp_id);
                                        self.touch();
                                    }
                                }
                            }
                        }
                    });
                });
            }
            if show_editor && show_preview {
                ui.separator();
            }
            if show_preview {
                let w = if split { avail.x * 0.5 - 12.0 } else { avail.x };
                ui.allocate_ui(egui::vec2(w, pane_h), |ui| {
                    egui::ScrollArea::vertical().id_salt("preview").show(ui, |ui| {
                        CommonMarkViewer::new().show(ui, &mut self.md_cache, &self.body);
                    });
                });
            }
        });

        if editor_focused {
            let cmd = ui.input(|i| {
                if !i.modifiers.command {
                    return None;
                }
                if i.key_pressed(egui::Key::B) {
                    Some(MdCmd::Bold)
                } else if i.key_pressed(egui::Key::I) {
                    Some(MdCmd::Italic)
                } else if i.key_pressed(egui::Key::E) {
                    Some(MdCmd::Code)
                } else if i.key_pressed(egui::Key::K) {
                    Some(MdCmd::Link)
                } else {
                    None
                }
            });
            if let Some(c) = cmd {
                self.apply_md(c);
            }
        }

        let links = self.outgoing_links();
        if !links.is_empty() {
            ui.separator();
            ui.label(format!("► LINKS ({})", links.len()));
            for (target, hit) in links {
                match hit {
                    Some(note) => {
                        if ui
                            .selectable_label(false, format!("  » {}", note.title))
                            .clicked()
                        {
                            self.flush();
                            self.load(&note);
                        }
                    }
                    None => {
                        if ui
                            .selectable_label(false, format!("  + {target}  (create)"))
                            .on_hover_text("unresolved link — click to create the note")
                            .clicked()
                        {
                            self.flush();
                            if let Ok(n) = self.db.daily(&target, "") {
                                self.refresh();
                                self.load(&n);
                            }
                        }
                    }
                }
            }
        }

        let back = self.backlinks();
        if !back.is_empty() {
            ui.separator();
            ui.label(format!("◄ BACKLINKS ({})", back.len()));
            for b in back {
                if ui
                    .selectable_label(false, format!("  » {}", b.title))
                    .clicked()
                {
                    self.flush();
                    self.load(&b);
                }
            }
        }
    }
}

enum ListEdit {
    /// Prefix to insert on the new line to continue the list.
    Continue(String),
    /// Previous line was an empty marker — clear it to end the list.
    ClearPrev,
}

/// Given the line the user just pressed Enter on, decide how to continue a
/// markdown list: repeat the bullet/checkbox, increment an ordered number, or
/// (when the item was empty) end the list. `None` = not a list line.
fn list_continuation(prev: &str) -> Option<ListEdit> {
    let indent_len = prev.len() - prev.trim_start().len();
    let indent = &prev[..indent_len];
    let rest = &prev[indent_len..];

    let (marker, content) = if let Some(c) = rest
        .strip_prefix("- [ ] ")
        .or_else(|| rest.strip_prefix("- [x] "))
        .or_else(|| rest.strip_prefix("- [X] "))
    {
        ("- [ ] ".to_string(), c)
    } else if let Some(c) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        (rest[..2].to_string(), c)
    } else {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let after = rest.get(digits.len()..).unwrap_or("");
        match (digits.is_empty(), after.strip_prefix(". ")) {
            (false, Some(c)) => {
                let n: u64 = digits.parse().unwrap_or(0);
                (format!("{}. ", n + 1), c)
            }
            _ => return None,
        }
    };

    if content.trim().is_empty() {
        Some(ListEdit::ClearPrev)
    } else {
        Some(ListEdit::Continue(format!("{indent}{marker}")))
    }
}

/// Extract `[[target]]` titles from note text. Targets are single-line and
/// trimmed; `[[ ]]` and unterminated `[[` are ignored.
fn wiki_targets(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(end) = body[i + 2..].find("]]") {
                let inner = &body[i + 2..i + 2 + end];
                if !inner.contains('\n') {
                    let t = inner.trim();
                    if !t.is_empty() {
                        out.push(t.to_string());
                    }
                }
                i += 2 + end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn split_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn normalize_tags(s: &str) -> String {
    split_tags(s).join(", ")
}

/// Obsidian-style #tags embedded in note text. Skips markdown headings
/// (`# ` / `## `) since those have whitespace right after the hash.
fn inline_tags(body: &str) -> Vec<String> {
    let chars: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            let prev_ok = i == 0
                || chars[i - 1].is_whitespace()
                || matches!(chars[i - 1], '(' | '[' | '{' | ',' | ';' | '"' | '\'');
            let start = i + 1;
            let mut j = start;
            while j < chars.len()
                && (chars[j].is_alphanumeric() || matches!(chars[j], '-' | '_' | '/'))
            {
                j += 1;
            }
            if prev_ok && j > start {
                out.push(chars[start..j].iter().collect::<String>().to_lowercase());
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Parse a markdown task line. `done=false` returns text of `- [ ]` items,
/// `done=true` returns text of `- [x]` items. Supports -, *, + bullets.
fn task_text(l: &str, done: bool) -> Option<&str> {
    let bullet = l.chars().next()?;
    if !matches!(bullet, '-' | '*' | '+') {
        return None;
    }
    let inner = l.get(1..)?.strip_prefix(' ')?.strip_prefix('[')?;
    let mark = inner.chars().next()?;
    let after = inner.get(1..)?.strip_prefix(']')?;
    let text = after.strip_prefix(' ').unwrap_or(after);
    match (done, mark) {
        (true, 'x') | (true, 'X') => Some(text),
        (false, ' ') => Some(text),
        _ => None,
    }
}

/// All tags for a note: explicit TAGS field plus inline #tags in the body.
fn note_tags(n: &Note) -> Vec<String> {
    let mut v = split_tags(&n.tags);
    for t in inline_tags(&n.body) {
        if !v.contains(&t) {
            v.push(t);
        }
    }
    v
}

impl eframe::App for StarDeck {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let boot_elapsed = self.boot.elapsed().as_secs_f32();
        if boot_elapsed < 2.0 {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.32);
                    ui.heading("S T A R D E C K");
                    ui.add_space(8.0);
                    let lines = [
                        "> POST ........................ OK",
                        "> WORKSPACE ................... MOUNTED",
                        "> SYNC ........................ VIA FOLDER",
                        "> NOTE INDEX .................. REBUILT",
                        "> READY",
                    ];
                    let shown = ((boot_elapsed / 2.0) * lines.len() as f32) as usize;
                    for l in lines.iter().take(shown.max(1)) {
                        ui.monospace(*l);
                    }
                });
            });
            theme::glow(ctx, &self.cfg);
            theme::scanlines(ctx, &self.cfg);
            ctx.request_repaint();
            return;
        }

        if self.theme_dirty {
            theme::apply(ctx, &self.cfg);
            self.theme_dirty = false;
        }
        if self.dirty && self.last_edit.elapsed() > Duration::from_millis(700) {
            self.flush();
        }

        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::P)) && !self.palette_open {
            self.open_palette();
        }
        if ctx.input(|i| {
            i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::I)
        }) && !self.capture_open
        {
            self.capture_open = true;
            self.capture_focus = true;
        }

        egui::TopBottomPanel::top("hud").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("STARDECK");
                ui.separator();
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(self.show_settings, "[CFG]").clicked() {
                        self.show_settings = !self.show_settings;
                        if self.show_settings {
                            self.show_tasks = false;
                        } else {
                            self.cfg.save();
                        }
                    }
                    if ui.selectable_label(self.show_tasks, "[TASKS]").clicked() {
                        self.show_tasks = !self.show_tasks;
                        if self.show_tasks {
                            self.show_settings = false;
                        }
                    }
                    ui.label(if self.dirty { "● UNSAVED" } else { "● SAVED" });
                });
            });
        });

        egui::SidePanel::left("notes")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("[+ NEW]").clicked() {
                        self.flush();
                        if let Ok(n) = self.db.create() {
                            self.refresh();
                            self.load(&n);
                            self.show_settings = false;
                        }
                    }
                    if ui.button("[DELETE]").clicked() {
                        if let Some(id) = self.selected.clone() {
                            let _ = self.db.delete(&id);
                            self.selected = None;
                            self.title.clear();
                            self.body.clear();
                            self.folder.clear();
                            self.tags.clear();
                            self.dirty = false;
                            self.refresh();
                        }
                    }
                    if self.cfg.daily_notes && ui.button("[TODAY]").clicked() {
                        self.open_daily();
                    }
                    if ui
                        .button("[CAPTURE]")
                        .on_hover_text("ctrl+shift+i — file a quick line to today's journal")
                        .clicked()
                    {
                        self.capture_open = true;
                        self.capture_focus = true;
                    }
                });
                ui.add_space(4.0);
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text(":search")
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.refresh();
                }

                let tags = self.all_tags();
                if !tags.is_empty() {
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .selectable_label(self.active_tag.is_none(), "#all")
                            .clicked()
                        {
                            self.active_tag = None;
                        }
                        for t in tags {
                            let on = self.active_tag.as_deref() == Some(t.as_str());
                            if ui.selectable_label(on, format!("#{t}")).clicked() {
                                self.active_tag = if on { None } else { Some(t) };
                            }
                        }
                    });
                }
                ui.separator();
                self.tree_panel(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.show_tasks {
                self.tasks_ui(ui);
            } else if self.show_settings {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_ui(ui));
            } else {
                self.editor_ui(ui);
            }
        });

        if self.palette_open {
            let q = self.palette_query.to_lowercase();
            let all = self.palette_notes.clone();
            let matches: Vec<Note> = all
                .into_iter()
                .filter(|n| {
                    q.is_empty()
                        || n.title.to_lowercase().contains(&q)
                        || n.folder.to_lowercase().contains(&q)
                        || n.tags.to_lowercase().contains(&q)
                })
                .take(50)
                .collect();

            let mut chosen: Option<Note> = None;
            let mut close = false;

            egui::Window::new("» JUMP TO NOTE  (esc to close)")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 90.0))
                .default_width(520.0)
                .show(ctx, |ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.palette_query)
                            .desired_width(f32::INFINITY)
                            .hint_text("type to filter…"),
                    );
                    if self.palette_focus {
                        resp.request_focus();
                        self.palette_focus = false;
                    }
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for n in &matches {
                            let loc = if n.folder.is_empty() {
                                String::new()
                            } else {
                                format!("  ({})", n.folder)
                            };
                            let title = if n.title.trim().is_empty() {
                                "untitled"
                            } else {
                                n.title.as_str()
                            };
                            if ui
                                .selectable_label(false, format!("» {title}{loc}"))
                                .clicked()
                            {
                                chosen = Some(n.clone());
                            }
                        }
                    });
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if let Some(first) = matches.first() {
                            chosen = Some(first.clone());
                        }
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });

            if let Some(n) = chosen {
                self.flush();
                self.load(&n);
                self.show_settings = false;
                self.palette_open = false;
            } else if close {
                self.palette_open = false;
            }
        }

        if self.capture_open {
            let mut commit = false;
            let mut cancel = false;
            egui::Window::new("» QUICK CAPTURE → today's journal  (esc to cancel)")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 90.0))
                .default_width(520.0)
                .show(ctx, |ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.capture_text)
                            .desired_width(f32::INFINITY)
                            .hint_text("jot a line — enter to file it, keeps your place"),
                    );
                    if self.capture_focus {
                        resp.request_focus();
                        self.capture_focus = false;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                });
            if commit {
                self.commit_capture();
            } else if cancel {
                self.capture_text.clear();
                self.capture_open = false;
            }
        }

        theme::glow(ctx, &self.cfg);
        theme::scanlines(ctx, &self.cfg);

        if self.dirty {
            ctx.request_repaint_after(Duration::from_millis(750));
        }
    }
}
