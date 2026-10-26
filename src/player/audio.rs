//! Sound alone through libmpv: voice messages (src/telegram/voice.rs). An mpv of its own, with no
//! video output, so that a voice message and the media viewer never take each other's player. What
//! it reports goes to the one listener, on the UI thread: how far it has played, the length, the
//! end.
//!
//! Everything runs on the UI thread; mpv's own threads only ask the event loop to look.

use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use super::mpv;

/// What mpv reports, by the number given when observing it.
const POSITION: u64 = 1;
const DURATION: u64 = 2;
const PAUSED: u64 = 3;

/// What the player tells its listener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Heard {
    /// How far it has played, in seconds.
    Position(f64),
    /// The file's length, in seconds, once mpv knows it.
    Duration(f64),
    Paused(bool),
    /// Played to its end.
    Ended,
    /// The file could not be played.
    Failed,
}

struct Audio {
    /// Created when the first sound plays.
    mpv: *mut mpv::mpv_handle,
    loaded: bool,
}

thread_local! {
    static AUDIO: RefCell<Audio> = const { RefCell::new(Audio { mpv: ptr::null_mut(), loaded: false }) };
    static LISTENER: Cell<Option<fn(Heard)>> = const { Cell::new(None) };
}

/// Set while the UI thread has been asked to look and has not yet: mpv wakes it once.
static EVENTS_ASKED: AtomicBool = AtomicBool::new(false);

/// Who hears what the player says: one, set once.
pub fn listen(listener: fn(Heard)) {
    LISTENER.with(|slot| slot.set(Some(listener)));
}

/// Play the file at `path` from `from` seconds, in place of what plays.
pub fn play(path: &str, from: f64, speed: f64) {
    AUDIO.with(|audio| {
        let mut audio = audio.borrow_mut();
        if audio.mpv.is_null()
            && let Err(err) = audio.start()
        {
            eprintln!("audio: cannot start mpv: {err}");
            return;
        }
        audio.command(&["set", "start", &format!("{:.3}", from.max(0.0))]);
        audio.command(&["set", "speed", &speed.to_string()]);
        audio.command(&["loadfile", path, "replace"]);
        audio.command(&["set", "pause", "no"]);
        audio.loaded = true;
    });
}

pub fn set_paused(paused: bool) {
    AUDIO.with(|audio| audio.borrow().command(&["set", "pause", if paused { "yes" } else { "no" }]));
}

pub fn seek(seconds: f64) {
    AUDIO.with(|audio| audio.borrow().command(&["seek", &format!("{:.3}", seconds.max(0.0)), "absolute"]));
}

pub fn set_speed(speed: f64) {
    AUDIO.with(|audio| audio.borrow().command(&["set", "speed", &speed.to_string()]));
}

/// Nothing plays.
pub fn stop() {
    AUDIO.with(|audio| {
        let mut audio = audio.borrow_mut();
        if audio.loaded {
            audio.command(&["stop"]);
        }
        audio.loaded = false;
    });
}

impl Audio {
    fn start(&mut self) -> Result<(), String> {
        // SAFETY: plain libmpv calls on a handle we own; strings outlive the calls.
        unsafe {
            let mpv = mpv::mpv_create();
            if mpv.is_null() {
                return Err("mpv_create failed".into());
            }
            // Nothing of the user's own mpv is read, no video is decoded or shown, and a file
            // played to its end is let go.
            for (name, value) in [
                ("config", "no"),
                ("vo", "null"),
                ("vid", "no"),
                ("audio-display", "no"),
                ("idle", "yes"),
                ("keep-open", "no"),
                ("terminal", "no"),
                ("input-default-bindings", "no"),
                ("osd-level", "0"),
                ("sub-auto", "no"),
            ] {
                let (name, value) = (CString::new(name).unwrap_or_default(), CString::new(value).unwrap_or_default());
                let status = mpv::mpv_set_option_string(mpv, name.as_ptr(), value.as_ptr());
                if status < 0 {
                    eprintln!("audio: option {name:?}: {}", super::error_string(status));
                }
            }
            let status = mpv::mpv_initialize(mpv);
            if status < 0 {
                mpv::mpv_terminate_destroy(mpv);
                return Err(super::error_string(status));
            }
            for (id, name, format) in [(POSITION, c"time-pos", mpv::MPV_FORMAT_DOUBLE), (DURATION, c"duration", mpv::MPV_FORMAT_DOUBLE), (PAUSED, c"pause", mpv::MPV_FORMAT_FLAG)] {
                mpv::mpv_observe_property(mpv, id, name.as_ptr(), format);
            }
            mpv::mpv_set_wakeup_callback(mpv, wake_for_events, ptr::null_mut());
            self.mpv = mpv;
        }
        Ok(())
    }

