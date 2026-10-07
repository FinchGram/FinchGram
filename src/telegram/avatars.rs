//! The photos of chats and people (TDLib's chat photos and profile photos), which the chat lists,
//! the open chat's rows, the media viewer and the account show instead of a letter square once
//! they are here. The store notes the photos its rows wanted and did not have; after each refresh
//! they are downloaded, in their small size (160 pixels; the account's own in its big size too,
//! for the profile page), decoded off the UI thread and kept in the store (store.rs). A photo is
//! asked for once: one that changes is a new file. A chat without a photo keeps its letter.

use std::cell::Cell;
use std::time::Duration;

use super::api::File;
use super::{files, store};
use crate::images;

/// Photos that arrive together (the whole chat list, at the first start) reach the pages
/// together, after this long, not one by one: each refresh builds every row again.
const REFRESH_AFTER: Duration = Duration::from_millis(50);

thread_local! {
    /// A refresh is on its way for the photos that arrived.
    static REFRESH_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Download and decode `files`, the photos the rows wanted; the pages show them as they arrive.
pub fn fetch(files: Vec<File>) {
    for file in files {
        let id = file.id;
        files::download(&file, files::IN_LISTS, move |path| {
            images::load(path, move |picture| {
                let Some(picture) = picture else { return };
                store::with(|store| {
                    store.avatars.insert(id, picture);
                    store.dirty.chats = true;
                    store.dirty.account = true;
                    store.dirty.viewer = true;
                    // The rows of the open chat show their senders' photos.
                    store.dirty.conversation |= store.open.is_some();
                });
                refresh_soon();
            });
        });
    }
}

/// finchgram-tdlib ended: what it was downloading will not arrive, so the photos it owed are
/// asked for again once it runs.
pub fn forget() {
    store::with(|store| store.ask_photos_again());
}

fn refresh_soon() {
    if REFRESH_PENDING.replace(true) {
        return;
    }
    slint::Timer::single_shot(REFRESH_AFTER, || {
        REFRESH_PENDING.set(false);
        store::refresh();
        super::actions::photos_arrived();
    });
}
