//! Managing a group or channel (the design's "管理群组 / 频道" round, ChatManage): the info
//! panel's Manage row (chat_info.rs) swaps the panel for this page in the same column, drawn once
//! for the three themes with each theme's tokens (ui/chat-manage-panel.slint), as the design
//! draws it; Back or Esc gives the panel back. The page: the chat's photo (a picture the user
//! picks, cropped square), name and description, saved together; whether it is public (a
//! username, checked as it is typed) or private (its primary invite link, and further links with
//! an expiry and a limit); its members (added from the contacts, removed after a word, made
//! administrators) and its administrators, each with a page of their rights and their title; and
//! the group's own settings (what members may do, slow mode, approving new members, topics, and
//! the fifth round's protected content, history, hidden members and anti-spam) or the channel's
//! (signatures, the discussion group). What the user may not do is drawn faded, with a lock;
//! what Telegram refuses is said at the top of the section it was asked in. A basic group is
//! made a supergroup when something only a supergroup has is asked of it (a username, slow mode,
//! …), as Telegram's own apps do.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use slint::{ComponentHandle, Image, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::api::{self, AdministratorRights, BasicGroupFullInfo, ChatInviteLink, ChatInviteLinks, ChatMember, ChatMemberStatus, ChatMembers, ChatType, CheckChatUsernameResult, MessageSender};
use super::store::{self, Names, Store, initial};
use super::{Error, actions, contacts, conversation, errors, send, with_ui};
use crate::{ActionNotice, ChatManage, ManageChip, ManageChoice, ManageLink, ManagePage, ManagePerson, ManageToggle, ManageWords, MainWindow, UsernameCheck};

/// How many members the page lists, and how many links.
const MEMBERS: i32 = 200;
const LINKS: i32 = 50;
/// A username is checked this long after the last keystroke.
const CHECK_AFTER: Duration = Duration::from_millis(500);
/// An administrator's title is saved this long after the last keystroke.
const TITLE_AFTER: Duration = Duration::from_millis(900);
/// The new photo's side, in pixels: what Telegram's own apps upload.
const PHOTO_SIDE: u32 = 640;

/// The rights of a group's administrator and of a channel's, in the design's order.
const GROUP_RIGHTS: [&str; 9] = ["info", "del", "ban", "invite", "pin", "topics", "call", "anon", "promote"];
const CHANNEL_RIGHTS: [&str; 8] = ["info", "post", "edit", "del", "invite", "call", "stories", "promote"];
/// What members may do, in the design's order (the fifth round's set).
const PERMISSIONS: [&str; 8] = ["send", "media", "stickers", "links", "polls", "invite", "pin", "info"];
/// Slow mode's choices, in seconds.
const SLOW: [i32; 7] = [0, 10, 30, 60, 300, 900, 3600];
/// A new link's expiry choices (seconds; 0 for never) and limit choices (0 for none).
const EXPIRIES: [i32; 4] = [3600, 86_400, 604_800, 0];
const LIMITS: [i32; 4] = [1, 10, 100, 0];

#[derive(Default, Clone, Copy, PartialEq)]
enum Page {
    #[default]
    Main,
    Rights,
    Crop,
}

/// Where the username typed stands.
#[derive(Default, Clone, Copy, PartialEq)]
enum Check {
    #[default]
    Same,
    Ok,
    Taken,
    Checking,
    Short,
    Chars,
    Start,
    Empty,
    Invalid,
    TooMany,
}

struct LinkForm {
    name: String,
    expiry: i32,
    limit: i32,
}

/// A picture picked for the photo, shown for cropping.
struct Crop {
    path: PathBuf,
    image: Image,
    width: u32,
    height: u32,
}

#[derive(Default)]
struct State {
    /// The chat managed; 0 for none (the page is shut).
    chat_id: i64,
    /// Counted up at each opening, so that an answer about an earlier one is let go.
    lookups: u64,
    page: Page,
    /// The name, description and type as saved, and as typed.
    saved_title: String,
    saved_description: String,
    saved_public: bool,
    title: String,
    description: String,
    public: bool,
    /// The username as it is, and as typed.
    username: String,
    draft: String,
    check: Check,
    suggestions: Vec<String>,
    /// A basic group's full info (the store keeps a supergroup's).
    basic_full: Option<BasicGroupFullInfo>,
    links: Vec<ChatInviteLink>,
    revoking: bool,
    form: Option<LinkForm>,
    members: Vec<ChatMember>,
    member_count: i32,
    adding: bool,
    query: String,
    /// The contacts the add box offers (load_candidates), by user id.
    candidate_ids: Vec<i64>,
    /// The member asked about removing, by user id.
    removing: Option<i64>,
    perms_open: bool,
    discussion_open: bool,
    discussion_options: Vec<i64>,
    /// The administrator whose rights page shows, and their title as typed.
    who: Option<i64>,
    custom_title: String,
    /// What Telegram refused, by section: "profile", "type", "members", "group", "rights".
    errors: HashMap<&'static str, String>,
    saving: bool,
    crop: Option<Crop>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static CHECK_TIMER: Timer = Timer::default();
    static TITLE_TIMER: Timer = Timer::default();
}

pub fn connect(ui: &MainWindow) {
    let manage = ui.global::<ChatManage>();
    manage.on_back(back);
    manage.on_save(save);
    manage.on_dismiss_error(|section| {
        STATE.with(|state| state.borrow_mut().errors.retain(|kept, _| *kept != section.as_str()));
        refresh();
    });
    manage.on_edit_name(|text| {
        STATE.with(|state| state.borrow_mut().title = text.to_string());
        refresh();
    });
    manage.on_edit_description(|text| {
        STATE.with(|state| state.borrow_mut().description = text.to_string());
        refresh();
    });
    manage.on_change_photo(change_photo);
    manage.on_set_public(set_public);
    manage.on_edit_username(|text| edit_username(text.trim()));
    manage.on_take_suggestion(|text| {
        with_ui(|ui| ui.global::<ChatManage>().set_username(text.clone()));
        edit_username(text.trim());
    });
    manage.on_copy_primary(|| copy_link(None));
    manage.on_revoke_primary(|| {
        if !allowed("type", "invite") {
            return;
        }
        STATE.with(|state| state.borrow_mut().revoking = true);
        refresh();
    });
    manage.on_confirm_revoke(revoke_primary);
    manage.on_cancel_revoke(|| {
        STATE.with(|state| state.borrow_mut().revoking = false);
        refresh();
    });
    manage.on_new_link(|| {
        if !allowed("type", "invite") {
            return;
        }
        STATE.with(|state| state.borrow_mut().form = Some(LinkForm { name: String::new(), expiry: 86_400, limit: 0 }));
        refresh();
    });
    manage.on_edit_link_name(|text| {
        STATE.with(|state| {
            if let Some(form) = &mut state.borrow_mut().form {
                form.name = text.to_string();
            }
        });
    });
    manage.on_choose_expiry(|id| {
        STATE.with(|state| {
            if let Some(form) = &mut state.borrow_mut().form {
                form.expiry = id.parse().unwrap_or(0);
            }
        });
        refresh();
    });
    manage.on_choose_limit(|id| {
        STATE.with(|state| {
            if let Some(form) = &mut state.borrow_mut().form {
                form.limit = id.parse().unwrap_or(0);
            }
        });
        refresh();
    });
    manage.on_create_link(create_link);
    manage.on_cancel_link(|| {
        STATE.with(|state| state.borrow_mut().form = None);
        refresh();
    });
    manage.on_copy_link(|link| copy_link(Some(link.to_string())));
    manage.on_revoke_link(|link| revoke_link(link.to_string()));
    manage.on_toggle_adding(|| {
        if !allowed("members", "invite") {
            return;
        }
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.adding = !state.adding;
            state.query.clear();
        });
        load_candidates();
    });
    manage.on_edit_query(|text| {
        STATE.with(|state| state.borrow_mut().query = text.to_string());
        load_candidates();
    });
    manage.on_add_member(|id| add_member(id.parse().unwrap_or(0)));
    manage.on_promote(|id| promote(id.parse().unwrap_or(0)));
    manage.on_remove(|id| ask_remove(id.parse().unwrap_or(0)));
    manage.on_confirm_remove(remove_member);
    manage.on_cancel_remove(|| {
        STATE.with(|state| state.borrow_mut().removing = None);
        refresh();
    });
    manage.on_open_admin(|id| {
        let user_id = id.parse().unwrap_or(0);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.who = Some(user_id);
            state.custom_title = state.members.iter().find(|member| user_of(member) == user_id).map(title_of).unwrap_or_default();
            state.page = Page::Rights;
            state.errors.remove("rights");
        });
        refresh();
    });
    manage.on_toggle_perms(|| {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.perms_open = !state.perms_open;
        });
        refresh();
    });
    manage.on_toggle_permission(|id| toggle_permission(&id));
    manage.on_set_slow(|id| set_slow_mode(id.parse().unwrap_or(0)));
    manage.on_toggle_option(|id| toggle_option(&id));
    manage.on_toggle_discussion(toggle_discussion);
    manage.on_choose_discussion(|id| set_discussion(id.parse().unwrap_or(0)));
    manage.on_unlink_discussion(|| set_discussion(0));
    manage.on_edit_title(|text| edit_title(text.to_string()));
    manage.on_toggle_right(|id| toggle_right(&id));
    manage.on_dismiss_admin(dismiss_admin);
    manage.on_use_photo(use_photo);
    manage.on_cancel_crop(|| {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.crop = None;
            state.page = Page::Main;
        });
        refresh();
    });
    manage.on_pick_photo(pick_photo);
}

