//! The chat's info panel (the design's chat-info round): beside the conversation in every theme,
//! squeezing it, opened with ⌘I or the header's sidebar icon and remembered across restarts
//! (chat_info_open in settings.toml). It heads the open chat with its picture, name and status,
//! then lists its facts (a user's phone, username and bio; a bot's or a group's description; a
//! public group's link), its settings (notifications, pin, the folders it is in, mark as read),
//! what is drawn for later (shared media, members, the invite link: greyed, as the design has
//! them in this version), and the actions (block, report, leave, delete), which ask as the chat
//! menu does (chats.rs). The rows are built here and drawn by the themes (ui/<theme>/
//! chat-info.slint), so that Terminal's keyboard walks the same list the pointer sees. A group's
//! or channel's Manage row opens its administration (admin.rs) in the panel's place.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use serde_json::json;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::api::{self, BasicGroupFullInfo, ChatMemberStatus, ChatMembers, ChatType, MessageSender, SupergroupFullInfo, UserFullInfo};
use super::store::{self, Names};
use super::{Error, actions, conversation, send, with_ui};
use crate::settings::Settings;
use crate::{ChatInfo, ChatKind, Chats, Conversation, InfoMember, InfoRow, InfoRowKind, InfoSection, InfoWords, MainWindow};

/// How many of a group's members the panel lists.
const MEMBERS: i32 = 5;

#[derive(Default)]
struct State {
    /// The chat the facts below are about; 0 for none.
    chat_id: i64,
    /// Counted up at each chat, so that an answer about an earlier one is let go.
    lookups: u64,
    /// A user's bio, a bot's or a group's description.
    about: String,
    /// A group's or channel's primary invite link, given to an administrator who may invite.
    invite_link: String,
    admins: i32,
    member_count: i32,
    members: Vec<api::ChatMember>,
    /// We own the group or channel or administer it: the Manage row.
    administrator: bool,
    /// The folders row shows its choices.
    folders_open: bool,
    /// Workbench's sections folded away.
    collapsed: HashSet<String>,
    /// Terminal's cursor: the row it is at.
    cursor: String,
    /// What an info row copies, by row.
    copies: HashMap<String, String>,
    /// The rows the keyboard walks, in order: their ids and letters.
    walk: Vec<(String, String)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    let info = ui.global::<ChatInfo>();
    info.set_open(settings.borrow().chat_info_open);
    info.on_toggle(move || toggle(&settings));
    info.on_run(|id| run(&id));
    info.on_collapse(|id| collapse(&id));
    info.on_move(move_cursor);
    info.on_run_cursor(run_cursor);
    info.on_key(|key| run_key(&key));
}

fn with_ui_value<R>(read: impl FnOnce(&MainWindow) -> R) -> Option<R> {
    let mut value = None;
    with_ui(|ui| value = Some(read(ui)));
    value
}

/// ⌘I, the header's icon, the panel's ×, Esc: open or shut, and remembered.
fn toggle(settings: &RefCell<Settings>) {
    let mut open = false;
    with_ui(|ui| {
        let info = ui.global::<ChatInfo>();
        open = !info.get_open();
        info.set_open(open);
        // The administration shown in the panel's place goes with it.
        if !open {
            ui.global::<Conversation>().set_settings_open(false);
        }
    });
    settings.borrow_mut().chat_info_open = open;
    settings.borrow().save();
    if open {
        chat_changed();
    }
}

/// Another chat came to the front (conversation.rs), or the panel opened: its facts are asked
/// for, and the rows are built from what is here meanwhile.
pub fn chat_changed() {
    let open = with_ui_value(|ui| ui.global::<ChatInfo>().get_open()).unwrap_or(false);
    let chat_id = store::with(|store| store.open).flatten().unwrap_or(0);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.chat_id != chat_id {
            let lookups = state.lookups + 1;
            *state = State { chat_id, lookups, ..State::default() };
        }
    });
    if open && chat_id != 0 {
        fetch(chat_id);
        refresh();
    }
}

