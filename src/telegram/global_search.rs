//! The search over everything (the design's compose.js: the title bar's box, ⌘K, Terminal's
//! /search): with nothing typed, the chats used last; with words, the chats here whose names have
//! them and the contacts (by name, username, or phone number when the words are one), then what
//! Telegram's servers find among public users, groups and channels (searchPublicChats), a person
//! found by their phone number (searchUserByPhoneNumber, which finds those who let others find
//! them so), and messages (searchMessages). A public group or channel opens as a card first, with
//! its description, and joins from there (joinChat); a person opens as a chat; a message opens its
//! chat at the message. A phone number nobody has can be added as a contact (new_chat.rs).
//!
//! The local lists are made at each keystroke; the servers are asked after a pause, and an answer
//! to older words is let go.

use std::cell::RefCell;
use std::time::Duration;

use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{self, ChatJoinResult, ChatList, ChatType, Chats as ChatIds, FoundMessages, SupergroupFullInfo};
use super::store::{self, Names};
use super::{Error, actions, contacts, conversation, new_chat, send, with_ui};
use crate::{ChatKind, FoundKind, FoundRow, FoundSection, MainWindow, Search, Status};

/// After the last keystroke, before the servers are asked.
const PAUSE: Duration = Duration::from_millis(300);
/// The chats used last, shown with nothing typed.
const RECENT: usize = 6;
/// The chats here and the contacts shown at most for the words (compose.js shows five of each).
const LOCAL: usize = 5;
/// Messages asked for (TDLib chooses how many it returns, up to this).
const MESSAGES: i32 = 20;
/// Words asked of the servers only from this long on.
const SHORTEST: usize = 2;
/// A phone number is looked up from this many digits: shorter ones are no whole number.
const PHONE_DIGITS: usize = 7;

/// What a row of the results stands for.
#[derive(Clone, Debug, PartialEq)]
enum Found {
    Chat(i64),
    /// A person, by their user id (their chat may not be made yet).
    User(i64),
    /// A public group or channel, not joined: its card first.
    Public(i64),
    Message { chat_id: i64, message_id: i64 },
}

#[derive(Default)]
struct State {
    open: bool,
    /// The words as typed, and lower case without a leading @.
    query: String,
    words: String,
    /// The words are a phone number: its digits.
    digits: Option<String>,
    /// Counted up each time the words change, so that the answer to older words is let go.
    lookups: u64,
    /// Answers still on their way: public chats, the person by phone, messages.
    waiting: u8,
    /// What the servers found for the words.
    public: Vec<i64>,
    by_phone: Option<i64>,
    /// None: not asked, or asked and not answered; Some(false): nobody has the number.
    phone_found: Option<bool>,
    messages: Vec<api::Message>,
    /// Telegram refused something: its words.
    error: String,
    /// The rows shown, in order (the one ↑↓ are at is the page's Search.selected: the pointer
    /// sets it there too).
    rows: Vec<Found>,
    /// The card open, if any: the public chat.
    card: Option<i64>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static PAUSE_TIMER: slint::Timer = slint::Timer::default();
}

pub fn connect(ui: &MainWindow) {
    let search = ui.global::<Search>();
    search.on_show(show);
    search.on_show_with(|line| show_with(&line));
    search.on_close(close);
    search.on_edited(|words| edited(&words));
    search.on_move(move_selection);
    search.on_choose(choose);
    search.on_add_contact(add_contact);
    search.on_join(join);
    search.on_back(back);
}

fn with_search(change: impl FnOnce(&Search)) {
    with_ui(|ui| change(&ui.global::<Search>()));
}

/// ⌘K, the search box: open, with nothing typed; ⌘K again closes (the design's toggle).
fn show() {
    if STATE.with(|state| state.borrow().open) {
        close();
        return;
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let lookups = state.lookups + 1;
        *state = State { open: true, lookups, ..State::default() };
    });
    PAUSE_TIMER.with(|timer| timer.stop());
    with_search(|search| {
        search.set_query(SharedString::new());
        search.set_card_open(false);
        search.set_card_about(SharedString::new());
        search.set_error(SharedString::new());
        search.set_open(true);
    });
    contacts::load(fill);
    fill();
}