fn is_open() -> bool {
    STATE.with(|state| state.borrow().chat_id != 0)
}

fn chat_id() -> i64 {
    STATE.with(|state| state.borrow().chat_id)
}

fn lookups() -> u64 {
    STATE.with(|state| state.borrow().lookups)
}

/// The Manage row: the page for the open chat, in the panel's place.
pub fn open() {
    let Some(chat_id) = store::with(|store| store.open).flatten() else { return };
    let Some((title, public, username)) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let username = match chat.kind {
            ChatType::Supergroup { supergroup_id, .. } => {
                store.supergroups.get(&supergroup_id).and_then(|group| group.usernames.as_ref()).map(|names| names.editable_username.clone()).unwrap_or_default()
            }
            _ => String::new(),
        };
        Some((chat.title.clone(), !username.is_empty(), username))
    })
    .flatten() else {
        return;
    };
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let lookups = state.lookups + 1;
        *state = State {
            chat_id,
            lookups,
            saved_title: title.clone(),
            title,
            saved_public: public,
            public,
            draft: username.clone(),
            username,
            ..State::default()
        };
    });
    with_ui(|ui| {
        let manage = ui.global::<ChatManage>();
        manage.set_open(true);
        manage.set_page(ManagePage::Main);
    });
    fetch();
    refresh();
}

/// The page shuts: the info panel shows again.
pub fn close() {
    STATE.with(|state| *state.borrow_mut() = State { lookups: state.borrow().lookups, ..State::default() });
    CHECK_TIMER.with(Timer::stop);
    TITLE_TIMER.with(Timer::stop);
    with_ui(|ui| ui.global::<ChatManage>().set_open(false));
}

/// Back, or Esc: a subpage goes back to the main one, the main one gives the panel back.
fn back() {
    let main = STATE.with(|state| state.borrow().page == Page::Main);
    if main {
        close();
        return;
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.page = Page::Main;
        state.who = None;
        state.crop = None;
        state.errors.remove("rights");
    });
    refresh();
}

/// Esc while the page shows: one step back. False when it does not show.
pub(super) fn escape() -> bool {
    if !is_open() {
        return false;
    }
    back();
    true
}

/// Another chat came to the front: the page was about the one before.
pub fn chat_changed() {
    let open = store::with(|store| store.open).flatten().unwrap_or(0);
    if is_open() && chat_id() != open {
        close();
    }
}

/// Logged out, or another account in front.
pub fn forget() {
    if is_open() {
        close();
    }
}

// ---- what the page shows -------------------------------------------------------------------

/// The chat's facts the page is built from, as the store has them.
struct Facts {
    group: bool,
    supergroup_id: Option<i64>,
    basic_group_id: Option<i64>,
    description: String,
    picture: Image,
    has_picture: bool,
    status: ChatMemberStatus,
    can_set_username: bool,
    permissions: api::ChatPermissions,
    member_count: i32,
    slow_mode: i32,
    join_by_request: bool,
    is_forum: bool,
    sign_messages: bool,
    protected: bool,
    history: bool,
    hidden: bool,
    can_hide: bool,
    anti_spam: bool,
    can_anti_spam: bool,
    linked_chat_id: i64,
    linked_name: String,
    pending: i32,
    invite_link: String,
    my_id: i64,
}

fn facts(store: &Store, names: &Names, chat_id: i64, basic_full: Option<&BasicGroupFullInfo>) -> Option<Facts> {
    let chat = store.chats.get(&chat_id)?;
    let (picture, has_picture) = store.avatar(chat.photo.as_ref(), false);
    let mut facts = Facts {
        group: true,
        supergroup_id: None,
        basic_group_id: None,
        description: String::new(),
        picture,
        has_picture,
        status: ChatMemberStatus::Member,
        can_set_username: false,
        permissions: chat.permissions.clone(),
        member_count: store.members(chat),
        slow_mode: 0,
        join_by_request: false,
        is_forum: false,
        sign_messages: false,
        protected: chat.has_protected_content,
        history: true,
        hidden: false,
        can_hide: false,
        anti_spam: false,
        can_anti_spam: false,
        linked_chat_id: 0,
        linked_name: String::new(),
        pending: chat.pending_join_requests.as_ref().map_or(0, |requests| requests.total_count),
        invite_link: String::new(),
        my_id: store.my_id,
    };
    match chat.kind {
        ChatType::Supergroup { supergroup_id, is_channel } => {
            facts.group = !is_channel;
            facts.supergroup_id = Some(supergroup_id);
            if let Some(group) = store.supergroups.get(&supergroup_id) {
                facts.status = group.status.clone();
                facts.join_by_request = group.join_by_request;
                facts.is_forum = group.is_forum;
                facts.sign_messages = group.sign_messages;
                if facts.member_count == 0 {
                    facts.member_count = group.member_count;
                }
            }
            if let Some(full) = store.supergroup_full.get(&supergroup_id) {
                facts.description = full.description.clone();
                facts.can_set_username = full.can_set_username;
                facts.slow_mode = full.slow_mode_delay;
                facts.history = full.is_all_history_available;
                facts.hidden = full.has_hidden_members;
                facts.can_hide = full.can_hide_members || full.has_hidden_members;
                facts.anti_spam = full.has_aggressive_anti_spam_enabled;
                facts.can_anti_spam = full.can_toggle_aggressive_anti_spam || full.has_aggressive_anti_spam_enabled;
                facts.linked_chat_id = full.linked_chat_id;
                facts.invite_link = full.invite_link.as_ref().map(|link| link.invite_link.clone()).unwrap_or_default();
                if full.member_count > 0 {
                    facts.member_count = full.member_count;
                }
            }
            if facts.linked_chat_id != 0 {
                facts.linked_name = store.chats.get(&facts.linked_chat_id).map(|chat| store.title(chat, names)).unwrap_or_default();
            }
        }
        ChatType::BasicGroup { basic_group_id } => {
            facts.basic_group_id = Some(basic_group_id);
            if let Some(group) = store.basic_groups.get(&basic_group_id) {
                facts.status = group.status.clone();
                facts.member_count = group.member_count;
            }
            if let Some(full) = basic_full {
                facts.description = full.description.clone();
                facts.invite_link = full.invite_link.as_ref().map(|link| link.invite_link.clone()).unwrap_or_default();
                facts.member_count = full.members.len() as i32;
            }
            // A basic group's owner may make it public: it becomes a supergroup then.
            facts.can_set_username = matches!(facts.status, ChatMemberStatus::Creator);
        }
        _ => return None,
    }
    Some(facts)
}

/// Whether a status lets its holder do `right` ("info", "del", …): the owner everything.
fn may(status: &ChatMemberStatus, right: &str) -> bool {
    match status {
        ChatMemberStatus::Creator => true,
        ChatMemberStatus::Administrator { rights, .. } => right_on(rights, right),
        _ => false,
    }
}

fn right_on(rights: &AdministratorRights, right: &str) -> bool {
    match right {
        "info" => rights.can_change_info,
        "del" => rights.can_delete_messages,
        "ban" => rights.can_restrict_members,
        "invite" => rights.can_invite_users,
        "pin" => rights.can_pin_messages,
        "topics" => rights.can_manage_topics,
        "call" => rights.can_manage_video_chats,
        "anon" => rights.is_anonymous,
        "promote" => rights.can_promote_members,
        "post" => rights.can_post_messages,
        "edit" => rights.can_edit_messages,
        "stories" => rights.can_post_stories && rights.can_edit_stories && rights.can_delete_stories,
        _ => false,
    }
}

fn set_right(rights: &mut AdministratorRights, right: &str, on: bool) {
    match right {
        "info" => rights.can_change_info = on,
        "del" => rights.can_delete_messages = on,
        "ban" => rights.can_restrict_members = on,
        "invite" => rights.can_invite_users = on,
        "pin" => rights.can_pin_messages = on,
        "topics" => rights.can_manage_topics = on,
        "call" => rights.can_manage_video_chats = on,
        "anon" => rights.is_anonymous = on,
        "promote" => rights.can_promote_members = on,
        "post" => rights.can_post_messages = on,
        "edit" => rights.can_edit_messages = on,
        "stories" => {
            rights.can_post_stories = on;
            rights.can_edit_stories = on;
            rights.can_delete_stories = on;
        }
        _ => {}
    }
    rights.can_manage_chat = true;
}

