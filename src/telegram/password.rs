//! Two-step verification: Settings → Privacy & security → Two-step verification
//! (ui/pages/settings.slint, whose forms are in ui/pages/password.slint), and the checks of a new
//! password that logging in shares when a forgotten one is recovered (login.rs).
//!
//! A change asks for the current password first. TDLib checks it by giving the recovery email
//! address for it (getRecoveryEmailAddress), and it is kept here until the change is made or
//! cancelled. A forgotten password is recovered with a code sent to the recovery email address;
//! without one it can be reset, which Telegram completes only a week after it was asked for.

use std::cell::RefCell;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use slint::{ComponentHandle, SharedString};

use super::api::{EmailAddressAuthenticationCodeInfo, PasswordState, RecoveryEmailAddress, ResetPasswordResult};
use super::{Error, send, time, with_ui};
use crate::{AppState, MainWindow, PasswordChange, PasswordSettings, PasswordStep, SettingsSection};

#[derive(Default)]
struct State {
    has_password: bool,
    has_recovery_email: bool,
    /// The current password, as entered on the verify step, while a change that needs it goes on.
    password: String,
    /// Where the verified password leads.
    target: Option<PasswordStep>,
    /// The recovery code TDLib accepted, for the new password.
    recovery_code: String,
    /// A new recovery email address, waiting for the code sent to it.
    pending_email: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn connect(ui: &MainWindow) {
    let settings = ui.global::<PasswordSettings>();
    settings.on_load(load);
    settings.on_choose(choose);
    settings.on_verify(|password| verify(password.to_string()));
    settings.on_forgot(forgot);
    settings.on_submit_recovery_code(|code| submit_recovery_code(digits(&code)));
    settings.on_request_reset(request_reset);
    settings.on_cancel_reset(cancel_reset);
    settings.on_submit_new_password(|password, again, hint| submit_new_password(&password, &again, &hint));
    settings.on_submit_hint(|hint| submit_hint(hint.trim().to_string()));
    settings.on_submit_recovery_email(|address| submit_recovery_email(address.trim().to_string()));
    settings.on_submit_recovery_email_code(|code| submit_recovery_email_code(digits(&code)));
    settings.on_turn_off(turn_off);
    settings.on_cancel(cancel);
}

/// What is wrong with a new password, typed twice, and its hint: one of the words the pages know
/// (ui/pages/password.slint).
pub fn check_new_password(password: &str, again: &str, hint: &str) -> Result<(), &'static str> {
    if password.is_empty() {
        return Err("NEW_PASSWORD_EMPTY");
    }
    if password != again {
        return Err("NEW_PASSWORD_MISMATCH");
    }
    if hint.trim() == password {
        return Err("NEW_PASSWORD_HINT_SAME");
    }
    Ok(())
}

/// Logged out: nothing of the account's password is known any more.
pub fn forget() {
    STATE.with(|state| *state.borrow_mut() = State::default());
    with_ui(|ui| {
        let settings = ui.global::<PasswordSettings>();
        settings.set_open(false);
        settings.set_loaded(false);
        settings.set_has_password(false);
        settings.set_hint(SharedString::new());
        settings.set_has_recovery_email(false);
        settings.set_recovery_email(SharedString::new());
        settings.set_code_sent_to(SharedString::new());
        settings.set_reset_pending(false);
        settings.set_step(PasswordStep::Overview);
        settings.set_busy(false);
        settings.set_error(SharedString::new());
        settings.set_done(PasswordChange::Nothing);
        // Privacy & security is the account's: without one, Settings opens elsewhere.
        let app = ui.global::<AppState>();
        if app.get_settings_section() == SettingsSection::Privacy {
            app.set_settings_section(SettingsSection::Appearance);
        }
    });
}

/// How the password is: asked for when Privacy & security opens, and after a reset.
fn load() {
    send(json!({ "@type": "getPasswordState" }), |answer| match answer.and_then(parse::<PasswordState>) {
        Ok(state) => show(&state),
        Err(Error::Stopped) => {}
        Err(err) => eprintln!("telegram: cannot get the password state: {err}"),
    });
}

