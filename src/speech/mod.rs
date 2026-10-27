//! Voice input (docs/voice-input.md): the user presses the sparkle in the composer (in Terminal,
//! types /voice) and speaks; the words come back from the speech service the user chose, under
//! their own key, and show in the strip above the composer; when the user is done (the sparkle
//! again, ⏎, or letting go of it after holding it), a text model tidies them and they go into the
//! composer, for the user to send. Esc drops them. The sound goes from the microphone to that
//! service and nowhere else, and FinchGram keeps no recording. (The microphone in the composer is
//! voice messages', src/telegram/voice.rs.)
//!
//! The user adds AI vendors: a name of theirs, an API format, an address and a key, any vendor,
//! relay or server of their own that speaks one of the formats (service.rs). Three AI features use
//! them, each with its own switch, its vendor and its model, chosen by the user from lists (the
//! vendor's own list of models) and by nobody else: nothing is chosen until the user chooses (the
//! maintainer's word of 2026-10-26). Voice input hears the user (OpenAI's realtime transcription or
//! its upload, Alibaba Cloud's Qwen, ByteDance's Doubao: realtime.rs, upload.rs, doubao.rs); tidying
//! puts its words in order (Chat Completions, Anthropic's or Google's format: tidy.rs); and voice
//! messages are transcribed by their own vendor (messages.rs). The main language, chosen in the
//! panel or in the strip while speaking, only tells the vendor what is spoken; words of other
//! languages mixed in are kept as spoken.
//!
//! Voice input is a tool of the toolbox's AI category (src/toolbox.rs, ui/pages/voice-input.slint),
//! off until the user turns it on there: off, the sparkle is not in the composer, and Terminal's
//! /voice, or the button of a stop for want of a vendor, a model or a key, opens its panel, where
//! the vendors and the features are set.
//!
//! One dictation at a time, its state here on the UI thread. The microphone's samples come from
//! one of the system's threads (src/platform/microphone.rs); the session runs on a thread of its
//! own ("voice", `dictation::dictate`), which reports back through the event loop. The keys are
//! the database's (`database::credential`, `voice_input.connection.<id>`, named when the vendors
//! were called connections), never in settings.toml.

mod audio;
pub mod dictation;
mod doubao;
pub mod messages;
mod realtime;
pub mod service;
mod tidy;
mod upload;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::database::{credential, remove_credential, save_credential};
use crate::platform::{self, MicrophoneAccess};
use crate::settings::{AiVendor, Settings, VoiceInput};
use crate::{Conversation, KeyCheck, MainWindow, ModelsState, Recorder, Toolbox, Voice, VoiceItem, VoiceLanguage, VoiceLanguageRow, VoiceProblem, VoiceState, VoiceVendorRow};
#[cfg(test)]
pub use audio::wav;
pub use audio::wav_samples;
use dictation::{Input, Options, Report};
use service::{Ability, AddressProblem, ConnectionFormat, Endpoint, FailureKind, KeyAnswer, ListProblem, Speech, SpeechFormat, Tidy};

/// 0.7.0's and 0.7.1's one key, OpenAI's.
const OLD_KEY: &str = "openai_api_key";
/// Held longer than this, the microphone finishes when let go; a shorter press is a click.
const HOLD: Duration = Duration::from_millis(600);
/// The longest dictation: it finishes by itself then, so that a microphone left open does not go
/// on costing the user.
const LONGEST: Duration = Duration::from_secs(5 * 60);
/// How often the bars and the clock are drawn again.
const TICK: Duration = Duration::from_millis(50);
/// The languages offered as main languages, in the order of the UI's language list: its code
/// there, and the one the speech services take.
const LANGUAGES: [(&str, &str); 11] = [
    ("en", "en"),
    ("zh_Hans", "zh-cn"),
    ("zh_Hant", "zh-tw"),
    ("ja", "ja"),
    ("ko", "ko"),
    ("fr", "fr"),
    ("de", "de"),
    ("es", "es"),
    ("pt", "pt"),
    ("ru", "ru"),
    ("ar", "ar"),
];
/// Chips per line in the panel.
const PER_LINE: usize = 4;

/// Whether `code` is one of the main languages offered (settings.rs keeps only those).
pub fn is_language(code: &str) -> bool {
    LANGUAGES.iter().any(|(_, known)| *known == code)
}

/// The database's name for a vendor's key (named when the vendors were called connections).
fn key_name(id: &str) -> String {
    format!("voice_input.connection.{id}")
}

/// A vendor's key; none for a vendor that is not there, or has none.
fn key_of(id: &str) -> Option<String> {
    if id.is_empty() { None } else { credential(&key_name(id)) }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DICTATION: RefCell<Option<Dictation>> = const { RefCell::new(None) };
    /// Counted up with every dictation and every cancel: what an older one's thread still says is
    /// not heard.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    /// When the microphone was pressed to start the dictation, while it is still held.
    static PRESSED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Counted up with every check of a vendor, so that only the last one's answer shows.
    static CHECKS: Cell<u64> = const { Cell::new(0) };
    /// The vendor open in the panel for editing, by id; "" for none.
    static OPEN: RefCell<String> = const { RefCell::new(String::new()) };
    /// The models each vendor offers, by vendor id and ability, as it listed them this run.
    static MODELS: RefCell<HashMap<(String, Ability), Models>> = RefCell::new(HashMap::new());
    /// The lists asked for and not answered yet, by the number of the request: only the last
    /// request's answer is kept.
    static PENDING: RefCell<HashMap<(String, Ability), u64>> = RefCell::new(HashMap::new());
    static LISTS: Cell<u64> = const { Cell::new(0) };
    /// What is typed in each feature's search over its models.
    static SEARCH: RefCell<HashMap<Feature, String>> = RefCell::new(HashMap::new());
}

/// What a vendor said of its models this run.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Models {
    Loading,
    Listed(Vec<String>),
    /// Its own address without its key: not asked.
    NeedKey,
    Refused,
    Failed(String),
}

/// The three AI features, as the panel names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Feature {
    /// Voice input: the user's speech.
    Speech,
    /// Tidying what voice input heard.
    Tidy,
    /// Transcribing voice messages.
    Messages,
}

impl Feature {
    const ALL: [Feature; 3] = [Feature::Speech, Feature::Tidy, Feature::Messages];

    fn of(name: &str) -> Option<Feature> {
        match name {
            "speech" => Some(Feature::Speech),
            "tidy" => Some(Feature::Tidy),
            "messages" => Some(Feature::Messages),
            _ => None,
        }
    }

    fn ability(self) -> Ability {
        match self {
            Feature::Tidy => Ability::Write,
            Feature::Speech | Feature::Messages => Ability::Hear,
        }
    }

    /// Whether a vendor of `format` can do it.
    fn can(self, format: ConnectionFormat) -> bool {
        match self.ability() {
            Ability::Hear => format.hears(),
            Ability::Write => format.writes(),
        }
    }

    fn vendor(self, voice_input: &VoiceInput) -> &str {
        match self {
            Feature::Speech => &voice_input.vendor,
            Feature::Tidy => &voice_input.tidy_vendor,
            Feature::Messages => &voice_input.messages_vendor,
        }
    }

    fn model(self, voice_input: &VoiceInput) -> &str {
        match self {
            Feature::Speech => &voice_input.model,
            Feature::Tidy => &voice_input.tidy_model,
            Feature::Messages => &voice_input.messages_model,
        }
    }

    /// Its vendor and model, both chosen anew (a vendor's models are its own).
    fn choose(self, voice_input: &mut VoiceInput, vendor: &str, model: &str) {
        let (chosen, its_model) = match self {
            Feature::Speech => (&mut voice_input.vendor, &mut voice_input.model),
            Feature::Tidy => (&mut voice_input.tidy_vendor, &mut voice_input.tidy_model),
            Feature::Messages => (&mut voice_input.messages_vendor, &mut voice_input.messages_model),
        };
        *chosen = vendor.to_string();
        *its_model = model.to_string();
    }
}

struct App {
    ui: slint::Weak<MainWindow>,
    settings: Rc<RefCell<Settings>>,
}

/// The dictation going on.
struct Dictation {
    /// To the dictation's thread: Finish or Cancel (the microphone sends the sound).
    inputs: Sender<Input>,
    /// Listening while it is here; gone once the user is done.
    microphone: Option<platform::Microphone>,
    /// How loud the microphone last was, an f32's bits.
    level: Arc<AtomicU32>,
    started: Instant,
    _ticks: slint::Timer,
}

pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    adopt_keys(&mut settings.borrow_mut());
    // A settings file from before the switch (0.7.0) has none: voice input is on there when a key
    // was kept. The choice is written with the next save of the settings.
    let on = {
        let mut settings = settings.borrow_mut();
        let has_key = settings.voice_input.vendors.iter().any(|vendor| key_of(&vendor.id).is_some());
        *settings.voice_input.enabled.get_or_insert(has_key)
    };
    let voice = ui.global::<Voice>();
    voice.set_on(on);
    APP.with(|app| *app.borrow_mut() = Some(App { ui: ui.as_weak(), settings }));
    show_panel(ui);
    voice.on_press(press);
    voice.on_release(release);
    voice.on_stop(finish);
    voice.on_cancel(cancel);
    voice.on_act(act);
    voice.on_next_language(next_language);
    voice.on_set_on(set_on);
    voice.on_choose_language(|code| choose_language(&code));
    voice.on_start_adding(|| with_ui(|ui| ui.global::<Voice>().set_adding(true)));
    voice.on_cancel_adding(|| with_ui(|ui| ui.global::<Voice>().set_adding(false)));
    voice.on_add_language(|code| add_language(&code));
    voice.on_remove_language(remove_language);
    voice.on_add_vendor(add_vendor);
    voice.on_toggle_vendor(|id| toggle_vendor(&id));
    voice.on_edit_vendor_label(|text| edit_vendor(VendorField::Label, &text));
    voice.on_edit_vendor_url(|text| edit_vendor(VendorField::Url, &text));
    voice.on_set_vendor_format(|name| set_vendor_format(&name));
    voice.on_check_vendor(check_vendor);
    voice.on_save_vendor_key(|text| save_key(&text));
    voice.on_remove_vendor_key(remove_key);
    voice.on_remove_vendor(remove_vendor);
    voice.on_open_key_page(open_key_page);
    voice.on_choose_vendor(|which, id| choose_vendor(&which, &id));
    voice.on_choose_model(|which, model| choose_model(&which, &model));
    voice.on_search_models(|which, text| search_models(&which, &text));
    voice.on_set_mode(|mode| set_mode(&mode));
    voice.on_edit_keywords(|text| edit_keywords(&text));
    voice.on_set_tidy(set_tidy);
    voice.on_set_transcribe_messages(set_transcribe_messages);
}

/// 0.7's keys into the vendors its settings became (settings.rs), and the vendors that turn out to
/// be the same (one format, one address, one key: a key copied to two languages in 0.7.2) made one.
/// Once: a vendor's `key_from` goes with its key.
fn adopt_keys(settings: &mut Settings) {
    let voice_input = &mut settings.voice_input;
    if !voice_input.vendors.iter().any(|vendor| vendor.key_from.is_some()) {
        return;
    }
    let mut adopted: Vec<String> = Vec::new();
    for vendor in &mut voice_input.vendors {
        let Some(from) = vendor.key_from.take() else { continue };
        // 0.7.2 moved 0.7.1's key under the first profile's name at its start, even when its
        // settings were not written again: the key is under either name. 0.7.1's was OpenAI's.
        let (key, source) = match credential(&from) {
            Some(key) => (Some(key), from),
            None if vendor.format == "openai" => (credential(OLD_KEY), OLD_KEY.to_string()),
            None => continue,
        };
        let Some(key) = key else { continue };
        let name = key_name(&vendor.id);
        if credential(&name).is_none()
            && let Err(err) = save_credential(&name, &key)
        {
            eprintln!("voice input: cannot move a key of 0.7: {err}");
            vendor.key_from = Some(source);
            continue;
        }
        adopted.push(source);
    }
    // The same format, address and key twice: the first one stays, and takes the other's place.
    // One whose key could not move yet is left as it is, for the next start.
    let mut keys: Vec<Option<String>> = voice_input.vendors.iter().map(|vendor| key_of(&vendor.id)).collect();
    let mut index = 0;
    while index < voice_input.vendors.len() {
        let twin = (0..index).find(|&earlier| {
            let (a, b) = (&voice_input.vendors[earlier], &voice_input.vendors[index]);
            a.format == b.format && a.url == b.url && keys[earlier] == keys[index] && a.key_from.is_none() && b.key_from.is_none()
        });
        match twin {
            Some(earlier) => {
                let (gone, kept) = (voice_input.vendors[index].id.clone(), voice_input.vendors[earlier].id.clone());
                repoint(voice_input, &gone, &kept);
                let _ = remove_credential(&key_name(&gone));
                voice_input.vendors.remove(index);
                keys.remove(index);
            }
            None => index += 1,
        }
    }
    // An old key goes once every connection that took it has its copy.
    for source in adopted {
        if !voice_input.vendors.iter().any(|vendor| vendor.key_from.as_deref() == Some(source.as_str())) {
            let _ = remove_credential(&source);
        }
    }
    settings.save();
}

/// The features that used vendor `gone` use `kept` from now on, the same vendor merged into it.
fn repoint(voice_input: &mut VoiceInput, gone: &str, kept: &str) {
    for vendor in [&mut voice_input.vendor, &mut voice_input.tidy_vendor, &mut voice_input.messages_vendor] {
        if vendor == gone {
            *vendor = kept.to_string();
        }
    }
}

fn with_ui(work: impl FnOnce(&MainWindow)) {
    let ui = APP.with(|app| app.borrow().as_ref().and_then(|app| app.ui.upgrade()));
    if let Some(ui) = ui {
        work(&ui);
    }
}

fn with_settings<R>(work: impl FnOnce(&mut Settings) -> R) -> Option<R> {
    let settings = APP.with(|app| app.borrow().as_ref().map(|app| app.settings.clone()))?;
    let mut settings = settings.borrow_mut();
    Some(work(&mut settings))
}

fn state() -> VoiceState {
    let mut state = VoiceState::Idle;
    with_ui(|ui| state = ui.global::<Voice>().get_state());
    state
}

/// A vendor as the services take it; none when it is not there or has no format yet, Err when its
/// address is not one.
fn endpoint_of(voice_input: &VoiceInput, id: &str) -> Option<Result<Endpoint, AddressProblem>> {
    let vendor = voice_input.find_vendor(id)?;
    ConnectionFormat::known(&vendor.format)?;
    Some(Endpoint::new(&vendor.format, &vendor.url, &vendor.label))
}

/// Voice input's speech service: its vendor, how it hears, its model; what is not chosen yet stops
/// it, and the strip says which.
fn speech_of(voice_input: &VoiceInput) -> Result<Speech, VoiceProblem> {
    let format = voice_input.find_vendor(&voice_input.vendor).and_then(|vendor| ConnectionFormat::known(&vendor.format)).filter(|format| format.hears()).ok_or(VoiceProblem::NoService)?;
    if format == ConnectionFormat::OpenAi && voice_input.mode.is_empty() {
        return Err(VoiceProblem::NoMode);
    }
    if voice_input.model.trim().is_empty() {
        return Err(VoiceProblem::NoModel);
    }
    let endpoint = endpoint_of(voice_input, &voice_input.vendor).ok_or(VoiceProblem::NoService)?.map_err(|_| VoiceProblem::BadAddress)?;
    Speech::new(endpoint, voice_input.mode == "upload", &voice_input.model).ok_or(VoiceProblem::NoModel)
}

/// Tidying's text model and its key, when it is on and all of it is chosen; none for the words as
/// heard (the panel says what is missing).
fn tidy_of(voice_input: &VoiceInput) -> Option<(Tidy, Option<String>)> {
    if !voice_input.tidy {
        return None;
    }
    let endpoint = endpoint_of(voice_input, &voice_input.tidy_vendor)?.ok()?;
    let tidy = Tidy::new(endpoint, &voice_input.tidy_model)?;
    let key = key_of(&voice_input.tidy_vendor);
    if key.is_none() && tidy.official() { None } else { Some((tidy, key)) }
}

/// A dictation's settings: Err with the problem that stops it.
fn dictation_options(voice_input: &VoiceInput) -> Result<Options, VoiceProblem> {
    let speech = speech_of(voice_input)?;
    let speech_key = key_of(&voice_input.vendor);
    // The vendor's own address wants a key; a server of the user's own may not.
    if speech_key.is_none() && speech.official() {
        return Err(VoiceProblem::NoKey);
    }
    Ok(Options {
        speech,
        speech_key,
        language: voice_input.language.clone(),
        keywords: dictation::keywords(&voice_input.keywords),
        delay: voice_input.delay.clone(),
        tidy: tidy_of(voice_input),
        recorded: false,
    })
}

