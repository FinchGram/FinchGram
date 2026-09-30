//! FinchGram — an open-source Telegram desktop client with a media center.
//!
//! This is the shell: the pages, the settings and the app's state. Telegram itself is reached only
//! through finchgram-tdlib, a separate program next to this executable (src/telegram/,
//! docs/architecture.md).

mod i18n;
mod platform;
mod settings;
mod telegram;
mod update;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use settings::Settings;

slint::include_modules!();

/// How often a running app looks at the clock to see whether its daily update check is due
/// (the check itself happens once a day, at a random moment; see update.rs). No network here.
const UPDATE_SCHEDULE_TICK: Duration = Duration::from_secs(60);

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let state = ui.global::<AppState>();
    let mut settings = Settings::load();

    // Language: saved choice > system locale > English.
    // (Bundled translations can only be selected once a component exists.)
    let language = i18n::initial_language(&settings.language);
    i18n::apply(&language);
    state.set_language(language.clone().into());
    if settings.language != language {
        settings.language = language;
        settings.save();
    }
    state.set_appearance(settings.appearance.clone().into());
    state.set_app_version(update::CURRENT_VERSION.into());

    telegram::start(&ui);

    // Updates: the newest release found by the last check, kept here so "install" knows what
    // to download. Checked once a day at a random moment (unless check_for_updates is off in
    // settings.toml), and whenever the user asks (Help menu).
    let pending: Arc<Mutex<Option<update::Release>>> = Arc::new(Mutex::new(None));
    state.on_check_for_updates({
        let ui = ui.as_weak();
        let pending = pending.clone();
        move || check_for_updates(ui.clone(), pending.clone(), true)
    });
    state.on_install_update({
        let ui = ui.as_weak();
        let pending = pending.clone();
        move || install_update(ui.clone(), pending.clone())
    });
    let update_timer = slint::Timer::default();
    if settings.check_for_updates {
        update_timer.start(slint::TimerMode::Repeated, UPDATE_SCHEDULE_TICK, {
            let ui = ui.as_weak();
            let pending = pending.clone();
            move || automatic_update_check(ui.clone(), pending.clone())
        });
        automatic_update_check(ui.as_weak(), pending);
    }

    let result = ui.run();
    // TDLib writes its database out before finchgram-tdlib ends.
    telegram::shut_down();
    result
}

/// The daily check, when its moment has come. Quiet: a newer version only changes the Help menu
/// and the status line; a failure is only logged.
fn automatic_update_check(ui: slint::Weak<MainWindow>, pending: Arc<Mutex<Option<update::Release>>>) {
    if update::automatic_check_due() {
        check_for_updates(ui, pending, false);
    }
}

/// Ask GitHub for the newest release in a thread and show the outcome. A `manual` check (Help
/// menu) reports "up to date" and failures; an automatic one stays quiet.
fn check_for_updates(ui: slint::Weak<MainWindow>, pending: Arc<Mutex<Option<update::Release>>>, manual: bool) {
    let Some(window) = ui.upgrade() else { return };
    let state = window.global::<AppState>();
    // One check at a time, and never in the middle of an install.
    if matches!(state.get_update_state(), UpdateState::Checking | UpdateState::Installing) {
        return;
    }
    state.set_update_state(UpdateState::Checking);

    std::thread::spawn(move || {
        let result = update::check();
        match &result {
            Ok(_) => update::plan_next_automatic_check(),
            Err(_) => update::plan_retry_after_failure(),
        }
        let _ = ui.upgrade_in_event_loop(move |ui| {
            let state = ui.global::<AppState>();
            match result {
                Ok(update::Check::Available(release)) => {
                    eprintln!("update: {} is available (running {})", release.version, update::CURRENT_VERSION);
                    state.set_update_version(release.version.to_string().into());
                    state.set_update_notes(release.notes.clone().into());
                    state.set_update_state(UpdateState::Available);
                    *pending.lock().unwrap() = Some(release);
                }
                Ok(update::Check::UpToDate) => {
                    state.set_update_state(if manual { UpdateState::UpToDate } else { UpdateState::Idle });
                    *pending.lock().unwrap() = None;
                }
                Err(err) => {
                    eprintln!("update: check failed: {err}");
                    if manual {
                        state.set_update_error(err.into());
                        state.set_update_state(UpdateState::Failed);
                    } else {
                        state.set_update_state(UpdateState::Idle);
                    }
                }
            }
        });
    });
}

/// Download and install the release found by the last check, then quit: the new copy is
/// started as soon as this process has exited (see update::install).
fn install_update(ui: slint::Weak<MainWindow>, pending: Arc<Mutex<Option<update::Release>>>) {
    let Some(release) = pending.lock().unwrap().clone() else { return };
    let Some(window) = ui.upgrade() else { return };
    let state = window.global::<AppState>();
    if state.get_update_state() == UpdateState::Installing {
        return;
    }
    state.set_update_progress(0.0);
    state.set_update_state(UpdateState::Installing);

    std::thread::spawn(move || {
        let progress = {
            let ui = ui.clone();
            move |fraction: f32| {
                let _ = ui.upgrade_in_event_loop(move |ui| {
                    ui.global::<AppState>().set_update_progress(fraction);
                });
            }
        };
        let result = update::install(&release, &progress);
        let _ = ui.upgrade_in_event_loop(move |ui| match result {
            Ok(()) => {
                eprintln!("update: installed {}, restarting", release.version);
                let _ = slint::quit_event_loop();
            }
            Err(err) => {
                eprintln!("update: install failed: {err}");
                let state = ui.global::<AppState>();
                state.set_update_error(err.into());
                state.set_update_state(UpdateState::Failed);
            }
        });
    });
}
