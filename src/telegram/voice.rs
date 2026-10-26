//! Voice messages (TDLib's voiceNote), from the design's fifth round ("秘密聊天与通话", B1 and B2):
//! playing the ones in a chat, and recording and sending new ones from the composer's microphone.
//!
//! A voice message plays through an mpv of its own (player::audio) once TDLib has its file;
//! playing an incoming one tells Telegram it was listened to (openMessageContent), as Telegram's
//! own apps do, and its sender sees that. Which one plays and how far is Conversation's
//! (voice-playing, voice-progress …), not the rows', so that a chat's rows stay as they are while
//! it plays. It stops at its end (the design: no playing on into the next one), and when another
//! chat comes to the front.
//!
//! A new one is recorded by the microphone into an M4A file (platform::VoiceRecorder: AAC, which
//! TDLib takes as a voice message) in the caches folder, and sent as inputMessageVoiceNote with
//! its length and its waveform; TDLib uploads it from there. The microphone is held while
//! speaking: letting go sends, dragged left it cancels, dragged up it locks, and then it records
//! on its own, to be deleted, stopped and listened to, or sent. A click that does not hold says to
//! hold.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use base64::Engine as _;
use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::api::MessageContent as M;
use super::store::{self, duration_text};
use super::{actions, conversation, files, send, with_ui};
use crate::platform::{self, MicrophoneAccess};
use crate::player::audio::{self, Heard};
use crate::{Conversation, MainWindow, RecordStage, Recorder, Voice};

/// A voice message's file is waited for: before anything else of the chat's.
const PRIORITY: i32 = 32;
/// Let go sooner than this, the microphone was clicked, not held: the tip says to hold.
const CLICK: Duration = Duration::from_millis(400);
/// A recording shorter than this is not sent.
const SHORTEST: f64 = 1.0;
/// The longest recording: it is sent by itself then.
const LONGEST: Duration = Duration::from_secs(60 * 60);
/// How often the recording's clock and bars are drawn again.
const TICK: Duration = Duration::from_millis(50);
/// How long the tip to hold stays.
const TIP: Duration = Duration::from_secs(2);
/// Bars a voice message shows in a row, as the design draws it.
const BARS: usize = 28;
/// Block characters of Terminal's waveform.
const BLOCKS: usize = 20;
/// Values in the waveform sent, as Telegram's own apps send it.
const WAVEFORM_VALUES: usize = 100;
/// Bars of the recording's live waveform.
const LIVE_BARS: usize = 22;
/// Recordings are deleted at the start once they are this old: TDLib has long uploaded them.
const KEPT: Duration = Duration::from_secs(24 * 60 * 60);
/// The speeds a voice message plays at, in the order a click goes through them.
const SPEEDS: [(f64, &str); 3] = [(1.0, "1×"), (1.5, "1.5×"), (2.0, "2×")];

thread_local! {
    static PLAYING: RefCell<Option<Playing>> = const { RefCell::new(None) };
    static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) };
    /// The speed voice messages play at, by its place in SPEEDS: kept from one to the next.
    static SPEED: Cell<usize> = const { Cell::new(0) };
    static TIP_TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

/// What plays: a voice message of the chat, or the recording listened to before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Message(i64),
    Preview,
}

/// The sound that plays, or whose file is on its way.
struct Playing {
    target: Target,
    /// In seconds: as the sender said, then as mpv measures it.
    duration: f64,
    paused: bool,
    /// The file is there and mpv plays it.
    loaded: bool,
}

/// The voice message being recorded, or recorded and not yet sent.
struct Recording {
    /// Recording while it is here; gone once stopped.
    recorder: Option<platform::VoiceRecorder>,
    path: PathBuf,
    chat_id: i64,
    started: Instant,
    /// The loudness every tick, 0 … 1.
    levels: Vec<f32>,
    stage: RecordStage,
    /// Its length once stopped, in seconds.
    seconds: f64,
    ticks: Option<Timer>,
}