/// A voice message's settings (src/speech/messages.rs): its own vendor and model, which upload the
/// file when the vendor is OpenAI-compatible; the language is found from the message itself, and
/// nothing is tidied (the words are someone else's).
fn messages_options_of(voice_input: &VoiceInput) -> Result<Options, VoiceProblem> {
    let format = voice_input.find_vendor(&voice_input.messages_vendor).and_then(|vendor| ConnectionFormat::known(&vendor.format)).filter(|format| format.hears()).ok_or(VoiceProblem::NoService)?;
    if voice_input.messages_model.trim().is_empty() {
        return Err(VoiceProblem::NoModel);
    }
    let endpoint = endpoint_of(voice_input, &voice_input.messages_vendor).ok_or(VoiceProblem::NoService)?.map_err(|_| VoiceProblem::BadAddress)?;
    let speech = Speech::new(endpoint, format == ConnectionFormat::OpenAi, &voice_input.messages_model).ok_or(VoiceProblem::NoModel)?;
    let speech_key = key_of(&voice_input.messages_vendor);
    if speech_key.is_none() && speech.official() {
        return Err(VoiceProblem::NoKey);
    }
    Ok(Options { speech, speech_key, language: "auto".into(), keywords: Vec::new(), delay: String::new(), tidy: None, recorded: true })
}

/// The voice messages' own settings, from the settings in use.
pub fn message_options() -> Result<Options, VoiceProblem> {
    with_settings(|settings| messages_options_of(&settings.voice_input)).unwrap_or(Err(VoiceProblem::NoService))
}

/// Whether voice messages are transcribed by the user's own vendor, not Telegram.
pub fn transcribes_messages() -> bool {
    with_settings(|settings| settings.voice_input.transcribe_messages).unwrap_or(false)
}

// ---- a dictation -------------------------------------------------------------------------------

/// The sparkle pressed: a dictation starts, or the one listening finishes (a second click). Not
/// while a voice message records: the microphone is its. Off (Terminal's /voice: the sparkle is
/// not there), its panel in the toolbox comes instead, where it is turned on.
fn press() {
    let (mut recording, mut on) = (false, false);
    with_ui(|ui| {
        recording = ui.global::<Recorder>().get_busy();
        on = ui.global::<Voice>().get_on();
    });
    if recording {
        return;
    }
    if !on {
        PRESSED.set(None);
        with_ui(open_panel);
        return;
    }
    match state() {
        VoiceState::Idle | VoiceState::Failed => {
            PRESSED.set(Some(Instant::now()));
            start();
        }
        VoiceState::Listening => {
            PRESSED.set(None);
            finish();
        }
        VoiceState::Finishing | VoiceState::Tidying => {}
    }
}

/// The microphone let go: after holding it while speaking, that is the end of it.
fn release() {
    let Some(pressed) = PRESSED.take() else { return };
    let listening = DICTATION.with(|dictation| dictation.borrow().as_ref().is_some_and(|dictation| dictation.microphone.is_some()));
    if pressed.elapsed() >= HOLD && listening {
        finish();
    }
}

fn start() {
    let generation = GENERATION.get() + 1;
    GENERATION.set(generation);
    end_dictation();
    show_strip_service();
    let options = match with_settings(|settings| dictation_options(&settings.voice_input)).unwrap_or(Err(VoiceProblem::NoService)) {
        Ok(options) => options,
        Err(problem) => {
            fail(problem, "");
            return;
        }
    };
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        voice.set_problem(VoiceProblem::None);
        voice.set_detail("".into());
        voice.set_words("".into());
        voice.set_seconds(0);
        voice.set_level(0.0);
        voice.set_connecting(true);
        voice.set_state(VoiceState::Listening);
    });
    match platform::microphone_access() {
        MicrophoneAccess::Allowed => begin(generation, options),
        MicrophoneAccess::Denied => fail(VoiceProblem::MicrophoneDenied, ""),
        MicrophoneAccess::Undetermined => {
            // The system's question takes the press: what comes after it is a click.
            PRESSED.set(None);
            platform::ask_for_microphone(move |allowed| {
                let _ = slint::invoke_from_event_loop(move || {
                    if GENERATION.get() != generation {
                        return;
                    }
                    if allowed {
                        begin(generation, options);
                    } else {
                        fail(VoiceProblem::MicrophoneDenied, "");
                    }
                });
            });
        }
    }
}

/// The strip's words: who hears the user, the main language in use, whether words come while the
/// user speaks.
fn show_strip_service() {
    let found = with_settings(|settings| {
        let voice_input = &settings.voice_input;
        let speech = speech_of(voice_input).ok();
        let vendor = voice_input.find_vendor(&voice_input.vendor).map(label_of).unwrap_or_default();
        (speech, vendor, voice_input.language.clone(), voice_input.main_languages.len())
    });
    let Some((speech, vendor, language, count)) = found else { return };
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        voice.set_service(speech.as_ref().map(Speech::shown).unwrap_or(vendor).into());
        voice.set_live(speech.as_ref().is_some_and(|speech| live(speech, &language)));
        voice.set_language_name(language_name(&language).into());
        voice.set_can_switch(count > 1);
    });
}

/// Whether words show while the user speaks: not when the recording goes up at the end, nor with
/// Doubao's languages other than Chinese and English.
fn live(speech: &Speech, language: &str) -> bool {
    match speech.format {
        SpeechFormat::OpenAiTranscriptions => false,
        SpeechFormat::Doubao => matches!(language, "auto" | "zh-cn" | "zh-tw" | "en"),
        SpeechFormat::OpenAiRealtime => speech.model.starts_with("gpt-live") || speech.model.contains("realtime") || !speech.official(),
        SpeechFormat::QwenRealtime => true,
    }
}

/// A main language's name, in itself; "any language" in the UI's.
fn language_name(code: &str) -> String {
    match LANGUAGES.iter().find(|(_, known)| *known == code) {
        Some((ui_code, _)) => crate::i18n::native_name(ui_code).to_string(),
        None => String::new(),
    }
}

/// The microphone on, and the session's thread started.
fn begin(generation: u64, options: Options) {
    let (inputs, received) = mpsc::channel::<Input>();
    let level = Arc::new(AtomicU32::new(0));
    let microphone = {
        let inputs = inputs.clone();
        let level = level.clone();
        platform::Microphone::start(move |samples, rate| {
            level.store(audio::level(samples).to_bits(), Ordering::Relaxed);
            let _ = inputs.send(Input::Audio { samples: samples.to_vec(), rate });
        })
    };
    let microphone = match microphone {
        Ok(microphone) => microphone,
        Err(err) => {
            fail(VoiceProblem::NoMicrophone, &err);
            return;
        }
    };
    let spawned = std::thread::Builder::new().name("voice".into()).spawn(move || {
        dictation::dictate(&options, &received, &|report| {
            let _ = slint::invoke_from_event_loop(move || reported(generation, report));
        });
    });
    if let Err(err) = spawned {
        fail(VoiceProblem::Service, &err.to_string());
        return;
    }
    let ticks = slint::Timer::default();
    ticks.start(slint::TimerMode::Repeated, TICK, tick);
    DICTATION.with(|dictation| {
        *dictation.borrow_mut() = Some(Dictation { inputs, microphone: Some(microphone), level, started: Instant::now(), _ticks: ticks });
    });
}

/// The bars and the clock, and the longest dictation's end.
fn tick() {
    let now = DICTATION.with(|dictation| {
        dictation.borrow().as_ref().filter(|dictation| dictation.microphone.is_some()).map(|dictation| (f32::from_bits(dictation.level.load(Ordering::Relaxed)), dictation.started.elapsed()))
    });
    let Some((level, elapsed)) = now else { return };
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        // The bars rise at once and fall a little slower.
        let shown = voice.get_level();
        voice.set_level(if level >= shown { level } else { shown * 0.7 + level * 0.3 });
        voice.set_seconds(elapsed.as_secs() as i32);
    });
    if elapsed >= LONGEST {
        finish();
    }
}

