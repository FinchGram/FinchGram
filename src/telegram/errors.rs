//! Telegram's refusals in the user's words: the codes its servers answer with (FLOOD_WAIT_12,
//! SLOWMODE_WAIT_30, CHAT_ADMIN_REQUIRED, REACTION_INVALID, TOPIC_CLOSED,
//! CHAT_FORWARDS_RESTRICTED, …) become the pages' sentences (ui/extras.slint's ExtraWords);
//! anything else is shown as Telegram said it.

use slint::ComponentHandle;

use super::with_ui;
use crate::ExtraWords;

/// What Telegram said, explained where the pages have words for it.
pub fn explain(message: &str) -> String {
    let (code, number) = split(message);
    let mut words = String::new();
    with_ui(|ui| words = ui.global::<ExtraWords>().invoke_telegram_error(code.into(), number).to_string());
    if words.is_empty() { message.to_string() } else { words }
}

/// A code's trailing number, when it has one: "FLOOD_WAIT_12" is ("FLOOD_WAIT", 12).
pub fn split(message: &str) -> (&str, i32) {
    let code = message.trim();
    match code.rsplit_once('_') {
        Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => (head, tail.parse().unwrap_or(0)),
        _ => (code, 0),
    }
}

/// The seconds a FLOOD_WAIT or SLOWMODE_WAIT asks for, when the message is one.
pub fn wait_seconds(message: &str) -> Option<i32> {
    match split(message) {
        ("FLOOD_WAIT" | "SLOWMODE_WAIT", seconds) if seconds > 0 => Some(seconds),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_split_from_their_numbers() {
        assert_eq!(split("FLOOD_WAIT_12"), ("FLOOD_WAIT", 12));
        assert_eq!(split("SLOWMODE_WAIT_30"), ("SLOWMODE_WAIT", 30));
        assert_eq!(split("CHAT_ADMIN_REQUIRED"), ("CHAT_ADMIN_REQUIRED", 0));
        assert_eq!(split("Bad Request: message is too long"), ("Bad Request: message is too long", 0));
        assert_eq!(wait_seconds("SLOWMODE_WAIT_7"), Some(7));
        assert_eq!(wait_seconds("TOPIC_CLOSED"), None);
    }
}
