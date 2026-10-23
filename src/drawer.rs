//! The drawer (the design's sixth round): everything beyond the chats in one place, slid out from
//! the logo in Workbench and Broadsheet, a pane under /tools and /accounts in Terminal. Its
//! toolbox lists every tool with a switch that pins it to the bar: the activity bar, the rail
//! and Terminal's bottom bar show only the pinned tools, one list for the three themes
//! (`[toolbox] pinned` in settings.toml; downloads by default). The screenshot is not a tool of
//! the toolbox: its scissors stay under the composer, at the maintainer's word (2026-10-23). The tools the
//! design draws but FinchGram does not have yet (live streams, Jellyfin, keyword hiding,
//! scheduled messages) are listed faded, and cannot be pinned. The accounts part shows the one
//! account; adding another waits for several accounts.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::settings::Settings;
use crate::{AppState, Drawer, DrawerWords, MainWindow, Page, ToolRow};

/// The tools, in the toolbox's order: id, icon, Terminal's mark, and whether FinchGram has it.
const TOOLS: [(&str, &str, &str, bool); 5] = [
    ("downloads", "download-simple", "↓", true),
    ("live", "broadcast", "▶", false),
    ("jellyfin", "television", "■", false),
    ("hiding", "eye-slash", "⊘", false),
    ("scheduled", "clock", "○", false),
];

pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    fill(ui, &settings.borrow().toolbox.pinned);
    let drawer = ui.global::<Drawer>();
    drawer.on_toggle({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                let drawer = ui.global::<Drawer>();
                drawer.set_open(!drawer.get_open());
            }
        }
    });
    drawer.on_close({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.global::<Drawer>().set_open(false);
            }
        }
    });
    drawer.on_pin({
        let ui = ui.as_weak();
        let settings = settings.clone();
        move |id, on| {
            let Some(ui) = ui.upgrade() else { return };
            if !TOOLS.iter().any(|(tool, _, _, built)| *tool == id.as_str() && *built) {
                return;
            }
            {
                let mut settings = settings.borrow_mut();
                settings.toolbox.pinned.retain(|tool| *tool != id.as_str());
                if on {
                    settings.toolbox.pinned.push(id.to_string());
                }
                settings.save();
            }
            fill(&ui, &settings.borrow().toolbox.pinned);
        }
    });
    drawer.on_open_tool({
        let ui = ui.as_weak();
        move |id| {
            let Some(ui) = ui.upgrade() else { return };
            ui.global::<Drawer>().set_open(false);
            if id.as_str() == "downloads" {
                ui.global::<AppState>().set_page(Page::Downloads);
            }
        }
    });
}

/// The tools and the pinned ones, from the setting, in the toolbox's order.
fn fill(ui: &MainWindow, pinned: &[String]) {
    let words = ui.global::<DrawerWords>();
    let tools: Vec<ToolRow> = TOOLS
        .iter()
        .map(|(id, icon, sigil, built)| {
            let (name, line): (SharedString, SharedString) = match *id {
                "downloads" => (words.invoke_downloads(), words.invoke_downloads_line()),
                "live" => (words.invoke_live(), words.invoke_live_line()),
                "jellyfin" => (words.invoke_jellyfin(), words.invoke_jellyfin_line()),
                "hiding" => (words.invoke_hiding(), words.invoke_hiding_line()),
                _ => (words.invoke_scheduled(), words.invoke_scheduled_line()),
            };
            ToolRow {
                id: (*id).into(),
                name,
                line,
                icon: (*icon).into(),
                sigil: (*sigil).into(),
                pinned: *built && pinned.iter().any(|tool| tool == id),
                grey: !built,
            }
        })
        .collect();
    let bar: Vec<ToolRow> = tools.iter().filter(|tool| tool.pinned).cloned().collect();
    let drawer = ui.global::<Drawer>();
    drawer.set_downloads_pinned(bar.iter().any(|tool| tool.id == "downloads"));
    drawer.set_tools(ModelRc::new(VecModel::from(tools)));
    drawer.set_pinned(ModelRc::new(VecModel::from(bar)));
}

/// The UI language changed: the tools' names with it.
pub fn language_changed(ui: &MainWindow, settings: &Settings) {
    fill(ui, &settings.toolbox.pinned);
}