/// The store changed (store.rs): the rows follow while the panel shows; with `facts`, a full info
/// came, and the panel asks for its facts anew.
pub fn changed(facts: bool) {
    let open = with_ui_value(|ui| ui.global::<ChatInfo>().get_open()).unwrap_or(false);
    if !open {
        return;
    }
    let chat_id = STATE.with(|state| state.borrow().chat_id);
    if facts && chat_id != 0 {
        fetch(chat_id);
    }
    refresh();
}

/// Logged out: the next account's chats are its own.
pub fn forget() {
    STATE.with(|state| *state.borrow_mut() = State::default());
}

/// Esc: the folders' choices fold first, then the administration gives the panel back, then the
/// panel shuts. False when it is not open.
pub(super) fn escape() -> bool {
    let (open, administration) =
        with_ui_value(|ui| (ui.global::<ChatInfo>().get_open(), ui.global::<Conversation>().get_settings_open())).unwrap_or((false, false));
    if !open {
        return false;
    }
    if STATE.with(|state| state.borrow().folders_open) {
        run("folders");
        return true;
    }
    if administration {
        with_ui(|ui| ui.global::<Conversation>().set_settings_open(false));
        return true;
    }
    with_ui(|ui| ui.global::<ChatInfo>().invoke_toggle());
    true
}

fn administers(status: &ChatMemberStatus) -> bool {
    matches!(status, ChatMemberStatus::Creator | ChatMemberStatus::Administrator { .. })
}

/// Ask TDLib for what the store does not keep: a user's bio, a group's description, link,
/// administrators and first members. The answers come from its cache at once, from the servers
/// when it has none.
fn fetch(chat_id: i64) {
    let Some(kind) = store::with(|store| store.chats.get(&chat_id).map(|chat| chat.kind)).flatten() else { return };
    let lookups = STATE.with(|state| state.borrow().lookups);
    match kind {
        ChatType::Private { user_id } | ChatType::Secret { user_id } => {
            send(json!({ "@type": "getUserFullInfo", "user_id": user_id }), move |answer| {
                let Ok(Ok(info)) = answer.map(serde_json::from_value::<UserFullInfo>) else { return };
                let about = info
                    .bio
                    .map(|bio| bio.text)
                    .filter(|bio| !bio.is_empty())
                    .or_else(|| info.bot_info.map(|bot| bot.description))
                    .unwrap_or_default();
                answered(lookups, |state| state.about = about);
            });
        }
        ChatType::Supergroup { supergroup_id, is_channel } => {
            let administrator =
                store::with(|store| store.supergroups.get(&supergroup_id).is_some_and(|group| administers(&group.status))).unwrap_or(false);
            STATE.with(|state| state.borrow_mut().administrator = administrator);
            send(json!({ "@type": "getSupergroupFullInfo", "supergroup_id": supergroup_id }), move |answer| {
                let Ok(Ok(info)) = answer.map(serde_json::from_value::<SupergroupFullInfo>) else { return };
                answered(lookups, |state| {
                    state.about = info.description;
                    state.invite_link = info.invite_link.map(|link| link.invite_link).unwrap_or_default();
                    state.admins = info.administrator_count;
                    state.member_count = info.member_count;
                });
            });
            if !is_channel {
                let request = json!({ "@type": "getSupergroupMembers", "supergroup_id": supergroup_id, "filter": null, "offset": 0, "limit": MEMBERS });
                send(request, move |answer| {
                    let Ok(Ok(found)) = answer.map(serde_json::from_value::<ChatMembers>) else { return };
                    answered(lookups, |state| {
                        state.members = found.members;
                        if state.member_count == 0 {
                            state.member_count = found.total_count;
                        }
                    });
                });
            }
        }
        ChatType::BasicGroup { basic_group_id } => {
            let administrator =
                store::with(|store| store.basic_groups.get(&basic_group_id).is_some_and(|group| administers(&group.status))).unwrap_or(false);
            STATE.with(|state| state.borrow_mut().administrator = administrator);
            send(json!({ "@type": "getBasicGroupFullInfo", "basic_group_id": basic_group_id }), move |answer| {
                let Ok(Ok(info)) = answer.map(serde_json::from_value::<BasicGroupFullInfo>) else { return };
                answered(lookups, |state| {
                    state.about = info.description;
                    state.invite_link = info.invite_link.map(|link| link.invite_link).unwrap_or_default();
                    state.admins = info.members.iter().filter(|member| administers(&member.status)).count() as i32;
                    state.member_count = info.members.len() as i32;
                    // The owner first, then the administrators, as Telegram's apps list them.
                    let mut members = info.members;
                    members.sort_by_key(|member| match member.status {
                        ChatMemberStatus::Creator => 0,
                        ChatMemberStatus::Administrator { .. } => 1,
                        _ => 2,
                    });
                    members.truncate(MEMBERS as usize);
                    state.members = members;
                });
            });
        }
    }
}

