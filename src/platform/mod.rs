//! The platform layer (docs/architecture.md): the few things the shell does differently on each
//! operating system. Everything else stays free of platform code.

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
