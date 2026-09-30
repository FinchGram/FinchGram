//! FinchGram — an open-source Telegram desktop client with a media center.
//!
//! This is the shell: the pages, the settings and the app's state. Telegram itself is reached only
//! through finchgram-tdlib, a separate program next to this executable (src/telegram/,
//! docs/architecture.md).

mod fonts;
mod i18n;
mod images;
mod platform;
#[cfg(test)]
mod screenshots;
mod settings;
mod telegram;
mod update;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ModelRc, VecModel};

use settings::Settings;

slint::include_modules!();

/// How often a running app looks at the clock to see whether its daily update check is due
/// (the check itself happens once a day, at a random moment; see update.rs). No network here.
const UPDATE_SCHEDULE_TICK: Duration = Duration::from_secs(60);

fn main() -> Result<(), slint::PlatformError> {
    platform::select_backend()?;
    // Our own fonts, before the first text is laid out.
    fonts::register();

    let ui = MainWindow::new()?;
    let state = ui.global::<AppState>();
    let settings = Rc::new(RefCell::new(Settings::load()));

    // Language: saved choice > system locale > English.
    // (Bundled translations can only be selected once a component exists.)
    let languages: Vec<Language> = i18n::LANGUAGES
        .iter()
        .map(|language| Language { code: language.code.into(), native: language.native.into(), english: language.english.into() })
        .collect();
    state.set_languages(ModelRc::new(VecModel::from(languages)));
    let language = i18n::initial_language(&settings.borrow().language);
    i18n::apply(&language);
    state.set_language(language.clone().into());
    state.set_language_name(i18n::native_name(&language).into());
    if settings.borrow().language != language {
        settings.borrow_mut().language = language;
        settings.borrow().save();
    }

    state.set_appearance(settings.borrow().appearance.clone().into());
    state.set_theme(theme_from_name(&settings.borrow().theme));
    state.set_check_for_updates_daily(settings.borrow().check_for_updates);
    state.set_app_version(update::CURRENT_VERSION.into());

    state.on_change_language({
        let ui = ui.as_weak();
        let settings = settings.clone();
        move |code| {
            if !i18n::apply(&code) {
                return;
            }
            settings.borrow_mut().language = code.to_string();
            settings.borrow().save();
            if let Some(ui) = ui.upgrade() {
                let state = ui.global::<AppState>();
                state.set_language_name(i18n::native_name(&code).into());
                state.set_language(code);
            }
            telegram::language_changed();
        }
    });
    state.on_change_appearance({
        let ui = ui.as_weak();
        let settings = settings.clone();
        move |appearance| {
            if !matches!(appearance.as_str(), "system" | "light" | "dark") {
                return;
            }
            settings.borrow_mut().appearance = appearance.to_string();
            settings.borrow().save();
            if let Some(ui) = ui.upgrade() {
                ui.global::<AppState>().set_appearance(appearance.clone());
                apply_window_appearance(&ui, &appearance);
            }
        }
    });
    // A new theme shows at once: the three share every model and callback, only the pages differ.
    state.on_change_theme({
        let ui = ui.as_weak();
        let settings = settings.clone();
        move |theme| {
            settings.borrow_mut().theme = theme_name(theme).to_string();
            settings.borrow().save();
            if let Some(ui) = ui.upgrade() {
                ui.global::<AppState>().set_theme(theme);
            }
        }
    });
    state.on_change_check_for_updates_daily({
        let ui = ui.as_weak();
        let settings = settings.clone();
        move |daily| {
            settings.borrow_mut().check_for_updates = daily;
            settings.borrow().save();
            if let Some(ui) = ui.upgrade() {
                ui.global::<AppState>().set_check_for_updates_daily(daily);
            }
        }
    });
    state.on_open_url(|url| {
        // Only our own pages, and Telegram's terms of service when signing up, are opened from here.
        if url.starts_with("https://github.com/FinchGram/") || url == "https://telegram.org/tos" {
            platform::open_link(&url);
        }
    });
    state.on_move_window({
        let ui = ui.as_weak();
        move || {
            use slint::winit_030::WinitWindowAccessor;
            if let Some(ui) = ui.upgrade() {
                ui.window().with_winit_window(|window| {
                    let _ = window.drag_window();
                });
            }
        }
    });
    state.on_zoom_window({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                let window = ui.window();
                window.set_maximized(!window.is_maximized());
            }
        }
    });

    telegram::start(&ui);

    // Updates: the newest release found by the last check, kept here so "install" knows what
    // to download. Checked once a day at a random moment (unless switched off in Settings →
    // About), and whenever the user asks (Help menu, or the button in Settings → About).
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
    update_timer.start(slint::TimerMode::Repeated, UPDATE_SCHEDULE_TICK, {
        let ui = ui.as_weak();
        let pending = pending.clone();
        let settings = settings.clone();
        move || {
            if settings.borrow().check_for_updates {
                automatic_update_check(ui.clone(), pending.clone());
            }
        }
    });
    if settings.borrow().check_for_updates {
        automatic_update_check(ui.as_weak(), pending);
    }

    // The window's own appearance (its buttons, its edge) follows the choice in Settings.
    apply_window_appearance_when_ready(ui.as_weak(), settings.borrow().appearance.clone(), 40);

    let result = ui.run();
    // TDLib writes its database out before finchgram-tdlib ends.
    telegram::shut_down();
    result
}

fn theme_from_name(name: &str) -> Theme {
    match name {
        "broadsheet" => Theme::Broadsheet,
        "terminal" => Theme::Terminal,
        _ => Theme::Workbench,
    }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Workbench => "workbench",
        Theme::Broadsheet => "broadsheet",
        Theme::Terminal => "terminal",
    }
}

/// The native window exists only once the event loop runs: try every 50 ms until it is there.
fn apply_window_appearance_when_ready(ui: slint::Weak<MainWindow>, appearance: String, attempts: u32) {
    let Some(window) = ui.upgrade() else { return };
    if apply_window_appearance(&window, &appearance) || attempts == 0 {
        return;
    }
    slint::Timer::single_shot(Duration::from_millis(50), move || {
        apply_window_appearance_when_ready(ui, appearance, attempts - 1)
    });
}

/// Light or dark window buttons and edge to match the app; "system" follows macOS. False while
/// the native window does not exist yet.
fn apply_window_appearance(ui: &MainWindow, appearance: &str) -> bool {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::window::Theme;
    let theme = match appearance {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    };
    ui.window().with_winit_window(|window| window.set_theme(theme)).is_some()
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
