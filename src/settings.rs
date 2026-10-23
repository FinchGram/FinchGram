//! User settings: the user's own preferences, nothing else. Stored as TOML in the platform's
//! config folder (`~/Library/Application Support/FinchGram/settings.toml` on macOS).
//!
//! Keep this deliberately small, and for preferences only. Records (the WebDAV servers and their
//! passwords, and what comes next) are the app's database's (src/database.rs); Telegram's own
//! state (accounts, chats, downloaded files) lives in TDLib's database, never here.

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
    /// The chat's info panel stays open or shut across chats and restarts (⌘I; src/telegram/
    /// chat_info.rs).
    pub chat_info_open: bool,
    /// How the chats of every folder are ordered under the pinned ones (a folder's menu).
    pub chat_order: ChatOrder,
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
    /// Settings → AI access: the MCP server for AI assistants (src/mcp/, docs/mcp.md).
    pub mcp: Mcp,
    /// The downloads page (src/downloads/, docs/downloads.md): where files go, how many connections
    /// a file comes over, whether the queue column shows.
    pub downloads: Downloads,
}

/// The downloads page's own settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Downloads {
    /// Where downloaded files go, a folder per server under it; "" for ~/Documents/FinchGram/webdav.
    pub folder: String,
    /// How many streams a file is fetched over at once, 1 … 64 (src/downloads/transfer.rs).
    pub connections: u32,
    /// The queue column of the page shows; the user can hide it.
    pub queue_shown: bool,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads { folder: String::new(), connections: crate::downloads::transfer::DEFAULT_CONNECTIONS as u32, queue_shown: true }
    }
}

/// How the chats of a folder are ordered under the pinned ones, which stay on top in Telegram's
/// order whatever the choice (a folder's menu, src/telegram/chats.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatOrder {
    /// Telegram's own order: the chat with the newest message first.
    LatestMessage,
    /// By name, as the UI language orders names (src/platform/).
    Name,
    /// The most unread messages first; muted chats count as read, as their grey badges say.
    Unread,
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

/// Settings → AI access: what an AI assistant connected through the MCP server (src/mcp/,
/// docs/mcp.md) may do. Nothing until the user turns the server on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mcp {
    /// The server answers assistants at all.
    pub enabled: bool,
    /// Which chats an assistant may see and act in: every chat, or only `chats`.
    pub scope: McpScope,
    /// The chats an assistant may see when `scope` is "chosen", by TDLib's chat ids.
    pub chats: Vec<i64>,
    /// Whether an assistant may send messages under the user's name, only write drafts for the
    /// user to send, or neither.
    pub sending: McpSending,
    /// The Telegram account these settings are for, by its user id: `chats` were chosen for it,
    /// and the programs allowed (the database's, src/mcp/access.rs) were allowed for it. When
    /// another account logs in they go, so that every program asks again (`bind_to_account`). 0
    /// until an account has logged in since the setting exists.
    pub account: i64,
}

impl Default for Mcp {
    fn default() -> Self {
        Mcp { enabled: false, scope: McpScope::All, chats: Vec::new(), sending: McpSending::On, account: 0 }
    }
}

impl Mcp {
    /// These settings are the account's with `id` from now on. The chats chosen were for the
    /// account that was logged in then: when this is another one, they go, and true says so (the
    /// caller drops the programs allowed, the database's, too), so that the chats are chosen
    /// again and every program asked about again for this account. The same account again (a
    /// restart, or logging out and in) changes nothing, and a file from before the account was
    /// written down (0) is this account's.
    pub fn bind_to_account(&mut self, id: i64) -> bool {
        if id == 0 || self.account == id {
            return false;
        }
        let first = self.account == 0;
        self.account = id;
        if first {
            return false;
        }
        self.chats.clear();
        true
    }

    /// Whether the chat is within what the user allows an assistant. (Secret chats never are, whatever
    /// the scope: src/telegram/tools.rs.)
    pub fn allows(&self, chat_id: i64) -> bool {
        match self.scope {
            McpScope::All => true,
            McpScope::Chosen => self.chats.contains(&chat_id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpScope {
    All,
    Chosen,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpSending {
    /// Messages go out at once.
    On,
    /// Only drafts, for the user to send.
    Drafts,
    Off,
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
            chat_info_open: false,
            chat_order: ChatOrder::LatestMessage,
            video_volume: 100,
            video_muted: false,
            appearance: Appearance::default(),
            notifications: Notifications::default(),
            list_widths: ListWidths::default(),
            screenshots: Screenshots::default(),
            mcp: Mcp::default(),
            downloads: Downloads::default(),
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
        // 24 was the default from v0.4.7 to v0.4.9 and nothing in the app sets the number, so a
        // file that says 24 means the default, 12 again (src/downloads/transfer.rs).
        if settings.downloads.connections == 24 {
            settings.downloads.connections = crate::downloads::transfer::DEFAULT_CONNECTIONS as u32;
        }
        settings.downloads.connections = settings.downloads.connections.clamp(1, crate::downloads::transfer::MOST_CONNECTIONS as u32);
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
        assert_eq!(read.chat_order, ChatOrder::LatestMessage);
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

    #[test]
    fn the_mcp_server_is_off_until_turned_on_and_its_table_reads_back() {
        let read: Settings = toml::from_str("theme = \"terminal\"\n").expect("read");
        assert_eq!(read.mcp, Mcp::default());
        assert!(!read.mcp.enabled && read.mcp.allows(42) && read.mcp.sending == McpSending::On);
        let settings = Settings {
            mcp: Mcp { enabled: true, scope: McpScope::Chosen, chats: vec![-1001, 42], sending: McpSending::Drafts, account: 1001 },
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&settings).expect("TOML");
        assert!(text.contains("[mcp]\nenabled = true\nscope = \"chosen\"\n"), "{text}");
        assert!(text.contains("sending = \"drafts\"\naccount = 1001\n"), "{text}");
        let read: Settings = toml::from_str(&text).expect("read back");
        assert_eq!(read.mcp, settings.mcp);
        assert!(read.mcp.allows(42) && !read.mcp.allows(7));
        let read: Settings = toml::from_str("[mcp]\nenabled = true\n").expect("read");
        assert!(read.mcp.enabled && read.mcp.scope == McpScope::All && read.mcp.sending == McpSending::On);
        assert_eq!(read.mcp.account, 0);
    }

    #[test]
    fn another_account_starts_without_the_chosen_chats() {
        let mut mcp = Mcp { enabled: true, scope: McpScope::Chosen, chats: vec![42], ..Mcp::default() };
        // A file from before the account was written down is the account's that logs in.
        assert!(!mcp.bind_to_account(1001));
        assert_eq!((mcp.account, mcp.chats.len()), (1001, 1));
        // The same account again: a restart, or logging out and in.
        assert!(!mcp.bind_to_account(1001));
        assert_eq!((mcp.account, mcp.chats.len()), (1001, 1));
        // Another account: the chats chosen were the other one's (and the programs go with them, in access.rs).
        assert!(mcp.bind_to_account(2002));
        assert_eq!((mcp.account, mcp.chats.len()), (2002, 0));
        assert!(mcp.enabled && mcp.scope == McpScope::Chosen && mcp.sending == McpSending::On);
        // Nobody is not an account.
        assert!(!mcp.bind_to_account(0));
        assert_eq!(mcp.account, 2002);
        let text = toml::to_string_pretty(&Settings { mcp, ..Settings::default() }).expect("TOML");
        assert!(text.contains("[mcp]\nenabled = true\nscope = \"chosen\"\nchats = []\nsending = \"on\"\naccount = 2002\n"), "{text}");
    }
}
