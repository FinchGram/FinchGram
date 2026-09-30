#!/usr/bin/env bash
# Builds libmpv for macOS (Apple silicon) from pinned upstream sources: mpv and what it needs
# (FFmpeg, libplacebo, and libass with FreeType, FriBidi and HarfBuzz), all linked statically into
# one library, libmpv.2.dylib, which depends only on what ships with macOS. The app plays video
# with it (docs/architecture.md).
#
#   vendor/mpv/build.sh           # everything: fetch, build each library, check, package
#   vendor/mpv/build.sh ffmpeg    # a single step: fetch freetype fribidi harfbuzz libass ffmpeg libplacebo mpv check package install
#   vendor/mpv/build.sh install   # unpack the package into vendor/mpv/bin/, for local development only
#
# The package goes to vendor/mpv/dist/, intermediate files to vendor/mpv/work/, and the library the
# app uses lives in vendor/mpv/bin/ (scripts/fetch-mpv.sh fills it from a release). None of the three
# is tracked by git. Releases are built by CI from an mpv-* tag (.github/workflows/mpv.yml).
# Everything below the "pinned versions" block is mechanism. To upgrade anything, only that block changes.
set -euo pipefail

# ---- pinned versions ------------------------------------------------------------------
MPV_VERSION="0.41.0"
MPV_GIT="https://github.com/mpv-player/mpv.git"
MPV_COMMIT="41f6a645068483470267271e1d09966ca3b9f413" # tag v0.41.0, 2025-12-21

FFMPEG_VERSION="8.1.3"
FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz"
FFMPEG_SHA256="7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3"

LIBPLACEBO_VERSION="7.360.1"
LIBPLACEBO_GIT="https://github.com/haasn/libplacebo.git" # its submodules (glad, jinja, …) come pinned with it
LIBPLACEBO_COMMIT="cee9b076f2c63104ccfd497fa79c39a867293ec4" # tag v7.360.1, 2026-03-13

LIBASS_VERSION="0.17.5"
LIBASS_URL="https://github.com/libass/libass/releases/download/$LIBASS_VERSION/libass-$LIBASS_VERSION.tar.xz"
LIBASS_SHA256="2dca25c0e0c837ddf00b52011b3f82cac1e4ddd3ad018227806b0c2288864acc"

FREETYPE_VERSION="2.14.3"
FREETYPE_URL="https://download.savannah.gnu.org/releases/freetype/freetype-$FREETYPE_VERSION.tar.xz"
FREETYPE_SHA256="36bc4f1cc413335368ee656c42afca65c5a3987e8768cc28cf11ba775e785a5f"

FRIBIDI_VERSION="1.0.17"
FRIBIDI_URL="https://github.com/fribidi/fribidi/releases/download/v$FRIBIDI_VERSION/fribidi-$FRIBIDI_VERSION.tar.xz"
FRIBIDI_SHA256="6949dcde27d41cebad1fd741fcafc36d55a1020d2d872d4a6eb3914caabbada2"

HARFBUZZ_VERSION="14.5.0"
HARFBUZZ_URL="https://github.com/harfbuzz/harfbuzz/releases/download/$HARFBUZZ_VERSION/harfbuzz-$HARFBUZZ_VERSION.tar.xz"
HARFBUZZ_SHA256="b7132e148358a45185c9feafd049dbaf243649d3c44414b3534d9c95d18592b9"

MACOS_MIN="12.0" # oldest macOS the library runs on; keep in sync with the app's LSMinimumSystemVersion
ARCH="arm64"
# ---------------------------------------------------------------------------------------

HERE="$(cd "$(dirname "$0")" && pwd)" # vendor/mpv
WORK="$HERE/work"
DOWNLOADS="$WORK/downloads"
SRC="$WORK/src"
PREFIX="$WORK/prefix" # the static libraries land here
OUT="$WORK/out"       # the finished library, with its headers
LOGS="$WORK/logs"
DIST="$HERE/dist"
BIN="$HERE/bin" # what the app uses (build.rs links it and copies it next to the executable)
LIBRARY="libmpv.2.dylib"
PACKAGE="libmpv-$MPV_VERSION-macos-$ARCH"
JOBS="$(sysctl -n hw.ncpu)"
mkdir -p "$DOWNLOADS" "$SRC" "$PREFIX" "$LOGS"

export MACOSX_DEPLOYMENT_TARGET="$MACOS_MIN"
export CC=clang CXX=clang++ OBJC=clang
# pkg-config sees our own libraries only: nothing Homebrew installed can be picked up.
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
unset PKG_CONFIG_PATH

