#!/usr/bin/env bash
# Builds Skia for macOS (Apple silicon) from pinned sources, as the archive the skia-bindings crate
# links: Skia's static libraries with the Rust bindings generated for them. rust-skia publishes
# such an archive for every version of its crates and skia-bindings downloads it by default; this
# builds the same one here instead, so that nothing prebuilt by a third party goes into the app
# (docs/conventions.md). Slint's Skia renderer draws the window with it.
#
#   vendor/skia/build.sh           # everything: fetch, build, check, package
#   vendor/skia/build.sh build     # a single step: fetch build check package install
#   vendor/skia/build.sh install   # unpack the package into vendor/skia/bin/, for local development only
#
# The package goes to vendor/skia/dist/, intermediate files to vendor/skia/work/, and what the app's
# build uses lives in vendor/skia/bin/ (scripts/fetch-skia.sh fills it from a release). None of the
# three is tracked by git. Releases are built by CI from a skia-* tag (.github/workflows/skia.yml).
# Everything below the "pinned versions" block is mechanism. To upgrade (a new Slint asks for a new
# skia-safe), only that block and Cargo.lock beside this script change (docs/skia.md).
set -euo pipefail

# ---- pinned versions ------------------------------------------------------------------
# The skia-bindings crate of the app's Cargo.lock, which Slint's Skia renderer asks for.
SKIA_BINDINGS_VERSION="0.153.3"
# The commit of rust-skia that version was published from (.cargo_vcs_info.json in the crate): the
# archive is named by it, and the crate looks for exactly that name.
RUST_SKIA_GIT="https://github.com/rust-skia/rust-skia.git"
RUST_SKIA_COMMIT="b7f043e0b1e2a850e702245de811f01df1f6b0ff" # 2026-09-04
# Skia itself: the submodule that commit pins, rust-skia's fork of Skia at its tag m153-0.101.2.
# Skia's own third-party code (expat, libjpeg-turbo, libpng, zlib, wuffs) comes at the commits its
# DEPS file pins.
SKIA_COMMIT="61e7ca4e99062cdd0ab69445d5963fb3365778f6"
# What Slint's Skia renderer asks of skia-safe on macOS, beside skia-safe's defaults (pdf, jpeg).
FEATURES='"gl", "metal"'
# The name those features, that commit and the target give the archive. The app's build asks for
# skia-binaries-<this>.tar.gz, so a build that comes out under another name is of no use: checked.
KEY="b7f043e0b1e2a850e702-aarch64-apple-darwin-ganesh-gl-jpegd-jpege-metal-pdf"

MACOS_MIN="12.0" # oldest macOS the libraries run on; keep in sync with the app's LSMinimumSystemVersion
ARCH="arm64"
TARGET="aarch64-apple-darwin"
# ---------------------------------------------------------------------------------------

HERE="$(cd "$(dirname "$0")" && pwd)" # vendor/skia
WORK="$HERE/work"
SRC="$WORK/src"
CRATE="$WORK/crate"   # a crate of one line that depends on skia-safe: what cargo builds
EXPORT="$WORK/export" # skia-bindings exports its archive's contents here
LOGS="$WORK/logs"
DIST="$HERE/dist"
ARCHIVE="skia-binaries-$KEY.tar.gz"
PACKAGE="skia-$SKIA_BINDINGS_VERSION-macos-$ARCH"
SKIA="$SRC/rust-skia/skia-bindings/skia"
mkdir -p "$SRC" "$LOGS"

export MACOSX_DEPLOYMENT_TARGET="$MACOS_MIN"
export CC=clang CXX=clang++

# ---- helpers ---------------------------------------------------------------------------

log() { printf '\n==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required: $2"; }

# Check out exactly one commit (no branch or tag can move it later), with the submodules that
# commit pins.
fetch_git() { # name url commit
  local name=$1 url=$2 commit=$3 dir="$SRC/$1"
  [ -n "$commit" ] || die "no commit pinned for $name"
  if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD)" = "$commit" ]; then
    git -C "$dir" submodule update -q --init --recursive --depth 1
    return
  fi
  log "cloning $name @ $commit"
  rm -rf "$dir"
  mkdir -p "$dir"
  git -C "$dir" init -q
  git -C "$dir" remote add origin "$url"
  git -C "$dir" fetch -q --depth 1 origin "$commit"
  git -C "$dir" checkout -q FETCH_HEAD
  [ "$(git -C "$dir" rev-parse HEAD)" = "$commit" ] || die "$name: checked-out commit is not $commit"
  git -C "$dir" submodule update -q --init --recursive --depth 1
}