/// An answer came, for the chat `lookups` (let go when another chat came since).
fn answered(lookups: u64, take: impl FnOnce(&mut State)) {
    let current = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.lookups != lookups {
            return false;
        }
        take(&mut state);
        true
    });
    if current {
        refresh();
    }
}

fn row(id: &str, kind: InfoRowKind, icon: &str, label: SharedString) -> InfoRow {
    InfoRow {
        id: id.into(),
        key: SharedString::new(),
        icon: icon.into(),
        label,
        sub: SharedString::new(),
        value: SharedString::new(),
        kind,
        on: false,
        open: false,
        danger: false,
        grey: false,
        enabled: true,
        child: false,
    }
}

fn section(id: &str, title: SharedString, rows: Vec<InfoRow>, collapsed: &HashSet<String>) -> InfoSection {
    InfoSection {
        id: id.into(),
        title,
        grey: false,
        collapsed: collapsed.contains(id),
        shared: false,
        rows: ModelRc::new(VecModel::from(rows)),
        members: ModelRc::default(),
        tabs: ModelRc::default(),
    }
}

/// Build the head and the rows from the store and the facts fetched, and show them.
fn refresh() {
    with_ui(|ui| {
        let info = ui.global::<ChatInfo>();
        if !info.get_open() {
            return;
        }
        let names = Names::from(ui);
        let words = ui.global::<InfoWords>();
        let conversation = ui.global::<Conversation>();
        let (muted, pinned, header_members) = (conversation.get_muted(), conversation.get_pinned(), conversation.get_members());
        let (chat_id, folders_open, collapsed, about, invite_link, admins, member_count, members, administrator) = STATE.with(|state| {
            let state = state.borrow();
            (
                state.chat_id,
                state.folders_open,
                state.collapsed.clone(),
                state.about.clone(),
                state.invite_link.clone(),
                state.admins,
                state.member_count,
                state.members.clone(),
                state.administrator,
            )
        });
        let built = store::with(|store| {
            let chat = store.chats.get(&chat_id)?;
            let kind = store.kind(chat);
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
            let phone = user
                .map(|user| user.phone_number.as_str())
                .filter(|number| !number.is_empty())
                .map(|number| format!("+{number}"))
                .unwrap_or_default();
            let blocked = chat.block_list == Some(api::BlockList::Main);
            let (picture, has_picture) = store.chat_avatar(chat);
            let folders = store.folder_choices(chat_id);
            let listed: Vec<InfoMember> = members
                .iter()
                .filter_map(|member| {
                    let MessageSender::User { user_id } = member.member_id else { return None };
                    let user = store.users.get(&user_id)?;
                    let name = store.user_name(user_id, &names);
                    let (picture, has_picture) = store.user_avatar(user);
                    let (status, last_seen) = store::user_status(user);
                    let role = match member.status {
                        ChatMemberStatus::Creator => words.invoke_owner(),
                        ChatMemberStatus::Administrator { .. } => words.invoke_admin(),
                        _ => SharedString::new(),
                    };
                    Some(InfoMember { initial: store::initial(&name), name: name.into(), picture, has_picture, status, last_seen, role })
                })
                .collect();
            Some((kind, username, phone, blocked, chat.unread_count, picture, has_picture, folders, listed))
        })
        .flatten();
        let Some((kind, username, phone, blocked, unread, picture, has_picture, folders, listed)) = built else {
            info.set_quick(ModelRc::default());
            info.set_sections(ModelRc::default());
            return;
        };
        let saved = kind == ChatKind::Saved;
        let group = kind == ChatKind::Group;
        let channel = kind == ChatKind::Channel;
        let person = matches!(kind, ChatKind::User | ChatKind::Bot);
        let mut copies = HashMap::new();
        let mut copy_row = |id: &str, icon: &str, value: &str, label: SharedString| {
            copies.insert(id.to_string(), value.to_string());
            InfoRow { value: value.into(), ..row(id, InfoRowKind::Info, icon, label) }
        };

        // Under the name: the search in the chat; a call, which is still to come.
        let mut quick = vec![InfoRow { key: "s".into(), ..row("q-search", InfoRowKind::Action, "magnifying-glass", words.invoke_search()) }];
        if kind == ChatKind::User {
            quick.push(InfoRow { grey: true, ..row("q-call", InfoRowKind::Action, "phone", words.invoke_call()) });
        }

        let mut sections = Vec::new();
        // The facts, to copy.
        let mut facts = Vec::new();
        if !phone.is_empty() {
            facts.push(copy_row("phone", "phone", &phone, words.invoke_mobile()));
        }
        if !username.is_empty() && !group && !channel {
            facts.push(copy_row("uname", "at", &format!("@{username}"), format!("{} · t.me/{username}", words.invoke_username()).into()));
        }
        if !about.is_empty() && !saved {
            let label = if kind == ChatKind::User { words.invoke_bio() } else { words.invoke_description() };
            facts.push(copy_row(if kind == ChatKind::User { "bio" } else { "desc" }, "info", &about, label));
        }
        if !username.is_empty() && (group || channel) {
            facts.push(copy_row("link", "link", &format!("t.me/{username}"), words.invoke_link()));
        }
        if !facts.is_empty() {
            sections.push(section("info", words.invoke_info(), facts, &collapsed));
        }

        // The settings: what the chat menu offers, as switches and steps.
        let mut settings = Vec::new();
        if !saved {
            settings.push(InfoRow {
                key: "m".into(),
                on: !muted,
                sub: if muted { words.invoke_muted_forever() } else { SharedString::new() },
                ..row("notif", InfoRowKind::Toggle, if muted { "bell-slash" } else { "bell" }, words.invoke_notifications())
            });
            settings.push(InfoRow { grey: true, ..row("mutefor", InfoRowKind::Nav, "clock", words.invoke_mute_for()) });
        }
        settings.push(InfoRow { key: "p".into(), on: pinned, ..row("pin", InfoRowKind::Toggle, "push-pin", words.invoke_pin_chat()) });
        let in_folders: Vec<String> = folders.iter().filter(|choice| choice.inside).map(|choice| choice.name.to_string()).collect();
        settings.push(InfoRow {
            key: "f".into(),
            open: folders_open,
            value: if in_folders.is_empty() { words.invoke_no_folders() } else { in_folders.join(&names.list_separator).into() },
            ..row("folders", InfoRowKind::Nav, "folder-simple", words.invoke_add_to_folder())
        });
        if folders_open {
            for choice in &folders {
                settings.push(InfoRow {
                    on: choice.inside,
                    child: true,
                    ..row(&format!("fold-{}", choice.folder), InfoRowKind::Toggle, "", choice.name.clone())
                });
            }
        }
        if !saved {
            settings.push(InfoRow {
                key: "r".into(),
                enabled: unread > 0,
                value: if unread > 0 { words.invoke_unread(unread) } else { words.invoke_no_unread() },
                ..row("read", InfoRowKind::Action, "checks", words.invoke_mark_read())
            });
            settings.push(InfoRow { grey: true, value: words.invoke_off(), ..row("autodel", InfoRowKind::Nav, "timer", words.invoke_auto_delete()) });
        }
        sections.push(section("settings", words.invoke_settings(), settings, &collapsed));

        // Shared media: drawn, for later.
        sections.push(InfoSection {
            grey: true,
            shared: true,
            tabs: ModelRc::new(VecModel::from(vec![words.invoke_tab_media(), words.invoke_tab_files(), words.invoke_tab_links()])),
            ..section("shared", words.invoke_shared(), Vec::new(), &collapsed)
        });

        // A group's members: the first few, and the ways to all of them, for later.
        if group {
            let count = member_count.max(header_members);
            let title: SharedString = if count > 0 { format!("{} · {count}", words.invoke_members()).into() } else { words.invoke_members() };
            let rows = vec![
                InfoRow {
                    grey: true,
                    value: if admins > 0 { admins.to_string().into() } else { SharedString::new() },
                    ..row("admins", InfoRowKind::Nav, "shield-check", words.invoke_administrators())
                },
                InfoRow {
                    grey: true,
                    value: if count > 0 { count.to_string().into() } else { SharedString::new() },
                    ..row("allm", InfoRowKind::Nav, "users", words.invoke_all_members())
                },
            ];
            sections.push(InfoSection { grey: true, members: ModelRc::new(VecModel::from(listed)), ..section("members", title, rows, &collapsed) });
        }

        // The invite link an administrator has, and the group's or channel's administration.
        if group || channel {
            let mut rows = Vec::new();
            if !invite_link.is_empty() {
                rows.push(InfoRow { grey: true, ..copy_row("invite", "link-simple", &invite_link, words.invoke_invite_link()) });
            }
            if administrator {
                let label = if group { words.invoke_manage_group() } else { words.invoke_manage_channel() };
                rows.push(row("manage", InfoRowKind::Nav, "gear-six", label));
            }
            if !rows.is_empty() {
                sections.push(section("invite", words.invoke_invite(), rows, &collapsed));
            }
        }

        // The actions: the dangerous ones last, the least undoable at the bottom.
        let mut doings = Vec::new();
        if person {
            doings.push(if blocked {
                InfoRow { key: "b".into(), ..row("block", InfoRowKind::Action, "check-circle", words.invoke_unblock()) }
            } else {
                let label = if kind == ChatKind::Bot { words.invoke_block_bot() } else { words.invoke_block_user() };
                InfoRow { key: "b".into(), danger: true, ..row("block", InfoRowKind::Action, "prohibit", label) }
            });
        }
        if !saved {
            doings.push(InfoRow { key: "x".into(), danger: true, ..row("report", InfoRowKind::Action, "flag", words.invoke_report()) });
        }
        if !channel {
            doings.push(InfoRow { danger: true, grey: true, ..row("clear", InfoRowKind::Action, "broom", words.invoke_clear_history()) });
        }
        if group || channel {
            let label = if group { words.invoke_leave_group() } else { words.invoke_leave_channel() };
            doings.push(InfoRow { key: "l".into(), danger: true, ..row("leave", InfoRowKind::Action, "sign-out", label) });
        }
        if person {
            doings.push(InfoRow { key: "d".into(), danger: true, ..row("del", InfoRowKind::Action, "trash", words.invoke_delete_chat()) });
        }
        sections.push(section("actions", words.invoke_actions(), doings, &collapsed));

        // What the keyboard walks: every row that does something, in the order shown.
        let mut walk: Vec<(String, String)> = Vec::new();
        for item in quick.iter().cloned().chain(sections.iter().flat_map(|section| {
            let rows = section.rows.clone();
            (0..rows.row_count()).filter_map(move |index| rows.row_data(index))
        })) {
            if item.enabled && !item.grey {
                walk.push((item.id.to_string(), item.key.to_string()));
            }
        }
        let cursor = STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.copies = copies;
            if !walk.iter().any(|(id, _)| *id == state.cursor) {
                state.cursor.clear();
            }
            state.walk = walk;
            state.cursor.clone()
        });

        info.set_picture(picture);
        info.set_has_picture(has_picture);
        info.set_quick(ModelRc::new(VecModel::from(quick)));
        info.set_sections(ModelRc::new(VecModel::from(sections)));
        info.set_cursor(cursor.into());
    });
}

