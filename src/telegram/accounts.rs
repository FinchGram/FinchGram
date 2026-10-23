//! The accounts side by side (the Accounts global): the rows the account menu, the drawer and
//! Broadsheet's rail show, one per account in the order of their slots, with the account's colour
//! and key (its place: ⌘1 … ⌘0), its unread messages and whether it is the one in front; and what
//! the rows ask for: switching, adding an account. mod.rs keeps the accounts themselves.

use std::cell::Cell;

use slint::winit_030::winit::event::{ElementState, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, ModifiersState};
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};

use super::store;
use super::{AccountFacts, Slot};
use crate::{AccountRow, Accounts, AppState, MainWindow, TelegramState, Theme};

thread_local! {
    /// Something a row shows changed since the rows were last built.
    static CHANGED: Cell<bool> = const { Cell::new(false) };
    /// The keyboard's modifiers, as winit reports them, for Terminal's ⌃1 … ⌃0.
    static MODIFIERS: Cell<ModifiersState> = const { Cell::new(ModifiersState::empty()) };
}

/// Terminal's shortcuts: ⌃1 … ⌃0 bring the account in that place to the front (the pane says so;
/// in the other themes the menu bar's ⌘1 … ⌘0 do, in Terminal too).
pub fn window_event(event: &WindowEvent) {
    match event {
        WindowEvent::ModifiersChanged(modifiers) => MODIFIERS.set(modifiers.state()),
        WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
            let modifiers = MODIFIERS.get();
            if !modifiers.control_key() || modifiers.super_key() || modifiers.alt_key() {
                return;
            }
            let Key::Character(text) = &event.logical_key else { return };
            let Some(digit) = text.chars().next().filter(|_| text.len() == 1).and_then(|c| c.to_digit(10)) else { return };
            let terminal = super::window().is_some_and(|ui| ui.global::<AppState>().get_theme() == Theme::Terminal);
            if terminal {
                super::switch_to_index(if digit == 0 { 9 } else { digit as usize - 1 });
            }
        }
        _ => {}
    }
}

pub fn connect(ui: &MainWindow) {
    let accounts = ui.global::<Accounts>();
    accounts.on_switch(|slot| {
        if let Ok(slot) = Slot::try_from(slot) {
            super::switch_to(slot);
        }
    });
    accounts.on_switch_index(|index| {
        if let Ok(index) = usize::try_from(index) {
            super::switch_to_index(index);
        }
    });
    accounts.on_add(super::add_account);
}

/// An account's state, name, photo or unread count changed: the rows are built again after the
/// batch being handled ([`refresh_if_changed`]).
pub fn changed() {
    CHANGED.set(true);
}

pub fn refresh_if_changed() {
    let stores_changed = super::account_facts().iter().any(|facts| store::with_slot(facts.slot, |store| store.dirty.take_accounts()).unwrap_or(false));
    if CHANGED.replace(false) || stores_changed {
        refresh();
    }
}

/// Build the rows anew from every account's store.
pub fn refresh() {
    let Some(ui) = super::window() else { return };
    CHANGED.set(false);
    let count_muted = ui.global::<AppState>().get_count_muted_chats();
    let shown = super::shown();
    let facts = super::account_facts();
    let mut rows = Vec::with_capacity(facts.len());
    let mut others_unread = 0i32;
    let mut index_shown = 0;
    for (index, AccountFacts { slot, state, adding, unread }) in facts.into_iter().enumerate() {
        let (name, sub, initial, picture, has_picture) = store::with_slot(slot, |store| store.account_row()).flatten().unwrap_or_default();
        let unread = if count_muted { unread.0 } else { unread.1 };
        let current = slot == shown;
        if current {
            index_shown = index;
        } else {
            others_unread = others_unread.saturating_add(unread);
        }
        rows.push(AccountRow {
            slot: i32::try_from(slot).unwrap_or(0),
            index: i32::try_from(index).unwrap_or(0),
            name: if name.is_empty() { SharedString::from("…") } else { name.into() },
            sub: sub.into(),
            initial: initial.into(),
            picture,
            has_picture,
            unread,
            current,
            ready: state == TelegramState::Ready && !adding,
        });
    }
    let accounts = ui.global::<Accounts>();
    accounts.set_count(i32::try_from(rows.len()).unwrap_or(0));
    accounts.set_index(i32::try_from(index_shown).unwrap_or(0));
    accounts.set_others_unread(others_unread);
    let others: Vec<AccountRow> = rows.iter().filter(|row| !row.current).cloned().collect();
    accounts.set_others(ModelRc::new(VecModel::from(others)));
    accounts.set_rows(ModelRc::new(VecModel::from(rows)));
    // A row's photo that was not here yet is fetched, as the account's own store asks for it.
    for slot in super::account_facts().into_iter().map(|facts| facts.slot) {
        super::in_context(slot, store::fetch_wanted_photos);
    }
}

/// What a row shows of an account, from its store: name, "+86 138 …" or "@name", initial, photo.
pub(super) type RowFacts = (String, String, String, Image, bool);
