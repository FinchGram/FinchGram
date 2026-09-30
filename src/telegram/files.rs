//! Files: TDLib downloads them into its own folder (downloadFile) and says how far it has got
//! (updateFile). Whatever waits for a file gets its local path once it is complete; a file asked
//! for twice is downloaded once.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::json;

use super::api::File;
use super::{Error, send};

/// How soon a download starts among the others, 1 … 32: what is on screen goes first.
pub const ON_SCREEN: i32 = 16;

type Ready = Box<dyn FnOnce(String)>;

thread_local! {
    /// By file id: what waits for the file.
    static WAITING: RefCell<HashMap<i32, Vec<Ready>>> = RefCell::new(HashMap::new());
}

/// Call `ready` with the local path of `file`, downloading it first unless it is on disk already.
pub fn download(file: &File, priority: i32, ready: impl FnOnce(String) + 'static) {
    if let Some(path) = completed(file) {
        ready(path);
        return;
    }
    let id = file.id;
    let first = WAITING.with(|waiting| {
        let mut waiting = waiting.borrow_mut();
        let list = waiting.entry(id).or_default();
        list.push(Box::new(ready));
        list.len() == 1
    });
    if !first {
        return;
    }
    let request = json!({ "@type": "downloadFile", "file_id": id, "priority": priority, "offset": 0, "limit": 0, "synchronous": false });
    send(request, move |answer| match answer.map(serde_json::from_value::<File>) {
        // Already complete, or on its way: updateFile says when it is.
        Ok(Ok(file)) => updated(&file),
        Ok(Err(err)) => give_up(id, &format!("unreadable answer: {err}")),
        Err(Error::Stopped) => give_up(id, "finchgram-tdlib stopped"),
        Err(err) => give_up(id, &err.to_string()),
    });
}

/// updateFile, or a download's answer: a complete file goes to what waits for it.
pub fn updated(file: &File) {
    let Some(path) = completed(file) else { return };
    let waiting = WAITING.with(|waiting| waiting.borrow_mut().remove(&file.id)).unwrap_or_default();
    for ready in waiting {
        ready(path.clone());
    }
}

/// Logged out, or finchgram-tdlib started again: nothing it was downloading will arrive.
pub fn forget() {
    WAITING.with(|waiting| waiting.borrow_mut().clear());
}

fn completed(file: &File) -> Option<String> {
    (file.local.is_downloading_completed && !file.local.path.is_empty()).then(|| file.local.path.clone())
}

/// What waited for the file waits no longer; asking again starts a new download.
fn give_up(id: i32, why: &str) {
    eprintln!("telegram: cannot download file {id}: {why}");
    WAITING.with(|waiting| waiting.borrow_mut().remove(&id));
}
