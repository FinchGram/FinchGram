//! Searching in the open chat (the chat header's magnifying glass, Terminal's "[search]", ⌘F). TDLib
//! looks the words up (searchChatMessages, which asks Telegram's servers for every chat but a
//! secret one), a page of matches at a time, newest first, and the view goes to the matches one by
//! one the way it goes to a reply's original (actions.rs): the page around a match far up takes the
//! place of the messages loaded, and the chat's end comes again on the way down (conversation.rs).

use std::cell::RefCell;
use std::time::Duration;

use serde_json::json;
use slint::ComponentHandle;

use super::api::FoundChatMessages;
use super::{Error, actions, send, store, with_ui};
use crate::{Conversation, MainWindow};

/// After the last keystroke, before the words are looked up: each keystroke does not ask the servers.
const PAUSE: Duration = Duration::from_millis(300);
/// Matches asked for at a time (TDLib's most).
const PAGE: i32 = 100;

struct Search {
    /// The chat searched; 0 for none.
    chat_id: i64,
    words: String,
    /// The matches found so far, newest first.
    found: Vec<i64>,
    /// The message TDLib's next page of matches starts from; 0 when `found` has them all.
    next_from: i64,
    /// How many there are in all, as TDLib says; -1 before it has answered.
    total: i32,
    /// Which of `found` the view is at.
    at: Option<usize>,
    /// A step asked while an answer was on its way: to the next match up (true) or down.
    step: Option<bool>,
    /// Counted up each time the words change, so that the answer to older words is let go.
    lookups: u64,
    /// A lookup is on its way.
    waiting: bool,
}

impl Default for Search {
    fn default() -> Search {
        Search { chat_id: 0, words: String::new(), found: Vec::new(), next_from: 0, total: -1, at: None, step: None, lookups: 0, waiting: false }
    }
}

thread_local! {
    static STATE: RefCell<Search> = RefCell::new(Search::default());
    static PAUSE_TIMER: slint::Timer = slint::Timer::default();
}

pub fn connect(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.on_search(|words| search(&words));
    conversation.on_search_step(step);
    conversation.on_close_search(close);
}

fn open_chat() -> Option<i64> {
    store::with(|store| store.open).flatten()
}

/// The words in the search line changed: looked up after a pause. Without words, nothing is
/// looked for and nothing is shown.
fn search(words: &str) {
    let words = words.trim().to_string();
    let Some(chat_id) = open_chat() else { return };
    let changed = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.chat_id == chat_id && state.words == words {
            return false;
        }
        let lookups = state.lookups + 1;
        *state = Search { chat_id, words: words.clone(), lookups, ..Search::default() };
        true
    });
    if !changed {
        return;
    }
    PAUSE_TIMER.with(|timer| timer.stop());
    show();
    if !words.is_empty() {
        PAUSE_TIMER.with(|timer| timer.start(slint::TimerMode::SingleShot, PAUSE, lookup));
    }
}

/// Ask TDLib for a page of matches: the first, or the one after those here. The first match is
/// gone to as soon as it is known; a step asked meanwhile is taken then.
fn lookup() {
    let Some((chat_id, words, from, lookups)) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.chat_id == 0 || state.words.is_empty() || state.waiting {
            return None;
        }
        state.waiting = true;
        Some((state.chat_id, state.words.clone(), state.next_from, state.lookups))
    }) else {
        return;
    };
    show();
    let request = json!({
        "@type": "searchChatMessages", "chat_id": chat_id, "query": words, "sender_id": null,
        "from_message_id": from, "offset": 0, "limit": PAGE, "filter": { "@type": "searchMessagesFilterEmpty" },
    });
    send(request, move |answer| {
        let found = match answer.map(serde_json::from_value::<FoundChatMessages>) {
            Ok(Ok(found)) => Some(found),
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read the messages found: {err}");
                None
            }
            Err(Error::Stopped) => None,
            Err(err) => {
                eprintln!("telegram: cannot search a chat: {err}");
                None
            }
        };
        enum Then {
            Go(usize),
            Step(bool),
            Nothing,
        }
        let then = STATE.with(|state| {
            let mut state = state.borrow_mut();
            // The words changed, or the chat: this answer is to what was.
            if state.lookups != lookups || state.chat_id != chat_id {
                return Then::Nothing;
            }
            state.waiting = false;
            let Some(found) = found else { return Then::Nothing };
            state.found.extend(found.messages.iter().map(|message| message.id));
            state.next_from = found.next_from_message_id;
            let here = state.found.len() as i32;
            state.total = if found.total_count >= 0 { found.total_count.max(here) } else { here };
            match (state.at, state.step.take()) {
                (None, _) if !state.found.is_empty() => Then::Go(0),
                (Some(_), Some(older)) => Then::Step(older),
                _ => Then::Nothing,
            }
        });
        match then {
            Then::Go(index) => go(index),
            Then::Step(older) => step(older),
            Then::Nothing => show(),
        }
    });
}

/// ⏎, the arrows: the next match up the chat (older, `older`) or down it. Beyond the matches
/// here, the next page of them is asked for first.
fn step(older: bool) {
    enum Then {
        Go(usize),
        More,
        Nothing,
    }
    let then = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.words.is_empty() {
            return Then::Nothing;
        }
        let Some(at) = state.at else {
            // The first answer is still on its way: its first match, once it is here.
            if state.waiting {
                state.step = Some(older);
            }
            return Then::Nothing;
        };
        if !older {
            return if at > 0 { Then::Go(at - 1) } else { Then::Nothing };
        }
        if at + 1 < state.found.len() {
            Then::Go(at + 1)
        } else if state.next_from != 0 {
            state.step = Some(true);
            if state.waiting { Then::Nothing } else { Then::More }
        } else {
            Then::Nothing
        }
    });
    match then {
        Then::Go(index) => go(index),
        Then::More => lookup(),
        Then::Nothing => {}
    }
}

/// The view goes to the match at `index` of those found.
fn go(index: usize) {
    let message_id = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.at = Some(index);
        state.found.get(index).copied()
    });
    show();
    if let Some(message_id) = message_id {
        actions::jump(message_id);
    }
}

/// The search line closes: nothing is looked for, and the keyboard goes back to the composer.
fn close() {
    reset();
    with_ui(|ui| ui.global::<Conversation>().set_search_open(false));
    show();
    actions::focus_composer();
}

/// The open chat changed, or closed: a search is of its chat alone, and goes with it.
pub(super) fn chat_changed(open: Option<i64>) {
    if STATE.with(|state| state.borrow().chat_id == open.unwrap_or(0)) {
        return;
    }
    reset();
    with_ui(|ui| ui.global::<Conversation>().set_search_open(false));
    show();
}

fn reset() {
    PAUSE_TIMER.with(|timer| timer.stop());
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let lookups = state.lookups + 1;
        *state = Search { lookups, ..Search::default() };
    });
}

/// Tell the pages where the search is: how many matches, which the view is at, and whether an
/// answer is on its way.
fn show() {
    let (total, at, waiting) = STATE.with(|state| {
        let state = state.borrow();
        (state.total, state.at.map_or(0, |at| at as i32 + 1), state.waiting)
    });
    with_ui(|ui| {
        let conversation = ui.global::<Conversation>();
        conversation.set_search_count(total);
        conversation.set_search_at(at);
        conversation.set_search_waiting(waiting);
    });
}
