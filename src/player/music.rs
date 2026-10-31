//! Music through libmpv (src/music/): an mpv of its own, with no video, so that the music, a
//! voice message and the media viewer never take each other's player. It plays one song after
//! another without a gap: the next song is queued behind the current one in mpv's playlist, and
//! mpv opens it before the current one ends (prefetch-playlist, gapless-audio). Songs come as
//! "finchgram://" streams the music module fetches (src/music/fetch.rs, src/player/stream.rs), or
//! as the cache file when it is whole. What it reports goes to the one listener, on the UI
//! thread: how far it has played, the length, which file plays, that nothing does.
//!
//! Everything runs on the UI thread; mpv's own threads only ask the event loop to look.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{mpv, stream};

/// What mpv reports, by the number given when observing it.
const POSITION: u64 = 1;
const DURATION: u64 = 2;
const PAUSED: u64 = 3;
const BUFFERING: u64 = 4;
const PATH: u64 = 5;
const IDLE: u64 = 6;

/// What the player tells its listener.
#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    /// How far it has played, in seconds.
    Position(f64),
    /// The file's length, in seconds, once mpv knows it.
    Duration(f64),
    Paused(bool),
    /// mpv waits for bytes of a file that is still arriving.
    Buffering(bool),
    /// A file plays: this one, by the URL or path it was given.
    Started(String),
    /// Nothing plays any more: the last file ended, or a stop.
    Idle,
    /// The file could not be played: mpv's words. mpv goes on to the next one queued, if any.
    Failed(String),
}

struct Music {
    /// Created when the first song plays.
    mpv: *mut mpv::mpv_handle,
    loaded: bool,
}

thread_local! {
    static MUSIC: RefCell<Music> = const { RefCell::new(Music { mpv: ptr::null_mut(), loaded: false }) };
    static LISTENER: Cell<Option<fn(Heard)>> = const { Cell::new(None) };
}

/// Set while the UI thread has been asked to look and has not yet: mpv wakes it once.
static EVENTS_ASKED: AtomicBool = AtomicBool::new(false);

/// Who hears what the player says: one, set once.
pub fn listen(listener: fn(Heard)) {
    LISTENER.with(|slot| slot.set(Some(listener)));
}

/// Play `url` (a "finchgram://" stream, or a file's path) from its start, in place of whatever
/// plays and whatever is queued, at `volume` (0 … 100).
pub fn play(url: &str, volume: i32) {
    MUSIC.with(|music| {
        let mut music = music.borrow_mut();
        if music.mpv.is_null()
            && let Err(err) = music.start()
        {
            eprintln!("music: cannot start mpv: {err}");
            return;
        }
        music.command(&["set", "volume", &volume.clamp(0, 100).to_string()]);
        music.command(&["loadfile", url, "replace"]);
        music.command(&["set", "pause", "no"]);
        music.loaded = true;
    });
}

/// `url` plays after the song playing, in place of anything queued before: mpv opens it ahead of
/// time, and goes on to it without a gap.
pub fn queue_next(url: &str) {
    MUSIC.with(|music| {
        let music = music.borrow();
        if !music.loaded {
            return;
        }
        music.command(&["playlist-clear"]);
        music.command(&["loadfile", url, "append"]);
    });
}

/// Nothing plays after the song playing.
pub fn clear_next() {
    MUSIC.with(|music| {
        let music = music.borrow();
        if music.loaded {
            music.command(&["playlist-clear"]);
        }
    });
}

pub fn set_paused(paused: bool) {
    MUSIC.with(|music| music.borrow().command(&["set", "pause", if paused { "yes" } else { "no" }]));
}

pub fn seek(seconds: f64) {
    MUSIC.with(|music| music.borrow().command(&["seek", &format!("{:.3}", seconds.max(0.0)), "absolute"]));
}

/// `volume` is 0 … 100.
pub fn set_volume(volume: i32) {
    MUSIC.with(|music| music.borrow().command(&["set", "volume", &volume.clamp(0, 100).to_string()]));
}

