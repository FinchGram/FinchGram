//! What can be done with a message (the design's fourth round, "FinchGram Desktop 消息操作", and its
//! message-actions.js): its menu (a right click on it), replying, editing, copying its words, its
//! picture or its link, saving its photo or video, forwarding, reporting, deleting, and choosing
//! several messages to do some of that with at once.
//!
//! As in Telegram's own apps, the menu leaves out what cannot be done with the message, and TDLib
//! says what that is (getMessageProperties): no link in basic groups, no copying, forwarding or
//! saving where the chat restricts saving content, no replying in a channel one only reads.
//!
//! The strip above the composer (replying, editing, forwarding) belongs to the open chat; opening
//! another chat drops it, but for forwarding, which opens the chat it forwards to.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde_json::json;
use slint::winit_030::winit::event::{ElementState, MouseButton, WindowEvent};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{self, ChatType, MessageContent, MessageProperties, ReportChatResult, ReportOption};
use super::store::{self, Names};
use super::{Error, conversation, files, send, time, viewer, with_ui};
use crate::{ActionNotice, Actions, ChatKind, ComposeBar, Conversation, DeleteChoice, Fmt, MainWindow, MessageAction};

/// How long a notice stays, and how long a message jumped to is lit up (the design's).
const NOTICE_SHOWN: Duration = Duration::from_millis(2000);
const FLASH_SHOWN: Duration = Duration::from_millis(1500);
/// Older pages loaded at most, looking for the message a reply answers.
const JUMP_PAGES: u32 = 20;
/// A report's words, at most (reportChat).
const REPORT_TEXT_LIMIT: usize = 1024;
/// What the user asked for is downloaded before anything else.
const ASKED: i32 = 32;
/// The forward picker's chats, at most.
const PICKER_CHATS: usize = 100;

/// The strip above the composer.
enum Bar {
    Reply { message_id: i64 },
    /// `words`: what the composer was given, with the message's formatting written as Markdown when
    /// `markdown` (read back when saving, so that it is kept); `draft`: what was being written
    /// before, back when editing ends.
    Edit { message_id: i64, words: String, markdown: bool, draft: String },
    Forward { from_chat_id: i64, message_ids: Vec<i64> },
}

/// A message's menu while it is open.
struct Menu {
    chat_id: i64,
    row: i64,
    /// The message it does things with: the photo or video clicked, else the row's first.
    message_id: i64,
    /// The row's messages: several for an album, which is forwarded, deleted, … whole.
    messages: Vec<i64>,
}

/// A report, step by step: the lists of reasons so far, then maybe words to add.
struct Report {
    chat_id: i64,
    message_ids: Vec<i64>,
    steps: Vec<(String, Vec<ReportOption>)>,
    /// The last step: the reason the words go with, its name (the title), whether they are needed.
    comment: Option<(String, String, bool)>,
    /// The reason chosen last, whose name titles the step after it.
    chosen: String,
}

/// Deleting: the messages, and whether they go for the others too when no box is offered.
struct Deleting {
    chat_id: i64,
    message_ids: Vec<i64>,
    revoke: bool,
}

type Waiting = Box<dyn FnOnce(MessageProperties)>;

#[derive(Default)]
struct State {
    menu: Option<Menu>,
    /// The strip, and the chat it is in.
    bar: Option<(i64, Bar)>,
    /// Choosing messages: the chat, and the row clicked last (a shift-click goes from there). The
    /// rows chosen are the store's `selected`.
    selection: Option<(i64, i64)>,
    /// What TDLib said can be done with messages, by chat and message.
    properties: HashMap<(i64, i64), MessageProperties>,
    /// What waits for TDLib's answer about a message, asked once.
    waiting: HashMap<(i64, i64), Vec<Waiting>>,
    /// Forwarding: the chat and the messages, until a chat is picked to forward them to.
    picking: Option<(i64, Vec<i64>)>,
    picker_query: String,
    report: Option<Report>,
    deleting: Option<Deleting>,
    /// Counted up for each notice and flash, so that an older one's timer leaves a newer one be.
    notices: u64,
    flashes: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn connect(ui: &MainWindow) {
    let actions = ui.global::<Actions>();
    actions.on_ask_menu(|row, media| {
        if let Ok(row) = row.parse() {
            ask_menu(row, media.parse().ok());
        }
    });
    actions.on_choose(choose);
    actions.on_close_menu(close_menu);
    actions.on_cancel_bar(|| {
        cancel_bar();
        focus_composer();
    });
    actions.on_edit_last(edit_last);
    actions.on_toggle(|row, shift| {
        if let Ok(row) = row.parse() {
            toggle(row, shift);
        }
    });
    actions.on_forward_selected(|| {
        if let Some((chat_id, ids)) = selected_messages() {
            open_picker(chat_id, ids);
        }
    });
    actions.on_copy_selected(copy_selected);
    actions.on_delete_selected(|| {
        if let Some((chat_id, ids)) = selected_messages() {
            open_delete(chat_id, ids);
        }
    });
    actions.on_report_selected(|| {
        if let Some((chat_id, ids)) = selected_messages() {
            start_report(chat_id, ids);
        }
    });
    actions.on_cancel_selection(end_selection);
    actions.on_search_picker(|query| {
        STATE.with(|state| state.borrow_mut().picker_query = query.to_lowercase());
        fill_picker();
    });
    actions.on_pick(|id| pick(&id));
    actions.on_pick_first(pick_first);
    actions.on_close_picker(close_picker);
    actions.on_report_choose(|index| {
        if let Ok(index) = usize::try_from(index) {
            report_choose(index);
        }
    });
    actions.on_report_back(report_back);
    actions.on_report_submit(|text| report_submit(&text));
    actions.on_close_report(close_report);
    actions.on_confirm_delete(confirm_delete);
    actions.on_close_delete(close_delete);
    actions.on_jump(|id| {
        if let Ok(id) = id.parse() {
            jump(id);
        }
    });
    actions.on_escape(escape);
}

fn with_actions(change: impl FnOnce(&Actions)) {
    with_ui(|ui| change(&ui.global::<Actions>()));
}

// ---- right clicks ------------------------------------------------------------------------------

/// The window's own events, before the pages see them: a right click is counted in
/// Actions.right-clicks, and the message under the pointer asks for its menu.
pub fn window_event(event: &WindowEvent) {
    if let WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } = event {
        with_actions(|actions| actions.set_right_clicks(actions.get_right_clicks() + 1));
    }
}