/// The right's name as the page says it, for the locks and the errors.
fn right_label(words: &ManageWords<'_>, group: bool, right: &str) -> SharedString {
    match (right, group) {
        ("info", true) => words.invoke_right_group_info(),
        ("info", false) => words.invoke_right_channel_info(),
        ("del", true) => words.invoke_right_delete(),
        ("del", false) => words.invoke_right_delete_others(),
        ("ban", _) => words.invoke_right_ban(),
        ("invite", true) => words.invoke_right_invite(),
        ("invite", false) => words.invoke_right_invite_subscribers(),
        ("pin", _) => words.invoke_right_pin(),
        ("topics", _) => words.invoke_right_topics(),
        ("call", true) => words.invoke_right_video_chats(),
        ("call", false) => words.invoke_right_live_streams(),
        ("anon", _) => words.invoke_right_anonymous(),
        ("promote", _) => words.invoke_right_promote(),
        ("post", _) => words.invoke_right_post(),
        ("edit", _) => words.invoke_right_edit(),
        ("stories", _) => words.invoke_right_stories(),
        _ => SharedString::new(),
    }
}

fn user_of(member: &ChatMember) -> i64 {
    match member.member_id {
        MessageSender::User { user_id } => user_id,
        MessageSender::Chat { .. } => 0,
    }
}

fn title_of(member: &ChatMember) -> String {
    match &member.status {
        ChatMemberStatus::Administrator { custom_title, .. } => custom_title.clone(),
        _ => String::new(),
    }
}

/// A number with its thousands separated, as the page counts members.
fn count(n: i32) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn now() -> i32 {
    i32::try_from(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())).unwrap_or(i32::MAX)
}