/// The user is done: the microphone stops, and the last words are waited for.
fn finish() {
    let finishing = DICTATION.with(|dictation| {
        let mut dictation = dictation.borrow_mut();
        let dictation = dictation.as_mut()?;
        let microphone = dictation.microphone.take()?;
        let _ = dictation.inputs.send(Input::Finish);
        Some(microphone)
    });
    match finishing {
        // Dropped out of the borrow: the engine stops.
        Some(microphone) => {
            drop(microphone);
            with_ui(|ui| {
                let voice = ui.global::<Voice>();
                voice.set_state(VoiceState::Finishing);
                voice.set_level(0.0);
            });
        }
        // Still waiting for the system's question, or the microphone never came: nothing to wait for.
        None if state() == VoiceState::Listening => cancel(),
        None => {}
    }
}

/// The words are not wanted (Esc, ×), or the reason has been read.
fn cancel() {
    GENERATION.set(GENERATION.get() + 1);
    PRESSED.set(None);
    end_dictation();
    with_ui(reset);
}

/// The strip's language, clicked: the next main language, and a dictation that listens starts
/// again with it (what was heard in the other language is dropped).
fn next_language() {
    let changed = with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let count = voice_input.main_languages.len();
        if count < 2 {
            return false;
        }
        let index = voice_input.main_languages.iter().position(|language| *language == voice_input.language).unwrap_or(0);
        voice_input.language = voice_input.main_languages[(index + 1) % count].clone();
        settings.save();
        true
    })
    .unwrap_or(false);
    if !changed {
        return;
    }
    with_ui(show_panel);
    if matches!(state(), VoiceState::Listening | VoiceState::Failed) {
        PRESSED.set(None);
        start();
    } else {
        show_strip_service();
    }
}

/// The problem's button: where it can be put right.
fn act() {
    let mut problem = VoiceProblem::None;
    with_ui(|ui| problem = ui.global::<Voice>().get_problem());
    cancel();
    match problem {
        VoiceProblem::NoKey | VoiceProblem::NoService | VoiceProblem::NoMode | VoiceProblem::NoModel | VoiceProblem::RefusedKey | VoiceProblem::BadAddress => with_ui(open_panel),
        VoiceProblem::MicrophoneDenied => platform::open_microphone_settings(),
        _ => {}
    }
}

/// What the dictation's thread said; nothing from an older one.
fn reported(generation: u64, report: Report) {
    if generation != GENERATION.get() {
        return;
    }
    match report {
        Report::Ready => with_ui(|ui| ui.global::<Voice>().set_connecting(false)),
        Report::Words(words) => with_ui(|ui| ui.global::<Voice>().set_words(words.into())),
        Report::Tidying => with_ui(|ui| ui.global::<Voice>().set_state(VoiceState::Tidying)),
        Report::Done(words) => {
            end_dictation();
            if words.trim().is_empty() {
                fail(VoiceProblem::NothingHeard, "");
                return;
            }
            with_ui(|ui| {
                insert(ui, &words);
                reset(ui);
            });
        }
        Report::Failed(failure) => fail(problem_of(failure.kind), &failure.detail),
    }
}

/// What the user is told of a failure.
pub fn problem_of(kind: FailureKind) -> VoiceProblem {
    match kind {
        FailureKind::RefusedKey => VoiceProblem::RefusedKey,
        FailureKind::Quota => VoiceProblem::Quota,
        FailureKind::Network => VoiceProblem::Network,
        FailureKind::Service => VoiceProblem::Service,
    }
}

/// The dictation stopped, and why: the strip says so until the user closes it or starts again.
fn fail(problem: VoiceProblem, detail: &str) {
    PRESSED.set(None);
    end_dictation();
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        voice.set_problem(problem);
        voice.set_detail(detail.into());
        voice.set_connecting(false);
        voice.set_level(0.0);
        voice.set_state(VoiceState::Failed);
    });
}

/// The dictation's thread told to stop, the microphone and the clock with it.
fn end_dictation() {
    let ended = DICTATION.with(|dictation| dictation.borrow_mut().take());
    if let Some(ended) = ended {
        let _ = ended.inputs.send(Input::Cancel);
    }
}

fn reset(ui: &MainWindow) {
    let voice = ui.global::<Voice>();
    voice.set_state(VoiceState::Idle);
    voice.set_problem(VoiceProblem::None);
    voice.set_detail("".into());
    voice.set_words("".into());
    voice.set_level(0.0);
    voice.set_seconds(0);
    voice.set_connecting(false);
}

/// The words at the end of what the composer holds, and the composer told, as if typed.
fn insert(ui: &MainWindow, words: &str) {
    let conversation = ui.global::<Conversation>();
    let joined = join_words(&conversation.get_draft(), words.trim());
    conversation.set_draft(joined.as_str().into());
    conversation.invoke_edited(joined.into());
    let voice = ui.global::<Voice>();
    voice.set_insertions(voice.get_insertions() + 1);
}

/// `words` after `before`, as such text is written: a space between words of the scripts that
/// have spaces; none next to Chinese or Japanese, before punctuation that closes, or after
/// punctuation that opens.
pub fn join_words(before: &str, words: &str) -> String {
    let (Some(last), Some(first)) = (before.chars().last(), words.chars().next()) else {
        return format!("{before}{words}");
    };
    let close = last.is_whitespace() || first.is_whitespace() || spaceless(last) || spaceless(first) || closes(first) || opens(last);
    if close { format!("{before}{words}") } else { format!("{before} {words}") }
}

