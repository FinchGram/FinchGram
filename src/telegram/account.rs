//! The account logged in: who it is (the profile page, the title bars), editing its profile, and
//! logging out.

use serde_json::json;
use slint::{ComponentHandle, SharedString};

use super::api::{PhoneNumberInfo, User, UserFullInfo};
use super::store;
use super::{Error, send, with_ui};
use crate::{Account, MainWindow};

/// Telegram's limit on a bio without Premium.
const BIO_LIMIT: usize = 70;

pub fn connect(ui: &MainWindow) {
    let account = ui.global::<Account>();
    account.on_save(|first_name, last_name, username, bio| {
        save(first_name.trim().to_string(), last_name.trim().to_string(), username.trim().trim_start_matches('@').to_string(), bio.trim().to_string());
    });
    account.on_log_out(|| {
        send(json!({ "@type": "logOut" }), |answer| {
            if let Err(err @ Error::Telegram { .. }) = answer {
                eprintln!("telegram: cannot log out: {err}");
            }
        });
    });
}

/// Logged in: who we are.
pub fn load() {
    send(json!({ "@type": "getMe" }), |answer| {
        let Ok(Ok(me)) = answer.map(serde_json::from_value::<User>) else { return };
        let (id, phone) = (me.id, me.phone_number.clone());
        store::with(|store| {
            store.my_id = id;
            store.users.insert(id, me);
            store.dirty.account = true;
            store.dirty.chats = true;
        });
        store::refresh();
        send(json!({ "@type": "getUserFullInfo", "user_id": id }), move |answer| {
            let Ok(Ok(info)) = answer.map(serde_json::from_value::<UserFullInfo>) else { return };
            store::with(|store| {
                store.my_bio = info.bio.map(|bio| bio.text).unwrap_or_default();
                store.dirty.account = true;
            });
            store::refresh();
        });
        // The number as people write it there: "+86 138 0013 2046".
        send(json!({ "@type": "getPhoneNumberInfo", "phone_number_prefix": phone }), |answer| {
            let Ok(Ok(info)) = answer.map(serde_json::from_value::<PhoneNumberInfo>) else { return };
            let phone = format!("+{} {}", info.country_calling_code, info.formatted_phone_number);
            with_ui(|ui| ui.global::<Account>().set_phone(phone.trim().into()));
        });
    });
}

/// Logged out: nobody.
pub fn forget() {
    with_ui(|ui| {
        let account = ui.global::<Account>();
        account.set_name(SharedString::new());
        account.set_first_name(SharedString::new());
        account.set_last_name(SharedString::new());
        account.set_username(SharedString::new());
        account.set_phone(SharedString::new());
        account.set_bio(SharedString::new());
        account.set_initial(SharedString::new());
        account.set_saved(false);
        account.set_save_error(SharedString::new());
    });
}

/// Save what changed, one request after the other; the first error stops and is shown.
fn save(first_name: String, last_name: String, username: String, bio: String) {
    let Some((current_first, current_last, current_username, current_bio)) = store::with(|store| {
        let me = store.users.get(&store.my_id)?;
        let username = me.usernames.as_ref().map(|names| names.editable_username.clone()).unwrap_or_default();
        Some((me.first_name.clone(), me.last_name.clone(), username, store.my_bio.clone()))
    })
    .flatten() else {
        return;
    };
    let bio: String = bio.chars().take(BIO_LIMIT).collect();
    let mut requests = Vec::new();
    if (first_name.as_str(), last_name.as_str()) != (current_first.as_str(), current_last.as_str()) {
        requests.push(json!({ "@type": "setName", "first_name": first_name, "last_name": last_name }));
    }
    if username != current_username {
        requests.push(json!({ "@type": "setUsername", "username": username }));
    }
    if bio != current_bio {
        requests.push(json!({ "@type": "setBio", "bio": bio }));
    }
    with_ui(|ui| {
        let account = ui.global::<Account>();
        account.set_save_error(SharedString::new());
        account.set_saving(!requests.is_empty());
        account.set_saved(requests.is_empty());
    });
    send_in_turn(requests);
}

fn send_in_turn(mut requests: Vec<serde_json::Value>) {
    if requests.is_empty() {
        return;
    }
    let request = requests.remove(0);
    send(request, move |answer| match answer {
        Ok(_) if requests.is_empty() => with_ui(|ui| {
            let account = ui.global::<Account>();
            account.set_saving(false);
            account.set_saved(true);
            // getUserFullInfo again: TDLib does not always say that the bio changed.
            let id = store::with(|store| store.my_id).unwrap_or(0);
            send(json!({ "@type": "getUserFullInfo", "user_id": id }), |answer| {
                let Ok(Ok(info)) = answer.map(serde_json::from_value::<UserFullInfo>) else { return };
                store::with(|store| {
                    store.my_bio = info.bio.map(|bio| bio.text).unwrap_or_default();
                    store.dirty.account = true;
                });
                store::refresh();
            });
        }),
        Ok(_) => send_in_turn(requests),
        Err(err) => with_ui(|ui| {
            let account = ui.global::<Account>();
            account.set_saving(false);
            account.set_save_error(err.to_string().into());
        }),
    });
}