/// A row chosen, by the pointer or the keyboard.
fn run(id: &str) {
    let chat_id = STATE.with(|state| state.borrow().chat_id);
    if chat_id == 0 {
        return;
    }
    let chat = SharedString::from(chat_id.to_string());
    match id {
        "q-search" => with_ui(|ui| {
            let conversation = ui.global::<Conversation>();
            conversation.set_search_open(true);
            conversation.set_search_requests(conversation.get_search_requests() + 1);
        }),
        "notif" => conversation::toggle_mute(chat_id),
        "pin" => conversation::toggle_pin(chat_id),
        "folders" => {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                state.folders_open = !state.folders_open;
            });
            refresh();
        }
        "read" => mark_read(chat_id),
        "manage" => with_ui(|ui| ui.global::<Conversation>().set_settings_open(true)),
        "block" => with_ui(|ui| ui.global::<Chats>().invoke_toggle_block(chat.clone())),
        "report" => with_ui(|ui| ui.global::<Chats>().invoke_report(chat.clone())),
        "leave" => with_ui(|ui| ui.global::<Chats>().invoke_leave(chat.clone())),
        "del" => with_ui(|ui| ui.global::<Chats>().invoke_delete_chat(chat.clone())),
        _ => {
            if let Some(folder) = id.strip_prefix("fold-").and_then(|index| index.parse::<i32>().ok()) {
                with_ui(|ui| ui.global::<Chats>().invoke_toggle_in_folder(chat.clone(), folder));
            } else if let Some(text) = STATE.with(|state| state.borrow().copies.get(id).cloned()) {
                actions::copy_words_of(&text);
            }
        }
    }
}

