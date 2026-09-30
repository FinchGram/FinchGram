//! UI language selection.
//!
//! English is the source language of every `@tr("...")` in the .slint files. The other languages
//! live in `lang/<code>/LC_MESSAGES/finchgram.po` and are bundled into the binary by build.rs, so
//! switching happens instantly at runtime with no external files. The chosen language is
//! remembered in the settings file (see settings.rs).

pub const ENGLISH: &str = "en";

/// A language the app ships: its folder name under lang/ ("en" has none) and its name, in itself
/// and in English, for the language list.
pub struct UiLanguage {
    pub code: &'static str,
    pub native: &'static str,
    pub english: &'static str,
}

/// Every language the app ships, in the order the language list shows them.
pub const LANGUAGES: &[UiLanguage] = &[
    UiLanguage { code: "en", native: "English", english: "English" },
    UiLanguage { code: "zh_Hans", native: "简体中文", english: "Chinese (Simplified)" },
];

fn is_supported(code: &str) -> bool {
    LANGUAGES.iter().any(|language| language.code == code)
}

/// The language's name in itself, for the language picker's button.
pub fn native_name(code: &str) -> &'static str {
    LANGUAGES.iter().find(|language| language.code == code).map_or("English", |language| language.native)
}

/// The language to start with: the saved choice if it is one we ship, else the system locale,
/// else English.
pub fn initial_language(saved: &str) -> String {
    if is_supported(saved) {
        return saved.to_string();
    }
    from_system_locale().unwrap_or_else(|| ENGLISH.to_string())
}

/// Switch every `@tr` in the UI to `code`. Returns false, leaving the UI unchanged, if that
/// language is not bundled.
pub fn apply(code: &str) -> bool {
    if !is_supported(code) {
        eprintln!("i18n: {code:?} is not a bundled language");
        return false;
    }
    match slint::select_bundled_translation(code) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("i18n: cannot select language {code:?}: {err:?}");
            false
        }
    }
}

/// Map the system locale (e.g. "zh-Hans-CN", "en-US") onto a supported language. Until there is
/// a Traditional Chinese translation, every Chinese locale gets Simplified Chinese.
fn from_system_locale() -> Option<String> {
    let locale = sys_locale::get_locale()?.to_ascii_lowercase();
    let language = locale.split(['-', '_']).next()?;
    let code = match language {
        "zh" => "zh_Hans",
        _ => return None,
    };
    Some(code.to_string())
}
