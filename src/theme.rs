use crate::config::Config;
use egui::{Color32, FontFamily, FontId, Rounding, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(2, 6, 3);
pub const PANEL: Color32 = Color32::from_rgb(4, 12, 6);

fn dim(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(
        (c[0] as f32 * 0.55) as u8,
        (c[1] as f32 * 0.55) as u8,
        (c[2] as f32 * 0.55) as u8,
    )
}

/// Death Star console styling, driven by the user's configured colors.
pub fn apply(ctx: &egui::Context, cfg: &Config) {
    let text = Color32::from_rgb(cfg.text_color[0], cfg.text_color[1], cfg.text_color[2]);
    let accent = Color32::from_rgb(
        cfg.accent_color[0],
        cfg.accent_color[1],
        cfg.accent_color[2],
    );
    let accent_dim = dim(cfg.accent_color);

    let mut style = (*ctx.style()).clone();

    use FontFamily::Monospace as M;
    style.text_styles = [
        (TextStyle::Heading, FontId::new(24.0, M)),
        (TextStyle::Body, FontId::new(16.0, M)),
        (TextStyle::Monospace, FontId::new(16.0, M)),
        (TextStyle::Button, FontId::new(15.0, M)),
        (TextStyle::Small, FontId::new(12.0, M)),
    ]
    .into();

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(text);
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = Color32::BLACK;
    v.faint_bg_color = PANEL;
    v.hyperlink_color = text;
    v.selection.bg_fill = accent_dim.linear_multiply(0.5);
    v.selection.stroke = Stroke::new(1.0, accent);

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_fill = PANEL;
        w.weak_bg_fill = PANEL;
        w.bg_stroke = Stroke::new(1.0, accent_dim);
        w.fg_stroke = Stroke::new(1.0, text);
        w.rounding = Rounding::ZERO;
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
    v.widgets.active.bg_stroke = Stroke::new(1.5, accent);

    ctx.set_style(style);
}

/// Faint CRT scanlines on a foreground layer. Skipped entirely at alpha 0.
pub fn scanlines(ctx: &egui::Context, cfg: &Config) {
    if cfg.scanline_alpha == 0 {
        return;
    }
    let screen = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("scanlines"),
    ));
    let line = Color32::from_rgba_unmultiplied(0, 0, 0, cfg.scanline_alpha);
    let gap = (cfg.scanline_gap.max(2)) as f32;
    let mut y = screen.top();
    while y < screen.bottom() {
        painter.hline(screen.x_range(), y, Stroke::new(1.0, line));
        y += gap;
    }
}
