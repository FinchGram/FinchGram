//! A stream of bytes that arrive while mpv reads them, held in memory: a live stream
//! (src/player/live.rs). The live side pushes what it has (remux.rs makes it), mpv reads it
//! through libmpv's stream callbacks under the "finchgram-live" protocol, waiting for bytes
//! that have not come yet, and a read returns 0 once the stream has ended and been read.
//!
//! Unlike stream.rs, there is nothing on the disk and nothing to seek: a live stream goes one way.

use std::collections::VecDeque;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::mpv;

/// What a read answers once the stream was given up (stream_cb.h: "-1 on error").
const READ_FAILED: i64 = -1;
/// How long a read waits before it looks again, so that a wait never outlives a stream given up
/// without a word.
const LOOK_AGAIN: Duration = Duration::from_millis(250);
/// What may wait unread before the stream is too far behind to be worth playing: about a minute
/// of a 6 Mbit/s stream. Beyond it, pushing fails, and the live side starts again at the edge.
const MOST_WAITING: usize = 48 * 1024 * 1024;

/// The protocol mpv knows these streams by.
pub const PROTOCOL: &CStr = c"finchgram-live";

/// The bytes of one stream, as they arrive.
pub struct Pipe {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    /// What has arrived and not been read, in the order it came.
    waiting: VecDeque<Vec<u8>>,
    /// How much of the first of them has been read.
    read_of_first: usize,
    /// How many bytes wait, all told.
    waiting_bytes: usize,
    /// Nothing more will come: a read gets what is left, then 0.
    ended: bool,
    /// mpv gave the stream up, or the live side did: reads fail at once.
    over: bool,
}

impl Pipe {
    pub fn new() -> Arc<Pipe> {
        Arc::new(Pipe { state: Mutex::new(State::default()), changed: Condvar::new() })
    }

    /// More of the stream. False when it is over, or when so much waits unread that mpv is far
    /// behind: the bytes are not kept then.
    pub fn push(&self, bytes: Vec<u8>) -> bool {
        if bytes.is_empty() {
            return true;
        }
        let mut state = self.state.lock().unwrap();
        if state.over || state.ended || state.waiting_bytes + bytes.len() > MOST_WAITING {
            return false;
        }
        state.waiting_bytes += bytes.len();
        state.waiting.push_back(bytes);
        drop(state);
        self.changed.notify_all();
        true
    }

    /// Nothing more will come: what waits is still read, and then the stream ends.
    pub fn end(&self) {
        self.state.lock().unwrap().ended = true;
        self.changed.notify_all();
    }

    /// The stream is given up: a read under way returns, and every read after fails.
    pub fn cancel(&self) {
        self.state.lock().unwrap().over = true;
        self.changed.notify_all();
    }

    /// How many bytes wait unread.
    #[cfg(test)]
    fn waiting(&self) -> usize {
        self.state.lock().unwrap().waiting_bytes
    }

    /// Up to `buffer.len()` bytes, waiting for the first: how many, 0 at the end, or an error
    /// once the stream is given up.
    fn read(&self, buffer: &mut [u8]) -> i64 {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.over {
                return READ_FAILED;
            }
            if let Some(first) = state.waiting.front() {
                let from = state.read_of_first;
                let count = (first.len() - from).min(buffer.len());
                buffer[..count].copy_from_slice(&first[from..from + count]);
                let first_len = first.len();
                state.read_of_first += count;
                state.waiting_bytes -= count;
                if state.read_of_first == first_len {
                    state.waiting.pop_front();
                    state.read_of_first = 0;
                }
                return count as i64;
            }
            if state.ended {
                return 0;
            }
            state = self.changed.wait_timeout(state, LOOK_AGAIN).unwrap().0;
        }
    }
}

// ---- mpv's side ----------------------------------------------------------------------------------

