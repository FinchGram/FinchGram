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
    /// The music page (src/music/, docs/music.md): the volume, shuffle and repeat, whether the queue
    /// column shows, the source looked at last, how much the cache may hold. The sources with
    /// their passwords are the database's (database::music_sources), never here.
    pub music: Music,
    /// The toolbox (src/toolbox.rs): which tools are pinned to the activity bar, in the
    /// toolbox's order; one list for the three themes.
    pub toolbox: Toolbox,
    /// Voice input, a tool of the toolbox's AI category (src/speech/, docs/voice-input.md): the AI
    /// vendors the user added, and the three AI features that use them (voice input, tidying its
    /// words, transcribing voice messages). The vendors' keys are credentials in the database
    /// (database::credential), never here.
    pub voice_input: VoiceInput,
}

/// Voice input's own settings (src/speech/, docs/voice-input.md): the AI vendors the user added,
/// each a name, an API format and an address (its key is the database's); then three AI features,
/// each with its own switch, its AI vendor and its model, chosen by the user and by nobody else:
/// nothing is chosen until they choose (the maintainer's word of 2026-10-26). Voice input also has
/// the main languages, which only tell its vendor what is spoken, how an OpenAI-compatible vendor
/// hears, and the words to know. The keys are credentials in the database
/// (`voice_input.connection.<id>`, named when the vendors were called connections), never here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceInput {
    /// Voice input itself, the tool's switch: on, the sparkle shows in the composer. Off until the
    /// user turns it on. A file written before the switch existed (0.7.0) has none, and voice input
    /// is on there when a key was kept (src/speech/ decides at the start).
    pub enabled: Option<bool>,
    /// The main language dictated now, one of `main_languages`.
    pub language: String,
    /// The main languages the user speaks, in the order they were added: "auto" (any language) or
    /// one of the UI's languages by the codes speech services take ("en", "zh-cn", "zh-tw", "ja",
    /// …). They only tell the vendor what is spoken; words of other languages mixed in are kept as
    /// spoken. While the user speaks, a click in the strip takes the next one.
    pub main_languages: Vec<String>,
    /// The AI vendors, in the order they were added (0.7.3's connections).
    #[serde(alias = "connections")]
    pub vendors: Vec<AiVendor>,
    /// Voice input's AI vendor, by id ("" for none chosen), and its model ("" for none).
    pub vendor: String,
    /// How an OpenAI-compatible vendor hears: "realtime" (words as the user speaks) or "upload" (the
    /// recording sent at the end, which most relays and servers of one's own take); "" for none
    /// chosen.
    pub mode: String,
    pub model: String,
    /// Names and terms the user says often, separated by commas or lines: each speech service takes
    /// them as hints (OpenAI's keywords, Qwen's corpus, Doubao's hot words, an upload's prompt), so
    /// that they are heard right.
    pub keywords: String,
    /// OpenAI's live model only: how long it waits to be sure of a word, "minimal" … "xhigh"; empty
    /// for FinchGram's own choice. Only in this file.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub delay: String,
    /// Tidying, the second feature: a text model takes the filler words, the repeats and the false
    /// starts out of what voice input heard, and sets the punctuation. Its switch, AI vendor and
    /// model.
    pub tidy: bool,
    pub tidy_vendor: String,
    pub tidy_model: String,
    /// Transcribing voice messages, the third feature: Transcribe under a voice message sends it to
    /// this AI vendor instead of Telegram. That hands Telegram's content, someone's voice, to a
    /// service of the user's choosing (docs/voice-input.md), so it is off until the user turns it on.
    pub transcribe_messages: bool,
    #[serde(alias = "messages_connection")]
    pub messages_vendor: String,
    pub messages_model: String,
    /// 0.7.0's and 0.7.1's languages, which only they wrote, and always: read once, never written
    /// again. Their `tidy`, `model` and `tidy_model` were voice input's own, as these are now.
    #[serde(skip_serializing)]
    pub languages: Option<Vec<String>>,
    /// 0.7.2's and 0.7.3's settings of each main language: read once, never written again.
    #[serde(skip_serializing)]
    pub profiles: Vec<VoiceProfile>,
}

