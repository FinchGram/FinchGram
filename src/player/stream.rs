//! A file played as it arrives: mpv reads it through callbacks of ours (libmpv's stream_cb, under
//! the "finchgram://" protocol) from the file TDLib is writing, waits when it asks for bytes that
//! have not arrived, and has the download moved when it seeks past them. The viewer plays a video
//! this way when it is not on the disk yet (src/telegram/viewer.rs); the Telegram side feeds the
//! stream what updateFile says of the file (src/telegram/files.rs), on the UI thread, while mpv
//! reads on a thread of its own.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::os::unix::fs::FileExt as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::mpv;

/// What a read answers when the stream is over (stream_cb.h: "-1 on error").
const READ_FAILED: i64 = -1;
/// How long a read waits for bytes before it looks again. A wait ends when they are said to have
/// arrived; this is for the file on the disk growing without a word.
const LOOK_AGAIN: Duration = Duration::from_millis(500);

/// Who offered a stream: the media viewer's video, of which there is one at a time, or the music
/// player (src/player/music.rs), which keeps the song playing and the next one; each withdraws
/// its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Video,
    Music,
}

/// The streams mpv may open, by their URLs.
static OFFERED: Mutex<Vec<(String, Arc<Stream>, Owner)>> = Mutex::new(Vec::new());
/// Each stream gets a URL of its own, so that a load under way cannot open a later one.
static NEXT: AtomicU64 = AtomicU64::new(1);

/// A file as it arrives, read by mpv.
pub struct Stream {
    state: Mutex<State>,
    changed: Condvar,
    /// Asked, on mpv's thread, for the bytes from an offset the download is not bringing: the
    /// Telegram side moves the download there.
    ask: Box<dyn Fn(u64) + Send + Sync>,
    /// Told, on the UI thread, what has arrived, as parts of the file (from, to): the viewer's
    /// seek bar shows it. (0, 0) once the file is whole.
    arrived_shown: Box<dyn Fn(f32, f32) + Send + Sync>,
}

struct State {
    /// The file's size in bytes; 0 while unknown.
    size: u64,
    /// Where the file's bytes are: TDLib's partial file, then the whole one.
    path: Option<PathBuf>,
    /// The file open, and the path it was opened at.
    file: Option<(PathBuf, std::fs::File)>,
    /// What has arrived: `prefix` bytes from `offset`, and everything once `complete`.
    offset: u64,
    prefix: u64,
    complete: bool,
    /// Where mpv reads next.
    position: u64,
    /// The offset last asked for, until the download is there.
    asked: Option<u64>,
    /// mpv gave the stream up, or nothing more will come: a read returns at once.
    over: bool,
}

impl Stream {
    pub fn new(size: u64, ask: impl Fn(u64) + Send + Sync + 'static, arrived_shown: impl Fn(f32, f32) + Send + Sync + 'static) -> Arc<Stream> {
        Arc::new(Stream {
            state: Mutex::new(State { size, path: None, file: None, offset: 0, prefix: 0, complete: false, position: 0, asked: None, over: false }),
            changed: Condvar::new(),
            ask: Box::new(ask),
            arrived_shown: Box::new(arrived_shown),
        })
    }

    /// What TDLib says of the file (updateFile, or a downloadFile's answer): where it is, which
    /// of its bytes are on the disk, whether all of them are, and how large it is. UI thread.
    pub fn arrived(&self, path: Option<PathBuf>, offset: u64, prefix: u64, complete: bool, size: u64) {
        let mut state = self.state.lock().unwrap();
        if path.is_some() {
            state.path = path;
        }
        state.offset = offset;
        state.prefix = prefix;
        state.complete = complete;
        if size > 0 {
            state.size = size;
        }
        if state.asked == Some(offset) {
            state.asked = None;
        }
        let shown = if complete || state.size == 0 {
            (0.0, 0.0)
        } else {
            let part = |bytes: u64| (bytes as f64 / state.size as f64).min(1.0) as f32;
            (part(offset), part(offset.saturating_add(prefix)))
        };
        drop(state);
        (self.arrived_shown)(shown.0, shown.1);
        self.changed.notify_all();
    }

