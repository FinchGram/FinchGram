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
    /// The account in front when the app last ran, by its slot (src/telegram/mod.rs): the one the
    /// next start shows.
    pub account: u32,
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
    /// The toolbox (src/toolbox.rs): which tools are pinned to the activity bar, in the
    /// toolbox's order; one list for the three themes.
    pub toolbox: Toolbox,
    /// Voice input, a tool of the toolbox's AI category (src/speech/, docs/voice-input.md): the
    /// user speaks, and the words go into the composer, through their own OpenAI key. The key
    /// itself is a credential and lives in the database (database::credential), never here.
    pub voice_input: VoiceInput,
}

/// Voice input's own settings (src/speech/, docs/voice-input.md): the switch, the main language
/// spoken now, and for each main language the user set up, the services that hear it and tidy it,
/// each named by its API's format, its address and its model, so that any vendor or relay that
/// speaks one of the formats can be used. The keys are credentials and live in the database
/// (`voice_input.<language>.speech` and `….tidy`), never here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceInput {
    /// The tool is on: the sparkle shows in the composer. Off until the user turns it on in the
    /// toolbox. A file written before the switch existed (0.7.0) has none, and voice input is on
    /// there when a key was kept (src/speech/ decides at the start).
    pub enabled: Option<bool>,
    /// The main language dictated now, one of the profiles' ("auto", "en", "zh-cn", …).
    pub language: String,
    /// A voice message the user asks to transcribe goes to their own speech service when
    /// Telegram's own transcription cannot be had: that hands Telegram's content to a service of
    /// the user's choosing (docs/voice-input.md), so it is off until the user turns it on.
    pub transcribe_messages: bool,
    /// 0.7.0's and 0.7.1's settings, read once to make the first profile and never written again:
    /// the languages spoken (hints), whether the words were tidied, and the two OpenAI models.
    #[serde(skip_serializing)]
    pub languages: Option<Vec<String>>,
    #[serde(skip_serializing)]
    pub tidy: Option<bool>,
    #[serde(skip_serializing)]
    pub model: Option<String>,
    #[serde(skip_serializing)]
    pub tidy_model: Option<String>,
    /// One per main language the user set up, in the order they were added; never none once
    /// read. Missing from a file it is none (not the default's one), which is how a file of 0.7.x
    /// is known.
    #[serde(default)]
    pub profiles: Vec<VoiceProfile>,
}

/// How one main language is heard and tidied. An empty address or model is the format's own
/// (src/speech/service.rs names them), so that a change of format brings its defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceProfile {
    /// "auto" (any language, found as the user speaks) or one of the UI's languages by the codes
    /// speech services take ("en", "zh-cn", "zh-tw", "ja", …). Words of other languages mixed in
    /// are kept as spoken, whichever it is.
    pub language: String,
    /// The speech service: its API's format ("openai-realtime", "openai-transcriptions",
    /// "qwen-realtime", "doubao"), its address, its model.
    pub format: String,
    pub url: String,
    pub model: String,
    /// Names and terms the user says often, separated by commas or lines: each speech service
    /// takes them as hints (OpenAI's keywords, Qwen's corpus, Doubao's hot words, an upload's
    /// prompt), so that they are heard right.
    pub keywords: String,
    /// OpenAI's live model only: how long it waits to be sure of a word, "minimal" … "xhigh";
    /// empty for FinchGram's own choice. Only in this file.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub delay: String,
    /// After the transcript, a text model takes out the filler words, the repeats and the false
    /// starts and sets the punctuation; off, the words come as heard.
    pub tidy: bool,
    /// The text model's API format ("openai", "anthropic", "gemini"), address and model.
    pub tidy_format: String,
    pub tidy_url: String,
    pub tidy_model: String,
    /// The text model takes the speech service's key: one key for the two.
    pub tidy_same_key: bool,
}