/// Chinese characters, kana, and the full-width forms and punctuation that go with them.
fn spaceless(character: char) -> bool {
    matches!(character as u32, 0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

fn closes(character: char) -> bool {
    matches!(character, ',' | '.' | '!' | '?' | ';' | ':' | ')' | ']' | '}' | '…' | '»' | '”' | '’' | '،' | '؟' | '؛')
}

fn opens(character: char) -> bool {
    matches!(character, '(' | '[' | '{' | '«' | '“' | '‘' | '¿' | '¡')
}

// ---- the toolbox's Voice input panel -----------------------------------------------------------

/// The toolbox, open at voice input's panel.
fn open_panel(ui: &MainWindow) {
    ui.global::<Toolbox>().invoke_open_tool("voice-input".into());
}

/// Voice input's switch, in its panel or on its card: on, the sparkle shows in the composer; off,
/// it goes, and a dictation going on is dropped.
fn set_on(on: bool) {
    with_settings(|settings| {
        settings.voice_input.enabled = Some(on);
        settings.save();
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_on(on);
        refresh(ui, false);
    });
    if !on && state() != VoiceState::Idle {
        cancel();
    }
}

/// The vendor open for editing, as the panel shows it.
struct OpenVendor {
    label: String,
    format: String,
    url: String,
    url_placeholder: String,
    url_problem: i32,
    has_key: bool,
    key_hint: String,
    official: bool,
    shown: String,
}

/// A feature as the panel shows it: the vendors that can do it, the one chosen, its models (as the
/// vendor listed them, narrowed by the search), the one chosen, and what the vendor said of them.
struct FeatureView {
    vendors: Vec<VoiceItem>,
    vendor: String,
    vendor_label: String,
    format: String,
    models: Vec<VoiceItem>,
    model: String,
    state: ModelsState,
    detail: String,
}

/// What the panel shows of the settings, with the keys' letters (the database's, or the
/// screenshots' own).
struct PanelView {
    languages: Vec<VoiceLanguage>,
    addable: Vec<VoiceLanguage>,
    language_name: String,
    vendors: Vec<VoiceVendorRow>,
    open: Option<OpenVendor>,
    speech: FeatureView,
    mode: String,
    keywords: String,
    tidy_on: bool,
    tidy: FeatureView,
    messages_on: bool,
    messages: FeatureView,
}

fn format_of(voice_input: &VoiceInput, id: &str) -> Option<ConnectionFormat> {
    voice_input.find_vendor(id).and_then(|vendor| ConnectionFormat::known(&vendor.format))
}

fn feature_view(voice_input: &VoiceInput, feature: Feature, models: &dyn Fn(&str, Ability) -> Option<Models>, search: &str) -> FeatureView {
    let vendors = voice_input
        .vendors
        .iter()
        .filter(|vendor| ConnectionFormat::known(&vendor.format).is_some_and(|format| feature.can(format)))
        .map(|vendor| VoiceItem { id: vendor.id.as_str().into(), label: label_of(vendor).into() })
        .collect();
    let id = feature.vendor(voice_input);
    let chosen = voice_input.find_vendor(id);
    let listed = chosen.and_then(|_| models(id, feature.ability()));
    let (state, detail) = match &listed {
        None => (ModelsState::None, String::new()),
        Some(Models::Loading) => (ModelsState::Loading, String::new()),
        Some(Models::Listed(list)) if list.is_empty() => (ModelsState::Empty, String::new()),
        Some(Models::Listed(_)) => (ModelsState::Listed, String::new()),
        Some(Models::NeedKey) => (ModelsState::NeedKey, String::new()),
        Some(Models::Refused) => (ModelsState::Refused, String::new()),
        Some(Models::Failed(detail)) => (ModelsState::Failed, detail.clone()),
    };
    let needle = search.trim().to_lowercase();
    let models = match &listed {
        Some(Models::Listed(list)) => list.iter().filter(|model| needle.is_empty() || model.to_lowercase().contains(&needle)).map(|model| VoiceItem { id: model.as_str().into(), label: model.as_str().into() }).collect(),
        _ => Vec::new(),
    };
    FeatureView {
        vendors,
        vendor: id.to_string(),
        vendor_label: chosen.map(label_of).unwrap_or_default(),
        format: chosen.map(|vendor| vendor.format.clone()).unwrap_or_default(),
        models,
        model: feature.model(voice_input).to_string(),
        state,
        detail,
    }
}

fn panel_view(voice_input: &VoiceInput, open: &str, key_hint: &dyn Fn(&str) -> Option<String>, models: &dyn Fn(&str, Ability) -> Option<Models>, search: &dyn Fn(Feature) -> String) -> PanelView {
    let chip = |code: &str| VoiceLanguage { code: code.into(), name: language_name(code).into(), on: code == voice_input.language };
    let languages = voice_input.main_languages.iter().map(|code| chip(code)).collect();
    let mut addable: Vec<VoiceLanguage> = Vec::new();
    let set_up = |code: &str| voice_input.main_languages.iter().any(|language| language == code);
    if !set_up("auto") {
        addable.push(chip("auto"));
    }
    addable.extend(LANGUAGES.iter().filter(|(_, code)| !set_up(code)).map(|(_, code)| chip(code)));
    let vendors = voice_input
        .vendors
        .iter()
        .map(|vendor| {
            let hint = key_hint(&vendor.id);
            // An address that is no address shows as typed, for the user to know the row.
            let host = match ConnectionFormat::known(&vendor.format) {
                Some(_) => Endpoint::new(&vendor.format, &vendor.url, "").map(|endpoint| endpoint.address.shown()).unwrap_or_else(|_| vendor.url.trim().to_string()),
                None => vendor.url.trim().to_string(),
            };
            VoiceVendorRow {
                id: vendor.id.as_str().into(),
                label: label_of(vendor).into(),
                format: vendor.format.as_str().into(),
                host: host.into(),
                has_key: hint.is_some(),
                key_hint: hint.unwrap_or_default().into(),
            }
        })
        .collect();
    let open = voice_input.find_vendor(open).map(|vendor| {
        let format = ConnectionFormat::known(&vendor.format);
        let endpoint = format.map(|_| Endpoint::new(&vendor.format, &vendor.url, &vendor.label));
        let hint = key_hint(&vendor.id);
        OpenVendor {
            label: vendor.label.clone(),
            format: vendor.format.clone(),
            url: vendor.url.clone(),
            url_placeholder: format.map(ConnectionFormat::default_url).unwrap_or_default().into(),
            url_problem: problem_number(endpoint.as_ref().and_then(|endpoint| endpoint.as_ref().err())),
            has_key: hint.is_some(),
            key_hint: hint.unwrap_or_default(),
            official: endpoint.as_ref().is_some_and(|endpoint| endpoint.as_ref().is_ok_and(Endpoint::official)),
            shown: label_of(vendor),
        }
    });
    PanelView {
        languages,
        addable,
        language_name: language_name(&voice_input.language),
        vendors,
        open,
        speech: feature_view(voice_input, Feature::Speech, models, &search(Feature::Speech)),
        mode: voice_input.mode.clone(),
        keywords: voice_input.keywords.clone(),
        tidy_on: voice_input.tidy,
        tidy: feature_view(voice_input, Feature::Tidy, models, &search(Feature::Tidy)),
        messages_on: voice_input.transcribe_messages,
        messages: feature_view(voice_input, Feature::Messages, models, &search(Feature::Messages)),
    }
}

/// A vendor's name: the user's, else its own for its own address or its host ("" while it has
/// neither a format nor an address: the panel calls it a new vendor).
fn label_of(vendor: &AiVendor) -> String {
    if vendor.label.trim().is_empty() { service::default_label(&vendor.format, &vendor.url) } else { vendor.label.trim().to_string() }
}

/// What is wrong with an address, as the panel numbers it: 0 nothing; 1 not an address; 2 a name
/// and password in it; 3 plain http to the internet.
fn problem_number(problem: Option<&AddressProblem>) -> i32 {
    match problem {
        None => 0,
        Some(AddressProblem::Malformed) => 1,
        Some(AddressProblem::Credentials) => 2,
        Some(AddressProblem::Unencrypted) => 3,
    }
}

/// The panel drawn again from the settings.
fn show_panel(ui: &MainWindow) {
    refresh(ui, true);
}

/// The panel drawn again; `typed`: the boxes the user types into too (not while they type in one).
fn refresh(ui: &MainWindow, typed: bool) {
    let open = OPEN.with(|open| open.borrow().clone());
    let known = |id: &str, ability: Ability| MODELS.with(|models| models.borrow().get(&(id.to_string(), ability)).cloned());
    let search = |feature: Feature| SEARCH.with(|search| search.borrow().get(&feature).cloned().unwrap_or_default());
    let Some(view) = with_settings(|settings| panel_view(&settings.voice_input, &open, &|id| credential(&key_name(id)).as_deref().map(hint), &known, &search)) else {
        return;
    };
    apply(ui, view, typed);
}

fn items(items: Vec<VoiceItem>) -> ModelRc<VoiceItem> {
    ModelRc::new(VecModel::from(items))
}

fn apply(ui: &MainWindow, view: PanelView, typed: bool) {
    let voice = ui.global::<Voice>();
    voice.set_can_remove(view.languages.len() > 1);
    voice.set_language_rows(lines(view.languages));
    voice.set_addable_rows(lines(view.addable));
    voice.set_language_name(view.language_name.into());
    voice.set_vendors(ModelRc::new(VecModel::from(view.vendors)));
    match view.open {
        Some(open) => {
            voice.set_open_vendor(OPEN.with(|id| id.borrow().clone()).into());
            if typed {
                voice.set_vendor_label(open.label.into());
                voice.set_vendor_url(open.url.into());
            }
            voice.set_vendor_format(open.format.into());
            voice.set_vendor_url_placeholder(open.url_placeholder.into());
            voice.set_vendor_url_problem(open.url_problem);
            voice.set_vendor_has_key(open.has_key);
            voice.set_vendor_key_hint(open.key_hint.into());
            voice.set_vendor_official(open.official);
            voice.set_vendor_shown(open.shown.into());
        }
        None => voice.set_open_vendor(SharedString::new()),
    }
    let speech = view.speech;
    voice.set_speech_vendors(items(speech.vendors));
    voice.set_speech_vendor(speech.vendor.into());
    voice.set_speech_vendor_label(speech.vendor_label.into());
    voice.set_speech_format(speech.format.into());
    voice.set_speech_mode(view.mode.into());
    voice.set_speech_models(items(speech.models));
    voice.set_speech_model(speech.model.into());
    voice.set_speech_models_state(speech.state);
    voice.set_speech_models_detail(speech.detail.into());
    voice.set_tidy(view.tidy_on);
    let tidy = view.tidy;
    voice.set_tidy_vendors(items(tidy.vendors));
    voice.set_tidy_vendor(tidy.vendor.into());
    voice.set_tidy_vendor_label(tidy.vendor_label.into());
    voice.set_tidy_models(items(tidy.models));
    voice.set_tidy_model(tidy.model.into());
    voice.set_tidy_models_state(tidy.state);
    voice.set_tidy_models_detail(tidy.detail.into());
    voice.set_transcribe_messages(view.messages_on);
    let messages = view.messages;
    voice.set_messages_vendors(items(messages.vendors));
    voice.set_messages_vendor(messages.vendor.into());
    voice.set_messages_vendor_label(messages.vendor_label.into());
    voice.set_messages_models(items(messages.models));
    voice.set_messages_model(messages.model.into());
    voice.set_messages_models_state(messages.state);
    voice.set_messages_models_detail(messages.detail.into());
    if typed {
        voice.set_keywords(view.keywords.into());
    }
}

/// Chips in lines of PER_LINE.
fn lines(chips: Vec<VoiceLanguage>) -> ModelRc<VoiceLanguageRow> {
    let lines: Vec<VoiceLanguageRow> = chips.chunks(PER_LINE).map(|line| VoiceLanguageRow { languages: ModelRc::new(VecModel::from(line.to_vec())) }).collect();
    ModelRc::new(VecModel::from(lines))
}

/// The panel for the screenshot test (src/screenshots.rs): `voice_input` as the settings, the
/// vendor `open` open, with keys whose letters are `hints` and models `models` (by vendor id).
#[cfg(test)]
pub fn show_for_pictures(ui: &MainWindow, voice_input: &VoiceInput, open: &str, hints: &[(&str, &str)], models: &[(&str, &[&str])]) {
    OPEN.with(|id| *id.borrow_mut() = open.to_string());
    let known = |id: &str, _: Ability| models.iter().find(|(known, _)| *known == id).map(|(_, list)| Models::Listed(list.iter().map(|model| model.to_string()).collect()));
    let view = panel_view(voice_input, open, &|id| hints.iter().find(|(known, _)| *known == id).map(|(_, hint)| hint.to_string()), &known, &|_| String::new());
    apply(ui, view, true);
    let speech = speech_of(voice_input).ok();
    let voice = ui.global::<Voice>();
    voice.set_service(speech.as_ref().map(Speech::shown).unwrap_or_default().into());
    voice.set_live(speech.as_ref().is_some_and(|speech| live(speech, &voice_input.language)));
    voice.set_can_switch(voice_input.main_languages.len() > 1);
}

// ---- the main languages ------------------------------------------------------------------------

/// A main language chosen in the panel: the one used from now on.
fn choose_language(code: &str) {
    with_settings(|settings| {
        if settings.voice_input.main_languages.iter().any(|language| language == code) {
            settings.voice_input.language = code.to_string();
            settings.save();
        }
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_adding(false);
        show_panel(ui);
    });
}

/// A main language added, and used from now on.
fn add_language(code: &str) {
    if code != "auto" && !is_language(code) {
        return;
    }
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        if !voice_input.main_languages.iter().any(|language| language == code) {
            voice_input.main_languages.push(code.to_string());
        }
        voice_input.language = code.to_string();
        settings.save();
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_adding(false);
        show_panel(ui);
    });
}