    /// Nothing more will come: the download failed, or the account's finchgram-tdlib ended. A
    /// read under way returns, and none waits again.
    pub fn failed(&self) {
        self.state.lock().unwrap().over = true;
        self.changed.notify_all();
    }

    /// mpv gives the stream up: as `failed`.
    fn cancel(&self) {
        self.failed();
    }

    /// Up to `buffer.len()` bytes from where mpv reads, as many of them as have arrived, waiting
    /// for the first: how many, 0 at the end of the file, or an error once the stream is over.
    fn read(&self, buffer: &mut [u8]) -> i64 {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.over {
                return READ_FAILED;
            }
            let position = state.position;
            if state.size > 0 && position >= state.size {
                return 0;
            }
            let ready = if state.complete {
                if state.size > 0 { state.size - position } else { u64::MAX }
            } else if position >= state.offset {
                (state.offset + state.prefix).saturating_sub(position)
            } else {
                0
            };
            if ready > 0 && state.path.is_some() {
                let want = usize::try_from(ready.min(buffer.len() as u64)).unwrap_or(buffer.len());
                match state.read_at(position, &mut buffer[..want]) {
                    // Said to be there and not: the end, when the file is whole; else not written yet.
                    Ok(0) if state.complete => return 0,
                    Ok(0) => {}
                    Ok(read) => {
                        state.position += read as u64;
                        return read as i64;
                    }
                    Err(err) => {
                        eprintln!("player: cannot read the file as it arrives: {err}");
                        return READ_FAILED;
                    }
                }
            } else if !state.complete && (position < state.offset || position > state.offset + state.prefix) && state.asked != Some(position) {
                // The download is not bringing these bytes: it is moved here.
                state.asked = Some(position);
                (self.ask)(position);
            }
            state = self.changed.wait_timeout(state, LOOK_AGAIN).unwrap().0;
        }
    }

    /// mpv reads from `offset` next.
    fn seek(&self, offset: i64) -> i64 {
        let Ok(position) = u64::try_from(offset) else { return i64::from(mpv::MPV_ERROR_GENERIC) };
        self.state.lock().unwrap().position = position;
        offset
    }

    fn size(&self) -> i64 {
        match self.state.lock().unwrap().size {
            0 => i64::from(mpv::MPV_ERROR_UNSUPPORTED),
            size => i64::try_from(size).unwrap_or(i64::MAX),
        }
    }
}

impl State {
    /// `buffer` from `position` of the file, opened again when it moved (TDLib moves a whole
    /// file out of its temporary folder).
    fn read_at(&mut self, position: u64, buffer: &mut [u8]) -> std::io::Result<usize> {
        let Some(path) = self.path.clone() else { return Ok(0) };
        if self.file.as_ref().is_none_or(|(opened, _)| *opened != path) {
            self.file = Some((path.clone(), std::fs::File::open(&path)?));
        }
        match &self.file {
            Some((_, file)) => file.read_at(buffer, position),
            None => Ok(0),
        }
    }
}

// ---- mpv's side ----------------------------------------------------------------------------------

/// The URL the video player plays `stream` by: the one it may open from now on, in place of any
/// video stream before, which is over.
pub(super) fn offer(stream: Arc<Stream>) -> String {
    withdraw();
    offer_for(Owner::Video, stream)
}

/// No video stream is to be opened, and the one offered is over.
pub(super) fn withdraw() {
    OFFERED.lock().unwrap().retain(|(_, stream, owner)| {
        if *owner == Owner::Video {
            stream.cancel();
        }
        *owner != Owner::Video
    });
}

/// The URL the music player plays `stream` by, beside the music streams offered before (the song
/// playing keeps its stream while the next is offered).
pub fn offer_music(stream: Arc<Stream>) -> String {
    offer_for(Owner::Music, stream)
}

