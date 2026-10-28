//! The pencil's menu (the design's compose.js round): a new group, a new channel, a new chat with a
//! contact, a new contact; Terminal's /group, /channel, /dm and /contact.
//!
//! A group takes its members first, then its name and photo: with members it is a basic group
//! (createNewBasicGroupChat), without any, or with more than a basic group holds, a supergroup
//! (createNewSupergroupChat, the members added after). A channel is made at its first step, as
//! Telegram's own apps make it (createNewSupergroupChat with is_channel), so that its second step
//! can check and set its public link (checkChatUsername, setSupergroupUsername) or show its invite
//! link (the primary one TDLib knows, or a new one: replacePrimaryChatInviteLink), and its third
//! invites subscribers (addChatMembers). A contact is added by phone number (importContacts): a
//! number with no account is said so. What Telegram refuses (FLOOD_WAIT, USERNAME_OCCUPIED,
//! USERS_TOO_MUCH, …) is shown in the dialog, which stays open. The service messages a new chat
//! shows (created, members added) are the ones Telegram's servers make.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{self, CheckChatUsernameResult, CreatedBasicGroupChat, ImportedContacts, SupergroupFullInfo};
use super::store::{self, Names};
use super::{Error, actions, contacts, conversation, send, with_ui};
use crate::{ChatKind, FoundKind, FoundRow, LinkCheck, MainWindow, NewChat, NewDialog, Status};

/// After the last keystroke in the link, before Telegram is asked whether it is free.
const LINK_PAUSE: Duration = Duration::from_millis(300);
/// A name's and a description's longest, as Telegram takes them.
const NAME_LENGTH: usize = 128;
const ABOUT_LENGTH: usize = 255;
/// A contact's phone number has at least this many digits (a country code and a number).
const PHONE_DIGITS: usize = 7;
/// Members added to a supergroup and to a channel in one request, TDLib's most.
const SUPERGROUP_BATCH: usize = 20;
const CHANNEL_BATCH: usize = 100;
/// A chat's photo, as uploaded: a square of so many pixels, as Telegram's own apps send it.
const PHOTO_SIDE: u32 = 640;
/// How long "Copied" stays by the invite link.
const COPIED_SHOWN: Duration = Duration::from_millis(1500);