/// The pipes mpv may open, by their URLs.
static OFFERED: Mutex<Vec<(String, Arc<Pipe>)>> = Mutex::new(Vec::new());
/// Tests that offer pipes take turns: offering one gives up the one before.
#[cfg(test)]
pub(super) static OFFERING: Mutex<()> = Mutex::new(());
/// Each pipe gets a URL of its own, so that a load under way cannot open a later one.
static NEXT: AtomicU64 = AtomicU64::new(1);

/// The URL mpv plays `pipe` by, in place of any pipe before (which is given up).
pub fn offer(pipe: Arc<Pipe>) -> String {
    withdraw();
    let url = format!("{}://{}", PROTOCOL.to_string_lossy(), NEXT.fetch_add(1, Ordering::Relaxed));
    OFFERED.lock().unwrap().push((url.clone(), pipe));
    url
}

/// No pipe is to be opened, and the one offered is given up.
pub fn withdraw() {
    for (_, pipe) in OFFERED.lock().unwrap().drain(..) {
        pipe.cancel();
    }
}

/// mpv opens a "finchgram-live://" URL: the pipe offered under it. The cookie is a reference of
/// the pipe's own, given back in `close`.
pub(super) unsafe extern "C" fn open(_: *mut c_void, uri: *mut c_char, info: *mut mpv::mpv_stream_cb_info) -> c_int {
    // SAFETY: mpv hands a C string that lives for the call.
    let uri = unsafe { CStr::from_ptr(uri) }.to_string_lossy().into_owned();
    let pipe = OFFERED.lock().unwrap().iter().find(|(url, _)| *url == uri).map(|(_, pipe)| pipe.clone());
    let Some(pipe) = pipe else { return mpv::MPV_ERROR_LOADING_FAILED };
    // SAFETY: mpv hands the structure to fill in, which lives for the call. No seek and no
    // size: a live stream has neither.
    unsafe {
        (*info).cookie = Arc::into_raw(pipe).cast_mut().cast();
        (*info).read_fn = Some(read);
        (*info).seek_fn = None;
        (*info).size_fn = None;
        (*info).close_fn = Some(close);
        (*info).cancel_fn = Some(cancel);
    }
    0
}

/// The pipe a cookie stands for.
///
/// # Safety
///
/// `cookie` is what `open` put there, and `close` has not been called for it.
unsafe fn pipe_of<'a>(cookie: *mut c_void) -> &'a Pipe {
    // SAFETY: the caller's.
    unsafe { &*cookie.cast_const().cast::<Pipe>() }
}

unsafe extern "C" fn read(cookie: *mut c_void, buf: *mut c_char, nbytes: u64) -> i64 {
    let length = usize::try_from(nbytes).unwrap_or(0).min(isize::MAX as usize);
    // SAFETY: mpv's buffer holds `nbytes`; the cookie is `open`'s.
    let buffer = unsafe { std::slice::from_raw_parts_mut(buf.cast::<u8>(), length) };
    unsafe { pipe_of(cookie) }.read(buffer)
}

unsafe extern "C" fn cancel(cookie: *mut c_void) {
    // SAFETY: the cookie is `open`'s.
    unsafe { pipe_of(cookie) }.cancel();
}

