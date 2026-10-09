//! User settings: the user's own preferences, nothing else. Stored as TOML in the platform's
//! config folder (`~/Library/Application Support/FinchGram/settings.toml` on macOS).
//!
//! Keep this deliberately small. Telegram's own state (accounts, chats, downloaded files) lives in
//! TDLib's database, never here.

use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// UI language, a folder name under lang/ ("en", "zh_Hans"); empty until the user or the system
    /// locale picks one.
    pub language: String,
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
    /// The media viewer's video volume, 0 … 100, and whether its sound is off: as the user last
    /// left them, for the next video and the next start (src/telegram/viewer.rs).
    pub video_volume: i32,
    pub video_muted: bool,
    /// Light or dark, each theme's own. (Tables, this and the next ones, have to come after the
    /// plain values in the file.)
    #[serde(deserialize_with = "appearance_in_file")]
    pub appearance: Appearance,
    /// Settings → Notifications & sounds, FinchGram's own part of it.
    pub notifications: Notifications,
    /// How wide each theme's chat list is, as the user last dragged its edge.
    pub list_widths: ListWidths,
    /// Settings → General → Screenshots (src/screenshot/).
    pub screenshots: Screenshots,
}

/// The screenshot tool: its shortcut, and whether FinchGram's own window hides while the screen is
/// captured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Screenshots {
    /// The keys, as "cmd+shift+a": "cmd", "ctrl", "alt", "shift" and a letter or digit, joined with
    /// "+" in that order.
    pub shortcut: String,
    /// FinchGram's own window hides while the screen is captured, to stay out of the picture; off,
    /// it is in the picture like any other window, and can be taken. (Named afresh: v0.3.0 wrote
    /// `hide_window = true`, then the default, into every settings file.)
    pub hide_own_window: bool,
    /// The shortcut works while FinchGram is in the background too (still to come).
    pub global: bool,
}

impl Default for Screenshots {
    fn default() -> Self {
        Screenshots { shortcut: "cmd+shift+a".to_string(), hide_own_window: false, global: false }
    }
}

/// "system", "light" or "dark" for each of the three themes: a theme switched to comes back as it
/// was left. Workbench and Broadsheet follow the system until the user chooses; Terminal is dark,
/// its better side, until the user chooses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub workbench: String,
    pub broadsheet: String,
    pub terminal: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance { workbench: "system".to_string(), broadsheet: "system".to_string(), terminal: "dark".to_string() }
    }
}

impl Appearance {
    /// The theme's choice, by the theme's name as `Settings::theme` has it.
    pub fn of(&self, theme: &str) -> &str {
        match theme {
            "broadsheet" => &self.broadsheet,
            "terminal" => &self.terminal,
            _ => &self.workbench,
        }
    }

    pub fn set(&mut self, theme: &str, appearance: &str) {
        let slot = match theme {
            "broadsheet" => &mut self.broadsheet,
            "terminal" => &mut self.terminal,
            _ => &mut self.workbench,
        };
        *slot = appearance.to_string();
    }

    /// A word that is none of the three (edited by hand) is the theme's default again.
    fn sanitized(self) -> Appearance {
        let default = Appearance::default();
        let sane = |word: String, fallback: String| if matches!(word.as_str(), "system" | "light" | "dark") { word } else { fallback };
        Appearance {
            workbench: sane(self.workbench, default.workbench),
            broadsheet: sane(self.broadsheet, default.broadsheet),
            terminal: sane(self.terminal, default.terminal),
        }
    }
}

/// Up to v0.3.9 the file had one word for all three themes, `appearance = "dark"`: it still reads,
/// as Workbench's and Broadsheet's; Terminal, which had no choice of its own before, starts dark.
fn appearance_in_file<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Appearance, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum InFile {
        Each(Appearance),
        One(String),
    }
    Ok(match InFile::deserialize(deserializer)? {
        InFile::Each(appearance) => appearance,
        InFile::One(word) => Appearance { workbench: word.clone(), broadsheet: word, ..Appearance::default() },
    })
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
            theme: "workbench".to_string(),
            check_for_updates: true,
            quit_on_close: false,
            show_in_menu_bar: true,
            send_with_enter: true,
            video_volume: 100,
            video_muted: false,
            appearance: Appearance::default(),
            notifications: Notifications::default(),
            list_widths: ListWidths::default(),
            screenshots: Screenshots::default(),
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
        if !matches!(settings.theme.as_str(), "workbench" | "broadsheet" | "terminal") {
            settings.theme = "workbench".to_string();
        }
        settings.appearance = settings.appearance.clone().sanitized();
        settings.list_widths = settings.list_widths.sanitized();
        // A volume that makes no sense (edited by hand) is kept within the slider's range.
        settings.video_volume = settings.video_volume.clamp(0, 100);
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
        assert_eq!((read.video_volume, read.video_muted), (100, false));
        assert_eq!(read.notifications, Notifications::default());
        assert_eq!(read.appearance, Appearance::default());
        assert_eq!((read.appearance.of("workbench"), read.appearance.of("terminal")), ("system", "dark"));
    }

    #[test]
    fn one_appearance_for_all_themes_reads_as_the_two_older_themes_choice() {
        let read: Settings = toml::from_str("appearance = \"light\"\ntheme = \"terminal\"\n").expect("read");
        assert_eq!(read.appearance, Appearance { workbench: "light".into(), broadsheet: "light".into(), terminal: "dark".into() });
    }

    #[test]
    fn each_theme_keeps_its_own_appearance() {
        let mut settings = Settings::default();
        settings.appearance.set("terminal", "light");
        settings.appearance.set("broadsheet", "dark");
        let text = toml::to_string_pretty(&settings).expect("TOML");
        assert!(text.contains("[appearance]\nworkbench = \"system\"\nbroadsheet = \"dark\"\nterminal = \"light\"\n"), "{text}");
        let read: Settings = toml::from_str(&text).expect("read back");
        assert_eq!(read.appearance, settings.appearance);
        let read: Settings = toml::from_str("[appearance]\nworkbench = \"dark\"\n").expect("read");
        assert_eq!((read.appearance.of("workbench"), read.appearance.of("broadsheet"), read.appearance.of("terminal")), ("dark", "system", "dark"));
        let odd = Appearance { terminal: "sepia".into(), ..Appearance::default() }.sanitized();
        assert_eq!(odd, Appearance::default());
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