#[derive(Default)]
struct State {
    dialog: Option<NewDialog>,
    step: i32,
    /// The contacts search's words, lower case.
    query: String,
    /// The contacts chosen, in the order chosen.
    picked: Vec<i64>,
    /// The photo chosen for the group or channel.
    photo: Option<PathBuf>,
    /// The channel made at its first step: its chat and its supergroup.
    channel: Option<(i64, i64)>,
    /// Counted up each time the link typed changes, so that an older check's answer is let go.
    link_lookups: u64,
    invite_link: String,
    busy: bool,
    /// Counted up for each "Copied", so that an older one's timer leaves a newer one be.
    copies: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static LINK_TIMER: slint::Timer = slint::Timer::default();
    /// Counted up each time a dialog opens, so that what comes back for an earlier one (a group
    /// made, a photo loaded) is not taken for this one's.
    static OPENINGS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Which opening of the dialog this is.
fn opening() -> u64 {
    OPENINGS.with(std::cell::Cell::get)
}

pub fn connect(ui: &MainWindow) {
    let new_chat = ui.global::<NewChat>();
    new_chat.on_start(|dialog| start(dialog, ""));
    new_chat.on_close(close);
    new_chat.on_search(|words| search(&words));
    new_chat.on_toggle(|id| {
        if let Ok(user_id) = id.parse() {
            toggle(user_id);
        }
    });
    new_chat.on_open_contact(|id| {
        if let Ok(user_id) = id.parse() {
            close();
            conversation::open_user(user_id);
        }
    });
    new_chat.on_choose_photo(choose_photo);
    new_chat.on_next(next);
    new_chat.on_back(back);
    new_chat.on_create(create);
    new_chat.on_set_public(set_public);
    new_chat.on_username_edited(username_edited);
    new_chat.on_copy_invite_link(copy_invite_link);
    new_chat.on_name_edited(name_edited);
    new_chat.on_phone_edited(phone_edited);
    new_chat.on_add_contact(add_contact);
}

fn with_dialog(change: impl FnOnce(&NewChat)) {
    with_ui(|ui| change(&ui.global::<NewChat>()));
}

/// Open a dialog afresh. `phone`: a number the search did not find, for the Add contact dialog.
fn start(dialog: NewDialog, phone: &str) {
    LINK_TIMER.with(|timer| timer.stop());
    OPENINGS.with(|openings| openings.set(openings.get() + 1));
    STATE.with(|state| *state.borrow_mut() = State { dialog: Some(dialog), step: 1, ..State::default() });
    let blank = store::with(|store| store.blank_image()).unwrap_or_default();
    with_dialog(|new_chat| {
        new_chat.set_step(1);
        new_chat.set_query(SharedString::new());
        new_chat.set_contacts(ModelRc::default());
        new_chat.set_picked(ModelRc::default());
        new_chat.set_name(SharedString::new());
        new_chat.set_name_initial(SharedString::new());
        new_chat.set_picked_names(SharedString::new());
        new_chat.set_about(SharedString::new());
        new_chat.set_photo(blank);
        new_chat.set_has_photo(false);
        new_chat.set_public(true);
        new_chat.set_username(SharedString::new());
        new_chat.set_link_check(LinkCheck::Empty);
        new_chat.set_invite_link(SharedString::new());
        new_chat.set_first_name(SharedString::new());
        new_chat.set_last_name(SharedString::new());
        new_chat.set_phone(phone.into());
        new_chat.set_phone_ok(contacts::digits(phone).len() >= PHONE_DIGITS);
        new_chat.set_busy(false);
        new_chat.set_error(SharedString::new());
        new_chat.set_link_copied(false);
        new_chat.set_can_go_back(false);
        new_chat.set_dialog(dialog);
    });
    if matches!(dialog, NewDialog::Group | NewDialog::Channel | NewDialog::Direct) {
        contacts::load(fill_contacts);
    }
}

/// The search found nobody with a phone number: add them as a contact, the number filled in.
pub fn start_contact(phone: &str) {
    start(NewDialog::Contact, if contacts::is_phone(phone) { phone.trim() } else { "" });
}

/// Close the dialog. A channel made at its first step opens: it exists now.
fn close() {
    LINK_TIMER.with(|timer| timer.stop());
    let channel = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let channel = state.channel.take();
        *state = State::default();
        channel
    });
    with_dialog(|new_chat| {
        new_chat.set_dialog(NewDialog::None);
        new_chat.set_busy(false);
    });
    if let Some((chat_id, _)) = channel {
        conversation::open(chat_id);
    } else {
        actions::focus_composer();
    }
}

/// Esc, when the keyboard is elsewhere: close the dialog if one is open.
pub(super) fn escape() -> bool {
    let open = STATE.with(|state| state.borrow().dialog.is_some());
    if open {
        close();
    }
    open
}

/// Logged out: a dialog open belongs to the account that was.
pub fn forget() {
    if STATE.with(|state| state.borrow().dialog.is_some()) {
        STATE.with(|state| state.borrow_mut().channel = None);
        close();
    }
}

fn set_error(message: impl Into<SharedString>) {
    let message = message.into();
    with_dialog(|new_chat| {
        new_chat.set_busy(false);
        new_chat.set_error(message);
    });
}

fn set_busy(busy: bool) {
    STATE.with(|state| state.borrow_mut().busy = busy);
    with_dialog(|new_chat| {
        new_chat.set_busy(busy);
        if busy {
            new_chat.set_error(SharedString::new());
        }
    });
}

/// What Telegram said, for the dialog; nothing when finchgram-tdlib ended (it starts again).
fn failed(what: &str, err: &Error) -> bool {
    match err {
        Error::Stopped => {
            set_busy(false);
            false
        }
        Error::Telegram { message, .. } => {
            eprintln!("telegram: cannot {what}: {err}");
            set_error(message.as_str());
            true
        }
    }
}

