//! Telegram, through TDLib (docs/architecture.md). FinchGram never speaks Telegram's protocol
//! itself: everything goes through finchgram-tdlib, a separate program next to our executable that
//! runs the TDLib pinned in vendor/tdlib/build.sh (process.rs), and this module is its only
//! user.
//!
//! Requests are TDLib's own JSON objects ([`send`]). Each gets an "@extra" number, which TDLib
//! copies into its answer, and the answer goes to the callback given with the request. Updates are
//! parsed on the reader thread and reach the UI thread in batches: logging in follows the
//! authorization state (login.rs), notifications and the unread count go to notifications.rs,
//! everything else goes into the store (store.rs), and after each batch the pages are brought up to
//! date. chats.rs, conversation.rs, actions.rs, account.rs and password.rs do what the pages ask
//! for; global_search.rs and new_chat.rs (with contacts.rs) search everything and make new chats;
//! bots.rs does what bots' buttons, keyboards and commands ask, and keeps the pinned message bar;
//! avatars.rs fetches the photos of chats and people; online.rs tells TDLib whether the account
//! is online.
//!
//! Several accounts (accounts.rs): each has a finchgram-tdlib of its own, with a database folder
//! and a key of its own, and a store of its own. All of them run; one is in front, the one the
//! pages show. Which account a piece of code means is a matter of context ([`context`]): while a
//! batch from one account's finchgram-tdlib is handled, [`send`] and the store are that account's;
//! at any other time (a click, a timer) they are the account in front's. Code that runs later for
//! an account of its own (a timer it set) keeps its account with [`in_context`].
//!
//! When finchgram-tdlib ends unexpectedly, the requests still waiting fail and it is started
//! again: TDLib's database is on disk, so it carries on where it was. After a log out TDLib closes
//! itself: with other accounts there, the account is gone, folders and all; the only account's
//! fresh finchgram-tdlib is started for the next login.
//!
//! Everything here runs on the UI thread.

mod account;
mod accounts;
mod actions;
mod admin;
mod api;
mod avatars;
mod bots;
mod chat_info;
mod chats;
mod contacts;
mod conversation;
mod drafts;
mod errors;
mod files;
mod global_search;
mod login;
mod manage;
mod new_chat;
mod notifications;
mod online;
mod password;
mod process;
mod reactions;
mod rich_text;
mod search;
mod share;
mod store;
mod time;
#[cfg(test)]
mod timing;
pub mod tools;
mod topics;
mod attachments;
mod viewer;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use slint::ComponentHandle;

use crate::settings::Settings;
use crate::{AppState, Connection, Login, MainWindow, Page, TelegramState};
use api::{AuthorizationState, ConnectionState, OptionValue, Update};
use store::Followup;
use process::{Inbox, Output, Process};

/// The TDLib version api.rs is written for. finchgram-tdlib must be exactly this version, or the
/// app is packaged wrongly. Pinned in vendor/tdlib/build.sh and scripts/fetch-tdlib.sh.
pub const TDLIB_VERSION: &str = "1.8.67";

/// Telegram API credentials, from the environment at build time (README). Each developer uses
/// their own; the release workflow has FinchGram's.
const API_ID: Option<&str> = option_env!("FINCHGRAM_API_ID");
const API_HASH: Option<&str> = option_env!("FINCHGRAM_API_HASH");

/// How long the app waits for TDLib to close and write its database out when quitting.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// A finchgram-tdlib that ran at least this long before ending unexpectedly is started again;
/// one that ends sooner is broken, and the app says so instead of starting it over and over.
const RESTART_AFTER: Duration = Duration::from_secs(30);
const RESTART_DELAY: Duration = Duration::from_secs(1);

/// How many accounts at once at most, as in Telegram's own apps (the design's "n/10").
pub const ACCOUNTS_MAX: usize = 10;

/// An account's place on disk, 1 … [`ACCOUNTS_MAX`]: 1 has the folders `tdlib` and `media_cache`
/// and the Keychain key "tdlib", 2 has `tdlib-2`, `media_cache-2` and "tdlib-2", and so on
/// ([`database_directory`]). A slot whose account is gone is free for the next one added.
pub type Slot = u32;

/// An answer that is not what was asked for.
#[derive(Debug, Clone)]
pub enum Error {
    /// TDLib's own error: its code and message ("PHONE_NUMBER_INVALID", …).
    Telegram { code: i32, message: String },
    /// finchgram-tdlib is not running, or ended before it answered.
    Stopped,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Telegram { code, message } => write!(f, "{message} ({code})"),
            Error::Stopped => f.write_str("finchgram-tdlib is not running"),
        }
    }
}

type Answer = Box<dyn FnOnce(Result<Value, Error>)>;

/// One account's finchgram-tdlib, and what the window shows of the account while it is in front.
struct Client {
    slot: Slot,
    process: Option<Process>,
    /// Which start of finchgram-tdlib this is. Batches from an earlier one are dropped.
    run: u64,
    started: Instant,
    /// TDLib answered with the version api.rs is written for. Until then it gets no parameters.
    version_checked: bool,
    /// TDLib asked for its parameters before its version was checked.
    parameters_wanted: bool,
    /// TDLib said authorizationStateClosed in this run: it ends because it closed (a log out).
    closed: bool,
    /// Closed with other accounts there: the account is gone once finchgram-tdlib has ended.
    removed: bool,
    /// Something is wrong that starting again would not fix; see `fail`.
    failed: bool,
    next_extra: u64,
    waiting: HashMap<u64, Answer>,

