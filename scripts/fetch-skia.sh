#!/usr/bin/env bash
# Downloads the pinned Skia (its static libraries with their Rust bindings, built from source by
# this repository's own vendor/skia/build.sh) into vendor/skia/bin/, and tells cargo where it is.
# Slint's Skia renderer draws the window through the skia-bindings crate, which takes that archive
# from the address in SKIA_BINARIES_URL: a file:// address with an absolute path, which no file in
# git can say, so this script writes it into .cargo/config.toml (not in git). The app's build only
# ever uses this copy; see docs/conventions.md and docs/skia.md.
#
#   scripts/fetch-skia.sh           # the pinned release
#   scripts/fetch-skia.sh --local   # this machine's own build (vendor/skia/dist/), to try a change to it
#
# To upgrade (a new Slint asks for a new skia-safe): change the pinned block of vendor/skia/build.sh
# and the Cargo.lock beside it, push a tag skia-<version>-<n> (the skia workflow builds and publishes
# it), then change RELEASE, ASSET and SHA256 below, commit, and every developer re-runs this script.
set -euo pipefail

# ---- pinned version ---------------------------------------------------------------
REPO="FinchGram/FinchGram"                  # this repository on GitHub
RELEASE="skia-0.153.3-1"                    # release tag
ASSET="skia-0.153.3-macos-arm64.tar.gz"     # asset name in that release
SHA256="PENDING: push the tag skia-0.153.3-1 and take this from the release's SHA256SUMS"
# -----------------------------------------------------------------------------------

URL="https://github.com/$REPO/releases/download/$RELEASE/$ASSET"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/vendor/skia/bin"
CONFIG="$ROOT/.cargo/config.toml"
MARK="# Written by scripts/fetch-skia.sh; not in git: the path is this machine's."
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [ "${1:-}" = "--local" ]; then
  PACKAGE="$ROOT/vendor/skia/dist/$ASSET"
  [ -f "$PACKAGE" ] || { echo "no package in vendor/skia/dist: run vendor/skia/build.sh first" >&2; exit 1; }
  echo "taking this machine's own build, $PACKAGE (releases fetch the published one)"
else
  case "$SHA256" in *PENDING*)
    echo "SHA256 in $0 is not pinned yet: publish the $RELEASE release first (push the tag)." >&2
    echo "Until then, build Skia here: vendor/skia/build.sh && vendor/skia/build.sh install" >&2
    exit 1 ;;
  esac
  PACKAGE="$TMP/$ASSET"
  echo "downloading $URL"
  # --retry-all-errors: GitHub's download host can answer 504 after the transfer began, which plain
  # --retry does not count as passing (fetch-mpv.sh).
  curl -fL --retry 3 --retry-all-errors --progress-bar -o "$PACKAGE" "$URL"
  actual="$(shasum -a 256 "$PACKAGE" | cut -d' ' -f1)"
  if [ "$actual" != "$SHA256" ]; then
    echo "SHA-256 mismatch for $ASSET" >&2
    echo "  expected $SHA256" >&2
    echo "  actual   $actual" >&2
    exit 1
  fi
fi

# cargo's file is written between single quotes, which take every character as it is but a quote.
case "$ROOT" in *"'"* | *$'\n'*)
  echo "the repository's path has a quote or a line break in it, which .cargo/config.toml cannot say: $ROOT" >&2
  exit 1 ;;
esac
ADDRESS="file://$DEST/skia-binaries-{key}.tar.gz"
# A cargo configuration somebody wrote for themselves is never overwritten: it has to say where the
# archive is.
OWN_CONFIG=""
if [ -f "$CONFIG" ] && [ "$(head -1 "$CONFIG")" != "$MARK" ]; then
  if ! grep -qF "SKIA_BINARIES_URL = '$ADDRESS'" "$CONFIG"; then
    echo "$CONFIG is there and is not this script's. Add this to its [env] table, then run this script again:" >&2
    echo "  SKIA_BINARIES_URL = '$ADDRESS'" >&2
    exit 1
  fi
  OWN_CONFIG=1
fi

rm -rf "$DEST"
mkdir -p "$DEST"
tar -xzf "$PACKAGE" -C "$DEST"

if [ -z "$OWN_CONFIG" ]; then
  mkdir -p "$(dirname "$CONFIG")"
  printf "%s\n[env]\nSKIA_BINARIES_URL = '%s'\n" "$MARK" "$ADDRESS" >"$CONFIG"
fi

echo
head -1 "$DEST/BUILD-INFO.txt"
echo "installed into $DEST; cargo finds it through .cargo/config.toml"