// ---- the contacts -------------------------------------------------------------------------------

fn search(words: &str) {
    let lower = words.trim().to_lowercase();
    STATE.with(|state| state.borrow_mut().query = lower.strip_prefix('@').unwrap_or(&lower).to_string());
    fill_contacts();
}

/// The contacts matching the words, the picked ones ticked; and the picked ones as chips.
fn fill_contacts() {
    let open = STATE.with(|state| state.borrow().dialog.is_some());
    if !open {
        return;
    }
    with_ui(|ui| {
        let names = Names::from(ui);
        let (query, picked) = STATE.with(|state| {
            let state = state.borrow();
            (state.query.clone(), state.picked.clone())
        });
        let digits = contacts::is_phone(&query).then(|| contacts::digits(&query));
        let (rows, chips) = store::with(|store| {
            let ids = contacts::matching(store, &query, digits.as_deref(), &names);
            let rows: Vec<FoundRow> = ids
                .iter()
                .filter_map(|id| store.users.get(id))
                .map(|user| FoundRow { picked: picked.contains(&user.id), ..contact_row(store, user, &names) })
                .collect();
            let chips: Vec<FoundRow> = picked.iter().filter_map(|id| store.users.get(id)).map(|user| contact_row(store, user, &names)).collect();
            (rows, chips)
        })
        .unwrap_or_default();
        let new_chat = ui.global::<NewChat>();
        let picked_names: Vec<String> = chips.iter().map(|chip| chip.title.to_string()).collect();
        new_chat.set_picked_names(picked_names.join(&names.list_separator).into());
        new_chat.set_contacts(ModelRc::new(VecModel::from(rows)));
        new_chat.set_picked(ModelRc::new(VecModel::from(chips)));
    });
    // The rows are built outside a refresh: their photos are downloaded here.
    store::fetch_wanted_photos();
}

fn contact_row(store: &store::Store, user: &api::User, names: &Names) -> FoundRow {
    let title = store.user_name(user.id, names);
    let (picture, has_picture) = store.user_avatar(user);
    let (status, last_seen) = store::user_status(user);
    FoundRow {
        section: FoundKind::Contact,
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
        ..FoundRow { picture: store.blank_image(), status: Status::None, ..FoundRow::default() }
    }
}

/// Photos of people arrived (avatars.rs): the rows show them.
pub fn photos_arrived() {
    fill_contacts();
}

fn toggle(user_id: i64) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if let Some(at) = state.picked.iter().position(|id| *id == user_id) {
            state.picked.remove(at);
        } else {
            state.picked.push(user_id);
        }
    });
    fill_contacts();
}

// ---- the photo ----------------------------------------------------------------------------------

/// The system's open panel: a picture for the group or channel, shown in the dialog and set once
/// the chat exists.
fn choose_photo() {
    let chosen_for = opening();
    crate::platform::choose_picture(move |path| {
        let Some(path) = path else { return };
        let shown = path.clone();
        crate::images::load(path, move |picture| {
            let Some(picture) = picture else { return };
            // The dialog it was chosen for, still: not another opened since.
            if STATE.with(|state| state.borrow().dialog.is_none()) || opening() != chosen_for {
                return;
            }
            STATE.with(|state| state.borrow_mut().photo = Some(shown));
            with_dialog(|new_chat| {
                new_chat.set_photo(picture);
                new_chat.set_has_photo(true);
            });
        });
    });
}

