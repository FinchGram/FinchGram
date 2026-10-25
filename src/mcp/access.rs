//! Settings → AI access and the dialog that allows an app: the user's side of the MCP server, on
//! the UI thread. The settings hold what an assistant may do (`settings::Mcp`), the database which
//! programs the user has allowed or turned away (`database::McpApp`, by the key peer.rs gives
//! them); this module shows them in the AiAccess global (ui/ai-access.slint), takes the page's
//! changes, answers the server's questions (may this program connect? is it connected now?) and
//! keeps the page's log of what assistants asked for.
//! The server's threads reach it through `upgrade_in_event_loop` (mod.rs).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use super::peer::Host;
use super::protocol::Admission;
use super::tools::{Activity, ActivityKind};
use crate::database::{self, McpApp};
use crate::settings::{Mcp, McpScope, McpSending, Settings};
use crate::telegram::tools::{chosen_chats, moment, picker_chats};
use crate::{AiAccess, AiActivity, AiActivityKind, AiApp, AiChat, AiScope, AiWriting, MainWindow};

/// How many lines of the log the page shows; the rest is in mcp.log.
const ACTIVITY_SHOWN: usize = 5;
/// "Copied" stays on the button this long.
const COPIED_FOR: Duration = Duration::from_millis(1600);

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// A program the system would not name, allowed or turned away for this run: by "pid N", and
/// only for the process that was, by when it started (peer.rs).
struct SessionApp {
    app: McpApp,
    started: u64,
}

struct State {
    ui: slint::Weak<MainWindow>,
    settings: Rc<RefCell<Settings>>,
    /// Programs the system would not name: their answers last while FinchGram runs, and only
    /// for the process they were given for.
    session_apps: Vec<SessionApp>,
    /// How many connections each program has open now, by the key it is remembered by.
    live: HashMap<String, usize>,
    /// The picker's words.
    query: String,
    /// The log, the newest first.
    activity: Vec<AiActivity>,
    /// Programs waiting for the user's answer, the first asked about in front, each with where its
    /// answer goes.
    asking: VecDeque<(Host, mpsc::Sender<Admission>)>,
    copied: slint::Timer,
}

/// The Keychain account that says the AI access settings have been tagged once: until it is
/// there (the first start with this code), the file has no tag, and is taken as it is.
const POLICY_TAGGED: &str = "mcp-policy";