unsafe extern "C" fn close(cookie: *mut c_void) {
    // SAFETY: the reference `open` made, given back once.
    let pipe = unsafe { Arc::from_raw(cookie.cast_const().cast::<Pipe>()) };
    pipe.cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(pipe: &Arc<Pipe>, size: usize) -> std::thread::JoinHandle<(i64, Vec<u8>)> {
        let pipe = pipe.clone();
        std::thread::spawn(move || {
            let mut buffer = vec![0u8; size];
            let read = pipe.read(&mut buffer);
            buffer.truncate(read.max(0) as usize);
            (read, buffer)
        })
    }

    #[test]
    fn a_read_waits_for_bytes_and_takes_them_in_order() {
        let pipe = Pipe::new();
        let waiting = reading(&pipe, 4);
        std::thread::sleep(Duration::from_millis(50));
        assert!(pipe.push(vec![1, 2, 3]));
        assert!(pipe.push(vec![4, 5]));
        assert_eq!(waiting.join().unwrap(), (3, vec![1, 2, 3]), "what had arrived when it woke");
        let mut buffer = [0u8; 1];
        assert_eq!(pipe.read(&mut buffer), 1);
        assert_eq!(buffer[0], 4);
        assert_eq!(pipe.waiting(), 1);
        assert_eq!(pipe.read(&mut buffer), 1);
        assert_eq!(buffer[0], 5);
        assert_eq!(pipe.waiting(), 0);
    }

    #[test]
    fn an_ended_stream_is_read_to_its_end_and_a_cancelled_one_fails() {
        let pipe = Pipe::new();
        pipe.push(vec![9]);
        pipe.end();
        assert!(!pipe.push(vec![10]), "nothing after the end");
        let mut buffer = [0u8; 4];
        assert_eq!(pipe.read(&mut buffer), 1);
        assert_eq!(pipe.read(&mut buffer), 0, "then the end");

        let pipe = Pipe::new();
        let waiting = reading(&pipe, 4);
        std::thread::sleep(Duration::from_millis(50));
        pipe.cancel();
        assert_eq!(waiting.join().unwrap().0, READ_FAILED);
        assert!(!pipe.push(vec![1]));
    }

    #[test]
    fn a_stream_too_far_behind_takes_no_more() {
        let pipe = Pipe::new();
        assert!(pipe.push(vec![0; MOST_WAITING]));
        assert!(!pipe.push(vec![0]));
        let mut buffer = vec![0u8; 1024];
        assert_eq!(pipe.read(&mut buffer), 1024);
        assert!(pipe.push(vec![0]), "room again once read");
    }

    #[test]
    fn only_the_pipe_offered_last_opens() {
        let _turn = OFFERING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let first = Pipe::new();
        let first_url = offer(first.clone());
        let second = Pipe::new();
        let second_url = offer(second.clone());
        assert_ne!(first_url, second_url);
        let mut buffer = [0u8; 1];
        assert_eq!(first.read(&mut buffer), READ_FAILED, "the first was given up");
        let mut info = mpv::mpv_stream_cb_info { cookie: std::ptr::null_mut(), read_fn: None, seek_fn: None, size_fn: None, close_fn: None, cancel_fn: None };
        let url = std::ffi::CString::new(first_url).unwrap();
        assert_eq!(unsafe { open(std::ptr::null_mut(), url.as_ptr().cast_mut(), &mut info) }, mpv::MPV_ERROR_LOADING_FAILED);
        let url = std::ffi::CString::new(second_url).unwrap();
        assert_eq!(unsafe { open(std::ptr::null_mut(), url.as_ptr().cast_mut(), &mut info) }, 0);
        assert!(info.read_fn.is_some() && info.seek_fn.is_none() && info.size_fn.is_none());
        unsafe { close(info.cookie) };
        withdraw();
    }

    /// What a headless mpv made of a stream: the tracks it found, how far it played, the width
    /// of the pictures it showed and how many it dropped, and how it ended.
    struct Played {
        tracks: i64,
        furthest: f64,
        picture_width: i64,
        dropped: i64,
        ended_well: bool,
    }

    /// Play `url` in a libmpv of its own with no window and no sound device, to its end.
    fn play_headless(url: &str) -> Played {
        use std::ffi::CString;
        let mut played = Played { tracks: 0, furthest: 0.0, picture_width: 0, dropped: 0, ended_well: false };
        // SAFETY: a libmpv handle of the test's own, destroyed before returning.
        unsafe {
            let handle = mpv::mpv_create();
            assert!(!handle.is_null());
            let terminal = if std::env::var("FINCHGRAM_MPV_LOG").is_ok() { "yes" } else { "no" };
            for (name, value) in [("vo", "null"), ("ao", "null"), ("terminal", terminal), ("msg-level", "all=v"), ("config", "no"), ("idle", "yes"), ("hwdec", "no")] {
                let (name, value) = (CString::new(name).unwrap(), CString::new(value).unwrap());
                mpv::mpv_set_option_string(handle, name.as_ptr(), value.as_ptr());
            }
            assert!(mpv::mpv_initialize(handle) >= 0);
            assert!(mpv::mpv_stream_cb_add_ro(handle, PROTOCOL.as_ptr(), std::ptr::null_mut(), open) >= 0);
            mpv::mpv_observe_property(handle, 1, c"time-pos".as_ptr(), mpv::MPV_FORMAT_DOUBLE);
            let (command, file) = (CString::new("loadfile").unwrap(), CString::new(url).unwrap());
            let mut arguments = [command.as_ptr(), file.as_ptr(), std::ptr::null()];
            assert!(mpv::mpv_command(handle, arguments.as_mut_ptr()) >= 0);
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(20) {
                let event = &*mpv::mpv_wait_event(handle, 0.25);
                match event.event_id {
                    mpv::MPV_EVENT_FILE_LOADED => {
                        let mut count: i64 = 0;
                        mpv::mpv_get_property(handle, c"track-list/count".as_ptr(), mpv::MPV_FORMAT_INT64, (&mut count as *mut i64).cast());
                        played.tracks = count;
                    }
                    mpv::MPV_EVENT_PROPERTY_CHANGE => {
                        let property = &*(event.data as *const mpv::mpv_event_property);
                        if !property.data.is_null() {
                            played.furthest = played.furthest.max(*(property.data as *const f64));
                        }
                        let mut width: i64 = 0;
                        if mpv::mpv_get_property(handle, c"video-out-params/w".as_ptr(), mpv::MPV_FORMAT_INT64, (&mut width as *mut i64).cast()) >= 0 {
                            played.picture_width = played.picture_width.max(width);
                        }
                        let mut dropped: i64 = 0;
                        if mpv::mpv_get_property(handle, c"frame-drop-count".as_ptr(), mpv::MPV_FORMAT_INT64, (&mut dropped as *mut i64).cast()) >= 0 {
                            played.dropped = played.dropped.max(dropped);
                        }
                    }
                    mpv::MPV_EVENT_END_FILE => {
                        let end = &*(event.data as *const mpv::mpv_event_end_file);
                        played.ended_well = end.reason == mpv::MPV_END_FILE_REASON_EOF;
                        break;
                    }
                    mpv::MPV_EVENT_SHUTDOWN => break,
                    _ => {}
                }
            }
            mpv::mpv_terminate_destroy(handle);
        }
        played
    }

    #[test]
    fn mpv_plays_a_live_stream_as_it_arrives() {
        let _turn = OFFERING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let pipe = Pipe::new();
        let url = offer(pipe.clone());
        // The seconds come as a live stream's do, one a second, after the first.
        let feeding = std::thread::spawn(move || {
            let mut remuxer = super::super::remux::Remuxer::new();
            for (index, second) in super::super::remux::tests::SECONDS.iter().enumerate() {
                if index > 0 {
                    std::thread::sleep(Duration::from_millis(900));
                }
                assert!(pipe.push(remuxer.push(second, index as i64 * 1000).expect("a second")));
            }
            pipe.end();
        });
        let played = play_headless(&url);
        feeding.join().unwrap();
        withdraw();
        assert_eq!(played.tracks, 2, "a picture and a sound");
        assert!(played.ended_well, "played to the end of the stream, not cut short");
        assert!(played.furthest > 2.5, "played the three seconds, not {:.2}", played.furthest);
        assert_eq!(played.picture_width, 160, "showed the pictures");
        // Dropped pictures are counted on a Mac only: GitHub's runners, virtual Macs, dropped 17
        // to 24 of them on every check since the test came in, while a Mac drops none even with
        // every core busy (2026-11-11). How smoothly a stream plays is a Mac's to tell.
        if std::env::var_os("GITHUB_ACTIONS").is_none() {
            assert_eq!(played.dropped, 0, "none dropped");
        }
    }
}
