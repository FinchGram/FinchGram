//! Voice input (docs/voice-input.md): the user presses the sparkle in the composer (in Terminal,
//! types /voice) and speaks; the words come back from the speech service the user chose, under
//! their own key, and show in the strip above the composer; when the user is done (the sparkle
//! again, ⏎, or letting go of it after holding it), a text model tidies them and they go into the
//! composer, for the user to send. Esc drops them. The sound goes from the microphone to that
//! service and nowhere else, and FinchGram keeps no recording. (The microphone in the composer is
//! voice messages', src/telegram/voice.rs.)
//!
//! The user adds connections: a name of theirs, an API format, an address and a key, any vendor,
//! relay or server of their own that speaks one of the formats (service.rs). Each ability picks a
//! connection and a model: for each main language the user speaks (a profile), the speech
//! recognition that hears it (OpenAI's realtime transcription or its upload, Alibaba Cloud's Qwen,
//! ByteDance's Doubao: realtime.rs, upload.rs, doubao.rs) and the text model that tidies the words
//! (Chat Completions, Anthropic's or Google's format: tidy.rs); and the one that transcribes voice
//! messages (messages.rs). Which main language is used is chosen in the panel, or in the strip while
//! speaking; words of other languages mixed in are kept as spoken.
//!
//! Voice input is a tool of the toolbox's AI category (src/toolbox.rs, ui/pages/voice-input.slint),
//! off until the user turns it on there: off, the sparkle is not in the composer, and Terminal's
//! /voice, or the button of a stop for want of a key, opens its panel, where the connections and
//! the abilities are set.
//!
//! One dictation at a time, its state here on the UI thread. The microphone's samples come from
//! one of the system's threads (src/platform/microphone.rs); the session runs on a thread of its
//! own ("voice", `dictation::dictate`), which reports back through the event loop. The keys are
//! the database's (`database::credential`, `voice_input.connection.<id>`), never in settings.toml.

mod audio;
pub mod dictation;
mod doubao;
pub mod messages;
mod realtime;
pub mod service;
mod tidy;
mod upload;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::database::{credential, remove_credential, save_credential};
use crate::platform::{self, MicrophoneAccess};
use crate::settings::{Settings, VoiceConnection, VoiceInput, VoiceProfile};
use crate::{Conversation, KeyCheck, MainWindow, Recorder, Toolbox, Voice, VoiceChoice, VoiceChoiceRow, VoiceConnectionRow, VoiceLanguage, VoiceLanguageRow, VoiceProblem, VoiceState};
#[cfg(test)]
pub use audio::wav;
pub use audio::wav_samples;
use dictation::{Input, Options, Report};
use service::{AddressProblem, ConnectionFormat, Endpoint, FailureKind, KeyAnswer, Speech, SpeechFormat, Tidy};

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

/// The database's name for a connection's key.
fn key_name(id: &str) -> String {
    format!("voice_input.connection.{id}")
}

/// A connection's key; none for a connection that is not there, or has none.
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
    /// Counted up with every check of a connection, so that only the last one's answer shows.
    static CHECKS: Cell<u64> = const { Cell::new(0) };
    /// The connection open in the panel for editing, by id; "" for none.
    static OPEN: RefCell<String> = const { RefCell::new(String::new()) };
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
        let has_key = settings.voice_input.connections.iter().any(|connection| key_of(&connection.id).is_some());
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
    voice.on_add_connection(add_connection);
    voice.on_toggle_connection(|id| toggle_connection(&id));
    voice.on_edit_connection_label(|text| edit_connection(ConnectionField::Label, &text));
    voice.on_edit_connection_url(|text| edit_connection(ConnectionField::Url, &text));
    voice.on_set_connection_format(|name| set_connection_format(&name));
    voice.on_check_connection(check_connection);
    voice.on_save_connection_key(|text| save_key(&text));
    voice.on_remove_connection_key(remove_key);
    voice.on_remove_connection(remove_connection);
    voice.on_open_key_page(open_key_page);
    voice.on_choose(|which, id| choose(&which, &id));
    voice.on_set_mode(|mode| set_mode(&mode));
    voice.on_edit_model(|which, text| edit_model(&which, &text));
    voice.on_edit_keywords(|text| edit_keywords(&text));
    voice.on_set_tidy(set_tidy);
    voice.on_set_transcribe_messages(set_transcribe_messages);
}