/// Build the page anew from the state and the store.
fn refresh() {
    let Some(ui) = super::window() else { return };
    let names = Names::from(&ui);
    let words = ui.global::<ManageWords>();
    let manage = ui.global::<ChatManage>();
    let chat_id = chat_id();
    if chat_id == 0 {
        manage.set_open(false);
        return;
    }
    let (facts, member_rows, candidate_rows, discussion_names) = STATE.with(|state| {
        let state = state.borrow();
        store::with(|store| {
            let facts = facts(store, &names, chat_id, state.basic_full.as_ref())?;
            let people: Vec<(i64, String, String, Image, bool)> = state
                .members
                .iter()
                .map(|member| person(store, &names, user_of(member)))
                .collect();
            let candidates: Vec<(i64, String, String, Image, bool)> = state.candidates().iter().map(|id| person(store, &names, *id)).collect();
            let discussion: Vec<(i64, String)> = state
                .discussion_options
                .iter()
                .map(|id| (*id, store.chats.get(id).map(|chat| store.title(chat, &names)).unwrap_or_else(|| format!("Chat {id}"))))
                .collect();
            Some((facts, people, candidates, discussion))
        })
        .flatten()
        .map_or((None, Vec::new(), Vec::new(), Vec::new()), |(facts, people, candidates, discussion)| (Some(facts), people, candidates, discussion))
    });
    let Some(facts) = facts else {
        close();
        return;
    };
    let group = facts.group;
    let owner = matches!(facts.status, ChatMemberStatus::Creator);
    let can = |right: &str| may(&facts.status, right);
    let right_name = |right: &str| right_label(&words, group, right);
    let needs = |right: &str| words.invoke_needs_right(right_name(right));

    STATE.with(|state| {
        let state = state.borrow();
        let error = |section: &str| SharedString::from(state.errors.get(section).cloned().unwrap_or_default());

        manage.set_open(true);
        manage.set_group(group);
        manage.set_supergroup(facts.supergroup_id.is_some());
        manage.set_page(match state.page {
            Page::Main => ManagePage::Main,
            Page::Rights => ManagePage::Rights,
            Page::Crop => ManagePage::Crop,
        });
        manage.set_title(match state.page {
            Page::Rights => words.invoke_admin_rights(),
            Page::Crop => words.invoke_change_photo(),
            Page::Main if group => words.invoke_manage_group(),
            Page::Main => words.invoke_manage_channel(),
        });

        // The profile.
        let name_error = if state.title.trim().is_empty() { words.invoke_name_empty() } else { SharedString::new() };
        manage.set_name(state.title.clone().into());
        manage.set_description(state.description.clone().into());
        manage.set_initial(initial(&state.title));
        manage.set_picture(facts.picture.clone());
        manage.set_has_picture(facts.has_picture);
        manage.set_name_error(name_error.clone());
        manage.set_profile_error(error("profile"));
        manage.set_can_info(can("info"));

        // The type.
        manage.set_public(state.public);
        manage.set_username(state.draft.clone().into());
        let (check, check_text) = match state.check {
            Check::Same => (UsernameCheck::Same, words.invoke_check_same()),
            Check::Ok => (UsernameCheck::Ok, words.invoke_check_ok(state.draft.clone().into())),
            Check::Taken => (UsernameCheck::Taken, words.invoke_check_taken(state.draft.clone().into())),
            Check::Checking => (UsernameCheck::Checking, words.invoke_check_checking(state.draft.clone().into())),
            Check::Short => (UsernameCheck::Short, words.invoke_check_short()),
            Check::Chars => (UsernameCheck::Chars, words.invoke_check_chars()),
            Check::Start => (UsernameCheck::Start, words.invoke_check_start()),
            Check::Empty => (UsernameCheck::Empty, if group { words.invoke_check_empty_group() } else { words.invoke_check_empty_channel() }),
            Check::Invalid => (UsernameCheck::Invalid, words.invoke_check_invalid()),
            Check::TooMany => (UsernameCheck::TooMany, words.invoke_check_too_many()),
        };
        manage.set_check(check);
        manage.set_check_text(check_text);
        manage.set_suggestions(chips(state.suggestions.iter().map(|text| (text.clone(), text.clone(), false)).collect()));
        manage.set_type_error(error("type"));
        manage.set_can_invite(can("invite"));

        // The links.
        let primary = state.links.iter().find(|link| link.is_primary).map(|link| link.invite_link.clone()).unwrap_or_else(|| facts.invite_link.clone());
        manage.set_primary_link(primary.into());
        manage.set_revoking(state.revoking);
        let now = now();
        let links: Vec<ManageLink> = state
            .links
            .iter()
            .filter(|link| !link.is_primary && !link.is_revoked)
            .map(|link| {
                let expiry = if link.expiration_date == 0 {
                    words.invoke_never_expires()
                } else {
                    let left = link.expiration_date - now;
                    if left <= 0 {
                        words.invoke_expired()
                    } else if left >= 86_400 {
                        words.invoke_days_left(left / 86_400)
                    } else if left >= 3600 {
                        words.invoke_hours_left(left / 3600)
                    } else {
                        words.invoke_minutes_left((left / 60).max(1))
                    }
                };
                let limit = if link.member_limit == 0 { words.invoke_no_limit() } else { words.invoke_up_to(link.member_limit) };
                let joined = words.invoke_joined(link.member_count);
                ManageLink {
                    link: link.invite_link.clone().into(),
                    name: if link.name.is_empty() { words.invoke_unnamed_link() } else { link.name.clone().into() },
                    meta: format!("{expiry} · {limit} · {joined}").into(),
                }
            })
            .collect();
        manage.set_links(ModelRc::new(VecModel::from(links)));
        manage.set_link_form(state.form.is_some());
        let (expiry, limit, link_name) = state.form.as_ref().map_or((86_400, 0, String::new()), |form| (form.expiry, form.limit, form.name.clone()));
        manage.set_link_name(link_name.into());
        manage.set_expiries(chips(
            EXPIRIES
                .iter()
                .map(|seconds| {
                    let label = match *seconds {
                        3600 => words.invoke_one_hour(),
                        86_400 => words.invoke_one_day(),
                        604_800 => words.invoke_one_week(),
                        _ => words.invoke_no_limit(),
                    };
                    (seconds.to_string(), label.to_string(), *seconds == expiry)
                })
                .collect(),
        ));
        manage.set_limits(chips(
            LIMITS
                .iter()
                .map(|n| {
                    let label = if *n == 0 { words.invoke_no_limit().to_string() } else { words.invoke_people(*n).to_string() };
                    (n.to_string(), label, *n == limit)
                })
                .collect(),
        ));

        // The members and the administrators.
        manage.set_count_line(if group { words.invoke_members_count(count(facts.member_count).into()) } else { words.invoke_subscribers_count(count(facts.member_count).into()) });
        manage.set_members_error(error("members"));
        manage.set_adding(state.adding);
        manage.set_query(state.query.clone().into());
        let owner_word = if group { words.invoke_group_owner() } else { words.invoke_channel_owner() };
        let can_promote = can("promote");
        let can_ban = can("ban");
        let mut rows: Vec<(usize, ManagePerson)> = Vec::new();
        let mut admin_rows: Vec<(usize, ManagePerson)> = Vec::new();
        for (member, (user_id, name, letter, picture, has_picture)) in state.members.iter().zip(member_rows) {
            let is_owner = matches!(member.status, ChatMemberStatus::Creator);
            let is_admin = matches!(member.status, ChatMemberStatus::Administrator { .. });
            let you = user_id == facts.my_id;
            let by_me = member.inviter_user_id == facts.my_id;
            let editable = match &member.status {
                ChatMemberStatus::Administrator { can_be_edited, .. } => *can_be_edited,
                _ => false,
            };
            // An administrator someone else made cannot be removed by one who did not (the owner
            // can).
            let removable = !you && !is_owner && can_ban && (!is_admin || owner || (editable && by_me));
            let order = if is_owner {
                0
            } else if is_admin {
                1
            } else {
                2
            };
            let person = ManagePerson {
                id: user_id.to_string().into(),
                name: name.clone().into(),
                initial: letter.clone().into(),
                picture: picture.clone(),
                has_picture,
                you,
                tag: if is_owner {
                    owner_word.clone()
                } else if is_admin {
                    words.invoke_administrator()
                } else {
                    SharedString::new()
                },
                owner: is_owner,
                admin: is_admin,
                can_promote: can_promote && !is_admin && !is_owner && !you,
                promote_lock: if can_promote { SharedString::new() } else { needs("promote") },
                can_remove: removable,
                confirming: state.removing == Some(user_id),
                sub: SharedString::new(),
            };
            if is_owner || is_admin {
                let rights_count = match &member.status {
                    ChatMemberStatus::Administrator { rights, .. } => {
                        let list: &[&str] = if group { &GROUP_RIGHTS } else { &CHANNEL_RIGHTS };
                        list.iter().filter(|right| right_on(rights, right)).count() as i32
                    }
                    _ => 0,
                };
                let by = if member.inviter_user_id == 0 {
                    String::new()
                } else {
                    store::with(|store| store.user_name(member.inviter_user_id, &names)).unwrap_or_default()
                };
                let sub = if is_owner {
                    words.invoke_owner_all_rights(owner_word.clone())
                } else if member.inviter_user_id == 0 || by.is_empty() {
                    words.invoke_rights_count(rights_count)
                } else {
                    format!("{} · {}", words.invoke_rights_count(rights_count), words.invoke_set_by(by.into())).into()
                };
                admin_rows.push((order, ManagePerson { sub, ..person.clone() }));
            }
            rows.push((order, person));
        }
        rows.sort_by_key(|(order, _)| *order);
        admin_rows.sort_by_key(|(order, _)| *order);
        manage.set_members(ModelRc::new(VecModel::from(rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>())));
        manage.set_admins(ModelRc::new(VecModel::from(admin_rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>())));
        let candidates: Vec<ManagePerson> = candidate_rows
            .into_iter()
            .map(|(user_id, name, letter, picture, has_picture)| ManagePerson {
                id: user_id.to_string().into(),
                name: name.into(),
                initial: letter.into(),
                picture,
                has_picture,
                you: false,
                tag: SharedString::new(),
                owner: false,
                admin: false,
                can_promote: false,
                promote_lock: SharedString::new(),
                can_remove: false,
                confirming: false,
                sub: SharedString::new(),
            })
            .collect();
        manage.set_candidates(ModelRc::new(VecModel::from(candidates)));

        // The group's settings, or the channel's.
        manage.set_group_error(error("group"));
        manage.set_perms_open(state.perms_open);
        let permissions = &facts.permissions;
        let perm_on = |id: &str| match id {
            "send" => permissions.can_send_basic_messages,
            "media" => permissions.can_send_photos || permissions.can_send_videos,
            "stickers" => permissions.can_send_other_messages,
            "links" => permissions.can_add_link_previews,
            "polls" => permissions.can_send_polls,
            "invite" => permissions.can_invite_users,
            "pin" => permissions.can_pin_messages,
            "info" => permissions.can_change_info,
            _ => false,
        };
        let allowed_count = PERMISSIONS.iter().filter(|id| perm_on(id)).count() as i32;
        manage.set_perms_summary(words.invoke_perms_summary(allowed_count, PERMISSIONS.len() as i32));
        let perms: Vec<ManageToggle> = PERMISSIONS
            .iter()
            .map(|id| ManageToggle {
                id: (*id).into(),
                label: match *id {
                    "send" => words.invoke_perm_send(),
                    "media" => words.invoke_perm_media(),
                    "stickers" => words.invoke_perm_stickers(),
                    "links" => words.invoke_perm_links(),
                    "polls" => words.invoke_perm_polls(),
                    "invite" => words.invoke_perm_invite(),
                    "pin" => words.invoke_perm_pin(),
                    _ => words.invoke_perm_info(),
                },
                note: SharedString::new(),
                on: perm_on(id),
                enabled: can_ban,
                lock: if can_ban { SharedString::new() } else { needs("ban") },
            })
            .collect();
        manage.set_perms(ModelRc::new(VecModel::from(perms)));
        let slow_chip = |seconds: &i32| (seconds.to_string(), slow_label(&words, *seconds).to_string(), *seconds == facts.slow_mode);
        manage.set_slow_rows(ModelRc::new(VecModel::from(vec![chips(SLOW[..4].iter().map(slow_chip).collect()), chips(SLOW[4..].iter().map(slow_chip).collect())])));
        manage.set_slow_note(if facts.slow_mode > 0 { words.invoke_slow_note_on(slow_label(&words, facts.slow_mode)) } else { words.invoke_slow_note_off() });
        let option = |id: &str, label: SharedString, note: SharedString, on: bool, right: &str, possible: bool| ManageToggle {
            id: id.into(),
            label,
            note,
            on,
            enabled: possible && can(right),
            lock: if !possible {
                words.invoke_not_available()
            } else if can(right) {
                SharedString::new()
            } else {
                needs(right)
            },
        };
        let options: Vec<ManageToggle> = if group {
            vec![
                option("approve", words.invoke_approve(), words.invoke_approve_note(), facts.join_by_request, "ban", true),
                option("topics", words.invoke_topics(), words.invoke_topics_note(), facts.is_forum, "info", true),
                option("protect", words.invoke_protect(), words.invoke_protect_note(), facts.protected, "info", true),
                option("history", words.invoke_history(), words.invoke_history_note(), facts.history, "info", true),
                option("hidden", words.invoke_hidden(), words.invoke_hidden_note(), facts.hidden, "ban", facts.can_hide || facts.supergroup_id.is_none()),
                option("antispam", words.invoke_anti_spam(), words.invoke_anti_spam_note(), facts.anti_spam, "del", facts.can_anti_spam),
            ]
        } else {
            vec![
                option("signatures", words.invoke_signatures(), words.invoke_signatures_note(), facts.sign_messages, "info", true),
                option("protect", words.invoke_protect(), words.invoke_protect_channel_note(), facts.protected, "info", true),
            ]
        };
        manage.set_options(ModelRc::new(VecModel::from(options)));
        manage.set_pending(if facts.join_by_request && facts.pending > 0 { words.invoke_pending(facts.pending) } else { SharedString::new() });
        manage.set_discussion(if facts.linked_chat_id != 0 { facts.linked_name.clone().into() } else { words.invoke_not_linked() });
        manage.set_discussion_linked(facts.linked_chat_id != 0);
        manage.set_discussion_open(state.discussion_open);
        let choices: Vec<ManageChoice> = discussion_names
            .into_iter()
            .map(|(id, name)| ManageChoice { id: id.to_string().into(), name: name.into(), on: id == facts.linked_chat_id })
            .collect();
        manage.set_discussion_options(ModelRc::new(VecModel::from(choices)));

        // The rights page.
        let who = state.who.and_then(|id| state.members.iter().find(|member| user_of(member) == id));
        if let Some(member) = who {
            let user_id = user_of(member);
            let (name, letter, picture, has_picture) = store::with(|store| person(store, &names, user_id))
                .map(|(_, name, letter, picture, has_picture)| (name, letter, picture, has_picture))
                .unwrap_or_default();
            let is_owner = matches!(member.status, ChatMemberStatus::Creator);
            let you = user_id == facts.my_id;
            let editable = !you && matches!(&member.status, ChatMemberStatus::Administrator { can_be_edited: true, .. });
            let by = store::with(|store| store.user_name(member.inviter_user_id, &names)).unwrap_or_default();
            let owner_name = state
                .members
                .iter()
                .find(|member| matches!(member.status, ChatMemberStatus::Creator))
                .map(|member| store::with(|store| store.user_name(user_of(member), &names)).unwrap_or_default())
                .unwrap_or_default();
            manage.set_who(ManagePerson {
                id: user_id.to_string().into(),
                name: if you { format!("{name} ({})", words.invoke_you()).into() } else { name.into() },
                initial: letter.into(),
                picture,
                has_picture,
                you,
                tag: SharedString::new(),
                owner: is_owner,
                admin: !is_owner,
                can_promote: false,
                promote_lock: SharedString::new(),
                can_remove: false,
                confirming: false,
                sub: if is_owner {
                    owner_word.clone()
                } else if by.is_empty() {
                    words.invoke_administrator()
                } else {
                    format!("{} · {}", words.invoke_administrator(), words.invoke_set_by(by.clone().into())).into()
                },
            });
            manage.set_lock_note(if is_owner {
                words.invoke_owner_locked()
            } else if you {
                words.invoke_own_rights_locked(by.clone().into())
            } else if editable {
                SharedString::new()
            } else {
                words.invoke_view_only(owner_name.into())
            });
            manage.set_rights_error(error("rights"));
            manage.set_custom_title(state.custom_title.clone().into());
            manage.set_title_editable(editable);
            let list: &[&str] = if group { &GROUP_RIGHTS } else { &CHANNEL_RIGHTS };
            let rights: Vec<ManageToggle> = list
                .iter()
                .map(|right| {
                    let on = is_owner || matches!(&member.status, ChatMemberStatus::Administrator { rights, .. } if right_on(rights, right));
                    let enabled = editable && (owner || can(right));
                    ManageToggle {
                        id: (*right).into(),
                        label: right_name(right),
                        note: SharedString::new(),
                        on,
                        enabled,
                        lock: if enabled || !editable { SharedString::new() } else { words.invoke_cannot_give(right_name(right)) },
                    }
                })
                .collect();
            manage.set_rights(ModelRc::new(VecModel::from(rights)));
            manage.set_can_dismiss(editable);
        }

        // The crop page.
        if let Some(crop) = &state.crop {
            manage.set_source(crop.image.clone());
        }

        // The head's Save: something changed, and nothing is wrong.
        let dirty = state.title != state.saved_title
            || state.description != state.saved_description
            || state.public != state.saved_public
            || (state.public && state.draft != state.username && state.check == Check::Ok);
        let valid = name_error.is_empty() && (!state.public || matches!(state.check, Check::Same | Check::Ok));
        manage.set_can_save(dirty && valid && !state.saving);
        manage.set_saving(state.saving);
    });
}

