//! What bots put in a chat beyond messages: the buttons under their messages (an inline
//! keyboard), the keyboard a bot asks the chat to show by the composer (replyMarkupShowKeyboard,
//! taken away by replyMarkupRemoveKeyboard, a reply asked for by replyMarkupForceReply), their
//! commands (offered when "/" is typed) and their menu button; and the chat's newest pinned
//! message, as a bar under the header. There is no design for these yet: the pages draw them
//! plainly, in each theme's own colours.
//!
//! A button does what its kind says, as Telegram's own apps do it: a callback asks the bot
//! (getCallbackQueryAnswer) and shows its words, as a notice or as a dialog, or opens its link; a
//! link asks first, with its site (chats.rs's question); a login button asks Telegram what the
//! site wants (getLoginUrlInfo) and asks the user before logging in (getLoginUrl); a web app
//! opens in the browser (FinchGram has no web view); switch-inline writes "@bot words" into this
//! chat or one the user picks; copy copies; a user opens their chat. A key of the keyboard sends
//! its words; one that asks for the phone number asks the user first and sends a contact card.
//! What FinchGram cannot do yet (a location, a poll, choosing a chat for the bot, paying) is
//! said so.

use std::cell::RefCell;

use serde_json::json;
use slint::{ComponentHandle, SharedString};

use super::api::{self, CallbackQueryAnswer, ChatType, InlineKeyboardButtonType, KeyboardButtonType, LoginUrlInfo, ReplyMarkup, TargetChat};
use super::store::{self, Names};
use super::{Error, actions, chats, conversation, send, with_ui};
use crate::{Actions, ButtonKind, ChatConfirm, Conversation, MainWindow};

/// A login button waiting for the user's word: the chat, the message, the button and its plain link.
struct Login {
    chat_id: i64,
    message_id: i64,
    button_id: i64,
    url: String,
}

thread_local! {
    static LOGIN: RefCell<Option<Login>> = const { RefCell::new(None) };
}

pub fn connect(ui: &MainWindow) {
    let actions = ui.global::<Actions>();
    actions.on_press(|message, row, column| {
        if let Ok(message_id) = message.parse() {
            press(message_id, row, column);
        }
    });
    actions.on_close_alert(close_alert);
    let conversation = ui.global::<Conversation>();
    conversation.on_press_key(press_key);
    conversation.on_toggle_keyboard(toggle_keyboard);
    conversation.on_command_typed(|words| draft_changed(&words));
    conversation.on_send_command(|command| send_command(&command));
    conversation.on_open_bot_menu(open_bot_menu);
    conversation.on_open_pinned(open_pinned);
}

fn open_chat() -> Option<i64> {
    store::with(|store| store.open).flatten()
}

/// A chat came to the front: its newest pinned message, the keyboard it shows, and what its bot
/// (or bots) can do are asked for, each once.
pub fn chat_opened(chat_id: i64) {
    load_pinned(chat_id);
    load_keyboard(chat_id);
    load_bot_info(chat_id);
}

/// Logged out: nothing waits for the account that was.
pub fn forget() {
    LOGIN.with(|login| *login.borrow_mut() = None);
    with_ui(|ui| ui.global::<Actions>().set_alert_open(false));
}

// ---- the pinned message ------------------------------------------------------------------------

/// The chat's newest pinned message, for the bar under the header (getChatPinnedMessage; 404
/// when there is none).
fn load_pinned(chat_id: i64) {
    send(json!({ "@type": "getChatPinnedMessage", "chat_id": chat_id }), move |answer| {
        let pinned = match answer.map(serde_json::from_value::<api::Message>) {
            Ok(Ok(message)) => Some(message),
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read the pinned message: {err}");
                return;
            }
            Err(Error::Telegram { code: 404, .. }) => None,
            Err(Error::Stopped) => return,
            Err(err) => {
                eprintln!("telegram: cannot ask for the pinned message: {err}");
                return;
            }
        };
        store::with(|store| {
            store.pinned.insert(chat_id, pinned);
            store.dirty.header |= store.open == Some(chat_id);
        });
        store::refresh();
    });
}

/// A message of the open chat was pinned or unpinned: the bar follows.
pub fn pinned_changed(chat_id: i64) {
    load_pinned(chat_id);
}

/// The bar clicked: the view goes to the pinned message.
fn open_pinned() {
    let Some(message_id) = store::with(|store| store.pinned.get(&store.open?).and_then(|pinned| pinned.as_ref()).map(|message| message.id)).flatten()
    else {
        return;
    };
    actions::jump(message_id);
}