/// Give the chat the photo chosen: cut square and made [`PHOTO_SIDE`] wide off the UI thread, as
/// a JPEG in the cache folder, then uploaded (setChatPhoto); the file goes once it is up.
fn set_photo(chat_id: i64, path: PathBuf) {
    std::thread::Builder::new()
        .name("chat-photo".into())
        .spawn(move || {
            let prepared = prepare_photo(&path);
            let _ = slint::invoke_from_event_loop(move || match prepared {
                Ok(jpeg) => {
                    let request = json!({
                        "@type": "setChatPhoto", "chat_id": chat_id,
                        "photo": { "@type": "inputChatPhotoStatic", "photo": { "@type": "inputFileLocal", "path": jpeg } },
                    });
                    send(request, move |answer| {
                        if let Err(err) = answer {
                            eprintln!("telegram: cannot set the chat's photo: {err}");
                        }
                        let _ = std::fs::remove_file(&jpeg);
                    });
                }
                Err(err) => eprintln!("telegram: cannot prepare the chat's photo: {err}"),
            });
        })
        .map(|_| ())
        .unwrap_or_else(|err| eprintln!("telegram: cannot start the photo thread: {err}"));
}

/// The picture at `path` as Telegram takes a chat's photo: square, from the middle, at most
/// [`PHOTO_SIDE`] wide, JPEG. The file it was written to.
fn prepare_photo(path: &std::path::Path) -> Result<String, String> {
    // Read as the dialog showed it: turned the way it was taken, whatever kind of picture.
    let picture = crate::images::read(path, 2 * PHOTO_SIDE)?;
    let side = picture.width().min(picture.height());
    if side == 0 {
        return Err("an empty picture".into());
    }
    let square = picture.crop_imm((picture.width() - side) / 2, (picture.height() - side) / 2, side, side);
    let square = if side > PHOTO_SIDE { square.resize_exact(PHOTO_SIDE, PHOTO_SIDE, image::imageops::FilterType::Triangle) } else { square };
    let folder = dirs::cache_dir().ok_or("the system has no cache folder")?.join("FinchGram");
    std::fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_millis()).unwrap_or(0);
    let file = folder.join(format!("chat-photo-{stamp}.jpg"));
    square.into_rgb8().save_with_format(&file, image::ImageFormat::Jpeg).map_err(|err| err.to_string())?;
    Ok(file.to_string_lossy().into_owned())
}

// ---- the steps ----------------------------------------------------------------------------------

/// The dialog's name or description, trimmed and cut to Telegram's length.
fn field(read: impl FnOnce(&NewChat) -> SharedString, longest: usize) -> String {
    let mut value = String::new();
    with_dialog(|new_chat| value = read(new_chat).trim().chars().take(longest).collect());
    value
}

/// The main button before the last step: a group's members chosen, a channel made or its type set.
fn next() {
    let (dialog, step, busy) = STATE.with(|state| {
        let state = state.borrow();
        (state.dialog, state.step, state.busy)
    });
    if busy {
        return;
    }
    match (dialog, step) {
        (Some(NewDialog::Group), 1) => go_to(2, true),
        (Some(NewDialog::Channel), 1) => create_channel(),
        (Some(NewDialog::Channel), 2) => {
            let public = with_value(|new_chat| new_chat.get_public()).unwrap_or(true);
            if public { set_username() } else { go_to(3, false) }
        }
        _ => {}
    }
}

fn with_value<R>(read: impl FnOnce(&NewChat) -> R) -> Option<R> {
    let mut value = None;
    with_dialog(|new_chat| value = Some(read(new_chat)));
    value
}

fn go_to(step: i32, can_go_back: bool) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.step = step;
        state.query.clear();
    });
    with_dialog(|new_chat| {
        new_chat.set_step(step);
        new_chat.set_query(SharedString::new());
        new_chat.set_can_go_back(can_go_back);
        new_chat.set_busy(false);
        new_chat.set_error(SharedString::new());
    });
    fill_contacts();
}

/// A step back: a group's name to its members, a channel's subscribers to its type. A channel is
/// made at its first step, so there is no way back to it.
fn back() {
    let (dialog, step) = STATE.with(|state| {
        let state = state.borrow();
        (state.dialog, state.step)
    });
    match (dialog, step) {
        (Some(NewDialog::Group), 2) => go_to(1, false),
        (Some(NewDialog::Channel), 3) => go_to(2, false),
        _ => {}
    }
}

