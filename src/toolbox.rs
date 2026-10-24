//! The toolbox (the design's seventh round, "百宝箱"): everything beyond the chats in one place,
//! slid out from the logo in Workbench and Broadsheet, the pane's place under /tools in Terminal.
//! Its categories list every tool with a switch that pins it to the bar: the activity bar, the
//! rail and Terminal's status bar show only the pinned tools, one list for the three themes
//! (`[toolbox] pinned` in settings.toml; downloads by default). Downloads is a family: the
//! protocols a server can speak are listed under it; a protocol is a server's property, chosen
//! when a server is added (src/downloads/), never a tool of its own. The tools the design draws
//! but FinchGram does not have yet (live streams, Jellyfin, keyword hiding, scheduled messages;
//! FTP, FTPS and SFTP) are listed faded, and cannot be pinned. The screenshot is not a tool of the
//! toolbox: its scissors stay under the composer, at the maintainer's word (2026-10-23).

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use crate::settings::Settings;
use crate::{AccountRow, Accounts, AppState, MainWindow, NavRow, Page, ProtocolRow, SettingsSection, ToolRow, ToolSection, Toolbox, ToolboxWords};

/// A tool: its id, its category, its Phosphor icon, Terminal's mark, its shortcut, whether
/// FinchGram has it, and whether it is a family (the protocols are listed under it).
struct Tool {
    id: &'static str,
    category: &'static str,
    icon: &'static str,
    sigil: &'static str,
    key: &'static str,
    built: bool,
    family: bool,
}

/// The tools, in the toolbox's order.
const TOOLS: [Tool; 6] = [
    Tool { id: "downloads", category: "transfers", icon: "download-simple", sigil: "↓", key: "⌥⌘L", built: true, family: true },
    Tool { id: "live", category: "media", icon: "broadcast", sigil: "◉", key: "", built: false, family: false },
    Tool { id: "jellyfin", category: "media", icon: "film-reel", sigil: "▷", key: "", built: false, family: false },
    Tool { id: "hiding", category: "messages", icon: "funnel-simple", sigil: "⊘", key: "", built: false, family: false },
    Tool { id: "scheduled", category: "messages", icon: "clock", sigil: "○", key: "", built: false, family: false },
    Tool { id: "ai-access", category: "connections", icon: "plugs-connected", sigil: "◆", key: "", built: true, family: false },
];

/// The categories, in the column's order: id, icon, Broadsheet's letter.
const CATEGORIES: [(&str, &str, &str); 5] = [
    ("accounts", "users-three", "A"),
    ("transfers", "arrows-down-up", "B"),
    ("media", "film-strip", "C"),
    ("messages", "chat-text", "D"),
    ("connections", "plugs-connected", "E"),
];

/// The protocols of the downloads family: id, name, default port, whether FinchGram speaks it.
const PROTOCOLS: [(&str, &str, &str, bool); 4] = [
    ("webdav", "WebDAV", "80 / 443", true),
    ("ftp", "FTP", "21", false),
    ("ftps", "FTPS", "990", false),
    ("sftp", "SFTP", "22", false),
];

/// What the toolbox shows: the category chosen in the column and the words in the search box.
struct View {
    ui: Weak<MainWindow>,
    settings: Rc<RefCell<Settings>>,
    category: String,
    query: String,
    /// The first tool shown that FinchGram has: ⏎ in the search box opens it.
    first: Option<String>,
}

thread_local! {
    static VIEW: RefCell<Option<View>> = const { RefCell::new(None) };
}

pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    VIEW.with(|view| {
        *view.borrow_mut() = Some(View { ui: ui.as_weak(), settings, category: "all".to_string(), query: String::new(), first: None });
    });
    refresh();
    let toolbox = ui.global::<Toolbox>();
    toolbox.on_toggle({
        let ui = ui.as_weak();
        move || {
            let Some(ui) = ui.upgrade() else { return };
            if ui.global::<Toolbox>().get_open() {
                ui.global::<Toolbox>().set_open(false);
            } else {
                open(&ui);
            }
        }
    });
    toolbox.on_close({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.global::<Toolbox>().set_open(false);
            }
        }
    });
    toolbox.on_show({
        let ui = ui.as_weak();
        move |category| {
            let Some(ui) = ui.upgrade() else { return };
            set_view(&category, "");
            open(&ui);
        }
    });
    toolbox.on_pin(move |id, on| {
        if !TOOLS.iter().any(|tool| tool.id == id.as_str() && tool.built) {
            return;
        }
        let settings = VIEW.with(|view| view.borrow().as_ref().map(|view| view.settings.clone()));
        if let Some(settings) = settings {
            let mut settings = settings.borrow_mut();
            settings.toolbox.pinned.retain(|tool| *tool != id.as_str());
            if on {
                settings.toolbox.pinned.push(id.to_string());
            }
            settings.save();
        }
        refresh();
    });
    toolbox.on_open_tool({
        let ui = ui.as_weak();
        move |id| {
            if let Some(ui) = ui.upgrade() {
                open_tool(&ui, &id);
            }
        }
    });
    toolbox.on_choose(move |category| {
        set_view(&category, "");
    });
    toolbox.on_search(move |query| {
        VIEW.with(|view| {
            if let Some(view) = view.borrow_mut().as_mut() {
                view.query = query.to_string();
            }
        });
        refresh();
    });
    toolbox.on_open_first({
        let ui = ui.as_weak();
        move || {
            let first = VIEW.with(|view| view.borrow().as_ref().and_then(|view| view.first.clone()));
            if let (Some(ui), Some(first)) = (ui.upgrade(), first) {
                open_tool(&ui, &first);
            }
        }
    });
    toolbox.on_command({
        let ui = ui.as_weak();
        move |words| {
            let Some(ui) = ui.upgrade() else { return false };
            let Some(category) = category_named(&ui, &words) else { return false };
            let toolbox = ui.global::<Toolbox>();
            let shown = VIEW.with(|view| view.borrow().as_ref().map(|view| view.category == category && view.query.is_empty()).unwrap_or(false));
            if toolbox.get_open() && shown {
                toolbox.set_open(false);
            } else {
                set_view(&category, "");
                open(&ui);
            }
            true
        }
    });
}