# ---- helpers ---------------------------------------------------------------------------

log() { printf '\n==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required: brew install $2"; }
sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }

# Download (if needed) and verify one tarball; prints its path.
fetch_tarball() { # url expected-sha256
  local url=$1 expected=$2 file="$DOWNLOADS/$(basename "$1")" actual
  [ -n "$expected" ] || die "no SHA-256 pinned for $url"
  if [ ! -f "$file" ] || [ "$(sha256 "$file")" != "$expected" ]; then
    log "downloading $(basename "$url")"
    curl -fL --retry 3 -o "$file.tmp" "$url"
    mv "$file.tmp" "$file"
  fi
  actual=$(sha256 "$file")
  [ "$actual" = "$expected" ] || die "SHA-256 mismatch for $file: expected $expected, got $actual"
  echo "$file"
}

# Fresh extraction of a tarball into a directory, dropping the top-level folder.
extract() { # tarball dest
  rm -rf "$2"
  mkdir -p "$2"
  tar -xf "$1" -C "$2" --strip-components=1
}

# Check out exactly one commit (no branch or tag can move it later), with the submodules that
# commit pins.
fetch_git() { # name url commit
  local name=$1 url=$2 commit=$3 dir="$SRC/$1"
  [ -n "$commit" ] || die "no commit pinned for $name"
  if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD)" = "$commit" ]; then
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
  log "building $name  (log: vendor/mpv/work/logs/$name.log)"
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

built() { [ -f "$PREFIX/.built-$1" ]; }

# Build one library unless a stamp says this exact version is already in $PREFIX.
lib() { # name version build-function
  [ -d "$SRC/$1" ] || die "no source for $1 in vendor/mpv/work/src: run 'vendor/mpv/build.sh fetch' first"
  if built "$1-$2"; then
    log "$1 $2 already built, skipping"
    return
  fi
  run_logged "$1" "$3"
  touch "$PREFIX/.built-$1-$2"
}

# A static meson build into $PREFIX. Subprojects are never downloaded: every dependency comes from
# $PREFIX, or from macOS.
meson_static() { # source-dir [meson options...]
  local src=$1
  shift
  rm -rf "$src/build"
  meson setup "$src/build" "$src" \
    --prefix="$PREFIX" --libdir=lib --buildtype=release --default-library=static \
    -Db_ndebug=true --wrap-mode=nofallback \
    -Dc_args="-arch $ARCH -mmacosx-version-min=$MACOS_MIN" \
    -Dcpp_args="-arch $ARCH -mmacosx-version-min=$MACOS_MIN" \
    "$@"
  meson compile -C "$src/build" -j "$JOBS"
  meson install -C "$src/build"
}

# ---- steps ------------------------------------------------------------------------------

step_fetch() {
  need meson meson; need ninja ninja; need pkg-config pkgconf; need python3 python
  extract "$(fetch_tarball "$FFMPEG_URL" "$FFMPEG_SHA256")" "$SRC/ffmpeg"
  extract "$(fetch_tarball "$LIBASS_URL" "$LIBASS_SHA256")" "$SRC/libass"
  extract "$(fetch_tarball "$FREETYPE_URL" "$FREETYPE_SHA256")" "$SRC/freetype"
  extract "$(fetch_tarball "$FRIBIDI_URL" "$FRIBIDI_SHA256")" "$SRC/fribidi"
  extract "$(fetch_tarball "$HARFBUZZ_URL" "$HARFBUZZ_SHA256")" "$SRC/harfbuzz"
  fetch_git libplacebo "$LIBPLACEBO_GIT" "$LIBPLACEBO_COMMIT"
  fetch_git mpv "$MPV_GIT" "$MPV_COMMIT"
  log "all sources fetched and verified"
}

# Fonts for subtitles: libass finds them through macOS (CoreText), FreeType reads them.
build_freetype() {
  meson_static "$SRC/freetype" -Dbrotli=disabled -Dbzip2=disabled -Dharfbuzz=disabled -Dpng=disabled \
    -Dzlib=internal -Dtests=disabled
}

build_fribidi() {
  meson_static "$SRC/fribidi" -Ddocs=false -Dbin=false -Dtests=false
}

