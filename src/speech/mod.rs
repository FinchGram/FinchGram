//! Voice input (docs/voice-input.md): the user presses the sparkle in the composer (in Terminal,
//! types /voice) and speaks; the words come back from OpenAI's realtime transcription while they
//! speak, under the user's own key, and show in the strip above the composer; when the user is done
//! (the sparkle again, ⏎, or letting go of it after holding it), a small model tidies them and
//! they go into the composer, for the user to send. Esc drops them. The sound goes from the
//! microphone to OpenAI and nowhere else, and FinchGram keeps no recording. (The microphone in the
//! composer is voice messages', src/telegram/voice.rs.)
//!
//! One dictation at a time, its state here on the UI thread. The microphone's samples come from
//! one of the system's threads (src/platform/microphone.rs); the session with OpenAI runs on a
//! thread of its own ("voice", `openai::dictate`), which reports back through the event loop. The
//! key is the database's (`database::credential`), never in settings.toml; the languages and the
//! tidying are settings (`[voice_input]`).

mod audio;
mod openai;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::platform::{self, MicrophoneAccess};
use crate::settings::Settings;
use crate::{AppState, Conversation, KeyCheck, MainWindow, Page, Recorder, SettingsSection, Voice, VoiceLanguage, VoiceLanguageRow, VoiceProblem, VoiceState};
use openai::{FailureKind, Input, KeyAnswer, Options, Report};

/// The database's name for the user's OpenAI key.
const KEY: &str = "openai_api_key";
/// Where the user gets one.
const KEY_PAGE: &str = "https://platform.openai.com/api-keys";
/// Held longer than this, the microphone finishes when let go; a shorter press is a click.
const HOLD: Duration = Duration::from_millis(600);
/// The longest dictation: it finishes by itself then, so that a microphone left open does not go
/// on costing the user.
const LONGEST: Duration = Duration::from_secs(5 * 60);
/// How often the bars and the clock are drawn again.
const TICK: Duration = Duration::from_millis(50);
/// The languages offered, in the order of the UI's language list: its code there, and OpenAI's.
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
/// Languages per line of chips in the settings.
const PER_LINE: usize = 4;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DICTATION: RefCell<Option<Dictation>> = const { RefCell::new(None) };
    /// Counted up with every dictation and every cancel: what an older one's thread still says is
    /// not heard.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    /// When the microphone was pressed to start the dictation, while it is still held.
    static PRESSED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Counted up with every check of a key, so that only the last one's answer shows.
    static KEY_CHECKS: Cell<u64> = const { Cell::new(0) };
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
    let voice = ui.global::<Voice>();
    voice.set_tidy(settings.borrow().voice_input.tidy);
    APP.with(|app| *app.borrow_mut() = Some(App { ui: ui.as_weak(), settings }));
    show_key(ui);
    show_languages(ui);
    voice.on_press(press);
    voice.on_release(release);
    voice.on_stop(finish);
    voice.on_cancel(cancel);
    voice.on_act(act);
    voice.on_save_key(|text| save_key(&text));
    voice.on_remove_key(remove_key);
    voice.on_open_key_page(|| platform::open_link(KEY_PAGE));
    voice.on_toggle_language(|code| toggle_language(&code));
    voice.on_set_tidy(set_tidy);
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

// ---- a dictation -------------------------------------------------------------------------------

/// The sparkle pressed: a dictation starts, or the one listening finishes (a second click). Not
/// while a voice message records: the microphone is its.
fn press() {
    let mut recording = false;
    with_ui(|ui| recording = ui.global::<Recorder>().get_busy());
    if recording {
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
    let Some(key) = crate::database::credential(KEY) else {
        fail(VoiceProblem::NoKey, "");
        return;
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
        MicrophoneAccess::Allowed => begin(generation, key),
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
                        begin(generation, key);
                    } else {
                        fail(VoiceProblem::MicrophoneDenied, "");
                    }
                });
            });
        }
    }
}

