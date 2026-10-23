//! The account's contacts (getContacts), for the search over everything and the dialogs that
//! make a new group, channel or chat (global_search.rs, new_chat.rs). TDLib sends each contact's
//! user before it answers, so the store has them; this keeps their ids, asked for once per account
//! and again after a contact is added.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::json;

use serde_json::Value;
use slint::ComponentHandle;

use super::api::{self, Users};
use super::store::{self, Names, Store};
use super::{Error, Slot, actions, conversation, send, with_ui};
use crate::{Conversation, ExtraWords, MainWindow};

type Then = Box<dyn FnOnce()>;

thread_local! {
    /// Each account's contacts' user ids, once TDLib has said.
    static CONTACTS: RefCell<HashMap<Slot, Vec<i64>>> = RefCell::new(HashMap::new());
    /// What waits for them while they are asked for, by account.
    static WAITING: RefCell<HashMap<Slot, Vec<Then>>> = RefCell::new(HashMap::new());
}

/// Run `then` once the contacts are known: at once when they are, else after TDLib has answered.
pub fn load(then: impl FnOnce() + 'static) {
    let slot = super::context();
    if CONTACTS.with(|contacts| contacts.borrow().contains_key(&slot)) {
        then();
        return;
    }
    let asked = WAITING.with(|waiting| {
        let mut waiting = waiting.borrow_mut();
        let list = waiting.entry(slot).or_default();
        list.push(Box::new(then));
        list.len() > 1
    });
    if asked {
        return;
    }
    send(json!({ "@type": "getContacts" }), move |answer| {
        match answer.map(serde_json::from_value::<Users>) {
            Ok(Ok(users)) => {
                CONTACTS.with(|contacts| contacts.borrow_mut().insert(slot, users.user_ids));
            }
            Ok(Err(err)) => eprintln!("telegram: cannot read the contacts: {err}"),
            Err(Error::Stopped) => {}
            Err(err) => eprintln!("telegram: cannot load the contacts: {err}"),
        }
        for then in WAITING.with(|waiting| waiting.borrow_mut().remove(&slot).unwrap_or_default()) {
            then();
        }
    });
}

/// A contact was added: the list is asked for anew next time.
pub fn changed() {
    let slot = super::context();
    CONTACTS.with(|contacts| contacts.borrow_mut().remove(&slot));
}

/// Logged out: the account's contacts go with it.
pub fn forget() {
    changed();
    let slot = super::context();
    WAITING.with(|waiting| waiting.borrow_mut().remove(&slot));
}

/// Whether the user is among the contacts, as TDLib's list or the user's own flag says. The store
/// is the caller's: this is asked while the caller holds it (global_search.rs builds its rows
/// inside `store::with`), and the store, one RefCell, cannot be borrowed a second time.
pub fn is_contact(store: &Store, user_id: i64) -> bool {
    CONTACTS.with(|contacts| contacts.borrow().get(&super::context()).is_some_and(|contacts| contacts.contains(&user_id)))
        || store.users.get(&user_id).is_some_and(|user| user.is_contact)
}

/// The contacts whose name or username has `words` (lower case, without a leading @), or whose
/// phone number has `digits` when the words are a phone number, ordered by name as the UI language
/// orders names. Every contact when the words are empty. The store is the caller's (is_contact).
pub fn matching(store: &Store, words: &str, digits: Option<&str>, names: &Names) -> Vec<i64> {
    let ids = CONTACTS.with(|contacts| contacts.borrow().get(&super::context()).cloned()).unwrap_or_default();
    let mut found: Vec<(i64, String)> = ids
        .iter()
        .filter_map(|id| store.users.get(id))
        .filter(|user| user.kind != super::api::UserType::Deleted)
        .filter(|user| match digits {
            Some(digits) => user.phone_number.contains(digits),
            None => {
                words.is_empty()
                    || store.user_name(user.id, names).to_lowercase().contains(words)
                    || user.usernames.as_ref().is_some_and(|names| names.active_usernames.iter().any(|name| name.to_lowercase().contains(words)))
            }
        })
        .map(|user| (user.id, store.user_name(user.id, names)))
        .collect();
    found.sort_by(|(a_id, a), (b_id, b)| names.name_order.compare(a, b).then(a_id.cmp(b_id)));
    found.into_iter().map(|(id, _)| id).collect()
}