pub fn connect(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.on_play_voice(|id| {
        if let Ok(message_id) = id.parse() {
            toggle(message_id);
        }
    });
    conversation.on_seek_voice(|id, fraction| {
        if let Ok(message_id) = id.parse() {
            seek(message_id, fraction);
        }
    });
    conversation.on_next_voice_speed(next_speed);
    audio::listen(heard);
    let recorder = ui.global::<Recorder>();
    recorder.on_press(press);
    recorder.on_release(release);
    recorder.on_lock(lock);
    recorder.on_stop(stop_recording);
    recorder.on_send(send_recording);
    recorder.on_delete(delete_recording);
    recorder.on_play_preview(play_preview);
    recorder.on_open_microphone_settings(|| {
        with_ui(|ui| ui.global::<Recorder>().set_denied(false));
        platform::open_microphone_settings();
    });
    recorder.on_dismiss_denied(|| with_ui(|ui| ui.global::<Recorder>().set_denied(false)));
    delete_old_recordings();
}

// ---- playing -----------------------------------------------------------------------------------

/// Play the voice message, or pause it when it plays, or go on when it is paused; again after its
/// file could not be had.
fn toggle(message_id: i64) {
    let current = PLAYING.with(|playing| {
        playing.borrow().as_ref().filter(|playing| playing.target == Target::Message(message_id)).map(|playing| (playing.loaded, playing.paused))
    });
    match current {
        Some((true, paused)) => audio::set_paused(!paused),
        // Its file is still on its way: a second click stops it.
        Some((false, _)) => stop(),
        None => start(message_id, 0.0),
    }
}

/// A click on the waveform: from there, playing it if it does not yet.
fn seek(message_id: i64, fraction: f32) {
    let fraction = f64::from(fraction.clamp(0.0, 1.0));
    let loaded = PLAYING.with(|playing| {
        playing.borrow().as_ref().filter(|playing| playing.target == Target::Message(message_id) && playing.loaded).map(|playing| playing.duration)
    });
    match loaded {
        Some(duration) => audio::seek(fraction * duration),
        None => start(message_id, fraction),
    }
}

/// The voice message's file fetched, then played from `fraction` of it.
fn start(message_id: i64, fraction: f64) {
    stop();
    let found = store::with(|store| {
        let chat_id = store.open?;
        let message = store.histories.get(&chat_id)?.messages.get(&message_id)?;
        match &message.content {
            M::VoiceNote { voice_note, is_listened, .. } => Some((chat_id, voice_note.voice.clone(), voice_note.duration, !message.is_outgoing && !is_listened)),
            _ => None,
        }
    })
    .flatten();
    let Some((chat_id, file, duration, unheard)) = found else { return };
    let duration = f64::from(duration.max(1));
    PLAYING.with(|playing| *playing.borrow_mut() = Some(Playing { target: Target::Message(message_id), duration, paused: false, loaded: false }));
    with_ui(|ui| {
        let conversation = ui.global::<Conversation>();
        conversation.set_voice_playing(message_id.to_string().into());
        conversation.set_voice_loading(true);
        conversation.set_voice_failed(false);
        conversation.set_voice_paused(false);
        conversation.set_voice_progress(fraction as f32);
        conversation.set_voice_time(duration_text((fraction * duration) as i32).into());
    });
    // Telegram's apps count a voice message listened to when it plays.
    if unheard {
        send(json!({ "@type": "openMessageContent", "chat_id": chat_id, "message_id": message_id }), |_| {});
    }
    files::download_or_fail(
        &file,
        PRIORITY,
        move |path| {
            if take_loaded(Target::Message(message_id)) {
                audio::play(&path, fraction * duration, SPEEDS[SPEED.get()].0);
                with_ui(|ui| ui.global::<Conversation>().set_voice_loading(false));
            }
        },
        move || {
            // The file could not be had: the bubble says so, and a click tries again.
            let wanted = PLAYING.with(|playing| playing.borrow().as_ref().is_some_and(|playing| playing.target == Target::Message(message_id) && !playing.loaded));
            if wanted {
                PLAYING.with(|playing| *playing.borrow_mut() = None);
                with_ui(|ui| {
                    let conversation = ui.global::<Conversation>();
                    conversation.set_voice_loading(false);
                    conversation.set_voice_failed(true);
                });
            }
        },
    );
}

