use crate::config::Config;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, FontFamily, FontId, Pos2, Rounding, Shape, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(2, 6, 3);
pub const PANEL: Color32 = Color32::from_rgb(4, 12, 6);

fn dim(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(
        (c[0] as f32 * 0.55) as u8,
        (c[1] as f32 * 0.55) as u8,
        (c[2] as f32 * 0.55) as u8,
    )
}

/// Push a color toward white. Used for **bold** / headings, which egui can only
/// distinguish by color (there is no bold monospace glyph set loaded).
fn bright(c: [u8; 3]) -> Color32 {
    let lift = |v: u8| (v as f32 + (255.0 - v as f32) * 0.55) as u8;
    Color32::from_rgb(lift(c[0]), lift(c[1]), lift(c[2]))
}

/// Death Star console styling, driven by the user's configured colors.
pub fn apply(ctx: &egui::Context, cfg: &Config) {
    let clamp = |v: f32| v.clamp(0.0, 255.0) as u8;
    let brightness_mult = cfg.text_brightness.max(0.0);
    let text = Color32::from_rgb(
        clamp(cfg.text_color[0] as f32 * brightness_mult),
        clamp(cfg.text_color[1] as f32 * brightness_mult),
        clamp(cfg.text_color[2] as f32 * brightness_mult),
    );
    let accent = Color32::from_rgb(
        cfg.accent_color[0],
        cfg.accent_color[1],
        cfg.accent_color[2],
    );
    let accent_dim = dim(cfg.accent_color);

    let mut style = (*ctx.style()).clone();

    use FontFamily::Monospace as M;
    // Heading is the top of the range egui_commonmark interpolates h1..h6
    // against (down to Body). A wide spread gives markdown a real hierarchy.
    style.text_styles = [
        (TextStyle::Heading, FontId::new(34.0, M)),
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
    // Inline `code` background. Unset, egui falls back to a gray-64 box that
    // clashes with the near-black phosphor theme; a faint green-black reads as
    // a subtle inline chip instead.
    v.code_bg_color = Color32::from_rgb(6, 26, 12);
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

    // strong_text_color() resolves to widgets.active.fg_stroke. The loop above
    // set it to plain `text`, making **bold** and markdown headings identical
    // to body text. A brighter phosphor makes them stand out.
    v.widgets.active.fg_stroke = Stroke::new(1.0, bright(cfg.text_color));

    // egui keeps a separate Style for dark and light themes and follows the
    // OS theme. `ctx.set_style` only touches the *active* one, so a user whose
    // OS is in the other mode than the dev sees egui's unstyled default
    // (white bg, proportional font, blue selection). Pin the theme and style
    // *both* slots so the console look is identical on every machine.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|s| *s = style.clone());
}

/// Soft radial phosphor glow, brightest at screen center and fading to nothing
/// before the edges. A faint foreground wash (like the scanlines), so it lifts
/// the whole surface without the opaque panel fills hiding it.
/// Skipped entirely at alpha 0.
pub fn glow(ctx: &egui::Context, cfg: &Config) {
    if cfg.glow_alpha == 0 {
        return;
    }
    let screen = ctx.screen_rect();
    let center = screen.center();
    let radius = screen.width().max(screen.height()) * 0.7;

    // Foreground (not Background) — a Background layer would be hidden by the
    // opaque panel fill. Called before scanlines(), so it paints underneath it.
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("bg_glow"),
    ));

    let hot = Color32::from_rgba_unmultiplied(
        cfg.accent_color[0],
        cfg.accent_color[1],
        cfg.accent_color[2],
        cfg.glow_alpha,
    );
    let edge = Color32::from_rgba_unmultiplied(
        cfg.accent_color[0],
        cfg.accent_color[1],
        cfg.accent_color[2],
        0,
    );

    // Triangle fan: one bright center vertex ringed by transparent ones.
    const SEGMENTS: usize = 48;
    let mut mesh = Mesh::default();
    mesh.vertices.push(Vertex {
        pos: center,
        uv: WHITE_UV,
        color: hot,
    });
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        mesh.vertices.push(Vertex {
            pos: Pos2::new(center.x + a.cos() * radius, center.y + a.sin() * radius),
            uv: WHITE_UV,
            color: edge,
        });
    }
    for i in 0..SEGMENTS as u32 {
        mesh.indices.extend_from_slice(&[0, i + 1, i + 2]);
    }
    painter.add(Shape::mesh(mesh));
}
