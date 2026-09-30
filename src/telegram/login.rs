//! Logging in (ui/pages/login.slint). The page's step follows TDLib's authorization state
//! ([`on_state`]); what the user enters goes back to TDLib from here. The page may go back to the
//! phone number on its own: TDLib takes a new phone number in every state that waits for a code,
//! a password or an email. From the QR code it does not, so going back from there logs out, and
//! TDLib starts over (mod.rs starts a new finchgram-tdlib once the old one has closed).

use std::cell::RefCell;

use serde_json::json;
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::api::{AuthenticationCodeType, AuthorizationState, Countries, CountryInfo, PhoneNumberInfo, Text};
use super::{Error, send, with_ui};
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
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
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
        AuthorizationState::WaitPassword { password_hint } => {
            let hint = password_hint.clone();
            with_login(|login| {
                login.set_password_hint(hint.into());
                login.set_error(SharedString::new());
                login.set_step(LoginStep::Password);
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
        AuthorizationState::WaitRegistration => with_login(|login| login.set_step(LoginStep::Registration)),
        AuthorizationState::WaitPremiumPurchase => with_login(|login| login.set_step(LoginStep::Premium)),
        AuthorizationState::Ready | AuthorizationState::Closed => {
            // Logged in, or starting over: the next login begins at the phone number.
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
    fn a_number_is_sent_with_its_country_code() {
        assert_eq!(plus("8613800132046"), "+8613800132046");
        assert_eq!(plus("+49301234567"), "+49301234567");
    }
}
