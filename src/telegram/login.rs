//! Logging in (ui/pages/login.slint). The page's step follows TDLib's authorization state
//! ([`on_state`]); what the user enters goes back to TDLib from here. The page may go back to the
//! phone number on its own: TDLib takes a new phone number in every state that waits for a code,
//! a password or an email. From the QR code it does not, so going back from there logs out, and
//! TDLib starts over (mod.rs starts a new finchgram-tdlib once the old one has closed).
//!
//! A forgotten password is recovered while TDLib waits for it: a code goes to the recovery email
//! address, and with it a new password logs in. Without that address, the account can only be
//! reset, which Telegram delays by a week when the account was in use lately.
//!
//! A login left half done when the app quit is not resumed at the next start, as in Telegram's own
//! desktop app: TDLib keeps its progress on disk, and a code already accepted would stay there,
//! waiting only for the password. It is thrown away ([`discard_unfinished`]) and logging in starts
//! again at the phone number. Within one run of the app, a restarted finchgram-tdlib carries on.

use std::cell::{Cell, RefCell};

use serde_json::json;
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::api::{AuthenticationCodeType, AuthorizationState, Countries, CountryInfo, PhoneNumberInfo, Text};
use super::{Error, password, send, time, with_ui};
use crate::{CodeDelivery, Country, Login, LoginStep, MainWindow};

/// How many countries the picker shows for its search words.
const COUNTRIES_SHOWN: usize = 300;

#[derive(Default)]
struct State {
    /// Every country TDLib knows, sorted by name, once asked for.
    countries: Vec<CountryInfo>,
    asked_for_countries: bool,
    /// TDLib's authorization state, as last reported.
    authorization: Option<AuthorizationState>,
    /// The recovery code TDLib accepted, for the new password.
    recovery_code: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// TDLib has not yet said where logging in is since the app started.
    static FIRST_STATE: Cell<bool> = const { Cell::new(true) };
    /// A login left half done is being thrown away: TDLib closes, and a fresh one starts.
    static DISCARDING: Cell<bool> = const { Cell::new(false) };
}

/// Called with every authorization state before anything else sees it. The first one since the app
/// started is a login left half done by an earlier run: TDLib drops it (logOut destroys the keys of
/// a session that is not logged in, without the network), and true says to show nothing of it.
pub fn discard_unfinished(state: &AuthorizationState) -> bool {
    if matches!(
        state,
        AuthorizationState::WaitTdlibParameters
            | AuthorizationState::LoggingOut
            | AuthorizationState::Closing
            | AuthorizationState::Closed
    ) {
        return false;
    }
    // A fresh TDLib says where it is: whatever was thrown away is gone.
    DISCARDING.with(|discarding| discarding.set(false));
    if !FIRST_STATE.with(|first| first.replace(false)) || !is_unfinished(state) {
        return false;
    }
    DISCARDING.with(|discarding| discarding.set(true));
    send(json!({ "@type": "logOut" }), |answer| {
        if let Err(err @ Error::Telegram { .. }) = answer {
            eprintln!("telegram: cannot drop the unfinished login: {err}");
        }
    });
    true
}

/// A login left half done is being thrown away (see [`discard_unfinished`]).
pub fn discarding() -> bool {
    DISCARDING.with(Cell::get)
}

/// Somewhere between the phone number and being logged in.
fn is_unfinished(state: &AuthorizationState) -> bool {
    matches!(
        state,
        AuthorizationState::WaitCode { .. }
            | AuthorizationState::WaitPassword { .. }
            | AuthorizationState::WaitEmailAddress
            | AuthorizationState::WaitEmailCode { .. }
            | AuthorizationState::WaitRegistration
            | AuthorizationState::WaitOtherDeviceConfirmation { .. }
            | AuthorizationState::WaitPremiumPurchase
    )
}