/// Whether the file that came is still wanted by what waits for it: it is then marked loaded.
fn take_loaded(target: Target) -> bool {
    PLAYING.with(|playing| {
        let mut playing = playing.borrow_mut();
        match playing.as_mut() {
            Some(playing) if playing.target == target && !playing.loaded => {
                playing.loaded = true;
                true
            }
            _ => false,
        }
    })
}

/// What the player says of what plays.
fn heard(what: Heard) {
    let Some((target, duration)) = PLAYING.with(|playing| playing.borrow().as_ref().filter(|playing| playing.loaded).map(|playing| (playing.target, playing.duration))) else {
        return;
    };
    match what {
        Heard::Position(seconds) => {
            let progress = (seconds / duration).clamp(0.0, 1.0) as f32;
            with_ui(|ui| match target {
                Target::Message(_) => {
                    let conversation = ui.global::<Conversation>();
                    conversation.set_voice_progress(progress);
                    conversation.set_voice_time(duration_text(seconds as i32).into());
                }
                Target::Preview => ui.global::<Recorder>().set_preview_progress(progress),
            });
        }
        Heard::Duration(seconds) if seconds > 0.0 => PLAYING.with(|playing| {
            if let Some(playing) = playing.borrow_mut().as_mut() {
                playing.duration = seconds;
            }
        }),
        Heard::Duration(_) => {}
        Heard::Paused(paused) => {
            PLAYING.with(|playing| {
                if let Some(playing) = playing.borrow_mut().as_mut() {
                    playing.paused = paused;
                }
            });
            with_ui(|ui| match target {
                Target::Message(_) => ui.global::<Conversation>().set_voice_paused(paused),
                Target::Preview => ui.global::<Recorder>().set_preview_playing(!paused),
            });
        }
        Heard::Ended | Heard::Failed => stop(),
    }
}

/// 1× → 1.5× → 2× → 1×, for what plays now and the ones after it.
fn next_speed() {
    let next = (SPEED.get() + 1) % SPEEDS.len();
    SPEED.set(next);
    audio::set_speed(SPEEDS[next].0);
    with_ui(|ui| ui.global::<Conversation>().set_voice_speed(SPEEDS[next].1.into()));
}

/// Nothing plays.
pub fn stop() {
    let Some(was) = PLAYING.with(|playing| playing.borrow_mut().take()) else { return };
    audio::stop();
    with_ui(|ui| match was.target {
        Target::Message(_) => {
            let conversation = ui.global::<Conversation>();
            conversation.set_voice_playing(SharedString::new());
            conversation.set_voice_loading(false);
            conversation.set_voice_failed(false);
            conversation.set_voice_paused(false);
            conversation.set_voice_progress(0.0);
            conversation.set_voice_time(SharedString::new());
        }
        Target::Preview => {
            let recorder = ui.global::<Recorder>();
            recorder.set_preview_playing(false);
            recorder.set_preview_progress(0.0);
        }
    });
}

/// Another chat came to the front: the voice message that plays stops, and a recording is
/// dropped (it was for the chat that left).
pub fn chat_changed() {
    stop();
    let recording = RECORDING.with(|recording| recording.borrow().is_some());
    if recording {
        delete_recording();
    }
}

// ---- recording ---------------------------------------------------------------------------------

/// The microphone pressed down: a recording starts, held.
fn press() {
    if RECORDING.with(|recording| recording.borrow().is_some()) {
        return;
    }
    // The microphone is voice input's while it listens.
    let mut dictating = false;
    with_ui(|ui| dictating = ui.global::<Voice>().get_busy());
    if dictating {
        return;
    }
    let Some(chat_id) = store::with(|store| store.open).flatten() else { return };
    with_ui(|ui| {
        let recorder = ui.global::<Recorder>();
        recorder.set_denied(false);
        recorder.set_too_short(false);
        recorder.set_cancelling(false);
    });
    match platform::microphone_access() {
        MicrophoneAccess::Allowed => begin_recording(chat_id),
        MicrophoneAccess::Denied => with_ui(|ui| ui.global::<Recorder>().set_denied(true)),
        // The system asks the user, once; the next press records.
        MicrophoneAccess::Undetermined => platform::ask_for_microphone(|allowed| {
            if !allowed {
                let _ = slint::invoke_from_event_loop(|| with_ui(|ui| ui.global::<Recorder>().set_denied(true)));
            }
        }),
    }
}