/// Mark the chat read: its last message viewed, and the mark set by hand taken off.
fn mark_read(chat_id: i64) {
    let last = store::with(|store| store.chats.get(&chat_id).and_then(|chat| chat.last_message.as_ref().map(|message| message.id))).flatten();
    if let Some(message_id) = last {
        let request = json!({ "@type": "viewMessages", "chat_id": chat_id, "message_ids": [message_id], "source": null, "force_read": true });
        send(request, |answer| log("mark the chat read", answer));
    }
    send(json!({ "@type": "toggleChatIsMarkedAsUnread", "chat_id": chat_id, "is_marked_as_unread": false }), |answer| log("take the unread mark off", answer));
}

fn log(what: &str, answer: Result<serde_json::Value, Error>) {
    match answer {
        Ok(_) | Err(Error::Stopped) => {}
        Err(err) => eprintln!("telegram: cannot {what}: {err}"),
    }
}

/// Workbench: fold a section away, or open it again.
fn collapse(id: &str) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if !state.collapsed.remove(id) {
            state.collapsed.insert(id.to_string());
        }
    });
    refresh();
}

/// Terminal: ↑ (-1) and ↓ (+1) over the rows that do something.
fn move_cursor(by: i32) {
    let cursor = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.walk.is_empty() {
            return None;
        }
        let at = state.walk.iter().position(|(id, _)| *id == state.cursor);
        let next = match at {
            Some(index) => (index as i32 + by).clamp(0, state.walk.len() as i32 - 1) as usize,
            None if by < 0 => state.walk.len() - 1,
            None => 0,
        };
        state.cursor = state.walk[next].0.clone();
        Some(state.cursor.clone())
    });
    if let Some(cursor) = cursor {
        with_ui(|ui| ui.global::<ChatInfo>().set_cursor(cursor.into()));
    }
}

/// Terminal: ⏎ on the row the cursor is at.
fn run_cursor() {
    let cursor = STATE.with(|state| state.borrow().cursor.clone());
    if !cursor.is_empty() {
        run(&cursor);
    }
}

/// Terminal: a row's letter (s search · m notifications · p pin · f folders · r read · b block ·
/// x report · l leave · d delete).
fn run_key(key: &str) {
    let found = STATE.with(|state| state.borrow().walk.iter().find(|(_, letter)| letter == key).map(|(id, _)| id.clone()));
    if let Some(id) = found {
        run(&id);
    }
}