// ---- the menu ----------------------------------------------------------------------------------

/// A right click on a message row (on the photo or video `media`, when it was): ask TDLib what can
/// be done with it, then show the menu.
fn ask_menu(row: i64, media: Option<i64>) {
    let busy = STATE.with(|state| {
        let state = state.borrow();
        state.selection.is_some() || state.picking.is_some() || state.report.is_some() || state.deleting.is_some()
    });
    if busy {
        return;
    }
    let Some((chat_id, messages)) = store::with(|store| {
        let chat_id = store.open?;
        let (_, messages) = store.rows.iter().find(|(id, _)| *id == row)?;
        Some((chat_id, messages.clone()))
    })
    .flatten() else {
        return;
    };
    let on_media = media.filter(|id| messages.contains(id));
    let message_id = on_media.unwrap_or(row);
    // Asked again each time: what can be done changes (an edit's time runs out, rights change).
    STATE.with(|state| state.borrow_mut().properties.remove(&(chat_id, message_id)));
    properties(chat_id, message_id, move |properties| {
        let still_open = store::with(|store| store.open == Some(chat_id)).unwrap_or(false);
        if !still_open {
            return;
        }
        let Some((items, head)) = store::with(|store| {
            let chat = store.chats.get(&chat_id)?;
            let history = store.histories.get(&chat_id)?;
            let message = history.messages.get(&message_id)?;
            let words = messages.iter().filter_map(|id| history.messages.get(id)).any(|message| has_words(&message.content));
            let media = messages.len() > 1 || store::picture(&message.content).is_some();
            let items = menu_items(&message.content, &properties, on_media.is_some(), store.can_write(chat), words, media);
            let head = with_names(|names| {
                let me = store.user_name(store.my_id, names);
                let sender = if message.is_outgoing && !me.is_empty() { me } else { store.sender_name(&message.sender_id, names) };
                format!("{sender} · {}", time::clock(message.date))
            })?;
            Some((items, head))
        })
        .flatten() else {
            return;
        };
        STATE.with(|state| state.borrow_mut().menu = Some(Menu { chat_id, row, message_id, messages }));
        with_actions(|actions| {
            actions.set_menu_items(ModelRc::new(VecModel::from(items)));
            actions.set_menu_head(head.into());
            actions.set_menu_row(row.to_string().into());
            actions.set_menu_open(true);
        });
    });
}

/// The menu of a message, in the design's order. `on_media`: the click was on a photo or video;
/// `words`: the message (an album: one of its messages) has words; `media`: it has photos or videos,
/// so copying its words says "Copy Text".
fn menu_items(content: &MessageContent, properties: &MessageProperties, on_media: bool, can_write: bool, words: bool, media: bool) -> Vec<MessageAction> {
    let mut items = Vec::new();
    if properties.can_be_replied && can_write {
        items.push(MessageAction::Reply);
    }
    if properties.can_be_edited && has_words_to_edit(content) {
        items.push(MessageAction::Edit);
    }
    if on_media && properties.can_be_saved {
        match content {
            MessageContent::Photo { .. } => items.extend([MessageAction::CopyImage, MessageAction::SaveImage]),
            MessageContent::Video { .. } | MessageContent::Animation { .. } => items.push(MessageAction::SaveVideo),
            _ => {}
        }
    }
    if properties.can_be_saved && words {
        items.push(if media { MessageAction::CopyText } else { MessageAction::Copy });
    }
    if properties.can_get_link {
        items.push(MessageAction::CopyLink);
    }
    if properties.can_be_forwarded {
        items.push(MessageAction::Forward);
    }
    if properties.can_report_chat {
        items.push(MessageAction::Report);
    }
    if properties.can_be_deleted_only_for_self || properties.can_be_deleted_for_all_users {
        items.push(MessageAction::Delete);
    }
    items.push(MessageAction::Select);
    items
}

/// A message with words to copy: a text, or a caption that is not empty.
fn has_words(content: &MessageContent) -> bool {
    store::formatted(content).is_some_and(|text| !text.text.trim().is_empty())
}

/// A message whose words can be edited here: a text, or anything with a caption.
fn has_words_to_edit(content: &MessageContent) -> bool {
    store::formatted(content).is_some()
}

fn choose(action: MessageAction) {
    let Some(menu) = STATE.with(|state| state.borrow_mut().menu.take()) else { return };
    hide_menu();
    let Menu { chat_id, row, message_id, messages } = menu;
    match action {
        MessageAction::Reply => start_reply(chat_id, message_id),
        MessageAction::Edit => start_edit(chat_id, message_id),
        MessageAction::CopyImage => copy_image(chat_id, message_id),
        MessageAction::SaveImage | MessageAction::SaveVideo => save(chat_id, message_id),
        MessageAction::Copy | MessageAction::CopyText => copy_words(chat_id, &messages),
        MessageAction::CopyLink => copy_link(chat_id, message_id, messages.len() > 1),
        MessageAction::Forward => open_picker(chat_id, messages),
        MessageAction::Report => start_report(chat_id, messages),
        MessageAction::Delete => open_delete(chat_id, messages),
        MessageAction::Select => start_selection(chat_id, row),
    }
}

fn close_menu() {
    STATE.with(|state| state.borrow_mut().menu = None);
    hide_menu();
    focus_composer();
}

fn hide_menu() {
    with_actions(|actions| {
        actions.set_menu_open(false);
        actions.set_menu_row(SharedString::new());
    });
}

/// What can be done with a message, from what TDLib said before or asked now, for `then`.
pub(super) fn properties(chat_id: i64, message_id: i64, then: impl FnOnce(MessageProperties) + 'static) {
    let key = (chat_id, message_id);
    let known = STATE.with(|state| state.borrow().properties.get(&key).copied());
    if let Some(known) = known {
        then(known);
        return;
    }
    let asked = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let waiting = state.waiting.entry(key).or_default();
        waiting.push(Box::new(then));
        waiting.len() > 1
    });
    if asked {
        return;
    }
    send(json!({ "@type": "getMessageProperties", "chat_id": chat_id, "message_id": message_id }), move |answer| {
        let waiting = STATE.with(|state| state.borrow_mut().waiting.remove(&key)).unwrap_or_default();
        match answer.map(serde_json::from_value::<MessageProperties>) {
            Ok(Ok(properties)) => {
                STATE.with(|state| state.borrow_mut().properties.insert(key, properties));
                for then in waiting {
                    then(properties);
                }
            }
            Ok(Err(err)) => eprintln!("telegram: cannot read a message's properties: {err}"),
            Err(err) => log_error("ask what can be done with a message", Err(err)),
        }
    });
}

