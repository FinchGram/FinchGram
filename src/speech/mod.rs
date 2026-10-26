//! Voice input (docs/voice-input.md): the user presses the sparkle in the composer (in Terminal,
//! types /voice) and speaks; the words come back from the speech service the user chose, under
//! their own key, and show in the strip above the composer; when the user is done (the sparkle
//! again, ⏎, or letting go of it after holding it), a text model tidies them and they go into the
//! composer, for the user to send. Esc drops them. The sound goes from the microphone to that
//! service and nowhere else, and FinchGram keeps no recording. (The microphone in the composer is
//! voice messages', src/telegram/voice.rs.)
//!
//! The services are named by their API's format, address and model (service.rs), so that any
//! vendor, relay or server of the user's own that speaks one of the formats can be used: OpenAI's
//! realtime transcription (realtime.rs), OpenAI's upload of a recording (upload.rs), Alibaba Cloud's
//! Qwen (realtime.rs), ByteDance's Doubao (doubao.rs); the tidying in Chat Completions, Anthropic's
//! or Google's format (tidy.rs). Each main language the user speaks has a set of its own (a
//! profile, settings.toml's `[[voice_input.profiles]]`): which one is used is the main language
//! chosen in the panel, or in the strip while speaking. Words of other languages mixed in are kept
//! as spoken.
//!
//! Voice input is a tool of the toolbox's AI category (src/toolbox.rs, ui/pages/voice-input.slint),
//! off until the user turns it on there: off, the sparkle is not in the composer, and Terminal's
//! /voice, or the button of a stop for want of a key, opens its panel, where the services and the
//! keys are set.
//!
//! One dictation at a time, its state here on the UI thread. The microphone's samples come from
//! one of the system's threads (src/platform/microphone.rs); the session runs on a thread of its
//! own ("voice", `dictation::dictate`), which reports back through the event loop. The keys are
//! the database's (`database::credential`, `voice_input.<language>.speech` and `….tidy`), never in
//! settings.toml.

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

use crate::platform::{self, MicrophoneAccess};
use crate::settings::{Settings, VoiceInput, VoiceProfile};
use crate::{Conversation, KeyCheck, MainWindow, Recorder, Toolbox, Voice, VoiceLanguage, VoiceLanguageRow, VoiceProblem, VoiceService, VoiceState};
#[cfg(test)]
pub use audio::wav;
pub use audio::wav_samples;
use dictation::{Input, Options, Report};
use service::{AddressProblem, FailureKind, KeyAnswer, Speech, SpeechFormat, Tidy};

/// 0.7's one key, OpenAI's, from before the profiles: it becomes the first profile's speech key.
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

/// Which of a profile's two services.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Speech,
    Tidy,
}

impl Which {
    fn of(name: &str) -> Which {
        if name == "tidy" { Which::Tidy } else { Which::Speech }
    }
}

/// The database's name for a profile's key.
fn key_name(language: &str, which: Which) -> String {
    format!("voice_input.{language}.{}", if which == Which::Tidy { "tidy" } else { "speech" })
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DICTATION: RefCell<Option<Dictation>> = const { RefCell::new(None) };
    /// Counted up with every dictation and every cancel: what an older one's thread still says is
    /// not heard.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    /// When the microphone was pressed to start the dictation, while it is still held.
    static PRESSED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Counted up with every check of a service, so that only the last one's answer shows.
    static CHECKS: Cell<u64> = const { Cell::new(0) };
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
    // 0.7's OpenAI key goes to the profile its settings became (settings.rs).
    let first = settings.borrow().voice_input.current().language.clone();
    move_old_key(&first);
    // A settings file from before the switch (0.7.0) has none: voice input is on there when a key
    // was kept. The choice is written with the next save of the settings.
    let on = *settings.borrow_mut().voice_input.enabled.get_or_insert_with(|| crate::database::credential(&key_name(&first, Which::Speech)).is_some());
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
    voice.on_set_format(|which, name| set_format(Which::of(&which), &name));
    voice.on_edit_url(|which, text| edit(Which::of(&which), Field::Url, &text));
    voice.on_edit_model(|which, text| edit(Which::of(&which), Field::Model, &text));
    voice.on_edit_keywords(|text| edit(Which::Speech, Field::Keywords, &text));
    voice.on_check(|which| check(Which::of(&which)));
    voice.on_save_key(|which, text| save_key(Which::of(&which), &text));
    voice.on_remove_key(|which| remove_key(Which::of(&which)));
    voice.on_open_key_page(|which| open_key_page(Which::of(&which)));
    voice.on_set_tidy(set_tidy);
    voice.on_set_same_key(set_same_key);
    voice.on_set_transcribe_messages(set_transcribe_messages);
}