/// The last step's button: a group is made; a channel's subscribers are invited, and it opens.
fn create() {
    let (dialog, step, busy) = STATE.with(|state| {
        let state = state.borrow();
        (state.dialog, state.step, state.busy)
    });
    if busy {
        return;
    }
    match (dialog, step) {
        (Some(NewDialog::Group), 2) => create_group(),
        (Some(NewDialog::Channel), 3) => invite_subscribers(),
        _ => {}
    }
}

// ---- a group ------------------------------------------------------------------------------------

/// Make the group: a basic group with its members, or a supergroup when there are none or too
/// many for a basic group (the members are then added after), with the photo chosen; then it opens.
fn create_group() {
    let title = field(|new_chat| new_chat.get_name(), NAME_LENGTH);
    if title.is_empty() {
        return;
    }
    let picked = STATE.with(|state| state.borrow().picked.clone());
    // The photo as chosen now, for this group: Telegram may answer after the dialog was closed and
    // opened again for another group, with another photo chosen.
    let photo = STATE.with(|state| state.borrow().photo.clone());
    let made_for = opening();
    let most = store::with(|store| store.basic_group_size_max).unwrap_or(200) as usize;
    set_busy(true);
    if !picked.is_empty() && picked.len() < most {
        let request = json!({ "@type": "createNewBasicGroupChat", "user_ids": picked, "title": title, "message_auto_delete_time": 0 });
        send(request, move |answer| match answer.map(serde_json::from_value::<CreatedBasicGroupChat>) {
            Ok(Ok(created)) => made(created.chat_id, photo, made_for),
            Ok(Err(err)) => set_error(format!("cannot read the group made: {err}")),
            Err(err) => {
                failed("make a group", &err);
            }
        });
        return;
    }
    let request = json!({
        "@type": "createNewSupergroupChat", "title": title, "is_forum": false, "is_channel": false,
        "description": "", "location": null, "message_auto_delete_time": 0, "for_import": false,
    });
    send(request, move |answer| match answer {
        Ok(chat) => match chat["id"].as_i64() {
            Some(chat_id) => add_members(chat_id, picked, SUPERGROUP_BATCH, move |err| match err {
                None => made(chat_id, photo, made_for),
                Some(err) => {
                    if failed("add the members", &err) {
                        // The group exists without them: it opens all the same, the words said.
                        slint::Timer::single_shot(Duration::from_millis(1500), move || finish_for(made_for, chat_id));
                    }
                }
            }),
            None => set_error("cannot read the group made"),
        },
        Err(err) => {
            failed("make a group", &err);
        }
    });
}

/// Add `users` to the chat, `batch` at a time (TDLib's most), then `then` with what went wrong, if
/// anything.
fn add_members(chat_id: i64, users: Vec<i64>, batch: usize, then: impl FnOnce(Option<Error>) + 'static) {
    if users.is_empty() {
        then(None);
        return;
    }
    let (now, later) = users.split_at(users.len().min(batch));
    let (now, later) = (now.to_vec(), later.to_vec());
    send(json!({ "@type": "addChatMembers", "chat_id": chat_id, "user_ids": now }), move |answer| match answer {
        Ok(_) => add_members(chat_id, later, batch, then),
        Err(err) => then(Some(err)),
    });
}

/// The group is made: the photo chosen for it goes up, and it opens.
fn made(chat_id: i64, photo: Option<PathBuf>, made_for: u64) {
    if let Some(photo) = photo {
        set_photo(chat_id, photo);
    }
    finish_for(made_for, chat_id);
}

/// The chat made by the dialog's opening `made_for` opens; that opening's dialog closes with it,
/// a later one is left as it is.
fn finish_for(made_for: u64, chat_id: i64) {
    if opening() == made_for {
        finish_with(chat_id);
    } else {
        conversation::open(chat_id);
    }
}

/// Close the dialog and open the chat made.
fn finish_with(chat_id: i64) {
    STATE.with(|state| state.borrow_mut().channel = None);
    close();
    conversation::open(chat_id);
}

// ---- a channel ----------------------------------------------------------------------------------