impl Default for VoiceProfile {
    fn default() -> Self {
        VoiceProfile {
            language: "auto".into(),
            format: "openai-realtime".into(),
            url: String::new(),
            model: String::new(),
            keywords: String::new(),
            delay: String::new(),
            tidy: true,
            tidy_format: "openai".into(),
            tidy_url: String::new(),
            tidy_model: String::new(),
            tidy_same_key: true,
        }
    }
}

impl Default for VoiceInput {
    fn default() -> Self {
        VoiceInput {
            enabled: None,
            language: "auto".into(),
            transcribe_messages: false,
            languages: None,
            tidy: None,
            model: None,
            tidy_model: None,
            profiles: vec![VoiceProfile::default()],
        }
    }
}

impl VoiceInput {
    /// The profile used now: the current language's.
    pub fn current(&self) -> &VoiceProfile {
        self.profiles.iter().find(|profile| profile.language == self.language).or(self.profiles.first()).expect("a profile, kept by sanitized")
    }

    pub fn current_mut(&mut self) -> &mut VoiceProfile {
        let index = self.profiles.iter().position(|profile| profile.language == self.language).unwrap_or(0);
        &mut self.profiles[index]
    }

    /// A file of 0.7.x becomes the first profile; what makes no sense (edited by hand) is put
    /// right: unknown languages and formats, a language twice, no profile at all.
    fn sanitized(mut self) -> VoiceInput {
        let legacy = (self.languages.take(), self.tidy.take(), self.model.take(), self.tidy_model.take());
        if self.profiles.is_empty() && legacy != (None, None, None, None) {
            let (languages, tidy, model, tidy_model) = legacy;
            // One language chosen is the main one; several, or none, are any language.
            let language = match languages.as_deref() {
                Some([only]) => only.trim().to_string(),
                _ => "auto".to_string(),
            };
            // The models of 0.7.x's defaults are the formats' own now.
            let own = |name: Option<String>, default: &str| name.map(|name| name.trim().to_string()).filter(|name| !name.is_empty() && name != default).unwrap_or_default();
            self.profiles.push(VoiceProfile {
                language: language.clone(),
                model: own(model, "gpt-live-transcribe"),
                tidy: tidy.unwrap_or(true),
                tidy_model: own(tidy_model, "gpt-5-nano"),
                ..VoiceProfile::default()
            });
            self.language = language;
        }
        let mut seen = Vec::new();
        self.profiles.retain(|profile| {
            let known = profile.language == "auto" || crate::speech::is_language(&profile.language);
            let first = !seen.contains(&profile.language);
            seen.push(profile.language.clone());
            known && first
        });
        for profile in &mut self.profiles {
            if !crate::speech::service::SPEECH_FORMATS.contains(&profile.format.as_str()) {
                profile.format = VoiceProfile::default().format;
            }
            if !crate::speech::service::TIDY_FORMATS.contains(&profile.tidy_format.as_str()) {
                profile.tidy_format = VoiceProfile::default().tidy_format;
            }
            for text in [&mut profile.url, &mut profile.model, &mut profile.keywords, &mut profile.delay, &mut profile.tidy_url, &mut profile.tidy_model] {
                *text = text.trim().to_string();
            }
        }
        if self.profiles.is_empty() {
            self.profiles.push(VoiceProfile::default());
        }
        if !self.profiles.iter().any(|profile| profile.language == self.language) {
            self.language = self.profiles[0].language.clone();
        }
        self
    }
}

/// The toolbox's own setting: the tools pinned to the bar, by id ("downloads", …); nothing
/// until the user pins something (the maintainer's word of 2026-10-24).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toolbox {
    pub pinned: Vec<String>,
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
    /// The tag of these settings under the key only this app has (`database::tag`), written with
    /// them (`Settings::save`) and checked at the start (src/mcp/access.rs): a file changed by
    /// anything but FinchGram, a program allowed for some chats widening its own reach, is not
    /// believed, and these settings go back to their defaults. Not for editing by hand: the AI
    /// access page is.
    pub seal: String,
}

impl Default for Mcp {
    fn default() -> Self {
        Mcp { enabled: false, scope: McpScope::All, chats: Vec::new(), sending: McpSending::On, account: 0, seal: String::new() }
    }
}

