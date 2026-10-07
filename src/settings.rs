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
    /// Dock), its icon is in the menu bar (it is, unless the user says otherwise), and Enter sends
    /// a message (else ⌘Enter does).
    pub quit_on_close: bool,
    pub show_in_menu_bar: bool,
    pub send_with_enter: bool,
    /// Settings → Notifications & sounds, FinchGram's own part of it. (Tables, this and the next,
    /// have to come after the plain values in the file.)
    pub notifications: Notifications,
    /// How wide each theme's chat list is, as the user last dragged its edge.
    pub list_widths: ListWidths,
}

/// What FinchGram does when a message arrives (src/telegram/notifications.rs), as the design has it
/// by default. Which kinds of chat notify is Telegram's own setting, kept for the account.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    /// A notification for each new message, in the system's notification centre.
    pub desktop: bool,
    /// The notification shows the message, else only that one came.
    pub previews: bool,
    /// The notification plays the system's sound.
    pub sounds: bool,
    /// FinchGram's icon draws the eye while FinchGram is not in front: on macOS the Dock icon
    /// bounces once.
    pub flash_taskbar: bool,
    /// The unread count on FinchGram's icon counts the messages of muted chats too.
    pub count_muted_chats: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications { desktop: true, previews: true, sounds: false, flash_taskbar: true, count_muted_chats: false }
    }
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
            show_in_menu_bar: true,
            send_with_enter: true,
            notifications: Notifications::default(),
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
        assert!(!read.quit_on_close && read.show_in_menu_bar && read.send_with_enter);
        assert_eq!(read.notifications, Notifications::default());
    }

    #[test]
    fn notifications_are_a_table_of_their_own() {
        let settings = Settings { notifications: Notifications { sounds: true, ..Notifications::default() }, ..Settings::default() };
        let text = toml::to_string_pretty(&settings).expect("TOML");
        assert!(text.contains("[notifications]\ndesktop = true\npreviews = true\nsounds = true\n"), "{text}");
        let read: Settings = toml::from_str(&text).expect("read back");
        assert_eq!(read.notifications, settings.notifications);
        let read: Settings = toml::from_str("[notifications]\nflash_taskbar = false\n").expect("read");
        assert!(!read.notifications.flash_taskbar && read.notifications.desktop);
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
