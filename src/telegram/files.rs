//! Files: TDLib downloads them into its own folder (downloadFile) and says how far it has got
//! (updateFile). Whatever waits for a file gets its local path once it is complete; a file asked
//! for twice is downloaded once, at the highest priority it was asked at. A video the viewer shows
//! before it is complete plays as it arrives, through a stream of the player's
//! (src/player/stream.rs) that is fed here what TDLib says of the file, and that has the download
//! moved when mpv seeks past what has arrived. When a file that was on its way is complete, the
//! log says how fast it came.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Instant;

use serde_json::json;

use super::api::File;
use super::{Error, Slot, send};
use crate::player::stream::Stream;

/// How soon a download starts among the others, 1 … 32: what is on screen goes first, then the
/// photos of the chats and people in the lists, wanted all at once.
pub const ON_SCREEN: i32 = 16;
pub const IN_LISTS: i32 = 8;

type Ready = Box<dyn FnOnce(String)>;
type Failed = Box<dyn FnOnce()>;

/// What waits for a file: what to do with its path, and, for some, what to do when it cannot be
/// had.
struct Waiter {
    ready: Ready,
    failed: Option<Failed>,
}

/// A file on its way: what waits for it, the priority it was asked at, when it was first asked
/// for and whether it was seen on its way since (for the log; one TDLib answers for from the disk
/// was not), and the stream that reads it as it comes, when one does.
struct Download {
    waiters: Vec<Waiter>,
    priority: i32,
    started: Instant,
    moving: bool,
    stream: Option<Weak<Stream>>,
}

thread_local! {
    /// By account and file id (TDLib's file ids are each account's own): the files on their way.
    static DOWNLOADS: RefCell<HashMap<(Slot, i32), Download>> = RefCell::new(HashMap::new());
}

/// Call `ready` with the local path of `file`, downloading it first unless it is on disk already.
pub fn download(file: &File, priority: i32, ready: impl FnOnce(String) + 'static) {
    wait(file, priority, Waiter { ready: Box::new(ready), failed: None });
}

/// As `download`, and `failed` is called when TDLib cannot get the file.
pub fn download_or_fail(file: &File, priority: i32, ready: impl FnOnce(String) + 'static, failed: impl FnOnce() + 'static) {
    wait(file, priority, Waiter { ready: Box::new(ready), failed: Some(Box::new(failed)) });
}

fn wait(file: &File, priority: i32, waiter: Waiter) {
    if let Some(path) = completed(file) {
        (waiter.ready)(path);
        return;
    }
    let slot = super::context();
    let id = file.id;
    let asking = DOWNLOADS.with(|downloads| {
        let mut downloads = downloads.borrow_mut();
        match downloads.get_mut(&(slot, id)) {
            Some(download) => {
                download.waiters.push(waiter);
                // Wanted sooner than it was asked for: asked again, and TDLib moves it up.
                let sooner = priority > download.priority;
                download.priority = download.priority.max(priority);
                sooner
            }
            None => {
                downloads.insert((slot, id), Download { waiters: vec![waiter], priority, started: Instant::now(), moving: false, stream: None });
                true
            }
        }
    });
    if asking {
        ask(slot, id, priority, 0);
    }
}

/// The file as it arrives, for the player to read while TDLib downloads it at `priority`
/// (src/player/stream.rs): fed what TDLib says of the file from here on (`updated`), and the
/// download moved to where it asks when mpv seeks past what has arrived. `arrived_shown` is told
/// what has arrived, as parts of the file, on the UI thread.
pub fn stream(file: &File, priority: i32, arrived_shown: impl Fn(f32, f32) + Send + Sync + 'static) -> Arc<Stream> {
    let slot = super::context();
    let id = file.id;
    let stream = Stream::new(
        u64::try_from(file.size_or_expected()).unwrap_or(0),
        move |offset| {
            // On mpv's thread: the request goes from the UI thread, as the account's.
            let _ = slint::invoke_from_event_loop(move || super::in_context(slot, || ask(slot, id, priority, offset)));
        },
        arrived_shown,
    );
    feed(&stream, file);
    DOWNLOADS.with(|downloads| {
        let mut downloads = downloads.borrow_mut();
        let download = downloads.entry((slot, id)).or_insert_with(|| Download { waiters: Vec::new(), priority, started: Instant::now(), moving: false, stream: None });
        download.priority = download.priority.max(priority);
        download.stream = Some(Arc::downgrade(&stream));
    });
    ask(slot, id, priority, 0);
    stream
}