/// The first step's Next: the channel is made, with its name and description and the photo chosen;
/// then its type is chosen.
fn create_channel() {
    let title = field(|new_chat| new_chat.get_name(), NAME_LENGTH);
    if title.is_empty() {
        return;
    }
    let about = field(|new_chat| new_chat.get_about(), ABOUT_LENGTH);
    // The photo as chosen now, for this channel (as for a group, `create_group`).
    let photo = STATE.with(|state| state.borrow().photo.clone());
    let made_for = opening();
    set_busy(true);
    let request = json!({
        "@type": "createNewSupergroupChat", "title": title, "is_forum": false, "is_channel": true,
        "description": about, "location": null, "message_auto_delete_time": 0, "for_import": false,
    });
    send(request, move |answer| match answer {
        Ok(chat) => {
            let chat_id = chat["id"].as_i64();
            let supergroup_id = chat["type"]["supergroup_id"].as_i64();
            let (Some(chat_id), Some(supergroup_id)) = (chat_id, supergroup_id) else {
                return set_error("cannot read the channel made");
            };
            if let Some(photo) = photo {
                set_photo(chat_id, photo);
            }
            if opening() != made_for || STATE.with(|state| state.borrow().dialog) != Some(NewDialog::Channel) {
                // Closed meanwhile, or opened again for something else: the channel exists all
                // the same.
                conversation::open(chat_id);
                return;
            }
            STATE.with(|state| state.borrow_mut().channel = Some((chat_id, supergroup_id)));
            go_to(2, false);
            load_invite_link(chat_id, supergroup_id);
        }
        Err(err) => {
            failed("make a channel", &err);
        }
    });
}

/// The channel's primary invite link, for its private choice: the one TDLib knows, else a new one.
fn load_invite_link(chat_id: i64, supergroup_id: i64) {
    let show = move |link: String| {
        if STATE.with(|state| state.borrow().channel.map(|(id, _)| id)) != Some(chat_id) {
            return;
        }
        STATE.with(|state| state.borrow_mut().invite_link = link.clone());
        with_dialog(|new_chat| new_chat.set_invite_link(link.into()));
    };
    send(json!({ "@type": "getSupergroupFullInfo", "supergroup_id": supergroup_id }), move |answer| {
        let known = match answer.map(serde_json::from_value::<SupergroupFullInfo>) {
            Ok(Ok(info)) => info.invite_link.map(|link| link.invite_link),
            _ => None,
        };
        if let Some(link) = known {
            show(link);
            return;
        }
        send(json!({ "@type": "replacePrimaryChatInviteLink", "chat_id": chat_id }), move |answer| {
            match answer.map(serde_json::from_value::<api::ChatInviteLink>) {
                Ok(Ok(link)) => show(link.invite_link),
                Ok(Err(err)) => eprintln!("telegram: cannot read the invite link: {err}"),
                Err(Error::Stopped) => {}
                Err(err) => eprintln!("telegram: cannot make an invite link: {err}"),
            }
        });
    });
}

fn set_public(public: bool) {
    with_dialog(|new_chat| {
        new_chat.set_public(public);
        new_chat.set_error(SharedString::new());
    });
}

