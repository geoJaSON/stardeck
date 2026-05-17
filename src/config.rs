use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewMode {
    Markdown,
    Preview,
    #[default]
    Split,
}

/// Remote sync target. Only Postgres is implemented; the enum exists so the
/// sync engine can grow new `RemoteStore` impls without a schema rewrite.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BackendKind {
    #[default]
    Postgres,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Primary phosphor color (all text).
    pub text_color: [u8; 3],
    /// Accent used for borders, separators, selection.
    pub accent_color: [u8; 3],
    /// Scanline overlay opacity, 0 (off) .. 120 (heavy).
    pub scanline_alpha: u8,
    /// Pixels between scanlines; larger = sparser.
    pub scanline_gap: u32,
    pub view_mode: ViewMode,
    /// Show the [TODAY] button that opens journal/<date>.
    pub daily_notes: bool,
    pub backend: BackendKind,
    /// e.g. postgres://user:pass@host:5432/stardeck — empty until configured.
    pub connection_string: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            text_color: [51, 255, 102],
            accent_color: [40, 200, 90],
            scanline_alpha: 18,
            scanline_gap: 4,
            view_mode: ViewMode::Split,
            daily_notes: true,
            backend: BackendKind::Postgres,
            connection_string: String::new(),
        }
    }
}

impl Config {
    pub fn load() -> Config {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn config_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("stardeck");
    p.push("config.json");
    p
}
