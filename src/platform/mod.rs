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

/// Put `text` on the clipboard.
pub fn copy_text(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::copy_text(text);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        Err("copying is not built for this system".into())
    }
}

/// Put a picture on the clipboard: the contents of an image file (JPEG, PNG, WebP).
pub fn copy_image(contents: &[u8]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::copy_image(contents);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = contents;
        Err("copying pictures is not built for this system".into())
    }
}

/// Show the system's open panel for files (`media`: pictures and videos only), and hand `chosen`
/// the files picked, on the UI thread once the panel closes; none when it was cancelled.
pub fn choose_files(media: bool, chosen: impl FnOnce(Vec<std::path::PathBuf>) + 'static) {
    #[cfg(target_os = "macos")]
    macos::choose_files(media, chosen);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = media;
        chosen(Vec::new());
    }
}

/// Show a file in the system's file manager (the Finder).
pub fn reveal_file(path: &str) {
    #[cfg(target_os = "macos")]
    macos::reveal_file(path);
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// The files on the clipboard (copied in the Finder), if any.
pub fn pasteboard_files() -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    return macos::pasteboard_files();
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

/// The picture on the clipboard as PNG, if there is one.
pub fn pasteboard_image() -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    return macos::pasteboard_image();
    #[cfg(not(target_os = "macos"))]
    None
}

/// Whether the system lets FinchGram capture the screen (macOS: Screen Recording, in System
/// Settings → Privacy & Security). Systems without the screenshot tool say no.
pub fn screen_capture_allowed() -> bool {
    #[cfg(target_os = "macos")]
    return macos::screen_capture_allowed();
    #[cfg(not(target_os = "macos"))]
    false
}

/// Ask the system for leave to capture the screen: macOS shows its dialog once.
pub fn request_screen_capture() {
    #[cfg(target_os = "macos")]
    macos::request_screen_capture();
}

/// Open the system's settings where screen capture is allowed.
pub fn open_screen_capture_settings() {
    #[cfg(target_os = "macos")]
    macos::open_screen_capture_settings();
}

/// Capture one display (counted from 1, as the system counts them) into a PNG at `path`; false
/// when the system refused.
pub fn capture_display(number: usize, path: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    return macos::capture_display(number, path);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (number, path);
        false
    }
}

/// A window on the screen, as the system lists them: where it is, in points of the global
/// coordinate space (the main display's top left corner is its origin), front to back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenWindow {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The windows on the screen, front to back, for the screenshot tool to offer: FinchGram's own
/// among them, unless it hid for the capture.
pub fn windows_on_screen() -> Vec<ScreenWindow> {
    #[cfg(target_os = "macos")]
    return macos::windows_on_screen();
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

/// Put the screenshot tool's overlay above everything else on its display, and give it the
/// keyboard.
pub fn raise_overlay(window: &slint::Window) {
    #[cfg(target_os = "macos")]
    macos::raise_overlay(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// A notification of a new message, as the system shows it (on macOS in Notification Center).
/// Systems without notifications in FinchGram yet show none, and have no badge either.
pub struct Notification {
    /// Its name, by which it is taken back.
    pub id: String,
    /// The chat it is from: the system keeps a chat's notifications together, and a click on one
    /// opens the chat.
    pub chat_id: i64,
    pub title: String,
    /// Who wrote it, in a group; else empty.
    pub subtitle: String,
    pub body: String,
    /// It plays the system's sound.
    pub sound: bool,
}

/// Clicks on FinchGram's notifications show the window and hand their chat to `open_chat`. Call
/// before the event loop runs, so that the click that started FinchGram counts too.
pub fn handle_notification_clicks(open_chat: fn(i64)) {
    #[cfg(target_os = "macos")]
    macos::handle_notification_clicks(open_chat);
    #[cfg(not(target_os = "macos"))]
    let _ = open_chat;
}

/// Ask the user whether FinchGram may show notifications. The system asks only once and remembers
/// the answer, which the user can change in its settings.
pub fn ask_to_notify() {
    #[cfg(target_os = "macos")]
    macos::ask_to_notify();
}

pub fn show_notification(notification: &Notification) {
    #[cfg(target_os = "macos")]
    macos::show_notification(notification);
    #[cfg(not(target_os = "macos"))]
    let _ = notification;
}

/// Take back the notifications with these ids, where they are still shown.
pub fn remove_notifications(ids: &[String]) {
    #[cfg(target_os = "macos")]
    macos::remove_notifications(ids);
    #[cfg(not(target_os = "macos"))]
    let _ = ids;
}

/// Take back every notification shown so far but those with these ids.
pub fn keep_only_notifications(ids: Vec<String>) {
    #[cfg(target_os = "macos")]
    macos::keep_only_notifications(ids);
    #[cfg(not(target_os = "macos"))]
    let _ = ids;
}

pub fn remove_all_notifications() {
    #[cfg(target_os = "macos")]
    macos::remove_all_notifications();
}

/// The number on FinchGram's icon (the Dock's on macOS); none for 0.
pub fn set_badge(count: i32) {
    #[cfg(target_os = "macos")]
    macos::set_badge(count);
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

/// Draw the eye to FinchGram's icon: on macOS the Dock icon bounces once. Nothing happens while
/// FinchGram is in front.
pub fn flash_icon() {
    #[cfg(target_os = "macos")]
    macos::flash_icon();
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

/// Open a web, mail or Telegram link with the system (the browser, the mail app, or whatever
/// handles tg: links).
/// Anything else, a file or another kind of link, is refused.
pub fn open_link(url: &str) {
    let allowed = ["https://", "http://", "tg://", "mailto:"].iter().any(|scheme| url.starts_with(scheme));
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