/// Show the settings on the page and take its changes. Call once, on the UI thread.
pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    verify_policy(&settings);
    STATE.with(|slot| {
        *slot.borrow_mut() = Some(State {
            ui: ui.as_weak(),
            settings,
            session_apps: Vec::new(),
            live: HashMap::new(),
            query: String::new(),
            activity: Vec::new(),
            asking: VecDeque::new(),
            copied: slint::Timer::default(),
        })
    });
    let ai = ui.global::<AiAccess>();
    ai.set_chosen(ModelRc::new(VecModel::<AiChat>::default()));
    ai.set_all_chats(ModelRc::new(VecModel::<AiChat>::default()));
    ai.set_apps(ModelRc::new(VecModel::<AiApp>::default()));
    ai.set_activity(ModelRc::new(VecModel::<AiActivity>::default()));
    let (code, desktop) = commands(&executable());
    ai.set_claude_code_command(code.into());
    ai.set_claude_desktop_config(desktop.into());
    show_policy(ui);
    show_apps(ui);

    ai.on_opened(|| {
        if let Some(ui) = window() {
            show_policy(&ui);
            show_apps(&ui);
        }
    });
    ai.on_set_on(|on| {
        change(|settings| settings.mcp.enabled = on);
        if !on {
            turn_away_waiting();
        }
    });
    ai.on_set_scope(|scope| {
        change(|settings| {
            settings.mcp.scope = match scope {
                AiScope::All => McpScope::All,
                AiScope::Chosen => McpScope::Chosen,
            }
        })
    });
    ai.on_set_writing(|writing| {
        change(|settings| {
            settings.mcp.sending = match writing {
                AiWriting::Messages => McpSending::On,
                AiWriting::Drafts => McpSending::Drafts,
                AiWriting::Nothing => McpSending::Off,
            }
        })
    });
    ai.on_toggle_chat(|id| {
        let Ok(id) = id.parse::<i64>() else { return };
        change(|settings| {
            let chats = &mut settings.mcp.chats;
            match chats.iter().position(|chat| *chat == id) {
                Some(at) => {
                    chats.remove(at);
                }
                None => {
                    chats.push(id);
                    // Choosing one is choosing to choose.
                    settings.mcp.scope = McpScope::Chosen;
                }
            }
        });
    });
    ai.on_search_chats(|query| {
        STATE.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                state.query = query.to_string();
            }
        });
        if let Some(ui) = window() {
            show_policy(&ui);
        }
    });
    ai.on_remove_app(|key| {
        if let Err(err) = database::remove_mcp_app(&key) {
            eprintln!("mcp: cannot forget the program: {err}");
        }
        STATE.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                state.session_apps.retain(|known| known.app.key != key.as_str());
            }
        });
        if let Some(ui) = window() {
            show_apps(&ui);
        }
    });
    ai.on_allow_app(|key| {
        if let Some(mut app) = database::mcp_app(&key) {
            app.allowed = true;
            if let Err(err) = database::save_mcp_app(&app) {
                eprintln!("mcp: cannot allow the program: {err}");
            }
        }
        STATE.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut()
                && let Some(known) = state.session_apps.iter_mut().find(|known| known.app.key == key.as_str())
            {
                known.app.allowed = true;
            }
        });
        if let Some(ui) = window() {
            show_apps(&ui);
        }
    });
    ai.on_copy(|which| copy(&which));
    ai.on_show_log(show_log);
    ai.on_answer(answer);
}

/// The AI access settings as settings.toml has them, held to their tag (`Mcp::seal`, under the
/// key only this app has): a file changed by anything but FinchGram, say a program allowed for
/// some chats and drafts widening its own reach, is not believed, and the settings go back to
/// their defaults (the server off; the user sets them again on the page). The first start with
/// this code, before the Keychain says the settings were tagged once, takes the file as it is
/// and tags it. While the database is not open there is no key to check with, and nothing can be
/// allowed anyway.
fn verify_policy(settings: &Rc<RefCell<Settings>>) {
    if !database::is_open() {
        return;
    }
    match crate::platform::keychain_read(crate::platform::KEYCHAIN_SERVICE, POLICY_TAGGED) {
        Ok(Some(_)) => {}
        Ok(None) => {
            settings.borrow().save();
            if let Err(err) = crate::platform::keychain_store(crate::platform::KEYCHAIN_SERVICE, POLICY_TAGGED, "1") {
                eprintln!("mcp: cannot note in the Keychain that the AI access settings are tagged: {err}");
            }
            return;
        }
        Err(err) => {
            eprintln!("mcp: cannot ask the Keychain whether the AI access settings are tagged: {err}");
            return;
        }
    }
    let (text, seal) = {
        let settings = settings.borrow();
        (settings.mcp.sealed_text(), settings.mcp.seal.clone())
    };
    // Settings never touched (nothing on, no tag) are nothing to doubt.
    if database::tag_matches(&text, &seal) || (seal.is_empty() && text == Mcp::default().sealed_text()) {
        return;
    }
    eprintln!("mcp: the AI access settings in settings.toml were not written by FinchGram, or were changed since: back to the defaults");
    super::log("the AI access settings in settings.toml were not FinchGram's as it wrote them: back to the defaults, the server off");
    settings.borrow_mut().mcp = Mcp::default();
    settings.borrow().save();
}

// ---- the server's questions ----------------------------------------------------------------------