// ---- the keyboard ------------------------------------------------------------------------------

/// The message whose keyboard the chat shows (TDLib names it in the chat): fetched when it is
/// not among the messages here.
fn load_keyboard(chat_id: i64) {
    let wanted = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        if chat.reply_markup_message_id == 0 || store.keyboards.contains_key(&chat_id) {
            return None;
        }
        if let Some(message) = store.histories.get(&chat_id).and_then(|history| history.messages.get(&chat.reply_markup_message_id)) {
            store.keyboards.insert(chat_id, Some(message.clone()));
            return None;
        }
        Some(chat.reply_markup_message_id)
    })
    .flatten();
    let Some(message_id) = wanted else { return };
    send(json!({ "@type": "getMessage", "chat_id": chat_id, "message_id": message_id }), move |answer| {
        let message = match answer.map(serde_json::from_value::<api::Message>) {
            Ok(Ok(message)) => Some(message),
            Err(Error::Telegram { code: 404, .. }) => None,
            _ => return,
        };
        store::with(|store| {
            store.keyboards.insert(chat_id, message);
            store.dirty.header |= store.open == Some(chat_id);
        });
        store::refresh();
    });
}

/// The chat's keyboard is on another message now, or gone (updateChatReplyMarkup).
pub fn keyboard_changed(chat_id: i64) {
    load_keyboard(chat_id);
}

/// A key of the bot's keyboard pressed (its row and place): its words are sent; the phone number
/// is asked about first; a web app opens; the rest is said to be beyond FinchGram yet.
fn press_key(row: i32, column: i32) {
    let Some(chat_id) = open_chat() else { return };
    let Some((button, bot_user_id, one_time)) = store::with(|store| {
        let message = store.keyboards.get(&chat_id)?.as_ref()?;
        let Some(ReplyMarkup::ShowKeyboard { rows, one_time, .. }) = &message.reply_markup else { return None };
        let button = rows.get(usize::try_from(row).ok()?)?.get(usize::try_from(column).ok()?)?.clone();
        let bot_user_id = match message.sender_id {
            api::MessageSender::User { user_id } => user_id,
            api::MessageSender::Chat { .. } => 0,
        };
        Some((button, bot_user_id, *one_time))
    })
    .flatten() else {
        return;
    };
    match &button.kind {
        KeyboardButtonType::Text | KeyboardButtonType::Other => {
            conversation::send_text(chat_id, &button.text, None);
            if one_time {
                store::with(|store| {
                    store.keyboard_hidden.insert(chat_id);
                    store.dirty.header = true;
                });
                store::refresh();
            }
        }
        KeyboardButtonType::RequestPhoneNumber => {
            let bot = bot_name(bot_user_id);
            chats::ask_bot(chat_id, ChatConfirm::SharePhone, &bot);
        }
        KeyboardButtonType::WebApp { url } => open_web_app_by_url(bot_user_id, url),
        KeyboardButtonType::RequestLocation => say_not_here(ButtonKind::RequestLocation),
        KeyboardButtonType::RequestPoll {} => say_not_here(ButtonKind::RequestPoll),
        KeyboardButtonType::RequestUsers {} => say_not_here(ButtonKind::RequestUsers),
        KeyboardButtonType::RequestChat {} => say_not_here(ButtonKind::RequestChat),
    }
}

/// The keyboard folded away, or brought back.
fn toggle_keyboard() {
    let Some(chat_id) = open_chat() else { return };
    store::with(|store| {
        if !store.keyboard_hidden.remove(&chat_id) {
            store.keyboard_hidden.insert(chat_id);
        }
        store.dirty.header = true;
    });
    store::refresh();
}

/// The user agreed to share their phone number with the bot: a contact card with it goes.
pub(super) fn share_phone(chat_id: i64) {
    let Some(contact) = store::with(|store| {
        let me = store.users.get(&store.my_id)?;
        Some(json!({
            "@type": "contact", "phone_number": format!("+{}", me.phone_number), "first_name": me.first_name,
            "last_name": me.last_name, "vcard": "", "user_id": me.id,
        }))
    })
    .flatten() else {
        return;
    };
    let request = json!({
        "@type": "sendMessage", "chat_id": chat_id, "topic_id": super::topics::topic_json(chat_id), "reply_to": null, "options": null, "reply_markup": null,
        "input_message_content": { "@type": "inputMessageContact", "contact": contact },
    });
    send(request, |answer| log_error("share the phone number", answer));
}