/// An AI vendor the user added: a name of theirs, the format its API speaks ("openai" for OpenAI's
/// and every service that speaks it, "anthropic", "gemini", "dashscope" for Alibaba Cloud Model
/// Studio, "doubao" for Volcengine's Doubao speech; "" until the user chooses one), and its address
/// (empty: the format's own, src/speech/service.rs). A vendor serves every feature its format can
/// do: an OpenAI-compatible one hears and writes alike.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiVendor {
    /// Its id: the features name it by this, and its key is `voice_input.connection.<id>`.
    pub id: String,
    pub label: String,
    pub format: String,
    pub url: String,
    /// Where its key still is, made from 0.7's settings: the database's name of the credential it
    /// takes at the next start (src/speech/), until then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_from: Option<String>,
}

/// 0.7.2's and 0.7.3's settings of one main language (read once, never written): the connection
/// that heard it and the one that tidied it, with their models (empty: what the format used);
/// 0.7.2 named the services' formats, addresses and keys instead of connections.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct VoiceProfile {
    pub language: String,
    pub connection: String,
    pub mode: String,
    pub model: String,
    pub keywords: String,
    pub delay: String,
    pub tidy: bool,
    pub tidy_connection: String,
    pub tidy_model: String,
    pub format: Option<String>,
    pub url: Option<String>,
    pub tidy_format: Option<String>,
    pub tidy_url: Option<String>,
    pub tidy_same_key: Option<bool>,
}

/// What 0.7.2 and 0.7.3 took for a field their files lack.
impl Default for VoiceProfile {
    fn default() -> Self {
        VoiceProfile {
            language: "auto".into(),
            connection: String::new(),
            mode: "realtime".into(),
            model: String::new(),
            keywords: String::new(),
            delay: String::new(),
            tidy: true,
            tidy_connection: String::new(),
            tidy_model: String::new(),
            format: None,
            url: None,
            tidy_format: None,
            tidy_url: None,
            tidy_same_key: None,
        }
    }
}

impl Default for VoiceInput {
    fn default() -> Self {
        VoiceInput {
            enabled: None,
            language: "auto".into(),
            main_languages: vec!["auto".into()],
            vendors: Vec::new(),
            vendor: String::new(),
            mode: String::new(),
            model: String::new(),
            keywords: String::new(),
            delay: String::new(),
            tidy: false,
            tidy_vendor: String::new(),
            tidy_model: String::new(),
            transcribe_messages: false,
            messages_vendor: String::new(),
            messages_model: String::new(),
            languages: None,
            profiles: Vec::new(),
        }
    }
}

/// The speech model 0.7.3 used when the user had named none (the format's own then): a file of
/// before keeps hearing with it, now named, since nothing is chosen for the user any more.
fn former_speech_model(format: &str, mode: &str) -> &'static str {
    match (format, mode) {
        ("openai", "upload") => "gpt-transcribe",
        ("dashscope", _) => "qwen3-asr-flash-realtime",
        ("doubao", _) => "volc.seedasr.sauc.duration",
        _ => "gpt-live-transcribe",
    }
}

/// The text model 0.7.3 used when the user had named none, likewise.
fn former_text_model(format: &str) -> &'static str {
    match format {
        "anthropic" => "claude-haiku-5-5",
        "gemini" => "gemini-3.5-flash-lite",
        "dashscope" => "qwen-flash",
        _ => "gpt-5-mini",
    }
}

impl VoiceInput {
    /// The vendor of id `id`; none for "" or one that is not there.
    pub fn find_vendor(&self, id: &str) -> Option<&AiVendor> {
        if id.is_empty() {
            return None;
        }
        self.vendors.iter().find(|vendor| vendor.id == id)
    }

    /// An id no vendor has.
    pub fn next_vendor_id(&self) -> String {
        let highest = self.vendors.iter().filter_map(|vendor| vendor.id.parse::<u32>().ok()).max().unwrap_or(0);
        (highest + 1).to_string()
    }

    /// The vendor of `format` at `url` whose key comes from `key_from`, made when there is none
    /// (0.7.2's files).
    fn vendor_for(&mut self, format: &str, url: &str, key_from: &str) -> String {
        let found = self.vendors.iter().find(|vendor| vendor.format == format && vendor.url == url && vendor.key_from.as_deref() == Some(key_from));
        if let Some(found) = found {
            return found.id.clone();
        }
        let id = self.next_vendor_id();
        self.vendors.push(AiVendor {
            id: id.clone(),
            label: crate::speech::service::default_label(format, url),
            format: format.to_string(),
            url: url.to_string(),
            key_from: Some(key_from.to_string()),
        });
        id
    }

