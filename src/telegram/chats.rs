//! The chat list: loading its chats from TDLib, and what the pages do with it: show a folder
//! (Broadsheet, Terminal), expand or collapse a folder in the tree (Workbench), search, and a
//! chat's menu (a right click on it: mute, pin, mark as read, put it into one of the account's
//! folders or take it out).
//!
//! TDLib sends the chats of a list, with their positions, as updates once the list has been asked
//! for with loadChats; it says 404 when every chat of the list has been sent.

use std::cell::RefCell;
use std::collections::HashSet;

use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, VecModel};

use super::api::ChatList;
use super::{Error, send};
use super::{actions, conversation, store};
use crate::{ActionNotice, Chats, MainWindow};

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
    chats.on_open_menu(|id| {
        let choices = id.parse().ok().and_then(|chat_id| store::with(|store| store.folder_choices(chat_id))).unwrap_or_default();
        super::with_ui(|ui| ui.global::<Chats>().set_menu_folders(ModelRc::new(VecModel::from(choices))));
    });
    chats.on_toggle_in_folder(|id, folder| {
        let Ok(chat_id) = id.parse() else { return };
        let folder_id = usize::try_from(folder).ok().and_then(|index| store::with(|store| store.folder_list(index)));
        if let Some(ChatList::Folder { chat_folder_id }) = folder_id {
            toggle_in_folder(chat_id, chat_folder_id);
        }
    });
    chats.on_toggle_block(|id| {
        if let Ok(chat_id) = id.parse() {
            toggle_block(chat_id);
        }
    });
    chats.on_report(|id| {
        if let Ok(chat_id) = id.parse() {
            super::actions::start_report(chat_id, Vec::new());
        }
    });
    chats.on_leave(|id| {
        if let Ok(chat_id) = id.parse() {
            ask(chat_id, crate::ChatConfirm::Leave);
        }
    });
    chats.on_delete_chat(|id| {
        if let Ok(chat_id) = id.parse() {
            ask(chat_id, crate::ChatConfirm::Delete);
        }
    });
    chats.on_confirm(confirm);
    chats.on_cancel_confirm(close_question);
    chats.on_search(|words| {
        store::with(|store| {
            store.query = words.trim().to_lowercase();
            store.dirty.chats = true;
        });
        store::refresh();
        super::with_ui(|ui| ui.global::<Chats>().set_query(words));
    });
}

// ---- Telegram's own actions on a chat ---------------------------------------------------------------

thread_local! {
    /// The question open (the chat, and what for), until it is answered or closed.
    static QUESTION: std::cell::RefCell<Option<(i64, crate::ChatConfirm)>> = const { std::cell::RefCell::new(None) };
}

/// Block the user of a chat, after a question; or unblock them at once.
fn toggle_block(chat_id: i64) {
    let Some((user_id, blocked)) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let super::api::ChatType::Private { user_id } = chat.kind else { return None };
        Some((user_id, chat.block_list == Some(super::api::BlockList::Main)))
    })
    .flatten() else {
        return;
    };
    if blocked {
        set_blocked(user_id, false);
    } else {
        ask(chat_id, crate::ChatConfirm::Block);
    }
}

fn set_blocked(user_id: i64, blocked: bool) {
    let block_list = if blocked { serde_json::json!({ "@type": "blockListMain" }) } else { serde_json::Value::Null };
    let request = serde_json::json!({
        "@type": "setMessageSenderBlockList",
        "sender_id": { "@type": "messageSenderUser", "user_id": user_id },
        "block_list": block_list,
    });
    super::send(request, |answer| log_error("block a user", answer));
}