/// 0.7's keys into the connections its settings became (settings.rs), and the connections that
/// turn out to be the same (one format, one address, one key: a key copied to two languages in
/// 0.7.2) made one. Once: a connection's `key_from` goes with its key.
fn adopt_keys(settings: &mut Settings) {
    let voice_input = &mut settings.voice_input;
    if !voice_input.connections.iter().any(|connection| connection.key_from.is_some()) {
        return;
    }
    let mut adopted: Vec<String> = Vec::new();
    for connection in &mut voice_input.connections {
        let Some(from) = connection.key_from.take() else { continue };
        // 0.7.2 moved 0.7.1's key under the first profile's name at its start, even when its
        // settings were not written again: the key is under either name. 0.7.1's was OpenAI's.
        let (key, source) = match credential(&from) {
            Some(key) => (Some(key), from),
            None if connection.format == "openai" => (credential(OLD_KEY), OLD_KEY.to_string()),
            None => continue,
        };
        let Some(key) = key else { continue };
        let name = key_name(&connection.id);
        if credential(&name).is_none()
            && let Err(err) = save_credential(&name, &key)
        {
            eprintln!("voice input: cannot move a key of 0.7: {err}");
            connection.key_from = Some(source);
            continue;
        }
        adopted.push(source);
    }
    // The same format, address and key twice: the first one stays, and takes the other's place.
    // One whose key could not move yet is left as it is, for the next start.
    let mut keys: Vec<Option<String>> = voice_input.connections.iter().map(|connection| key_of(&connection.id)).collect();
    let mut index = 0;
    while index < voice_input.connections.len() {
        let twin = (0..index).find(|&earlier| {
            let (a, b) = (&voice_input.connections[earlier], &voice_input.connections[index]);
            a.format == b.format && a.url == b.url && keys[earlier] == keys[index] && a.key_from.is_none() && b.key_from.is_none()
        });
        match twin {
            Some(earlier) => {
                let (gone, kept) = (voice_input.connections[index].id.clone(), voice_input.connections[earlier].id.clone());
                repoint(voice_input, &gone, &kept);
                let _ = remove_credential(&key_name(&gone));
                voice_input.connections.remove(index);
                keys.remove(index);
            }
            None => index += 1,
        }
    }
    // An old key goes once every connection that took it has its copy.
    for source in adopted {
        if !voice_input.connections.iter().any(|connection| connection.key_from.as_deref() == Some(source.as_str())) {
            let _ = remove_credential(&source);
        }
    }
    settings.save();
}

