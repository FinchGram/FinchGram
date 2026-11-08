use std::path::PathBuf;

fn main() {
    // ui/ and lang/ stay at the top of the repository, two folders up.
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.join("../..").canonicalize().expect("the repository's top folder");

    // Translations live in lang/<code>/LC_MESSAGES/finchgram.po and are compiled into the binary,
    // so no gettext runtime is needed on the user's machine. No default context: a string has one
    // translation wherever it appears, unless the .slint file gives it a context of its own
    // (@tr("context" => "text")).
    let lang = root.join("lang");
    println!("cargo:rerun-if-changed={}", lang.display());
    // Slint names the translation domain after the package, which would make it look for
    // finchgram-ui.po; the catalogs are the app's, finchgram.po, as before the UI had a package of
    // its own. SAFETY: nothing else runs in this build script yet.
    unsafe { std::env::set_var("CARGO_PKG_NAME", "finchgram") };

    let config = slint_build::CompilerConfiguration::new()
        .with_style("cupertino".into())
        .with_bundled_translations(lang)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config(root.join("ui").join("app.slint"), config).expect("Slint UI failed to compile");
}