// ---- the strip above the composer --------------------------------------------------------------

fn start_reply(chat_id: i64, message_id: i64) {
    let draft = ending_edit_draft();
    set_bar(chat_id, Bar::Reply { message_id });
    if let Some(draft) = draft {
        set_draft(&draft);
    }
    focus_composer();
}

/// Edit one of our messages: its words go into the composer, and come back out when editing ends.
/// Its formatting (bold, links, …) is written out as Markdown by TDLib (getMarkdownText) and read
/// back when the edit is saved, so that editing keeps it.
fn start_edit(chat_id: i64, message_id: i64) {
    send(json!({ "@type": "getMessage", "chat_id": chat_id, "message_id": message_id }), move |answer| {
        let message = match answer {
            Ok(message) => message,
            Err(err) => return log_error("read a message to edit", Err(err)),
        };
        let content = &message["content"];
        let words = if content["@type"] == "messageText" { content["text"].clone() } else { content["caption"].clone() };
        let plain = words["text"].as_str().unwrap_or_default().to_string();
        let formatted = words["entities"].as_array().is_some_and(|entities| !entities.is_empty());
        if !formatted {
            begin_edit(chat_id, message_id, plain, false);
            return;
        }
        send(json!({ "@type": "getMarkdownText", "text": words }), move |answer| match answer {
            Ok(markdown) => {
                let text = markdown["text"].as_str().unwrap_or_default().to_string();
                // Formatting Markdown cannot write (links found in the words) changes nothing.
                let changed = !text.is_empty() && text != plain;
                begin_edit(chat_id, message_id, if changed { text } else { plain }, changed);
            }
            Err(err) => {
                log_error("write a message's formatting as Markdown", Err(err));
                begin_edit(chat_id, message_id, plain, false);
            }
        });
    });
}

fn begin_edit(chat_id: i64, message_id: i64, words: String, markdown: bool) {
    if store::with(|store| store.open) != Some(Some(chat_id)) {
        return;
    }
    let draft = ending_edit_draft().unwrap_or_else(draft);
    set_bar(chat_id, Bar::Edit { message_id, words: words.clone(), markdown, draft });
    set_draft(&words);
    focus_composer();
}

/// ↑ in an empty composer: edit our newest message with words, when it can be.
fn edit_last() {
    if STATE.with(|state| state.borrow().bar.is_some()) {
        return;
    }
    let Some((chat_id, message_id)) = store::with(|store| {
        let chat_id = store.open?;
        let messages = &store.histories.get(&chat_id)?.messages;
        let message = messages
            .values()
            .rev()
            .find(|message| message.is_outgoing && message.sending_state.is_none() && has_words_to_edit(&message.content))?;
        Some((chat_id, message.id))
    })
    .flatten() else {
        return;
    };
    STATE.with(|state| state.borrow_mut().properties.remove(&(chat_id, message_id)));
    properties(chat_id, message_id, move |properties| {
        let open = store::with(|store| store.open == Some(chat_id)).unwrap_or(false);
        if properties.can_be_edited && open && STATE.with(|state| state.borrow().bar.is_none()) {
            start_edit(chat_id, message_id);
        }
    });
}

/// Whether a message of `chat_id` is being edited: writing then is no typing.
pub fn editing(chat_id: i64) -> bool {
    STATE.with(|state| matches!(state.borrow().bar, Some((bar_chat, Bar::Edit { .. })) if bar_chat == chat_id))
}

/// When an edit is under way, it ends: what was being written before it.
fn ending_edit_draft() -> Option<String> {
    STATE.with(|state| match state.borrow().bar.as_ref() {
        Some((_, Bar::Edit { draft, .. })) => Some(draft.clone()),
        _ => None,
    })
}

fn set_bar(chat_id: i64, bar: Bar) {
    if let Some(view) = bar_view(chat_id, &bar) {
        show_bar(chat_id, bar, view);
    }
}

/// What the strip shows for `bar` in `chat_id`: what it is, the message (the first of those
/// forwarded) as a reply quotes it, and how many messages.
fn bar_view(chat_id: i64, bar: &Bar) -> Option<(ComposeBar, crate::ReplyQuote, i32)> {
    with_names(|names| {
        store::with(|store| {
            let history = store.histories.get(&chat_id);
            let me = store.user_name(store.my_id, names);
            match bar {
                Bar::Reply { message_id } | Bar::Edit { message_id, .. } => {
                    let message = history?.messages.get(message_id)?;
                    let quote = store.quote(message, &me, names);
                    let kind = if matches!(bar, Bar::Reply { .. }) { ComposeBar::Reply } else { ComposeBar::Edit };
                    Some((kind, quote, 1))
                }
                Bar::Forward { from_chat_id, message_ids } => {
                    let history = store.histories.get(from_chat_id)?;
                    let messages: Vec<&api::Message> = message_ids.iter().filter_map(|id| history.messages.get(id)).collect();
                    let mut senders: Vec<String> = Vec::new();
                    for message in &messages {
                        let name = match &message.forward_info {
                            Some(info) => store.origin_name(&info.origin, names),
                            None if message.is_outgoing && !me.is_empty() => me.clone(),
                            None => store.sender_name(&message.sender_id, names),
                        };
                        if !senders.contains(&name) {
                            senders.push(name);
                        }
                    }
                    let first = store.quote(messages.first()?, &me, names);
                    let quote = crate::ReplyQuote { sender: senders.join(&names.list_separator).into(), ..first };
                    Some((ComposeBar::Forward, quote, message_ids.len() as i32))
                }
            }
        })
        .flatten()
    })
    .flatten()
}

fn show_bar(chat_id: i64, bar: Bar, (kind, quote, count): (ComposeBar, crate::ReplyQuote, i32)) {
    STATE.with(|state| state.borrow_mut().bar = Some((chat_id, bar)));
    with_actions(|actions| {
        actions.set_bar_name(quote.sender);
        actions.set_bar_count(count);
        actions.set_bar_content(quote.content);
        actions.set_bar_text(quote.text);
        actions.set_bar_detail(quote.detail);
        actions.set_bar_picture(quote.picture);
        actions.set_bar_has_picture(quote.has_picture);
        if kind == ComposeBar::Forward {
            actions.set_hide_sender(false);
        }
        actions.set_bar(kind);
    });
}