    /// How far Telegram is, and the connection: the window's, while the account is in front.
    state: TelegramState,
    connection: Connection,
    /// Why it failed, when it did.
    error: String,
    /// TDLib's authorization state as last reported: the login page shows its step when the
    /// account comes to the front (login.rs).
    authorization: Option<AuthorizationState>,
    /// TDLib has not yet said where logging in is since this finchgram-tdlib was first started
    /// (login.rs throws a login left half done by an earlier run of the app away).
    first_state: bool,
    /// A login left half done is being thrown away: TDLib closes, and a fresh one starts.
    discarding: bool,
    /// Made by "Add account" in this run and not logged in yet: dropped when the user turns away.
    adding: bool,
    /// The account's user id, once getMe has answered (account.rs); 0 before.
    user_id: i64,
    /// Unread messages in the account's main chat list: all of them, and those in chats that are
    /// not muted (notifications.rs adds the accounts up for the Dock).
    unread: (i32, i32),
}

impl Client {
    fn new(slot: Slot, adding: bool) -> Client {
        Client {
            slot,
            process: None,
            run: 0,
            started: Instant::now(),
            version_checked: false,
            parameters_wanted: false,
            closed: false,
            removed: false,
            failed: false,
            next_extra: 1,
            waiting: HashMap::new(),
            state: TelegramState::Starting,
            connection: Connection::Connecting,
            error: String::new(),
            authorization: None,
            first_state: true,
            discarding: false,
            adding,
            user_id: 0,
            unread: (0, 0),
        }
    }
}

thread_local! {
    /// The accounts, in the order of their slots.
    static CLIENTS: RefCell<Vec<Client>> = const { RefCell::new(Vec::new()) };
    /// The account in front: the one the pages show.
    static SHOWN: Cell<Slot> = const { Cell::new(0) };
    /// The account whose batch (or whose own later work) is being handled right now, if any.
    static HANDLING: Cell<Option<Slot>> = const { Cell::new(None) };
    static WINDOW: RefCell<Option<slint::Weak<MainWindow>>> = const { RefCell::new(None) };
    static SETTINGS: RefCell<Option<Rc<RefCell<Settings>>>> = const { RefCell::new(None) };
    /// The app is quitting: no finchgram-tdlib is started again.
    static QUITTING: Cell<bool> = const { Cell::new(false) };
}

/// Start a finchgram-tdlib for every account on disk (the first one, when there is none), show
/// how far they are in `ui`, and connect the pages that show Telegram. The account in front is
/// the one that was when the app last ran (`account` in settings.toml). Call once.
pub fn start(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    WINDOW.with(|window| *window.borrow_mut() = Some(ui.as_weak()));
    login::connect(ui);
    chats::connect(ui);
    conversation::connect(ui);
    actions::connect(ui);
    account::connect(ui);
    accounts::connect(ui);
    password::connect(ui);
    viewer::connect(ui);
    notifications::connect(ui);
    attachments::connect(ui);
    search::connect(ui);
    global_search::connect(ui);
    new_chat::connect(ui);
    bots::connect(ui);
    reactions::connect(ui);
    topics::connect(ui);
    share::connect(ui);
    manage::connect(ui);
    contacts::connect(ui);
    viewer::connect_round(ui);
    {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
        ui.window().on_winit_window_event(|_, event| {
            // Terminal's ⌃1 … ⌃0 switch accounts, as its pane says (the menu bar's ⌘1 … ⌘0 work in
            // every theme).
            accounts::window_event(event);
            match event {
                WindowEvent::Focused(in_front) => {
                    online::window_in_front(*in_front);
                    if *in_front {
                        conversation::window_came_to_front();
                    }
                }
                WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::CursorMoved { .. } => online::input(),
                // Files dragged from the Finder (docs/drag-and-drop.md): one event per file.
                WindowEvent::HoveredFile(path) => attachments::hovering(path.clone()),
                WindowEvent::HoveredFileCancelled => attachments::hover_ended(),
                WindowEvent::DroppedFile(path) => attachments::dropped(path.clone()),
                _ => {}
            }
            // Right clicks are seen here, before the pages: a message's menu opens wherever on it
            // the click was, even on its formatted words, which keep clicks to themselves.
            actions::window_event(event);
            crate::screenshot::window_event(event);
            EventResult::Propagate
        });
    }

    let test_dc = use_test_dc();
    let mut slots = slots_on_disk(test_dc);
    if slots.is_empty() {
        slots.push(1);
    }
    let wanted = settings.borrow().account;
    let shown = if slots.contains(&wanted) { wanted } else { slots[0] };
    SETTINGS.with(|kept| *kept.borrow_mut() = Some(settings));
    for &slot in &slots {
        CLIENTS.with(|clients| clients.borrow_mut().push(Client::new(slot, false)));
        store::add(slot);
    }
    SHOWN.set(shown);
    store::show(shown);
    for &slot in &slots {
        launch(slot);
    }
    accounts::refresh();
}

/// The slots that have a database folder, in order.
fn slots_on_disk(test_dc: bool) -> Vec<Slot> {
    let Some(base) = dirs::data_dir() else { return Vec::new() };
    let base = base.join("FinchGram");
    (1..=ACCOUNTS_MAX as Slot).filter(|&slot| base.join(database_folder(test_dc, slot)).is_dir()).collect()
}

/// The UI language changed: the names Rust puts in rows ("Saved Messages") change with it.
pub fn language_changed() {
    store::refresh_all();
}

/// Settings → Notifications & sounds changed (main.rs saves them; notifications.rs reads them from
/// AppState).
pub fn notification_settings_changed(desktop_turned_on: bool) {
    notifications::settings_changed(desktop_turned_on);
}