/// The main language in use removed; the first one left is used.
fn remove_language() {
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        if voice_input.main_languages.len() < 2 {
            return;
        }
        let language = voice_input.language.clone();
        voice_input.main_languages.retain(|other| *other != language);
        voice_input.language = voice_input.main_languages[0].clone();
        settings.save();
    });
    with_ui(show_panel);
}

// ---- the AI vendors ----------------------------------------------------------------------------

/// A new vendor, with nothing chosen: no name, no format, no address, open for the user to fill
/// in. No feature takes it until the user chooses it there.
fn add_vendor() {
    let id = with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let id = voice_input.next_vendor_id();
        voice_input.vendors.push(AiVendor { id: id.clone(), ..AiVendor::default() });
        settings.save();
        id
    });
    let Some(id) = id else { return };
    OPEN.with(|open| *open.borrow_mut() = id);
    with_ui(|ui| {
        ui.global::<Voice>().set_vendor_check(KeyCheck::None);
        show_panel(ui);
    });
}

/// A vendor's row clicked: open for editing, or closed.
fn toggle_vendor(id: &str) {
    OPEN.with(|open| {
        let mut open = open.borrow_mut();
        *open = if *open == id { String::new() } else { id.to_string() };
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_vendor_check(KeyCheck::None);
        show_panel(ui);
    });
}

fn open_id() -> String {
    OPEN.with(|open| open.borrow().clone())
}

enum VendorField {
    Label,
    Url,
}

/// The open vendor's name or address typed: kept as typed, and what is said of it shown. What the
/// vendor said of its models is forgotten with its old address (asked again once it is typed).
fn edit_vendor(field: VendorField, text: &str) {
    let id = open_id();
    with_settings(|settings| {
        if let Some(vendor) = settings.voice_input.vendors.iter_mut().find(|vendor| vendor.id == id) {
            match field {
                VendorField::Label => vendor.label = text.trim().to_string(),
                VendorField::Url => {
                    vendor.url = text.trim().to_string();
                    forget_models(&id);
                }
            }
            settings.save();
        }
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_vendor_check(KeyCheck::None);
        refresh(ui, false);
    });
}

/// The open vendor's format chosen: its name follows when it was the format's own; the features
/// it can no longer do are left without it, and the models chosen from it go (they were the old
/// format's).
fn set_vendor_format(name: &str) {
    let Some(format) = ConnectionFormat::known(name) else { return };
    let id = open_id();
    let label = with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let vendor = voice_input.vendors.iter_mut().find(|vendor| vendor.id == id)?;
        if vendor.format == name {
            return None;
        }
        if vendor.label.is_empty() || vendor.label == service::default_label(&vendor.format, &vendor.url) {
            vendor.label = service::default_label(name, &vendor.url);
        }
        vendor.format = name.to_string();
        let label = vendor.label.clone();
        for feature in Feature::ALL {
            if feature.vendor(voice_input) == id {
                let kept = if feature.can(format) { id.clone() } else { String::new() };
                feature.choose(voice_input, &kept, "");
            }
        }
        if voice_input.vendor == id && format != ConnectionFormat::OpenAi {
            voice_input.mode.clear();
        }
        settings.save();
        Some(label)
    })
    .flatten();
    let Some(label) = label else { return };
    forget_models(&id);
    with_ui(|ui| {
        refresh(ui, false);
        // The name may follow the format.
        ui.global::<Voice>().set_vendor_label(label.into());
    });
    check_vendor();
}

/// The key as the panel shows it: its first letters and its last four.
fn hint(key: &str) -> String {
    let characters: Vec<char> = key.chars().collect();
    if characters.len() <= 10 {
        return format!("…{}", characters[characters.len().saturating_sub(2)..].iter().collect::<String>());
    }
    let head: String = characters[..3].iter().collect();
    let tail: String = characters[characters.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// What could be a key of any vendor's: plain characters, no spaces, not too short.
fn looks_like_a_key(key: &str) -> bool {
    key.len() >= 8 && key.chars().all(|character| character.is_ascii_graphic())
}

fn save_key(text: &str) {
    let key = text.trim();
    CHECKS.set(CHECKS.get() + 1);
    let id = open_id();
    if id.is_empty() {
        return;
    }
    if !looks_like_a_key(key) {
        set_check(KeyCheck::Malformed, "");
        return;
    }
    if let Err(err) = save_credential(&key_name(&id), key) {
        set_check(KeyCheck::Failed, &err);
        return;
    }
    forget_models(&id);
    with_ui(|ui| refresh(ui, false));
    check_vendor();
}

fn remove_key() {
    CHECKS.set(CHECKS.get() + 1);
    let id = open_id();
    if !id.is_empty()
        && let Err(err) = remove_credential(&key_name(&id))
    {
        eprintln!("voice input: cannot remove the key: {err}");
    }
    forget_models(&id);
    with_ui(|ui| {
        refresh(ui, false);
        ui.global::<Voice>().set_vendor_check(KeyCheck::None);
    });
}

/// The open vendor removed, with its key; the features that used it have none, nor its models.
fn remove_vendor() {
    let id = open_id();
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        voice_input.vendors.retain(|vendor| vendor.id != id);
        for feature in Feature::ALL {
            if feature.vendor(voice_input) == id {
                feature.choose(voice_input, "", "");
            }
        }
        if voice_input.vendor.is_empty() {
            voice_input.mode.clear();
        }
        settings.save();
    });
    let _ = remove_credential(&key_name(&id));
    forget_models(&id);
    OPEN.with(|open| open.borrow_mut().clear());
    with_ui(show_panel);
}

fn set_check(check: KeyCheck, detail: &str) {
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        voice.set_vendor_check(check);
        voice.set_vendor_check_detail(detail.into());
    });
}