    /// The format of the vendor of id `id`, "" for none.
    fn format_of(&self, id: &str) -> String {
        self.find_vendor(id).map(|vendor| vendor.format.clone()).unwrap_or_default()
    }

    /// 0.7.0's and 0.7.1's file: one OpenAI key for everything, voice input's own settings at the top.
    fn adopt_0_7_1(&mut self, languages: Vec<String>) {
        // One language chosen is the main one; several, or none, are any language.
        let language = match languages.as_slice() {
            [only] => only.trim().to_string(),
            _ => "auto".to_string(),
        };
        // 0.7.2 moved the key under the main language's name, even where it wrote no settings.
        self.vendors = vec![AiVendor {
            id: "1".into(),
            label: "OpenAI".into(),
            format: "openai".into(),
            url: String::new(),
            key_from: Some(format!("voice_input.{language}.speech")),
        }];
        self.vendor = "1".into();
        self.mode = "realtime".into();
        if self.model.trim().is_empty() {
            self.model = former_speech_model("openai", "realtime").into();
        }
        self.tidy_vendor = "1".into();
        // 0.7.1's own tidying model became the format's, gpt-5-mini, in 0.7.2.
        if self.tidy_model.trim().is_empty() || self.tidy_model.trim() == "gpt-5-nano" {
            self.tidy_model = former_text_model("openai").into();
        }
        self.main_languages = vec![language.clone()];
        self.language = language;
    }

    /// 0.7.2's and 0.7.3's files: a set of choices for each main language. 0.7.2's formats,
    /// addresses and keys become vendors, one for each address and key (the start merges those whose
    /// keys turn out the same); then the features take the choices of the main language in use,
    /// with the models it used named, and the words to know of every language.
    fn adopt_profiles(&mut self, mut profiles: Vec<VoiceProfile>) {
        for profile in &mut profiles {
            let Some(format) = profile.format.take() else { continue };
            let language = profile.language.clone();
            let speech_key = format!("voice_input.{language}.speech");
            let (family, mode) = match format.as_str() {
                "openai-transcriptions" => ("openai", "upload"),
                "qwen-realtime" => ("dashscope", "realtime"),
                "doubao" => ("doubao", "realtime"),
                _ => ("openai", "realtime"),
            };
            profile.connection = self.vendor_for(family, profile.url.take().unwrap_or_default().trim(), &speech_key);
            profile.mode = mode.to_string();
            let tidy_family = match profile.tidy_format.take().as_deref() {
                Some("anthropic") => "anthropic",
                Some("gemini") => "gemini",
                _ => "openai",
            };
            let tidy_key = if profile.tidy_same_key.take().unwrap_or(true) { speech_key } else { format!("voice_input.{language}.tidy") };
            profile.tidy_connection = self.vendor_for(tidy_family, profile.tidy_url.take().unwrap_or_default().trim(), &tidy_key);
        }
        let Some(current) = profiles.iter().find(|profile| profile.language == self.language).or(profiles.first()).cloned() else { return };
        self.main_languages = profiles.iter().map(|profile| profile.language.clone()).collect();
        self.language = current.language.clone();
        self.vendor = current.connection.clone();
        let format = self.format_of(&self.vendor);
        self.mode = if format == "openai" { current.mode.clone() } else { String::new() };
        self.model = match current.model.trim() {
            _ if self.vendor.is_empty() => String::new(),
            "" => former_speech_model(&format, &current.mode).into(),
            model => model.to_string(),
        };
        let mut words: Vec<String> = Vec::new();
        for profile in std::iter::once(&current).chain(profiles.iter()) {
            for word in crate::speech::dictation::keywords(&profile.keywords) {
                if !words.contains(&word) {
                    words.push(word);
                }
            }
        }
        self.keywords = words.join(", ");
        self.delay = current.delay.clone();
        self.tidy_vendor = current.tidy_connection.clone();
        self.tidy = current.tidy && !self.tidy_vendor.is_empty();
        self.tidy_model = match current.tidy_model.trim() {
            _ if self.tidy_vendor.is_empty() => String::new(),
            "" => former_text_model(&self.format_of(&self.tidy_vendor)).into(),
            model => model.to_string(),
        };
        // 0.7.3 sent a voice message, when Telegram could not, to its own connection when one was
        // chosen, else to the main language in use's speech recognition: now always to its own
        // vendor, which uploads the file when it is OpenAI-compatible.
        if self.messages_vendor.is_empty() && self.transcribe_messages && !self.vendor.is_empty() {
            self.messages_vendor = self.vendor.clone();
            self.messages_model = if format == "openai" && self.mode != "upload" { former_speech_model("openai", "upload").into() } else { self.model.clone() };
        } else if !self.messages_vendor.is_empty() && self.messages_model.trim().is_empty() {
            self.messages_model = former_speech_model(&self.format_of(&self.messages_vendor), "upload").into();
        }
    }

