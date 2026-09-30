//! The platform layer (docs/architecture.md): the few things the shell does differently on each
//! operating system. Everything else stays free of platform code.

#[cfg(target_os = "macos")]
mod macos;

use slint::ComponentHandle;

use crate::{MainWindow, PlatformWords};

/// Show the window and run until FinchGram quits. On macOS closing the window only hides it, as
/// the design's "When closing the window: Minimize to tray" (the default) has it: FinchGram stays
/// in the Dock, a click on the Dock icon shows the window again, and Quit (⌘Q, the Dock's menu)
/// runs `before_quit` first. Elsewhere there is no tray icon yet to come back from, so closing the
/// window quits.
pub fn run(ui: &MainWindow, before_quit: fn()) -> Result<(), slint::PlatformError> {
    ui.show()?;
    #[cfg(target_os = "macos")]
    {
        macos::install(ui, before_quit);
        slint::run_event_loop_until_quit()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = before_quit;
        slint::run_event_loop()
    }
}

/// Bring the window back: shown, not minimized, in front.
pub fn show_window(ui: &MainWindow) {
    ui.window().set_minimized(false);
    if let Err(err) = ui.show() {
        eprintln!("platform: cannot show the window: {err}");
    }
    #[cfg(target_os = "macos")]
    macos::bring_to_front();
}

/// FinchGram's icon in the menu bar (macOS), with a menu to open the window or quit; or none. Its
/// words are the UI language's: after a change of language it is made again.
pub fn set_menu_bar_icon(ui: &MainWindow, shown: bool) {
    #[cfg(target_os = "macos")]
    {
        let words = ui.global::<PlatformWords>();
        macos::set_menu_bar_icon(shown, &words.get_open(), &words.get_quit());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (ui, shown);
}

/// Whether FinchGram opens when the user logs in: None where the system has no such list for apps
/// (before macOS 13, and elsewhere so far).
pub fn launch_at_login() -> Option<bool> {
    #[cfg(target_os = "macos")]
    return macos::launch_at_login();
    #[cfg(not(target_os = "macos"))]
    None
}

pub fn set_launch_at_login(on: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::set_launch_at_login(on);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = on;
        Err("launching at login is not built for this system".into())
    }
}

/// What the account's list of sessions calls this device (TDLib's device_model, which must not be
/// empty). TDLib finds the system's version by itself.
pub fn device_model() -> &'static str {
    if cfg!(target_os = "macos") { "Mac" } else { "PC" }
}

/// Select Slint's backend, with the window the design draws: on macOS the title bar is
/// transparent and the pages run underneath it, the window buttons over each theme's own title
/// bar.
pub fn select_backend() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(|attributes| {
            #[cfg(target_os = "macos")]
            let attributes = {
                use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
                attributes.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true)
            };
            attributes
        })
        .select()
}

/// Open a web or Telegram link with the system (the browser, or whatever handles tg: links).
/// Anything else, a file or another kind of link, is refused.
pub fn open_link(url: &str) {
    let allowed = ["https://", "http://", "tg://"].iter().any(|scheme| url.starts_with(scheme));
    if !allowed {
        eprintln!("platform: not opening {url:?}: only web and Telegram links are opened");
        return;
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(err) = result {
        eprintln!("platform: cannot open {url}: {err}");
    }
}