/// The microphone let go: held long enough, the recording is sent, or dropped when it was dragged
/// away to cancel; a click says to hold. Once locked, letting go changes nothing.
fn release() {
    let held = RECORDING.with(|recording| recording.borrow().as_ref().filter(|recording| recording.stage == RecordStage::Holding).map(|recording| recording.started.elapsed()));
    let Some(held) = held else { return };
    let mut cancelling = false;
    with_ui(|ui| cancelling = ui.global::<Recorder>().get_cancelling());
    if held < CLICK {
        delete_recording();
        show_hold_tip();
    } else if cancelling {
        delete_recording();
    } else {
        send_recording();
    }
}

/// Dragged up while held: it records on its own.
fn lock() {
    let locked = RECORDING.with(|recording| {
        let mut recording = recording.borrow_mut();
        match recording.as_mut() {
            Some(recording) if recording.stage == RecordStage::Holding => {
                recording.stage = RecordStage::Locked;
                true
            }
            _ => false,
        }
    });
    if locked {
        with_ui(|ui| {
            let recorder = ui.global::<Recorder>();
            recorder.set_cancelling(false);
            recorder.set_stage(RecordStage::Locked);
        });
    }
}

fn begin_recording(chat_id: i64) {
    stop();
    let Some(folder) = recordings_folder() else { return };
    if let Err(err) = std::fs::create_dir_all(&folder) {
        eprintln!("voice: cannot make {}: {err}", folder.display());
        return;
    }
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|since| since.as_millis()).unwrap_or(0);
    let path = folder.join(format!("voice-{stamp}.m4a"));
    let recorder = match platform::VoiceRecorder::start(&path) {
        Ok(recorder) => recorder,
        Err(err) => {
            eprintln!("voice: cannot record: {err}");
            with_ui(|ui| ui.global::<Recorder>().set_denied(true));
            return;
        }
    };
    let ticks = Timer::default();
    ticks.start(TimerMode::Repeated, TICK, tick);
    RECORDING.with(|recording| {
        *recording.borrow_mut() = Some(Recording {
            recorder: Some(recorder),
            path,
            chat_id,
            started: Instant::now(),
            levels: Vec::new(),
            stage: RecordStage::Holding,
            seconds: 0.0,
            ticks: Some(ticks),
        })
    });
    with_ui(|ui| {
        let recorder = ui.global::<Recorder>();
        recorder.set_time(recording_time(Duration::ZERO).into());
        recorder.set_levels(ModelRc::new(VecModel::from(vec![0.0; LIVE_BARS])));
        recorder.set_stage(RecordStage::Holding);
    });
}

/// The clock and the live bars; the longest recording sends itself.
fn tick() {
    let now = RECORDING.with(|recording| {
        let mut recording = recording.borrow_mut();
        let recording = recording.as_mut()?;
        let level = recording.recorder.as_ref()?.level();
        recording.levels.push(level);
        let mut shown: Vec<f32> = recording.levels.iter().rev().take(LIVE_BARS).rev().copied().collect();
        // Fewer than the bars yet: the rest wait, silent, at the start.
        while shown.len() < LIVE_BARS {
            shown.insert(0, 0.0);
        }
        Some((recording.started.elapsed(), shown))
    });
    let Some((elapsed, shown)) = now else { return };
    with_ui(|ui| {
        let recorder = ui.global::<Recorder>();
        recorder.set_time(recording_time(elapsed).into());
        recorder.set_levels(ModelRc::new(VecModel::from(shown)));
    });
    if elapsed >= LONGEST {
        // After this tick: sending drops the timer that called it.
        Timer::single_shot(Duration::ZERO, send_recording);
    }
}