    /// Older files become vendors and features; what makes no sense (edited by hand) is put right:
    /// unknown formats and languages, ids twice, a feature's vendor that is not there or cannot do
    /// it, a way of hearing that is none.
    fn sanitized(mut self) -> VoiceInput {
        let profiles = std::mem::take(&mut self.profiles);
        let languages = self.languages.take();
        if !profiles.is_empty() {
            self.adopt_profiles(profiles);
        } else if let Some(languages) = languages
            && self.vendors.is_empty()
        {
            self.adopt_0_7_1(languages);
        }
        // The vendors: known formats ("" until chosen), ids of their own.
        let mut ids: Vec<String> = Vec::new();
        for (index, vendor) in self.vendors.iter_mut().enumerate() {
            if !vendor.format.is_empty() && !crate::speech::service::CONNECTION_FORMATS.contains(&vendor.format.as_str()) {
                vendor.format.clear();
            }
            vendor.label = vendor.label.trim().to_string();
            vendor.url = vendor.url.trim().to_string();
            if vendor.id.trim().is_empty() || ids.contains(&vendor.id) {
                let highest = ids.iter().filter_map(|id| id.parse::<u32>().ok()).max().unwrap_or(0).max(index as u32);
                vendor.id = (highest + 1).to_string();
            }
            ids.push(vendor.id.clone());
        }
        // Each feature's vendor is one that is there and can do it; its model goes with it.
        use crate::speech::service::ConnectionFormat;
        let can = |voice_input: &VoiceInput, id: &str, ability: fn(ConnectionFormat) -> bool| ConnectionFormat::known(&voice_input.format_of(id)).is_some_and(ability);
        if !can(&self, &self.vendor, ConnectionFormat::hears) {
            self.vendor.clear();
        }
        if !can(&self, &self.tidy_vendor, ConnectionFormat::writes) {
            self.tidy_vendor.clear();
        }
        if !can(&self, &self.messages_vendor, ConnectionFormat::hears) {
            self.messages_vendor.clear();
        }
        if self.format_of(&self.vendor) != "openai" || !matches!(self.mode.as_str(), "realtime" | "upload") {
            self.mode.clear();
        }
        for (vendor, model) in [(&self.vendor, &mut self.model), (&self.tidy_vendor, &mut self.tidy_model), (&self.messages_vendor, &mut self.messages_model)] {
            *model = if vendor.is_empty() { String::new() } else { model.trim().to_string() };
        }
        self.keywords = self.keywords.trim().to_string();
        self.delay = self.delay.trim().to_string();
        // The main languages: known ones, each once; any language when none is left.
        let mut seen: Vec<String> = Vec::new();
        for language in std::mem::take(&mut self.main_languages) {
            let language = language.trim().to_string();
            if (language == "auto" || crate::speech::is_language(&language)) && !seen.contains(&language) {
                seen.push(language);
            }
        }
        if seen.is_empty() {
            seen.push("auto".into());
        }
        if !seen.contains(&self.language) {
            self.language = seen[0].clone();
        }
        self.main_languages = seen;
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

/// The music page's own settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Music {
    /// 0 … 100.
    pub volume: u32,
    pub shuffle: bool,
    /// "off", "all" or "one".
    pub repeat: String,
    /// The queue column of the page shows; the user can hide it.
    pub queue_shown: bool,
    /// The source looked at last, by name; "" for the first.
    pub source: String,
    /// How many megabytes of songs the cache (~/Documents/FinchGram/music_cache) keeps before the
    /// oldest go; 0 for no limit.
    pub cache_limit_mb: u64,
}

impl Default for Music {
    fn default() -> Self {
        Music { volume: 100, shuffle: false, repeat: "off".to_string(), queue_shown: true, source: String::new(), cache_limit_mb: 2048 }
    }
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
            music: Music::default(),
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
    fn voice_input_starts_with_nothing_chosen_and_reads_back_what_the_user_chose() {
        let read: Settings = toml::from_str("theme = \"terminal\"\n").expect("read");
        let voice_input = read.voice_input.sanitized();
        assert_eq!(voice_input, VoiceInput::default());
        assert!(voice_input.vendors.is_empty() && voice_input.vendor.is_empty() && voice_input.model.is_empty() && voice_input.mode.is_empty());
        assert!(!voice_input.tidy && voice_input.tidy_vendor.is_empty() && voice_input.tidy_model.is_empty());
        assert!(!voice_input.transcribe_messages && voice_input.messages_vendor.is_empty());
        assert_eq!((voice_input.language.as_str(), voice_input.main_languages.clone()), ("auto", vec!["auto".to_string()]));
        // A new vendor has no format until the user chooses one.
        assert_eq!(AiVendor::default().format, "");
        // Two vendors: Doubao hears Chinese, the OpenAI-compatible one tidies and transcribes.
        let voice_input = VoiceInput {
            main_languages: vec!["zh-cn".into(), "en".into()],
            language: "zh-cn".into(),
            vendors: vec![
                AiVendor { id: "1".into(), label: "OpenAI".into(), format: "openai".into(), ..AiVendor::default() },
                AiVendor { id: "2".into(), label: "豆包语音".into(), format: "doubao".into(), ..AiVendor::default() },
            ],
            vendor: "2".into(),
            model: "volc.seedasr.sauc.duration".into(),
            keywords: "陈思远, FinchGram".into(),
            tidy: true,
            tidy_vendor: "1".into(),
            tidy_model: "gpt-5-mini".into(),
            transcribe_messages: true,
            messages_vendor: "1".into(),
            messages_model: "gpt-transcribe".into(),
            ..VoiceInput::default()
        };
        assert_eq!(voice_input.next_vendor_id(), "3");
        assert_eq!(voice_input.find_vendor("2").map(|vendor| vendor.format.as_str()), Some("doubao"));
        let settings = Settings { voice_input, ..Settings::default() };
        let text = toml::to_string_pretty(&settings).expect("TOML");
        assert!(text.contains("vendor = \"2\"\nmode = \"\"\nmodel = \"volc.seedasr.sauc.duration\"\n"), "{text}");
        assert!(text.contains("[[voice_input.vendors]]\nid = \"2\"\nlabel = \"豆包语音\"\nformat = \"doubao\"\nurl = \"\"\n"), "{text}");
        // Nothing of the older shapes, and the live model's delay only when set by hand.
        let section = text.split("[voice_input]").nth(1).unwrap_or("");
        for old in ["connections", "profiles", "\nlanguages =", "key_from", "delay", "messages_connection"] {
            assert!(!section.contains(old), "{old} in {text}");
        }
        let read: Settings = toml::from_str(&text).expect("read back");
        assert_eq!(read.voice_input, settings.voice_input);
        assert_eq!(read.voice_input.clone().sanitized(), settings.voice_input);
    }

