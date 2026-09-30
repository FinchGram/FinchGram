//! The chat list: loading its chats from TDLib, and what the pages do with it: show a folder
//! (Broadsheet, Terminal), expand or collapse a folder in the tree (Workbench), search, and a
//! chat's menu (a right click on it: mute, pin, mark as read).
//!
//! TDLib sends the chats of a list, with their positions, as updates once the list has been asked
//! for with loadChats; it says 404 when every chat of the list has been sent.

use std::cell::RefCell;
use std::collections::HashSet;

use serde_json::json;
use slint::ComponentHandle;

use super::api::ChatList;
use super::{conversation, store};
use super::{Error, send};
use crate::{Chats, MainWindow};

/// How many chats one loadChats asks for.
const PAGE: i32 = 100;

thread_local! {
    /// Lists being loaded or loaded completely: each is asked for once.
    static LOADED: RefCell<HashSet<ChatList>> = RefCell::new(HashSet::new());
}

pub fn connect(ui: &MainWindow) {
    let chats = ui.global::<Chats>();
    chats.on_show_folder(|index| {
        store::with(|store| {
            store.shown_folder = usize::try_from(index).unwrap_or(0);
            store.dirty.chats = true;
        });
        store::refresh();
    });
    chats.on_toggle_folder(|index| {
        store::with(|store| {
            let index = usize::try_from(index).unwrap_or(0);
            let id = index.checked_sub(1).and_then(|folder| store.folders.get(folder)).map_or(0, |folder| folder.id);
            let expanded = store.expanded.get(&id).copied().unwrap_or(index > 0 || store.folders.is_empty());
            store.expanded.insert(id, !expanded);
            store.dirty.chats = true;
        });
        store::refresh();
    });
    chats.on_toggle_mute(|id| {
        if let Ok(chat_id) = id.parse() {
            conversation::toggle_mute(chat_id);
        }
    });
    chats.on_toggle_pin(|id, folder| {
        let Ok(chat_id) = id.parse() else { return };
        match usize::try_from(folder) {
            Ok(index) => {
                if let Some(list) = store::with(|store| store.folder_list(index)) {
                    conversation::toggle_pin_in(chat_id, list);
                }
            }
            Err(_) => conversation::toggle_pin(chat_id),
        }
    });
    chats.on_mark_read(|id| {
        if let Ok(chat_id) = id.parse() {
            conversation::mark_read(chat_id);
        }
    });
    chats.on_search(|words| {
        store::with(|store| {
            store.query = words.trim().to_lowercase();
            store.dirty.chats = true;
        });
        store::refresh();
        super::with_ui(|ui| ui.global::<Chats>().set_query(words));
    });
}

/// Logged in, or finchgram-tdlib started again: ask for the main list anew (the folders follow when
/// TDLib sends them).
pub fn load_main_list() {
    forget();
    super::with_ui(|ui| ui.global::<Chats>().set_loaded(false));
    load(ChatList::Main);
}

/// The folders changed: ask for the chats of those not asked for yet.
pub fn load_folders() {
    let lists: Vec<ChatList> = store::with(|store| {
        store.folders.iter().map(|folder| ChatList::Folder { chat_folder_id: folder.id }).collect()
    })
    .unwrap_or_default();
    for list in lists {
        load(list);
    }
}

/// Logged out: the next account's lists are asked for anew.
pub fn forget() {
    LOADED.with(|loaded| loaded.borrow_mut().clear());
}

fn load(list: ChatList) {
    if LOADED.with(|loaded| !loaded.borrow_mut().insert(list)) {
        return;
    }
    load_page(list);
}

fn load_page(list: ChatList) {
    send(json!({ "@type": "loadChats", "chat_list": list.to_json(), "limit": PAGE }), move |answer| match answer {
        Ok(_) => load_page(list),
        // 404: every chat of the list has been sent.
        Err(Error::Telegram { code: 404, .. }) => {
            if list == ChatList::Main {
                super::with_ui(|ui| ui.global::<Chats>().set_loaded(true));
            }
        }
        Err(Error::Stopped) => {
            LOADED.with(|loaded| loaded.borrow_mut().remove(&list));
        }
        Err(err) => {
            eprintln!("telegram: cannot load the chats of {list:?}: {err}");
            if list == ChatList::Main {
                super::with_ui(|ui| ui.global::<Chats>().set_loaded(true));
            }
        }
    });
}