/// What used connection `gone` uses `kept` from now on ("" for none).
fn repoint(voice_input: &mut VoiceInput, gone: &str, kept: &str) {
    for profile in &mut voice_input.profiles {
        if profile.connection == gone {
            profile.connection = kept.to_string();
        }
        if profile.tidy_connection == gone {
            profile.tidy_connection = kept.to_string();
        }
    }
    if voice_input.messages_connection == gone {
        voice_input.messages_connection = kept.to_string();
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

/// A connection as the services take it; none when it is not there, Err when its address is not one.
fn endpoint_of(voice_input: &VoiceInput, id: &str) -> Option<Result<Endpoint, AddressProblem>> {
    let connection = voice_input.connection(id)?;
    Some(Endpoint::new(&connection.format, &connection.url, &connection.label))
}

/// The speech service of a profile: its connection, how, its model.
fn speech_of(voice_input: &VoiceInput, profile: &VoiceProfile) -> Result<Speech, VoiceProblem> {
    let endpoint = endpoint_of(voice_input, &profile.connection).ok_or(VoiceProblem::NoService)?.map_err(|_| VoiceProblem::BadAddress)?;
    Speech::new(endpoint, profile.mode == "upload", &profile.model).ok_or(VoiceProblem::NoService)
}

/// A dictation's settings, from a profile: Err with the problem that stops it.
fn options_of(voice_input: &VoiceInput, profile: &VoiceProfile, recorded: bool) -> Result<Options, VoiceProblem> {
    let speech = speech_of(voice_input, profile)?;
    let speech_key = key_of(&profile.connection);
    // The vendor's own address wants a key; a server of the user's own may not.
    if speech_key.is_none() && speech.official() {
        return Err(VoiceProblem::NoKey);
    }
    let tidy = if profile.tidy && !recorded {
        endpoint_of(voice_input, &profile.tidy_connection)
            .and_then(Result::ok)
            .and_then(|endpoint| Tidy::new(endpoint, &profile.tidy_model))
            .and_then(|tidy| {
                let key = key_of(&profile.tidy_connection);
                // Without its key the words go in as heard; the panel says what is missing.
                if key.is_none() && tidy.official() { None } else { Some((tidy, key)) }
            })
    } else {
        None
    };
    Ok(Options {
        speech,
        speech_key,
        language: profile.language.clone(),
        keywords: dictation::keywords(&profile.keywords),
        delay: profile.delay.clone(),
        tidy,
        recorded,
    })
}

/// The voice messages' own (src/speech/messages.rs): the connection chosen for them, which takes
/// an OpenAI connection's upload and finds the language itself; or, with none chosen, the speech
/// recognition of the main language in use.
pub fn message_options() -> Result<Options, VoiceProblem> {
    with_settings(|settings| {
        let voice_input = &settings.voice_input;
        if voice_input.messages_connection.is_empty() {
            return options_of(voice_input, voice_input.current(), true);
        }
        let own = VoiceProfile {
            connection: voice_input.messages_connection.clone(),
            mode: "upload".into(),
            model: voice_input.messages_model.clone(),
            tidy: false,
            ..VoiceProfile::default()
        };
        options_of(voice_input, &own, true)
    })
    .unwrap_or(Err(VoiceProblem::NoService))
}

/// Whether the user lets their own service transcribe voice messages.
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
    let options = match with_settings(|settings| options_of(&settings.voice_input, settings.voice_input.current(), false)).unwrap_or(Err(VoiceProblem::NoService)) {
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

/// The strip's words of the profile in use: who hears it, its language, whether words come
/// while the user speaks.
fn show_strip_service() {
    let found = with_settings(|settings| {
        let voice_input = &settings.voice_input;
        let profile = voice_input.current();
        let speech = speech_of(voice_input, profile).ok();
        (speech, profile.language.clone(), voice_input.profiles.len())
    });
    let Some((speech, language, count)) = found else { return };
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        voice.set_service(speech.as_ref().map(Speech::shown).unwrap_or_default().into());
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
        if voice_input.profiles.len() < 2 {
            return false;
        }
        let index = voice_input.profiles.iter().position(|profile| profile.language == voice_input.language).unwrap_or(0);
        voice_input.language = voice_input.profiles[(index + 1) % voice_input.profiles.len()].language.clone();
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
        VoiceProblem::NoKey | VoiceProblem::NoService | VoiceProblem::RefusedKey | VoiceProblem::BadAddress => with_ui(open_panel),
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

/// The panel's switch, or its card's: on, the sparkle shows in the composer; off, it goes, and a
/// dictation going on is dropped.
fn set_on(on: bool) {
    with_settings(|settings| {
        settings.voice_input.enabled = Some(on);
        settings.save();
    });
    with_ui(|ui| ui.global::<Voice>().set_on(on));
    if !on && state() != VoiceState::Idle {
        cancel();
    }
}

/// The connection open for editing, as the panel shows it.
struct OpenConnection {
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

/// What the panel shows of the settings, with the keys' letters (the database's, or the
/// screenshots' own).
struct PanelView {
    languages: Vec<VoiceLanguage>,
    addable: Vec<VoiceLanguage>,
    language_name: String,
    connections: Vec<VoiceConnectionRow>,
    open: Option<OpenConnection>,
    speech_choices: Vec<VoiceChoice>,
    speech_format: String,
    speech_mode: String,
    speech_model: String,
    speech_model_placeholder: String,
    keywords: String,
    tidy: bool,
    tidy_choices: Vec<VoiceChoice>,
    tidy_model: String,
    tidy_model_placeholder: String,
    transcribe_messages: bool,
    message_choices: Vec<VoiceChoice>,
    messages_model: String,
    messages_model_placeholder: String,
}

fn format_of(voice_input: &VoiceInput, id: &str) -> Option<ConnectionFormat> {
    voice_input.connection(id).map(|connection| ConnectionFormat::from_name(&connection.format))
}

fn panel_view(voice_input: &VoiceInput, open: &str, key_hint: &dyn Fn(&str) -> Option<String>) -> PanelView {
    let profile = voice_input.current();
    let chip = |code: &str, on: bool| VoiceLanguage { code: code.into(), name: language_name(code).into(), on };
    let languages = voice_input.profiles.iter().map(|other| chip(&other.language, other.language == profile.language)).collect();
    let mut addable: Vec<VoiceLanguage> = Vec::new();
    if !voice_input.profiles.iter().any(|other| other.language == "auto") {
        addable.push(chip("auto", false));
    }
    addable.extend(LANGUAGES.iter().filter(|(_, code)| !voice_input.profiles.iter().any(|other| other.language == *code)).map(|(_, code)| chip(code, false)));
    let connections = voice_input
        .connections
        .iter()
        .map(|connection| {
            let hint = key_hint(&connection.id);
            VoiceConnectionRow {
                id: connection.id.as_str().into(),
                label: label_of(connection).into(),
                format: connection.format.as_str().into(),
                // An address that is no address shows as typed, for the user to know the row.
                host: Endpoint::new(&connection.format, &connection.url, "").map(|endpoint| endpoint.address.shown()).unwrap_or_else(|_| connection.url.trim().to_string()).into(),
                has_key: hint.is_some(),
                key_hint: hint.unwrap_or_default().into(),
            }
        })
        .collect();
    let open = voice_input.connection(open).map(|connection| {
        let format = ConnectionFormat::from_name(&connection.format);
        let endpoint = Endpoint::new(&connection.format, &connection.url, &connection.label);
        let hint = key_hint(&connection.id);
        OpenConnection {
            label: connection.label.clone(),
            format: connection.format.clone(),
            url: connection.url.clone(),
            url_placeholder: format.default_url().into(),
            url_problem: problem_number(endpoint.as_ref().err()),
            has_key: hint.is_some(),
            key_hint: hint.unwrap_or_default(),
            official: endpoint.as_ref().is_ok_and(Endpoint::official),
            shown: label_of(connection),
        }
    });
    let choices = |chosen: &str, fits: fn(ConnectionFormat) -> bool| -> Vec<VoiceChoice> {
        voice_input
            .connections
            .iter()
            .filter(|connection| fits(ConnectionFormat::from_name(&connection.format)))
            .map(|connection| VoiceChoice { id: connection.id.as_str().into(), label: label_of(connection).into(), on: connection.id == chosen })
            .collect()
    };
    let speech_format = format_of(voice_input, &profile.connection);
    let upload = profile.mode == "upload";
    let tidy_format = format_of(voice_input, &profile.tidy_connection);
    let messages_format = format_of(voice_input, &voice_input.messages_connection);
    let mut message_choices = vec![VoiceChoice { id: SharedString::new(), label: SharedString::new(), on: voice_input.messages_connection.is_empty() }];
    message_choices.extend(choices(&voice_input.messages_connection, ConnectionFormat::hears));
    PanelView {
        languages,
        addable,
        language_name: language_name(&profile.language),
        connections,
        open,
        speech_choices: choices(&profile.connection, ConnectionFormat::hears),
        speech_format: speech_format.map(ConnectionFormat::name).unwrap_or_default().into(),
        speech_mode: profile.mode.clone(),
        speech_model: profile.model.clone(),
        speech_model_placeholder: speech_format.map(|format| format.speech_model(upload)).unwrap_or_default().into(),
        keywords: profile.keywords.clone(),
        tidy: profile.tidy,
        tidy_choices: choices(&profile.tidy_connection, ConnectionFormat::writes),
        tidy_model: profile.tidy_model.clone(),
        tidy_model_placeholder: tidy_format.map(ConnectionFormat::text_model).unwrap_or_default().into(),
        transcribe_messages: voice_input.transcribe_messages,
        message_choices,
        messages_model: voice_input.messages_model.clone(),
        messages_model_placeholder: messages_format.map(|format| format.speech_model(true)).unwrap_or_default().into(),
    }
}

/// A connection's name, its address's host when it has none.
fn label_of(connection: &VoiceConnection) -> String {
    if connection.label.trim().is_empty() { service::default_label(&connection.format, &connection.url) } else { connection.label.trim().to_string() }
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
    let Some(view) = with_settings(|settings| panel_view(&settings.voice_input, &open, &|id| credential(&key_name(id)).as_deref().map(hint))) else {
        return;
    };
    apply(ui, view, typed);
}

fn apply(ui: &MainWindow, view: PanelView, typed: bool) {
    let voice = ui.global::<Voice>();
    voice.set_can_remove(view.languages.len() > 1);
    voice.set_language_rows(lines(view.languages));
    voice.set_addable_rows(lines(view.addable));
    voice.set_language_name(view.language_name.into());
    voice.set_connections(ModelRc::new(VecModel::from(view.connections)));
    match view.open {
        Some(open) => {
            voice.set_open_connection(OPEN.with(|id| id.borrow().clone()).into());
            if typed {
                voice.set_connection_label(open.label.into());
                voice.set_connection_url(open.url.into());
            }
            voice.set_connection_format(open.format.into());
            voice.set_connection_url_placeholder(open.url_placeholder.into());
            voice.set_connection_url_problem(open.url_problem);
            voice.set_connection_has_key(open.has_key);
            voice.set_connection_key_hint(open.key_hint.into());
            voice.set_connection_official(open.official);
            voice.set_connection_shown(open.shown.into());
        }
        None => voice.set_open_connection(SharedString::new()),
    }
    voice.set_speech_choices(choice_lines(view.speech_choices));
    voice.set_speech_format(view.speech_format.into());
    voice.set_speech_mode(view.speech_mode.into());
    voice.set_speech_model_placeholder(view.speech_model_placeholder.into());
    voice.set_tidy(view.tidy);
    voice.set_tidy_choices(choice_lines(view.tidy_choices));
    voice.set_tidy_model_placeholder(view.tidy_model_placeholder.into());
    voice.set_transcribe_messages(view.transcribe_messages);
    voice.set_message_choices(choice_lines(view.message_choices));
    voice.set_messages_model_placeholder(view.messages_model_placeholder.into());
    if typed {
        voice.set_speech_model(view.speech_model.into());
        voice.set_keywords(view.keywords.into());
        voice.set_tidy_model(view.tidy_model.into());
        voice.set_messages_model(view.messages_model.into());
    }
}

/// Connections to choose from, three in a line.
fn choice_lines(choices: Vec<VoiceChoice>) -> ModelRc<VoiceChoiceRow> {
    let lines: Vec<VoiceChoiceRow> = choices.chunks(3).map(|line| VoiceChoiceRow { choices: ModelRc::new(VecModel::from(line.to_vec())) }).collect();
    ModelRc::new(VecModel::from(lines))
}

/// Chips in lines of PER_LINE.
fn lines(chips: Vec<VoiceLanguage>) -> ModelRc<VoiceLanguageRow> {
    let lines: Vec<VoiceLanguageRow> = chips.chunks(PER_LINE).map(|line| VoiceLanguageRow { languages: ModelRc::new(VecModel::from(line.to_vec())) }).collect();
    ModelRc::new(VecModel::from(lines))
}

/// The panel for the screenshot test (src/screenshots.rs): `voice_input` as the settings, the
/// connection `open` open, with keys whose letters are `hints` (by connection id).
#[cfg(test)]
pub fn show_for_pictures(ui: &MainWindow, voice_input: &VoiceInput, open: &str, hints: &[(&str, &str)]) {
    OPEN.with(|id| *id.borrow_mut() = open.to_string());
    let view = panel_view(voice_input, open, &|id| hints.iter().find(|(known, _)| *known == id).map(|(_, hint)| hint.to_string()));
    apply(ui, view, true);
    let profile = voice_input.current();
    let speech = speech_of(voice_input, profile).ok();
    let voice = ui.global::<Voice>();
    voice.set_service(speech.as_ref().map(Speech::shown).unwrap_or_default().into());
    voice.set_live(speech.as_ref().is_some_and(|speech| live(speech, &profile.language)));
    voice.set_can_switch(voice_input.profiles.len() > 1);
}

/// A main language chosen in the panel: the one used from now on, and its abilities shown.
fn choose_language(code: &str) {
    with_settings(|settings| {
        if settings.voice_input.profiles.iter().any(|profile| profile.language == code) {
            settings.voice_input.language = code.to_string();
            settings.save();
        }
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_adding(false);
        show_panel(ui);
    });
}

/// A main language added: its abilities start as the ones in use (the same connections), and it
/// is the one used from now on.
fn add_language(code: &str) {
    if code != "auto" && !is_language(code) {
        return;
    }
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        if voice_input.profiles.iter().any(|profile| profile.language == code) {
            return;
        }
        let from = voice_input.current().clone();
        voice_input.profiles.push(VoiceProfile { language: code.to_string(), ..from });
        voice_input.language = code.to_string();
        settings.save();
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_adding(false);
        show_panel(ui);
    });
}

/// The main language in use removed; the first one left is used. The connections stay.
fn remove_language() {
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        if voice_input.profiles.len() < 2 {
            return;
        }
        let language = voice_input.language.clone();
        voice_input.profiles.retain(|profile| profile.language != language);
        voice_input.language = voice_input.profiles[0].language.clone();
        settings.save();
    });
    with_ui(show_panel);
}

// ---- connections -------------------------------------------------------------------------------

/// A new connection, OpenAI's by default, open for the user to name, point and key; the main
/// language's abilities that have none take it.
fn add_connection() {
    let id = with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let id = voice_input.next_connection_id();
        voice_input.connections.push(VoiceConnection { id: id.clone(), label: service::default_label("openai", ""), ..VoiceConnection::default() });
        let profile = voice_input.current_mut();
        if profile.connection.is_empty() {
            profile.connection = id.clone();
        }
        if profile.tidy_connection.is_empty() {
            profile.tidy_connection = id.clone();
        }
        settings.save();
        id
    });
    let Some(id) = id else { return };
    OPEN.with(|open| *open.borrow_mut() = id);
    with_ui(|ui| {
        ui.global::<Voice>().set_connection_check(KeyCheck::None);
        show_panel(ui);
    });
}

