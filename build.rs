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
    link_bundled_mpv();
    check_bundled_fonts();
}

/// libmpv plays video (src/player/). The app links vendor/mpv/bin/libmpv.2.dylib and finds it at
/// run time through its run paths: next to the executable (`cargo run`: it is copied there), one
/// folder up (the test executables in target/<profile>/deps/), and in the bundle's Frameworks
/// folder (scripts/bundle.sh). Like finchgram-tdlib, it is never looked for anywhere else.
fn link_bundled_mpv() {
    println!("cargo:rerun-if-changed=vendor/mpv/bin");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let library = Path::new(&manifest_dir).join("vendor").join("mpv").join("bin").join("libmpv.2.dylib");
    assert!(
        library.is_file(),
        "\n\n{} is missing.\nFinchGram plays video with its own libmpv. \
         Run scripts/fetch-mpv.sh once to download it into vendor/mpv/bin/.\n\n",
        library.display()
    );

    // src/player/mpv.rs links "mpv": a libmpv.dylib here stands for the versioned file.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let link_dir = out_dir.join("mpv");
    std::fs::create_dir_all(&link_dir).expect("OUT_DIR/mpv");
    let link = link_dir.join("libmpv.dylib");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&library, &link).unwrap_or_else(|err| panic!("cannot link {}: {err}", link.display()));
    println!("cargo:rustc-link-search=native={}", link_dir.display());
    for path in ["@executable_path", "@executable_path/..", "@executable_path/../Frameworks"] {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
    }

    let exe_dir = out_dir.ancestors().nth(3).expect("unexpected OUT_DIR layout").to_path_buf();
    let dest = exe_dir.join("libmpv.2.dylib");
    std::fs::copy(&library, &dest)
        .unwrap_or_else(|err| panic!("cannot copy {} to {}: {err}", library.display(), dest.display()));
}

/// The UI fonts are compiled into the executable (src/fonts.rs includes them from vendor/fonts/).
/// Fail with a clear message instead of a bare include_bytes! error when they were never fetched.
fn check_bundled_fonts() {
    println!("cargo:rerun-if-changed=vendor/fonts");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let probe = Path::new(&manifest_dir).join("vendor").join("fonts").join("NotoSansSC-Variable.ttf");
    assert!(
        probe.is_file(),
        "\n\n{} is missing.\nThe UI fonts ship inside the app. \
         Run scripts/fetch-fonts.sh once to download them into vendor/fonts/.\n\n",
        probe.display()
    );
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