/// Locked, the user stops it: it can be listened to before it goes.
fn stop_recording() {
    let stopped = RECORDING.with(|recording| {
        let mut recording = recording.borrow_mut();
        let recording = recording.as_mut()?;
        let recorder = recording.recorder.take()?;
        recording.seconds = recorder.finish();
        recording.stage = RecordStage::Preview;
        recording.ticks = None;
        Some((recording.seconds, waveform_of(&recording.levels)))
    });
    let Some((seconds, waveform)) = stopped else { return };
    let bars = bars_of_values(&waveform, BARS);
    with_ui(|ui| {
        let recorder = ui.global::<Recorder>();
        recorder.set_preview_bars(ModelRc::new(VecModel::from(bars)));
        recorder.set_preview_time(duration_text(seconds.round() as i32).into());
        recorder.set_preview_progress(0.0);
        recorder.set_preview_playing(false);
        recorder.set_stage(RecordStage::Preview);
    });
}

/// The recording, stopped, listened to; paused when it plays.
fn play_preview() {
    let current = PLAYING.with(|playing| playing.borrow().as_ref().filter(|playing| playing.target == Target::Preview).map(|playing| playing.paused));
    if let Some(paused) = current {
        audio::set_paused(!paused);
        return;
    }
    let Some((path, seconds)) = RECORDING.with(|recording| {
        recording.borrow().as_ref().filter(|recording| recording.stage == RecordStage::Preview).map(|recording| (recording.path.clone(), recording.seconds))
    }) else {
        return;
    };
    stop();
    PLAYING.with(|playing| *playing.borrow_mut() = Some(Playing { target: Target::Preview, duration: seconds.max(0.1), paused: false, loaded: true }));
    audio::play(&path.to_string_lossy(), 0.0, 1.0);
    with_ui(|ui| ui.global::<Recorder>().set_preview_playing(true));
}

/// The recording goes to its chat as a voice message: a reply when the strip above the composer
/// says so.
fn send_recording() {
    let Some(recording) = RECORDING.with(|recording| recording.borrow_mut().take()) else { return };
    if PLAYING.with(|playing| playing.borrow().as_ref().is_some_and(|playing| playing.target == Target::Preview)) {
        stop();
    }
    let Recording { recorder, path, chat_id, levels, seconds, .. } = recording;
    let seconds = recorder.map_or(seconds, platform::VoiceRecorder::finish);
    with_ui(reset_recorder);
    if seconds < SHORTEST {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let waveform = base64::engine::general_purpose::STANDARD.encode(pack_waveform(&waveform_of(&levels)));
    let reply_to = actions::take_reply(chat_id).map(|message_id| {
        json!({ "@type": "inputMessageReplyToMessage", "message_id": message_id, "quote": null, "checklist_task_id": 0, "poll_option_id": "" })
    });
    conversation::back_to_end(chat_id);
    let request = json!({
        "@type": "sendMessage", "chat_id": chat_id, "topic_id": super::topics::topic_json(chat_id), "reply_to": reply_to,
        "options": null, "reply_markup": null,
        "input_message_content": {
            "@type": "inputMessageVoiceNote",
            "voice_note": {
                "@type": "inputVoiceNote",
                "voice_note": { "@type": "inputFileLocal", "path": path.to_string_lossy() },
                "duration": seconds.round().max(1.0) as i32,
                "waveform": waveform,
            },
            "caption": null,
            "self_destruct_type": null,
        },
    });
    send(request, move |answer| conversation::sent(chat_id, answer));
    store::with(|store| store.dirty.scroll_to_end = true);
}

/// The recording is dropped, its file with it.
fn delete_recording() {
    let Some(recording) = RECORDING.with(|recording| recording.borrow_mut().take()) else { return };
    if PLAYING.with(|playing| playing.borrow().as_ref().is_some_and(|playing| playing.target == Target::Preview)) {
        stop();
    }
    let path = recording.path.clone();
    // Dropped while it records, the recorder deletes its file; stopped, the file is still there.
    drop(recording);
    let _ = std::fs::remove_file(&path);
    with_ui(reset_recorder);
}

fn reset_recorder(ui: &MainWindow) {
    let recorder = ui.global::<Recorder>();
    recorder.set_stage(RecordStage::Idle);
    recorder.set_time(SharedString::new());
    recorder.set_levels(ModelRc::default());
    recorder.set_cancelling(false);
    recorder.set_preview_bars(ModelRc::default());
    recorder.set_preview_time(SharedString::new());
    recorder.set_preview_playing(false);
    recorder.set_preview_progress(0.0);
}

/// "Hold to record. Release to send.", for a moment.
fn show_hold_tip() {
    with_ui(|ui| ui.global::<Recorder>().set_too_short(true));
    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, TIP, || with_ui(|ui| ui.global::<Recorder>().set_too_short(false)));
    TIP_TIMER.with(|slot| *slot.borrow_mut() = Some(timer));
}