/// A connection's row clicked: open for editing, or closed.
fn toggle_connection(id: &str) {
    OPEN.with(|open| {
        let mut open = open.borrow_mut();
        *open = if *open == id { String::new() } else { id.to_string() };
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_connection_check(KeyCheck::None);
        show_panel(ui);
    });
}

fn open_id() -> String {
    OPEN.with(|open| open.borrow().clone())
}

enum ConnectionField {
    Label,
    Url,
}

/// The open connection's name or address typed: kept as typed, and what is said of it shown.
fn edit_connection(field: ConnectionField, text: &str) {
    let id = open_id();
    with_settings(|settings| {
        if let Some(connection) = settings.voice_input.connections.iter_mut().find(|connection| connection.id == id) {
            match field {
                ConnectionField::Label => connection.label = text.trim().to_string(),
                ConnectionField::Url => connection.url = text.trim().to_string(),
            }
            settings.save();
        }
    });
    with_ui(|ui| {
        ui.global::<Voice>().set_connection_check(KeyCheck::None);
        refresh(ui, false);
    });
}

/// The open connection's format chosen: its name follows when it was the format's own, and the
/// abilities it can no longer do are left without it.
fn set_connection_format(name: &str) {
    if !service::CONNECTION_FORMATS.contains(&name) {
        return;
    }
    let id = open_id();
    let changed = with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let connection = voice_input.connections.iter_mut().find(|connection| connection.id == id)?;
        if connection.format == name {
            return None;
        }
        if connection.label.is_empty() || connection.label == service::default_label(&connection.format, &connection.url) {
            connection.label = service::default_label(name, &connection.url);
        }
        connection.format = name.to_string();
        let format = ConnectionFormat::from_name(name);
        for profile in &mut voice_input.profiles {
            if profile.connection == id {
                profile.model.clear();
                if !format.hears() {
                    profile.connection.clear();
                }
            }
            if profile.tidy_connection == id {
                profile.tidy_model.clear();
                if !format.writes() {
                    profile.tidy_connection.clear();
                }
            }
        }
        if voice_input.messages_connection == id {
            voice_input.messages_model.clear();
            if !format.hears() {
                voice_input.messages_connection.clear();
            }
        }
        settings.save();
        Some(())
    })
    .flatten();
    if changed.is_some() {
        with_ui(|ui| {
            // The name may follow the format, and models typed for the old one went: the boxes
            // show what is kept now.
            let kept = with_settings(|settings| {
                let voice_input = &settings.voice_input;
                let label = voice_input.connection(&id).map(|connection| connection.label.clone()).unwrap_or_default();
                let profile = voice_input.current();
                (label, profile.model.clone(), profile.tidy_model.clone(), voice_input.messages_model.clone())
            });
            refresh(ui, false);
            if let Some((label, model, tidy_model, messages_model)) = kept {
                let voice = ui.global::<Voice>();
                voice.set_connection_label(label.into());
                voice.set_speech_model(model.into());
                voice.set_tidy_model(tidy_model.into());
                voice.set_messages_model(messages_model.into());
            }
        });
        check_connection();
    }
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
    with_ui(|ui| refresh(ui, false));
    check_connection();
}