/// May the program at `host` use the account? Said through `answer`, which the connection's thread
/// waits on: at once when the user has answered about this program before, else once they have
/// answered the dialog. UI thread.
pub fn admit(host: Host, answer: mpsc::Sender<Admission>) {
    let decided = STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let state = slot.as_mut()?;
        if !state.settings.borrow().mcp.enabled {
            return Some(Admission::Off);
        }
        let key = host.key();
        let now = now();
        // A program the system could not place at all, not even its process: nothing to ask the
        // user about, and nothing to remember an answer by.
        if !host.identified() && host.pid == 0 {
            super::log("a program the system could not identify at all: turned away");
            return Some(Admission::Denied);
        }
        if let Some(mut app) = database::mcp_app(&key) {
            // Where it is now: a newer version, or another copy, of the same program.
            app.path = host.location();
            app.name = host.name();
            app.last_used = now;
            if let Err(err) = database::save_mcp_app(&app) {
                eprintln!("mcp: cannot note the program's use: {err}");
            }
            return Some(if app.allowed { Admission::Allowed } else { Admission::Denied });
        }
        if let Some(known) = state.session_apps.iter_mut().find(|known| known.app.key == key && known.started == host.started) {
            known.app.last_used = now;
            return Some(if known.app.allowed { Admission::Allowed } else { Admission::Denied });
        }
        state.asking.push_back((host, answer.clone()));
        None
    });
    match decided {
        Some(admission) => {
            let _ = answer.send(admission);
        }
        None => {
            super::log("asking the user about a program that has not connected before");
            show_question();
        }
    }
    if let Some(ui) = window() {
        show_apps(&ui);
    }
}

/// The account with `id` is signed in (src/telegram/account.rs): the settings are bound to it.
/// When it is another account than the one the allowed programs and the chosen chats were for,
/// they are gone (`settings::Mcp::bind_to_account`), this run's answers with them: every program
/// asks again, for this account, and its old connections are turned away from here on (their
/// calls ask `admitted`). UI thread.
pub fn signed_in(id: i64) {
    let dropped = STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let state = slot.as_mut()?;
        let (before, dropped) = {
            let mut settings = state.settings.borrow_mut();
            let before = settings.mcp.account;
            (before, settings.mcp.bind_to_account(id))
        };
        if dropped {
            state.session_apps.clear();
            if let Err(err) = database::clear_mcp_apps() {
                eprintln!("mcp: cannot forget the programs of the account before: {err}");
            }
        }
        if before != state.settings.borrow().mcp.account {
            state.settings.borrow().save();
        }
        Some(dropped)
    });
    if dropped == Some(true) {
        super::log("another Telegram account signed in: the programs allowed for the one before ask again");
        if let Some(ui) = window() {
            show_policy(&ui);
            show_apps(&ui);
        }
    }
}

/// The Telegram account the settings are for (`settings::Mcp::account`); 0 until one signed in.
pub fn bound_account() -> i64 {
    STATE.with(|slot| slot.borrow().as_ref().map_or(0, |state| state.settings.borrow().mcp.account))
}

/// The user's standing answer about `host`, without asking: allowed, and the server on.
pub fn admitted(host: &Host) -> bool {
    STATE.with(|slot| {
        let slot = slot.borrow();
        let Some(state) = slot.as_ref() else { return false };
        let settings = state.settings.borrow();
        let key = host.key();
        settings.mcp.enabled
            && database::mcp_app(&key)
                .or_else(|| state.session_apps.iter().find(|known| known.app.key == key && known.started == host.started).map(|known| known.app.clone()))
                .is_some_and(|app| app.allowed)
    })
}

/// `host` is connected, allowed: the page says so until `disconnected`.
pub fn connected(host: &Host) {
    STATE.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            *state.live.entry(host.key()).or_insert(0) += 1;
        }
    });
    if let Some(ui) = window() {
        show_apps(&ui);
    }
}