/// The question before something that cannot be undone: the chat's name, its kind, and for a
/// deletion whether the messages can go for the other side too.
fn ask(chat_id: i64, what: crate::ChatConfirm) {
    super::with_ui(|ui| {
        let names = store::Names::from(ui);
        let Some((title, kind, revoke_choice)) = store::with(|store| {
            let chat = store.chats.get(&chat_id)?;
            Some((store.title(chat, &names), store.kind(chat), chat.can_be_deleted_for_all_users))
        })
        .flatten() else {
            return;
        };
        QUESTION.with(|question| *question.borrow_mut() = Some((chat_id, what)));
        let chats = ui.global::<Chats>();
        chats.set_confirm_title(title.into());
        chats.set_confirm_kind(kind);
        chats.set_confirm_revoke_choice(what == crate::ChatConfirm::Delete && revoke_choice);
        chats.set_confirm_revoke(false);
        chats.set_question(what);
    });
}

/// The question answered: do it. A chat left or deleted goes from the lists when TDLib says so,
/// and its tab closes now.
fn confirm() {
    let Some((chat_id, what)) = QUESTION.with(|question| question.borrow_mut().take()) else { return };
    let mut revoke = false;
    super::with_ui(|ui| revoke = ui.global::<Chats>().get_confirm_revoke());
    close_question();
    match what {
        crate::ChatConfirm::Block => {
            let user_id = store::with(|store| match store.chats.get(&chat_id).map(|chat| chat.kind) {
                Some(super::api::ChatType::Private { user_id }) => Some(user_id),
                _ => None,
            })
            .flatten();
            if let Some(user_id) = user_id {
                set_blocked(user_id, true);
            }
        }
        crate::ChatConfirm::Leave => {
            super::send(serde_json::json!({ "@type": "leaveChat", "chat_id": chat_id }), |answer| log_error("leave a chat", answer));
            conversation::close(chat_id);
        }
        crate::ChatConfirm::Delete => {
            let request = serde_json::json!({ "@type": "deleteChatHistory", "chat_id": chat_id, "remove_from_chat_list": true, "revoke": revoke });
            super::send(request, |answer| log_error("delete a chat", answer));
            conversation::close(chat_id);
        }
        crate::ChatConfirm::None => {}
    }
}

fn close_question() {
    QUESTION.with(|question| question.borrow_mut().take());
    super::with_ui(|ui| ui.global::<Chats>().set_question(crate::ChatConfirm::None));
}

/// Esc: close the question, if one is open.
pub(super) fn escape() -> bool {
    let open = QUESTION.with(|question| question.borrow().is_some());
    if open {
        close_question();
    }
    open
}

