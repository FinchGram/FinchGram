use std::path::{Path, PathBuf};

// The UI (ui/, lang/) is compiled by crates/finchgram-ui/build.rs.
fn main() {
    copy_bundled_tdlib();
    link_bundled_mpv();
    check_bundled_fonts();
    check_bundled_skia();
}

/// Skia draws the window (Slint's Skia renderer, docs/skia.md). The skia-bindings crate links the
/// archive vendor/skia/build.sh makes, and finds it by the address in SKIA_BINARIES_URL, which
/// scripts/fetch-skia.sh writes into .cargo/config.toml. Left to itself the crate would download
/// rust-skia's own archive: this stops the build unless the address points into vendor/skia/bin/
/// and an archive is there. (Cargo.toml's `no-compile` stops skia-bindings from building Skia.)
fn check_bundled_skia() {
    println!("cargo:rerun-if-env-changed=SKIA_BINARIES_URL");
    println!("cargo:rerun-if-changed=vendor/skia/bin");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let folder = Path::new(&manifest_dir).join("vendor").join("skia").join("bin");
    let address = std::env::var("SKIA_BINARIES_URL").unwrap_or_default();
    let named = address.strip_prefix("file://").map(Path::new).is_some_and(|archive| archive.parent() == Some(folder.as_path()));
    let there = std::fs::read_dir(&folder).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("skia-binaries-") && name.ends_with(".tar.gz")
        })
    });
    assert!(
        named && there,
        "\n\nSkia is missing from {}, or cargo was not told where it is (SKIA_BINARIES_URL is {address:?}).\n\
         FinchGram draws its window with its own build of Skia. \
         Run scripts/fetch-skia.sh once to download it into vendor/skia/bin/.\n\n",
        folder.display()
    );
}

/// Where cargo puts the executable it is about to produce (`cargo build`, `cargo run`):
/// target/<profile>, the folder its own "build" folder is in. OUT_DIR is
/// target/<profile>/build/<package>-<hash>/out up to cargo 1.99 and
/// target/<profile>/build/<package>/<hash>/out since 1.100, so the folder is found by its name,
/// not by counting up from OUT_DIR: counting put what is copied here into target/<profile>/build
/// under 1.100, where neither `cargo run` nor the tests found it.
fn executable_dir() -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let build = out_dir.ancestors().find(|folder| folder.file_name().is_some_and(|name| name == "build"));
    let build = build.unwrap_or_else(|| panic!("unexpected OUT_DIR layout: {} is in no folder called build", out_dir.display()));
    build.parent().expect("cargo's build folder is in the profile's").to_path_buf()
}

/// libmpv plays video (src/player/). The app links vendor/mpv/bin/libmpv.2.dylib and finds it at
/// run time through its run paths: next to the executable (`cargo run`: it is copied there), in
/// target/<profile> from where cargo puts a test executable, and in the bundle's Frameworks
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
    // A test executable is in target/<profile>/deps up to cargo 1.99: one folder up.
    for path in ["@executable_path", "@executable_path/..", "@executable_path/../Frameworks"] {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
    }
    // Since cargo 1.100 it is in target/<profile>/build/<package>/<hash>/out, as deep as OUT_DIR
    // is. `cargo test` finds the library all the same (it has the dynamic linker fall back to
    // target/<profile>); this is for a test executable run by anything else, a debugger say.
    // Never in a release build: the app that ships looks nowhere outside its bundle.
    let exe_dir = executable_dir();
    let depth = out_dir.strip_prefix(&exe_dir).map_or(0, |below| below.components().count());
    if depth > 3 && std::env::var("PROFILE").is_ok_and(|profile| profile != "release") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path{}", "/..".repeat(depth));
    }

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

    // The executable lands in target/<profile>.
    let exe_dir = executable_dir();

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