build_harfbuzz() {
  meson_static "$SRC/harfbuzz" -Dfreetype=enabled -Dglib=disabled -Dgobject=disabled -Dcairo=disabled \
    -Dchafa=disabled -Dicu=disabled -Dgraphite2=disabled -Dcoretext=disabled -Dpng=disabled -Dzlib=disabled \
    -Draster=disabled -Dvector=disabled -Dgpu=disabled -Dgpu_demo=disabled -Dsubset=disabled \
    -Dtests=disabled -Ddocs=disabled -Dintrospection=disabled -Dutilities=disabled -Dbenchmark=disabled
}

build_libass() {
  meson_static "$SRC/libass" -Dfontconfig=disabled -Dcoretext=enabled -Ddirectwrite=disabled \
    -Dlibunibreak=disabled -Dasm=disabled -Dtest=disabled -Dcompare=disabled -Dprofile=disabled \
    -Dfuzz=disabled -Dcheckasm=disabled
}

# Decoding: FFmpeg's own decoders and demuxers, and macOS's hardware decoders (VideoToolbox). Only
# what is named here: --disable-autodetect keeps anything installed on the build machine out. The
# app plays files TDLib has downloaded, so there is no network code.
build_ffmpeg() {
  cd "$SRC/ffmpeg"
  ./configure --prefix="$PREFIX" --arch="$ARCH" --cc=clang \
    --extra-cflags="-mmacosx-version-min=$MACOS_MIN" --extra-ldflags="-mmacosx-version-min=$MACOS_MIN" \
    --enable-static --disable-shared --enable-pic \
    --disable-autodetect --enable-videotoolbox --enable-audiotoolbox --enable-zlib \
    --disable-programs --disable-doc --disable-debug --disable-network \
    --disable-avdevice --disable-devices --disable-encoders --disable-muxers
  make -j"$JOBS"
  make install
}

# Video output: the renderer's shaders and colour handling. OpenGL, as mpv draws into the app's
# window through it; Vulkan is not used on macOS.
build_libplacebo() {
  meson_static "$SRC/libplacebo" -Dvulkan=disabled -Dopengl=enabled -Dd3d11=disabled -Dglslang=disabled \
    -Dshaderc=disabled -Dlcms=disabled -Dlibdovi=disabled -Ddemos=false -Dtests=false -Dbench=false \
    -Dfuzz=false -Dunwind=disabled -Dxxhash=disabled
}

# libmpv only (no player program), drawing through its render API into an OpenGL context the app
# gives it; sound through Core Audio. Every optional feature is off unless named. mpv's macOS code
# is partly Swift, which Xcode compiles; the Swift runtime is part of macOS. mpv's own windows
# (cocoa-cb), its Now Playing and Touch Bar integration stay out: the app has its own.
build_mpv() {
  local src="$SRC/mpv" lib
  rm -rf "$src/build" "$OUT"
  meson setup "$src/build" "$src" \
    --prefix="$PREFIX" --libdir=lib --buildtype=release --default-library=shared \
    -Db_ndebug=true --wrap-mode=nofallback -Dauto_features=disabled \
    -Dc_args="-arch $ARCH -mmacosx-version-min=$MACOS_MIN" \
    -Dobjc_args="-arch $ARCH -mmacosx-version-min=$MACOS_MIN" \
    -Dc_link_args="-arch $ARCH -mmacosx-version-min=$MACOS_MIN" \
    -Dlibmpv=true -Dcplayer=false -Dgpl=true \
    -Dcocoa=enabled -Dgl=enabled -Dgl-cocoa=enabled -Dvideotoolbox-gl=enabled \
    -Dcoreaudio=enabled -Dzlib=enabled -Dswift-build=enabled \
    -Dmacos-cocoa-cb=disabled -Dmacos-media-player=disabled -Dmacos-touchbar=disabled
  meson compile -C "$src/build" -j "$JOBS"
  mkdir -p "$OUT/lib" "$OUT/include/mpv"
  lib="$(cd "$src/build" && ls libmpv.*.dylib | head -1)"
  cp "$src/build/$lib" "$OUT/lib/$LIBRARY"
  cp "$src"/include/mpv/{client.h,render.h,render_gl.h,stream_cb.h} "$OUT/include/mpv/"
  # Found through the app's run path: next to the executable, or in the bundle's Frameworks.
  install_name_tool -id "@rpath/$LIBRARY" "$OUT/lib/$LIBRARY"
  strip -x "$OUT/lib/$LIBRARY"
  # Changing the library invalidates the linker's ad-hoc signature, without which Apple silicon
  # refuses to load it. The app's bundle script signs it again with the real identity.
  codesign --force --sign - "$OUT/lib/$LIBRARY"
}

