#!/usr/bin/env bash
# Builds finchgram-tdlib for macOS (Apple silicon) from pinned upstream sources: TDLib and OpenSSL,
# linked statically into one program with our small host (host/main.cpp), which speaks TDLib's JSON
# interface over standard input and output (docs/architecture.md).
#
#   vendor/tdlib/build.sh           # everything: fetch, build OpenSSL, build TDLib and the host, check, package
#   vendor/tdlib/build.sh tdlib     # a single step: fetch openssl tdlib check package install
#   vendor/tdlib/build.sh install   # unpack the package into vendor/tdlib/bin/, for local development only
#
# The package goes to vendor/tdlib/dist/, intermediate files to vendor/tdlib/work/, and the program the
# app uses lives in vendor/tdlib/bin/ (scripts/fetch-tdlib.sh fills it from a release). None of the three
# is tracked by git. Releases are built by CI from a tdlib-* tag (.github/workflows/tdlib.yml).
# Everything below the "pinned versions" block is mechanism. To upgrade anything, only that block changes.
set -euo pipefail

# ---- pinned versions ------------------------------------------------------------------
TDLIB_VERSION="1.8.67"
TDLIB_GIT="https://github.com/tdlib/td.git"
TDLIB_COMMIT="bc9c263e2bfee06aaab41e82db51a103376030bc" # "Update version to 1.8.67.", 2026-08-24

OPENSSL_VERSION="3.5.9" # the 3.5 long-term support series
OPENSSL_URL="https://github.com/openssl/openssl/releases/download/openssl-$OPENSSL_VERSION/openssl-$OPENSSL_VERSION.tar.gz"
OPENSSL_SHA256="603f5602e2eef00d77fbd429d34dcd5822bb301757a1bc9cdb24c670f1eb859a" # as published next to the release

MACOS_MIN="12.0" # oldest macOS the program runs on; keep in sync with the app's LSMinimumSystemVersion
ARCH="arm64"
# ---------------------------------------------------------------------------------------

HERE="$(cd "$(dirname "$0")" && pwd)" # vendor/tdlib
WORK="$HERE/work"
DOWNLOADS="$WORK/downloads"
SRC="$WORK/src"
PREFIX="$WORK/prefix" # the static OpenSSL lands here
BUILD="$WORK/build"   # CMake build of the host, TDLib included
OUT="$WORK/out"       # the finished program, stripped and signed
LOGS="$WORK/logs"
DIST="$HERE/dist"
BIN="$HERE/bin" # what the app uses (build.rs copies it next to the executable)
PROGRAM="finchgram-tdlib"
PACKAGE="$PROGRAM-$TDLIB_VERSION-macos-$ARCH"
JOBS="$(sysctl -n hw.ncpu)"
mkdir -p "$DOWNLOADS" "$SRC" "$PREFIX" "$LOGS"

export MACOSX_DEPLOYMENT_TARGET="$MACOS_MIN"
export CC=clang CXX=clang++

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

# Check out exactly one commit (no branch or tag can move it later).
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
}

# Run a build function in a subshell with its output in a log file; show the tail on failure.
run_logged() { # name function
  local name=$1 status
  log "building $name  (log: vendor/tdlib/work/logs/$name.log)"
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
  [ -d "$SRC/$1" ] || die "no source for $1 in vendor/tdlib/work/src: run 'vendor/tdlib/build.sh fetch' first"
  if built "$1-$2"; then
    log "$1 $2 already built, skipping"
    return
  fi
  run_logged "$1" "$3"
  touch "$PREFIX/.built-$1-$2"
}

# Run a command with a time limit (macOS has no timeout(1)). The alarm survives the exec.
run_with_timeout() { # seconds command...
  local seconds=$1
  shift
  perl -e 'alarm shift; exec @ARGV or die "cannot run $ARGV[0]: $!"' "$seconds" "$@"
}

# ---- steps ------------------------------------------------------------------------------

