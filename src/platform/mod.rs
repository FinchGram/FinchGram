//! The platform layer (docs/architecture.md): the few things the shell does differently on each
//! operating system. Everything else stays free of platform code.

/// What the account's list of sessions calls this device (TDLib's device_model, which must not be
/// empty). TDLib finds the system's version by itself.
pub fn device_model() -> &'static str {
    if cfg!(target_os = "macos") { "Mac" } else { "PC" }
}