# Run a build function in a subshell with its output in a log file; show the tail on failure.
run_logged() { # name function
  local name=$1 status
  log "building $name  (log: vendor/skia/work/logs/$name.log)"
  set +e
  ( set -e; "$2" ) >"$LOGS/$name.log" 2>&1
  status=$?
  set -e
  if [ "$status" -ne 0 ]; then
    echo "--- $name failed (exit $status); last 60 lines of $LOGS/$name.log ---" >&2
    tail -60 "$LOGS/$name.log" >&2
    exit 1
  fi
}

# ---- steps ------------------------------------------------------------------------------

step_fetch() {
  need git "it comes with Xcode's command line tools"
  need cargo "install Rust (rustup.rs)"
  need ninja "brew install ninja"
  need python3 "brew install python"
  fetch_git rust-skia "$RUST_SKIA_GIT" "$RUST_SKIA_COMMIT"
  local skia
  skia="$(git -C "$SKIA" rev-parse HEAD)"
  [ "$skia" = "$SKIA_COMMIT" ] || die "Skia is at $skia, not at the pinned $SKIA_COMMIT"
  # Skia's third-party code, each repository at the commit Skia's DEPS file names, and gn, the
  # program that writes Skia's build files (Skia's own bin/fetch-gn, at the version DEPS names):
  # a tool of the build, like ninja, not part of what is built.
  log "fetching what Skia's DEPS file pins"
  (cd "$SRC/rust-skia/skia-bindings" &&
    GIT_SYNC_DEPS_PATH=skia/DEPS GIT_SYNC_DEPS_SKIP_EMSDK=1 python3 skia/tools/git-sync-deps)
  log "all sources fetched and verified"
}

# skia-bindings' own build script builds Skia (gn, then ninja) and generates the bindings, as it
# would for anybody without a prebuilt archive, and writes out what its archive holds:
# FORCE_SKIA_BUILD keeps it from downloading rust-skia's, BUILD_ARTIFACTSTAGINGDIRECTORY is where
# it exports to. The crate built is one of our own that only depends on skia-safe with the
# features above, so that the set, and the archive's name, are the app's; its Cargo.lock, beside
# this script, pins the crates the build script itself uses (bindgen, cc, …).
build_skia() {
  rm -rf "$EXPORT" "$CRATE"
  mkdir -p "$CRATE/src"
  cat >"$CRATE/Cargo.toml" <<TOML
[package]
name = "finchgram-skia-build"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
skia-safe = { path = "../src/rust-skia/skia-safe", features = [$FEATURES] }
TOML
  : >"$CRATE/src/lib.rs"
  cp "$HERE/Cargo.lock" "$CRATE/Cargo.lock"
  cd "$CRATE"
  FORCE_SKIA_BUILD=1 BUILD_ARTIFACTSTAGINGDIRECTORY="$EXPORT" CARGO_TARGET_DIR="$WORK/target" \
    cargo build --locked --release --target "$TARGET"
}

step_build() {
  [ -d "$SKIA/third_party/externals" ] || die "no sources in vendor/skia/work/src: run 'vendor/skia/build.sh fetch' first"
  [ -f "$HERE/Cargo.lock" ] || die "vendor/skia/Cargo.lock is missing"
  run_logged skia build_skia
}

step_check() {
  local dir="$EXPORT/skia-binaries" key tag library archs oldest
  [ -d "$dir" ] || die "$dir is missing: run 'vendor/skia/build.sh build' first"
  log "checking the archive's contents"
  key="$(cat "$dir/key.txt")"
  [ "$key" = "$KEY" ] || die "the build is named $key, the app's build asks for $KEY"
  tag="$(cat "$dir/tag.txt")"
  [ "$tag" = "$SKIA_BINDINGS_VERSION" ] || die "the build is of skia-bindings $tag, not $SKIA_BINDINGS_VERSION"
  [ -s "$dir/bindings.rs" ] || die "bindings.rs is missing"
  for library in libskia.a libskia-bindings.a; do
    [ -f "$dir/$library" ] || die "$library is missing"
    archs="$(lipo -archs "$dir/$library")"
    [ "$archs" = "$ARCH" ] || die "$library is for $archs, not $ARCH"
    # Every object in the library says the oldest macOS it runs on.
    oldest="$(otool -l "$dir/$library" | awk '$1 == "minos" { print $2 }' | sort -u | tr '\n' ' ')"
    [ "$oldest" = "$MACOS_MIN " ] || die "$library is built for macOS $oldest, not $MACOS_MIN"
    printf '  %s: %s, macOS %s and newer, %s bytes\n' "$library" "$archs" "$MACOS_MIN" "$(stat -f %z "$dir/$library")"
  done
  printf '  named %s\n' "$KEY"
}