/// The contact with exactly this phone number (its digits), if any.
pub fn by_phone(digits: &str) -> Option<i64> {
    let ids = CONTACTS.with(|contacts| contacts.borrow().get(&super::context()).cloned()).unwrap_or_default();
    store::with(|store| {
        ids.iter().copied().find(|id| store.users.get(id).is_some_and(|user| user.phone_number.trim_start_matches('+') == digits))
    })
    .flatten()
}

/// The digits of a phone number as typed: "+86 139 1234 5678" → "8613912345678".
pub fn digits(phone: &str) -> String {
    phone.chars().filter(char::is_ascii_digit).collect()
}

/// Whether what was typed is a phone number (compose.js's isPhone): digits, spaces, dashes and
/// brackets only, a plus sign at most in front, and at least five digits.
pub fn is_phone(words: &str) -> bool {
    let words = words.trim();
    let rest = words.strip_prefix('+').unwrap_or(words);
    words.len() >= 5
        && !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit() || matches!(c, ' ' | '-' | '(' | ')'))
        && digits(words).len() >= 5
}

// ---- contact cards in messages (the design's fifth round) ----------------------------------

pub fn connect(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.on_contact_message(|id| {
        if let Ok(message_id) = id.parse() {
            card_message(message_id);
        }
    });
    conversation.on_contact_invite(|id| {
        if let Ok(message_id) = id.parse() {
            card_invite(message_id);
        }
    });
    conversation.on_contact_add(|id| {
        if let Ok(message_id) = id.parse() {
            card_add(message_id);
        }
    });
}

/// The contact a card in the open chat holds.
fn card_contact(message_id: i64) -> Option<api::Contact> {
    store::with(|store| match &store.histories.get(&store.open?)?.messages.get(&message_id)?.content {
        api::MessageContent::Contact { contact } => Some(contact.clone()),
        _ => None,
    })
    .flatten()
}

/// "Message": a chat with them.
fn card_message(message_id: i64) {
    let Some(contact) = card_contact(message_id) else { return };
    if contact.user_id != 0 {
        conversation::open_user(contact.user_id);
    }
}

/// "Invite", for someone not on Telegram: Telegram's invitation onto the clipboard, to send them
/// any way one likes (a desktop has no text messages to send).
fn card_invite(message_id: i64) {
    let Some(contact) = card_contact(message_id) else { return };
    let (mut invitation, mut notice) = (String::new(), String::new());
    with_ui(|ui| {
        let words = ui.global::<ExtraWords>();
        invitation = words.invoke_invitation().to_string();
        notice = words.invoke_invite_copied(contact.name().into()).to_string();
    });
    actions::copy_saying(&invitation, &notice);
}

/// "Add contact": into the account's contacts, as the card names them (addContact for someone
/// on Telegram, importContacts by the number otherwise).
fn card_add(message_id: i64) {
    let Some(contact) = card_contact(message_id) else { return };
    let imported = serde_json::json!({
        "@type": "importedContact", "phone_number": contact.phone_number, "first_name": contact.first_name,
        "last_name": contact.last_name, "note": null,
    });
    let request = if contact.user_id != 0 {
        serde_json::json!({ "@type": "addContact", "user_id": contact.user_id, "contact": imported, "share_phone_number": false })
    } else {
        serde_json::json!({ "@type": "importContacts", "contacts": [imported] })
    };
    let name = contact.name();
    let phone = contact.phone_number.clone();
    send(request, move |answer: Result<Value, Error>| match answer {
        Ok(_) => {
            store::with(|store| {
                store.added_contacts.insert(phone);
                store.dirty.conversation = true;
            });
            store::refresh();
            changed();
            let mut notice = String::new();
            with_ui(|ui| notice = ui.global::<ExtraWords>().invoke_contact_added(name.into()).to_string());
            actions::show_words(&notice);
        }
        Err(Error::Stopped) => {}
        Err(Error::Telegram { message, .. }) => actions::show_failure(&super::errors::explain(&message)),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_number_is_digits_with_a_plus_in_front_at_most() {
        assert!(is_phone("+86 139 1234 5678"));
        assert!(is_phone("13912345678"));
        assert!(is_phone("(628) 555-3390"));
        assert!(!is_phone("1234"), "too few digits");
        assert!(!is_phone("+1 2 3"), "five characters, but three digits");
        assert!(!is_phone("keeb"));
        assert!(!is_phone("lin xia 139"));
        assert!(!is_phone("1+39 1234 5678"), "a plus sign only in front");
        assert_eq!(digits("+86 139-1234 (5678)"), "8613912345678");
    }
}