/// The UI language changed: the words with it.
pub fn language_changed() {
    refresh();
}

/// The accounts signed in changed (src/telegram/accounts.rs): the accounts section with them.
pub fn accounts_changed() {
    refresh();
}

/// The toolbox as the picture test draws it (src/screenshots.rs): these tools pinned, this
/// category chosen, these words in the box.
#[cfg(test)]
pub fn show_for_pictures(ui: &MainWindow, pinned: &[&str], category: &str, query: &str) {
    let pinned: Vec<String> = pinned.iter().map(|tool| tool.to_string()).collect();
    fill(ui, &pinned, category, query);
}

fn open(ui: &MainWindow) {
    let toolbox = ui.global::<Toolbox>();
    toolbox.set_open(true);
    toolbox.set_opened(toolbox.get_opened() + 1);
}

/// The toolbox shuts and the tool's page comes.
fn open_tool(ui: &MainWindow, id: &str) {
    ui.global::<Toolbox>().set_open(false);
    let app = ui.global::<AppState>();
    match id {
        "downloads" => app.set_page(Page::Downloads),
        "ai-access" => {
            app.set_settings_section(SettingsSection::Ai);
            app.set_page(Page::Settings);
        }
        _ => {}
    }
}

/// The column's choice and the box's words, then everything drawn again.
fn set_view(category: &str, query: &str) {
    VIEW.with(|view| {
        if let Some(view) = view.borrow_mut().as_mut() {
            view.category = category.to_string();
            view.query = query.to_string();
        }
    });
    refresh();
}

/// "/tools 传输", "/tools transfers", "/tools": a category by its id, its English name or its name
/// in the UI language; "all" for none. None for words that name no category.
fn category_named(ui: &MainWindow, words: &str) -> Option<String> {
    let words = words.trim().to_lowercase();
    if words.is_empty() {
        return Some("all".to_string());
    }
    let names = ui.global::<ToolboxWords>();
    if words == "all" || words == names.invoke_all().to_lowercase() {
        return Some("all".to_string());
    }
    CATEGORIES
        .iter()
        .find(|(id, _, _)| words == *id || words == label_of(&names, id).to_lowercase())
        .map(|(id, _, _)| id.to_string())
}

fn label_of(words: &ToolboxWords, category: &str) -> SharedString {
    match category {
        "accounts" => words.invoke_accounts(),
        "transfers" => words.invoke_transfers(),
        "media" => words.invoke_media(),
        "messages" => words.invoke_messages(),
        _ => words.invoke_connections(),
    }
}

fn refresh() {
    let state = VIEW.with(|view| {
        view.borrow().as_ref().map(|view| (view.ui.clone(), view.settings.borrow().toolbox.pinned.clone(), view.category.clone(), view.query.clone()))
    });
    let Some((ui, pinned, category, query)) = state else { return };
    let Some(ui) = ui.upgrade() else { return };
    fill(&ui, &pinned, &category, &query);
}