fn bot_name(user_id: i64) -> String {
    with_names(|names| store::with(|store| store.user_name(user_id, names)).unwrap_or_default()).unwrap_or_default()
}

// ---- the buttons under a message ---------------------------------------------------------------

/// A button under a message of the open chat pressed (the message, the button's row and place).
fn press(message_id: i64, row: i32, column: i32) {
    let Some(chat_id) = open_chat() else { return };
    let Some((button, bot_user_id, game)) = store::with(|store| {
        let message = store.histories.get(&chat_id)?.messages.get(&message_id)?;
        let Some(ReplyMarkup::InlineKeyboard { rows }) = &message.reply_markup else { return None };
        let button = rows.get(usize::try_from(row).ok()?)?.get(usize::try_from(column).ok()?)?.clone();
        let bot_user_id = match message.sender_id {
            api::MessageSender::User { user_id } => user_id,
            api::MessageSender::Chat { .. } => 0,
        };
        let game = match &message.content {
            api::MessageContent::Game { game } => game.short_name.clone(),
            _ => String::new(),
        };
        Some((button, bot_user_id, game))
    })
    .flatten() else {
        return;
    };
    let key = format!("{message_id}:{row}:{column}");
    match button.kind {
        InlineKeyboardButtonType::Callback { data } => {
            ask_bot(chat_id, message_id, key, json!({ "@type": "callbackQueryPayloadData", "data": data }));
        }
        InlineKeyboardButtonType::CallbackGame => {
            ask_bot(chat_id, message_id, key, json!({ "@type": "callbackQueryPayloadGame", "game_short_name": game }));
        }
        // The bot wants the account's password with the data (handing a bot over): not here.
        InlineKeyboardButtonType::CallbackWithPassword { .. } => say_not_here(ButtonKind::Buy),
        InlineKeyboardButtonType::Url { url } => chats::ask_link(&url),
        InlineKeyboardButtonType::LoginUrl { url, id, .. } => login(chat_id, message_id, id, url),
        InlineKeyboardButtonType::WebApp { url } => open_web_app(chat_id, bot_user_id, &url),
        InlineKeyboardButtonType::SwitchInline { query, target_chat } => {
            let username = store::with(|store| {
                store.users.get(&bot_user_id).and_then(|bot| bot.usernames.as_ref()).and_then(|names| names.active_usernames.first().cloned())
            })
            .flatten()
            .unwrap_or_default();
            let draft = if query.is_empty() { format!("@{username} ") } else { format!("@{username} {query}") };
            match target_chat {
                TargetChat::Current => actions::put_draft(&draft),
                TargetChat::Chosen => actions::open_picker_for_draft(draft),
            }
        }
        InlineKeyboardButtonType::Buy => say_not_here(ButtonKind::Buy),
        InlineKeyboardButtonType::User { user_id } => conversation::open_user(user_id),
        InlineKeyboardButtonType::CopyText { text } => actions::copy_words_of(&text),
        InlineKeyboardButtonType::Disabled => {}
    }
}

/// Ask the bot what the button means (getCallbackQueryAnswer); the button waits meanwhile. Its
/// words show as a notice, or as a dialog when the bot asks; its link opens.
fn ask_bot(chat_id: i64, message_id: i64, key: String, payload: serde_json::Value) {
    with_actions(|actions| actions.set_busy_button(key.clone().into()));
    let request = json!({ "@type": "getCallbackQueryAnswer", "chat_id": chat_id, "message_id": message_id, "payload": payload });
    send(request, move |answer| {
        with_actions(|actions| {
            if actions.get_busy_button() == key.as_str() {
                actions.set_busy_button(SharedString::new());
            }
        });
        match answer.map(serde_json::from_value::<CallbackQueryAnswer>) {
            Ok(Ok(answer)) => {
                if !answer.text.is_empty() {
                    if answer.show_alert {
                        with_actions(|actions| {
                            actions.set_alert_text(answer.text.clone().into());
                            actions.set_alert_open(true);
                        });
                    } else {
                        actions::show_words(&answer.text);
                    }
                }
                if !answer.url.is_empty() {
                    conversation::open_link(&answer.url);
                }
            }
            Ok(Err(err)) => eprintln!("telegram: cannot read the bot's answer: {err}"),
            Err(Error::Stopped) => {}
            Err(Error::Telegram { message, .. }) => actions::show_failure(&message),
        }
    });
}