/// Nothing plays, and nothing is queued.
pub fn stop() {
    MUSIC.with(|music| {
        let mut music = music.borrow_mut();
        if music.loaded {
            music.command(&["stop"]);
        }
        music.loaded = false;
    });
}

impl Music {
    fn start(&mut self) -> Result<(), String> {
        // SAFETY: plain libmpv calls on a handle we own; strings outlive the calls.
        unsafe {
            let mpv = mpv::mpv_create();
            if mpv.is_null() {
                return Err("mpv_create failed".into());
            }
            // Nothing of the user's own mpv is read, no video is decoded or shown; songs follow
            // each other without a gap, the next opened while the current one plays; a file
            // played to its end is let go, and mpv waits for the next.
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
                ("cache", "yes"),
                ("gapless-audio", "yes"),
                ("prefetch-playlist", "yes"),
            ] {
                let (name, value) = (CString::new(name).unwrap_or_default(), CString::new(value).unwrap_or_default());
                let status = mpv::mpv_set_option_string(mpv, name.as_ptr(), value.as_ptr());
                if status < 0 {
                    eprintln!("music: option {name:?}: {}", super::error_string(status));
                }
            }
            let status = mpv::mpv_initialize(mpv);
            if status < 0 {
                mpv::mpv_terminate_destroy(mpv);
                return Err(super::error_string(status));
            }
            for (id, name, format) in [
                (POSITION, c"time-pos", mpv::MPV_FORMAT_DOUBLE),
                (DURATION, c"duration", mpv::MPV_FORMAT_DOUBLE),
                (PAUSED, c"pause", mpv::MPV_FORMAT_FLAG),
                (BUFFERING, c"paused-for-cache", mpv::MPV_FORMAT_FLAG),
                (PATH, c"path", mpv::MPV_FORMAT_STRING),
                (IDLE, c"idle-active", mpv::MPV_FORMAT_FLAG),
            ] {
                mpv::mpv_observe_property(mpv, id, name.as_ptr(), format);
            }
            // Songs played as they arrive: "finchgram://" URLs, read through stream.rs.
            let status = mpv::mpv_stream_cb_add_ro(mpv, c"finchgram".as_ptr(), ptr::null_mut(), stream::open);
            if status < 0 {
                eprintln!("music: cannot play songs as they arrive: {}", super::error_string(status));
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
            eprintln!("music: {arguments:?}: {}", super::error_string(status));
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
                    // SAFETY: as above: a double for the times, an int for the flags, a pointer
                    // to a C string for the path.
                    unsafe {
                        match event.reply_userdata {
                            POSITION => heard.push(Heard::Position(*(property.data as *const f64))),
                            DURATION => heard.push(Heard::Duration(*(property.data as *const f64))),
                            PAUSED => heard.push(Heard::Paused(*(property.data as *const c_int) != 0)),
                            BUFFERING => heard.push(Heard::Buffering(*(property.data as *const c_int) != 0)),
                            PATH => {
                                let text = *(property.data as *const *const c_char);
                                if !text.is_null() {
                                    heard.push(Heard::Started(CStr::from_ptr(text).to_string_lossy().into_owned()));
                                }
                            }
                            IDLE if *(property.data as *const c_int) != 0 => {
                                self.loaded = false;
                                heard.push(Heard::Idle);
                            }
                            _ => {}
                        }
                    }
                }
                mpv::MPV_EVENT_END_FILE => {
                    // SAFETY: an end-of-file event carries an mpv_event_end_file.
                    let end = unsafe { &*(event.data as *const mpv::mpv_event_end_file) };
                    if self.loaded && end.reason == mpv::MPV_END_FILE_REASON_ERROR {
                        heard.push(Heard::Failed(super::error_string(end.error)));
                    }
                    // Played to the end: the next file says so by starting, or idle-active does.
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
            let heard = MUSIC.with(|music| music.borrow_mut().drain_events());
            if let Some(listener) = LISTENER.with(Cell::get) {
                for what in heard {
                    listener(what);
                }
            }
        });
    }
}