/// Show a file FinchGram downloaded from a WebDAV server (src/downloads/) in the media viewer: a
/// photo, or a video that plays.
pub fn view_downloaded_file(path: &str, video: bool) {
    viewer::view_file(path, video);
}

/// The media viewer's volume and mute: the user's last, from the settings, and back into them as
/// they change (viewer.rs).
pub fn connect_viewer_settings(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    viewer::connect_settings(ui, settings);
}

/// How the folders' chats are ordered: from the settings, and back into them when a folder's menu
/// changes it (chats.rs).
pub fn connect_chat_order(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    chats::connect_order(ui, settings);
}

/// The chat's info panel remembers whether it is open (chat_info.rs).
pub fn connect_chat_info(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    chat_info::connect(ui, settings);
}

/// The app is quitting: let every TDLib write its database out and every finchgram-tdlib end, for
/// at most [`CLOSE_TIMEOUT`] all together. Call once the event loop has ended.
pub fn shut_down() {
    QUITTING.set(true);
    let processes: Vec<Process> = CLIENTS.with(|clients| clients.borrow_mut().iter_mut().filter_map(|client| client.process.take()).collect());
    process::shut_down_all(processes, CLOSE_TIMEOUT);
}

// ---- which account ----------------------------------------------------------------------------------

/// The account meant right now: the one whose batch is being handled, else the one in front.
pub(super) fn context() -> Slot {
    HANDLING.get().unwrap_or_else(shown)
}

/// The account in front: the one the pages show.
pub(super) fn shown() -> Slot {
    SHOWN.get()
}

/// Whether the account meant right now is the one in front: only then do the pages follow it.
pub(super) fn is_front() -> bool {
    context() == shown()
}

/// Run `work` as the account in `slot`: its store, its finchgram-tdlib. For work an account does
/// later on its own (a timer), outside the handling of its batches.
pub(super) fn in_context<R>(slot: Slot, work: impl FnOnce() -> R) -> R {
    let before = HANDLING.replace(Some(slot));
    let result = work();
    HANDLING.set(before);
    result
}

fn with_client<R>(slot: Slot, change: impl FnOnce(&mut Client) -> R) -> Option<R> {
    CLIENTS.with(|clients| clients.borrow_mut().iter_mut().find(|client| client.slot == slot).map(change))
}

fn slots() -> Vec<Slot> {
    CLIENTS.with(|clients| clients.borrow().iter().map(|client| client.slot).collect())
}

/// The slot of the signed-in account with `user_id`, if it is here.
pub(super) fn slot_of_user(user_id: i64) -> Option<Slot> {
    if user_id == 0 {
        return None;
    }
    CLIENTS.with(|clients| {
        clients.borrow().iter().find(|client| client.user_id == user_id && client.state == TelegramState::Ready).map(|client| client.slot)
    })
}

/// Run `work` as the signed-in account with `user_id`, when it is here: the AI access settings
/// (src/mcp/) are one account's, and assistants work with that account whichever is in front.
pub fn in_account<R>(user_id: i64, work: impl FnOnce() -> R) -> Option<R> {
    slot_of_user(user_id).map(|slot| in_context(slot, work))
}

// ---- the accounts ---------------------------------------------------------------------------------------

/// What accounts.rs shows of each account, in the order of their slots.
pub(super) struct AccountFacts {
    pub slot: Slot,
    pub state: TelegramState,
    pub adding: bool,
    pub unread: (i32, i32),
}

pub(super) fn account_facts() -> Vec<AccountFacts> {
    CLIENTS.with(|clients| {
        clients
            .borrow()
            .iter()
            .map(|client| AccountFacts { slot: client.slot, state: client.state, adding: client.adding, unread: client.unread })
            .collect()
    })
}

/// Bring the account in `slot` to the front.
pub fn switch_to(slot: Slot) {
    show(slot);
}

/// Bring the account in the `index`th place (0 … 9: ⌘1 … ⌘0) to the front.
pub fn switch_to_index(index: usize) {
    if let Some(slot) = slots().get(index).copied() {
        show(slot);
    }
}

/// "Add account": a finchgram-tdlib in the first free slot, and the login page for it. Nothing
/// beyond [`ACCOUNTS_MAX`] accounts.
pub fn add_account() {
    let taken = slots();
    if taken.len() >= ACCOUNTS_MAX {
        return;
    }
    let Some(slot) = (1..=ACCOUNTS_MAX as Slot).find(|slot| !taken.contains(slot)) else { return };
    CLIENTS.with(|clients| {
        let mut clients = clients.borrow_mut();
        clients.push(Client::new(slot, true));
        clients.sort_by_key(|client| client.slot);
    });
    store::add(slot);
    launch(slot);
    show(slot);
}

/// "Back" on the login page of an account being added: to another account, which drops this one.
pub(super) fn cancel_adding() {
    let current = shown();
    let other = CLIENTS.with(|clients| clients.borrow().iter().find(|client| client.slot != current && !client.adding).map(|client| client.slot));
    if let Some(other) = other {
        show(other);
    }
}