pub fn connect(ui: &MainWindow) {
    let login = ui.global::<Login>();
    login.on_search_countries(|words| show_countries(&words));
    login.on_choose_country(|country| {
        with_login(|login| login.set_country(country));
    });
    login.on_submit_phone(|calling_code, number| submit_phone(&calling_code, &number));
    login.on_use_qr_code(use_qr_code);
    login.on_use_phone_number(use_phone_number);
    login.on_submit_code(|code| {
        let code: String = code.chars().filter(char::is_ascii_digit).collect();
        request(json!({ "@type": "checkAuthenticationCode", "code": code }));
    });
    login.on_resend_code(|| {
        let request = json!({ "@type": "resendAuthenticationCode", "reason": { "@type": "resendCodeReasonUserRequest" } });
        busy(true);
        send(request, |answer| {
            busy(false);
            match answer {
                Ok(_) => with_login(|login| login.set_resent(true)),
                Err(err) => show_error(err),
            }
        });
    });
    login.on_submit_password(|password| {
        request(json!({ "@type": "checkAuthenticationPassword", "password": password.as_str() }));
    });
    login.on_forgot_password(forgot_password);
    login.on_submit_recovery_code(|code| {
        let code: String = code.chars().filter(char::is_ascii_digit).collect();
        busy(true);
        with_login(|login| login.set_error(SharedString::new()));
        send(json!({ "@type": "checkAuthenticationPasswordRecoveryCode", "recovery_code": code }), move |answer| {
            busy(false);
            match answer {
                Ok(_) => {
                    STATE.with(|state| state.borrow_mut().recovery_code = code);
                    with_login(|login| login.set_step(LoginStep::NewPassword));
                }
                Err(err) => show_error(err),
            }
        });
    });
    login.on_submit_new_password(|new_password, again, hint| {
        if let Err(problem) = password::check_new_password(&new_password, &again, &hint) {
            with_login(|login| login.set_error(problem.into()));
            return;
        }
        let code = STATE.with(|state| state.borrow().recovery_code.clone());
        request(json!({
            "@type": "recoverAuthenticationPassword",
            "recovery_code": code,
            "new_password": new_password.as_str(),
            "new_hint": hint.trim(),
        }));
    });
    login.on_reset_account(reset_account);
    login.on_submit_email(|email| {
        request(json!({ "@type": "setAuthenticationEmailAddress", "email_address": email.trim() }));
    });
    login.on_submit_email_code(|code| {
        let code: String = code.chars().filter(char::is_ascii_digit).collect();
        request(json!({
            "@type": "checkAuthenticationEmailCode",
            "code": { "@type": "emailAddressAuthenticationCode", "code": code },
        }));
    });
    login.on_sign_up(|first_name, last_name| {
        let first_name = first_name.trim();
        if first_name.is_empty() {
            with_login(|login| login.set_error("FIRSTNAME_INVALID".into()));
            return;
        }
        request(json!({
            "@type": "registerUser",
            "first_name": first_name,
            "last_name": last_name.trim(),
            "disable_notification": false,
        }));
    });
    login.on_initial(|name| name.trim().chars().next().map(|first| first.to_uppercase().collect::<String>()).unwrap_or_default().into());
    login.on_back(|| {
        let from_qr_code = STATE.with(|state| {
            matches!(state.borrow().authorization, Some(AuthorizationState::WaitOtherDeviceConfirmation { .. }))
        });
        if from_qr_code {
            use_phone_number();
        } else {
            with_login(|login| {
                login.set_error(SharedString::new());
                login.set_step(LoginStep::Phone);
            });
        }
    });
}