    fn command(&self, arguments: &[&str]) {
        if self.mpv.is_null() {
            return;
        }
        let owned: Vec<CString> = arguments.iter().filter_map(|argument| CString::new(*argument).ok()).collect();
        let mut pointers: Vec<*const c_char> = owned.iter().map(|argument| argument.as_ptr()).collect();
        pointers.push(ptr::null());
        // SAFETY: a null-terminated array of strings that live until the call returns.
        let status = unsafe { mpv::mpv_command(self.mpv, pointers.as_mut_ptr()) };
        if status < 0 {
            eprintln!("audio: {arguments:?}: {}", super::error_string(status));
        }
    }

    /// What mpv has to say since it last woke us.
    fn drain_events(&mut self) -> Vec<Heard> {
        let mut heard = Vec::new();
        if self.mpv.is_null() {
            return heard;
        }
        loop {
            // SAFETY: mpv_wait_event returns a valid event until the next call.
            let event = unsafe { &*mpv::mpv_wait_event(self.mpv, 0.0) };
            match event.event_id {
                mpv::MPV_EVENT_NONE | mpv::MPV_EVENT_SHUTDOWN => break,
                mpv::MPV_EVENT_PROPERTY_CHANGE => {
                    // SAFETY: a property change carries an mpv_event_property in the format we asked for.
                    let property = unsafe { &*(event.data as *const mpv::mpv_event_property) };
                    if property.data.is_null() || !self.loaded {
                        continue;
                    }
                    // SAFETY: as above: a double for the times, an int for the flag.
                    unsafe {
                        match event.reply_userdata {
                            POSITION => heard.push(Heard::Position(*(property.data as *const f64))),
                            DURATION => heard.push(Heard::Duration(*(property.data as *const f64))),
                            PAUSED => heard.push(Heard::Paused(*(property.data as *const c_int) != 0)),
                            _ => {}
                        }
                    }
                }
                mpv::MPV_EVENT_END_FILE => {
                    // SAFETY: an end-of-file event carries an mpv_event_end_file.
                    let end = unsafe { &*(event.data as *const mpv::mpv_event_end_file) };
                    if !self.loaded {
                        continue;
                    }
                    match end.reason {
                        mpv::MPV_END_FILE_REASON_EOF => {
                            self.loaded = false;
                            heard.push(Heard::Ended);
                        }
                        mpv::MPV_END_FILE_REASON_ERROR => {
                            eprintln!("audio: cannot play the file: {}", super::error_string(end.error));
                            self.loaded = false;
                            heard.push(Heard::Failed);
                        }
                        // Stopped, or another file in its place: what plays now says so itself.
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        heard
    }
}

/// mpv has events: the UI thread reads them (once for however many wake-ups came meanwhile), and
/// the listener hears them once the player is let go.
unsafe extern "C" fn wake_for_events(_: *mut c_void) {
    if !EVENTS_ASKED.swap(true, Ordering::AcqRel) {
        let _ = slint::invoke_from_event_loop(|| {
            EVENTS_ASKED.store(false, Ordering::Release);
            let heard = AUDIO.with(|audio| audio.borrow_mut().drain_events());
            if let Some(listener) = LISTENER.with(Cell::get) {
                for what in heard {
                    listener(what);
                }
            }
        });
    }
}