/// Drop the strip; an edit gives back what was being written before it.
fn cancel_bar() {
    let bar = STATE.with(|state| state.borrow_mut().bar.take());
    if let Some((_, Bar::Edit { draft, .. })) = bar {
        set_draft(&draft);
    }
    with_actions(|actions| actions.set_bar(ComposeBar::None));
}

/// Send what was written in `chat_id` the strip's way: a reply, the edit, or the messages to
/// forward after the words. False: there is no strip, and the words are sent as they are.
/// The message a reply is being written to in `chat_id`, taken: what is sent now answers it, and
/// the strip goes. None when the strip is for something else, or there is none.
pub fn take_reply(chat_id: i64) -> Option<i64> {
    let message_id = STATE.with(|state| {
        let mut state = state.borrow_mut();
        match state.bar.take() {
            Some((bar_chat, Bar::Reply { message_id })) if bar_chat == chat_id => Some(message_id),
            other => {
                state.bar = other;
                None
            }
        }
    })?;
    with_actions(|actions| actions.set_bar(ComposeBar::None));
    Some(message_id)
}

pub fn send_with_bar(chat_id: i64, text: &str) -> bool {
    let bar = STATE.with(|state| {
        let mut state = state.borrow_mut();
        match state.bar.take() {
            Some((bar_chat, bar)) if bar_chat == chat_id => Some(bar),
            other => {
                state.bar = other;
                None
            }
        }
    });
    let Some(bar) = bar else { return false };
    let mut hide_sender = false;
    with_actions(|actions| {
        hide_sender = actions.get_hide_sender();
        actions.set_bar(ComposeBar::None);
    });
    match bar {
        Bar::Reply { message_id } => {
            if text.is_empty() {
                STATE.with(|state| state.borrow_mut().bar = Some((chat_id, Bar::Reply { message_id })));
                with_actions(|actions| actions.set_bar(ComposeBar::Reply));
            } else {
                conversation::send_text(chat_id, text, Some(message_id));
            }
        }
        Bar::Edit { message_id, words, markdown, draft } => {
            if text != words.trim() {
                save_edit(chat_id, message_id, text, markdown);
            }
            set_draft(&draft);
        }
        Bar::Forward { from_chat_id, mut message_ids } => {
            if !text.is_empty() {
                conversation::send_text(chat_id, text, None);
            }
            // TDLib wants them oldest first.
            message_ids.sort_unstable();
            message_ids.dedup();
            let request = json!({
                "@type": "forwardMessages", "chat_id": chat_id, "topic_id": null, "from_chat_id": from_chat_id,
                "message_ids": message_ids, "options": null, "send_copy": hide_sender, "remove_caption": false,
            });
            send(request, |answer| log_error("forward messages", answer));
        }
    }
    true
}

/// Save an edit: new words for a text, or for a caption; with its Markdown read back into
/// formatting first (parseMarkdown) when the composer was given the message's formatting so.
fn save_edit(chat_id: i64, message_id: i64, text: &str, markdown: bool) {
    let Some(caption) = store::with(|store| {
        let message = store.histories.get(&chat_id)?.messages.get(&message_id)?;
        Some(!matches!(message.content, MessageContent::Text { .. }))
    })
    .flatten() else {
        return;
    };
    // Only a text needs words; a caption may become empty.
    if text.is_empty() && !caption {
        return;
    }
    let words = json!({ "@type": "formattedText", "text": text, "entities": [] });
    if markdown {
        send(json!({ "@type": "parseMarkdown", "text": words }), move |answer| match answer {
            Ok(formatted) => send(edit_request(chat_id, message_id, caption, formatted), |answer| log_error("edit a message", answer)),
            Err(err) => log_error("read Markdown back into formatting", Err(err)),
        });
    } else {
        send(edit_request(chat_id, message_id, caption, words), |answer| log_error("edit a message", answer));
    }
}

/// The request that gives a message new words (`words`, a formattedText): a text's, or a caption.
fn edit_request(chat_id: i64, message_id: i64, caption: bool, words: serde_json::Value) -> serde_json::Value {
    if caption {
        json!({
            "@type": "editMessageCaption", "chat_id": chat_id, "message_id": message_id, "reply_markup": null,
            "caption": words, "show_caption_above_media": false,
        })
    } else {
        json!({
            "@type": "editMessageText", "chat_id": chat_id, "message_id": message_id, "reply_markup": null,
            "input_message_content": { "@type": "inputMessageText", "text": words, "link_preview_options": null, "clear_draft": false },
        })
    }
}

fn draft() -> String {
    let mut draft = String::new();
    with_ui(|ui| draft = ui.global::<Conversation>().get_draft().into());
    draft
}

fn set_draft(text: &str) {
    with_ui(|ui| ui.global::<Conversation>().set_draft(text.into()));
}

pub(super) fn focus_composer() {
    with_actions(|actions| actions.set_focus_requests(actions.get_focus_requests() + 1));
}

// ---- copying and saving ------------------------------------------------------------------------

/// A message's words, whole: a text, or the caption of an album (whichever message has it).
fn copy_words(chat_id: i64, messages: &[i64]) {
    let words = store::with(|store| {
        let history = store.histories.get(&chat_id)?;
        messages
            .iter()
            .filter_map(|id| history.messages.get(id))
            .filter_map(|message| store::formatted(&message.content))
            .map(|text| text.text.clone())
            .find(|text| !text.trim().is_empty())
    })
    .flatten();
    if let Some(words) = words {
        copy(&words, ActionNotice::Copied);
    }
}

fn copy(text: &str, notice: ActionNotice) {
    match crate::platform::copy_text(text) {
        Ok(()) => show_notice(notice),
        Err(err) => eprintln!("actions: cannot copy: {err}"),
    }
}

/// The message's link (t.me/…): an album's, for the whole of it.
fn copy_link(chat_id: i64, message_id: i64, album: bool) {
    let request = json!({
        "@type": "getMessageLink", "chat_id": chat_id, "message_id": message_id, "media_timestamp": 0,
        "checklist_task_id": 0, "poll_option_id": "", "for_album": album, "in_message_thread": false,
    });
    send(request, |answer| match answer.map(serde_json::from_value::<api::MessageLink>) {
        Ok(Ok(link)) => copy(&link.link, ActionNotice::LinkCopied),
        Ok(Err(err)) => eprintln!("telegram: cannot read a message's link: {err}"),
        Err(err) => log_error("get a message's link", Err(err)),
    });
}