fn chips(items: Vec<(String, String, bool)>) -> ModelRc<ManageChip> {
    ModelRc::new(VecModel::from(items.into_iter().map(|(id, label, on)| ManageChip { id: id.into(), label: label.into(), on }).collect::<Vec<_>>()))
}

fn slow_label(words: &ManageWords<'_>, seconds: i32) -> SharedString {
    match seconds {
        0 => words.invoke_off(),
        s if s < 60 => words.invoke_seconds_short(s),
        s if s < 3600 => words.invoke_minutes_short(s / 60),
        s => words.invoke_hours_short(s / 3600),
    }
}

/// A person as a row shows them: their id, name, initial and photo.
fn person(store: &Store, names: &Names, user_id: i64) -> (i64, String, String, Image, bool) {
    let name = store.user_name(user_id, names);
    let (picture, has_picture) = store.avatar(store.users.get(&user_id).and_then(|user| user.profile_photo.as_ref()), false);
    (user_id, name.clone(), initial(&name).to_string(), picture, has_picture)
}

impl State {
    /// The contacts the add box offers: not in the chat, matching the words typed.
    fn candidates(&self) -> &[i64] {
        &self.candidate_ids
    }
}

// ---- what the page asks Telegram for ----------------------------------------------------------

/// Members, links and (for a basic group) the full info, from TDLib.
fn fetch() {
    let chat_id = chat_id();
    let lookups = lookups();
    let Some((kind, my_id)) = store::with(|store| store.chats.get(&chat_id).map(|chat| (chat.kind, store.my_id))).flatten() else { return };
    match kind {
        ChatType::Supergroup { supergroup_id, .. } => {
            send(json!({ "@type": "getSupergroupFullInfo", "supergroup_id": supergroup_id }), move |_| {});
            let request = json!({ "@type": "getSupergroupMembers", "supergroup_id": supergroup_id, "filter": null, "offset": 0, "limit": MEMBERS });
            send(request, move |answer| match answer.map(serde_json::from_value::<ChatMembers>) {
                Ok(Ok(found)) => answered(lookups, |state| {
                    state.members = found.members;
                    if state.member_count == 0 {
                        state.member_count = found.total_count;
                    }
                }),
                Ok(Err(err)) => eprintln!("manage: unreadable members: {err}"),
                Err(Error::Stopped) => {}
                Err(err) => eprintln!("manage: cannot load the members: {err}"),
            });
        }
        ChatType::BasicGroup { basic_group_id } => {
            send(json!({ "@type": "getBasicGroupFullInfo", "basic_group_id": basic_group_id }), move |answer| {
                let Ok(Ok(info)) = answer.map(serde_json::from_value::<BasicGroupFullInfo>) else { return };
                answered(lookups, |state| {
                    state.members = info.members.clone();
                    state.member_count = info.members.len() as i32;
                    state.basic_full = Some(info);
                });
            });
        }
        _ => return,
    }
    // The description is in the full info, which the store keeps for a supergroup: it comes with
    // the update, and the drafts start from it then (fill_description).
    let request = json!({
        "@type": "getChatInviteLinks", "chat_id": chat_id, "creator_user_id": my_id, "is_revoked": false,
        "offset_date": 0, "offset_invite_link": "", "limit": LINKS,
    });
    send(request, move |answer| match answer.map(serde_json::from_value::<ChatInviteLinks>) {
        Ok(Ok(found)) => answered(lookups, |state| state.links = found.invite_links),
        Ok(Err(err)) => eprintln!("manage: unreadable invite links: {err}"),
        Err(Error::Stopped) => {}
        Err(Error::Telegram { message, .. }) => eprintln!("manage: cannot load the invite links: {message}"),
    });
}

/// An answer came, for the opening `lookups` (let go when the page was opened anew since).
fn answered(lookups: u64, take: impl FnOnce(&mut State)) {
    let current = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.lookups != lookups || state.chat_id == 0 {
            return false;
        }
        take(&mut state);
        true
    });
    if current {
        fill_description();
        refresh();
    }
}

/// The description draft starts from what the chat has, once that is known.
fn fill_description() {
    let chat_id = chat_id();
    let description = STATE.with(|state| {
        let state = state.borrow();
        store::with(|store| {
            let chat = store.chats.get(&chat_id)?;
            Some(match chat.kind {
                ChatType::Supergroup { supergroup_id, .. } => store.supergroup_full.get(&supergroup_id)?.description.clone(),
                ChatType::BasicGroup { .. } => state.basic_full.as_ref()?.description.clone(),
                _ => return None,
            })
        })
        .flatten()
    });
    if let Some(description) = description {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.description == state.saved_description {
                state.description = description.clone();
            }
            state.saved_description = description;
        });
    }
}

/// A full info or a chat update came (store.rs): the description may be known now.
pub fn facts_changed() {
    if is_open() {
        fill_description();
        refresh();
    }
}

/// Whether we may do what needs `right`; else the section says so.
fn allowed(section: &'static str, right: &str) -> bool {
    let chat_id = chat_id();
    let (status, group) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let status = match chat.kind {
            ChatType::Supergroup { supergroup_id, .. } => store.supergroups.get(&supergroup_id)?.status.clone(),
            ChatType::BasicGroup { basic_group_id } => store.basic_groups.get(&basic_group_id)?.status.clone(),
            _ => return None,
        };
        Some((status, !matches!(chat.kind, ChatType::Supergroup { is_channel: true, .. })))
    })
    .flatten()
    .unwrap_or((ChatMemberStatus::Member, true));
    if may(&status, right) {
        return true;
    }
    let mut words = SharedString::new();
    with_ui(|ui| {
        let manage_words = ui.global::<ManageWords>();
        words = manage_words.invoke_needs_right(right_label(&manage_words, group, right));
    });
    set_error(section, words.to_string());
    false
}

fn set_error(section: &'static str, text: String) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.errors.insert(section, text);
        state.removing = None;
    });
    refresh();
}

/// Send `request` for the chat; a refusal is said in `section`, an answer goes to `then`.
fn ask(section: &'static str, request: Value, then: impl FnOnce(Value) + 'static) {
    let lookups = lookups();
    send(request, move |answer| {
        if lookups != self::lookups() {
            return;
        }
        match answer {
            Ok(value) => then(value),
            Err(Error::Stopped) => {}
            Err(Error::Telegram { message, .. }) => set_error(section, errors::explain(&message)),
        }
    });
}

/// The chat as a supergroup: itself, or a basic group made one first (TDLib's
/// upgradeBasicGroupChatToSupergroupChat, which Telegram's own apps do when a basic group is
/// asked for what only a supergroup has). `then` gets the chat id and the supergroup id.
fn as_supergroup(section: &'static str, then: impl FnOnce(i64, i64) + 'static) {
    let chat_id = chat_id();
    let kind = store::with(|store| store.chats.get(&chat_id).map(|chat| chat.kind)).flatten();
    match kind {
        Some(ChatType::Supergroup { supergroup_id, .. }) => then(chat_id, supergroup_id),
        Some(ChatType::BasicGroup { .. }) => {
            ask(section, json!({ "@type": "upgradeBasicGroupChatToSupergroupChat", "chat_id": chat_id }), move |value| {
                let Ok(chat) = serde_json::from_value::<api::Chat>(value) else { return };
                let ChatType::Supergroup { supergroup_id, .. } = chat.kind else { return };
                let new_id = chat.id;
                // The page and the conversation follow the group into its new chat.
                STATE.with(|state| {
                    let mut state = state.borrow_mut();
                    state.chat_id = new_id;
                    state.basic_full = None;
                });
                conversation::open(new_id);
                fetch();
                then(new_id, supergroup_id);
            });
        }
        _ => {}
    }
}

// ---- the profile ------------------------------------------------------------------------------