write_build_info() {
  cat <<INFO
$ARCHIVE: Skia as static libraries, with the Rust bindings of skia-bindings $SKIA_BINDINGS_VERSION, for macOS $ARCH
(runs on macOS $MACOS_MIN and newer)
built $(date -u +%Y-%m-%dT%H:%M:%SZ) on macOS $(sw_vers -productVersion), $(xcodebuild -version 2>/dev/null | head -1 | tr -d '\n'), $(clang --version | head -1), $(rustc --version)

statically linked:
  Skia           git $SKIA_COMMIT (rust-skia's fork, m153-0.101.2), BSD-3-Clause (LICENSE-Skia.txt)
  expat          $(git -C "$SKIA/third_party/externals/expat" rev-parse HEAD), MIT (LICENSE-expat.txt)
  libjpeg-turbo  $(git -C "$SKIA/third_party/externals/libjpeg-turbo" rev-parse HEAD), IJG and BSD-3-Clause (LICENSE-libjpeg-turbo.txt)
  libpng         $(git -C "$SKIA/third_party/externals/libpng" rev-parse HEAD), libpng licence (LICENSE-libpng.txt)
  zlib           $(git -C "$SKIA/third_party/externals/zlib" rev-parse HEAD), zlib licence (LICENSE-zlib.txt)
  wuffs          $(git -C "$SKIA/third_party/externals/wuffs" rev-parse HEAD), Apache-2.0 (LICENSE-wuffs.txt)

The bindings were generated by skia-bindings' build script at rust-skia git $RUST_SKIA_COMMIT
(MIT), with Skia's features: $(echo "$KEY" | sed "s/.*$TARGET-//" | tr '-' ' ').
INFO
}

step_package() {
  local stage="$WORK/stage" externals="$SKIA/third_party/externals"
  [ -f "$EXPORT/skia-binaries/key.txt" ] || die "nothing to package: run 'vendor/skia/build.sh build' first"
  rm -rf "$DIST" "$stage"
  mkdir -p "$DIST" "$stage"
  # The archive as skia-bindings unpacks it: one folder, skia-binaries/.
  tar -czf "$stage/$ARCHIVE" -C "$EXPORT" skia-binaries
  cp "$SKIA/LICENSE" "$stage/LICENSE-Skia.txt"
  cp "$externals/expat/expat/COPYING" "$stage/LICENSE-expat.txt"
  cp "$externals/libjpeg-turbo/LICENSE.md" "$stage/LICENSE-libjpeg-turbo.txt"
  cp "$externals/libpng/LICENSE" "$stage/LICENSE-libpng.txt"
  cp "$externals/zlib/LICENSE" "$stage/LICENSE-zlib.txt"
  cp "$externals/wuffs/LICENSE" "$stage/LICENSE-wuffs.txt"
  write_build_info >"$stage/BUILD-INFO.txt"
  cp "$stage/BUILD-INFO.txt" "$DIST/"
  tar -czf "$DIST/$PACKAGE.tar.gz" -C "$stage" .
  (cd "$DIST" && shasum -a 256 "$PACKAGE.tar.gz" >SHA256SUMS)
  log "done: $DIST"
  ls -la "$DIST" | tail -n +2 | awk '{print "  " $NF, $5}'
  head -1 "$DIST/SHA256SUMS"
}

# For local development before (or instead of) a published release: what scripts/fetch-skia.sh
# would put into vendor/skia/bin/, taken from this machine's package, by that script itself (it
# also tells cargo where the archive is). Releases never use this.
step_install() {
  [ -f "$DIST/$PACKAGE.tar.gz" ] || die "no package in $DIST: run vendor/skia/build.sh first"
  "$HERE/../../scripts/fetch-skia.sh" --local
}

# ---- main --------------------------------------------------------------------------------

ALL_STEPS="fetch build check package"
for step in ${*:-$ALL_STEPS}; do
  case $step in
    fetch)   step_fetch ;;
    build)   step_build ;;
    check)   step_check ;;
    package) step_package ;;
    install) step_install ;;
    *) die "unknown step: $step (valid: $ALL_STEPS install)" ;;
  esac
done
