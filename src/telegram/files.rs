//! Files: TDLib downloads them into its own folder (downloadFile) and says how far it has got
//! (updateFile). Whatever waits for a file gets its local path once it is complete; a file asked
//! for twice is downloaded once.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::json;

use super::api::File;
use super::{Error, Slot, send};

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

thread_local! {
    /// By account and file id (TDLib's file ids are each account's own): what waits for the file.
    static WAITING: RefCell<HashMap<(Slot, i32), Vec<Waiter>>> = RefCell::new(HashMap::new());
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
    let first = WAITING.with(|waiting| {
        let mut waiting = waiting.borrow_mut();
        let list = waiting.entry((slot, id)).or_default();
        list.push(waiter);
        list.len() == 1
    });
    if !first {
        return;
    }
    let request = json!({ "@type": "downloadFile", "file_id": id, "priority": priority, "offset": 0, "limit": 0, "synchronous": false });
    send(request, move |answer| match answer.map(serde_json::from_value::<File>) {
        // Already complete, or on its way: updateFile says when it is.
        Ok(Ok(file)) => updated(&file),
        Ok(Err(err)) => give_up(slot, id, &format!("unreadable answer: {err}")),
        Err(Error::Stopped) => give_up(slot, id, "finchgram-tdlib stopped"),
        Err(err) => give_up(slot, id, &err.to_string()),
    });
}

/// updateFile, or a download's answer: a complete file goes to what waits for it.
pub fn updated(file: &File) {
    let Some(path) = completed(file) else { return };
    let waiting = WAITING.with(|waiting| waiting.borrow_mut().remove(&(super::context(), file.id))).unwrap_or_default();
    for waiter in waiting {
        (waiter.ready)(path.clone());
    }
}

/// Logged out, or the account's finchgram-tdlib started again: nothing it was downloading will
/// arrive.
pub fn forget() {
    let slot = super::context();
    WAITING.with(|waiting| waiting.borrow_mut().retain(|(kept, _), _| *kept != slot));
}

/// The path of a file that is here: one TDLib calls complete and that is on the disk. One that
/// is gone from the disk (the media cache cleared) is not: asked for, TDLib checks the file,
/// finds it gone, and downloads it again.
pub(super) fn completed(file: &File) -> Option<String> {
    (file.local.is_downloading_completed && !file.local.path.is_empty() && std::path::Path::new(&file.local.path).exists()).then(|| file.local.path.clone())
}

/// What waited for the file waits no longer, and is told so where it asked to be; asking again
/// starts a new download.
fn give_up(slot: Slot, id: i32, why: &str) {
    eprintln!("telegram: cannot download file {id}: {why}");
    let waiting = WAITING.with(|waiting| waiting.borrow_mut().remove(&(slot, id))).unwrap_or_default();
    for failed in waiting.into_iter().filter_map(|waiter| waiter.failed) {
        failed();
    }
}