/// A login button: Telegram says whether the site may know us at once, or asks first; the user
/// is then asked (chats.rs's question, with the site), and the link opens with their word.
fn login(chat_id: i64, message_id: i64, button_id: i64, url: String) {
    let request = json!({ "@type": "getLoginUrlInfo", "chat_id": chat_id, "message_id": message_id, "button_id": button_id });
    send(request, move |answer| match answer.map(serde_json::from_value::<LoginUrlInfo>) {
        Ok(Ok(LoginUrlInfo::Open { url })) => crate::platform::open_link(&url),
        Ok(Ok(LoginUrlInfo::RequestConfirmation { domain, .. })) => {
            LOGIN.with(|login| *login.borrow_mut() = Some(Login { chat_id, message_id, button_id, url }));
            chats::ask_bot(chat_id, ChatConfirm::Login, &domain);
        }
        Ok(Err(err)) => eprintln!("telegram: cannot read a login link's info: {err}"),
        Err(Error::Stopped) => {}
        // Telegram's own apps open the plain link then.
        Err(_) => chats::ask_link(&url),
    });
}

/// The user agreed to log in: the link with Telegram's token opens (getLoginUrl), or the plain
/// link when Telegram refuses.
pub(super) fn login_confirmed(chat_id: i64) {
    let Some(login) = LOGIN.with(|login| login.borrow_mut().take()).filter(|login| login.chat_id == chat_id) else { return };
    let request = json!({
        "@type": "getLoginUrl", "chat_id": login.chat_id, "message_id": login.message_id, "button_id": login.button_id,
        "allow_write_access": false,
    });
    send(request, move |answer| match answer.map(serde_json::from_value::<api::HttpUrl>) {
        Ok(Ok(http)) => crate::platform::open_link(&http.url),
        Err(Error::Stopped) => {}
        _ => crate::platform::open_link(&login.url),
    });
}

/// A web app from a button under a message: opened through TDLib (openWebApp), which gives the
/// address with what the bot needs, then in the browser: FinchGram has no web view.
fn open_web_app(chat_id: i64, bot_user_id: i64, url: &str) {
    let request = json!({
        "@type": "openWebApp", "chat_id": chat_id, "bot_user_id": bot_user_id, "url": url, "topic_id": null, "reply_to": null,
        "parameters": web_app_parameters(),
    });
    send(request, |answer| match answer.map(serde_json::from_value::<api::WebAppInfo>) {
        Ok(Ok(info)) => crate::platform::open_link(&info.url.url),
        Ok(Err(err)) => eprintln!("telegram: cannot read the web app's address: {err}"),
        Err(Error::Stopped) => {}
        Err(Error::Telegram { message, .. }) => actions::show_failure(&message),
    });
}

/// A web app from a key of the keyboard or from the bot's menu button (getWebAppUrl).
fn open_web_app_by_url(bot_user_id: i64, url: &str) {
    let request = json!({ "@type": "getWebAppUrl", "bot_user_id": bot_user_id, "url": url, "parameters": web_app_parameters() });
    send(request, |answer| match answer.map(serde_json::from_value::<api::WebAppUrl>) {
        Ok(Ok(web)) => crate::platform::open_link(&web.url),
        Ok(Err(err)) => eprintln!("telegram: cannot read the web app's address: {err}"),
        Err(Error::Stopped) => {}
        Err(Error::Telegram { message, .. }) => actions::show_failure(&message),
    });
}

fn web_app_parameters() -> serde_json::Value {
    json!({ "@type": "webAppOpenParameters", "theme": null, "application_name": "FinchGram", "mode": { "@type": "webAppOpenModeFullSize" } })
}

/// What a bot asked that FinchGram cannot do here yet: said for a moment, in the pages' words.
fn say_not_here(kind: ButtonKind) {
    with_ui(|ui| {
        let words = ui.global::<crate::BotWords>().invoke_not_here(kind);
        if !words.is_empty() {
            actions::show_words(&words);
        }
    });
}

/// The bot's answer shown as a dialog: closed.
fn close_alert() {
    with_actions(|actions| actions.set_alert_open(false));
    actions::focus_composer();
}

/// Esc, when the keyboard is elsewhere: close the bot's dialog if it is open.
pub(super) fn escape() -> bool {
    let mut open = false;
    with_actions(|actions| open = actions.get_alert_open());
    if open {
        close_alert();
    }
    open
}