/// TDLib's authorization state changed: show the step it waits for.
pub fn on_state(state: &AuthorizationState) {
    STATE.with(|login| login.borrow_mut().authorization = Some(state.clone()));
    busy(false);
    match state {
        AuthorizationState::WaitPhoneNumber => {
            with_login(|login| login.set_step(LoginStep::Phone));
            load_countries();
        }
        AuthorizationState::WaitCode { code_info } => {
            let (delivery, length) = match code_info.kind {
                AuthenticationCodeType::TelegramMessage { length } => (CodeDelivery::Telegram, length),
                AuthenticationCodeType::Sms { length } => (CodeDelivery::Sms, length),
                AuthenticationCodeType::Call { length } => (CodeDelivery::Call, length),
                AuthenticationCodeType::FlashCall => (CodeDelivery::FlashCall, 5),
                AuthenticationCodeType::MissedCall { length } => (CodeDelivery::MissedCall, length),
                AuthenticationCodeType::Fragment { length } => (CodeDelivery::Fragment, length),
                AuthenticationCodeType::Other => (CodeDelivery::Other, 5),
            };
            let can_resend = code_info.next_type.is_some();
            let phone = code_info.phone_number.clone();
            with_login(|login| {
                login.set_delivery(delivery);
                login.set_code_length(length.max(1));
                login.set_can_resend(can_resend);
                login.set_code_phone(plus(&phone).into());
                login.set_error(SharedString::new());
                login.set_step(LoginStep::Code);
            });
            // Written the way people write numbers there: "+86 138 0013 2046".
            send(json!({ "@type": "getPhoneNumberInfo", "phone_number_prefix": phone }), |answer| {
                if let Ok(info) = answer.and_then(|value| serde_json::from_value::<PhoneNumberInfo>(value).map_err(|_| Error::Stopped)) {
                    let formatted = format!("+{} {}", info.country_calling_code, info.formatted_phone_number);
                    with_login(|login| login.set_code_phone(formatted.trim().into()));
                }
            });
        }
        AuthorizationState::WaitPassword { password_hint, has_recovery_email_address, recovery_email_address_pattern } => {
            let (hint, has_email, pattern) = (password_hint.clone(), *has_recovery_email_address, recovery_email_address_pattern.clone());
            with_login(|login| {
                login.set_password_hint(hint.into());
                login.set_has_recovery_email(has_email);
                login.set_recovery_email(pattern.into());
                // Asking for a recovery code says this state again: the recovery goes on.
                let recovering = matches!(
                    login.get_step(),
                    LoginStep::RecoveryCode | LoginStep::NewPassword | LoginStep::ResetAccount | LoginStep::AccountResetRequested
                );
                if !recovering {
                    login.set_error(SharedString::new());
                    login.set_step(LoginStep::Password);
                }
            });
        }
        AuthorizationState::WaitOtherDeviceConfirmation { link } => {
            let image = qr_code(link).unwrap_or_default();
            with_login(|login| {
                login.set_qr_code(image);
                login.set_error(SharedString::new());
                login.set_step(LoginStep::Qr);
            });
        }
        AuthorizationState::WaitEmailAddress => with_login(|login| login.set_step(LoginStep::EmailAddress)),
        AuthorizationState::WaitEmailCode { code_info } => {
            let pattern = code_info.email_address_pattern.clone();
            let length = code_info.length;
            with_login(|login| {
                login.set_email_pattern(pattern.into());
                login.set_code_length(length.max(1));
                login.set_error(SharedString::new());
                login.set_step(LoginStep::EmailCode);
            });
        }
        AuthorizationState::WaitRegistration => with_login(|login| {
            login.set_error(SharedString::new());
            login.set_step(LoginStep::Registration);
        }),
        AuthorizationState::WaitPremiumPurchase => with_login(|login| login.set_step(LoginStep::Premium)),
        AuthorizationState::Ready | AuthorizationState::Closed => {
            // Logged in, or starting over: the next login begins at the phone number.
            STATE.with(|state| state.borrow_mut().recovery_code.clear());
            with_login(|login| {
                login.set_error(SharedString::new());
                login.set_resent(false);
                login.set_qr_code(Image::default());
                login.set_step(LoginStep::Phone);
            });
        }
        AuthorizationState::WaitTdlibParameters | AuthorizationState::LoggingOut | AuthorizationState::Closing => {}
    }
}

/// "Forgot password?": a recovery code by email, or without a recovery email address, the way
/// that is left.
fn forgot_password() {
    let has_email = STATE.with(|state| {
        matches!(state.borrow().authorization, Some(AuthorizationState::WaitPassword { has_recovery_email_address: true, .. }))
    });
    with_login(|login| login.set_error(SharedString::new()));
    if !has_email {
        with_login(|login| login.set_step(LoginStep::ResetAccount));
        return;
    }
    busy(true);
    send(json!({ "@type": "requestAuthenticationPasswordRecovery" }), |answer| {
        busy(false);
        match answer {
            Ok(_) => with_login(|login| login.set_step(LoginStep::RecoveryCode)),
            Err(err) => show_error(err),
        }
    });
}

