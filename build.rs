use std::path::{Path, PathBuf};

fn main() {
    // Translations live in lang/<code>/LC_MESSAGES/finchgram.po and are compiled into the binary,
    // so no gettext runtime is needed on the user's machine. No default context: a string has one
    // translation wherever it appears, unless the .slint file gives it a context of its own
    // (@tr("context" => "text")).
    println!("cargo:rerun-if-changed=lang");

    let config = slint_build::CompilerConfiguration::new()
        .with_style("cupertino".into())
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI failed to compile");

    copy_bundled_tdlib();
}

/// Put vendor/tdlib/bin/finchgram-tdlib next to the executable cargo is about to produce, so
/// `cargo run` uses exactly the same program as the packaged app (see src/telegram/process.rs).
/// It is a hard requirement: FinchGram never talks to Telegram in any other way.
fn copy_bundled_tdlib() {
    println!("cargo:rerun-if-changed=vendor/tdlib/bin");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let bin = Path::new(&manifest_dir).join("vendor").join("tdlib").join("bin");

    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the executable lands in target/<profile>/.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let exe_dir = out_dir.ancestors().nth(3).expect("unexpected OUT_DIR layout").to_path_buf();

    let program = bin.join("finchgram-tdlib");
    assert!(
        program.is_file(),
        "\n\n{} is missing.\nFinchGram only ever talks to Telegram through its own finchgram-tdlib. \
         Run scripts/fetch-tdlib.sh once to download it into vendor/tdlib/bin/.\n\n",
        program.display()
    );
    let dest = exe_dir.join("finchgram-tdlib");
    std::fs::copy(&program, &dest)
        .unwrap_or_else(|err| panic!("cannot copy {} to {}: {err}", program.display(), dest.display()));
}