/// The recording's clock as the design writes it: "0:07,4".
fn recording_time(elapsed: Duration) -> String {
    let tenths = elapsed.as_millis() / 100;
    let seconds = tenths / 10;
    format!("{}:{:02},{}", seconds / 60, seconds % 60, tenths % 10)
}

fn recordings_folder() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("FinchGram").join("voice"))
}

/// Recordings TDLib has long uploaded go.
fn delete_old_recordings() {
    let Some(folder) = recordings_folder() else { return };
    let Ok(entries) = std::fs::read_dir(&folder) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let old = entry.metadata().and_then(|metadata| metadata.modified()).ok().and_then(|modified| modified.elapsed().ok()).is_some_and(|age| age > KEPT);
        if old && is_recording(&path) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn is_recording(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("voice-") && name.ends_with(".m4a"))
}

// ---- the waveform ------------------------------------------------------------------------------

/// TDLib's waveform, values of 5 bits packed with the first in the lowest bits of the first byte,
/// unpacked: 0 … 31 each.
pub fn unpack_waveform(bytes: &[u8]) -> Vec<u8> {
    let count = bytes.len() * 8 / 5;
    (0..count)
        .map(|index| {
            let bit = index * 5;
            let low = u16::from(bytes[bit / 8]);
            let high = u16::from(bytes.get(bit / 8 + 1).copied().unwrap_or(0));
            (((low | (high << 8)) >> (bit % 8)) & 0x1f) as u8
        })
        .collect()
}

/// Values of 0 … 31 packed as TDLib takes them.
pub fn pack_waveform(values: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; (values.len() * 5).div_ceil(8)];
    for (index, value) in values.iter().enumerate() {
        let bit = index * 5;
        let word = u16::from((*value).min(31)) << (bit % 8);
        bytes[bit / 8] |= (word & 0xff) as u8;
        if let Some(next) = bytes.get_mut(bit / 8 + 1) {
            *next |= (word >> 8) as u8;
        }
    }
    bytes
}

/// The recording's loudness as the waveform sent: WAVEFORM_VALUES values, the loudest 31.
fn waveform_of(levels: &[f32]) -> Vec<u8> {
    let peak = levels.iter().copied().fold(0.0f32, f32::max);
    if levels.is_empty() || peak <= 0.0 {
        return vec![0; WAVEFORM_VALUES];
    }
    loudest_in_each(levels, WAVEFORM_VALUES).into_iter().map(|level| (level / peak * 31.0).round().clamp(0.0, 31.0) as u8).collect()
}

/// `count` values from `values`: the loudest in each of `count` equal parts of it.
fn loudest_in_each(values: &[f32], count: usize) -> Vec<f32> {
    (0..count)
        .map(|part| {
            let start = part * values.len() / count;
            let end = ((part + 1) * values.len() / count).max(start + 1).min(values.len());
            values[start.min(values.len() - 1)..end].iter().copied().fold(0.0f32, f32::max)
        })
        .collect()
}

/// Waveform values (0 … 31) as `count` bars, 0 … 1, the loudest full height; flat for none.
fn bars_of_values(values: &[u8], count: usize) -> Vec<f32> {
    let values: Vec<f32> = values.iter().copied().map(f32::from).collect();
    let peak = values.iter().copied().fold(0.0f32, f32::max);
    if values.is_empty() || peak <= 0.0 {
        return vec![0.0; count];
    }
    loudest_in_each(&values, count).into_iter().map(|value| value / peak).collect()
}