pub fn disconnected(host: &Host) {
    STATE.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            let key = host.key();
            let left = state.live.get(&key).copied().unwrap_or(0).saturating_sub(1);
            if left == 0 {
                state.live.remove(&key);
            } else {
                state.live.insert(key, left);
            }
        }
    });
    if let Some(ui) = window() {
        show_apps(&ui);
    }
}

/// What `host` asked for: the newest line of the page's log, and its last use now.
pub fn record(host: &Host, activity: Activity) {
    let Some(ui) = window() else { return };
    let now = now();
    let entry = AiActivity {
        kind: kind_shown(activity.kind),
        app: host.name().into(),
        time: moment(now),
        chat: activity.chat.into(),
        count: i32::try_from(activity.count).unwrap_or(i32::MAX),
    };
    let rows = STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let state = slot.as_mut()?;
        let key = host.key();
        if let Some(mut app) = database::mcp_app(&key) {
            app.last_used = now;
            if let Err(err) = database::save_mcp_app(&app) {
                eprintln!("mcp: cannot note the program's use: {err}");
            }
        }
        if let Some(known) = state.session_apps.iter_mut().find(|known| known.app.key == key && known.started == host.started) {
            known.app.last_used = now;
        }
        state.activity.insert(0, entry);
        state.activity.truncate(ACTIVITY_SHOWN);
        Some(state.activity.clone())
    });
    if let Some(rows) = rows {
        replace(&ui.global::<AiAccess>().get_activity(), rows);
    }
    show_apps(&ui);
}

fn kind_shown(kind: ActivityKind) -> AiActivityKind {
    match kind {
        ActivityKind::Connected => AiActivityKind::Connected,
        ActivityKind::TurnedAway => AiActivityKind::TurnedAway,
        ActivityKind::Listed => AiActivityKind::Listed,
        ActivityKind::SearchedChats => AiActivityKind::SearchedChats,
        ActivityKind::Looked => AiActivityKind::Looked,
        ActivityKind::Read => AiActivityKind::Read,
        ActivityKind::Searched => AiActivityKind::Searched,
        ActivityKind::SearchedAll => AiActivityKind::SearchedAll,
        ActivityKind::Unread => AiActivityKind::Unread,
        ActivityKind::Media => AiActivityKind::Media,
        ActivityKind::Draft => AiActivityKind::Draft,
        ActivityKind::Sent => AiActivityKind::Sent,
        ActivityKind::Deleted => AiActivityKind::Deleted,
        ActivityKind::Marked => AiActivityKind::Marked,
        ActivityKind::Account => AiActivityKind::Account,
        ActivityKind::Refused => AiActivityKind::Refused,
    }
}

// ---- the dialog ------------------------------------------------------------------------------------

/// Ask about the program in front of the queue, unless the dialog is open already. The window
/// comes to the front: the bridge may have started FinchGram in the background.
fn show_question() {
    let Some(ui) = window() else { return };
    let ai = ui.global::<AiAccess>();
    if ai.get_ask_open() {
        return;
    }
    let next = STATE.with(|slot| slot.borrow().as_ref().and_then(|state| state.asking.front().map(|(host, _)| host.clone())));
    let Some(host) = next else { return };
    let name = host.name();
    ai.set_ask_initial(initial(&name).into());
    ai.set_ask_name(name.into());
    ai.set_ask_path(host.location().into());
    ai.set_ask_open(true);
    crate::platform::show_window(&ui);
}

/// The user's answer about the program in front of the queue: remembered, told to the waiting
/// connection, and the next question, if any.
fn answer(allow: bool) {
    let Some(ui) = window() else { return };
    ui.global::<AiAccess>().set_ask_open(false);
    let asked = STATE.with(|slot| slot.borrow_mut().as_mut().and_then(|state| state.asking.pop_front()));
    if let Some((host, sender)) = asked {
        remember(&host, allow);
        super::log(&format!("{}: {}", host.describe(), if allow { "allowed by the user" } else { "turned away by the user" }));
        let _ = sender.send(if allow { Admission::Allowed } else { Admission::Denied });
    }
    show_apps(&ui);
    show_question();
}