    #[test]
    fn a_voice_input_file_of_0_7_1_becomes_an_openai_vendor_for_voice_input_and_tidying() {
        // One language chosen is the main one; the models are kept, named.
        let read: Settings = toml::from_str(
            "[voice_input]\nenabled = true\nlanguages = [\"zh-cn\"]\ntidy = false\nmodel = \"gpt-live-transcribe\"\ntidy_model = \"gpt-5-mini\"\n",
        )
        .expect("read");
        let voice_input = read.voice_input.sanitized();
        assert_eq!((voice_input.enabled, voice_input.language.as_str()), (Some(true), "zh-cn"));
        assert_eq!(voice_input.main_languages, vec!["zh-cn".to_string()]);
        assert_eq!(
            voice_input.vendors,
            vec![AiVendor { id: "1".into(), label: "OpenAI".into(), format: "openai".into(), key_from: Some("voice_input.zh-cn.speech".into()), ..AiVendor::default() }]
        );
        assert_eq!((voice_input.vendor.as_str(), voice_input.mode.as_str(), voice_input.model.as_str()), ("1", "realtime", "gpt-live-transcribe"));
        assert_eq!((voice_input.tidy, voice_input.tidy_vendor.as_str(), voice_input.tidy_model.as_str()), (false, "1", "gpt-5-mini"));
        assert!(!voice_input.transcribe_messages && voice_input.messages_vendor.is_empty());
        // Several languages, as 0.7 let the user choose them: any language; 0.7's own tidying model
        // is the one 0.7.2 took in its place.
        let read: Settings = toml::from_str(
            "[voice_input]\nlanguages = [\n    \"en\",\n    \"zh-cn\",\n    \"zh-tw\",\n]\ntidy = true\nmodel = \"gpt-live-transcribe\"\ntidy_model = \"gpt-5-nano\"\n",
        )
        .expect("read");
        let voice_input = read.voice_input.sanitized();
        assert_eq!((voice_input.language.as_str(), voice_input.tidy, voice_input.tidy_model.as_str()), ("auto", true, "gpt-5-mini"));
        assert_eq!(voice_input.vendors[0].key_from.as_deref(), Some("voice_input.auto.speech"));
        // Written again, none of 0.7's keys is left, but the key's way to its vendor is.
        let text = toml::to_string_pretty(&Settings { voice_input, ..Settings::default() }).expect("TOML");
        let section = text.split("[voice_input]").nth(1).unwrap_or("");
        assert!(!section.contains("\nlanguages =") && !section.contains("gpt-5-nano"), "{text}");
        assert!(section.contains("key_from = \"voice_input.auto.speech\""), "{text}");
    }

