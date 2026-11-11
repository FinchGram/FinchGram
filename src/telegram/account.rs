//! The account logged in: who it is (the profile page, the title bars), editing its profile and
//! its photo, and logging out.

use serde_json::json;
use slint::{ComponentHandle, SharedString};

use super::api::{PhoneNumberInfo, User, UserFullInfo};
use super::store;
use super::{Error, errors, manage, send, with_ui};
use crate::{Account, MainWindow, ManageWords};

/// Telegram's limit on a bio without Premium.
const BIO_LIMIT: usize = 70;

pub fn connect(ui: &MainWindow) {
    let account = ui.global::<Account>();
    account.on_save(|first_name, last_name, username, bio| {
        save(first_name.trim().to_string(), last_name.trim().to_string(), username.trim().trim_start_matches('@').to_string(), bio.trim().to_string());
    });
    account.on_change_photo(change_photo);
    // The id onto the clipboard. The profile page says "Copied" itself: the chat windows' notice
    // is not on it.
    account.on_copy_id(|| {
        let id = store::with(|store| store.my_id).unwrap_or(0);
        if id == 0 {
            return;
        }
        if let Err(err) = crate::platform::copy_text(&id.to_string()) {
            eprintln!("account: cannot copy the id: {err}");
        }
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
            store.dirty.accounts = true;
            store.dirty.chats = true;
        });
        store::refresh();
        // The account is known by its id now: to the list of accounts, and to the AI access
        // settings, which are one account's (mod.rs, src/mcp/access.rs).
        super::account_signed_in(id);
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
            let phone = format!("+{} {}", info.country_calling_code, info.formatted_phone_number).trim().to_string();
            store::with(|store| {
                store.my_phone = phone;
                store.dirty.account = true;
                store.dirty.accounts = true;
            });
            store::refresh();
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
        account.set_id(SharedString::new());
        account.set_bio(SharedString::new());
        account.set_initial(SharedString::new());
        account.set_has_picture(false);
        account.set_photo_saving(false);
        account.set_saved(false);
        account.set_save_error(SharedString::new());
    });
}

/// The profile page's photo clicked: a picture from the system's file panel, its middle square of
/// 640 pixels (made as a group's photo is, manage.rs, at the crop frame's widest), sent as the
/// account's new photo; Telegram's update of the account then shows it everywhere. The profile's
/// design has no crop page, so there is none here (GitHub issue 2).
fn change_photo() {
    let slot = super::context();
    crate::platform::choose_picture(move |path| {
        let Some(path) = path else { return };
        super::in_context(slot, || {
            with_ui(|ui| {
                let account = ui.global::<Account>();
                account.set_save_error(SharedString::new());
                account.set_photo_saving(true);
            });
        });
        std::thread::spawn(move || {
            let cropped = manage::crop_file(&path, 0, 0, 1.0, 0.5, 0.5);
            let _ = slint::invoke_from_event_loop(move || {
                super::in_context(slot, || match cropped {
                    Ok((file, _)) => {
                        let request = json!({
                            "@type": "setProfilePhoto",
                            "photo": { "@type": "inputChatPhotoStatic", "photo": { "@type": "inputFileLocal", "path": file.as_str() } },
                            "is_public": false,
                        });
                        send(request, move |answer| {
                            // TDLib answers once the photo is up and set (or refused): the file is done.
                            let _ = std::fs::remove_file(&file);
                            with_ui(|ui| {
                                let account = ui.global::<Account>();
                                account.set_photo_saving(false);
                                match answer {
                                    Ok(_) => {}
                                    Err(Error::Telegram { message, .. }) => account.set_save_error(errors::explain(&message).into()),
                                    Err(err) => account.set_save_error(err.to_string().into()),
                                }
                            });
                        });
                    }
                    Err(err) => {
                        eprintln!("account: cannot make the photo: {err}");
                        with_ui(|ui| {
                            let account = ui.global::<Account>();
                            account.set_photo_saving(false);
                            account.set_save_error(ui.global::<ManageWords>().invoke_cannot_read_picture());
                        });
                    }
                });
            });
        });
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
