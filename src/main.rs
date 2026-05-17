#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod db;
mod theme;

use config::{BackendKind, Config, ViewMode};
use db::{Db, Note};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([720.0, 480.0])
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
}

impl StarDeck {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load();
        theme::apply(&cc.egui_ctx, &cfg);
        let db = Db::open().expect("open local cache");
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
            status: " // DATA LINK: OFFLINE".to_string(),
            show_settings: false,
            theme_dirty: false,
            palette_open: false,
            palette_query: String::new(),
            palette_notes: Vec::new(),
            palette_focus: false,
            show_tasks: false,
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

    fn backlinks(&self) -> Vec<Note> {
        if self.title.trim().is_empty() {
            return vec![];
        }
        let needle = format!("[[{}]]", self.title.trim());
        self.notes
            .iter()
            .filter(|n| Some(&n.id) != self.selected.as_ref() && n.body.contains(&needle))
            .cloned()
            .collect()
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
        ui.label("SYNC TARGET  (engine not wired yet)");
        egui::ComboBox::from_label("backend")
            .selected_text(format!("{:?}", self.cfg.backend))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.cfg.backend, BackendKind::Postgres, "Postgres");
            });
        ui.label("connection string");
        ui.add(
            egui::TextEdit::singleline(&mut self.cfg.connection_string)
                .desired_width(f32::INFINITY)
                .hint_text("postgres://user:pass@host:5432/stardeck"),
        );

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
                ui.small(format!("{} words · {} chars", words, self.body.chars().count()));
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
                            .hint_text("# markdown\n\nctrl+b bold · ctrl+i italic\nlink with [[note title]]")
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
                        "> LOCAL CACHE ................. MOUNTED",
                        "> DATA LINK ................... DEFERRED",
                        "> NOTE INDEX .................. LOADED",
                        "> READY",
                    ];
                    let shown = ((boot_elapsed / 2.0) * lines.len() as f32) as usize;
                    for l in lines.iter().take(shown.max(1)) {
                        ui.monospace(*l);
                    }
                });
            });
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

        theme::scanlines(ctx, &self.cfg);

        if self.dirty {
            ctx.request_repaint_after(Duration::from_millis(750));
        }
    }
}