/// The server was switched off while programs waited: they are turned away, without an answer
/// being remembered.
fn turn_away_waiting() {
    let waiting = STATE.with(|slot| slot.borrow_mut().as_mut().map(|state| std::mem::take(&mut state.asking)).unwrap_or_default());
    for (_, sender) in waiting {
        let _ = sender.send(Admission::Off);
    }
    if let Some(ui) = window() {
        ui.global::<AiAccess>().set_ask_open(false);
    }
}

/// Keep the answer: in the database for a program the system named, else for this run only.
fn remember(host: &Host, allowed: bool) {
    let app = McpApp { key: host.key(), path: host.location(), name: host.name(), allowed, last_used: now() };
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(state) = slot.as_mut() else { return };
        if host.identified() {
            if let Err(err) = database::save_mcp_app(&app) {
                eprintln!("mcp: cannot keep the answer about the program: {err}");
            }
        } else {
            state.session_apps.retain(|known| known.app.key != app.key);
            state.session_apps.push(SessionApp { app, started: host.started });
        }
    });
}

// ---- the page --------------------------------------------------------------------------------------

/// Change the settings, save them, and show them again.
fn change(apply: impl FnOnce(&mut Settings)) {
    let Some(settings) = STATE.with(|slot| slot.borrow().as_ref().map(|state| state.settings.clone())) else { return };
    {
        let mut settings = settings.borrow_mut();
        apply(&mut settings);
        settings.save();
    }
    if let Some(ui) = window() {
        show_policy(&ui);
    }
}

/// The settings as the page shows them: the switch, which chats (and the chosen ones by name), the
/// writing choice, and the picker's chats.
fn show_policy(ui: &MainWindow) {
    let Some((mcp, query)) = STATE.with(|slot| slot.borrow().as_ref().map(|state| (state.settings.borrow().mcp.clone(), state.query.clone()))) else {
        return;
    };
    let ai = ui.global::<AiAccess>();
    ai.set_on(mcp.enabled);
    ai.set_scope(match mcp.scope {
        McpScope::All => AiScope::All,
        McpScope::Chosen => AiScope::Chosen,
    });
    ai.set_writing(match mcp.sending {
        McpSending::On => AiWriting::Messages,
        McpSending::Drafts => AiWriting::Drafts,
        McpSending::Off => AiWriting::Nothing,
    });
    ai.set_chosen_count(i32::try_from(mcp.chats.len()).unwrap_or(i32::MAX));
    // The chats are the account's the settings are for, whichever account is in front; none
    // while it is not signed in here.
    replace(&ai.get_chosen(), crate::telegram::in_account(mcp.account, || chosen_chats(ui, &mcp.chats)).unwrap_or_default());
    replace(&ai.get_all_chats(), crate::telegram::in_account(mcp.account, || picker_chats(ui, &query, &mcp.chats)).unwrap_or_default());
}