fn show(state: &PasswordState) {
    STATE.with(|known| {
        let mut known = known.borrow_mut();
        known.has_password = state.has_password;
        known.has_recovery_email = state.has_recovery_email_address;
    });
    let state = state.clone();
    with_settings(move |settings| {
        settings.set_loaded(true);
        settings.set_has_password(state.has_password);
        settings.set_hint(state.password_hint.into());
        settings.set_has_recovery_email(state.has_recovery_email_address);
        if !state.has_recovery_email_address {
            settings.set_recovery_email(SharedString::new());
        }
        settings.set_reset_pending(state.pending_reset_date > 0);
        settings.set_reset_date(time::moment(state.pending_reset_date));
    });
}

/// From the overview: what to change. All but a first password need the current one first.
fn choose(target: PasswordStep) {
    let has_password = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.password.clear();
        state.recovery_code.clear();
        state.pending_email.clear();
        state.target = Some(target);
        state.has_password
    });
    with_settings(|settings| {
        settings.set_error(SharedString::new());
        settings.set_done(PasswordChange::Nothing);
        settings.set_code_sent_to(SharedString::new());
        settings.set_step(if has_password { PasswordStep::Verify } else { target });
    });
}

/// The current password is right when TDLib gives the recovery email address for it.
fn verify(password: String) {
    start();
    send(json!({ "@type": "getRecoveryEmailAddress", "password": password }), move |answer| {
        finish();
        match answer.and_then(parse::<RecoveryEmailAddress>) {
            Ok(address) => {
                let target = STATE.with(|state| {
                    let mut state = state.borrow_mut();
                    state.password = password;
                    state.target.unwrap_or(PasswordStep::Overview)
                });
                let shown = hide_email_address(&address.recovery_email_address);
                with_settings(move |settings| {
                    settings.set_recovery_email(shown.into());
                    settings.set_step(target);
                });
            }
            Err(err) => show_error(err),
        }
    });
}

/// "Forgot password?": a code to the recovery email address, or without one, a reset.
fn forgot() {
    let has_email = STATE.with(|state| state.borrow().has_recovery_email);
    with_settings(|settings| settings.set_error(SharedString::new()));
    if !has_email {
        with_settings(|settings| settings.set_step(PasswordStep::Reset));
        return;
    }
    start();
    send(json!({ "@type": "requestPasswordRecovery" }), |answer| {
        finish();
        match answer.and_then(parse::<EmailAddressAuthenticationCodeInfo>) {
            Ok(info) => with_settings(move |settings| {
                settings.set_code_sent_to(info.email_address_pattern.into());
                settings.set_step(PasswordStep::RecoveryCode);
            }),
            Err(err) => show_error(err),
        }
    });
}

fn submit_recovery_code(code: String) {
    start();
    send(json!({ "@type": "checkPasswordRecoveryCode", "recovery_code": code }), move |answer| {
        finish();
        match answer {
            Ok(_) => {
                STATE.with(|state| state.borrow_mut().recovery_code = code);
                with_settings(|settings| settings.set_step(PasswordStep::NewPassword));
            }
            Err(err) => show_error(err),
        }
    });
}

/// Without the password or the recovery email: Telegram removes the password after a wait, when
/// asked again then.
fn request_reset() {
    start();
    send(json!({ "@type": "resetPassword" }), |answer| {
        finish();
        match answer.and_then(parse::<ResetPasswordResult>) {
            Ok(ResetPasswordResult::Ok | ResetPasswordResult::Pending { .. }) => {
                end(PasswordChange::Nothing);
                load();
            }
            // A reset was cancelled lately; another may be asked for later.
            Ok(ResetPasswordResult::Declined { .. }) => {
                show_error(Error::Telegram { code: 429, message: "Too Many Requests: password reset declined".into() })
            }
            Err(err) => show_error(err),
        }
    });
}