/// The account in `slot` comes to the front: the pages show its store, its state, its login step.
/// The account that was in front keeps its chats, tabs and drafts for when it comes back; one
/// that was being added and is not logged in is dropped.
fn show(slot: Slot) {
    let old = shown();
    if slot == old || with_client(slot, |_| ()).is_none() {
        return;
    }
    let old_there = with_client(old, |_| ()).is_some();
    if old_there {
        in_context(old, leave_front);
    }
    SHOWN.set(slot);
    SETTINGS.with(|kept| {
        if let Some(settings) = &*kept.borrow() {
            let mut settings = settings.borrow_mut();
            if settings.account != slot {
                settings.account = slot;
                settings.save();
            }
        }
    });
    let Some((state, connection, error, authorization, adding)) =
        with_client(slot, |client| (client.state, client.connection, client.error.clone(), client.authorization.clone(), client.adding))
    else {
        return;
    };
    with_state(|app| {
        app.set_telegram_state(state);
        app.set_connection(connection);
        app.set_telegram_error(error.into());
    });
    with_ui(|ui| ui.global::<Login>().set_adding(adding));
    in_context(slot, || {
        // The pages' models become this account's, and are built again from its store.
        store::show(slot);
        login::show(authorization.as_ref());
        online::set_ready(state == TelegramState::Ready);
        enter_front();
    });
    if old_there && with_client(old, |client| client.adding && client.state != TelegramState::Ready).unwrap_or(false) {
        drop_slot(old);
    }
    accounts::refresh();
}

/// The account in front is about to be another: what belongs to this one and the window at once
/// is put away. The words in the composer stay with the chat they were for (drafts.rs); menus,
/// dialogs, the strip and the viewer close; the account goes offline.
fn leave_front() {
    if let Some(open) = store::with(|store| store.open).flatten() {
        drafts::leaving(open);
    }
    online::leaving();
    viewer::close();
    attachments::escape();
    actions::chat_changed(None);
    global_search::forget();
    new_chat::forget();
    chat_info::forget();
    manage::forget();
    bots::forget();
    topics::forget();
    drafts::forget();
    share::forget();
    admin::forget();
    password::forget();
}

/// The account's store shows: what follows its open chat follows it anew (as when a chat opens,
/// conversation.rs), and what arrived in that chat while the account was not in front is seen.
fn enter_front() {
    let open = store::with(|store| store.open).flatten();
    actions::chat_changed(open);
    reactions::chat_changed();
    viewer::chat_changed();
    chat_info::chat_changed();
    if let Some(open) = open {
        drafts::entering(open);
    }
    conversation::window_came_to_front();
}

/// A slot without an account in it (adding was given up; empty on disk at the start) is of no use
/// while other accounts are there: TDLib destroys what little it has, and the slot goes with its
/// folders (`Closed`, then on_ended).
fn drop_slot(slot: Slot) {
    with_client(slot, |client| {
        client.adding = false;
        client.removed = true;
    });
    in_context(slot, || {
        send(json!({ "@type": "destroy" }), |answer| {
            if let Err(err @ Error::Telegram { .. }) = answer {
                eprintln!("telegram: cannot destroy an empty account: {err}");
            }
        });
    });
}

/// finchgram-tdlib has ended after TDLib closed with other accounts there: the account is gone.
/// Its folders go (TDLib emptied them), its slot is free, and another account comes to the front
/// when it was the one there.
fn remove_account(slot: Slot) {
    let test_dc = use_test_dc();
    CLIENTS.with(|clients| clients.borrow_mut().retain(|client| client.slot != slot));
    store::remove(slot);
    if let Some(base) = dirs::data_dir() {
        remove_folder(&base.join("FinchGram").join(database_folder(test_dc, slot)));
    }
    if let Some(documents) = dirs::document_dir() {
        remove_folder(&documents.join("FinchGram").join(media_folder(test_dc, slot)));
    }
    if slot == shown()
        && let Some(next) = slots().first().copied()
    {
        show(next);
    }
    accounts::refresh();
    bind_ai_access(0);
}

fn remove_folder(folder: &std::path::Path) {
    if folder.is_dir()
        && let Err(err) = std::fs::remove_dir_all(folder)
    {
        eprintln!("telegram: cannot remove {}: {err}", folder.display());
    }
}

/// An account's getMe answered (account.rs): the account is known by its user id.
pub(super) fn account_signed_in(user_id: i64) {
    with_client(context(), |client| client.user_id = user_id);
    accounts::changed();
    bind_ai_access(user_id);
}

/// The AI access settings (src/mcp/, docs/mcp.md) are one account's: the first that signed in,
/// and that one as long as it is here. Once it is gone (logged out) and every account left has
/// said who it is, the account in front takes its place (or `fallback`, the one that just signed
/// in): the apps allowed ask again and the chosen chats start empty, as when another account logs
/// in with one account.
fn bind_ai_access(fallback: i64) {
    let bound = crate::mcp::bound_account();
    let (present, all_known, front) = CLIENTS.with(|clients| {
        let clients = clients.borrow();
        let present = bound != 0 && clients.iter().any(|client| client.user_id == bound);
        let all_known = clients.iter().all(|client| {
            client.user_id != 0 || !matches!(client.state, TelegramState::Starting | TelegramState::Ready | TelegramState::LoggingOut | TelegramState::Closing)
        });
        let front = clients.iter().find(|client| client.slot == shown()).map_or(0, |client| client.user_id);
        (present, all_known, front)
    });
    if present || (bound != 0 && !all_known) {
        return;
    }
    let id = if front != 0 { front } else { fallback };
    if id != 0 {
        crate::mcp::signed_in(id);
    }
}

// ---- the login's flags and the unread counts, kept per account ----------------------------------------------

/// Whether TDLib has not yet said where logging in is since this account's finchgram-tdlib was
/// first started; false from then on.
pub(super) fn take_first_state() -> bool {
    with_client(context(), |client| std::mem::replace(&mut client.first_state, false)).unwrap_or(false)
}

