//! The account's contacts (getContacts), for the search over everything and the dialogs that
//! make a new group, channel or chat (global_search.rs, new_chat.rs). TDLib sends each contact's
//! user before it answers, so the store has them; this keeps their ids, asked for once per account
//! and again after a contact is added.

use std::cell::RefCell;

use serde_json::json;

use super::api::Users;
use super::store::{self, Names};
use super::{Error, send};

type Then = Box<dyn FnOnce()>;

thread_local! {
    /// The contacts' user ids, once TDLib has said.
    static CONTACTS: RefCell<Option<Vec<i64>>> = const { RefCell::new(None) };
    /// What waits for them while they are asked for.
    static WAITING: RefCell<Vec<Then>> = const { RefCell::new(Vec::new()) };
}

/// Run `then` once the contacts are known: at once when they are, else after TDLib has answered.
pub fn load(then: impl FnOnce() + 'static) {
    if CONTACTS.with(|contacts| contacts.borrow().is_some()) {
        then();
        return;
    }
    let asked = WAITING.with(|waiting| {
        let mut waiting = waiting.borrow_mut();
        waiting.push(Box::new(then));
        waiting.len() > 1
    });
    if asked {
        return;
    }
    send(json!({ "@type": "getContacts" }), |answer| {
        match answer.map(serde_json::from_value::<Users>) {
            Ok(Ok(users)) => CONTACTS.with(|contacts| *contacts.borrow_mut() = Some(users.user_ids)),
            Ok(Err(err)) => eprintln!("telegram: cannot read the contacts: {err}"),
            Err(Error::Stopped) => {}
            Err(err) => eprintln!("telegram: cannot load the contacts: {err}"),
        }
        for then in WAITING.with(|waiting| std::mem::take(&mut *waiting.borrow_mut())) {
            then();
        }
    });
}

/// A contact was added: the list is asked for anew next time.
pub fn changed() {
    CONTACTS.with(|contacts| *contacts.borrow_mut() = None);
}

/// Logged out: the next account's contacts are its own.
pub fn forget() {
    changed();
    WAITING.with(|waiting| waiting.borrow_mut().clear());
}

/// Whether the user is among the contacts, as TDLib's list or the user's own flag says.
pub fn is_contact(user_id: i64) -> bool {
    CONTACTS.with(|contacts| contacts.borrow().as_ref().is_some_and(|contacts| contacts.contains(&user_id)))
        || store::with(|store| store.users.get(&user_id).is_some_and(|user| user.is_contact)).unwrap_or(false)
}

/// The contacts whose name or username has `words` (lower case, without a leading @), or whose
/// phone number has `digits` when the words are a phone number, ordered by name as the UI language
/// orders names. Every contact when the words are empty.
pub fn matching(words: &str, digits: Option<&str>, names: &Names) -> Vec<i64> {
    let ids = CONTACTS.with(|contacts| contacts.borrow().clone()).unwrap_or_default();
    store::with(|store| {
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
    })
    .unwrap_or_default()
}

/// The contact with exactly this phone number (its digits), if any.
pub fn by_phone(digits: &str) -> Option<i64> {
    let ids = CONTACTS.with(|contacts| contacts.borrow().clone()).unwrap_or_default();
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
