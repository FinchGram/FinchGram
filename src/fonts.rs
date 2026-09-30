//! The UI typefaces, compiled into the executable so every Mac shows the same letters whatever
//! fonts it has. Each theme asks for its own (ui/look.slint): IBM Plex Sans and IBM Plex Mono for
//! Workbench, Source Sans 3 for Broadsheet, JetBrains Mono for Terminal. Chinese comes from Noto
//! Sans SC in all three. The files come from vendor/fonts/ (scripts/fetch-fonts.sh); build.rs fails
//! early when they are missing.
//!
//! A letter the requested font does not have is looked up in the generic families, which are set
//! here to our own fonts: IBM Plex Sans, then Noto Sans SC. What neither has (other scripts, emoji
//! in messages) comes from the system's fallback fonts.

use std::sync::Arc;

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
const SIMPLIFIED_CHINESE: &[&[u8]] = &[font!("NotoSansSC-Variable.ttf")];

/// Register the bundled fonts with Slint and make them the fallbacks. Needs the Slint platform
/// (after the backend is selected) and must run before the first text is laid out.
pub fn register() {
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
    let latin = register(LATIN);
    let chinese = register(SIMPLIFIED_CHINESE);

    // IBM Plex Sans is the first Latin family registered.
    let order: Vec<FamilyId> = latin.first().into_iter().chain(chinese.iter()).copied().collect();
    for generic in [GenericFamily::SansSerif, GenericFamily::SystemUi, GenericFamily::UiSansSerif] {
        collection.set_generic_families(generic, order.iter().copied());
    }
}