/// Save: the name, the description and the type, each only when it changed, one after the
/// other; the first refusal stops the rest.
fn save() {
    let (chat_id, title, description, public, draft, changed_title, changed_description, changed_type) = STATE.with(|state| {
        let state = state.borrow();
        (
            state.chat_id,
            state.title.trim().to_string(),
            state.description.trim().to_string(),
            state.public,
            state.draft.clone(),
            state.title.trim() != state.saved_title,
            state.description.trim() != state.saved_description,
            state.public != state.saved_public || (state.public && state.draft != state.username),
        )
    });
    if chat_id == 0 || title.is_empty() {
        return;
    }
    STATE.with(|state| state.borrow_mut().saving = true);
    refresh();
    let after_type = move || {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.saving = false;
            state.saved_public = state.public;
            if state.public {
                state.username = state.draft.clone();
            }
            state.check = Check::Same;
            state.suggestions.clear();
        });
        refresh();
    };
    let save_type = move || {
        if !changed_type {
            after_type();
            return;
        }
        if !public {
            as_supergroup("type", move |_, supergroup_id| {
                ask("type", json!({ "@type": "setSupergroupUsername", "supergroup_id": supergroup_id, "username": "" }), move |_| after_type());
            });
            return;
        }
        as_supergroup("type", move |_, supergroup_id| {
            ask("type", json!({ "@type": "setSupergroupUsername", "supergroup_id": supergroup_id, "username": draft }), move |_| after_type());
        });
    };
    let save_description = move || {
        if !changed_description {
            save_type();
            return;
        }
        ask("profile", json!({ "@type": "setChatDescription", "chat_id": chat_id, "description": description.clone() }), move |_| {
            STATE.with(|state| state.borrow_mut().saved_description = description);
            save_type();
        });
    };
    if changed_title {
        ask("profile", json!({ "@type": "setChatTitle", "chat_id": chat_id, "title": title.clone() }), move |_| {
            STATE.with(|state| state.borrow_mut().saved_title = title);
            save_description();
        });
    } else {
        save_description();
    }
    // A refusal on the way leaves saving on: off again at the next refresh after the error.
    let lookups = lookups();
    slint::Timer::single_shot(Duration::from_secs(20), move || {
        if lookups == self::lookups() {
            STATE.with(|state| state.borrow_mut().saving = false);
            refresh();
        }
    });
}

// ---- the type: the username, the links ----------------------------------------------------------

fn set_public(public: bool) {
    if !allowed("type", "info") {
        return;
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.public = public;
        if public && state.draft.is_empty() {
            state.check = Check::Empty;
        }
    });
    refresh();
}

/// The username typed: checked for its form at once, with Telegram after a pause.
fn edit_username(text: &str) {
    let text = text.trim_start_matches('@').to_string();
    let same = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.draft = text.clone();
        state.suggestions.clear();
        if text == state.username {
            state.check = Check::Same;
            return true;
        }
        state.check = if text.is_empty() {
            Check::Empty
        } else if !text.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            Check::Start
        } else if !text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            Check::Chars
        } else if text.chars().count() < 5 {
            Check::Short
        } else {
            Check::Checking
        };
        state.check == Check::Same
    });
    CHECK_TIMER.with(Timer::stop);
    refresh();
    if same || STATE.with(|state| state.borrow().check != Check::Checking) {
        return;
    }
    let slot = super::context();
    CHECK_TIMER.with(|timer| {
        timer.start(TimerMode::SingleShot, CHECK_AFTER, move || super::in_context(slot, || check_username(text.clone(), true)));
    });
}

/// Ask Telegram whether `username` is free: for the draft (`for_draft`), or for a suggestion.
fn check_username(username: String, for_draft: bool) {
    let chat_id = chat_id();
    let lookups = lookups();
    // A basic group is not yet a chat a username can be checked for: as one being made.
    let check_in = store::with(|store| match store.chats.get(&chat_id).map(|chat| chat.kind) {
        Some(ChatType::Supergroup { .. }) => chat_id,
        _ => 0,
    })
    .unwrap_or(0);
    send(json!({ "@type": "checkChatUsername", "chat_id": check_in, "username": username.clone() }), move |answer| {
        if lookups != self::lookups() {
            return;
        }
        let result = match answer {
            Ok(value) => serde_json::from_value::<CheckChatUsernameResult>(value).unwrap_or(CheckChatUsernameResult::UsernameInvalid),
            Err(Error::Stopped) => return,
            Err(Error::Telegram { message, .. }) => {
                let (code, _) = errors::split(&message);
                match code {
                    "USERNAME_OCCUPIED" => CheckChatUsernameResult::UsernameOccupied,
                    "CHANNELS_ADMIN_PUBLIC_TOO_MUCH" => CheckChatUsernameResult::PublicChatsTooMany,
                    _ => CheckChatUsernameResult::UsernameInvalid,
                }
            }
        };
        if for_draft {
            let still = STATE.with(|state| state.borrow().draft == username);
            if !still {
                return;
            }
            let check = match result {
                CheckChatUsernameResult::Ok => Check::Ok,
                CheckChatUsernameResult::UsernameOccupied | CheckChatUsernameResult::UsernamePurchasable => Check::Taken,
                CheckChatUsernameResult::PublicChatsTooMany | CheckChatUsernameResult::PublicGroupsUnavailable => Check::TooMany,
                CheckChatUsernameResult::UsernameInvalid => Check::Invalid,
            };
            STATE.with(|state| state.borrow_mut().check = check);
            refresh();
            if check == Check::Taken {
                for suggestion in [format!("{username}_club"), format!("{username}_chat"), format!("{username}2026")] {
                    check_username(suggestion, false);
                }
            }
        } else if result == CheckChatUsernameResult::Ok {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                if state.check == Check::Taken && !state.suggestions.contains(&username) {
                    state.suggestions.push(username);
                }
            });
            refresh();
        }
    });
}

/// Copy a link (the primary one, or `link`) and say so.
fn copy_link(link: Option<String>) {
    let link = link.unwrap_or_else(|| {
        let mut primary = String::new();
        with_ui(|ui| primary = ui.global::<ChatManage>().get_primary_link().to_string());
        primary
    });
    if link.is_empty() {
        return;
    }
    if let Err(err) = crate::platform::copy_text(&link) {
        eprintln!("manage: cannot copy the link: {err}");
        return;
    }
    actions::show_notice(ActionNotice::LinkCopied);
}

/// The primary link revoked, and a new one made in its place.
fn revoke_primary() {
    let chat_id = chat_id();
    ask("type", json!({ "@type": "replacePrimaryChatInviteLink", "chat_id": chat_id }), |value| {
        let Ok(link) = serde_json::from_value::<ChatInviteLink>(value) else { return };
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.revoking = false;
            state.links.retain(|kept| !kept.is_primary);
            state.links.insert(0, link);
        });
        refresh();
    });
}

fn create_link() {
    let chat_id = chat_id();
    let Some((name, expiry, limit)) = STATE.with(|state| state.borrow().form.as_ref().map(|form| (form.name.trim().to_string(), form.expiry, form.limit))) else {
        return;
    };
    let request = json!({
        "@type": "createChatInviteLink", "chat_id": chat_id, "name": name,
        "expiration_date": if expiry > 0 { now() + expiry } else { 0 }, "member_limit": limit, "creates_join_request": false,
    });
    ask("type", request, |value| {
        let Ok(link) = serde_json::from_value::<ChatInviteLink>(value) else { return };
        let text = link.invite_link.clone();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.form = None;
            state.links.push(link);
        });
        refresh();
        copy_link(Some(text));
    });
}

fn revoke_link(link: String) {
    let chat_id = chat_id();
    ask("type", json!({ "@type": "revokeChatInviteLink", "chat_id": chat_id, "invite_link": link.clone() }), move |_| {
        STATE.with(|state| state.borrow_mut().links.retain(|kept| kept.invite_link != link));
        refresh();
    });
}

// ---- the members ----------------------------------------------------------------------------------

/// The contacts not in the chat, matching the words typed, once the contacts are known.
fn load_candidates() {
    let lookups = lookups();
    contacts::load(move || {
        if lookups != self::lookups() {
            return;
        }
        let Some(ui) = super::window() else { return };
        let names = Names::from(&ui);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if !state.adding {
                state.candidate_ids.clear();
                return;
            }
            let query = state.query.trim().to_lowercase();
            let digits: String = query.chars().filter(char::is_ascii_digit).collect();
            let digits = (!digits.is_empty() && digits.len() + 1 >= query.len()).then_some(digits.as_str());
            let members: Vec<i64> = state.members.iter().map(user_of).collect();
            let found = store::with(|store| contacts::matching(store, &query, digits, &names)).unwrap_or_default();
            state.candidate_ids = found.into_iter().filter(|id| !members.contains(id)).take(30).collect();
        });
        refresh();
    });
}

fn add_member(user_id: i64) {
    if user_id == 0 || !allowed("members", "invite") {
        return;
    }
    let chat_id = chat_id();
    let basic = store::with(|store| matches!(store.chats.get(&chat_id).map(|chat| chat.kind), Some(ChatType::BasicGroup { .. }))).unwrap_or(false);
    let request = if basic {
        json!({ "@type": "addChatMember", "chat_id": chat_id, "user_id": user_id, "forward_limit": 0 })
    } else {
        json!({ "@type": "addChatMembers", "chat_id": chat_id, "user_ids": [user_id] })
    };
    ask("members", request, move |_| {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if !state.members.iter().any(|member| user_of(member) == user_id) {
                let me = store::with(|store| store.my_id).unwrap_or(0);
                state.members.push(ChatMember { member_id: MessageSender::User { user_id }, inviter_user_id: me, status: ChatMemberStatus::Member });
                state.member_count += 1;
            }
            state.candidate_ids.retain(|id| *id != user_id);
        });
        refresh();
    });
}