/// Delete the account to sign up again with the same number. When it was in use lately, Telegram
/// waits a week first (2FA_CONFIRM_WAIT, which TDLib reports as "retry after" that many seconds);
/// the reset is then completed by asking again once the week is over.
fn reset_account() {
    busy(true);
    with_login(|login| login.set_error(SharedString::new()));
    send(json!({ "@type": "deleteAccount", "reason": "", "password": "" }), |answer| {
        busy(false);
        match answer {
            // The account is gone: TDLib logs out, and the next login begins at the phone number.
            Ok(_) => {}
            Err(Error::Telegram { message, .. }) if wait_seconds(&message).is_some() => {
                let seconds = wait_seconds(&message).unwrap_or_default();
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
                let date = i32::try_from(now.saturating_add(seconds)).unwrap_or(i32::MAX);
                with_login(|login| {
                    login.set_reset_date(time::moment(date));
                    login.set_step(LoginStep::AccountResetRequested);
                });
            }
            Err(err) => show_error(err),
        }
    });
}

/// "Too Many Requests: retry after 604800" → 604800
fn wait_seconds(message: &str) -> Option<u64> {
    message.strip_prefix("Too Many Requests: retry after ")?.trim().parse().ok()
}

fn submit_phone(calling_code: &str, number: &str) {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return;
    }
    // A number typed with its own "+…" keeps it; otherwise the chosen country's code goes first.
    let phone = if number.trim_start().starts_with('+') { format!("+{digits}") } else { format!("+{calling_code}{digits}") };
    request(json!({ "@type": "setAuthenticationPhoneNumber", "phone_number": phone, "settings": null }));
}

fn use_qr_code() {
    request(json!({ "@type": "requestQrCodeAuthentication", "other_user_ids": [] }));
}

/// From the QR code back to the phone number: TDLib only allows it by starting over.
fn use_phone_number() {
    busy(true);
    send(json!({ "@type": "logOut" }), |answer| {
        if let Err(err) = answer {
            busy(false);
            show_error(err);
        }
    });
}

/// Send one step's answer: busy until TDLib answers; an error is shown under the step, a success
/// shows itself as the next authorization state.
fn request(request: serde_json::Value) {
    busy(true);
    with_login(|login| login.set_error(SharedString::new()));
    send(request, |answer| {
        busy(false);
        if let Err(err) = answer {
            show_error(err);
        }
    });
}

fn busy(busy: bool) {
    with_login(|login| login.set_busy(busy));
}

/// TDLib's error, which the page words where it knows it (ui/pages/login.slint).
fn show_error(err: Error) {
    let message = match err {
        Error::Telegram { message, .. } => message,
        Error::Stopped => return,
    };
    with_login(|login| login.set_error(message.into()));
}

fn with_login(change: impl FnOnce(&Login)) {
    with_ui(|ui| change(&ui.global::<Login>()));
}

/// "8613800132046" → "+8613800132046"
fn plus(phone: &str) -> String {
    if phone.starts_with('+') { phone.to_string() } else { format!("+{phone}") }
}

/// The country list, once, and a first guess at the country: where TDLib thinks we are.
fn load_countries() {
    let ask = STATE.with(|state| !std::mem::replace(&mut state.borrow_mut().asked_for_countries, true));
    if !ask {
        return;
    }
    send(json!({ "@type": "getCountries" }), |answer| {
        let Ok(Ok(countries)) = answer.map(serde_json::from_value::<Countries>) else {
            STATE.with(|state| state.borrow_mut().asked_for_countries = false);
            return;
        };
        let mut countries: Vec<CountryInfo> =
            countries.countries.into_iter().filter(|country| !country.is_hidden && !country.calling_codes.is_empty()).collect();
        countries.sort_by(|a, b| a.english_name.cmp(&b.english_name));
        STATE.with(|state| state.borrow_mut().countries = countries);
        show_countries("");
        send(json!({ "@type": "getCountryCode" }), |answer| {
            let code = answer.ok().and_then(|value| serde_json::from_value::<Text>(value).ok()).map(|text| text.text);
            let chosen = STATE.with(|state| {
                let state = state.borrow();
                code.and_then(|code| state.countries.iter().find(|country| country.country_code == code))
                    .or_else(|| state.countries.first())
                    .map(country)
            });
            if let Some(chosen) = chosen {
                with_login(|login| {
                    if login.get_country().calling_code.is_empty() {
                        login.set_country(chosen);
                    }
                });
            }
        });
    });
}