pub(super) fn set_discarding(discarding: bool) {
    with_client(context(), |client| client.discarding = discarding);
}

pub(super) fn discarding() -> bool {
    with_client(context(), |client| client.discarding).unwrap_or(false)
}

/// The account's unread messages in its main chat list: all, and those in chats that are not muted.
pub(super) fn set_unread(unread: (i32, i32)) {
    with_client(context(), |client| client.unread = unread);
    accounts::changed();
}

/// Every account's unread messages added up, counting muted chats or not (the Dock's number).
pub(super) fn unread_sum(count_muted: bool) -> i32 {
    CLIENTS.with(|clients| {
        clients.borrow().iter().map(|client| if count_muted { client.unread.0 } else { client.unread.1 }).fold(0i32, i32::saturating_add)
    })
}

// ---- finchgram-tdlib -------------------------------------------------------------------------------------

/// Send `request`, one of TDLib's functions as a JSON object (`{"@type": "getMe"}`), to the
/// account meant right now ([`context`]), and hand its answer to `on_answer`. When its
/// finchgram-tdlib is not running, or ends before it answers, `on_answer` gets [`Error::Stopped`].
pub fn send(mut request: Value, on_answer: impl FnOnce(Result<Value, Error>) + 'static) {
    let slot = context();
    let mut on_answer: Option<Answer> = Some(Box::new(on_answer));
    with_client(slot, |client| {
        if client.process.is_none() {
            return;
        }
        let extra = client.next_extra;
        client.next_extra += 1;
        request["@extra"] = extra.into();
        client.waiting.insert(extra, on_answer.take().expect("taken once"));
        if let Some(process) = &client.process {
            process.send(request.to_string());
        }
    });
    // Not running: the answer comes later all the same, so that no caller is called back while it
    // is still in the middle of sending.
    if let Some(on_answer) = on_answer {
        slint::Timer::single_shot(Duration::ZERO, move || in_context(slot, || on_answer(Err(Error::Stopped))));
    }
}

/// Start the account's finchgram-tdlib (again), as a new run.
fn launch(slot: Slot) {
    let started = with_client(slot, |client| {
        client.run += 1;
        let run = client.run;
        let inbox = Inbox::default();
        let wake = {
            let inbox = inbox.clone();
            move || {
                let inbox = inbox.clone();
                // Fails only once the event loop has ended, when nothing waits for TDLib any more.
                let _ = slint::invoke_from_event_loop(move || deliver(slot, run, inbox.take()));
            }
        };
        let process = Process::start(inbox, wake)?;
        client.process = Some(process);
        client.started = Instant::now();
        client.version_checked = false;
        client.parameters_wanted = false;
        client.closed = false;
        Ok::<(), String>(())
    });
    match started {
        Some(Ok(())) => {
            // A new finchgram-tdlib begins offline, without an account until it says it has one.
            if slot == shown() {
                online::set_ready(false);
            }
            set_state(slot, TelegramState::Starting);
            in_context(slot, check_version);
        }
        Some(Err(err)) => in_context(slot, || fail(err)),
        None => {}
    }
}

/// A batch from one account's finchgram-tdlib.
fn deliver(slot: Slot, run: u64, batch: Vec<Output>) {
    in_context(slot, || {
        for output in batch {
            // Looked at before every message: handling one may have started a new run, or removed
            // the account.
            if with_client(slot, |client| client.run) != Some(run) {
                return;
            }
            match output {
                Output::Answer { extra, result } => {
                    if let Some(on_answer) = with_client(slot, |client| client.waiting.remove(&extra)).flatten() {
                        on_answer(result);
                    }
                }
                Output::Update(update) => on_update(*update),
                Output::Ended => on_ended(),
            }
        }
        store::refresh();
    });
    notifications::open_wanted();
    accounts::refresh_if_changed();
}

fn on_update(update: Update) {
    // A message an assistant sent has its final id now, or was not sent (tools.rs answers then).
    if let Update::MessageSendSucceeded { message, old_message_id } | Update::MessageSendFailed { message, old_message_id } = &update {
        tools::message_sent(*old_message_id, message, matches!(update, Update::MessageSendSucceeded { .. }));
    }
    match update {
        Update::AuthorizationState { authorization_state } => on_authorization_state(authorization_state),
        Update::ConnectionState { state } => set_connection(context(), connection(state)),
        Update::File { file } => {
            files::updated(&file);
            store::with(|store| store.file_progress(&file));
            store::refresh();
        }
        Update::NotificationGroup {
            notification_group_id,
            chat_id,
            notification_settings_chat_id,
            notification_sound_id,
            added_notifications,
            removed_notification_ids,
        } => notifications::group_changed(
            notification_group_id,
            chat_id,
            notification_settings_chat_id,
            notification_sound_id,
            added_notifications,
            removed_notification_ids,
        ),
        Update::ActiveNotifications { groups } => notifications::active(groups),
        Update::UnreadMessageCount { chat_list, unread_count, unread_unmuted_count } => {
            notifications::unread_changed(chat_list, unread_count, unread_unmuted_count)
        }
        update => match store::with(|store| store.apply(update)) {
            Some(Followup::LoadFolders) => chats::load_folders(),
            Some(Followup::View { chat_id, message_ids }) => conversation::view(chat_id, message_ids),
            Some(Followup::TypingExpires) => {
                let slot = context();
                slint::Timer::single_shot(store::TYPING_LASTS, move || {
                    in_context(slot, || {
                        store::with(|store| store.dirty.header = true);
                        store::refresh();
                    });
                });
            }
            Some(Followup::Deleted { chat_id, message_ids }) => actions::deleted(chat_id, &message_ids),
            Some(Followup::PinnedChanged { chat_id }) => bots::pinned_changed(chat_id),
            Some(Followup::KeyboardChanged { chat_id }) => bots::keyboard_changed(chat_id),
            Some(Followup::TopicChanged { chat_id, topic_id }) => topics::topic_changed(chat_id, topic_id),
            Some(Followup::DraftChanged { chat_id }) => drafts::draft_changed(chat_id),
            Some(Followup::SlowModeChanged) => admin::slow_mode_changed(),
            Some(Followup::ProtectionChanged) => conversation::protect_window(),
            Some(Followup::None) | None => {}
        },
    }
}