fn ask_remove(user_id: i64) {
    if user_id == 0 || !allowed("members", "ban") {
        return;
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.removing = Some(user_id);
        state.errors.remove("members");
    });
    refresh();
}

fn remove_member() {
    let Some(user_id) = STATE.with(|state| state.borrow().removing) else { return };
    let chat_id = chat_id();
    let basic = store::with(|store| matches!(store.chats.get(&chat_id).map(|chat| chat.kind), Some(ChatType::BasicGroup { .. }))).unwrap_or(false);
    let status = if basic { json!({ "@type": "chatMemberStatusLeft" }) } else { json!({ "@type": "chatMemberStatusBanned", "banned_until_date": 0 }) };
    let request = json!({ "@type": "setChatMemberStatus", "chat_id": chat_id, "member_id": { "@type": "messageSenderUser", "user_id": user_id }, "status": status });
    ask("members", request, move |_| {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.members.retain(|member| user_of(member) != user_id);
            state.member_count = (state.member_count - 1).max(0);
            state.removing = None;
        });
        refresh();
    });
}

/// The status of a member, sent to Telegram; the page follows once it answers.
fn set_status(section: &'static str, user_id: i64, status: ChatMemberStatus, status_json: Value, then: impl FnOnce() + 'static) {
    let chat_id = chat_id();
    let request = json!({ "@type": "setChatMemberStatus", "chat_id": chat_id, "member_id": { "@type": "messageSenderUser", "user_id": user_id }, "status": status_json });
    ask(section, request, move |_| {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let me = store::with(|store| store.my_id).unwrap_or(0);
            if let Some(member) = state.members.iter_mut().find(|member| user_of(member) == user_id) {
                member.status = status;
                if matches!(member.status, ChatMemberStatus::Administrator { .. }) && member.inviter_user_id == 0 {
                    member.inviter_user_id = me;
                }
            }
        });
        then();
        refresh();
    });
}

fn admin_status(custom_title: &str, rights: &AdministratorRights) -> (ChatMemberStatus, Value) {
    let status = ChatMemberStatus::Administrator { custom_title: custom_title.to_string(), can_be_edited: true, rights: rights.clone() };
    let value = json!({ "@type": "chatMemberStatusAdministrator", "custom_title": custom_title, "can_be_edited": true, "rights": rights.to_json() });
    (status, value)
}

/// Make a member an administrator, with the few rights the design starts with, and open their
/// rights page.
fn promote(user_id: i64) {
    if user_id == 0 || !allowed("members", "promote") {
        return;
    }
    let group = STATE.with(|state| {
        let state = state.borrow();
        store::with(|store| !matches!(store.chats.get(&state.chat_id).map(|chat| chat.kind), Some(ChatType::Supergroup { is_channel: true, .. }))).unwrap_or(true)
    });
    let mut rights = AdministratorRights { can_manage_chat: true, ..AdministratorRights::default() };
    if group {
        rights.can_delete_messages = true;
        rights.can_pin_messages = true;
    } else {
        rights.can_post_messages = true;
    }
    let (status, value) = admin_status("", &rights);
    set_status("members", user_id, status, value, move || {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let me = store::with(|store| store.my_id).unwrap_or(0);
            if let Some(member) = state.members.iter_mut().find(|member| user_of(member) == user_id) {
                member.inviter_user_id = me;
            }
            state.who = Some(user_id);
            state.custom_title.clear();
            state.page = Page::Rights;
        });
    });
}

fn current_rights(user_id: i64) -> Option<(String, AdministratorRights)> {
    STATE.with(|state| {
        let state = state.borrow();
        let member = state.members.iter().find(|member| user_of(member) == user_id)?;
        match &member.status {
            ChatMemberStatus::Administrator { custom_title, rights, .. } => Some((custom_title.clone(), rights.clone())),
            _ => None,
        }
    })
}

fn toggle_right(right: &str) {
    let Some(user_id) = STATE.with(|state| state.borrow().who) else { return };
    let Some((custom_title, mut rights)) = current_rights(user_id) else { return };
    let (editable, mine) = STATE.with(|state| {
        let state = state.borrow();
        let member = state.members.iter().find(|member| user_of(member) == user_id);
        let editable = matches!(member.map(|member| &member.status), Some(ChatMemberStatus::Administrator { can_be_edited: true, .. }));
        let my_status = store::with(|store| {
            let chat = store.chats.get(&state.chat_id)?;
            Some(match chat.kind {
                ChatType::Supergroup { supergroup_id, .. } => store.supergroups.get(&supergroup_id)?.status.clone(),
                ChatType::BasicGroup { basic_group_id } => store.basic_groups.get(&basic_group_id)?.status.clone(),
                _ => return None,
            })
        })
        .flatten()
        .unwrap_or(ChatMemberStatus::Member);
        (editable, may(&my_status, right))
    });
    if !editable {
        let mut words = SharedString::new();
        with_ui(|ui| words = ui.global::<ChatManage>().get_lock_note());
        set_error("rights", words.to_string());
        return;
    }
    if !mine {
        let mut words = SharedString::new();
        with_ui(|ui| {
            let manage_words = ui.global::<ManageWords>();
            let group = ui.global::<ChatManage>().get_group();
            words = manage_words.invoke_cannot_give(right_label(&manage_words, group, right));
        });
        set_error("rights", words.to_string());
        return;
    }
    let on = !right_on(&rights, right);
    set_right(&mut rights, right, on);
    let custom = STATE.with(|state| state.borrow().custom_title.clone());
    let title = if custom.is_empty() { custom_title } else { custom };
    let (status, value) = admin_status(&title, &rights);
    set_status("rights", user_id, status, value, || {});
}

/// The title typed: saved a moment after the last keystroke, with the rights as they are.
fn edit_title(text: String) {
    let text: String = text.chars().take(16).collect();
    STATE.with(|state| state.borrow_mut().custom_title = text);
    let slot = super::context();
    TITLE_TIMER.with(|timer| {
        timer.start(TimerMode::SingleShot, TITLE_AFTER, move || {
            super::in_context(slot, || {
                let Some(user_id) = STATE.with(|state| state.borrow().who) else { return };
                let Some((_, rights)) = current_rights(user_id) else { return };
                let title = STATE.with(|state| state.borrow().custom_title.clone());
                let (status, value) = admin_status(&title, &rights);
                set_status("rights", user_id, status, value, || {});
            });
        });
    });
}

fn dismiss_admin() {
    let Some(user_id) = STATE.with(|state| state.borrow().who) else { return };
    TITLE_TIMER.with(Timer::stop);
    set_status("rights", user_id, ChatMemberStatus::Member, json!({ "@type": "chatMemberStatusMember" }), || {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.who = None;
            state.page = Page::Main;
        });
    });
}

// ---- the group's settings, and the channel's ------------------------------------------------------

fn toggle_permission(name: &str) {
    if !allowed("group", "ban") {
        return;
    }
    let chat_id = chat_id();
    let Some(permissions) = store::with(|store| {
        let mut permissions = store.chats.get(&chat_id)?.permissions.clone();
        match name {
            "send" => permissions.can_send_basic_messages = !permissions.can_send_basic_messages,
            "media" => {
                let on = !(permissions.can_send_photos || permissions.can_send_videos);
                permissions.can_send_photos = on;
                permissions.can_send_videos = on;
                permissions.can_send_audios = on;
                permissions.can_send_documents = on;
                permissions.can_send_video_notes = on;
                permissions.can_send_voice_notes = on;
            }
            "stickers" => permissions.can_send_other_messages = !permissions.can_send_other_messages,
            "links" => permissions.can_add_link_previews = !permissions.can_add_link_previews,
            "polls" => permissions.can_send_polls = !permissions.can_send_polls,
            "invite" => permissions.can_invite_users = !permissions.can_invite_users,
            "pin" => permissions.can_pin_messages = !permissions.can_pin_messages,
            "info" => permissions.can_change_info = !permissions.can_change_info,
            _ => return None,
        }
        Some(permissions)
    })
    .flatten() else {
        return;
    };
    ask("group", json!({ "@type": "setChatPermissions", "chat_id": chat_id, "permissions": permissions.to_json() }), |_| {});
}

fn set_slow_mode(seconds: i32) {
    if !allowed("group", "ban") {
        return;
    }
    as_supergroup("group", move |chat_id, _| {
        ask("group", json!({ "@type": "setChatSlowModeDelay", "chat_id": chat_id, "slow_mode_delay": seconds }), |_| {});
    });
}