/// A photo, in its largest size, onto the clipboard.
fn copy_image(chat_id: i64, message_id: i64) {
    let Some(file) = original_file(chat_id, message_id) else { return };
    files::download(&file, ASKED, |path| {
        let copied = std::fs::read(&path).map_err(|err| err.to_string()).and_then(|contents| crate::platform::copy_image(&contents));
        match copied {
            Ok(()) => show_notice(ActionNotice::ImageCopied),
            Err(err) => eprintln!("actions: cannot copy a picture: {err}"),
        }
    });
}

/// A photo or a video, whole, into the Downloads folder, as the media viewer saves it.
fn save(chat_id: i64, message_id: i64) {
    let Some((file, name)) = store::with(|store| {
        let message = store.histories.get(&chat_id)?.messages.get(&message_id)?;
        Some((store::original(&message.content)?.clone(), viewer::download_name(&message.content, message.date)))
    })
    .flatten() else {
        return;
    };
    files::download(&file, ASKED, move |path| match viewer::save_to_downloads(Path::new(&path), &name) {
        Ok(_) => show_notice(ActionNotice::Saved),
        Err(err) => eprintln!("actions: cannot save {name} to Downloads: {err}"),
    });
}

fn original_file(chat_id: i64, message_id: i64) -> Option<api::File> {
    store::with(|store| store::original(&store.histories.get(&chat_id)?.messages.get(&message_id)?.content).cloned()).flatten()
}

// ---- choosing messages ---------------------------------------------------------------------------

fn start_selection(chat_id: i64, row: i64) {
    STATE.with(|state| state.borrow_mut().selection = Some((chat_id, row)));
    store::with(|store| {
        store.selected.clear();
        store.selected.insert(row);
        store.dirty.conversation = true;
    });
    store::refresh();
    selection_changed();
}

/// A click on a row while choosing: it is chosen, or no longer; with shift, every row from the one
/// clicked last up to it is chosen.
fn toggle(row: i64, shift: bool) {
    let Some((chat_id, anchor)) = STATE.with(|state| state.borrow().selection) else { return };
    let empty = store::with(|store| {
        let rows: Vec<i64> = store.rows.iter().map(|(id, _)| *id).collect();
        let (from, to) = (rows.iter().position(|id| *id == anchor), rows.iter().position(|id| *id == row));
        if let (true, Some(from), Some(to)) = (shift, from, to) {
            store.selected.extend(&rows[from.min(to)..=from.max(to)]);
        } else if to.is_some() && !store.selected.remove(&row) {
            store.selected.insert(row);
        }
        store.dirty.conversation = true;
        store.selected.is_empty()
    })
    .unwrap_or(true);
    if empty {
        end_selection();
        return;
    }
    STATE.with(|state| state.borrow_mut().selection = Some((chat_id, row)));
    store::refresh();
    selection_changed();
}

fn end_selection() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.selection = None;
        // What TDLib said of the messages chosen is asked again next time.
        state.properties.clear();
    });
    store::with(|store| {
        if !store.selected.is_empty() {
            store.selected.clear();
            store.dirty.conversation = true;
        }
    });
    store::refresh();
    with_actions(|actions| {
        actions.set_selecting(false);
        actions.set_selected_count(0);
    });
    focus_composer();
}

/// The chosen rows' messages, oldest first.
fn selected_messages() -> Option<(i64, Vec<i64>)> {
    let (chat_id, _) = STATE.with(|state| state.borrow().selection)?;
    let ids = store::with(|store| {
        store.rows.iter().filter(|(row, _)| store.selected.contains(row)).flat_map(|(_, ids)| ids.iter().copied()).collect::<Vec<_>>()
    })?;
    (!ids.is_empty()).then_some((chat_id, ids))
}

/// Say how many messages are chosen and what can be done with all of them, asking TDLib about any
/// it has not been asked about yet.
fn selection_changed() {
    let Some((chat_id, ids)) = selected_messages() else { return };
    let (known, missing): (Vec<Option<MessageProperties>>, Vec<i64>) = STATE.with(|state| {
        let state = state.borrow();
        let known: Vec<Option<MessageProperties>> = ids.iter().map(|id| state.properties.get(&(chat_id, *id)).copied()).collect();
        let missing = ids.iter().zip(&known).filter(|(_, known)| known.is_none()).map(|(id, _)| *id).collect();
        (known, missing)
    });
    for id in missing {
        properties(chat_id, id, |_| selection_changed());
    }
    let all = |test: fn(&MessageProperties) -> bool| known.iter().all(|properties| properties.as_ref().is_some_and(test));
    let offered = store::with(|store| {
        store.chats.get(&chat_id).is_some_and(|chat| matches!(store.kind(chat), ChatKind::Group | ChatKind::Channel))
    })
    .unwrap_or(false);
    let (forward, copy, delete, report) = (
        all(|properties| properties.can_be_forwarded),
        all(|properties| properties.can_be_saved),
        all(|properties| properties.can_be_deleted_only_for_self || properties.can_be_deleted_for_all_users),
        all(|properties| properties.can_report_chat),
    );
    with_actions(|actions| {
        actions.set_selected_count(ids.len() as i32);
        actions.set_can_forward(forward);
        actions.set_can_copy(copy);
        actions.set_can_delete(delete);
        actions.set_can_report(report);
        actions.set_report_offered(offered);
        actions.set_selecting(true);
    });
}

/// The chosen messages onto the clipboard, as Telegram's desktop app copies them: who and when,
/// then the words, a blank line between messages.
fn copy_selected() {
    let Some((chat_id, _)) = selected_messages() else { return };
    let mut text = None;
    with_ui(|ui| {
        let names = Names::from(ui);
        let fmt = ui.global::<Fmt>();
        text = store::with(|store| {
            let history = store.histories.get(&chat_id)?;
            let me = store.user_name(store.my_id, &names);
            let parts: Vec<String> = store
                .rows
                .iter()
                .filter(|(row, _)| store.selected.contains(row))
                .filter_map(|(row, ids)| {
                    let message = history.messages.get(row)?;
                    let sender = if message.is_outgoing && !me.is_empty() { me.clone() } else { store.sender_name(&message.sender_id, &names) };
                    let words = ids
                        .iter()
                        .filter_map(|id| history.messages.get(id))
                        .filter_map(|message| store::formatted(&message.content))
                        .map(|text| text.text.clone())
                        .find(|text| !text.trim().is_empty());
                    let (content, _, detail) = store.content(message, &names);
                    let words = words.unwrap_or_else(|| fmt.invoke_body(content, SharedString::new(), detail.into()).into());
                    Some(format!("{sender}, [{}]\n{words}", time::clock(message.date)))
                })
                .collect();
            Some(parts.join("\n\n"))
        })
        .flatten();
    });
    if let Some(text) = text {
        end_selection();
        copy(&text, ActionNotice::Copied);
    }
}