step_fetch() {
  need cmake cmake; need ninja ninja; need gperf gperf
  extract "$(fetch_tarball "$OPENSSL_URL" "$OPENSSL_SHA256")" "$SRC/openssl"
  fetch_git td "$TDLIB_GIT" "$TDLIB_COMMIT"
  log "all sources fetched and verified"
}

build_openssl() {
  cd "$SRC/openssl"
  # OpenSSL reads its configuration from OPENSSLDIR at run time. /var/empty is empty and belongs to
  # root on every Mac, so nothing on the user's machine is ever picked up.
  ./Configure "darwin64-$ARCH" no-shared no-apps no-docs no-tests \
    --prefix="$PREFIX" --libdir=lib --openssldir=/var/empty \
    -mmacosx-version-min="$MACOS_MIN"
  make -j"$JOBS"
  make install_sw
}

build_tdlib() {
  local sdk
  sdk="$(xcrun --sdk macosx --show-sdk-path)"
  rm -rf "$BUILD" "$OUT"
  # Our own static OpenSSL, and zlib from the macOS SDK (part of every Mac), named explicitly so
  # that nothing Homebrew installed can be picked up instead.
  cmake -S "$HERE/host" -B "$BUILD" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_OSX_ARCHITECTURES="$ARCH" -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_MIN" \
    -DTD_SOURCE_DIR="$SRC/td" \
    -DOPENSSL_USE_STATIC_LIBS=TRUE \
    -DOPENSSL_ROOT_DIR="$PREFIX" \
    -DOPENSSL_INCLUDE_DIR="$PREFIX/include" \
    -DOPENSSL_CRYPTO_LIBRARY="$PREFIX/lib/libcrypto.a" \
    -DOPENSSL_SSL_LIBRARY="$PREFIX/lib/libssl.a" \
    -DZLIB_INCLUDE_DIR="$sdk/usr/include" \
    -DZLIB_LIBRARY="$sdk/usr/lib/libz.tbd"
  cmake --build "$BUILD" --target "$PROGRAM" -j "$JOBS"
  mkdir -p "$OUT"
  cp "$BUILD/$PROGRAM" "$OUT/$PROGRAM"
  strip "$OUT/$PROGRAM"
  # Stripping invalidates the linker's ad-hoc signature, without which Apple silicon refuses to
  # run the program. The app's bundle script signs it again with the real identity.
  codesign --force --sign - "$OUT/$PROGRAM"
}

step_check() {
  local program="$OUT/$PROGRAM" deps bad version transcript
  [ -x "$program" ] || die "$program is missing: run 'vendor/tdlib/build.sh tdlib' first"
  log "checking $PROGRAM"
  deps=$(otool -L "$program" | tail -n +2 | awk '{print $1}')
  bad=$(printf '%s\n' "$deps" | grep -vE '^(/usr/lib/|/System/Library/)' || true)
  [ -z "$bad" ] || die "$PROGRAM links libraries that are not part of macOS:
$bad"
  printf '  %s dynamic libraries, all from macOS\n' "$(printf '%s\n' "$deps" | wc -l | tr -d ' ')"
  version=$("$program" --version)
  [ "$version" = "$TDLIB_VERSION $TDLIB_COMMIT" ] || die "$PROGRAM --version says '$version', expected '$TDLIB_VERSION $TDLIB_COMMIT'"
  echo "  TDLib $version"
  # Smoke test: one request, then standard input closes. Expect its answer, TDLib closing cleanly
  # and the program ending, within 30 seconds.
  transcript=$(mktemp)
  printf '%s\n' '{"@type":"getOption","name":"version","@extra":1}' \
    | run_with_timeout 30 "$program" >"$transcript" 2>/dev/null \
    || die "$PROGRAM did not end cleanly after its input closed"
  grep -q '"@extra":1' "$transcript" || die "no answer to getOption: $(head -c 400 "$transcript")"
  grep -q '"authorizationStateClosed"' "$transcript" || die "TDLib did not report authorizationStateClosed"
  printf '  smoke test passed: %s messages, the answer, then a clean close\n' "$(wc -l <"$transcript" | tr -d ' ')"
  rm -f "$transcript"
}