fn remove_key() {
    CHECKS.set(CHECKS.get() + 1);
    let id = open_id();
    if !id.is_empty()
        && let Err(err) = remove_credential(&key_name(&id))
    {
        eprintln!("voice input: cannot remove the key: {err}");
    }
    with_ui(|ui| {
        refresh(ui, false);
        ui.global::<Voice>().set_connection_check(KeyCheck::None);
    });
}

/// The open connection removed, with its key; the abilities that used it have none.
fn remove_connection() {
    let id = open_id();
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        voice_input.connections.retain(|connection| connection.id != id);
        repoint(voice_input, &id, "");
        settings.save();
    });
    let _ = remove_credential(&key_name(&id));
    OPEN.with(|open| open.borrow_mut().clear());
    with_ui(show_panel);
}

fn set_check(check: KeyCheck, detail: &str) {
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        voice.set_connection_check(check);
        voice.set_connection_check_detail(detail.into());
    });
}

/// The open connection asked, on a thread of its own, whether it answers at its address and
/// takes the key; the answer shows under it. Nothing is asked of an address that is not one, nor
/// of a vendor's own address without a key.
fn check_connection() {
    let id = open_id();
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

/// The page of the open connection's vendor's keys.
fn open_key_page() {
    let id = open_id();
    if let Some(Some(format)) = with_settings(|settings| format_of(&settings.voice_input, &id)) {
        platform::open_link(format.key_page());
    }
}

// ---- the abilities -----------------------------------------------------------------------------

/// A connection chosen for an ability ("speech", "tidy", "messages"; "" for the messages' default):
/// a model typed for another format goes.
fn choose(which: &str, id: &str) {
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let format = format_of(voice_input, id);
        match which {
            "speech" => {
                let before = format_of(voice_input, &voice_input.current().connection);
                let profile = voice_input.current_mut();
                profile.connection = id.to_string();
                if before != format {
                    profile.model.clear();
                }
            }
            "tidy" => {
                let before = format_of(voice_input, &voice_input.current().tidy_connection);
                let profile = voice_input.current_mut();
                profile.tidy_connection = id.to_string();
                if before != format {
                    profile.tidy_model.clear();
                }
            }
            _ => {
                if format_of(voice_input, &voice_input.messages_connection) != format {
                    voice_input.messages_model.clear();
                }
                voice_input.messages_connection = id.to_string();
            }
        }
        settings.save();
    });
    with_ui(show_panel);
}