// ---- forwarding ----------------------------------------------------------------------------------

fn open_picker(chat_id: i64, message_ids: Vec<i64>) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.picking = Some((chat_id, message_ids));
        state.picker_query.clear();
    });
    fill_picker();
    with_actions(|actions| actions.set_picker_open(true));
}

/// The chats to forward to that match the search: Saved Messages, then the chats we can write in,
/// in the order of the chat list.
fn fill_picker() {
    let query = STATE.with(|state| state.borrow().picker_query.clone());
    with_ui(|ui| {
        let names = Names::from(ui);
        let saved = names.saved_messages.to_lowercase().contains(&query);
        let chats = store::with(|store| {
            let mut chats: Vec<(&api::Chat, i64)> = store
                .chats
                .values()
                .filter_map(|chat| store::position(chat, api::ChatList::Main).map(|position| (chat, position.order)))
                .filter(|(chat, _)| store.kind(chat) != ChatKind::Saved && store.can_write(chat))
                .filter(|(chat, _)| query.is_empty() || store.title(chat, &names).to_lowercase().contains(&query))
                .collect();
            chats.sort_by(|(a, a_order), (b, b_order)| b_order.cmp(a_order).then(b.id.cmp(&a.id)));
            chats.into_iter().take(PICKER_CHATS).map(|(chat, _)| store.chat_row(chat, api::ChatList::Main, &names)).collect::<Vec<_>>()
        })
        .unwrap_or_default();
        let actions = ui.global::<Actions>();
        actions.set_picker_saved(saved);
        actions.set_picker_chats(ModelRc::new(VecModel::from(chats)));
    });
    // The rows are built outside a refresh: their photos are downloaded here.
    store::fetch_wanted_photos();
}

/// Photos of chats arrived (avatars.rs): the picker's rows, which are not the store's models,
/// show them too.
pub fn photos_arrived() {
    if STATE.with(|state| state.borrow().picking.is_some()) {
        fill_picker();
    }
}

/// Forward to a chat: it opens with the strip that forwards there.
fn pick(id: &str) {
    let Some((from_chat_id, message_ids)) = STATE.with(|state| state.borrow_mut().picking.take()) else { return };
    with_actions(|actions| actions.set_picker_open(false));
    end_selection();
    if id == "saved" {
        conversation::with_saved_messages(move |chat_id| forward_to(chat_id, from_chat_id, message_ids));
    } else if let Ok(chat_id) = id.parse() {
        forward_to(chat_id, from_chat_id, message_ids);
    }
}

fn forward_to(chat_id: i64, from_chat_id: i64, message_ids: Vec<i64>) {
    // What the strip shows is read first: opening a chat may close the oldest tab, the messages' own.
    let bar = Bar::Forward { from_chat_id, message_ids };
    let Some(view) = bar_view(chat_id, &bar) else { return };
    conversation::open(chat_id);
    let draft = ending_edit_draft();
    show_bar(chat_id, bar, view);
    if let Some(draft) = draft {
        set_draft(&draft);
    }
    focus_composer();
}

/// ⏎ in the search: the first chat shown.
fn pick_first() {
    let mut first = None;
    with_actions(|actions| {
        first = if actions.get_picker_saved() {
            Some("saved".to_string())
        } else {
            slint::Model::row_data(&actions.get_picker_chats(), 0).map(|chat| chat.id.to_string())
        };
    });
    if let Some(first) = first {
        pick(&first);
    }
}

fn close_picker() {
    STATE.with(|state| state.borrow_mut().picking = None);
    with_actions(|actions| {
        actions.set_picker_open(false);
        actions.set_picker_chats(ModelRc::default());
    });
    focus_composer();
}

// ---- reporting -----------------------------------------------------------------------------------

/// Report a chat (no messages), or messages of it: Telegram's steps follow in the dialog.
pub(super) fn start_report(chat_id: i64, message_ids: Vec<i64>) {
    STATE.with(|state| {
        state.borrow_mut().report = Some(Report { chat_id, message_ids, steps: Vec::new(), comment: None, chosen: String::new() });
    });
    with_actions(|actions| {
        actions.set_report_title(SharedString::new());
        actions.set_report_options(ModelRc::default());
        actions.set_report_comment(false);
        actions.set_report_first(true);
        actions.set_report_open(true);
    });
    report_request(String::new(), String::new());
}

/// Ask Telegram with the reason chosen (none at first) and the words; it answers with the next
/// step, or that the report is made.
fn report_request(option_id: String, text: String) {
    let Some((chat_id, message_ids)) = STATE.with(|state| state.borrow().report.as_ref().map(|report| (report.chat_id, report.message_ids.clone())))
    else {
        return;
    };
    with_actions(|actions| actions.set_report_busy(true));
    let request = json!({ "@type": "reportChat", "chat_id": chat_id, "option_id": option_id, "message_ids": message_ids, "text": text });
    send(request, |answer| {
        with_actions(|actions| actions.set_report_busy(false));
        let result = match answer.map(serde_json::from_value::<ReportChatResult>) {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read a report's answer: {err}");
                return close_report();
            }
            Err(err) => {
                log_error("report messages", Err(err));
                return close_report();
            }
        };
        match result {
            ReportChatResult::Ok => {
                close_report();
                end_selection();
                show_notice(ActionNotice::Reported);
            }
            ReportChatResult::OptionRequired { title, options } => {
                STATE.with(|state| {
                    if let Some(report) = state.borrow_mut().report.as_mut() {
                        report.steps.push((title, options));
                    }
                });
                show_report();
            }
            ReportChatResult::TextRequired { option_id, is_optional } => {
                STATE.with(|state| {
                    if let Some(report) = state.borrow_mut().report.as_mut() {
                        report.comment = Some((option_id, report.chosen.clone(), !is_optional));
                    }
                });
                show_report();
            }
            ReportChatResult::MessagesRequired => close_report(),
        }
    });
}