    #[test]
    fn a_voice_input_file_of_0_7_2_becomes_vendors_and_the_main_languages_choices() {
        let read: Settings = toml::from_str(
            "[voice_input]\nenabled = true\nlanguage = \"zh-cn\"\ntranscribe_messages = true\n\n\
             [[voice_input.profiles]]\nlanguage = \"auto\"\nformat = \"openai-realtime\"\nurl = \"\"\nmodel = \"\"\nkeywords = \"Blue Bottle\"\ntidy = true\ntidy_format = \"openai\"\ntidy_url = \"\"\ntidy_model = \"\"\ntidy_same_key = true\n\n\
             [[voice_input.profiles]]\nlanguage = \"zh-cn\"\nformat = \"doubao\"\nurl = \"\"\nmodel = \"volc.seedasr.sauc.concurrent\"\nkeywords = \"陈思远\"\ntidy = true\ntidy_format = \"anthropic\"\ntidy_url = \"https://relay.example.com\"\ntidy_model = \"\"\ntidy_same_key = false\n\n\
             [[voice_input.profiles]]\nlanguage = \"en\"\nformat = \"openai-transcriptions\"\nurl = \"https://api.groq.com/openai/v1\"\nmodel = \"whisper-large-v3\"\nkeywords = \"\"\ntidy = false\ntidy_format = \"openai\"\ntidy_url = \"\"\ntidy_model = \"\"\ntidy_same_key = true\n",
        )
        .expect("read");
        let voice_input = read.voice_input.sanitized();
        let vendors: Vec<(&str, &str, &str, &str, &str)> = voice_input
            .vendors
            .iter()
            .map(|vendor| (vendor.id.as_str(), vendor.label.as_str(), vendor.format.as_str(), vendor.url.as_str(), vendor.key_from.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(
            vendors,
            vec![
                ("1", "OpenAI", "openai", "", "voice_input.auto.speech"),
                ("2", "Doubao", "doubao", "", "voice_input.zh-cn.speech"),
                ("3", "relay.example.com", "anthropic", "https://relay.example.com", "voice_input.zh-cn.tidy"),
                ("4", "api.groq.com", "openai", "https://api.groq.com/openai/v1", "voice_input.en.speech"),
                ("5", "OpenAI", "openai", "", "voice_input.en.speech"),
            ]
        );
        // The main language in use's choices, its models named; every language's words to know.
        assert_eq!(voice_input.main_languages, vec!["auto".to_string(), "zh-cn".into(), "en".into()]);
        assert_eq!((voice_input.vendor.as_str(), voice_input.mode.as_str(), voice_input.model.as_str()), ("2", "", "volc.seedasr.sauc.concurrent"));
        assert_eq!(voice_input.keywords, "陈思远, Blue Bottle");
        assert_eq!((voice_input.tidy, voice_input.tidy_vendor.as_str(), voice_input.tidy_model.as_str()), (true, "3", "claude-haiku-5-5"));
        // Voice messages went to the speech recognition in use: its vendor now, chosen.
        assert_eq!((voice_input.messages_vendor.as_str(), voice_input.messages_model.as_str()), ("2", "volc.seedasr.sauc.concurrent"));
        assert!(voice_input.profiles.is_empty());
    }

    #[test]
    fn a_voice_input_file_of_0_7_3_keeps_its_vendors_and_the_main_languages_choices() {
        let read: Settings = toml::from_str(
            "[voice_input]\nenabled = true\nlanguage = \"en\"\ntranscribe_messages = false\nmessages_connection = \"\"\nmessages_model = \"\"\n\n\
             [[voice_input.connections]]\nid = \"1\"\nlabel = \"OpenAI\"\nformat = \"openai\"\nurl = \"\"\n\n\
             [[voice_input.connections]]\nid = \"2\"\nlabel = \"豆包语音\"\nformat = \"doubao\"\nurl = \"\"\n\n\
             [[voice_input.profiles]]\nlanguage = \"zh-cn\"\nconnection = \"2\"\nmode = \"realtime\"\nmodel = \"\"\nkeywords = \"\"\ntidy = true\ntidy_connection = \"1\"\ntidy_model = \"\"\n\n\
             [[voice_input.profiles]]\nlanguage = \"en\"\nconnection = \"1\"\nmode = \"upload\"\nmodel = \"\"\nkeywords = \"FinchGram\"\ntidy = true\ntidy_connection = \"\"\ntidy_model = \"\"\n",
        )
        .expect("read");
        let voice_input = read.voice_input.sanitized();
        assert_eq!(voice_input.vendors.iter().map(|vendor| vendor.label.as_str()).collect::<Vec<_>>(), vec!["OpenAI", "豆包语音"]);
        assert_eq!((voice_input.language.as_str(), voice_input.main_languages.clone()), ("en", vec!["zh-cn".to_string(), "en".into()]));
        assert_eq!((voice_input.vendor.as_str(), voice_input.mode.as_str(), voice_input.model.as_str()), ("1", "upload", "gpt-transcribe"));
        // Tidying was on with no connection chosen: off, nothing chosen.
        assert_eq!((voice_input.tidy, voice_input.tidy_vendor.as_str(), voice_input.tidy_model.as_str()), (false, "", ""));
        assert!(!voice_input.transcribe_messages && voice_input.messages_vendor.is_empty() && voice_input.messages_model.is_empty());
    }

    #[test]
    fn voice_settings_edited_by_hand_are_put_right() {
        let read: Settings = toml::from_str(
            "[voice_input]\nlanguage = \"xx\"\nmain_languages = [\"ja\", \"ja\", \"klingon\"]\nvendor = \"2\"\nmode = \"sideways\"\nmodel = \" gemini-x \"\ntidy = true\ntidy_vendor = \"7\"\ntidy_model = \"m\"\nmessages_vendor = \"1\"\nmessages_model = \"whisper-1\"\n\n\
             [[voice_input.vendors]]\nid = \"1\"\nlabel = \" Mine \"\nformat = \"whisper\"\nurl = \" https://example.com/v1 \"\n\n\
             [[voice_input.vendors]]\nid = \"1\"\nlabel = \"Twin\"\nformat = \"gemini\"\n",
        )
        .expect("read");
        let voice_input = read.voice_input.sanitized();
        let first = &voice_input.vendors[0];
        assert_eq!((first.id.as_str(), first.label.as_str(), first.format.as_str(), first.url.as_str()), ("1", "Mine", "", "https://example.com/v1"));
        assert_eq!(voice_input.vendors[1].id, "2");
        // Gemini cannot hear, vendor 7 is not there, vendor 1 has no format: nothing chosen.
        assert_eq!((voice_input.vendor.as_str(), voice_input.mode.as_str(), voice_input.model.as_str()), ("", "", ""));
        assert_eq!((voice_input.tidy, voice_input.tidy_vendor.as_str(), voice_input.tidy_model.as_str()), (true, "", ""));
        assert_eq!((voice_input.messages_vendor.as_str(), voice_input.messages_model.as_str()), ("", ""));
        assert_eq!((voice_input.language.as_str(), voice_input.main_languages.clone()), ("ja", vec!["ja".to_string()]));
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