/// The music stream at `url` is over: mpv may not open it, and a read under way returns.
pub fn withdraw_music(url: &str) {
    OFFERED.lock().unwrap().retain(|(offered, stream, owner)| {
        if *owner == Owner::Music && offered == url {
            stream.cancel();
            return false;
        }
        true
    });
}

/// Every music stream is over.
pub fn withdraw_all_music() {
    OFFERED.lock().unwrap().retain(|(_, stream, owner)| {
        if *owner == Owner::Music {
            stream.cancel();
        }
        *owner != Owner::Music
    });
}

fn offer_for(owner: Owner, stream: Arc<Stream>) -> String {
    let url = format!("finchgram://{}", NEXT.fetch_add(1, Ordering::Relaxed));
    OFFERED.lock().unwrap().push((url.clone(), stream, owner));
    url
}

/// mpv opens a "finchgram://" URL: the stream offered under it, through its callbacks. The
/// cookie is a reference of the stream's own, given back in `close`.
pub(super) unsafe extern "C" fn open(_: *mut c_void, uri: *mut c_char, info: *mut mpv::mpv_stream_cb_info) -> c_int {
    // SAFETY: mpv hands a C string that lives for the call.
    let uri = unsafe { CStr::from_ptr(uri) }.to_string_lossy().into_owned();
    let stream = OFFERED.lock().unwrap().iter().find(|(url, _, _)| *url == uri).map(|(_, stream, _)| stream.clone());
    let Some(stream) = stream else { return mpv::MPV_ERROR_LOADING_FAILED };
    // SAFETY: mpv hands the structure to fill in, which lives for the call.
    unsafe {
        (*info).cookie = Arc::into_raw(stream).cast_mut().cast();
        (*info).read_fn = Some(read);
        (*info).seek_fn = Some(seek);
        (*info).size_fn = Some(size);
        (*info).close_fn = Some(close);
        (*info).cancel_fn = Some(cancel);
    }
    0
}

/// The stream a cookie stands for.
///
/// # Safety
///
/// `cookie` is what `open` put there, and `close` has not been called for it.
unsafe fn stream_of<'a>(cookie: *mut c_void) -> &'a Stream {
    // SAFETY: the caller's.
    unsafe { &*cookie.cast_const().cast::<Stream>() }
}

unsafe extern "C" fn read(cookie: *mut c_void, buf: *mut c_char, nbytes: u64) -> i64 {
    let length = usize::try_from(nbytes).unwrap_or(0).min(isize::MAX as usize);
    // SAFETY: mpv's buffer holds `nbytes`; the cookie is `open`'s.
    let buffer = unsafe { std::slice::from_raw_parts_mut(buf.cast::<u8>(), length) };
    unsafe { stream_of(cookie) }.read(buffer)
}

unsafe extern "C" fn seek(cookie: *mut c_void, offset: i64) -> i64 {
    // SAFETY: the cookie is `open`'s.
    unsafe { stream_of(cookie) }.seek(offset)
}

unsafe extern "C" fn size(cookie: *mut c_void) -> i64 {
    // SAFETY: the cookie is `open`'s.
    unsafe { stream_of(cookie) }.size()
}

unsafe extern "C" fn cancel(cookie: *mut c_void) {
    // SAFETY: the cookie is `open`'s.
    unsafe { stream_of(cookie) }.cancel();
}

