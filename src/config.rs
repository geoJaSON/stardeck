use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewMode {
    Markdown,
    Preview,
    #[default]
    Split,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub app_version: String,
    /// Primary phosphor color (all text).
    pub text_color: [u8; 3],
    /// Accent used for borders, separators, selection.
    pub accent_color: [u8; 3],
    pub text_brightness: f32,
    /// Peak opacity of the radial background glow, 0 (off) .. 60 (strong).
    pub glow_alpha: u8,
    pub view_mode: ViewMode,
    /// Show the [TODAY] button that opens journal/<date>.
    pub daily_notes: bool,
    /// Folder of markdown files that are the source of truth. Point a sync tool
    /// (Dropbox / Syncthing / git) at it to share notes across machines. Empty
    /// = use the default location (see `workspace_path`).
    pub workspace_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            text_color: [51, 255, 102],
            accent_color: [40, 200, 90],
            text_brightness: 1.0,
            glow_alpha: 16,
            view_mode: ViewMode::Split,
            daily_notes: true,
            workspace_dir: String::new(),
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

    /// Resolved workspace folder: the configured path, or a default
    /// `stardeck` folder in the user's Documents (falling back to the data
    /// dir) when unset.
    pub fn workspace_path(&self) -> PathBuf {
        let s = self.workspace_dir.trim();
        if !s.is_empty() {
            return PathBuf::from(s);
        }
        let mut p = dirs::document_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        p.push("stardeck");
        p
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