step_check() {
  local library="$OUT/lib/$LIBRARY" deps bad smoke frames
  [ -f "$library" ] || die "$library is missing: run 'vendor/mpv/build.sh mpv' first"
  log "checking $LIBRARY"
  deps=$(otool -L "$library" | tail -n +2 | awk '{print $1}' | grep -v "@rpath/$LIBRARY")
  bad=$(printf '%s\n' "$deps" | grep -vE '^(/usr/lib/|/System/Library/)' || true)
  [ -z "$bad" ] || die "$LIBRARY links libraries that are not part of macOS:
$bad"
  printf '  %s dynamic libraries, all from macOS\n' "$(printf '%s\n' "$deps" | wc -l | tr -d ' ')"
  # Smoke test: a small program plays two seconds of raw video through the library, with no window
  # and no sound, and must reach the end of the file.
  smoke="$WORK/smoke"
  rm -rf "$smoke"
  mkdir -p "$smoke"
  frames="$smoke/frames.yuv"
  head -c $((64 * 64 * 3 / 2 * 20)) /dev/zero >"$frames" # 20 grey frames, 64 × 64, 4:2:0
  cat >"$smoke/smoke.c" <<'C'
#include <mpv/client.h>
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
    mpv_handle *mpv = mpv_create();
    if (!mpv) return 2;
    const char *options[][2] = {{"vo", "null"}, {"ao", "null"}, {"demuxer", "rawvideo"},
        {"demuxer-rawvideo-w", "64"}, {"demuxer-rawvideo-h", "64"}, {"demuxer-rawvideo-fps", "10"},
        {"demuxer-rawvideo-mp-format", "yuv420p"}, {"config", "no"}, {"terminal", "no"}};
    for (unsigned i = 0; i < sizeof options / sizeof options[0]; i++)
        if (mpv_set_option_string(mpv, options[i][0], options[i][1]) < 0) return 3;
    if (mpv_initialize(mpv) < 0) return 4;
    const char *load[] = {"loadfile", argv[1], NULL};
    if (mpv_command(mpv, load) < 0) return 5;
    int loaded = 0;
    for (;;) {
        mpv_event *event = mpv_wait_event(mpv, 30);
        if (event->event_id == MPV_EVENT_NONE) return 6; /* nothing for 30 seconds */
        if (event->event_id == MPV_EVENT_FILE_LOADED) loaded = 1;
        if (event->event_id == MPV_EVENT_END_FILE) {
            mpv_event_end_file *end = event->data;
            printf("mpv %s: loaded %d, end reason %d\n", mpv_client_api_version() ? "ok" : "?", loaded, end->reason);
            mpv_terminate_destroy(mpv);
            return loaded && end->reason == MPV_END_FILE_REASON_EOF ? 0 : 7;
        }
    }
}
C
  clang -arch "$ARCH" -mmacosx-version-min="$MACOS_MIN" -I"$OUT/include" "$smoke/smoke.c" \
    -L"$OUT/lib" -Wl,-rpath,"$OUT/lib" "$OUT/lib/$LIBRARY" -o "$smoke/smoke"
  "$smoke/smoke" "$frames" || die "the smoke test did not play to the end (exit $?)"
  echo "  smoke test passed: raw video played to the end"
}

write_build_info() {
  cat <<INFO
$LIBRARY: mpv $MPV_VERSION as a library (libmpv), static dependencies, for macOS $ARCH
(runs on macOS $MACOS_MIN and newer)
built $(date -u +%Y-%m-%dT%H:%M:%SZ) on macOS $(sw_vers -productVersion), $(xcodebuild -version 2>/dev/null | head -1 | tr -d '\n'), $(clang --version | head -1)

statically linked:
  mpv         $MPV_VERSION (git $MPV_COMMIT), GPL-2.0-or-later (LICENSE-mpv.txt)
  FFmpeg      $FFMPEG_VERSION, LGPL-2.1-or-later (LICENSE-FFmpeg.txt)
  libplacebo  $LIBPLACEBO_VERSION (git $LIBPLACEBO_COMMIT), LGPL-2.1-or-later (LICENSE-libplacebo.txt)
  libass      $LIBASS_VERSION, ISC (LICENSE-libass.txt)
  FreeType    $FREETYPE_VERSION, FreeType License (LICENSE-FreeType.txt)
  FriBidi     $FRIBIDI_VERSION, LGPL-2.1-or-later (LICENSE-FriBidi.txt)
  HarfBuzz    $HARFBUZZ_VERSION, MIT (LICENSE-HarfBuzz.txt)
  zlib        from the macOS SDK (part of macOS)

The headers in include/mpv/ are mpv's client API (ISC).
INFO
}