/// Terminal's "/search words": open with the words.
fn show_with(line: &str) {
    let words = line.trim().strip_prefix("/search").unwrap_or(line).trim();
    show();
    with_search(|search| search.set_query(words.into()));
    edited(words);
}

fn close() {
    PAUSE_TIMER.with(|timer| timer.stop());
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let lookups = state.lookups + 1;
        *state = State { lookups, ..State::default() };
    });
    with_search(|search| {
        search.set_open(false);
        search.set_card_open(false);
    });
    actions::focus_composer();
}

/// Esc, when the keyboard is elsewhere: close the search if it is open.
pub(super) fn escape() -> bool {
    let open = STATE.with(|state| state.borrow().open);
    if open {
        close();
    }
    open
}

/// Logged out: nothing found belongs to the next account.
pub fn forget() {
    if STATE.with(|state| state.borrow().open) {
        close();
    }
}

/// The words changed: the local lists now, the servers after a pause.
fn edited(words: &str) {
    let query = words.trim().to_string();
    let lower = query.to_lowercase();
    let plain = lower.strip_prefix('@').unwrap_or(&lower).to_string();
    let digits = contacts::is_phone(&query).then(|| contacts::digits(&query));
    let changed = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.query == query {
            return false;
        }
        let lookups = state.lookups + 1;
        *state = State { open: true, query, words: plain, digits, lookups, ..State::default() };
        true
    });
    if !changed {
        return;
    }
    PAUSE_TIMER.with(|timer| timer.stop());
    with_search(|search| search.set_selected(0));
    fill();
    let asks = STATE.with(|state| {
        let state = state.borrow();
        state.digits.as_ref().is_some_and(|digits| digits.len() >= PHONE_DIGITS)
            || (state.digits.is_none() && state.words.chars().count() >= SHORTEST)
    });
    if asks {
        PAUSE_TIMER.with(|timer| timer.start(slint::TimerMode::SingleShot, PAUSE, lookup));
    }
}

/// Ask the servers for the words: public chats and messages, or the person with the phone number.
fn lookup() {
    let Some((lookups, query, digits)) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if !state.open || state.words.is_empty() {
            return None;
        }
        state.waiting = if state.digits.is_some() { 1 } else { 2 };
        Some((state.lookups, state.query.clone(), state.digits.clone()))
    }) else {
        return;
    };
    fill();
    if let Some(digits) = digits {
        let request = json!({ "@type": "searchUserByPhoneNumber", "phone_number": format!("+{digits}"), "only_local": false });
        send(request, move |answer| {
            let found = match answer {
                Ok(user) => user["id"].as_i64().map(Some),
                Err(Error::Telegram { code: 404, .. }) => Some(None),
                Err(Error::Stopped) => None,
                Err(err) => {
                    note_error(lookups, &err);
                    None
                }
            };
            answered(lookups, |state| {
                if let Some(found) = found {
                    state.by_phone = found;
                    state.phone_found = Some(found.is_some());
                }
            });
        });
        return;
    }
    let plain = STATE.with(|state| state.borrow().words.clone());
    send(json!({ "@type": "searchPublicChats", "query": plain, "type_filter": null }), move |answer| {
        let found = match answer.map(serde_json::from_value::<ChatIds>) {
            Ok(Ok(chats)) => chats.chat_ids,
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read the public chats found: {err}");
                Vec::new()
            }
            Err(Error::Stopped) => Vec::new(),
            Err(err) => {
                note_error(lookups, &err);
                Vec::new()
            }
        };
        answered(lookups, |state| state.public = found);
    });
    let request = json!({
        "@type": "searchMessages", "chat_list": null, "query": query, "offset": "", "limit": MESSAGES,
        "filter": null, "chat_type_filter": null, "min_date": 0, "max_date": 0,
    });
    send(request, move |answer| {
        let found = match answer.map(serde_json::from_value::<FoundMessages>) {
            Ok(Ok(found)) => found.messages,
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read the messages found: {err}");
                Vec::new()
            }
            Err(Error::Stopped) => Vec::new(),
            Err(err) => {
                note_error(lookups, &err);
                Vec::new()
            }
        };
        answered(lookups, |state| state.messages = found);
    });
}