// ---- commands and the menu button --------------------------------------------------------------

/// What a bot (or the bots of a group) can do: its menu button and its commands, asked for once.
fn load_bot_info(chat_id: i64) {
    enum Ask {
        Bot(i64),
        BasicGroup(i64),
        Nothing,
    }
    let ask = store::with(|store| {
        let Some(chat) = store.chats.get(&chat_id) else { return Ask::Nothing };
        match chat.kind {
            ChatType::Private { user_id } | ChatType::Secret { user_id } => {
                let bot = store.users.get(&user_id).is_some_and(|user| user.kind == api::UserType::Bot);
                if bot && !store.bot_infos.contains_key(&user_id) { Ask::Bot(user_id) } else { Ask::Nothing }
            }
            ChatType::BasicGroup { basic_group_id } if !store.group_commands.contains_key(&chat_id) => Ask::BasicGroup(basic_group_id),
            // A supergroup's full info is asked for when it opens (conversation.rs), commands and all.
            _ => Ask::Nothing,
        }
    })
    .unwrap_or(Ask::Nothing);
    match ask {
        Ask::Bot(user_id) => {
            send(json!({ "@type": "getUserFullInfo", "user_id": user_id }), move |answer| {
                // TDLib sends updateUserFullInfo with the same, which the store takes in.
                if let Err(err) = answer {
                    log_error("ask what a bot does", Err(err));
                }
            });
        }
        Ask::BasicGroup(basic_group_id) => {
            send(json!({ "@type": "getBasicGroupFullInfo", "basic_group_id": basic_group_id }), move |answer| {
                let Ok(Ok(info)) = answer.map(serde_json::from_value::<api::BasicGroupFullInfo>) else { return };
                store::with(|store| {
                    store.group_commands.insert(chat_id, info.bot_commands);
                    store.dirty.header |= store.open == Some(chat_id);
                });
                store::refresh();
            });
        }
        Ask::Nothing => {}
    }
}

/// The composer's words changed: "/words" alone (no space yet) offers the bot's commands that
/// start so.
pub fn draft_changed(words: &str) {
    let filter = words.strip_prefix('/').filter(|rest| !rest.contains(char::is_whitespace)).map(|rest| rest.to_lowercase());
    let changed = store::with(|store| {
        if store.command_filter == filter {
            return false;
        }
        store.command_filter = filter;
        store.dirty.header = true;
        true
    })
    .unwrap_or(false);
    if changed {
        store::refresh();
    }
}

/// A command chosen from the list: sent as it is, and the composer emptied.
fn send_command(command: &str) {
    let Some(chat_id) = open_chat() else { return };
    actions::put_draft("");
    draft_changed("");
    conversation::send_text(chat_id, command, None);
}

/// The bot's menu button: its web app opens; without one, "/" goes into the composer, which
/// offers the commands.
fn open_bot_menu() {
    let Some(chat_id) = open_chat() else { return };
    let Some((bot_user_id, url)) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let (ChatType::Private { user_id } | ChatType::Secret { user_id }) = chat.kind else { return None };
        let info = store.bot_infos.get(&user_id)?;
        Some((user_id, info.menu_button.as_ref().map(|button| button.url.clone()).unwrap_or_default()))
    })
    .flatten() else {
        return;
    };
    if url.is_empty() {
        actions::put_draft("/");
        draft_changed("/");
    } else {
        open_web_app_by_url(bot_user_id, &url);
    }
}

fn with_actions(change: impl FnOnce(&Actions)) {
    with_ui(|ui| change(&ui.global::<Actions>()));
}

fn with_names<R>(read: impl FnOnce(&Names) -> R) -> Option<R> {
    let mut names = None;
    with_ui(|ui| names = Some(Names::from(ui)));
    names.map(|names| read(&names))
}

fn log_error(what: &str, answer: Result<serde_json::Value, Error>) {
    match answer {
        Ok(_) | Err(Error::Stopped) => {}
        Err(err) => eprintln!("telegram: cannot {what}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_command_is_offered_while_it_is_typed_alone() {
        let filter = |words: &str| words.strip_prefix('/').filter(|rest| !rest.contains(char::is_whitespace)).map(|rest| rest.to_lowercase());
        assert_eq!(filter("/"), Some(String::new()));
        assert_eq!(filter("/Sta"), Some("sta".into()));
        assert_eq!(filter("/start now"), None);
        assert_eq!(filter("hello"), None);
    }
}