/// The column, the sections and the bars, from the tools, the accounts, the category chosen and
/// the words in the box.
fn fill(ui: &MainWindow, pinned: &[String], category: &str, query: &str) {
    let words = ui.global::<ToolboxWords>();
    let needle = query.trim().to_lowercase();
    let searching = !needle.is_empty();
    let matches = |texts: &[&str]| !searching || texts.iter().any(|text| text.to_lowercase().contains(&needle));

    let tools: Vec<(ToolRow, bool)> = TOOLS
        .iter()
        .map(|tool| {
            let (name, line): (SharedString, SharedString) = match tool.id {
                "downloads" => (words.invoke_downloads(), words.invoke_downloads_line()),
                "live" => (words.invoke_live(), words.invoke_live_line()),
                "jellyfin" => (words.invoke_jellyfin(), words.invoke_jellyfin_line()),
                "hiding" => (words.invoke_hiding(), words.invoke_hiding_line()),
                "scheduled" => (words.invoke_scheduled(), words.invoke_scheduled_line()),
                _ => (words.invoke_ai_access(), words.invoke_ai_access_line()),
            };
            let mut texts: Vec<&str> = vec![&name, &line, tool.id];
            if tool.family {
                texts.extend(PROTOCOLS.iter().map(|(_, protocol, _, _)| *protocol));
            }
            let hit = matches(&texts);
            let row = ToolRow {
                id: tool.id.into(),
                category: tool.category.into(),
                name: name.clone(),
                line: line.clone(),
                icon: tool.icon.into(),
                sigil: tool.sigil.into(),
                key: tool.key.into(),
                pinned: tool.built && pinned.iter().any(|id| id == tool.id),
                grey: !tool.built,
                family: tool.family,
                slot: 0,
            };
            (row, hit)
        })
        .collect();

    let accounts = ui.global::<Accounts>();
    let rows: Vec<AccountRow> = accounts.get_rows().iter().filter(|row| matches(&[&row.name, &row.sub])).collect();
    let signed_in = accounts.get_count();

    let mut nav = Vec::new();
    let mut sections = Vec::new();
    let mut hits = 0;
    let mut first = None;
    for (id, icon, letter) in CATEGORIES {
        let label = label_of(&words, id);
        let shown_by_choice = category == "all" || category == id;
        if id == "accounts" {
            let found = rows.len() as i32;
            if searching {
                hits += found;
            }
            let shown = if searching { found > 0 } else { shown_by_choice && signed_in > 0 };
            nav.push(NavRow { id: id.into(), label: label.clone(), letter: letter.into(), icon: icon.into(), count: if searching { found } else { signed_in }, accounts: true });
            if shown {
                sections.push(ToolSection { id: id.into(), label, letter: letter.into(), icon: icon.into(), accounts: true, tools: ModelRc::default(), families: 0, others: 0 });
            }
            continue;
        }
        let all = tools.iter().filter(|(tool, _)| tool.category == id).count() as i32;
        let mut its: Vec<ToolRow> = tools.iter().filter(|(tool, hit)| tool.category == id && (!searching || *hit)).map(|(tool, _)| tool.clone()).collect();
        let mut families = 0;
        let mut others = 0;
        for tool in its.iter_mut() {
            if tool.family {
                families += 1;
            } else {
                tool.slot = others;
                others += 1;
            }
        }
        let found = its.len() as i32;
        if searching {
            hits += found;
        }
        let shown = if searching { found > 0 } else { shown_by_choice };
        nav.push(NavRow { id: id.into(), label: label.clone(), letter: letter.into(), icon: icon.into(), count: if searching { found } else { all }, accounts: false });
        if shown {
            if first.is_none() {
                first = its.iter().find(|tool| !tool.grey).map(|tool| tool.id.to_string());
            }
            sections.push(ToolSection { id: id.into(), label, letter: letter.into(), icon: icon.into(), accounts: false, tools: ModelRc::new(VecModel::from(its)), families, others });
        }
    }
    let everything = if searching { hits } else { tools.len() as i32 + i32::from(signed_in > 0) };
    nav.insert(0, NavRow { id: "all".into(), label: words.invoke_all(), letter: "·".into(), icon: "squares-four".into(), count: everything, accounts: false });

    let bar: Vec<ToolRow> = tools.iter().filter(|(tool, _)| tool.pinned).map(|(tool, _)| tool.clone()).collect();
    let protocols: Vec<ProtocolRow> = PROTOCOLS
        .iter()
        .map(|(id, name, port, built)| {
            let line = match *id {
                "webdav" => words.invoke_webdav_line(),
                "ftp" => words.invoke_ftp_line(),
                "ftps" => words.invoke_ftps_line(),
                _ => words.invoke_sftp_line(),
            };
            ProtocolRow { id: (*id).into(), name: (*name).into(), port: (*port).into(), line, grey: !built }
        })
        .collect();

    VIEW.with(|view| {
        if let Some(view) = view.borrow_mut().as_mut() {
            view.first = first;
        }
    });
    let toolbox = ui.global::<Toolbox>();
    toolbox.set_category(category.into());
    toolbox.set_query(query.into());
    toolbox.set_hits(hits);
    toolbox.set_downloads_pinned(bar.iter().any(|tool| tool.id == "downloads"));
    toolbox.set_nav(ModelRc::new(VecModel::from(nav)));
    toolbox.set_sections(ModelRc::new(VecModel::from(sections)));
    toolbox.set_pinned(ModelRc::new(VecModel::from(bar)));
    toolbox.set_accounts(ModelRc::new(VecModel::from(rows)));
    toolbox.set_protocols(ModelRc::new(VecModel::from(protocols)));
}