fn on_authorization_state(state: AuthorizationState) {
    let slot = context();
    // A login left half done by an earlier run of the app is thrown away, unseen (login.rs).
    if login::discard_unfinished(&state) {
        set_state(slot, TelegramState::Starting);
        return;
    }
    let discarding = login::discarding();
    with_client(slot, |client| client.authorization = Some(state.clone()));
    if is_front() {
        login::on_state(&state);
    }
    let shown = match state {
        AuthorizationState::WaitTdlibParameters => {
            let checked = with_client(slot, |client| {
                client.parameters_wanted = !client.version_checked;
                client.version_checked
            });
            if checked == Some(true) {
                set_parameters();
            }
            return;
        }
        AuthorizationState::WaitPhoneNumber => {
            // No account in this slot. With other accounts there it is of no use, unless the user
            // is adding an account in it right now: it goes.
            let adding = with_client(slot, |client| client.adding).unwrap_or(false);
            if slots().len() > 1 && !adding {
                drop_slot(slot);
                return;
            }
            TelegramState::WaitPhoneNumber
        }
        AuthorizationState::WaitPremiumPurchase => TelegramState::WaitPremiumPurchase,
        AuthorizationState::WaitEmailAddress => TelegramState::WaitEmailAddress,
        AuthorizationState::WaitEmailCode { .. } => TelegramState::WaitEmailCode,
        AuthorizationState::WaitCode { .. } => TelegramState::WaitCode,
        AuthorizationState::WaitOtherDeviceConfirmation { .. } => TelegramState::WaitOtherDevice,
        AuthorizationState::WaitRegistration => TelegramState::WaitRegistration,
        AuthorizationState::WaitPassword { .. } => TelegramState::WaitPassword,
        AuthorizationState::Ready => {
            with_client(slot, |client| client.adding = false);
            with_ui(|ui| {
                if is_front() {
                    ui.global::<Login>().set_adding(false);
                }
            });
            account::load();
            chats::load_main_list();
            notifications::start();
            if is_front() {
                online::set_ready(true);
            }
            TelegramState::Ready
        }
        AuthorizationState::LoggingOut | AuthorizationState::Closing if discarding => TelegramState::Starting,
        AuthorizationState::LoggingOut => TelegramState::LoggingOut,
        AuthorizationState::Closing => TelegramState::Closing,
        AuthorizationState::Closed => {
            // finchgram-tdlib ends right after this; on_ended decides what follows. The account
            // is gone (a log out): forget it. The last account's slot stays for the next login;
            // with others there, the slot goes.
            let only = slots().len() == 1;
            with_client(slot, |client| {
                client.closed = true;
                client.removed |= !only;
                client.user_id = 0;
                client.unread = (0, 0);
            });
            store::clear();
            chats::forget();
            contacts::forget();
            files::forget();
            notifications::forget();
            if is_front() {
                global_search::forget();
                new_chat::forget();
                chat_info::forget();
                manage::forget();
                bots::forget();
                topics::forget();
                drafts::forget();
                share::forget();
                admin::forget();
                account::forget();
                password::forget();
                online::set_ready(false);
                with_state(|app| app.set_page(Page::Chats));
            }
            accounts::changed();
            return;
        }
    };
    set_state(slot, shown);
}

/// The first request of every run: which TDLib this is. Anything but the version api.rs is
/// written for is a packaging error. (The first request is also what starts TDLib's client.)
fn check_version() {
    let slot = context();
    send(json!({ "@type": "getOption", "name": "version" }), move |answer| {
        let version = match answer.map(serde_json::from_value::<OptionValue>) {
            Ok(Ok(OptionValue::String { value })) => value,
            Err(Error::Stopped) => return, // on_ended tells what happened
            other => return fail(format!("cannot read the TDLib version of finchgram-tdlib: {other:?}")),
        };
        if version != TDLIB_VERSION {
            return fail(format!(
                "finchgram-tdlib runs TDLib {version}, this build is made for {TDLIB_VERSION}: the app is packaged wrongly"
            ));
        }
        with_state(|app| app.set_tdlib_version(version.into()));
        let wanted = with_client(slot, |client| {
            client.version_checked = true;
            std::mem::take(&mut client.parameters_wanted)
        });
        if wanted == Some(true) {
            set_parameters();
        }
    });
}