fn show_report() {
    let shown = STATE.with(|state| {
        let state = state.borrow();
        let report = state.report.as_ref()?;
        Some(match &report.comment {
            Some((_, title, required)) => (title.clone(), Vec::new(), true, *required, false),
            None => {
                let (title, options) = report.steps.last()?;
                let options: Vec<SharedString> = options.iter().map(|option| option.text.as_str().into()).collect();
                (title.clone(), options, false, false, report.steps.len() <= 1)
            }
        })
    });
    let Some((title, options, comment, required, first)) = shown else { return };
    with_actions(|actions| {
        actions.set_report_title(title.into());
        actions.set_report_options(ModelRc::new(VecModel::from(options)));
        actions.set_report_comment(comment);
        actions.set_report_comment_required(required);
        actions.set_report_first(first);
    });
}

fn report_choose(index: usize) {
    let option = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let report = state.report.as_mut()?;
        let option = report.steps.last()?.1.get(index)?.clone();
        report.chosen = option.text.clone();
        Some(option.id)
    });
    if let Some(option_id) = option {
        report_request(option_id, String::new());
    }
}

/// A step back: from the words to the reasons, from reasons to the reasons before them.
fn report_back() {
    STATE.with(|state| {
        if let Some(report) = state.borrow_mut().report.as_mut()
            && report.comment.take().is_none()
            && report.steps.len() > 1
        {
            report.steps.pop();
        }
    });
    show_report();
}

fn report_submit(text: &str) {
    let comment = STATE.with(|state| state.borrow().report.as_ref().and_then(|report| report.comment.clone()));
    let Some((option_id, _, required)) = comment else { return };
    let text: String = text.trim().chars().take(REPORT_TEXT_LIMIT).collect();
    if required && text.is_empty() {
        return;
    }
    report_request(option_id, text);
}

fn close_report() {
    STATE.with(|state| state.borrow_mut().report = None);
    with_actions(|actions| {
        actions.set_report_open(false);
        actions.set_report_busy(false);
        actions.set_report_options(ModelRc::default());
    });
    focus_composer();
}

// ---- deleting ------------------------------------------------------------------------------------

/// Ask whether to delete the messages; where they can go for the others too or only for us, a box
/// says which (ticked: for them too).
fn open_delete(chat_id: i64, message_ids: Vec<i64>) {
    let missing: Vec<i64> = STATE.with(|state| {
        let state = state.borrow();
        message_ids.iter().copied().filter(|id| !state.properties.contains_key(&(chat_id, *id))).collect()
    });
    if let Some(&first) = missing.first() {
        // Asked one at a time, then this again.
        properties(chat_id, first, move |_| open_delete(chat_id, message_ids));
        return;
    }
    let known: Vec<MessageProperties> = STATE.with(|state| {
        let state = state.borrow();
        message_ids.iter().filter_map(|id| state.properties.get(&(chat_id, *id)).copied()).collect()
    });
    let both = known.iter().all(|properties| properties.can_be_deleted_only_for_self && properties.can_be_deleted_for_all_users);
    let for_all = known.iter().all(|properties| properties.can_be_deleted_for_all_users);
    let (choice, name) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        Some(match chat.kind {
            _ if !both => (DeleteChoice::None, String::new()),
            ChatType::Private { .. } | ChatType::Secret { .. } => (DeleteChoice::ForThem, chat.title.clone()),
            _ => (DeleteChoice::ForEveryone, String::new()),
        })
    })
    .flatten()
    .unwrap_or((DeleteChoice::None, String::new()));
    let count = message_ids.len() as i32;
    STATE.with(|state| state.borrow_mut().deleting = Some(Deleting { chat_id, message_ids, revoke: for_all }));
    with_actions(|actions| {
        actions.set_delete_count(count);
        actions.set_delete_choice(choice);
        actions.set_delete_name(name.into());
        actions.set_revoke(true);
        actions.set_delete_open(true);
    });
}

fn confirm_delete() {
    let Some(deleting) = STATE.with(|state| state.borrow_mut().deleting.take()) else { return };
    let mut revoke = deleting.revoke;
    with_actions(|actions| {
        if actions.get_delete_choice() != DeleteChoice::None {
            revoke = actions.get_revoke();
        }
        actions.set_delete_open(false);
    });
    let request = json!({ "@type": "deleteMessages", "chat_id": deleting.chat_id, "message_ids": deleting.message_ids, "revoke": revoke });
    send(request, |answer| log_error("delete messages", answer));
    deleted(deleting.chat_id, &deleting.message_ids);
    end_selection();
}

fn close_delete() {
    STATE.with(|state| state.borrow_mut().deleting = None);
    with_actions(|actions| actions.set_delete_open(false));
    focus_composer();
}

/// Messages are gone: the strip, the menu and the rows chosen let go of them.
pub fn deleted(chat_id: i64, message_ids: &[i64]) {
    let (bar_gone, menu_gone) = STATE.with(|state| {
        let state = state.borrow();
        let bar_gone = match &state.bar {
            Some((bar_chat, Bar::Reply { message_id } | Bar::Edit { message_id, .. })) => *bar_chat == chat_id && message_ids.contains(message_id),
            _ => false,
        };
        let menu_gone = state.menu.as_ref().is_some_and(|menu| menu.chat_id == chat_id && menu.messages.iter().any(|id| message_ids.contains(id)));
        (bar_gone, menu_gone)
    });
    if bar_gone {
        cancel_bar();
    }
    if menu_gone {
        close_menu();
    }
    let choosing = STATE.with(|state| state.borrow().selection.is_some_and(|(selection_chat, _)| selection_chat == chat_id));
    if choosing {
        let empty = store::with(|store| {
            for id in message_ids {
                store.selected.remove(id);
            }
            store.selected.is_empty()
        })
        .unwrap_or(true);
        if empty { end_selection() } else { selection_changed() }
    }
}

// ---- moving around -------------------------------------------------------------------------------

/// Another chat is open (or none): the strip and the choosing of the one before end. A strip that
/// forwards stays when it is for the chat opened.
pub fn chat_changed(open: Option<i64>) {
    let (bar_elsewhere, choosing_elsewhere, menu) = STATE.with(|state| {
        let state = state.borrow();
        (
            state.bar.as_ref().is_some_and(|(chat_id, _)| Some(*chat_id) != open),
            state.selection.is_some_and(|(chat_id, _)| Some(chat_id) != open),
            state.menu.is_some(),
        )
    });
    if bar_elsewhere {
        cancel_bar();
    }
    if choosing_elsewhere {
        end_selection();
    }
    if menu {
        STATE.with(|state| state.borrow_mut().menu = None);
        hide_menu();
    }
}