/// The programs that have connected, the remembered ones first, then this run's unnamed ones.
fn show_apps(ui: &MainWindow) {
    let kept = database::mcp_apps();
    let rows: Vec<AiApp> = STATE
        .with(|slot| {
            let slot = slot.borrow();
            let state = slot.as_ref()?;
            Some(
                kept.iter()
                    .chain(state.session_apps.iter().map(|known| &known.app))
                    .map(|app| AiApp {
                        key: app.key.clone().into(),
                        name: app.name.clone().into(),
                        path: app.path.clone().into(),
                        allowed: app.allowed,
                        live: state.live.get(&app.key).is_some_and(|count| *count > 0),
                        has_last_used: app.last_used > 0,
                        last_used: moment(app.last_used),
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();
    replace(&ui.global::<AiAccess>().get_apps(), rows);
}

fn copy(which: &str) {
    let Some(ui) = window() else { return };
    let ai = ui.global::<AiAccess>();
    let text = match which {
        "code" => ai.get_claude_code_command(),
        "desktop" => ai.get_claude_desktop_config(),
        _ => return,
    };
    if let Err(err) = crate::platform::copy_text(&text) {
        eprintln!("mcp: cannot copy: {err}");
        return;
    }
    ai.set_copied(which.into());
    STATE.with(|slot| {
        if let Some(state) = slot.borrow().as_ref() {
            state.copied.start(slint::TimerMode::SingleShot, COPIED_FOR, || {
                if let Some(ui) = window() {
                    ui.global::<AiAccess>().set_copied(Default::default());
                }
            });
        }
    });
}

/// The log file in the Finder; there is one once anything has been logged, which listening was.
fn show_log() {
    let Some(path) = super::log_path() else { return };
    if !path.exists() {
        super::log("log opened");
    }
    crate::platform::reveal_file(&path.to_string_lossy());
}

// ---- small things ----------------------------------------------------------------------------------

fn window() -> Option<MainWindow> {
    STATE.with(|slot| slot.borrow().as_ref().and_then(|state| state.ui.upgrade()))
}

/// Replace the rows of a model the page holds.
fn replace<T: Clone + 'static>(model: &ModelRc<T>, rows: Vec<T>) {
    if let Some(model) = model.as_any().downcast_ref::<VecModel<T>>() {
        model.set_vec(rows);
    }
}

fn now() -> i64 {
    chrono::Local::now().timestamp()
}

/// The name's first letter, for the dialog's square.
fn initial(name: &str) -> String {
    name.chars().next().map(|first| first.to_uppercase().collect()).unwrap_or_default()
}

/// FinchGram's own executable, as an assistant's app would run it: the bridge is this program
/// with the word `mcp`.
fn executable() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .map(|exe| exe.display().to_string())
        .unwrap_or_else(|| "finchgram".to_string())
}

/// What to paste to connect Claude Code (a shell line) and Claude Desktop (its JSON).
fn commands(exe: &str) -> (String, String) {
    let needs_quotes = exe.chars().any(|c| c.is_whitespace() || "'\"\\$`&;|<>()".contains(c));
    let quoted = if needs_quotes { format!("'{}'", exe.replace('\'', "'\\''")) } else { exe.to_string() };
    (
        format!("claude mcp add finchgram -- {quoted} mcp"),
        format!("{{ \"mcpServers\": {{ \"finchgram\": {{ \"command\": {}, \"args\": [\"mcp\"] }} }} }}", serde_json::to_string(exe).unwrap_or_default()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_name_the_executable_and_quote_it_when_they_must() {
        let (code, desktop) = commands("/Applications/FinchGram.app/Contents/MacOS/finchgram");
        assert_eq!(code, "claude mcp add finchgram -- /Applications/FinchGram.app/Contents/MacOS/finchgram mcp");
        assert_eq!(desktop, r#"{ "mcpServers": { "finchgram": { "command": "/Applications/FinchGram.app/Contents/MacOS/finchgram", "args": ["mcp"] } } }"#);
        let (code, desktop) = commands("/Users/x/My Apps/FinchGram.app/Contents/MacOS/finchgram");
        assert_eq!(code, "claude mcp add finchgram -- '/Users/x/My Apps/FinchGram.app/Contents/MacOS/finchgram' mcp");
        assert!(desktop.contains(r#""command": "/Users/x/My Apps/FinchGram.app/Contents/MacOS/finchgram""#));
        let (code, _) = commands("/Users/x/it's here/finchgram");
        assert_eq!(code, "claude mcp add finchgram -- '/Users/x/it'\\''s here/finchgram' mcp");
    }

    #[test]
    fn the_initial_is_the_first_letter_in_capitals() {
        assert_eq!(initial("claude"), "C");
        assert_eq!(initial("Zed"), "Z");
        assert_eq!(initial("ß"), "SS");
        assert_eq!(initial(""), "");
    }
}
