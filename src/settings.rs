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
    /// Settings → General: closing the window quits FinchGram (else, on macOS, it stays in the
    /// Dock), its icon is in the menu bar, and Enter sends a message (else ⌘Enter does).
    pub quit_on_close: bool,
    pub show_in_menu_bar: bool,
    pub send_with_enter: bool,
    /// How wide each theme's chat list is, as the user last dragged its edge. (A table: it has to
    /// come after the plain values in the file.)
    pub list_widths: ListWidths,
}

/// The width of the chat list in each theme, in logical pixels: the design's until the user drags
/// the list's edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListWidths {
    pub workbench: f32,
    pub broadsheet: f32,
    pub terminal: f32,
}

impl Default for ListWidths {
    fn default() -> Self {
        ListWidths { workbench: 244.0, broadsheet: 300.0, terminal: 268.0 }
    }
}

impl ListWidths {
    /// A width that makes no sense (edited by hand) is the design's again. The pages keep a
    /// dragged width within their own limits.
    fn sanitized(self) -> ListWidths {
        let design = ListWidths::default();
        let sane = |width: f32, fallback: f32| if width.is_finite() && (120.0..=1200.0).contains(&width) { width } else { fallback };
        ListWidths {
            workbench: sane(self.workbench, design.workbench),
            broadsheet: sane(self.broadsheet, design.broadsheet),
            terminal: sane(self.terminal, design.terminal),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: String::new(),
            appearance: "system".to_string(),
            theme: "workbench".to_string(),
            check_for_updates: true,
            quit_on_close: false,
            show_in_menu_bar: false,
            send_with_enter: true,
            list_widths: ListWidths::default(),
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
        settings.list_widths = settings.list_widths.sanitized();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_from_an_older_version_gets_the_defaults() {
        let read: Settings = toml::from_str("theme = \"terminal\"\ncheck_for_updates = false\n").expect("read");
        assert_eq!((read.theme.as_str(), read.check_for_updates), ("terminal", false));
        assert!(!read.quit_on_close && !read.show_in_menu_bar && read.send_with_enter);
    }

    #[test]
    fn list_widths_are_written_after_the_plain_values_and_read_back() {
        let settings = Settings { list_widths: ListWidths { workbench: 320.0, ..ListWidths::default() }, ..Settings::default() };
        let text = toml::to_string_pretty(&settings).expect("TOML");
        let read: Settings = toml::from_str(&text).expect("read back");
        assert_eq!(read.list_widths.workbench, 320.0);
        assert_eq!(ListWidths { terminal: f32::NAN, ..ListWidths::default() }.sanitized().terminal, 268.0);
    }
}