/// The microphone on, and the session's thread started.
fn begin(generation: u64, key: String) {
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
    let options = with_settings(|settings| {
        let voice_input = &settings.voice_input;
        Options {
            key,
            model: voice_input.model.clone(),
            languages: voice_input.languages.clone(),
            tidy_model: voice_input.tidy.then(|| voice_input.tidy_model.clone()),
        }
    });
    let Some(options) = options else { return };
    let spawned = std::thread::Builder::new().name("voice".into()).spawn(move || {
        openai::dictate(&options, &received, &|report| {
            let _ = slint::invoke_from_event_loop(move || reported(generation, report));
        });
    });
    if let Err(err) = spawned {
        fail(VoiceProblem::Openai, &err.to_string());
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

/// The problem's button: where it can be put right.
fn act() {
    let mut problem = VoiceProblem::None;
    with_ui(|ui| problem = ui.global::<Voice>().get_problem());
    cancel();
    match problem {
        VoiceProblem::NoKey | VoiceProblem::RefusedKey => with_ui(|ui| {
            let app = ui.global::<AppState>();
            app.set_settings_section(SettingsSection::General);
            app.set_page(Page::Settings);
        }),
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
        Report::Failed(failure) => {
            let problem = match failure.kind {
                FailureKind::RefusedKey => VoiceProblem::RefusedKey,
                FailureKind::Quota => VoiceProblem::Quota,
                FailureKind::Network => VoiceProblem::Network,
                FailureKind::OpenAi => VoiceProblem::Openai,
            };
            fail(problem, &failure.detail);
        }
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

// ---- Settings → General → Voice input ----------------------------------------------------------

fn show_key(ui: &MainWindow) {
    let key = crate::database::credential(KEY);
    let voice = ui.global::<Voice>();
    voice.set_has_key(key.is_some());
    voice.set_key_hint(key.as_deref().map(hint).unwrap_or_default().into());
}

/// The key as the settings show it: its last four characters.
fn hint(key: &str) -> String {
    let tail: Vec<char> = key.chars().rev().take(4).collect();
    format!("sk-…{}", tail.iter().rev().collect::<String>())
}

/// What could be an OpenAI key: they all start with sk-, and are long and of plain characters.
fn looks_like_a_key(key: &str) -> bool {
    key.starts_with("sk-") && key.len() >= 20 && key.chars().all(|character| character.is_ascii_graphic())
}

fn save_key(text: &str) {
    let key = text.trim();
    KEY_CHECKS.set(KEY_CHECKS.get() + 1);
    if !looks_like_a_key(key) {
        with_ui(|ui| ui.global::<Voice>().set_key_check(KeyCheck::Malformed));
        return;
    }
    if let Err(err) = crate::database::save_credential(KEY, key) {
        with_ui(|ui| {
            let voice = ui.global::<Voice>();
            voice.set_key_error(err.into());
            voice.set_key_check(KeyCheck::Failed);
        });
        return;
    }
    with_ui(show_key);
    check_key(key.to_string());
}

/// OpenAI asked whether it takes the key, on a thread of its own; the answer shows under it.
fn check_key(key: String) {
    let check = KEY_CHECKS.get();
    with_ui(|ui| ui.global::<Voice>().set_key_check(KeyCheck::Checking));
    let spawned = std::thread::Builder::new().name("voice-key".into()).spawn(move || {
        let answer = openai::check_key(&key);
        let _ = slint::invoke_from_event_loop(move || key_checked(check, answer));
    });
    if spawned.is_err() {
        with_ui(|ui| ui.global::<Voice>().set_key_check(KeyCheck::None));
    }
}

fn key_checked(check: u64, answer: KeyAnswer) {
    if check != KEY_CHECKS.get() {
        return;
    }
    with_ui(|ui| {
        let voice = ui.global::<Voice>();
        match answer {
            KeyAnswer::Works => voice.set_key_check(KeyCheck::Works),
            KeyAnswer::Refused => voice.set_key_check(KeyCheck::Refused),
            KeyAnswer::Failed(err) => {
                voice.set_key_error(err.into());
                voice.set_key_check(KeyCheck::Failed);
            }
        }
    });
}

fn remove_key() {
    KEY_CHECKS.set(KEY_CHECKS.get() + 1);
    if let Err(err) = crate::database::remove_credential(KEY) {
        eprintln!("voice input: cannot remove the key: {err}");
    }
    with_ui(|ui| {
        show_key(ui);
        ui.global::<Voice>().set_key_check(KeyCheck::None);
    });
}

fn show_languages(ui: &MainWindow) {
    let chosen = with_settings(|settings| settings.voice_input.languages.clone()).unwrap_or_default();
    ui.global::<Voice>().set_language_rows(language_lines(&chosen));
}

/// The languages as the settings' lines of chips, those in `chosen` on.
fn language_lines(chosen: &[String]) -> ModelRc<VoiceLanguageRow> {
    let languages: Vec<VoiceLanguage> = LANGUAGES
        .iter()
        .map(|(ui_code, code)| VoiceLanguage { code: (*code).into(), name: crate::i18n::native_name(ui_code).into(), on: chosen.iter().any(|chosen| chosen == code) })
        .collect();
    let lines: Vec<VoiceLanguageRow> = languages.chunks(PER_LINE).map(|line| VoiceLanguageRow { languages: ModelRc::new(VecModel::from(line.to_vec())) }).collect();
    ModelRc::new(VecModel::from(lines))
}

/// The settings' languages for the screenshot test (src/screenshots.rs), with `chosen` on.
#[cfg(test)]
pub fn show_for_pictures(ui: &MainWindow, chosen: &[&str]) {
    let chosen: Vec<String> = chosen.iter().map(|code| code.to_string()).collect();
    ui.global::<Voice>().set_language_rows(language_lines(&chosen));
}

fn toggle_language(code: &str) {
    if !LANGUAGES.iter().any(|(_, known)| *known == code) {
        return;
    }
    with_settings(|settings| {
        let languages = &mut settings.voice_input.languages;
        match languages.iter().position(|chosen| chosen == code) {
            Some(index) => {
                languages.remove(index);
            }
            None => languages.push(code.to_string()),
        }
        languages.sort_by_key(|chosen| LANGUAGES.iter().position(|(_, known)| known == chosen).unwrap_or(usize::MAX));
        settings.save();
    });
    with_ui(show_languages);
}

fn set_tidy(on: bool) {
    with_settings(|settings| {
        settings.voice_input.tidy = on;
        settings.save();
    });
    with_ui(|ui| ui.global::<Voice>().set_tidy(on));
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
    fn a_key_shows_by_its_last_letters_and_only_openais_look_is_taken() {
        assert_eq!(hint("sk-proj-abcdefghijklmnop4f2a"), "sk-…4f2a");
        assert!(looks_like_a_key("sk-proj-abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(!looks_like_a_key("sk-short"));
        assert!(!looks_like_a_key("pk-abcdefghijklmnopqrstuvwxyz"));
        assert!(!looks_like_a_key("sk-abcdefghij klmnopqrstuvwxyz"));
        assert!(!looks_like_a_key("sk-abcdefghijklmnopqrstuvwxyz中"));
    }

    #[test]
    fn the_languages_offered_are_the_uis_by_openais_codes() {
        assert_eq!(LANGUAGES.len(), crate::i18n::LANGUAGES.len());
        for ((ui_code, _), language) in LANGUAGES.iter().zip(crate::i18n::LANGUAGES) {
            assert_eq!(*ui_code, language.code);
        }
    }
}