fn cancel_reset() {
    start();
    send(json!({ "@type": "cancelPasswordReset" }), |answer| {
        finish();
        if let Err(err @ Error::Telegram { .. }) = answer {
            eprintln!("telegram: cannot cancel the password reset: {err}");
        }
        load();
    });
}

/// A new password: after the current one, after a recovery code, or the first one.
fn submit_new_password(password: &str, again: &str, hint: &str) {
    if let Err(problem) = check_new_password(password, again, hint) {
        with_settings(|settings| settings.set_error(problem.into()));
        return;
    }
    let (current, recovery_code) = STATE.with(|state| {
        let state = state.borrow();
        (state.password.clone(), state.recovery_code.clone())
    });
    let request = if recovery_code.is_empty() {
        json!({
            "@type": "setPassword",
            "old_password": current,
            "new_password": password,
            "new_hint": hint.trim(),
            "set_recovery_email_address": false,
            "new_recovery_email_address": "",
        })
    } else {
        json!({ "@type": "recoverPassword", "recovery_code": recovery_code, "new_password": password, "new_hint": hint.trim() })
    };
    change(request, PasswordChange::Password);
}

/// TDLib changes the hint with the password, so the password is set again, unchanged.
fn submit_hint(hint: String) {
    let current = STATE.with(|state| state.borrow().password.clone());
    if !hint.is_empty() && hint == current {
        with_settings(|settings| settings.set_error("NEW_PASSWORD_HINT_SAME".into()));
        return;
    }
    change(
        json!({
            "@type": "setPassword",
            "old_password": current,
            "new_password": current,
            "new_hint": hint,
            "set_recovery_email_address": false,
            "new_recovery_email_address": "",
        }),
        PasswordChange::Hint,
    );
}

/// A new recovery email address: it counts once the code sent to it comes back.
fn submit_recovery_email(address: String) {
    if !looks_like_email_address(&address) {
        with_settings(|settings| settings.set_error("EMAIL_INVALID".into()));
        return;
    }
    let current = STATE.with(|state| state.borrow().password.clone());
    start();
    send(json!({ "@type": "setRecoveryEmailAddress", "password": current, "new_recovery_email_address": address }), move |answer| {
        finish();
        match answer.and_then(parse::<PasswordState>) {
            Ok(state) if state.recovery_email_address_code_info.is_some() => {
                show(&state);
                STATE.with(|known| known.borrow_mut().pending_email = address.clone());
                with_settings(move |settings| {
                    settings.set_code_sent_to(address.into());
                    settings.set_step(PasswordStep::RecoveryEmailCode);
                });
            }
            // The same address as before: nothing to confirm.
            Ok(state) => {
                show(&state);
                let shown = hide_email_address(&address);
                with_settings(move |settings| settings.set_recovery_email(shown.into()));
                end(PasswordChange::RecoveryEmail);
            }
            Err(err) => show_error(err),
        }
    });
}

fn submit_recovery_email_code(code: String) {
    start();
    send(json!({ "@type": "checkRecoveryEmailAddressCode", "code": code }), |answer| {
        finish();
        match answer.and_then(parse::<PasswordState>) {
            Ok(state) => {
                show(&state);
                let address = STATE.with(|known| std::mem::take(&mut known.borrow_mut().pending_email));
                let shown = hide_email_address(&address);
                with_settings(move |settings| settings.set_recovery_email(shown.into()));
                end(PasswordChange::RecoveryEmail);
            }
            Err(err) => show_error(err),
        }
    });
}

/// An empty new password turns two-step verification off.
fn turn_off() {
    let current = STATE.with(|state| state.borrow().password.clone());
    change(
        json!({
            "@type": "setPassword",
            "old_password": current,
            "new_password": "",
            "new_hint": "",
            "set_recovery_email_address": false,
            "new_recovery_email_address": "",
        }),
        PasswordChange::Nothing,
    );
}