fn country(info: &CountryInfo) -> Country {
    Country {
        code: info.country_code.clone().into(),
        name: info.name.clone().into(),
        calling_code: info.calling_codes.first().cloned().unwrap_or_default().into(),
    }
}

/// The picker's list: countries whose name (in its own language or in English) or calling code
/// matches `words`.
fn show_countries(words: &str) {
    let words = words.trim().trim_start_matches('+').to_lowercase();
    let shown: Vec<Country> = STATE.with(|state| {
        state
            .borrow()
            .countries
            .iter()
            .filter(|info| {
                words.is_empty()
                    || info.name.to_lowercase().contains(&words)
                    || info.english_name.to_lowercase().contains(&words)
                    || info.calling_codes.iter().any(|code| code.starts_with(&words))
            })
            .take(COUNTRIES_SHOWN)
            .map(country)
            .collect()
    });
    with_login(|login| login.set_countries(ModelRc::new(VecModel::from(shown))));
}

/// The QR code of `link`, black on white, with the quiet zone QR readers need around it.
fn qr_code(link: &str) -> Option<Image> {
    const QUIET: usize = 2;
    const SCALE: usize = 8;
    let code = qrcode::QrCode::new(link.as_bytes()).ok()?;
    let modules = code.width();
    let size = (modules + 2 * QUIET) * SCALE;
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(size as u32, size as u32);
    let pixels = buffer.make_mut_slice();
    for y in 0..size {
        for x in 0..size {
            let (column, row) = (x / SCALE, y / SCALE);
            let inside = (QUIET..modules + QUIET).contains(&column) && (QUIET..modules + QUIET).contains(&row);
            let dark = inside && code[(column - QUIET, row - QUIET)] == qrcode::Color::Dark;
            let level = if dark { 0 } else { 255 };
            pixels[y * size + x] = Rgba8Pixel { r: level, g: level, b: level, a: 255 };
        }
    }
    Some(Image::from_rgba8(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_link_makes_a_square_qr_code() {
        let image = qr_code("tg://login?token=AQJZcGhpbmUAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").expect("a QR code");
        let size = image.size();
        assert_eq!(size.width, size.height);
        assert!(size.width >= (21 + 4) * 8, "at least version 1 with its quiet zone");
    }

    #[test]
    fn only_the_steps_between_the_number_and_logged_in_are_unfinished() {
        let password = AuthorizationState::WaitPassword {
            password_hint: String::new(),
            has_recovery_email_address: false,
            recovery_email_address_pattern: String::new(),
        };
        assert!(is_unfinished(&password));
        assert!(is_unfinished(&AuthorizationState::WaitRegistration));
        assert!(!is_unfinished(&AuthorizationState::WaitPhoneNumber));
        assert!(!is_unfinished(&AuthorizationState::Ready));
        assert!(!is_unfinished(&AuthorizationState::Closed));
    }

    #[test]
    fn a_delayed_account_reset_says_how_long_it_waits() {
        assert_eq!(wait_seconds("Too Many Requests: retry after 604800"), Some(604_800));
        assert_eq!(wait_seconds("PASSWORD_HASH_INVALID"), None);
    }

    #[test]
    fn a_number_is_sent_with_its_country_code() {
        assert_eq!(plus("8613800132046"), "+8613800132046");
        assert_eq!(plus("+49301234567"), "+49301234567");
    }
}
