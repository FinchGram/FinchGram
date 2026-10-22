//! Sharing to other chats (the design's XOverlay): a channel's post from the share link under it,
//! or a contact card from its Share button, to several chats at once, with words of our own
//! sent first when given. Posts and messages are forwarded (forwardMessages), so that those who
//! get them see where they came from; a contact goes as a contact card (inputMessageContact). A
//! chat that protects its content cannot be shared from, and the page says so. What Telegram
//! refuses (CHAT_FORWARDS_RESTRICTED, FLOOD_WAIT, …) is shown in the panel, which stays open.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{self, ChatList};
use super::store::{self, Names};
use super::{Error, actions, errors, send, with_ui};
use crate::{ChatKind, Conversation, ExtraWords, MainWindow, Share, ShareKind, ShareRow};

/// How many chats the panel lists at most.
const ROWS: usize = 60;

/// What is being shared.
enum Payload {
    Messages { from_chat_id: i64, message_ids: Vec<i64> },
    Contact(api::Contact),
}

struct Sharing {
    payload: Payload,
    picked: Vec<i64>,
    query: String,
}

thread_local! {
    static SHARING: RefCell<Option<Sharing>> = const { RefCell::new(None) };
}

pub fn connect(ui: &MainWindow) {
    let share = ui.global::<Share>();
    share.on_search(|query| {
        SHARING.with(|sharing| {
            if let Some(sharing) = sharing.borrow_mut().as_mut() {
                sharing.query = query.trim().to_lowercase();
            }
        });
        fill();
    });
    share.on_toggle(|id| {
        if let Ok(chat_id) = id.parse::<i64>() {
            SHARING.with(|sharing| {
                if let Some(sharing) = sharing.borrow_mut().as_mut() {
                    if let Some(at) = sharing.picked.iter().position(|picked| *picked == chat_id) {
                        sharing.picked.remove(at);
                    } else {
                        sharing.picked.push(chat_id);
                    }
                }
            });
            fill();
        }
    });
    share.on_send(send_all);
    share.on_close(close);
    let conversation = ui.global::<Conversation>();
    conversation.on_share_message(|id| {
        if let Ok(message_id) = id.parse() {
            start_message(message_id);
        }
    });
    conversation.on_contact_share(|id| {
        if let Ok(message_id) = id.parse() {
            start_contact(message_id);
        }
    });
}

fn with_share(change: impl FnOnce(&Share)) {
    with_ui(|ui| change(&ui.global::<Share>()));
}

fn open_chat() -> Option<i64> {
    store::with(|store| store.open).flatten()
}

/// A post or message of the open chat: shared by forwarding, unless the chat forbids it.
fn start_message(message_id: i64) {
    let Some(chat_id) = open_chat() else { return };
    let Some((protected, preview)) = with_names(|names| {
        store::with(|store| {
            let chat = store.chats.get(&chat_id)?;
            let message = store.histories.get(&chat_id)?.messages.get(&message_id)?;
            let (content, text, detail) = store.content(message, names);
            let words = if text.is_empty() { detail } else { text };
            let words = if words.is_empty() { format!("{content:?}") } else { words };
            let from = if store.kind(chat) == ChatKind::Channel { store.title(chat, names) } else { store.sender_name(&message.sender_id, names) };
            Some((chat.has_protected_content, format!("{from}: {}", words.lines().next().unwrap_or_default())))
        })
        .flatten()
    })
    .flatten() else {
        return;
    };
    if protected {
        say(|words| words.invoke_cannot_forward());
        return;
    }
    open(Payload::Messages { from_chat_id: chat_id, message_ids: vec![message_id] }, ShareKind::Message, preview);
}

/// A contact card of the open chat: shared as a card.
fn start_contact(message_id: i64) {
    let Some(chat_id) = open_chat() else { return };
    let Some(contact) = store::with(|store| match &store.histories.get(&chat_id)?.messages.get(&message_id)?.content {
        api::MessageContent::Contact { contact } => Some(contact.clone()),
        _ => None,
    })
    .flatten() else {
        return;
    };
    let preview = format!("👤 {} · {}", contact.name(), store::phone_text(&contact.phone_number));
    open(Payload::Contact(contact), ShareKind::Contact, preview);
}

fn open(payload: Payload, kind: ShareKind, preview: String) {
    SHARING.with(|sharing| *sharing.borrow_mut() = Some(Sharing { payload, picked: Vec::new(), query: String::new() }));
    with_share(|share| {
        share.set_kind(kind);
        share.set_preview(preview.into());
        share.set_query(SharedString::new());
        share.set_comment(SharedString::new());
        share.set_error(SharedString::new());
        share.set_busy(false);
        share.set_picked(0);
        share.set_open(true);
    });
    fill();
}