/// TDLib's first question: where its database is, and who is asking.
fn set_parameters() {
    let (Some(api_id), Some(api_hash)) = (API_ID.and_then(|id| id.trim().parse::<i32>().ok()), API_HASH) else {
        eprintln!("telegram: this build has no api_id: build with FINCHGRAM_API_ID and FINCHGRAM_API_HASH set (README)");
        set_state(context(), TelegramState::MissingApiId);
        return;
    };
    let slot = context();
    let test_dc = use_test_dc();
    let database = match database_directory(test_dc, slot) {
        Ok(database) => database,
        Err(err) => return fail(err),
    };
    let media = match media_directory(test_dc, slot) {
        Ok(media) => media,
        Err(err) => return fail(err),
    };
    let key = database_key(test_dc, slot);
    let parameters = json!({
        "@type": "setTdlibParameters",
        "use_test_dc": test_dc,
        "database_directory": database,
        "files_directory": media,
        "database_encryption_key": "", // open_database fills it in
        "use_file_database": true,
        "use_chat_info_database": true,
        "use_message_database": true,
        "use_secret_chats": true,
        "api_id": api_id,
        "api_hash": api_hash,
        "system_language_code": sys_locale::get_locale().unwrap_or_else(|| "en".to_string()),
        "device_model": crate::platform::device_model(),
        "system_version": "", // detected by TDLib
        "application_version": env!("CARGO_PKG_VERSION"),
    });
    open_database(parameters, database, key);
}

/// The key TDLib encrypts the account's database with: random, made the first time and kept in
/// the Keychain under FinchGram's name (src/platform/, the database folder's name as the account),
/// where only this app reads it. None when the Keychain would not give or keep one: then the
/// database is opened without. The key is never logged.
fn database_key(test_dc: bool, slot: Slot) -> Option<String> {
    let account = database_folder(test_dc, slot);
    match crate::platform::keychain_read(crate::platform::KEYCHAIN_SERVICE, &account) {
        Ok(Some(key)) if !key.is_empty() => return Some(key),
        Ok(_) => {}
        Err(err) => {
            eprintln!("telegram: cannot read the database key from the Keychain: {err}");
            return None;
        }
    }
    let key = crate::platform::random_key()?;
    match crate::platform::keychain_store(crate::platform::KEYCHAIN_SERVICE, &account, &key) {
        Ok(()) => Some(key),
        Err(err) => {
            eprintln!("telegram: cannot keep a database key in the Keychain: {err}");
            None
        }
    }
}

/// setTdlibParameters with the key, or without when the Keychain gave none. A database that does
/// not open with the key is encrypted with another, gone from the Keychain: the app says so, and
/// how to log in again.
fn open_database(mut parameters: Value, database: PathBuf, key: Option<String>) {
    let with_key = key.is_some();
    parameters["database_encryption_key"] = key.unwrap_or_default().into();
    send(parameters, move |answer| match answer {
        Ok(_) => {}
        Err(Error::Telegram { code: 401, .. }) if with_key => fail(format!(
            "The Telegram database is encrypted with a key that is not in the Keychain. To log in again, quit FinchGram, delete the folder {} and open FinchGram.",
            database.display()
        )),
        Err(err @ Error::Telegram { .. }) => fail(format!("TDLib did not accept its parameters: {err}")),
        Err(Error::Stopped) => {}
    });
}

/// The account's finchgram-tdlib has ended: because the app asked it to (quitting, or after a
/// failure), because TDLib closed itself (a log out), or unexpectedly.
fn on_ended() {
    let slot = context();
    let ended = with_client(slot, |client| {
        let status = client.process.take().and_then(Process::reap);
        let waiting: Vec<Answer> = client.waiting.drain().map(|(_, on_answer)| on_answer).collect();
        let next = if QUITTING.get() || client.failed {
            Next::Nothing
        } else if client.removed {
            Next::Remove
        } else if client.closed {
            Next::StartAgain
        } else if client.started.elapsed() >= RESTART_AFTER {
            Next::StartAgainLater
        } else {
            Next::Fail
        };
        (status, waiting, next)
    });
    let Some((status, waiting, next)) = ended else { return };
    let status = status.map_or_else(|| "unknown".to_string(), |status| status.to_string());
    eprintln!("telegram: {} of account {slot} ended ({status})", process::PROGRAM);
    for on_answer in waiting {
        on_answer(Err(Error::Stopped));
    }
    // Downloads the program had under way end with it; the next one is asked again.
    files::forget();
    avatars::forget();
    match next {
        Next::Nothing => {}
        Next::Remove => remove_account(slot),
        Next::StartAgain => launch(slot),
        Next::StartAgainLater => {
            set_state(slot, TelegramState::Starting);
            slint::Timer::single_shot(RESTART_DELAY, move || launch(slot));
        }
        Next::Fail => fail(format!("{} stopped soon after it started ({status})", process::PROGRAM)),
    }
}

enum Next {
    Nothing,
    Remove,
    StartAgain,
    StartAgainLater,
    Fail,
}

/// Something is wrong with the account's finchgram-tdlib that starting again would not fix: show
/// it, and let the program end.
fn fail(message: String) {
    let slot = context();
    eprintln!("telegram: account {slot}: {message}");
    with_client(slot, |client| {
        client.failed = true;
        client.error = message.clone();
        if let Some(process) = &mut client.process {
            process.close_input();
        }
    });
    if slot == shown() {
        with_state(|app| app.set_telegram_error(message.into()));
    }
    set_state(slot, TelegramState::Failed);
}

/// How far the account is: the window shows it while the account is in front.
fn set_state(slot: Slot, state: TelegramState) {
    let changed = with_client(slot, |client| std::mem::replace(&mut client.state, state) != state).unwrap_or(false);
    if slot == shown() {
        with_state(|app| app.set_telegram_state(state));
    }
    if changed {
        accounts::changed();
    }
}

fn set_connection(slot: Slot, connection: Connection) {
    with_client(slot, |client| client.connection = connection);
    if slot == shown() {
        with_state(|app| app.set_connection(connection));
    }
}