fn log_error(what: &str, answer: Result<serde_json::Value, super::Error>) {
    if let Err(err) = answer {
        eprintln!("telegram: cannot {what}: {err}");
    }
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

/// Put a chat into one of the account's folders, or take it out of it. TDLib has no request for
/// just that: the folder is fetched whole (getChatFolder), its chosen chats changed, and sent back
/// (editChatFolder). What Telegram refuses (a folder left empty, too many chosen chats) is said in
/// a notice.
pub fn toggle_in_folder(chat_id: i64, folder_id: i32) {
    let list = ChatList::Folder { chat_folder_id: folder_id };
    let inside = store::with(|store| store.chats.get(&chat_id).is_some_and(|chat| store::position(chat, list).is_some())).unwrap_or(false);
    send(json!({ "@type": "getChatFolder", "chat_folder_id": folder_id }), move |answer| {
        let mut folder = match answer {
            Ok(folder) => folder,
            Err(Error::Stopped) => return,
            Err(err) => {
                eprintln!("telegram: cannot read a folder: {err}");
                return;
            }
        };
        if let Some(fields) = folder.as_object_mut() {
            fields.remove("@extra");
        }
        if inside {
            take_out(&mut folder, chat_id);
        } else {
            put_in(&mut folder, chat_id);
        }
        let request = json!({ "@type": "editChatFolder", "chat_folder_id": folder_id, "folder": folder });
        send(request, |answer| match answer {
            Ok(_) | Err(Error::Stopped) => {}
            Err(Error::Telegram { message, .. }) if message.contains("at least 1 chat") => actions::show_notice(ActionNotice::FolderEmpty),
            Err(Error::Telegram { message, .. }) => actions::show_failure(&message),
        });
    });
}

/// The chat among the folder's chosen chats (unless it is pinned there, which includes it), and no
/// longer among the excluded.
fn put_in(folder: &mut Value, chat_id: i64) {
    remove_id(folder, "excluded_chat_ids", chat_id);
    if !has_id(folder, "pinned_chat_ids", chat_id) {
        add_id(folder, "included_chat_ids", chat_id);
    }
}

/// The chat out of the folder's chosen and pinned chats. A folder that takes chats by kind
/// (contacts, groups, …) could keep it all the same, so there it is excluded by name too.
fn take_out(folder: &mut Value, chat_id: i64) {
    const BY_KIND: [&str; 5] = ["include_contacts", "include_non_contacts", "include_bots", "include_groups", "include_channels"];
    remove_id(folder, "included_chat_ids", chat_id);
    remove_id(folder, "pinned_chat_ids", chat_id);
    if BY_KIND.iter().any(|flag| folder[flag].as_bool() == Some(true)) {
        add_id(folder, "excluded_chat_ids", chat_id);
    }
}

fn has_id(folder: &Value, field: &str, chat_id: i64) -> bool {
    folder[field].as_array().is_some_and(|ids| ids.iter().any(|id| id.as_i64() == Some(chat_id)))
}

fn add_id(folder: &mut Value, field: &str, chat_id: i64) {
    if !has_id(folder, field, chat_id) {
        ids(folder, field).push(chat_id.into());
    }
}

fn remove_id(folder: &mut Value, field: &str, chat_id: i64) {
    ids(folder, field).retain(|id| id.as_i64() != Some(chat_id));
}

/// A folder's list of chat ids, made if the folder lacks it.
fn ids<'a>(folder: &'a mut Value, field: &str) -> &'a mut Vec<Value> {
    if !folder[field].is_array() {
        folder[field] = Value::Array(Vec::new());
    }
    folder[field].as_array_mut().expect("an array was just made sure of")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_put_into_a_folder_is_chosen_once_and_excluded_no_more() {
        let mut folder = json!({ "included_chat_ids": [1], "excluded_chat_ids": [5, 7], "pinned_chat_ids": [] });
        put_in(&mut folder, 5);
        assert_eq!(folder["included_chat_ids"], json!([1, 5]));
        assert_eq!(folder["excluded_chat_ids"], json!([7]));
        put_in(&mut folder, 5);
        assert_eq!(folder["included_chat_ids"], json!([1, 5]));
    }

    #[test]
    fn a_chat_pinned_in_a_folder_is_not_chosen_again() {
        let mut folder = json!({ "included_chat_ids": [], "excluded_chat_ids": [], "pinned_chat_ids": [5] });
        put_in(&mut folder, 5);
        assert_eq!(folder["included_chat_ids"], json!([]));
        assert_eq!(folder["pinned_chat_ids"], json!([5]));
    }

    #[test]
    fn a_chat_taken_out_is_excluded_only_where_its_kind_would_keep_it() {
        let mut by_name = json!({ "included_chat_ids": [1, 5], "pinned_chat_ids": [5], "excluded_chat_ids": [], "include_groups": false });
        take_out(&mut by_name, 5);
        assert_eq!(by_name["included_chat_ids"], json!([1]));
        assert_eq!(by_name["pinned_chat_ids"], json!([]));
        assert_eq!(by_name["excluded_chat_ids"], json!([]));

        let mut by_kind = json!({ "included_chat_ids": [5], "pinned_chat_ids": [], "excluded_chat_ids": [], "include_groups": true });
        take_out(&mut by_kind, 5);
        assert_eq!(by_kind["included_chat_ids"], json!([]));
        assert_eq!(by_kind["excluded_chat_ids"], json!([5]));
    }

    #[test]
    fn a_folder_without_the_lists_gets_them() {
        let mut folder = json!({ "@type": "chatFolder", "include_bots": true });
        put_in(&mut folder, 9);
        assert_eq!(folder["included_chat_ids"], json!([9]));
        assert_eq!(folder["excluded_chat_ids"], json!([]));
        assert_eq!(folder["@type"], "chatFolder");
    }
}