/// A click on a reply's quote: to the message it answers, lit up for a moment. One older than the
/// messages loaded is looked for page by page.
fn jump(message_id: i64) {
    jump_looking(message_id, JUMP_PAGES);
}

fn jump_looking(message_id: i64, pages_left: u32) {
    enum Found {
        Row(i64),
        /// Here, but above the rows shown.
        Hidden(i64),
        Older(i64),
        Nowhere,
    }
    let look = |store: &mut store::Store| {
        let Some(chat_id) = store.open else { return Found::Nowhere };
        if let Some((row, _)) = store.rows.iter().find(|(_, ids)| ids.contains(&message_id)) {
            return Found::Row(*row);
        }
        let Some(history) = store.histories.get(&chat_id) else { return Found::Nowhere };
        if history.messages.contains_key(&message_id) {
            return Found::Hidden(chat_id);
        }
        let older = history.has_older && history.messages.keys().next().is_some_and(|oldest| *oldest > message_id);
        if older { Found::Older(chat_id) } else { Found::Nowhere }
    };
    match store::with(look).unwrap_or(Found::Nowhere) {
        Found::Row(row) => reveal(row),
        Found::Hidden(chat_id) => {
            conversation::show_from(chat_id, message_id);
            if let Some(Found::Row(row)) = store::with(look) {
                reveal(row);
            }
        }
        Found::Older(chat_id) if pages_left > 0 => {
            conversation::load_older_then(chat_id, move || jump_looking(message_id, pages_left - 1));
        }
        Found::Older(_) | Found::Nowhere => {}
    }
}

/// Scroll to a row and light it up. The rows are given a moment to be laid out first: older
/// messages may have just been loaded above.
fn reveal(row: i64) {
    let flashes = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.flashes += 1;
        state.flashes
    });
    slint::Timer::single_shot(Duration::from_millis(50), move || {
        with_ui(|ui| {
            ui.global::<Conversation>().set_reveal(row.to_string().into());
            ui.global::<Actions>().set_flash(row.to_string().into());
        });
        // Cleared right after, so that the same message can be jumped to again.
        slint::Timer::single_shot(Duration::from_millis(100), || {
            with_ui(|ui| ui.global::<Conversation>().set_reveal(SharedString::new()));
        });
        slint::Timer::single_shot(FLASH_SHOWN, move || {
            if STATE.with(|state| state.borrow().flashes) == flashes {
                with_actions(|actions| actions.set_flash(SharedString::new()));
            }
        });
    });
}

pub(super) fn show_notice(notice: ActionNotice) {
    let notices = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.notices += 1;
        state.notices
    });
    with_actions(|actions| {
        actions.set_saved_name(SharedString::new());
        actions.set_notice(notice);
    });
    slint::Timer::single_shot(NOTICE_SHOWN, move || {
        if STATE.with(|state| state.borrow().notices) == notices {
            with_actions(|actions| actions.set_notice(ActionNotice::None));
        }
    });
}

/// Something was saved into the Downloads folder as `name`: say so, with the name.
pub(super) fn show_saved(name: &str) {
    show_notice(ActionNotice::Saved);
    with_actions(|actions| actions.set_saved_name(name.into()));
}

/// Telegram refused something: say so, in its words.
pub(super) fn show_failure(message: &str) {
    with_actions(|actions| actions.set_failure(message.into()));
    show_notice(ActionNotice::Failed);
}

/// Esc closes the topmost of the menu, a dialog, the strip and choosing; false when none is open.
fn escape() -> bool {
    if super::attachments::escape() || super::chats::escape() {
        return true;
    }
    let (menu, deleting, report, picking, bar, selection) = STATE.with(|state| {
        let state = state.borrow();
        (state.menu.is_some(), state.deleting.is_some(), state.report.is_some(), state.picking.is_some(), state.bar.is_some(), state.selection.is_some())
    });
    if menu {
        close_menu();
    } else if deleting {
        close_delete();
    } else if report {
        close_report();
    } else if picking {
        close_picker();
    } else if bar {
        cancel_bar();
        focus_composer();
    } else if selection {
        end_selection();
    } else {
        return false;
    }
    true
}

/// `read` with the names Rust puts in rows, in the UI language; None once the window is gone.
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
    use super::*;

    fn text() -> MessageContent {
        MessageContent::Text { text: api::FormattedText { text: "hi".into(), entities: Vec::new() }, link_preview: None }
    }

    #[test]
    fn the_menu_leaves_out_what_cannot_be_done() {
        let everything = MessageProperties {
            can_be_deleted_only_for_self: true,
            can_be_deleted_for_all_users: true,
            can_be_edited: true,
            can_be_forwarded: true,
            can_be_replied: true,
            can_be_saved: true,
            can_get_link: true,
            can_report_chat: false,
        };
        use MessageAction as A;
        assert_eq!(
            menu_items(&text(), &everything, false, true, true, false),
            vec![A::Reply, A::Edit, A::Copy, A::CopyLink, A::Forward, A::Delete, A::Select]
        );
        // Someone else's message in a basic group that restricts saving content.
        let theirs = MessageProperties { can_be_replied: true, can_report_chat: true, ..MessageProperties::default() };
        assert_eq!(menu_items(&text(), &theirs, false, true, true, false), vec![A::Reply, A::Report, A::Select]);
        // A channel one only reads: no reply, whatever TDLib says.
        assert_eq!(menu_items(&text(), &theirs, false, false, true, false), vec![A::Report, A::Select]);
        // A click on a photo; its caption is copied as "Copy Text".
        let photo = MessageContent::Photo {
            photo: api::Photo { minithumbnail: None, sizes: Vec::new() },
            caption: api::FormattedText { text: "the view".into(), entities: Vec::new() },
            is_secret: false,
        };
        let saved = MessageProperties { can_be_saved: true, can_be_forwarded: true, ..MessageProperties::default() };
        assert_eq!(
            menu_items(&photo, &saved, true, false, true, true),
            vec![A::CopyImage, A::SaveImage, A::CopyText, A::Forward, A::Select]
        );
    }
}