/// What TDLib says of `file`, to the stream that reads it.
fn feed(stream: &Stream, file: &File) {
    let path = (!file.local.path.is_empty()).then(|| PathBuf::from(&file.local.path));
    let unsigned = |bytes: i64| u64::try_from(bytes).unwrap_or(0);
    stream.arrived(path, unsigned(file.local.download_offset), unsigned(file.local.downloaded_prefix_size), completed(file).is_some(), unsigned(file.size_or_expected()));
}

/// downloadFile: the file from `offset`, at `priority`; one under way is moved there, or up.
fn ask(slot: Slot, id: i32, priority: i32, offset: u64) {
    let request = json!({ "@type": "downloadFile", "file_id": id, "priority": priority, "offset": offset, "limit": 0, "synchronous": false });
    send(request, move |answer| match answer.map(serde_json::from_value::<File>) {
        // Already complete, or on its way: updateFile says when it is.
        Ok(Ok(file)) => updated(&file),
        Ok(Err(err)) => give_up(slot, id, &format!("unreadable answer: {err}")),
        Err(Error::Stopped) => give_up(slot, id, "finchgram-tdlib stopped"),
        Err(err) => give_up(slot, id, &err.to_string()),
    });
}

/// updateFile, or a download's answer: the stream that reads the file, when one does, is told; a
/// complete file goes to what waits for it, and how fast it came into the log.
pub fn updated(file: &File) {
    let key = (super::context(), file.id);
    let complete = completed(file);
    let (stream, finished) = DOWNLOADS.with(|downloads| {
        let mut downloads = downloads.borrow_mut();
        let Some(download) = downloads.get_mut(&key) else { return (None, None) };
        if complete.is_none() {
            download.moving = true;
        }
        let stream = download.stream.as_ref().and_then(Weak::upgrade);
        let finished = if complete.is_some() { downloads.remove(&key) } else { None };
        (stream, finished)
    });
    if let Some(stream) = stream {
        feed(&stream, file);
    }
    let (Some(path), Some(download)) = (complete, finished) else { return };
    if download.moving {
        let seconds = download.started.elapsed().as_secs_f64();
        let megabytes = file.size_or_expected() as f64 / (1024.0 * 1024.0);
        eprintln!("telegram: file {}: {megabytes:.1} MB in {seconds:.1} s, {:.2} MB/s", file.id, megabytes / seconds.max(0.001));
    }
    for waiter in download.waiters {
        (waiter.ready)(path.clone());
    }
}

/// Logged out, or the account's finchgram-tdlib started again: nothing it was downloading will
/// arrive.
pub fn forget() {
    let slot = super::context();
    DOWNLOADS.with(|downloads| {
        let mut downloads = downloads.borrow_mut();
        for ((kept, _), download) in downloads.iter() {
            if *kept == slot
                && let Some(stream) = download.stream.as_ref().and_then(Weak::upgrade)
            {
                stream.failed();
            }
        }
        downloads.retain(|(kept, _), _| *kept != slot);
    });
}

/// The path of a file that is here: one TDLib calls complete and that is on the disk. One that
/// is gone from the disk (the media cache cleared) is not: asked for, TDLib checks the file,
/// finds it gone, and downloads it again.
pub(super) fn completed(file: &File) -> Option<String> {
    (file.local.is_downloading_completed && !file.local.path.is_empty() && std::path::Path::new(&file.local.path).exists()).then(|| file.local.path.clone())
}

/// What waited for the file waits no longer, and is told so where it asked to be; a stream that
/// read it is over; asking again starts a new download.
fn give_up(slot: Slot, id: i32, why: &str) {
    eprintln!("telegram: cannot download file {id}: {why}");
    let Some(download) = DOWNLOADS.with(|downloads| downloads.borrow_mut().remove(&(slot, id))) else { return };
    if let Some(stream) = download.stream.as_ref().and_then(Weak::upgrade) {
        stream.failed();
    }
    for failed in download.waiters.into_iter().filter_map(|waiter| waiter.failed) {
        failed();
    }
}