write_build_info() {
  cat <<INFO
$PROGRAM: TDLib $TDLIB_VERSION with FinchGram's JSON-over-stdio host, static build for macOS $ARCH
(runs on macOS $MACOS_MIN and newer)
built $(date -u +%Y-%m-%dT%H:%M:%SZ) on macOS $(sw_vers -productVersion), $(xcodebuild -version 2>/dev/null | head -1 | tr -d '\n'), $(clang --version | head -1)

statically linked:
  TDLib    $TDLIB_VERSION (git $TDLIB_COMMIT), Boost Software License 1.0 (LICENSE-TDLib.txt)
  OpenSSL  $OPENSSL_VERSION, Apache License 2.0 (LICENSE-OpenSSL.txt)
  zlib     from the macOS SDK (part of macOS)

The host (vendor/tdlib/host/) is part of FinchGram, GPL-3.0.
INFO
}

step_package() {
  local stage="$WORK/stage"
  [ -x "$OUT/$PROGRAM" ] || die "$OUT/$PROGRAM is missing: run 'vendor/tdlib/build.sh tdlib' first"
  rm -rf "$DIST" "$stage"
  mkdir -p "$DIST/sources" "$stage"
  cp "$OUT/$PROGRAM" "$stage/"
  cp "$SRC/td/LICENSE_1_0.txt" "$stage/LICENSE-TDLib.txt"
  cp "$SRC/openssl/LICENSE.txt" "$stage/LICENSE-OpenSSL.txt"
  write_build_info >"$stage/BUILD-INFO.txt"
  cp "$stage/BUILD-INFO.txt" "$DIST/"
  tar -czf "$DIST/$PACKAGE.tar.gz" -C "$stage" "$PROGRAM" LICENSE-TDLib.txt LICENSE-OpenSSL.txt BUILD-INFO.txt
  # The exact sources that went into the program travel with it.
  cp "$DOWNLOADS/$(basename "$OPENSSL_URL")" "$DIST/sources/"
  git -C "$SRC/td" archive --format=tar.gz --prefix="td-$TDLIB_COMMIT/" -o "$DIST/sources/td-$TDLIB_COMMIT.tar.gz" HEAD
  (cd "$DIST" && shasum -a 256 "$PACKAGE.tar.gz" sources/* >SHA256SUMS)
  log "done: $DIST"
  ls -la "$DIST" | tail -n +2 | awk '{print "  " $NF, $5}'
  head -1 "$DIST/SHA256SUMS"
}

# For local development before (or instead of) a published release: what scripts/fetch-tdlib.sh
# would put into vendor/tdlib/bin/, taken from this machine's package. Releases never use this.
step_install() {
  [ -f "$DIST/$PACKAGE.tar.gz" ] || die "no package in $DIST: run vendor/tdlib/build.sh first"
  rm -rf "$BIN"
  mkdir -p "$BIN"
  tar -xzf "$DIST/$PACKAGE.tar.gz" -C "$BIN"
  log "installed into $BIN (a local build: releases fetch the published one)"
  "$BIN/$PROGRAM" --version
}

# ---- main --------------------------------------------------------------------------------

ALL_STEPS="fetch openssl tdlib check package"
for step in ${*:-$ALL_STEPS}; do
  case $step in
    fetch)   step_fetch ;;
    openssl) lib openssl "$OPENSSL_VERSION" build_openssl ;;
    tdlib)   run_logged tdlib build_tdlib ;;
    check)   step_check ;;
    package) step_package ;;
    install) step_install ;;
    *) die "unknown step: $step (valid: $ALL_STEPS install)" ;;
  esac
done
