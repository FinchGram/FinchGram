//! User settings: the user's own preferences, nothing else. Stored as TOML in the platform's
//! config folder (`~/Library/Application Support/FinchGram/settings.toml` on macOS).
//!
//! Keep this deliberately small. Telegram's own state (accounts, chats, downloaded files) lives in
//! TDLib's database, never here.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// UI language, a folder name under lang/ ("en", "zh_Hans"); empty until the user or the system
    /// locale picks one.
    pub language: String,
    /// "system", "light" or "dark".
    pub appearance: String,
    /// One of the design's three themes: "workbench" (the default), "broadsheet" or "terminal".
    pub theme: String,
    /// The once-a-day update check (src/update.rs).
    pub check_for_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: String::new(),
            appearance: "system".to_string(),
            theme: "workbench".to_string(),
            check_for_updates: true,
        }
    }
}

impl Settings {
    /// Saved settings, with defaults filled in for anything missing.
    pub fn load() -> Self {
        let mut settings: Settings = file()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| match toml::from_str(&text) {
                Ok(settings) => Some(settings),
                Err(err) => {
                    eprintln!("settings: ignoring unreadable settings.toml: {err}");
                    None
                }
            })
            .unwrap_or_default();
        if !matches!(settings.appearance.as_str(), "system" | "light" | "dark") {
            settings.appearance = "system".to_string();
        }
        if !matches!(settings.theme.as_str(), "workbench" | "broadsheet" | "terminal") {
            settings.theme = "workbench".to_string();
        }
        settings
    }

    pub fn save(&self) {
        let Some(path) = file() else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let text = match toml::to_string_pretty(self) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("settings: cannot serialize settings: {err}");
                return;
            }
        };
        if let Err(err) = fs::write(&path, text) {
            eprintln!("settings: cannot write {}: {err}", path.display());
        }
    }
}

fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("FinchGram").join("settings.toml"))
}