/// The link typed changed: the rules first (compose.js's checkLink), then, after a pause,
/// Telegram's word on whether it is free.
fn username_edited() {
    LINK_TIMER.with(|timer| timer.stop());
    let username = with_value(|new_chat| new_chat.get_username().to_string()).unwrap_or_default();
    let lookups = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.link_lookups += 1;
        state.link_lookups
    });
    let check = link_rules(&username);
    with_dialog(|new_chat| new_chat.set_link_check(check));
    if check != LinkCheck::Ok {
        return;
    }
    with_dialog(|new_chat| new_chat.set_link_check(LinkCheck::Checking));
    LINK_TIMER.with(|timer| {
        timer.start(slint::TimerMode::SingleShot, LINK_PAUSE, move || {
            let chat_id = STATE.with(|state| state.borrow().channel.map(|(chat_id, _)| chat_id)).unwrap_or(0);
            send(json!({ "@type": "checkChatUsername", "chat_id": chat_id, "username": username }), move |answer| {
                if STATE.with(|state| state.borrow().link_lookups) != lookups {
                    return;
                }
                let check = match answer.map(serde_json::from_value::<CheckChatUsernameResult>) {
                    Ok(Ok(CheckChatUsernameResult::Ok)) => LinkCheck::Ok,
                    Ok(Ok(CheckChatUsernameResult::UsernameOccupied)) => LinkCheck::Taken,
                    Ok(Ok(CheckChatUsernameResult::UsernamePurchasable)) => LinkCheck::Purchasable,
                    Ok(Ok(CheckChatUsernameResult::UsernameInvalid)) => LinkCheck::BadCharacters,
                    Ok(Ok(CheckChatUsernameResult::PublicChatsTooMany)) => LinkCheck::TooMany,
                    Ok(Ok(CheckChatUsernameResult::PublicGroupsUnavailable)) => LinkCheck::Unavailable,
                    Ok(Err(err)) => {
                        eprintln!("telegram: cannot read the username check: {err}");
                        LinkCheck::Failed
                    }
                    Err(Error::Stopped) => LinkCheck::Failed,
                    Err(err) => {
                        with_dialog(|new_chat| new_chat.set_error(err.to_string().into()));
                        LinkCheck::Failed
                    }
                };
                with_dialog(|new_chat| new_chat.set_link_check(check));
            });
        });
    });
}

/// Telegram's rules for a public link: 5 to 32 characters, a–z, 0–9 and underscores, a letter
/// first, no underscore last. Ok here means the servers are still to be asked.
fn link_rules(username: &str) -> LinkCheck {
    if username.is_empty() {
        LinkCheck::Empty
    } else if !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        LinkCheck::BadCharacters
    } else if !username.starts_with(|c: char| c.is_ascii_alphabetic()) {
        LinkCheck::BadStart
    } else if username.len() < 5 {
        LinkCheck::Short
    } else if username.len() > 32 {
        LinkCheck::Long
    } else if username.ends_with('_') {
        LinkCheck::BadEnd
    } else {
        LinkCheck::Ok
    }
}

/// The second step's Next for a public channel: the link is set, then the subscribers are chosen.
fn set_username() {
    let Some((_, supergroup_id)) = STATE.with(|state| state.borrow().channel) else { return };
    let username = with_value(|new_chat| new_chat.get_username().to_string()).unwrap_or_default();
    if link_rules(&username) != LinkCheck::Ok {
        return;
    }
    set_busy(true);
    send(json!({ "@type": "setSupergroupUsername", "supergroup_id": supergroup_id, "username": username }), |answer| match answer {
        Ok(_) => go_to(3, true),
        Err(Error::Telegram { message, .. }) if message == "USERNAME_OCCUPIED" => {
            set_busy(false);
            with_dialog(|new_chat| new_chat.set_link_check(LinkCheck::Taken));
        }
        Err(err) => {
            failed("set the channel's link", &err);
        }
    });
}

fn copy_invite_link() {
    let link = STATE.with(|state| state.borrow().invite_link.clone());
    if link.is_empty() {
        return;
    }
    match crate::platform::copy_text(&link) {
        Ok(()) => {
            let copies = STATE.with(|state| {
                let mut state = state.borrow_mut();
                state.copies += 1;
                state.copies
            });
            with_dialog(|new_chat| new_chat.set_link_copied(true));
            slint::Timer::single_shot(COPIED_SHOWN, move || {
                if STATE.with(|state| state.borrow().copies) == copies {
                    with_dialog(|new_chat| new_chat.set_link_copied(false));
                }
            });
        }
        Err(err) => eprintln!("new chat: cannot copy the invite link: {err}"),
    }
}

/// The last step: the subscribers chosen are invited, and the channel opens.
fn invite_subscribers() {
    let Some((chat_id, _)) = STATE.with(|state| state.borrow().channel) else { return };
    let picked = STATE.with(|state| state.borrow().picked.clone());
    if picked.is_empty() {
        finish_with(chat_id);
        return;
    }
    set_busy(true);
    add_members(chat_id, picked, CHANNEL_BATCH, move |err| match err {
        None => finish_with(chat_id),
        Some(err) => {
            failed("invite the subscribers", &err);
        }
    });
}