impl Mcp {
    /// What the tag is over: everything an assistant's reach is decided by, and the account it
    /// is for.
    pub fn sealed_text(&self) -> String {
        let mut chats = self.chats.clone();
        chats.sort_unstable();
        chats.dedup();
        let chats: Vec<String> = chats.iter().map(i64::to_string).collect();
        let scope = match self.scope {
            McpScope::All => "all",
            McpScope::Chosen => "chosen",
        };
        let sending = match self.sending {
            McpSending::On => "on",
            McpSending::Drafts => "drafts",
            McpSending::Off => "off",
        };
        format!("mcp_policy\0{}\0{scope}\0{}\0{sending}\0{}", u8::from(self.enabled), chats.join(","), self.account)
    }

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
            account: 1,
            chat_order: ChatOrder::LatestMessage,
            video_volume: 100,
            video_muted: false,
            appearance: Appearance::default(),
            notifications: Notifications::default(),
            list_widths: ListWidths::default(),
            screenshots: Screenshots::default(),
            mcp: Mcp::default(),
            downloads: Downloads::default(),
            toolbox: Toolbox::default(),
            voice_input: VoiceInput::default(),
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
        settings.voice_input = settings.voice_input.clone().sanitized();
        settings
    }

    pub fn save(&self) {
        let Some(path) = file() else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // The AI access settings go with their tag (src/mcp/access.rs checks it at the start).
        // Before the database is open (the language is saved earlier at the start), the tag
        // there already stays.
        let mut written = self.clone();
        written.mcp.seal = crate::database::tag(&self.mcp.sealed_text()).unwrap_or_else(|| self.mcp.seal.clone());
        let text = match toml::to_string_pretty(&written) {
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
            mcp: Mcp { enabled: true, scope: McpScope::Chosen, chats: vec![-1001, 42], sending: McpSending::Drafts, account: 1001, seal: String::new() },
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
    fn voice_input_starts_with_one_profile_for_any_language_through_openai() {
        let read: Settings = toml::from_str("theme = \"terminal\"\n").expect("read");
        assert_eq!(read.voice_input, VoiceInput::default());
        let profile = read.voice_input.current();
        assert_eq!((profile.language.as_str(), profile.format.as_str(), profile.tidy_format.as_str()), ("auto", "openai-realtime", "openai"));
        assert!(profile.tidy && profile.tidy_same_key && profile.url.is_empty() && profile.model.is_empty());
        assert!(!read.voice_input.transcribe_messages);
        // Two profiles, the second in use, written and read back.
        let mut voice_input = VoiceInput::default();
        voice_input.profiles.push(VoiceProfile {
            language: "zh-cn".into(),
            format: "doubao".into(),
            tidy_format: "anthropic".into(),
            tidy_same_key: false,
            ..VoiceProfile::default()
        });
        voice_input.language = "zh-cn".into();
        assert_eq!(voice_input.current().format, "doubao");
        voice_input.current_mut().model = "volc.seedasr.sauc.concurrent".into();
        let settings = Settings { voice_input, ..Settings::default() };
        let text = toml::to_string_pretty(&settings).expect("TOML");
        assert!(text.contains("[[voice_input.profiles]]\nlanguage = \"zh-cn\"\nformat = \"doubao\"\nurl = \"\"\nmodel = \"volc.seedasr.sauc.concurrent\"\nkeywords = \"\"\n"), "{text}");
        // The live model's delay is written only when set by hand.
        assert!(!text.contains("delay"), "{text}");
        let read: Settings = toml::from_str(&text).expect("read back");
        assert_eq!(read.voice_input, settings.voice_input);
        assert_eq!(read.voice_input.clone().sanitized(), settings.voice_input);
    }

    #[test]
    fn a_voice_input_file_of_0_7_becomes_the_first_profile() {
        // One language chosen is the main one; models other than 0.7's defaults are kept.
        let read: Settings = toml::from_str(
            "[voice_input]\nenabled = true\nlanguages = [\"zh-cn\"]\ntidy = false\nmodel = \"gpt-live-transcribe\"\ntidy_model = \"gpt-5-mini\"\n",
        )
        .expect("read");
        let voice_input = read.voice_input.sanitized();
        assert_eq!((voice_input.enabled, voice_input.language.as_str()), (Some(true), "zh-cn"));
        assert_eq!(
            voice_input.profiles,
            vec![VoiceProfile { language: "zh-cn".into(), tidy: false, tidy_model: "gpt-5-mini".into(), ..VoiceProfile::default() }]
        );
        // Several languages, as 0.7 let the user choose them: any language, through OpenAI as before.
        let read: Settings = toml::from_str(
            "[voice_input]\nenabled = true\nlanguages = [\n    \"en\",\n    \"zh-cn\",\n    \"zh-tw\",\n]\ntidy = true\nmodel = \"gpt-live-transcribe\"\ntidy_model = \"gpt-5-nano\"\n",
        )
        .expect("read");
        let voice_input = read.voice_input.sanitized();
        assert_eq!(voice_input.language, "auto");
        assert_eq!(voice_input.profiles, vec![VoiceProfile::default()]);
        assert_eq!((voice_input.languages.clone(), voice_input.model.clone()), (None, None));
        // Written again, none of 0.7's keys is left.
        let text = toml::to_string_pretty(&Settings { voice_input, ..Settings::default() }).expect("TOML");
        let section = text.split("[voice_input]").nth(1).unwrap_or("");
        assert!(!section.contains("languages") && !section.contains("gpt-live-transcribe") && !section.contains("gpt-5-nano"), "{text}");
    }

    #[test]
    fn voice_profiles_edited_by_hand_are_put_right() {
        let read: Settings = toml::from_str(
            "[voice_input]\nlanguage = \"xx\"\n\n[[voice_input.profiles]]\nlanguage = \"ja\"\nformat = \"whisper\"\ntidy_format = \"cohere\"\nurl = \" https://example.com/v1 \"\n\n[[voice_input.profiles]]\nlanguage = \"ja\"\nformat = \"doubao\"\n\n[[voice_input.profiles]]\nlanguage = \"klingon\"\n",
        )
        .expect("read");
        let voice_input = read.voice_input.sanitized();
        assert_eq!(voice_input.profiles.len(), 1);
        let profile = &voice_input.profiles[0];
        assert_eq!((profile.format.as_str(), profile.tidy_format.as_str(), profile.url.as_str()), ("openai-realtime", "openai", "https://example.com/v1"));
        assert_eq!(voice_input.language, "ja");
        // No profile at all is one for any language.
        let read: Settings = toml::from_str("[voice_input]\nenabled = false\n").expect("read");
        assert_eq!(read.voice_input.sanitized().profiles, vec![VoiceProfile::default()]);
    }

    #[test]
    fn voice_inputs_switch_is_written_once_chosen_and_absent_in_an_older_file() {
        // 0.7.0 wrote no switch: none is read, and the start decides by the key.
        let read: Settings = toml::from_str("[voice_input]
tidy = true
").expect("read");
        assert_eq!(read.voice_input.enabled, None);
        let text = toml::to_string_pretty(&Settings::default()).expect("TOML");
        let section: Vec<&str> = text.split("[voice_input]").nth(1).unwrap_or("").lines().take_while(|line| !line.starts_with('[')).collect();
        assert!(!section.iter().any(|line| line.starts_with("enabled")), "{text}");
        for on in [true, false] {
            let settings = Settings { voice_input: VoiceInput { enabled: Some(on), ..VoiceInput::default() }, ..Settings::default() };
            let text = toml::to_string_pretty(&settings).expect("TOML");
            assert!(text.contains(&format!("[voice_input]\nenabled = {on}\n")), "{text}");
            let read: Settings = toml::from_str(&text).expect("read back");
            assert_eq!(read.voice_input.enabled, Some(on));
        }
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