/// A voice message's waveform (base64, as TDLib gives it) as the bars a row shows, and as
/// Terminal's block characters.
pub fn bars_of(waveform: &str) -> (Vec<f32>, String) {
    let values = base64::engine::general_purpose::STANDARD.decode(waveform).map(|bytes| unpack_waveform(&bytes)).unwrap_or_default();
    let blocks = bars_of_values(&values, BLOCKS).into_iter().map(block).collect();
    (bars_of_values(&values, BARS), blocks)
}

/// A bar of 0 … 1 as one of the eight block characters.
fn block(height: f32) -> char {
    const BLOCK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    BLOCK[((height * 8.0) as usize).min(7)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_waveform_packs_five_bits_a_value_as_telegram_does() {
        // 31, 0, 31: 11111 00000 11111, the first in the lowest bits.
        assert_eq!(pack_waveform(&[31, 0, 31]), vec![0b0001_1111, 0b0111_1100]);
        assert_eq!(pack_waveform(&[1, 2, 3, 4]), vec![0b0100_0001, 0b0000_1100, 0b0000_0010]);
        let values: Vec<u8> = (0..100).map(|index| (index * 7 % 32) as u8).collect();
        let packed = pack_waveform(&values);
        assert_eq!(packed.len(), 63);
        assert_eq!(unpack_waveform(&packed), values);
        // Values above 31 are 31.
        assert_eq!(unpack_waveform(&pack_waveform(&[40, 255])), vec![31, 31, 0]);
    }

    #[test]
    fn a_recordings_loudness_becomes_a_hundred_values_with_its_peak_at_31() {
        let levels: Vec<f32> = (0..400).map(|index| if index < 200 { 0.25 } else { 0.5 }).collect();
        let waveform = waveform_of(&levels);
        assert_eq!(waveform.len(), WAVEFORM_VALUES);
        assert_eq!((waveform[0], waveform[49], waveform[50], waveform[99]), (16, 16, 31, 31));
        // A short recording: each value from the level of its part, none missing.
        let waveform = waveform_of(&[0.1, 0.4]);
        assert_eq!(waveform.len(), WAVEFORM_VALUES);
        assert_eq!((waveform[0], waveform[99]), (8, 31));
        assert_eq!(waveform_of(&[]), vec![0; WAVEFORM_VALUES]);
        assert_eq!(waveform_of(&[0.0; 10]), vec![0; WAVEFORM_VALUES]);
    }

    #[test]
    fn a_row_shows_the_designs_28_bars_and_terminal_20_blocks() {
        let values: Vec<u8> = (0..100).map(|index| if index == 50 { 31 } else { 10 }).collect();
        let waveform = base64::engine::general_purpose::STANDARD.encode(pack_waveform(&values));
        let (bars, blocks) = bars_of(&waveform);
        assert_eq!(bars.len(), BARS);
        assert_eq!(bars.iter().copied().fold(0.0f32, f32::max), 1.0);
        assert!((bars[0] - 10.0 / 31.0).abs() < 0.001, "{}", bars[0]);
        assert_eq!(blocks.chars().count(), BLOCKS);
        assert!(blocks.contains('█') && blocks.starts_with('▃'), "{blocks}");
        // No waveform, or one that is not base64: flat.
        assert_eq!(bars_of("").0, vec![0.0; BARS]);
        assert_eq!(bars_of("not base64!").1, "▁".repeat(BLOCKS));
    }

    #[test]
    fn the_recordings_clock_has_tenths_as_the_design_writes_them() {
        assert_eq!(recording_time(Duration::from_millis(7_400)), "0:07,4");
        assert_eq!(recording_time(Duration::from_millis(72_850)), "1:12,8");
        assert_eq!(recording_time(Duration::ZERO), "0:00,0");
    }

    #[test]
    fn only_finchgrams_own_recordings_are_cleaned_up() {
        assert!(is_recording(Path::new("/x/voice-1761400000000.m4a")));
        assert!(!is_recording(Path::new("/x/voice-1761400000000.ogg")));
        assert!(!is_recording(Path::new("/x/notes.m4a")));
    }
}