/// The open vendor asked, on a thread of its own, whether it answers at its address and takes the
/// key; the answer shows under it, and the features that use it ask for its models again. Nothing
/// is asked of an address that is not one, nor of a vendor's own address without a key.
fn check_vendor() {
    let id = open_id();
    want_models_of(&id);
    let Some(Some(Ok(endpoint))) = with_settings(|settings| endpoint_of(&settings.voice_input, &id)) else {
        return set_check(KeyCheck::None, "");
    };
    let key = key_of(&id);
    if key.is_none() && endpoint.official() {
        return set_check(KeyCheck::None, "");
    }
    let number = CHECKS.get() + 1;
    CHECKS.set(number);
    set_check(KeyCheck::Checking, "");
    let spawned = std::thread::Builder::new().name("voice-check".into()).spawn(move || {
        let answer = service::check_connection(&endpoint, key.as_deref());
        let _ = slint::invoke_from_event_loop(move || {
            if number != CHECKS.get() {
                return;
            }
            match answer {
                KeyAnswer::Works => set_check(KeyCheck::Works, ""),
                KeyAnswer::Refused => set_check(KeyCheck::Refused, ""),
                KeyAnswer::Failed(err) => set_check(KeyCheck::Failed, &err),
            }
        });
    });
    if spawned.is_err() {
        set_check(KeyCheck::None, "");
    }
}

/// The page of the open vendor's keys.
fn open_key_page() {
    let id = open_id();
    if let Some(Some(format)) = with_settings(|settings| format_of(&settings.voice_input, &id)) {
        platform::open_link(format.key_page());
    }
}

// ---- the vendors' models -----------------------------------------------------------------------

/// The models of vendor `id` for `ability` asked for, unless they are known or on their way: on a
/// thread of its own, and the panel shows them when they come. A vendor's own address without its
/// key is not asked: the panel says to add the key.
fn want_models(voice_input: &VoiceInput, id: &str, ability: Ability) {
    let slot = (id.to_string(), ability);
    if id.is_empty() || MODELS.with(|models| models.borrow().contains_key(&slot)) {
        return;
    }
    let Some(Ok(endpoint)) = endpoint_of(voice_input, id) else { return };
    let key = key_of(id);
    if key.is_none() && endpoint.official() {
        MODELS.with(|models| models.borrow_mut().insert(slot, Models::NeedKey));
        return;
    }
    let number = LISTS.get() + 1;
    LISTS.set(number);
    PENDING.with(|pending| pending.borrow_mut().insert(slot.clone(), number));
    MODELS.with(|models| models.borrow_mut().insert(slot.clone(), Models::Loading));
    let spawned = std::thread::Builder::new().name("voice-models".into()).spawn({
        let slot = slot.clone();
        move || {
            let answer = service::list_models(&endpoint, key.as_deref(), ability);
            let _ = slint::invoke_from_event_loop(move || {
                if PENDING.with(|pending| pending.borrow().get(&slot) != Some(&number)) {
                    return;
                }
                PENDING.with(|pending| pending.borrow_mut().remove(&slot));
                let listed = match answer {
                    Ok(list) => Models::Listed(list),
                    Err(ListProblem::Refused) => Models::Refused,
                    Err(ListProblem::Failed(detail)) => Models::Failed(detail),
                };
                MODELS.with(|models| models.borrow_mut().insert(slot, listed));
                with_ui(|ui| refresh(ui, false));
            });
        }
    });
    if let Err(err) = spawned {
        PENDING.with(|pending| pending.borrow_mut().remove(&slot));
        MODELS.with(|models| models.borrow_mut().insert(slot, Models::Failed(err.to_string())));
    }
}

/// The models of the features that use vendor `id` asked for again.
fn want_models_of(id: &str) {
    if id.is_empty() {
        return;
    }
    with_settings(|settings| {
        let voice_input = &settings.voice_input;
        for feature in Feature::ALL {
            if feature.vendor(voice_input) == id {
                want_models(voice_input, id, feature.ability());
            }
        }
    });
    with_ui(|ui| refresh(ui, false));
}

/// What vendor `id` said of its models forgotten: its address, key or format changed.
fn forget_models(id: &str) {
    MODELS.with(|models| models.borrow_mut().retain(|(vendor, _), _| vendor != id));
    PENDING.with(|pending| pending.borrow_mut().retain(|(vendor, _), _| vendor != id));
}

// ---- the features ------------------------------------------------------------------------------

/// A vendor chosen for a feature ("speech", "tidy", "messages") from its list: the model chosen
/// from the vendor before goes (a vendor's models are its own), and the new one's are asked for.
/// How voice input hears stays only from one OpenAI-compatible vendor to another.
fn choose_vendor(which: &str, id: &str) {
    let Some(feature) = Feature::of(which) else { return };
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let Some(format) = format_of(voice_input, id).filter(|format| feature.can(*format)) else { return };
        if feature.vendor(voice_input) != id {
            let before = format_of(voice_input, feature.vendor(voice_input));
            feature.choose(voice_input, id, "");
            if feature == Feature::Speech && !(format == ConnectionFormat::OpenAi && before == Some(ConnectionFormat::OpenAi)) {
                voice_input.mode.clear();
            }
            settings.save();
        }
        want_models(&settings.voice_input, id, feature.ability());
    });
    SEARCH.with(|search| search.borrow_mut().remove(&feature));
    with_ui(|ui| refresh(ui, false));
}

/// A model chosen for a feature from its vendor's list.
fn choose_model(which: &str, model: &str) {
    let Some(feature) = Feature::of(which) else { return };
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let vendor = feature.vendor(voice_input).to_string();
        if !vendor.is_empty() {
            feature.choose(voice_input, &vendor, model.trim());
            settings.save();
        }
    });
    SEARCH.with(|search| search.borrow_mut().remove(&feature));
    with_ui(|ui| refresh(ui, false));
}

/// What is typed in a feature's search over its models (opening the list types nothing): the list
/// narrowed to it, and the models asked for if they are not known yet.
fn search_models(which: &str, text: &str) {
    let Some(feature) = Feature::of(which) else { return };
    SEARCH.with(|search| search.borrow_mut().insert(feature, text.to_string()));
    with_settings(|settings| {
        let voice_input = &settings.voice_input;
        want_models(voice_input, feature.vendor(voice_input), feature.ability());
    });
    with_ui(|ui| refresh(ui, false));
}

/// How an OpenAI-compatible vendor hears voice input: as it is spoken, or uploaded at the end.
fn set_mode(mode: &str) {
    if mode != "realtime" && mode != "upload" {
        return;
    }
    with_settings(|settings| {
        if format_of(&settings.voice_input, &settings.voice_input.vendor) == Some(ConnectionFormat::OpenAi) && settings.voice_input.mode != mode {
            settings.voice_input.mode = mode.to_string();
            settings.save();
        }
    });
    with_ui(|ui| refresh(ui, false));
}

fn edit_keywords(text: &str) {
    with_settings(|settings| {
        settings.voice_input.keywords = text.trim().to_string();
        settings.save();
    });
}

/// Tidying's switch.
fn set_tidy(on: bool) {
    with_settings(|settings| {
        settings.voice_input.tidy = on;
        settings.save();
    });
    with_ui(|ui| refresh(ui, false));
}