/// Change the app's state, when the window still exists.
fn with_state(change: impl FnOnce(&AppState)) {
    with_ui(|ui| change(&ui.global::<AppState>()));
}

/// Whether the window is in front, so that what it shows is being seen. True when that cannot be
/// told (no native window, as in the screenshots).
fn window_is_in_front() -> bool {
    use slint::winit_030::WinitWindowAccessor;
    let mut in_front = true;
    with_ui(|ui| {
        if let Some(focused) = ui.window().with_winit_window(|window| window.has_focus()) {
            in_front = focused;
        }
    });
    in_front
}

/// The window, when it still exists.
fn window() -> Option<MainWindow> {
    WINDOW.with(|window| window.borrow().as_ref().and_then(slint::Weak::upgrade))
}

/// Reach the window, when it still exists.
fn with_ui(change: impl FnOnce(&MainWindow)) {
    if let Some(ui) = window() {
        change(&ui);
    }
}

fn connection(state: ConnectionState) -> Connection {
    match state {
        ConnectionState::WaitingForNetwork => Connection::WaitingForNetwork,
        ConnectionState::ConnectingToProxy => Connection::ConnectingToProxy,
        ConnectionState::Connecting => Connection::Connecting,
        ConnectionState::Updating => Connection::Updating,
        ConnectionState::Ready => Connection::Ready,
    }
}

/// Telegram's test servers instead of the real ones, for development: FINCHGRAM_TEST_DC=1 when
/// starting the app. They have their own accounts, so their own databases.
fn use_test_dc() -> bool {
    std::env::var("FINCHGRAM_TEST_DC").is_ok_and(|value| value == "1")
}

/// The name of the account's database folder (and of its key in the Keychain): `tdlib` for the
/// first slot, `tdlib-2` for the second, …; `tdlib-test`, `tdlib-test-2` on the test servers.
fn database_folder(test_dc: bool, slot: Slot) -> String {
    let base = if test_dc { "tdlib-test" } else { "tdlib" };
    if slot == 1 { base.to_string() } else { format!("{base}-{slot}") }
}

/// The name of the account's media folder: `media_cache`, `media_cache-2`, …
fn media_folder(test_dc: bool, slot: Slot) -> String {
    let base = if test_dc { "media_cache_test" } else { "media_cache" };
    if slot == 1 { base.to_string() } else { format!("{base}-{slot}") }
}

/// Where TDLib keeps the media it downloads (photos, videos, files, stickers, profile photos), in
/// folders of its own under it: in the user's Documents, where they can see it and clear it; what
/// is cleared is fetched again when it is looked at.
fn media_directory(test_dc: bool, slot: Slot) -> Result<PathBuf, String> {
    let documents = dirs::document_dir().ok_or("the system has no Documents folder for the media cache")?;
    let media = documents.join("FinchGram").join(media_folder(test_dc, slot));
    std::fs::create_dir_all(&media).map_err(|err| format!("cannot create {}: {err}", media.display()))?;
    Ok(media)
}

/// TDLib's database, in the platform's data folder (`~/Library/Application Support/FinchGram/tdlib`
/// on macOS for the first slot).
fn database_directory(test_dc: bool, slot: Slot) -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or("the system has no data folder for TDLib's database")?;
    let database = base.join("FinchGram").join(database_folder(test_dc, slot));
    std::fs::create_dir_all(&database).map_err(|err| format!("cannot create {}: {err}", database.display()))?;
    Ok(database)
}

/// One account in front and no finchgram-tdlib, for tests that measure the store (timing.rs).
#[cfg(test)]
pub(super) fn start_alone_for_test(ui: &MainWindow) {
    WINDOW.with(|window| *window.borrow_mut() = Some(ui.as_weak()));
    CLIENTS.with(|clients| *clients.borrow_mut() = vec![Client::new(1, false)]);
    SHOWN.set(1);
    store::add(1);
    store::show(1);
}

// ---- the screenshot tool's side (src/screenshot/) -------------------------------------------

/// Whether the chat in front takes photos: the scissors and their shortcut work only then.
pub fn can_send_photos() -> bool {
    store::with(|store| {
        let chat = store.chats.get(&store.open?)?;
        Some(store.send_rights(chat).photos)
    })
    .flatten()
    .unwrap_or(false)
}

/// A screenshot just taken, into the card before sending, under the title "Send Screenshot".
pub fn add_screenshot(path: std::path::PathBuf) {
    attachments::add_screenshot(path);
}

/// A screenshot copied to the clipboard: say so.
pub fn screenshot_copied() {
    actions::show_notice(crate::ActionNotice::ImageCopied);
}

/// A screenshot saved into the Downloads folder as `name`: say so, with the name.
pub fn screenshot_saved(name: &str) {
    actions::show_saved(name);
}

/// Copy `from` into the Downloads folder as `name`, or as "name (2).png" … when that is taken;
/// the name it got.
pub fn save_to_downloads(from: &std::path::Path, name: &str) -> std::io::Result<String> {
    viewer::save_to_downloads(from, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_follow_the_slot() {
        assert_eq!(database_folder(false, 1), "tdlib");
        assert_eq!(database_folder(false, 2), "tdlib-2");
        assert_eq!(database_folder(true, 1), "tdlib-test");
        assert_eq!(database_folder(true, 3), "tdlib-test-3");
        assert_eq!(media_folder(false, 1), "media_cache");
        assert_eq!(media_folder(false, 10), "media_cache-10");
        assert_eq!(media_folder(true, 2), "media_cache_test-2");
    }
}
