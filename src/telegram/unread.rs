//! The Unread page (the design's round 2): the chats with unread messages, as the Unread group of
//! the chat list gathers them, each with its newest unread messages, one line each. An index, not a
//! reader: the lines come from getChatHistory, which marks nothing read; only opening a chat does
//! (conversation.rs, viewMessages). The page is a tab in Workbench ([`store::UNREAD_TAB`] among the
//! tabs) and takes the chat's place in Broadsheet and Terminal. store.rs builds its groups and says
//! which chats' lines it still wants; here they are fetched.

use serde_json::json;
use slint::ComponentHandle;

use super::api::{self, Messages};
use super::{Error, actions, conversation, send, store};
use crate::{MainWindow, Unread};

/// How many of a chat's newest messages one getChatHistory asks for: enough for the lines once our
/// own messages, which are never unread, are left out.
const ASKED: i32 = 20;
/// How many pages are asked for when the first comes back short (TDLib answers from its database
/// first) before the lines are shown as they are.
const PAGES: u32 = 3;

pub fn connect(ui: &MainWindow) {
    let unread = ui.global::<Unread>();
    unread.on_show(show);
    unread.on_open_message(|chat, message| {
        if let (Ok(chat_id), Ok(message_id)) = (chat.parse(), message.parse()) {
            conversation::open_at(chat_id, message_id);
        }
    });
    unread.on_open_chat(|chat| {
        if let Ok(chat_id) = chat.parse() {
            conversation::open(chat_id);
        }
    });
    unread.on_mark_read(|chat| {
        if let Ok(chat_id) = chat.parse() {
            conversation::mark_read(chat_id);
        }
    });
    unread.on_mark_all_read(mark_all_read);
}

/// Bring the page to the front: its tab (opened if needed), in the open chat's place.
pub fn show() {
    let previous = store::with(|store| {
        let previous = store.open.replace(store::UNREAD_TAB);
        store.opened_unread = false;
        if !store.tabs.contains(&store::UNREAD_TAB) {
            store.tabs.push(store::UNREAD_TAB);
        }
        store.dirty.chats = true;
        store.dirty.conversation = true;
        store.dirty.unread = true;
        previous
    })
    .flatten();
    if previous != Some(store::UNREAD_TAB) {
        actions::chat_changed(None);
        if let Some(previous) = previous {
            send(json!({ "@type": "closeChat", "chat_id": previous }), |_| {});
        }
    }
    store::refresh();
}

/// Mark every chat of the page as read.
fn mark_all_read() {
    let chats: Vec<i64> = store::with(|store| store.unread_chats(false).iter().map(|chat| chat.id).collect()).unwrap_or_default();
    for chat_id in chats {
        conversation::mark_read(chat_id);
    }
}

/// Fetch the lines of these chats: each one's newest messages, of which the unread ones that are
/// not ours, the newest [`store::UNREAD_LINES`] of them, become its lines.
pub fn fetch(chat_ids: Vec<i64>) {
    for chat_id in chat_ids {
        fetch_page(chat_id, 0, Vec::new(), PAGES);
    }
}

/// One page of `chat_id`'s history before `from_message_id` (0: the newest), added to `collected`;
/// another is asked for while the lines are short and older unread messages may follow.
fn fetch_page(chat_id: i64, from_message_id: i64, mut collected: Vec<api::Message>, pages_left: u32) {
    let request = json!({
        "@type": "getChatHistory", "chat_id": chat_id, "from_message_id": from_message_id,
        "offset": 0, "limit": ASKED, "only_local": false,
    });
    send(request, move |answer| {
        let messages = match answer.map(serde_json::from_value::<Messages>) {
            Ok(Ok(messages)) => messages.messages.into_iter().flatten().collect::<Vec<_>>(),
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read a chat's unread messages: {err}");
                Vec::new()
            }
            Err(Error::Stopped) => Vec::new(),
            Err(err) => {
                eprintln!("telegram: cannot read a chat's unread messages: {err}");
                Vec::new()
            }
        };
        let oldest = messages.iter().map(|message| message.id).min();
        collected.extend(messages);
        let read_up_to = store::with(|store| store.chats.get(&chat_id).map_or(0, |chat| chat.last_read_inbox_message_id)).unwrap_or(0);
        let unread_lines = collected.iter().filter(|message| !message.is_outgoing && message.id > read_up_to).count();
        // Older unread messages may still follow while the oldest message here is unread itself.
        if unread_lines < store::UNREAD_LINES
            && pages_left > 0
            && let Some(oldest) = oldest.filter(|oldest| *oldest > read_up_to)
        {
            fetch_page(chat_id, oldest, collected, pages_left - 1);
            return;
        }
        store::with(|store| {
            let mut unread: Vec<api::Message> = collected.into_iter().filter(|message| !message.is_outgoing && message.id > read_up_to).collect();
            unread.sort_by_key(|message| message.id);
            let skip = unread.len().saturating_sub(store::UNREAD_LINES);
            let entry = store.unread_index.entry(chat_id).or_default();
            entry.messages = unread.into_iter().skip(skip).collect();
            entry.loading = false;
            entry.stale = false;
            store.dirty.unread = true;
        });
        store::refresh();
    });
}