step_package() {
  local stage="$WORK/stage"
  [ -f "$OUT/lib/$LIBRARY" ] || die "$OUT/lib/$LIBRARY is missing: run 'vendor/mpv/build.sh mpv' first"
  rm -rf "$DIST" "$stage"
  mkdir -p "$DIST/sources" "$stage"
  cp "$OUT/lib/$LIBRARY" "$stage/"
  cp -R "$OUT/include" "$stage/"
  cp "$SRC/mpv/LICENSE.GPL" "$stage/LICENSE-mpv.txt"
  cp "$SRC/ffmpeg/COPYING.LGPLv2.1" "$stage/LICENSE-FFmpeg.txt"
  cp "$SRC/libplacebo/LICENSE" "$stage/LICENSE-libplacebo.txt"
  cp "$SRC/libass/COPYING" "$stage/LICENSE-libass.txt"
  cp "$SRC/freetype/docs/FTL.TXT" "$stage/LICENSE-FreeType.txt"
  cp "$SRC/fribidi/COPYING" "$stage/LICENSE-FriBidi.txt"
  cp "$SRC/harfbuzz/COPYING" "$stage/LICENSE-HarfBuzz.txt"
  write_build_info >"$stage/BUILD-INFO.txt"
  cp "$stage/BUILD-INFO.txt" "$DIST/"
  tar -czf "$DIST/$PACKAGE.tar.gz" -C "$stage" .
  # The exact sources that went into the library travel with it.
  for url in "$FFMPEG_URL" "$LIBASS_URL" "$FREETYPE_URL" "$FRIBIDI_URL" "$HARFBUZZ_URL"; do
    cp "$DOWNLOADS/$(basename "$url")" "$DIST/sources/"
  done
  git -C "$SRC/mpv" archive --format=tar.gz --prefix="mpv-$MPV_COMMIT/" -o "$DIST/sources/mpv-$MPV_COMMIT.tar.gz" HEAD
  tar -czf "$DIST/sources/libplacebo-$LIBPLACEBO_COMMIT.tar.gz" -C "$SRC" --exclude=.git --exclude=build libplacebo
  (cd "$DIST" && shasum -a 256 "$PACKAGE.tar.gz" sources/* >SHA256SUMS)
  log "done: $DIST"
  ls -la "$DIST" | tail -n +2 | awk '{print "  " $NF, $5}'
  head -1 "$DIST/SHA256SUMS"
}

# For local development before (or instead of) a published release: what scripts/fetch-mpv.sh
# would put into vendor/mpv/bin/, taken from this machine's package. Releases never use this.
step_install() {
  [ -f "$DIST/$PACKAGE.tar.gz" ] || die "no package in $DIST: run vendor/mpv/build.sh first"
  rm -rf "$BIN"
  mkdir -p "$BIN"
  tar -xzf "$DIST/$PACKAGE.tar.gz" -C "$BIN"
  log "installed into $BIN (a local build: releases fetch the published one)"
  head -1 "$BIN/BUILD-INFO.txt"
}

# ---- main --------------------------------------------------------------------------------

ALL_STEPS="fetch freetype fribidi harfbuzz libass ffmpeg libplacebo mpv check package"
for step in ${*:-$ALL_STEPS}; do
  case $step in
    fetch)      step_fetch ;;
    freetype)   lib freetype "$FREETYPE_VERSION" build_freetype ;;
    fribidi)    lib fribidi "$FRIBIDI_VERSION" build_fribidi ;;
    harfbuzz)   lib harfbuzz "$HARFBUZZ_VERSION" build_harfbuzz ;;
    libass)     lib libass "$LIBASS_VERSION" build_libass ;;
    ffmpeg)     lib ffmpeg "$FFMPEG_VERSION" build_ffmpeg ;;
    libplacebo) lib libplacebo "$LIBPLACEBO_VERSION" build_libplacebo ;;
    mpv)        run_logged mpv build_mpv ;;
    check)      step_check ;;
    package)    step_package ;;
    install)    step_install ;;
    *) die "unknown step: $step (valid: $ALL_STEPS install)" ;;
  esac
done
