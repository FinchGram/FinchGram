//! The UI typefaces, compiled into the executable so every Mac shows the same letters whatever
//! fonts it has. Each theme asks for its own (ui/look.slint): IBM Plex Sans and IBM Plex Mono for
//! Workbench, Source Sans 3 for Broadsheet, JetBrains Mono for Terminal. Chinese, Japanese, Korean
//! and Arabic come from Noto Sans SC, JP, KR and Arabic in all three. The files come from
//! vendor/fonts/ (scripts/fetch-fonts.sh); build.rs fails early when they are missing.
//!
//! A letter the requested font does not have is looked up in the generic families, which are set
//! here to our own fonts: IBM Plex Sans, then the Noto fonts, the UI language's own first (a
//! character the Chinese and Japanese fonts both have is drawn differently by each). What none of
//! them has (other scripts, emoji in messages) comes from the system's fallback fonts.

use std::sync::{Arc, OnceLock};

use slint::fontique_011::fontique::{self, FamilyId, GenericFamily};

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/fonts/", $file)) as &'static [u8]
    };
}

/// The fonts the themes name, and IBM Plex Sans first among the fallbacks.
const LATIN: &[&[u8]] = &[
    font!("IBMPlexSans-Variable.ttf"),
    font!("IBMPlexMono-Regular.ttf"),
    font!("IBMPlexMono-Medium.ttf"),
    font!("IBMPlexMono-SemiBold.ttf"),
    font!("SourceSans3-Variable.ttf"),
    font!("SourceSans3-Italic-Variable.ttf"),
    font!("JetBrainsMono-Variable.ttf"),
];
const CHINESE: &[&[u8]] = &[font!("NotoSansSC-Variable.ttf")];
const JAPANESE: &[&[u8]] = &[font!("NotoSansJP-Variable.ttf")];
const KOREAN: &[&[u8]] = &[font!("NotoSansKR-Variable.ttf")];
const ARABIC: &[&[u8]] = &[font!("NotoSansArabic-Variable.ttf")];

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
        }
    };
    let _ = FAMILIES.set(families);
    prefer("en");
}

/// Order the fallbacks for the UI language `code` ("ja", "zh_Hant", …): IBM Plex Sans, then the
/// language's own script's font, then the other Noto fonts.
pub fn prefer(code: &str) {
    let Some(families) = FAMILIES.get() else { return };
    let scripts: [&Vec<FamilyId>; 4] = match code.split(['-', '_']).next().unwrap_or_default() {
        "ja" => [&families.japanese, &families.chinese, &families.korean, &families.arabic],
        "ko" => [&families.korean, &families.chinese, &families.japanese, &families.arabic],
        "ar" => [&families.arabic, &families.chinese, &families.japanese, &families.korean],
        _ => [&families.chinese, &families.japanese, &families.korean, &families.arabic],
    };
    let order: Vec<FamilyId> = families.latin.into_iter().chain(scripts.into_iter().flatten().copied()).collect();
    let mut collection = slint::fontique_011::shared_collection();
    for generic in [GenericFamily::SansSerif, GenericFamily::SystemUi, GenericFamily::UiSansSerif] {
        collection.set_generic_families(generic, order.iter().copied());
    }
}