/// Transcribing voice messages' switch.
fn set_transcribe_messages(on: bool) {
    with_settings(|settings| {
        settings.voice_input.transcribe_messages = on;
        settings.save();
    });
    with_ui(|ui| refresh(ui, false));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_join_as_their_script_is_written() {
        assert_eq!(join_words("", "Hello."), "Hello.");
        assert_eq!(join_words("Hi", ""), "Hi");
        assert_eq!(join_words("Hi", "see you at three."), "Hi see you at three.");
        assert_eq!(join_words("Hi ", "there"), "Hi there");
        assert_eq!(join_words("Hi\n", "there"), "Hi\nthere");
        assert_eq!(join_words("好的", "明天见。"), "好的明天见。");
        assert_eq!(join_words("ok", "，明天见"), "ok，明天见");
        assert_eq!(join_words("了解", "OK"), "了解OK");
        assert_eq!(join_words("はい", "わかりました"), "はいわかりました");
        assert_eq!(join_words("네", "알겠습니다"), "네 알겠습니다");
        assert_eq!(join_words("Hello", ", friend"), "Hello, friend");
        assert_eq!(join_words("(", "note"), "(note");
        assert_eq!(join_words("Привет", "друг"), "Привет друг");
    }

    #[test]
    fn a_key_shows_by_its_first_and_last_letters_and_any_vendors_look_is_taken() {
        assert_eq!(hint("sk-proj-abcdefghijklmnop4f2a"), "sk-…4f2a");
        assert_eq!(hint("AIzaSyD-abcdefghij91c2"), "AIz…91c2");
        assert_eq!(hint("short-key"), "…ey");
        assert!(looks_like_a_key("sk-proj-abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(looks_like_a_key("gsk_abcdefghijklmnop"));
        assert!(looks_like_a_key("123456789:access-token"));
        assert!(!looks_like_a_key("short"));
        assert!(!looks_like_a_key("sk-abcdefghij klmnopqrstuvwxyz"));
        assert!(!looks_like_a_key("sk-abcdefghijklmnopqrstuvwxyz中"));
    }

    #[test]
    fn the_languages_offered_are_the_uis_by_the_services_codes() {
        assert_eq!(LANGUAGES.len(), crate::i18n::LANGUAGES.len());
        for ((ui_code, _), language) in LANGUAGES.iter().zip(crate::i18n::LANGUAGES) {
            assert_eq!(*ui_code, language.code);
        }
        assert!(is_language("zh-tw") && !is_language("auto") && !is_language("zh_Hans"));
        assert_eq!(key_name("3"), "voice_input.connection.3");
    }

    /// OpenAI-compatible (1), Doubao (2), a Claude relay (3), a vendor not filled in yet (4):
    /// Doubao hears Chinese, the relay tidies, OpenAI transcribes voice messages.
    fn three_vendors() -> VoiceInput {
        VoiceInput {
            main_languages: vec!["auto".into(), "zh-cn".into()],
            language: "zh-cn".into(),
            vendors: vec![
                AiVendor { id: "1".into(), label: "OpenAI".into(), format: "openai".into(), ..AiVendor::default() },
                AiVendor { id: "2".into(), label: "豆包语音".into(), format: "doubao".into(), ..AiVendor::default() },
                AiVendor { id: "3".into(), label: "Claude 中转".into(), format: "anthropic".into(), url: "https://relay.example.com".into(), ..AiVendor::default() },
                AiVendor { id: "4".into(), ..AiVendor::default() },
            ],
            vendor: "2".into(),
            model: "volc.seedasr.sauc.duration".into(),
            tidy: true,
            tidy_vendor: "3".into(),
            tidy_model: "claude-haiku-5-5".into(),
            transcribe_messages: true,
            messages_vendor: "1".into(),
            messages_model: "gpt-transcribe".into(),
            ..VoiceInput::default()
        }
    }

    #[test]
    fn each_feature_offers_the_vendors_that_can_do_it_and_their_models_as_listed() {
        let voice_input = three_vendors();
        let listed = |id: &str, _: Ability| match id {
            "1" => Some(Models::Listed(vec!["gpt-transcribe".into(), "whisper-1".into(), "gpt-5-mini".into()])),
            "2" => Some(Models::Listed(service::DOUBAO_RESOURCES.iter().map(|resource| resource.to_string()).collect())),
            _ => Some(Models::Loading),
        };
        let view = panel_view(&voice_input, "2", &|id| (id == "2").then(|| "6f1…9b2c".to_string()), &listed, &|feature| if feature == Feature::Messages { "whisp".into() } else { String::new() });
        let ids = |items: &[VoiceItem]| items.iter().map(|item| item.id.to_string()).collect::<Vec<_>>();
        // Voice input: OpenAI and Doubao, not Anthropic, not the one with no format.
        assert_eq!(ids(&view.speech.vendors), vec!["1", "2"]);
        assert_eq!((view.speech.vendor_label.as_str(), view.speech.format.as_str(), view.speech.state), ("豆包语音", "doubao", ModelsState::Listed));
        assert_eq!(view.speech.models.len(), 4);
        // Tidying: OpenAI and Anthropic, not Doubao; the relay's models on their way.
        assert_eq!(ids(&view.tidy.vendors), vec!["1", "3"]);
        assert_eq!((view.tidy.state, view.tidy.model.as_str()), (ModelsState::Loading, "claude-haiku-5-5"));
        // Voice messages: what was typed in its search narrows the list.
        assert_eq!(ids(&view.messages.vendors), vec!["1", "2"]);
        assert_eq!(ids(&view.messages.models), vec!["whisper-1"]);
        // The vendors listed, the open one's fields; the new one has neither a name nor a format.
        assert_eq!(view.vendors.iter().map(|row| (row.label.to_string(), row.format.to_string(), row.host.to_string(), row.has_key)).collect::<Vec<_>>(), vec![
            ("OpenAI".to_string(), "openai".to_string(), "api.openai.com".to_string(), false),
            ("豆包语音".to_string(), "doubao".to_string(), "openspeech.bytedance.com".to_string(), true),
            ("Claude 中转".to_string(), "anthropic".to_string(), "relay.example.com".to_string(), false),
            (String::new(), String::new(), String::new(), false),
        ]);
        let open = view.open.expect("the open one");
        assert_eq!((open.format.as_str(), open.key_hint.as_str(), open.official, open.url_problem), ("doubao", "6f1…9b2c", true, 0));
        assert_eq!(view.language_name, "简体中文");
        // Nothing chosen: no vendor, no models asked for.
        let empty = panel_view(&VoiceInput { vendors: voice_input.vendors.clone(), ..VoiceInput::default() }, "", &|_| None, &listed, &|_| String::new());
        assert_eq!((empty.speech.vendor.as_str(), empty.speech.model.as_str(), empty.speech.state), ("", "", ModelsState::None));
        assert!(!empty.tidy_on && !empty.messages_on);
    }

    #[test]
    fn voice_input_needs_its_vendor_its_way_of_hearing_and_its_model() {
        let mut voice_input = three_vendors();
        let speech = speech_of(&voice_input).map_err(|_| "no speech").expect("Doubao");
        assert_eq!((speech.format, speech.shown().as_str(), speech.model.as_str()), (SpeechFormat::Doubao, "豆包语音", "volc.seedasr.sauc.duration"));
        // An OpenAI-compatible vendor, no way of hearing chosen, then uploading.
        voice_input.vendor = "1".into();
        voice_input.model = "gpt-transcribe".into();
        assert!(matches!(speech_of(&voice_input), Err(VoiceProblem::NoMode)));
        voice_input.mode = "upload".into();
        assert_eq!(speech_of(&voice_input).map(|speech| speech.format).map_err(|_| "no speech"), Ok(SpeechFormat::OpenAiTranscriptions));
        // No model chosen; a vendor that cannot hear, or none.
        voice_input.model.clear();
        assert!(matches!(speech_of(&voice_input), Err(VoiceProblem::NoModel)));
        voice_input.vendor = "3".into();
        assert!(matches!(speech_of(&voice_input), Err(VoiceProblem::NoService)));
        voice_input.vendor.clear();
        assert!(matches!(speech_of(&voice_input), Err(VoiceProblem::NoService)));
        // Voice messages: their own vendor, which uploads; OpenAI's own address wants a key, a relay
        // may not; nothing chosen, nothing sent.
        let mut voice_input = three_vendors();
        assert!(matches!(messages_options_of(&voice_input), Err(VoiceProblem::NoKey)));
        voice_input.vendors[0].url = "https://relay.example.com/v1".into();
        let messages = messages_options_of(&voice_input).map_err(|_| "no options").expect("a relay");
        assert_eq!((messages.speech.format, messages.language.as_str(), messages.recorded, messages.tidy.is_none()), (SpeechFormat::OpenAiTranscriptions, "auto", true, true));
        assert!(matches!(messages_options_of(&VoiceInput { messages_model: String::new(), ..voice_input.clone() }), Err(VoiceProblem::NoModel)));
        assert!(matches!(messages_options_of(&VoiceInput::default()), Err(VoiceProblem::NoService)));
    }

    #[test]
    fn words_show_while_speaking_except_for_uploads_and_doubaos_other_languages() {
        let endpoint = |format: &str| Endpoint::new(format, "", "").expect("the default address");
        let speech = |format: &str, upload: bool, model: &str| Speech::new(endpoint(format), upload, model).expect("it hears");
        assert!(live(&speech("openai", false, "gpt-live-transcribe"), "auto"));
        assert!(!live(&speech("openai", false, "gpt-transcribe"), "auto"));
        assert!(!live(&speech("openai", true, "gpt-transcribe"), "zh-cn"));
        assert!(live(&speech("dashscope", false, "qwen3-asr-flash-realtime"), "ja"));
        assert!(live(&speech("doubao", false, "volc.seedasr.sauc.duration"), "zh-tw") && !live(&speech("doubao", false, "volc.seedasr.sauc.duration"), "ja"));
    }
}