/// One of the group's or channel's switches, turned the other way.
fn toggle_option(name: &str) {
    let right = match name {
        "approve" | "hidden" => "ban",
        "antispam" => "del",
        _ => "info",
    };
    if !allowed("group", right) {
        return;
    }
    let name = name.to_string();
    as_supergroup("group", move |chat_id, supergroup_id| {
        let Some(request) = store::with(|store| {
            let chat = store.chats.get(&chat_id)?;
            let group = store.supergroups.get(&supergroup_id)?;
            let full = store.supergroup_full.get(&supergroup_id);
            Some(match name.as_str() {
                "approve" => json!({
                    "@type": "toggleSupergroupJoinByRequest", "supergroup_id": supergroup_id, "join_by_request": !group.join_by_request,
                    "guard_bot_user_id": 0, "apply_to_invite_links": false,
                }),
                "topics" => json!({ "@type": "toggleSupergroupIsForum", "supergroup_id": supergroup_id, "is_forum": !group.is_forum, "has_forum_tabs": false }),
                "protect" => json!({ "@type": "toggleChatHasProtectedContent", "chat_id": chat_id, "has_protected_content": !chat.has_protected_content }),
                "history" => json!({
                    "@type": "toggleSupergroupIsAllHistoryAvailable", "supergroup_id": supergroup_id,
                    "is_all_history_available": !full.is_none_or(|info| info.is_all_history_available),
                }),
                "hidden" => json!({
                    "@type": "toggleSupergroupHasHiddenMembers", "supergroup_id": supergroup_id,
                    "has_hidden_members": !full.is_some_and(|info| info.has_hidden_members),
                }),
                "antispam" => json!({
                    "@type": "toggleSupergroupHasAggressiveAntiSpamEnabled", "supergroup_id": supergroup_id,
                    "has_aggressive_anti_spam_enabled": !full.is_some_and(|info| info.has_aggressive_anti_spam_enabled),
                }),
                "signatures" => json!({
                    "@type": "toggleSupergroupSignMessages", "supergroup_id": supergroup_id,
                    "sign_messages": !group.sign_messages, "show_message_sender": false,
                }),
                _ => return None,
            })
        })
        .flatten() else {
            return;
        };
        ask("group", request, |_| {});
    });
}

/// The discussion group's choices unfold (the groups the account may link, from Telegram).
fn toggle_discussion() {
    let open = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.discussion_open = !state.discussion_open;
        state.discussion_open
    });
    refresh();
    if !open {
        return;
    }
    let lookups = lookups();
    send(json!({ "@type": "getSuitableDiscussionChats" }), move |answer| {
        let Ok(Ok(chats)) = answer.map(serde_json::from_value::<api::Chats>) else { return };
        answered(lookups, |state| state.discussion_options = chats.chat_ids);
    });
}

fn set_discussion(discussion_chat_id: i64) {
    if !allowed("group", "info") {
        return;
    }
    let chat_id = chat_id();
    ask("group", json!({ "@type": "setChatDiscussionGroup", "chat_id": chat_id, "discussion_chat_id": discussion_chat_id }), |_| {
        STATE.with(|state| state.borrow_mut().discussion_open = false);
        // The full info follows from Telegram; until then the choice stands as made.
        refresh();
    });
}

// ---- the photo --------------------------------------------------------------------------------------

/// The photo clicked: a picture from the system's file panel, then the crop page.
fn change_photo() {
    if !allowed("profile", "info") {
        return;
    }
    pick_photo();
}

fn pick_photo() {
    let lookups = lookups();
    let slot = super::context();
    crate::platform::choose_files(true, move |paths| {
        let Some(path) = paths.into_iter().next() else { return };
        super::in_context(slot, || {
            crate::images::load(path.clone(), move |image| {
                super::in_context(slot, || {
                    let Some(image) = image else { return };
                    if lookups != self::lookups() {
                        return;
                    }
                    let size = image.size();
                    STATE.with(|state| {
                        let mut state = state.borrow_mut();
                        state.crop = Some(Crop { path, image, width: size.width, height: size.height });
                        state.page = Page::Crop;
                    });
                    with_ui(|ui| {
                        let manage = ui.global::<ChatManage>();
                        manage.set_zoom(1.2);
                        manage.set_offset_x(0.5);
                        manage.set_offset_y(0.5);
                    });
                    refresh();
                });
            });
        });
    });
}

/// Use: the frame's part of the picture, cropped square and scaled, becomes the chat's photo.
fn use_photo() {
    let Some((path, width, height)) = STATE.with(|state| state.borrow().crop.as_ref().map(|crop| (crop.path.clone(), crop.width, crop.height))) else { return };
    let (zoom, ox, oy) = {
        let mut values = (1.2f32, 0.5f32, 0.5f32);
        with_ui(|ui| {
            let manage = ui.global::<ChatManage>();
            values = (manage.get_zoom(), manage.get_offset_x(), manage.get_offset_y());
        });
        values
    };
    let lookups = lookups();
    let slot = super::context();
    let chat_id = chat_id();
    std::thread::spawn(move || {
        let result = crop_file(&path, width, height, zoom, ox, oy);
        let _ = slint::invoke_from_event_loop(move || {
            super::in_context(slot, || {
                if lookups != self::lookups() {
                    return;
                }
                match result {
                    Ok(cropped) => {
                        let request = json!({
                            "@type": "setChatPhoto", "chat_id": chat_id,
                            "photo": { "@type": "inputChatPhotoStatic", "photo": { "@type": "inputFileLocal", "path": cropped } },
                        });
                        ask("profile", request, |_| {});
                        STATE.with(|state| {
                            let mut state = state.borrow_mut();
                            state.crop = None;
                            state.page = Page::Main;
                        });
                        refresh();
                    }
                    Err(err) => set_error("profile", err),
                }
            });
        });
    });
}

/// The square the frame shows of the picture at `zoom`, with the picture's overflow placed by
/// `ox` and `oy` (0 … 1), as a JPEG file of [`PHOTO_SIDE`] pixels a side.
fn crop_file(path: &std::path::Path, width: u32, height: u32, zoom: f32, ox: f32, oy: f32) -> Result<String, String> {
    let picture = image::open(path).map_err(|err| format!("cannot read the picture: {err}"))?;
    let (width, height) = if picture.width() > 0 && picture.height() > 0 { (picture.width(), picture.height()) } else { (width, height) };
    let (left, top, side) = crop_rect(width, height, zoom, ox, oy);
    let cropped = picture.crop_imm(left, top, side, side).resize_exact(PHOTO_SIDE, PHOTO_SIDE, image::imageops::FilterType::Lanczos3);
    let folder = dirs::cache_dir().ok_or("the system has no cache folder")?.join("FinchGram");
    std::fs::create_dir_all(&folder).map_err(|err| format!("cannot create {}: {err}", folder.display()))?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis());
    let file = folder.join(format!("chat-photo-{stamp}.jpg"));
    let mut bytes = std::io::Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90);
    encoder.encode_image(&cropped.to_rgb8()).map_err(|err| format!("cannot encode the photo: {err}"))?;
    std::fs::write(&file, bytes.into_inner()).map_err(|err| format!("cannot write {}: {err}", file.display()))?;
    Ok(file.to_string_lossy().into_owned())
}

/// Where the frame sits in the picture, in its pixels: the square's left, top and side. The
/// frame shows the picture's shorter side divided by the zoom; the overflow is placed by the
/// offsets, as the page places it (ui/chat-manage-panel.slint does the same sums in points).
fn crop_rect(width: u32, height: u32, zoom: f32, ox: f32, oy: f32) -> (u32, u32, u32) {
    let zoom = zoom.clamp(1.0, 3.0);
    let short = width.min(height) as f32;
    let side = (short / zoom).max(1.0);
    let left = ((width as f32 - side) * ox.clamp(0.0, 1.0)).max(0.0);
    let top = ((height as f32 - side) * oy.clamp(0.0, 1.0)).max(0.0);
    let side = side.min(width as f32 - left).min(height as f32 - top).max(1.0) as u32;
    (left as u32, top as u32, side)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crop_covers_the_shorter_side_at_zoom_one() {
        assert_eq!(crop_rect(1000, 600, 1.0, 0.5, 0.5), (200, 0, 600));
        assert_eq!(crop_rect(600, 1000, 1.0, 0.0, 0.0), (0, 0, 600));
        assert_eq!(crop_rect(600, 1000, 1.0, 1.0, 1.0), (0, 400, 600));
    }

    #[test]
    fn zooming_in_takes_a_smaller_square_where_the_offsets_say() {
        let (left, top, side) = crop_rect(1000, 600, 2.0, 0.5, 0.5);
        assert_eq!(side, 300);
        assert_eq!((left, top), (350, 150));
        let (left, top, side) = crop_rect(1000, 600, 3.0, 1.0, 0.0);
        assert_eq!((left, top, side), (800, 0, 200));
    }

    #[test]
    fn counts_get_their_thousands_separated() {
        assert_eq!(count(486), "486");
        assert_eq!(count(32410), "32,410");
        assert_eq!(count(1234567), "1,234,567");
        assert_eq!(count(0), "0");
    }

    #[test]
    fn rights_switch_on_and_off_by_their_ids() {
        let mut rights = AdministratorRights::default();
        set_right(&mut rights, "stories", true);
        assert!(right_on(&rights, "stories") && rights.can_manage_chat);
        set_right(&mut rights, "stories", false);
        assert!(!right_on(&rights, "stories"));
        set_right(&mut rights, "promote", true);
        assert!(may(&ChatMemberStatus::Administrator { custom_title: String::new(), can_be_edited: true, rights }, "promote"));
        assert!(may(&ChatMemberStatus::Creator, "anything"));
        assert!(!may(&ChatMemberStatus::Member, "info"));
    }
}