unsafe extern "C" fn close(cookie: *mut c_void) {
    // SAFETY: the reference `open` made, given back once.
    let stream = unsafe { Arc::from_raw(cookie.cast_const().cast::<Stream>()) };
    stream.cancel();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("finchgram-stream-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        folder
    }

    /// The bytes `from` … `to` of the file, each its own offset.
    fn written(path: &PathBuf, from: u64, to: u64) {
        let file = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(path).expect("the file");
        let bytes: Vec<u8> = (from..to).map(|byte| byte as u8).collect();
        file.write_at(&bytes, from).expect("written");
        file.sync_all().expect("synced");
    }

    fn reading(stream: &Arc<Stream>) -> std::thread::JoinHandle<(i64, [u8; 16])> {
        let stream = stream.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 16];
            let read = stream.read(&mut buffer);
            (read, buffer)
        })
    }

    #[test]
    fn a_stream_serves_what_has_arrived_and_asks_for_what_has_not() {
        let folder = scratch("arriving");
        let path = folder.join("video.part");
        let asked = Arc::new(Mutex::new(Vec::new()));
        let shown = Arc::new(Mutex::new(Vec::new()));
        let stream = Stream::new(
            100,
            { let asked = asked.clone(); move |offset| asked.lock().unwrap().push(offset) },
            { let shown = shown.clone(); move |from, to| shown.lock().unwrap().push((from, to)) },
        );
        assert_eq!(stream.size(), 100);
        // Nothing has arrived: a read waits, and asks for nothing (the download is on its way
        // from the start).
        let reader = reading(&stream);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!reader.is_finished());
        assert!(asked.lock().unwrap().is_empty());
        // The first twenty bytes come: the read gets its sixteen, the next the other four, and
        // the one after waits at the edge.
        written(&path, 0, 20);
        stream.arrived(Some(path.clone()), 0, 20, false, 100);
        let (read, buffer) = reader.join().expect("read");
        assert_eq!((read, &buffer[..16]), (16, &(0..16).collect::<Vec<u8>>()[..]));
        let mut buffer = [0u8; 16];
        assert_eq!((stream.read(&mut buffer), &buffer[..4]), (4, &[16, 17, 18, 19][..]));
        assert_eq!(shown.lock().unwrap().last(), Some(&(0.0, 0.2)));
        let reader = reading(&stream);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!reader.is_finished() && asked.lock().unwrap().is_empty());
        // Twenty more come: the waiting read goes on.
        written(&path, 20, 40);
        stream.arrived(Some(path.clone()), 0, 40, false, 100);
        assert_eq!(reader.join().expect("read").0, 16);
        // A seek past what has arrived asks for the bytes there, once; they come, and are read.
        assert_eq!(stream.seek(60), 60);
        let reader = reading(&stream);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!((asked.lock().unwrap().as_slice(), reader.is_finished()), (&[60][..], false));
        written(&path, 60, 70);
        stream.arrived(Some(path.clone()), 60, 10, false, 100);
        let (read, buffer) = reader.join().expect("read");
        assert_eq!((read, &buffer[..10]), (10, &(60..70).map(|byte| byte as u8).collect::<Vec<u8>>()[..]));
        assert_eq!(shown.lock().unwrap().last(), Some(&(0.6, 0.7)));
        // Before the download's offset, too.
        assert_eq!(stream.seek(50), 50);
        let reader = reading(&stream);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(asked.lock().unwrap().as_slice(), &[60, 50]);
        // The whole file: read to its end, and nothing shown as arriving.
        written(&path, 0, 100);
        stream.arrived(Some(path.clone()), 0, 100, true, 100);
        assert_eq!(reader.join().expect("read").0, 16);
        assert_eq!(stream.seek(96), 96);
        let mut buffer = [0u8; 16];
        assert_eq!((stream.read(&mut buffer), &buffer[..4]), (4, &[96, 97, 98, 99][..]));
        assert_eq!(stream.read(&mut buffer), 0);
        assert_eq!(shown.lock().unwrap().last(), Some(&(0.0, 0.0)));
        // Given up: a read returns at once, with an error.
        stream.cancel();
        assert_eq!(stream.read(&mut buffer), READ_FAILED);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The streams offered are one list for the process: the tests that offer and withdraw take
    /// turns.
    static TURN: Mutex<()> = Mutex::new(());

    #[test]
    fn mpv_opens_the_stream_offered_under_its_url_and_no_other() {
        let _turn = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let stream = Stream::new(0, |_| {}, |_, _| {});
        let url = offer(stream.clone());
        assert!(url.starts_with("finchgram://"));
        let mut info = mpv::mpv_stream_cb_info { cookie: std::ptr::null_mut(), read_fn: None, seek_fn: None, size_fn: None, close_fn: None, cancel_fn: None };
        let other = CString::new("finchgram://0").expect("a string");
        // SAFETY: a C string and a structure that live for the call.
        assert_eq!(unsafe { open(std::ptr::null_mut(), other.as_ptr().cast_mut(), &mut info) }, mpv::MPV_ERROR_LOADING_FAILED);
        let right = CString::new(url).expect("a string");
        // SAFETY: as above.
        assert_eq!(unsafe { open(std::ptr::null_mut(), right.as_ptr().cast_mut(), &mut info) }, 0);
        assert!(!info.cookie.is_null() && info.read_fn.is_some() && info.cancel_fn.is_some());
        // The size is unknown here; and once closed, the stream is over.
        // SAFETY: the cookie `open` made.
        assert_eq!(unsafe { size(info.cookie) }, i64::from(mpv::MPV_ERROR_UNSUPPORTED));
        // SAFETY: given back once.
        unsafe { close(info.cookie) };
        assert_eq!(stream.read(&mut [0u8; 4]), READ_FAILED);
        withdraw();
        // SAFETY: as above.
        assert_eq!(unsafe { open(std::ptr::null_mut(), right.as_ptr().cast_mut(), &mut info) }, mpv::MPV_ERROR_LOADING_FAILED);
    }

    #[test]
    fn the_music_player_offers_its_streams_beside_each_other_and_the_videos() {
        let _turn = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (song, next, video) = (Stream::new(0, |_| {}, |_, _| {}), Stream::new(0, |_| {}, |_, _| {}), Stream::new(0, |_| {}, |_, _| {}));
        let song_url = offer_music(song.clone());
        let next_url = offer_music(next.clone());
        let video_url = offer(video.clone());
        let mut info = mpv::mpv_stream_cb_info { cookie: std::ptr::null_mut(), read_fn: None, seek_fn: None, size_fn: None, close_fn: None, cancel_fn: None };
        for url in [&song_url, &next_url, &video_url] {
            let url = CString::new(url.as_str()).expect("a string");
            // SAFETY: a C string and a structure that live for the call.
            assert_eq!(unsafe { open(std::ptr::null_mut(), url.as_ptr().cast_mut(), &mut info) }, 0);
            // SAFETY: the cookie `open` made, given back once.
            unsafe { close(info.cookie) };
        }
        // Closed by mpv, each is over; still, the song's stream alone is withdrawn, the next stays offered.
        withdraw_music(&song_url);
        let gone = CString::new(song_url.as_str()).expect("a string");
        let kept = CString::new(next_url.as_str()).expect("a string");
        // SAFETY: as above.
        assert_eq!(unsafe { open(std::ptr::null_mut(), gone.as_ptr().cast_mut(), &mut info) }, mpv::MPV_ERROR_LOADING_FAILED);
        // SAFETY: as above.
        assert_eq!(unsafe { open(std::ptr::null_mut(), kept.as_ptr().cast_mut(), &mut info) }, 0);
        // SAFETY: as above.
        unsafe { close(info.cookie) };
        // The video's withdrawal leaves the music alone, and the music's leaves the video alone.
        withdraw();
        // SAFETY: as above.
        assert_eq!(unsafe { open(std::ptr::null_mut(), kept.as_ptr().cast_mut(), &mut info) }, 0);
        // SAFETY: as above.
        unsafe { close(info.cookie) };
        withdraw_all_music();
        // SAFETY: as above.
        assert_eq!(unsafe { open(std::ptr::null_mut(), kept.as_ptr().cast_mut(), &mut info) }, mpv::MPV_ERROR_LOADING_FAILED);
    }
}