/// How an OpenAI connection hears the main language: as it is spoken, or uploaded at the end; a
/// model typed for the other way goes.
fn set_mode(mode: &str) {
    if mode != "realtime" && mode != "upload" {
        return;
    }
    with_settings(|settings| {
        let profile = settings.voice_input.current_mut();
        if profile.mode != mode {
            profile.mode = mode.to_string();
            profile.model.clear();
            settings.save();
        }
    });
    with_ui(show_panel);
}

/// A model typed, kept as typed.
fn edit_model(which: &str, text: &str) {
    with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        let text = text.trim().to_string();
        match which {
            "speech" => voice_input.current_mut().model = text,
            "tidy" => voice_input.current_mut().tidy_model = text,
            _ => voice_input.messages_model = text,
        }
        settings.save();
    });
}

fn edit_keywords(text: &str) {
    with_settings(|settings| {
        settings.voice_input.current_mut().keywords = text.trim().to_string();
        settings.save();
    });
}

fn set_tidy(on: bool) {
    with_settings(|settings| {
        settings.voice_input.current_mut().tidy = on;
        settings.save();
    });
    with_ui(show_panel);
}

fn set_transcribe_messages(on: bool) {
    with_settings(|settings| {
        settings.voice_input.transcribe_messages = on;
        settings.save();
    });
    with_ui(|ui| ui.global::<Voice>().set_transcribe_messages(on));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::VoiceConnection;

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

    /// OpenAI (1) for any language, Doubao (2) and Anthropic (3) for Chinese, in use.
    fn three_connections() -> VoiceInput {
        let mut voice_input = VoiceInput {
            connections: vec![
                VoiceConnection { id: "1".into(), label: "OpenAI".into(), ..VoiceConnection::default() },
                VoiceConnection { id: "2".into(), label: "豆包语音".into(), format: "doubao".into(), ..VoiceConnection::default() },
                VoiceConnection { id: "3".into(), label: "Claude 中转".into(), format: "anthropic".into(), url: "https://relay.example.com".into(), ..VoiceConnection::default() },
                    ],
            ..VoiceInput::default()
        };
        voice_input.profiles[0].connection = "1".into();
        voice_input.profiles[0].tidy_connection = "1".into();
        voice_input.profiles.push(VoiceProfile { language: "zh-cn".into(), connection: "2".into(), tidy_connection: "3".into(), ..VoiceProfile::default() });
        voice_input.language = "zh-cn".into();
        voice_input
    }

    #[test]
    fn each_ability_offers_the_connections_that_can_do_it() {
        let voice_input = three_connections();
        let view = panel_view(&voice_input, "2", &|id| (id == "2").then(|| "6f1…9b2c".to_string()));
        let ids = |choices: &[VoiceChoice]| choices.iter().map(|choice| (choice.id.to_string(), choice.on)).collect::<Vec<_>>();
        // Speech: OpenAI and Doubao, not Anthropic; Doubao chosen.
        assert_eq!(ids(&view.speech_choices), vec![("1".to_string(), false), ("2".to_string(), true)]);
        // Tidying: OpenAI and Anthropic, not Doubao; Anthropic chosen.
        assert_eq!(ids(&view.tidy_choices), vec![("1".to_string(), false), ("3".to_string(), true)]);
        // Voice messages: the speech recognition's by default, then the ones that hear.
        assert_eq!(ids(&view.message_choices), vec![(String::new(), true), ("1".to_string(), false), ("2".to_string(), false)]);
        assert_eq!((view.speech_format.as_str(), view.speech_model_placeholder.as_str()), ("doubao", "volc.seedasr.sauc.duration"));
        assert_eq!(view.tidy_model_placeholder.as_str(), "claude-haiku-5-5");
        // The connections listed, the open one's fields.
        assert_eq!(view.connections.iter().map(|row| (row.label.to_string(), row.host.to_string(), row.has_key)).collect::<Vec<_>>(), vec![
            ("OpenAI".to_string(), "api.openai.com".to_string(), false),
            ("豆包语音".to_string(), "openspeech.bytedance.com".to_string(), true),
            ("Claude 中转".to_string(), "relay.example.com".to_string(), false),
        ]);
        let open = view.open.expect("the open one");
        assert_eq!((open.format.as_str(), open.key_hint.as_str(), open.official, open.url_problem), ("doubao", "6f1…9b2c", true, 0));
        assert_eq!(view.language_name, "简体中文");
    }

    #[test]
    fn a_profile_hears_and_tidies_through_its_connections() {
        let voice_input = three_connections();
        let speech = speech_of(&voice_input, voice_input.current()).map_err(|_| "no speech").expect("Doubao");
        assert_eq!((speech.format, speech.shown().as_str()), (SpeechFormat::Doubao, "豆包语音"));
        // An OpenAI connection uploading.
        let mut upload = voice_input.profiles[0].clone();
        upload.mode = "upload".into();
        assert_eq!(speech_of(&voice_input, &upload).map(|speech| speech.format).map_err(|_| "no speech"), Ok(SpeechFormat::OpenAiTranscriptions));
        // None chosen, or one that cannot hear: no service.
        let mut none = voice_input.profiles[0].clone();
        none.connection.clear();
        assert!(matches!(speech_of(&voice_input, &none), Err(VoiceProblem::NoService)));
        none.connection = "3".into();
        assert!(matches!(speech_of(&voice_input, &none), Err(VoiceProblem::NoService)));
    }

    #[test]
    fn words_show_while_speaking_except_for_uploads_and_doubaos_other_languages() {
        let endpoint = |format: &str| Endpoint::new(format, "", "").expect("the default address");
        let speech = |format: &str, upload: bool, model: &str| Speech::new(endpoint(format), upload, model).expect("it hears");
        assert!(live(&speech("openai", false, ""), "auto"));
        assert!(!live(&speech("openai", false, "gpt-transcribe"), "auto"));
        assert!(!live(&speech("openai", true, ""), "zh-cn"));
        assert!(live(&speech("dashscope", false, ""), "ja"));
        assert!(live(&speech("doubao", false, ""), "zh-tw") && !live(&speech("doubao", false, ""), "ja"));
    }
}