/// The name typed changed: its first letter, for the photo's square.
fn name_edited() {
    let name = with_value(|new_chat| new_chat.get_name().to_string()).unwrap_or_default();
    with_dialog(|new_chat| new_chat.set_name_initial(store::initial(name.trim())));
}

// ---- a contact ----------------------------------------------------------------------------------

fn phone_edited() {
    let phone = with_value(|new_chat| new_chat.get_phone().to_string()).unwrap_or_default();
    with_dialog(|new_chat| {
        new_chat.set_phone_ok(contacts::digits(&phone).len() >= PHONE_DIGITS);
        new_chat.set_error(SharedString::new());
    });
}

/// Add the contact by their phone number: one who is a contact already opens; one with no account
/// is said so; else they are added, and their chat opens.
fn add_contact() {
    if STATE.with(|state| state.borrow().busy) {
        return;
    }
    let (first_name, last_name, phone) = with_value(|new_chat| {
        (
            new_chat.get_first_name().trim().chars().take(64).collect::<String>(),
            new_chat.get_last_name().trim().chars().take(64).collect::<String>(),
            new_chat.get_phone().trim().to_string(),
        )
    })
    .unwrap_or_default();
    let digits = contacts::digits(&phone);
    if first_name.is_empty() || digits.len() < PHONE_DIGITS {
        return;
    }
    if let Some(user_id) = contacts::by_phone(&digits) {
        close();
        conversation::open_user(user_id);
        return;
    }
    set_busy(true);
    let contact = json!({ "@type": "importedContact", "phone_number": format!("+{digits}"), "first_name": first_name, "last_name": last_name, "note": null });
    send(json!({ "@type": "importContacts", "contacts": [contact] }), |answer| match answer.map(serde_json::from_value::<ImportedContacts>) {
        Ok(Ok(imported)) => {
            contacts::changed();
            match imported.user_ids.first().copied().filter(|id| *id != 0) {
                Some(user_id) => {
                    close();
                    conversation::open_user(user_id);
                }
                None => {
                    let words = with_ui_words(|words| words.invoke_not_on_telegram().to_string());
                    set_error(words.unwrap_or_default());
                }
            }
        }
        Ok(Err(err)) => set_error(format!("cannot read the contact added: {err}")),
        Err(err) => {
            failed("add a contact", &err);
        }
    });
}

fn with_ui_words<R>(read: impl FnOnce(&crate::ComposeWords) -> R) -> Option<R> {
    let mut value = None;
    with_ui(|ui| value = Some(read(&ui.global::<crate::ComposeWords>())));
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_public_link_follows_telegrams_rules() {
        assert_eq!(link_rules(""), LinkCheck::Empty);
        assert_eq!(link_rules("keeb cn"), LinkCheck::BadCharacters);
        assert_eq!(link_rules("键圈"), LinkCheck::BadCharacters);
        assert_eq!(link_rules("1keeb"), LinkCheck::BadStart);
        assert_eq!(link_rules("_keeb"), LinkCheck::BadStart);
        assert_eq!(link_rules("keeb"), LinkCheck::Short);
        assert_eq!(link_rules(&"k".repeat(33)), LinkCheck::Long);
        assert_eq!(link_rules("keebcn_"), LinkCheck::BadEnd);
        assert_eq!(link_rules("keeb_cn"), LinkCheck::Ok);
        assert_eq!(link_rules(&"k".repeat(32)), LinkCheck::Ok);
    }

    #[test]
    fn members_are_added_in_batches() {
        let users: Vec<i64> = (1..=45).collect();
        let (now, later) = users.split_at(users.len().min(SUPERGROUP_BATCH));
        assert_eq!((now.len(), later.len()), (20, 25));
        let (now, later) = users.split_at(users.len().min(CHANNEL_BATCH));
        assert_eq!((now.len(), later.len()), (45, 0));
    }
}