/// The chats one may share to, matching the words typed: those we can write in, but not
/// channels nor groups with topics (a topic would have to be chosen), newest activity first.
fn fill() {
    let Some((query, picked)) = SHARING.with(|sharing| sharing.borrow().as_ref().map(|sharing| (sharing.query.clone(), sharing.picked.clone()))) else {
        return;
    };
    with_ui(|ui| {
        let names = Names::from(ui);
        let words = ui.global::<ExtraWords>();
        let rows = store::with(|store| {
            let mut chats: Vec<(&api::Chat, i64)> = store
                .chats
                .values()
                .filter_map(|chat| store::position(chat, ChatList::Main).map(|position| (chat, position.order)))
                .filter(|(chat, _)| store.can_write(chat) && store.kind(chat) != ChatKind::Channel && !store.is_forum(chat))
                .filter(|(chat, _)| query.is_empty() || store.title(chat, &names).to_lowercase().contains(&query))
                .collect();
            chats.sort_by(|(a, a_order), (b, b_order)| {
                let a_saved = store.kind(a) == ChatKind::Saved;
                let b_saved = store.kind(b) == ChatKind::Saved;
                b_saved.cmp(&a_saved).then(b_order.cmp(a_order)).then(b.id.cmp(&a.id))
            });
            chats
                .into_iter()
                .take(ROWS)
                .map(|(chat, _)| {
                    let row = store.chat_row(chat, ChatList::Main, &names);
                    let kind = store.kind(chat);
                    ShareRow {
                        id: row.id.clone(),
                        title: row.title.clone(),
                        initial: row.initial.clone(),
                        picture: row.picture.clone(),
                        has_picture: row.has_picture,
                        kind,
                        sub: words.invoke_chat_sub(kind, row.members, row.online),
                        picked: picked.contains(&chat.id),
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
        let share = ui.global::<Share>();
        share.set_rows(ModelRc::new(VecModel::from(rows)));
        share.set_picked(picked.len() as i32);
    });
    store::fetch_wanted_photos();
}

/// Photos of chats arrived (avatars.rs): the panel's rows show them too.
pub fn photos_arrived() {
    if SHARING.with(|sharing| sharing.borrow().is_some()) {
        fill();
    }
}

/// Send to every chat picked: our words first when there are any, then what is shared. The
/// panel closes when all went; what Telegram refused stays in it.
fn send_all() {
    let Some((payload, picked)) = SHARING.with(|sharing| {
        sharing.borrow().as_ref().map(|sharing| {
            let payload = match &sharing.payload {
                Payload::Messages { from_chat_id, message_ids } => Payload::Messages { from_chat_id: *from_chat_id, message_ids: message_ids.clone() },
                Payload::Contact(contact) => Payload::Contact(contact.clone()),
            };
            (payload, sharing.picked.clone())
        })
    }) else {
        return;
    };
    if picked.is_empty() {
        return;
    }
    let mut comment = String::new();
    with_share(|share| {
        comment = share.get_comment().trim().to_string();
        share.set_busy(true);
        share.set_error(SharedString::new());
    });
    let payload = Rc::new(payload);
    // One answer per chat: when every one is in, the panel closes or shows the first refusal.
    let left = Rc::new(RefCell::new((picked.len(), Option::<String>::None)));
    for chat_id in picked {
        if !comment.is_empty() {
            super::conversation::send_text(chat_id, &comment, None);
        }
        let request = match &*payload {
            Payload::Messages { from_chat_id, message_ids } => json!({
                "@type": "forwardMessages", "chat_id": chat_id, "topic_id": null, "from_chat_id": from_chat_id,
                "message_ids": message_ids, "options": null, "send_copy": false, "remove_caption": false,
            }),
            Payload::Contact(contact) => json!({
                "@type": "sendMessage", "chat_id": chat_id, "topic_id": null, "reply_to": null, "options": null, "reply_markup": null,
                "input_message_content": { "@type": "inputMessageContact", "contact": contact.to_json() },
            }),
        };
        let left = left.clone();
        send(request, move |answer| {
            let mut left = left.borrow_mut();
            left.0 -= 1;
            if let Err(Error::Telegram { message, .. }) = answer
                && left.1.is_none()
            {
                left.1 = Some(errors::explain(&message));
            }
            if left.0 > 0 {
                return;
            }
            let sent = SHARING.with(|sharing| sharing.borrow().as_ref().map_or(0, |sharing| sharing.picked.len()));
            match left.1.take() {
                Some(error) => with_share(|share| {
                    share.set_busy(false);
                    share.set_error(error.into());
                }),
                None => {
                    close();
                    say(|words| words.invoke_sent(sent as i32));
                }
            }
        });
    }
}

fn close() {
    SHARING.with(|sharing| *sharing.borrow_mut() = None);
    with_share(|share| {
        share.set_open(false);
        share.set_busy(false);
        share.set_rows(ModelRc::default());
    });
    actions::focus_composer();
}

/// Esc: the panel closes if it is open.
pub(super) fn escape() -> bool {
    let open = SHARING.with(|sharing| sharing.borrow().is_some());
    if open {
        close();
    }
    open
}

pub fn forget() {
    SHARING.with(|sharing| *sharing.borrow_mut() = None);
    with_share(|share| {
        share.set_open(false);
        share.set_rows(ModelRc::default());
    });
}

/// A notice in the pages' words.
fn say(words: impl FnOnce(&ExtraWords) -> SharedString) {
    let mut text = SharedString::new();
    with_ui(|ui| text = words(&ui.global::<ExtraWords>()));
    if !text.is_empty() {
        actions::show_words(&text);
    }
}

fn with_names<R>(read: impl FnOnce(&Names) -> R) -> Option<R> {
    let mut names = None;
    with_ui(|ui| names = Some(Names::from(ui)));
    names.map(|names| read(&names))
}