/// 0.7's key moved under the profile's name, once.
fn move_old_key(language: &str) {
    let Some(old) = crate::database::credential(OLD_KEY) else { return };
    let name = key_name(language, Which::Speech);
    if crate::database::credential(&name).is_none()
        && let Err(err) = crate::database::save_credential(&name, &old)
    {
        eprintln!("voice input: cannot move the key of 0.7: {err}");
        return;
    }
    if let Err(err) = crate::database::remove_credential(OLD_KEY) {
        eprintln!("voice input: cannot remove the key of 0.7: {err}");
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

/// The profile in use now.
fn current() -> Option<VoiceProfile> {
    with_settings(|settings| settings.voice_input.current().clone())
}

/// A profile's key for one of its services (the speech service's for the tidying, when the two
/// share it).
fn key_of(profile: &VoiceProfile, which: Which) -> Option<String> {
    let which = if which == Which::Tidy && profile.tidy_same_key { Which::Speech } else { which };
    crate::database::credential(&key_name(&profile.language, which))
}

/// A dictation's settings, from a profile: Err with the problem that stops it.
fn options_of(profile: &VoiceProfile, recorded: bool) -> Result<Options, VoiceProblem> {
    let speech = Speech::new(&profile.format, &profile.url, &profile.model).map_err(|_| VoiceProblem::BadAddress)?;
    let speech_key = key_of(profile, Which::Speech);
    // The vendor's own address wants a key; a server of the user's own may not.
    if speech_key.is_none() && speech.official() {
        return Err(VoiceProblem::NoKey);
    }
    let tidy = if profile.tidy && !recorded {
        match Tidy::new(&profile.tidy_format, &profile.tidy_url, &profile.tidy_model) {
            Ok(tidy) => {
                let key = key_of(profile, Which::Tidy);
                // Without its key the words go in as heard; the panel says what is missing.
                if key.is_none() && tidy.official() { None } else { Some((tidy, key)) }
            }
            Err(_) => None,
        }
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

/// The voice messages' own: the profile in use, its speech service only (src/speech/messages.rs).
pub fn message_options() -> Result<Options, VoiceProblem> {
    let profile = current().ok_or(VoiceProblem::NoKey)?;
    options_of(&profile, true)
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
    let Some(profile) = current() else { return };
    show_strip_service(&profile);
    let options = match options_of(&profile, false) {
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
fn show_strip_service(profile: &VoiceProfile) {
    let speech = Speech::new(&profile.format, &profile.url, &profile.model).ok();
    let count = with_settings(|settings| settings.voice_input.profiles.len()).unwrap_or(1);
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        voice.set_service(speech.as_ref().map(Speech::shown).unwrap_or_default().into());
        voice.set_live(speech.as_ref().is_some_and(|speech| live(speech, &profile.language)));
        voice.set_language_name(language_name(&profile.language).into());
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
    } else if let Some(profile) = current() {
        show_strip_service(&profile);
    }
}

/// The problem's button: where it can be put right.
fn act() {
    let mut problem = VoiceProblem::None;
    with_ui(|ui| problem = ui.global::<Voice>().get_problem());
    cancel();
    match problem {
        VoiceProblem::NoKey | VoiceProblem::RefusedKey | VoiceProblem::BadAddress => with_ui(open_panel),
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

/// What the panel shows of the settings, with the keys' last letters (the database's, or the
/// screenshots' own).
struct PanelView {
    languages: Vec<VoiceLanguage>,
    addable: Vec<VoiceLanguage>,
    speech: VoiceService,
    speech_url: String,
    speech_model: String,
    keywords: String,
    tidy: bool,
    tidying: VoiceService,
    tidy_url: String,
    tidy_model: String,
    same_key: bool,
    language_name: String,
    transcribe_messages: bool,
}

fn panel_view(voice_input: &VoiceInput, key_hint: &dyn Fn(&str, Which) -> Option<String>) -> PanelView {
    let profile = voice_input.current();
    let chip = |code: &str, on: bool| VoiceLanguage { code: code.into(), name: language_name(code).into(), on };
    let languages = voice_input.profiles.iter().map(|other| chip(&other.language, other.language == profile.language)).collect();
    let mut addable: Vec<VoiceLanguage> = Vec::new();
    if !voice_input.profiles.iter().any(|other| other.language == "auto") {
        addable.push(chip("auto", false));
    }
    addable.extend(LANGUAGES.iter().filter(|(_, code)| !voice_input.profiles.iter().any(|other| other.language == *code)).map(|(_, code)| chip(code, false)));
    let speech_format = SpeechFormat::from_name(&profile.format);
    let speech = service_view(
        speech_format.name(),
        Speech::new(&profile.format, &profile.url, &profile.model).map(|speech| (speech.shown(), speech.official(), live(&speech, &profile.language))),
        speech_format.default_url(),
        speech_format.default_model(),
        key_hint(&profile.language, Which::Speech),
    );
    let tidy_format = service::TidyFormat::from_name(&profile.tidy_format);
    let tidying = service_view(
        tidy_format.name(),
        Tidy::new(&profile.tidy_format, &profile.tidy_url, &profile.tidy_model).map(|tidy| (tidy.shown(), tidy.official(), false)),
        tidy_format.default_url(),
        tidy_format.default_model(),
        key_hint(&profile.language, if profile.tidy_same_key { Which::Speech } else { Which::Tidy }),
    );
    PanelView {
        languages,
        addable,
        speech,
        speech_url: profile.url.clone(),
        speech_model: profile.model.clone(),
        keywords: profile.keywords.clone(),
        tidy: profile.tidy,
        tidying,
        tidy_url: profile.tidy_url.clone(),
        tidy_model: profile.tidy_model.clone(),
        same_key: profile.tidy_same_key,
        language_name: language_name(&profile.language),
        transcribe_messages: voice_input.transcribe_messages,
    }
}

fn service_view(format: &str, address: Result<(String, bool, bool), AddressProblem>, url: &str, model: &str, hint: Option<String>) -> VoiceService {
    let (shown, official, live, problem) = match address {
        Ok((shown, official, live)) => (shown, official, live, 0),
        Err(AddressProblem::Malformed) => (String::new(), false, false, 1),
        Err(AddressProblem::Credentials) => (String::new(), false, false, 2),
        Err(AddressProblem::Unencrypted) => (String::new(), false, false, 3),
    };
    VoiceService {
        format: format.into(),
        url_placeholder: url.into(),
        url_problem: problem,
        model_placeholder: model.into(),
        has_key: hint.is_some(),
        key_hint: hint.unwrap_or_default().into(),
        check: KeyCheck::None,
        check_detail: SharedString::new(),
        shown: shown.into(),
        official,
        live,
    }
}

/// The panel drawn again from the settings; the checks' answers stay unless the service changed.
fn show_panel(ui: &MainWindow) {
    let Some(view) = with_settings(|settings| panel_view(&settings.voice_input, &|language, which| crate::database::credential(&key_name(language, which)).as_deref().map(hint)))
    else {
        return;
    };
    apply(ui, view);
}

fn apply(ui: &MainWindow, view: PanelView) {
    let voice = ui.global::<Voice>();
    let keep = |old: VoiceService, mut new: VoiceService| {
        if old.format == new.format && old.shown == new.shown && old.has_key == new.has_key {
            new.check = old.check;
            new.check_detail = old.check_detail;
        }
        new
    };
    voice.set_can_remove(view.languages.len() > 1);
    voice.set_language_rows(lines(view.languages));
    voice.set_addable_rows(lines(view.addable));
    voice.set_speech(keep(voice.get_speech(), view.speech));
    voice.set_tidying(keep(voice.get_tidying(), view.tidying));
    voice.set_speech_url(view.speech_url.into());
    voice.set_speech_model(view.speech_model.into());
    voice.set_keywords(view.keywords.into());
    voice.set_tidy_url(view.tidy_url.into());
    voice.set_tidy_model(view.tidy_model.into());
    voice.set_tidy(view.tidy);
    voice.set_same_key(view.same_key);
    voice.set_language_name(view.language_name.into());
    voice.set_transcribe_messages(view.transcribe_messages);
}

/// Chips in lines of PER_LINE.
fn lines(chips: Vec<VoiceLanguage>) -> ModelRc<VoiceLanguageRow> {
    let lines: Vec<VoiceLanguageRow> = chips.chunks(PER_LINE).map(|line| VoiceLanguageRow { languages: ModelRc::new(VecModel::from(line.to_vec())) }).collect();
    ModelRc::new(VecModel::from(lines))
}

/// The panel for the screenshot test (src/screenshots.rs): `voice_input` as the settings, with
/// keys whose last letters are `hints` (by the database's names).
#[cfg(test)]
pub fn show_for_pictures(ui: &MainWindow, voice_input: &VoiceInput, hints: &[(&str, &str)]) {
    let view = panel_view(voice_input, &|language, which| hints.iter().find(|(name, _)| *name == key_name(language, which)).map(|(_, hint)| hint.to_string()));
    apply(ui, view);
    show_strip_for_pictures(ui, voice_input);
}

#[cfg(test)]
fn show_strip_for_pictures(ui: &MainWindow, voice_input: &VoiceInput) {
    let profile = voice_input.current();
    let speech = Speech::new(&profile.format, &profile.url, &profile.model).ok();
    let voice = ui.global::<Voice>();
    voice.set_service(speech.as_ref().map(Speech::shown).unwrap_or_default().into());
    voice.set_live(speech.as_ref().is_some_and(|speech| live(speech, &profile.language)));
    voice.set_can_switch(voice_input.profiles.len() > 1);
}

/// A main language chosen in the panel: the one used from now on, and its settings shown.
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

/// A main language added: its set starts as a copy of the one in use, keys and all, and is the
/// one used from now on.
fn add_language(code: &str) {
    if code != "auto" && !is_language(code) {
        return;
    }
    let copied = with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        if voice_input.profiles.iter().any(|profile| profile.language == code) {
            return None;
        }
        let from = voice_input.current().clone();
        voice_input.profiles.push(VoiceProfile { language: code.to_string(), ..from.clone() });
        voice_input.language = code.to_string();
        settings.save();
        Some(from.language)
    })
    .flatten();
    if let Some(from) = copied {
        for which in [Which::Speech, Which::Tidy] {
            if let Some(key) = crate::database::credential(&key_name(&from, which))
                && let Err(err) = crate::database::save_credential(&key_name(code, which), &key)
            {
                eprintln!("voice input: cannot copy a key: {err}");
            }
        }
    }
    with_ui(|ui| {
        ui.global::<Voice>().set_adding(false);
        show_panel(ui);
    });
}

/// The main language in use removed, with its keys; the first one left is used.
fn remove_language() {
    let removed = with_settings(|settings| {
        let voice_input = &mut settings.voice_input;
        if voice_input.profiles.len() < 2 {
            return None;
        }
        let language = voice_input.language.clone();
        voice_input.profiles.retain(|profile| profile.language != language);
        voice_input.language = voice_input.profiles[0].language.clone();
        settings.save();
        Some(language)
    })
    .flatten();
    if let Some(language) = removed {
        for which in [Which::Speech, Which::Tidy] {
            let _ = crate::database::remove_credential(&key_name(&language, which));
        }
    }
    with_ui(show_panel);
}

/// A service's format chosen: its own address and model come with it, unless the address is one
/// the two formats share (a relay of OpenAI's speaks both of OpenAI's speech formats).
fn set_format(which: Which, name: &str) {
    with_settings(|settings| {
        let profile = settings.voice_input.current_mut();
        match which {
            Which::Speech => {
                if !service::SPEECH_FORMATS.contains(&name) || profile.format == name {
                    return;
                }
                let shared = SpeechFormat::from_name(&profile.format).default_url() == SpeechFormat::from_name(name).default_url();
                if !shared {
                    profile.url.clear();
                }
                profile.format = name.to_string();
                profile.model.clear();
            }
            Which::Tidy => {
                if !service::TIDY_FORMATS.contains(&name) || profile.tidy_format == name {
                    return;
                }
                profile.tidy_format = name.to_string();
                profile.tidy_url.clear();
                profile.tidy_model.clear();
            }
        }
        settings.save();
    });
    with_ui(show_panel);
    check(which);
}

enum Field {
    Url,
    Model,
    Keywords,
}

/// An address or a model typed: kept as it is typed, and the address's problem shown.
fn edit(which: Which, field: Field, text: &str) {
    with_settings(|settings| {
        let profile = settings.voice_input.current_mut();
        let slot = match (which, &field) {
            (_, Field::Keywords) => &mut profile.keywords,
            (Which::Speech, Field::Url) => &mut profile.url,
            (Which::Speech, Field::Model) => &mut profile.model,
            (Which::Tidy, Field::Url) => &mut profile.tidy_url,
            (Which::Tidy, Field::Model) => &mut profile.tidy_model,
        };
        *slot = text.trim().to_string();
        settings.save();
    });
    if matches!(field, Field::Keywords) {
        return;
    }
    let Some(profile) = current() else { return };
    let address = match which {
        Which::Speech => Speech::new(&profile.format, &profile.url, &profile.model).map(|speech| (speech.shown(), speech.official(), live(&speech, &profile.language))),
        Which::Tidy => Tidy::new(&profile.tidy_format, &profile.tidy_url, &profile.tidy_model).map(|tidy| (tidy.shown(), tidy.official(), false)),
    };
    // The typed text stays as it is; only what is said of it changes.
    let fresh = service_view("", address, "", "", None);
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        let mut shown = if which == Which::Speech { voice.get_speech() } else { voice.get_tidying() };
        shown.url_problem = fresh.url_problem;
        shown.shown = fresh.shown;
        shown.official = fresh.official;
        shown.live = fresh.live;
        shown.check = KeyCheck::None;
        if which == Which::Speech { voice.set_speech(shown) } else { voice.set_tidying(shown) }
    });
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

fn save_key(which: Which, text: &str) {
    let key = text.trim();
    CHECKS.set(CHECKS.get() + 1);
    if !looks_like_a_key(key) {
        set_check(which, KeyCheck::Malformed, "");
        return;
    }
    let Some(language) = current().map(|profile| profile.language) else { return };
    if let Err(err) = crate::database::save_credential(&key_name(&language, which), key) {
        set_check(which, KeyCheck::Failed, &err);
        return;
    }
    with_ui(show_panel);
    check(which);
    // The tidying that takes this key is checked again too.
    if which == Which::Speech && current().is_some_and(|profile| profile.tidy && profile.tidy_same_key) {
        check(Which::Tidy);
    }
}

fn remove_key(which: Which) {
    CHECKS.set(CHECKS.get() + 1);
    if let Some(language) = current().map(|profile| profile.language)
        && let Err(err) = crate::database::remove_credential(&key_name(&language, which))
    {
        eprintln!("voice input: cannot remove the key: {err}");
    }
    with_ui(|ui| {
        show_panel(ui);
        set_check_on(ui, which, KeyCheck::None, "");
    });
}

fn set_check(which: Which, check: KeyCheck, detail: &str) {
    with_ui(|ui| set_check_on(ui, which, check, detail));
}

fn set_check_on(ui: &MainWindow, which: Which, check: KeyCheck, detail: &str) {
    let voice = ui.global::<Voice>();
    let mut shown = if which == Which::Speech { voice.get_speech() } else { voice.get_tidying() };
    shown.check = check;
    shown.check_detail = detail.into();
    if which == Which::Speech { voice.set_speech(shown) } else { voice.set_tidying(shown) }
}

/// The service asked, on a thread of its own, whether it answers at its address and takes the
/// key; the answer shows under it. Nothing is asked of an address that is not one, nor of a
/// vendor's own address without a key.
fn check(which: Which) {
    let Some(profile) = current() else { return };
    let key = key_of(&profile, which);
    let job: Box<dyn FnOnce() -> KeyAnswer + Send> = match which {
        Which::Speech => match Speech::new(&profile.format, &profile.url, &profile.model) {
            Ok(speech) if key.is_some() || !speech.official() => Box::new(move || service::check_speech(&speech, key.as_deref())),
            _ => return set_check(which, KeyCheck::None, ""),
        },
        Which::Tidy => match Tidy::new(&profile.tidy_format, &profile.tidy_url, &profile.tidy_model) {
            Ok(tidy) if profile.tidy && (key.is_some() || !tidy.official()) => Box::new(move || service::check_tidy(&tidy, key.as_deref())),
            _ => return set_check(which, KeyCheck::None, ""),
        },
    };
    let number = CHECKS.get() + 1;
    CHECKS.set(number);
    set_check(which, KeyCheck::Checking, "");
    let spawned = std::thread::Builder::new().name("voice-check".into()).spawn(move || {
        let answer = job();
        let _ = slint::invoke_from_event_loop(move || {
            if number != CHECKS.get() {
                return;
            }
            match answer {
                KeyAnswer::Works => set_check(which, KeyCheck::Works, ""),
                KeyAnswer::Refused => set_check(which, KeyCheck::Refused, ""),
                KeyAnswer::Failed(err) => set_check(which, KeyCheck::Failed, &err),
            }
        });
    });
    if spawned.is_err() {
        set_check(which, KeyCheck::None, "");
    }
}

/// The page of the vendor's keys, for the format chosen.
fn open_key_page(which: Which) {
    let Some(profile) = current() else { return };
    let page = match which {
        Which::Speech => SpeechFormat::from_name(&profile.format).key_page(),
        Which::Tidy => service::TidyFormat::from_name(&profile.tidy_format).key_page(),
    };
    platform::open_link(page);
}

fn set_tidy(on: bool) {
    with_settings(|settings| {
        settings.voice_input.current_mut().tidy = on;
        settings.save();
    });
    with_ui(show_panel);
    if on {
        check(Which::Tidy);
    }
}

fn set_same_key(on: bool) {
    with_settings(|settings| {
        settings.voice_input.current_mut().tidy_same_key = on;
        settings.save();
    });
    with_ui(show_panel);
    check(Which::Tidy);
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
        assert_eq!(key_name("zh-cn", Which::Tidy), "voice_input.zh-cn.tidy");
    }

    #[test]
    fn the_panel_shows_the_profile_in_use_and_what_can_be_added() {
        let mut voice_input = VoiceInput::default();
        voice_input.profiles.push(VoiceProfile { language: "zh-cn".into(), format: "doubao".into(), tidy_same_key: false, ..VoiceProfile::default() });
        voice_input.language = "zh-cn".into();
        let view = panel_view(&voice_input, &|language, which| (language == "zh-cn" && which == Which::Speech).then(|| "6f1…9b2c".to_string()));
        assert_eq!(view.languages.iter().map(|chip| (chip.code.to_string(), chip.on)).collect::<Vec<_>>(), vec![("auto".to_string(), false), ("zh-cn".to_string(), true)]);
        assert_eq!(view.addable.len(), 10);
        assert!(!view.addable.iter().any(|chip| chip.code == "auto" || chip.code == "zh-cn"));
        assert_eq!((view.speech.format.as_str(), view.speech.shown.as_str(), view.speech.has_key), ("doubao", "Volcengine", true));
        assert_eq!(view.speech.model_placeholder.as_str(), "volc.seedasr.sauc.duration");
        assert!(view.speech.live);
        // The tidying has a key of its own here, and none is kept.
        assert!(!view.same_key && !view.tidying.has_key);
        assert_eq!(view.language_name, "简体中文");
        // An address that is not one says why.
        voice_input.current_mut().url = "http://example.com".into();
        let view = panel_view(&voice_input, &|_, _| None);
        assert_eq!(view.speech.url_problem, 3);
    }

    #[test]
    fn words_show_while_speaking_except_for_uploads_and_doubaos_other_languages() {
        let speech = |format: &str, model: &str| Speech::new(format, "", model).expect("the default address");
        assert!(live(&speech("openai-realtime", ""), "auto"));
        assert!(!live(&speech("openai-realtime", "gpt-transcribe"), "auto"));
        assert!(!live(&speech("openai-transcriptions", ""), "zh-cn"));
        assert!(live(&speech("qwen-realtime", ""), "ja"));
        assert!(live(&speech("doubao", ""), "zh-tw") && !live(&speech("doubao", ""), "ja"));
    }
}