/// Back to the overview, forgetting the password; a new recovery email address still waiting for
/// its code is dropped.
fn cancel() {
    let waiting = STATE.with(|state| !state.borrow().pending_email.is_empty());
    if waiting {
        send(json!({ "@type": "cancelRecoveryEmailAddressVerification" }), |answer| {
            if let Err(err @ Error::Telegram { .. }) = answer {
                eprintln!("telegram: cannot cancel the recovery email address: {err}");
            }
        });
    }
    end(PasswordChange::Nothing);
}

/// A change whose answer is the password's new state; then the overview says what was done.
fn change(request: Value, done: PasswordChange) {
    start();
    send(request, move |answer| {
        finish();
        match answer.and_then(parse::<PasswordState>) {
            Ok(state) => {
                show(&state);
                end(done);
            }
            Err(err) => show_error(err),
        }
    });
}

fn end(done: PasswordChange) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.password.clear();
        state.target = None;
        state.recovery_code.clear();
        state.pending_email.clear();
    });
    with_settings(move |settings| {
        settings.set_busy(false);
        settings.set_error(SharedString::new());
        settings.set_code_sent_to(SharedString::new());
        settings.set_done(done);
        settings.set_step(PasswordStep::Overview);
    });
}

fn start() {
    with_settings(|settings| {
        settings.set_busy(true);
        settings.set_error(SharedString::new());
    });
}

fn finish() {
    with_settings(|settings| settings.set_busy(false));
}

/// TDLib's error under the form; the page words it (ui/pages/password.slint).
fn show_error(err: Error) {
    let message = match err {
        Error::Telegram { message, .. } => message,
        Error::Stopped => return,
    };
    with_settings(move |settings| settings.set_error(message.into()));
}

fn with_settings(change: impl FnOnce(&PasswordSettings)) {
    with_ui(|ui| change(&ui.global::<PasswordSettings>()));
}

/// TDLib's answer as `T`; one that cannot be read fails like an error of TDLib's.
fn parse<T: DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|err| Error::Telegram { code: 0, message: format!("unreadable answer: {err}") })
}

fn digits(code: &str) -> String {
    code.chars().filter(char::is_ascii_digit).collect()
}

/// Something, an @, and a domain with a dot in it; Telegram checks the rest.
fn looks_like_email_address(address: &str) -> bool {
    match address.split_once('@') {
        Some((name, domain)) => {
            !name.is_empty()
                && !address.chars().any(char::is_whitespace)
                && !domain.contains('@')
                && domain.split('.').count() > 1
                && domain.split('.').all(|part| !part.is_empty())
        }
        None => false,
    }
}

/// The address on screen, as the design shows it: "zhou@gmail.com" → "z•••@gmail.com".
fn hide_email_address(address: &str) -> String {
    match address.split_once('@') {
        Some((name, domain)) if !name.is_empty() => {
            let first: String = name.chars().take(1).collect();
            format!("{first}•••@{domain}")
        }
        _ => address.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_password_is_typed_twice_and_its_hint_is_not_it() {
        assert_eq!(check_new_password("", "", ""), Err("NEW_PASSWORD_EMPTY"));
        assert_eq!(check_new_password("finch", "finch ", ""), Err("NEW_PASSWORD_MISMATCH"));
        assert_eq!(check_new_password("finch", "finch", " finch "), Err("NEW_PASSWORD_HINT_SAME"));
        assert_eq!(check_new_password("finch", "finch", "a bird"), Ok(()));
        assert_eq!(check_new_password("finch", "finch", ""), Ok(()));
    }

    #[test]
    fn email_addresses_are_checked_loosely_and_shown_partly() {
        assert!(looks_like_email_address("zhou@gmail.com"));
        assert!(!looks_like_email_address("zhou@gmail"));
        assert!(!looks_like_email_address("zhou gmail.com"));
        assert!(!looks_like_email_address("@gmail.com"));
        assert!(!looks_like_email_address("zhou@@gmail.com"));
        assert_eq!(hide_email_address("zhou@gmail.com"), "z•••@gmail.com");
        assert_eq!(hide_email_address(""), "");
    }
}
