//! The UI typefaces, compiled into the executable so every Mac shows the same letters whatever
//! fonts it has. Each theme asks for its own (ui/look.slint): IBM Plex Sans and IBM Plex Mono for
//! Workbench, Source Sans 3 for Broadsheet, JetBrains Mono for Terminal. Chinese, Japanese, Korean
//! and Arabic come from Noto Sans SC, JP, KR and Arabic in all three. The files come from
//! vendor/fonts/ (scripts/fetch-fonts.sh); build.rs fails early when they are missing.
//!
//! A letter the requested font does not have is looked up in the generic families, which are set
//! here to our own fonts: IBM Plex Sans, then the system's colour emoji font (Apple Color Emoji on
//! macOS), then the Noto fonts, the UI language's own first (a character the Chinese and Japanese
//! fonts both have is drawn differently by each). The emoji font comes before the Noto fonts
//! because those have black-and-white glyphs for some emoji (🆕, ㊙, ☺) and would otherwise be
//! found first; the text layout tries the emoji font last of all on its own. After them come Noto
//! Sans Math and Symbols 2, for the symbol blocks no script owns (the styled letters 𝐇𝐞𝐢𝐠𝐡𝐭 of
//! Telegram's ads, enclosed alphanumerics, arrows): fontique looks those up in the generic
//! families only, never in the system's fonts, so without them they were boxes (2026-10-24). What
//! none of them has (other scripts) comes from the system's fallback fonts.

use std::sync::{Arc, OnceLock};

use slint::fontique_011::fontique::{self, FamilyId, GenericFamily};

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/fonts/", $file)) as &'static [u8]
    };
}

// Statics, not consts: a const's bytes are laid down again wherever it is used, and with the two
// uses here (register, annotation_fonts) in different codegen units the linker kept two copies of
// all 39 MB of fonts (v0.3.25 was 39 MB larger than v0.3.24 for an unrelated change).

/// The fonts the themes name, and IBM Plex Sans first among the fallbacks.
static LATIN: &[&[u8]] = &[
    font!("IBMPlexSans-Variable.ttf"),
    font!("IBMPlexMono-Regular.ttf"),
    font!("IBMPlexMono-Medium.ttf"),
    font!("IBMPlexMono-SemiBold.ttf"),
    font!("SourceSans3-Variable.ttf"),
    font!("SourceSans3-Italic-Variable.ttf"),
    font!("JetBrainsMono-Variable.ttf"),
];
static CHINESE: &[&[u8]] = &[font!("NotoSansSC-Variable.ttf")];
static JAPANESE: &[&[u8]] = &[font!("NotoSansJP-Variable.ttf")];
static KOREAN: &[&[u8]] = &[font!("NotoSansKR-Variable.ttf")];
static ARABIC: &[&[u8]] = &[font!("NotoSansArabic-Variable.ttf")];
/// The symbol blocks no script owns, looked up last.
static SYMBOLS: &[&[u8]] = &[font!("NotoSansMath-Regular.ttf"), font!("NotoSansSymbols2-Regular.ttf")];

/// The fonts the screenshot tool draws its words with, into the picture (src/screenshot/export.rs):
/// IBM Plex Sans, and the Noto fonts for the other scripts.
pub fn annotation_fonts() -> [&'static [u8]; 5] {
    [LATIN[0], CHINESE[0], JAPANESE[0], KOREAN[0], ARABIC[0]]
}

/// The registered families, by script, for the order of the fallbacks.
struct Families {
    /// IBM Plex Sans, the first Latin family registered.
    latin: Option<FamilyId>,
    chinese: Vec<FamilyId>,
    japanese: Vec<FamilyId>,
    korean: Vec<FamilyId>,
    arabic: Vec<FamilyId>,
    symbols: Vec<FamilyId>,
}

static FAMILIES: OnceLock<Families> = OnceLock::new();

/// Register the bundled fonts with Slint and make them the fallbacks. Needs the Slint platform
/// (after the backend is selected) and must run before the first text is laid out.
pub fn register() {
    let families = {
        let mut collection = slint::fontique_011::shared_collection();
        let mut register = |files: &[&'static [u8]]| -> Vec<FamilyId> {
            let mut families = Vec::new();
            for data in files {
                let blob = fontique::Blob::new(Arc::new(*data));
                for (family, _) in collection.register_fonts(blob, None) {
                    if !families.contains(&family) {
                        families.push(family);
                    }
                }
            }
            families
        };
        Families {
            latin: register(LATIN).first().copied(),
            chinese: register(CHINESE),
            japanese: register(JAPANESE),
            korean: register(KOREAN),
            arabic: register(ARABIC),
            symbols: register(SYMBOLS),
        }
    };
    let _ = FAMILIES.set(families);
    prefer("en");
}

/// Order the fallbacks for the UI language `code` ("ja", "zh_Hant", …): IBM Plex Sans, the
/// system's colour emoji font, then the language's own script's font, then the other Noto fonts,
/// then the symbol fonts.
pub fn prefer(code: &str) {
    let Some(families) = FAMILIES.get() else { return };
    let scripts: [&Vec<FamilyId>; 4] = match code.split(['-', '_']).next().unwrap_or_default() {
        "ja" => [&families.japanese, &families.chinese, &families.korean, &families.arabic],
        "ko" => [&families.korean, &families.chinese, &families.japanese, &families.arabic],
        "ar" => [&families.arabic, &families.chinese, &families.japanese, &families.korean],
        _ => [&families.chinese, &families.japanese, &families.korean, &families.arabic],
    };
    let mut collection = slint::fontique_011::shared_collection();
    // The system's emoji font, as fontique names it for the platform (Apple Color Emoji, Segoe UI
    // Emoji, fontconfig's "emoji").
    let emoji: Vec<FamilyId> = collection.generic_families(GenericFamily::Emoji).collect();
    let order: Vec<FamilyId> = families.latin.into_iter().chain(emoji).chain(scripts.into_iter().flatten().copied()).chain(families.symbols.iter().copied()).collect();
    for generic in [GenericFamily::SansSerif, GenericFamily::SystemUi, GenericFamily::UiSansSerif] {
        collection.set_generic_families(generic, order.iter().copied());
    }
}