/// An answer came, for the words `lookups` (let go when the words changed since).
fn answered(lookups: u64, take: impl FnOnce(&mut State)) {
    let current = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.lookups != lookups {
            return false;
        }
        state.waiting = state.waiting.saturating_sub(1);
        take(&mut state);
        true
    });
    if current {
        fill();
    }
}

fn note_error(lookups: u64, err: &Error) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.lookups == lookups {
            state.error = err.to_string();
        }
    });
}

/// Build the rows from what is here and what the servers said, and show them.
fn fill() {
    with_ui(|ui| {
        let names = Names::from(ui);
        let search = ui.global::<Search>();
        let (words, digits, waiting, error, public, by_phone, phone_found, messages) = STATE.with(|state| {
            let state = state.borrow();
            (
                state.words.clone(),
                state.digits.clone(),
                state.waiting > 0,
                state.error.clone(),
                state.public.clone(),
                state.by_phone,
                state.phone_found,
                state.messages.clone(),
            )
        });
        search.set_waiting(waiting);
        search.set_error(error.into());
        if words.is_empty() {
            let rows = recent_rows(&names);
            let found: Vec<Found> = rows.iter().map(|(found, _)| found.clone()).collect();
            let count = found.len() as i32;
            STATE.with(|state| state.borrow_mut().rows = found);
            search.set_empty(true);
            search.set_recent(ModelRc::new(VecModel::from(rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>())));
            search.set_sections(ModelRc::default());
            search.set_count(count);
            search.set_selected(0);
            search.set_none(false);
            search.set_phone_missed(false);
            store::fetch_wanted_photos();
            return;
        }
        let mut sections: Vec<FoundSection> = Vec::new();
        let mut flat: Vec<Found> = Vec::new();
        let mut index = 0;
        let mut section = |kind: FoundKind, rows: Vec<(Found, FoundRow)>, flat: &mut Vec<Found>, sections: &mut Vec<FoundSection>| {
            if rows.is_empty() {
                return;
            }
            let mut shown = Vec::with_capacity(rows.len());
            for (found, mut row) in rows {
                row.index = index;
                index += 1;
                flat.push(found);
                shown.push(row);
            }
            sections.push(FoundSection { kind, count: shown.len() as i32, rows: ModelRc::new(VecModel::from(shown)) });
        };

        // The chats here and the contacts, one section.
        let mut local = store::with(|store| {
            let mut rows: Vec<(Found, FoundRow)> = Vec::new();
            let mut shown_users = std::collections::HashSet::new();
            if digits.is_none() {
                let mut chats: Vec<(&api::Chat, i64)> = store
                    .chats
                    .values()
                    .filter_map(|chat| store::position(chat, ChatList::Main).map(|position| (chat, position.order)))
                    .filter(|(chat, _)| store.title(chat, &names).to_lowercase().contains(&words))
                    .collect();
                chats.sort_by(|(a, a_order), (b, b_order)| b_order.cmp(a_order).then(b.id.cmp(&a.id)));
                for (chat, _) in chats.into_iter().take(LOCAL) {
                    if let ChatType::Private { user_id } | ChatType::Secret { user_id } = chat.kind {
                        shown_users.insert(user_id);
                    }
                    rows.push((Found::Chat(chat.id), chat_row(store, chat, FoundKind::Chat, &names)));
                }
            }
            let found: Vec<i64> = contacts::matching(&words, digits.as_deref(), &names).into_iter().filter(|id| !shown_users.contains(id)).take(LOCAL).collect();
            for user_id in found {
                if let Some(user) = store.users.get(&user_id) {
                    shown_users.insert(user_id);
                    rows.push((Found::User(user_id), user_row(store, user, FoundKind::Contact, &names)));
                }
            }
            (rows, shown_users)
        })
        .unwrap_or_default();
        let shown_users = std::mem::take(&mut local.1);
        let contacts_found = local.0.iter().any(|(found, _)| matches!(found, Found::User(_)));
        section(FoundKind::Chat, local.0, &mut flat, &mut sections);

        if digits.is_some() {
            // The person with the number, unless they are a contact (shown above already).
            let hit = by_phone.filter(|id| !shown_users.contains(id) && !contacts::is_contact(*id));
            let rows = store::with(|store| {
                hit.and_then(|id| store.users.get(&id)).map(|user| vec![(Found::User(user.id), user_row(store, user, FoundKind::ByPhone, &names))]).unwrap_or_default()
            })
            .unwrap_or_default();
            let found_one = !rows.is_empty();
            section(FoundKind::ByPhone, rows, &mut flat, &mut sections);
            let missed = !contacts_found && !found_one && phone_found == Some(false);
            search.set_phone_missed(missed);
            // A number too short to look up, and nobody here with it.
            search.set_none(sections.is_empty() && !waiting && !missed);
        } else {
            search.set_phone_missed(false);
            // Telegram's public chats, without those joined and the contacts.
            let rows = store::with(|store| {
                public
                    .iter()
                    .filter_map(|id| store.chats.get(id))
                    .filter(|chat| store::position(chat, ChatList::Main).is_none())
                    .filter(|chat| match chat.kind {
                        ChatType::Private { user_id } | ChatType::Secret { user_id } => !contacts::is_contact(user_id) && !shown_users.contains(&user_id),
                        _ => true,
                    })
                    .map(|chat| {
                        let found = match (chat.kind, store.kind(chat)) {
                            (ChatType::Private { user_id } | ChatType::Secret { user_id }, _) => Found::User(user_id),
                            (_, ChatKind::Group | ChatKind::Channel) => Found::Public(chat.id),
                            _ => Found::Chat(chat.id),
                        };
                        (found, chat_row(store, chat, FoundKind::Public, &names))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
            section(FoundKind::Public, rows, &mut flat, &mut sections);
            let rows = store::with(|store| {
                messages
                    .iter()
                    .filter_map(|message| {
                        let chat = store.chats.get(&message.chat_id)?;
                        let row = message_row(store, chat, message, &names);
                        Some((Found::Message { chat_id: message.chat_id, message_id: message.id }, row))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
            section(FoundKind::Message, rows, &mut flat, &mut sections);
            search.set_none(sections.is_empty() && !waiting);
        }

        let count = flat.len();
        STATE.with(|state| state.borrow_mut().rows = flat);
        search.set_empty(false);
        search.set_recent(ModelRc::default());
        search.set_sections(ModelRc::new(VecModel::from(sections)));
        search.set_count(count as i32);
        search.set_selected(search.get_selected().clamp(0, (count as i32 - 1).max(0)));
    });
    // The rows are built outside a refresh: their photos are downloaded here.
    store::fetch_wanted_photos();
}

/// Photos of chats and people arrived (avatars.rs): the rows show them.
pub fn photos_arrived() {
    if STATE.with(|state| state.borrow().open) {
        fill();
    }
}

/// The chats used last: the main list's first ones, in Telegram's order.
fn recent_rows(names: &Names) -> Vec<(Found, FoundRow)> {
    store::with(|store| {
        let mut chats: Vec<(&api::Chat, i64)> = store
            .chats
            .values()
            .filter_map(|chat| store::position(chat, ChatList::Main).map(|position| (chat, position.order)))
            .collect();
        chats.sort_by(|(a, a_order), (b, b_order)| b_order.cmp(a_order).then(b.id.cmp(&a.id)));
        chats.into_iter().take(RECENT).map(|(chat, _)| (Found::Chat(chat.id), chat_row(store, chat, FoundKind::Chat, names))).collect()
    })
    .unwrap_or_default()
}

/// A row with nothing in it, to build the others from.
fn blank_row(store: &store::Store) -> FoundRow {
    FoundRow { picture: store.blank_image(), status: Status::None, ..FoundRow::default() }
}

fn chat_row(store: &store::Store, chat: &api::Chat, section: FoundKind, names: &Names) -> FoundRow {
    let title = store.title(chat, names);
    let (picture, has_picture) = store.chat_avatar(chat);
    let (status, last_seen, members) = store.status_of(chat);
    let user = match chat.kind {
        ChatType::Private { user_id } | ChatType::Secret { user_id } => store.users.get(&user_id),
        _ => None,
    };
    let username = match chat.kind {
        ChatType::Supergroup { supergroup_id, .. } => store.supergroups.get(&supergroup_id).and_then(|group| group.usernames.as_ref()),
        _ => user.and_then(|user| user.usernames.as_ref()),
    }
    .and_then(|names| names.active_usernames.first())
    .cloned()
    .unwrap_or_default();
    FoundRow {
        section,
        id: chat.id.to_string().into(),
        user_id: user.map(|user| user.id.to_string()).unwrap_or_default().into(),
        initial: store::initial(&title),
        title: title.into(),
        picture,
        has_picture,
        kind: store.kind(chat),
        username: username.into(),
        status,
        last_seen,
        members,
        ..blank_row(store)
    }
}

fn user_row(store: &store::Store, user: &api::User, section: FoundKind, names: &Names) -> FoundRow {
    let title = store.user_name(user.id, names);
    let (picture, has_picture) = store.user_avatar(user);
    let (status, last_seen) = store::user_status(user);
    FoundRow {
        section,
        id: SharedString::new(),
        user_id: user.id.to_string().into(),
        initial: store::initial(&title),
        title: title.into(),
        picture,
        has_picture,
        kind: if user.kind == api::UserType::Bot { ChatKind::Bot } else { ChatKind::User },
        username: user.usernames.as_ref().and_then(|names| names.active_usernames.first()).cloned().unwrap_or_default().into(),
        status,
        last_seen,
        phone: if user.phone_number.is_empty() { SharedString::new() } else { format!("+{}", user.phone_number).into() },
        ..blank_row(store)
    }
}

fn message_row(store: &store::Store, chat: &api::Chat, message: &api::Message, names: &Names) -> FoundRow {
    let (content, text, detail) = store.content(message, names);
    let sender = if message.is_outgoing { String::new() } else { store.sender_name(&message.sender_id, names) };
    // A chat with a person: the row says what they wrote, not who (the title says).
    let sender = if matches!(store.kind(chat), ChatKind::User | ChatKind::Bot | ChatKind::Saved) { String::new() } else { sender };
    FoundRow {
        message_id: message.id.to_string().into(),
        sender: sender.into(),
        outgoing: message.is_outgoing,
        content,
        text: text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim().into(),
        detail: detail.into(),
        time: super::time::moment(message.date),
        ..chat_row(store, chat, FoundKind::Message, names)
    }
}

// ---- the keyboard, and what a row does ---------------------------------------------------------

/// ↑ (-1) and ↓ (+1).
fn move_selection(by: i32) {
    let count = STATE.with(|state| state.borrow().rows.len()) as i32;
    with_search(|search| search.set_selected((search.get_selected() + by).clamp(0, (count - 1).max(0))));
}

/// A row clicked (its index), or ⏎ (-1: the row selected). ⏎ with no row and a number nobody has
/// adds the contact.
fn choose(index: i32) {
    let index = if index < 0 { with_ui_value(|ui| ui.global::<Search>().get_selected()).unwrap_or(0) } else { index };
    let found = STATE.with(|state| usize::try_from(index).ok().and_then(|index| state.borrow().rows.get(index).cloned()));
    let Some(found) = found else {
        let missed = with_ui_value(|ui| ui.global::<Search>().get_phone_missed()).unwrap_or(false);
        if missed && index < 0 {
            add_contact();
        }
        return;
    };
    match found {
        Found::Chat(chat_id) => {
            close();
            conversation::open(chat_id);
        }
        Found::User(user_id) => {
            close();
            conversation::open_user(user_id);
        }
        Found::Message { chat_id, message_id } => {
            close();
            conversation::open_at(chat_id, message_id);
        }
        Found::Public(chat_id) => open_card(chat_id),
    }
}

fn with_ui_value<R>(read: impl FnOnce(&MainWindow) -> R) -> Option<R> {
    let mut value = None;
    with_ui(|ui| value = Some(read(ui)));
    value
}

/// The card of a public group or channel: its name, username, size and description (fetched), and
/// the button that joins it.
fn open_card(chat_id: i64) {
    let row = with_ui_value(|ui| {
        let names = Names::from(ui);
        store::with(|store| store.chats.get(&chat_id).map(|chat| chat_row(store, chat, FoundKind::Public, &names))).flatten()
    })
    .flatten();
    let Some(row) = row else { return };
    STATE.with(|state| state.borrow_mut().card = Some(chat_id));
    with_search(|search| {
        search.set_card(row);
        search.set_card_about(SharedString::new());
        search.set_card_busy(false);
        search.set_error(SharedString::new());
        search.set_card_open(true);
    });
    let supergroup_id = store::with(|store| match store.chats.get(&chat_id).map(|chat| chat.kind) {
        Some(ChatType::Supergroup { supergroup_id, .. }) => Some(supergroup_id),
        _ => None,
    })
    .flatten();
    if let Some(supergroup_id) = supergroup_id {
        send(json!({ "@type": "getSupergroupFullInfo", "supergroup_id": supergroup_id }), move |answer| {
            let Ok(Ok(info)) = answer.map(serde_json::from_value::<SupergroupFullInfo>) else { return };
            store::with(|store| {
                store.supergroup_members.insert(supergroup_id, info.member_count);
            });
            if STATE.with(|state| state.borrow().card) != Some(chat_id) {
                return;
            }
            with_ui(|ui| {
                let names = Names::from(ui);
                let search = ui.global::<Search>();
                if let Some(row) = store::with(|store| store.chats.get(&chat_id).map(|chat| chat_row(store, chat, FoundKind::Public, &names))).flatten() {
                    search.set_card(row);
                }
                search.set_card_about(info.description.into());
            });
        });
    }
}

fn back() {
    STATE.with(|state| state.borrow_mut().card = None);
    with_search(|search| {
        search.set_card_open(false);
        search.set_error(SharedString::new());
    });
}

/// Join the group or subscribe to the channel on the card, then open it. A group that approves
/// its members takes a request instead: said on the card.
fn join() {
    let Some(chat_id) = STATE.with(|state| state.borrow().card) else { return };
    with_search(|search| {
        search.set_card_busy(true);
        search.set_error(SharedString::new());
    });
    send(json!({ "@type": "joinChat", "chat_id": chat_id }), move |answer| {
        if STATE.with(|state| state.borrow().card) != Some(chat_id) {
            return;
        }
        with_search(|search| search.set_card_busy(false));
        match answer.map(serde_json::from_value::<ChatJoinResult>) {
            Ok(Ok(ChatJoinResult::Success { .. } | ChatJoinResult::Other)) => {
                close();
                conversation::open(chat_id);
            }
            Ok(Ok(ChatJoinResult::RequestSent)) => with_search(|search| search.set_error(request_sent_words().into())),
            Ok(Err(err)) => with_search(|search| search.set_error(err.to_string().into())),
            Err(Error::Stopped) => {}
            Err(err) => with_search(|search| search.set_error(err.to_string().into())),
        }
    });
}

/// What the card says when the group's administrators have to let one in.
fn request_sent_words() -> String {
    with_ui_value(|ui| ui.global::<crate::ComposeWords>().invoke_request_sent().to_string()).unwrap_or_default()
}

/// The number nobody has: the Add contact dialog, with it.
fn add_contact() {
    let query = STATE.with(|state| state.borrow().query.clone());
    close();
    new_chat::start_contact(&query);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_too_short_for_the_servers_is_still_a_number() {
        assert!(contacts::is_phone("+86 139"));
        assert!(contacts::digits("+86 139").len() < PHONE_DIGITS);
    }
}
