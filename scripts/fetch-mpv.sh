#!/usr/bin/env bash
# Downloads the pinned libmpv (mpv, FFmpeg, libplacebo and libass, built from source by this
# repository's own vendor/mpv/build.sh) into vendor/mpv/bin/. The app only ever uses this copy; see
# docs/conventions.md.
#
# To upgrade: change the pinned block of vendor/mpv/build.sh, push a tag mpv-<version>-<n> (the mpv
# workflow builds and publishes it), then change RELEASE, ASSET and SHA256 below, commit, and every
# developer re-runs this script.
set -euo pipefail

# ---- pinned version ---------------------------------------------------------------
REPO="FinchGram/FinchGram"                  # this repository on GitHub
RELEASE="mpv-0.41.0-1"                      # release tag
ASSET="libmpv-0.41.0-macos-arm64.tar.gz"    # asset name in that release
SHA256="PENDING"                            # from the release's SHA256SUMS
# -----------------------------------------------------------------------------------

URL="https://github.com/$REPO/releases/download/$RELEASE/$ASSET"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/vendor/mpv/bin"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

case "$SHA256" in *PENDING*)
  echo "SHA256 in $0 is not pinned yet: publish the $RELEASE release first (push the tag)." >&2
  echo "Until then, build libmpv here: vendor/mpv/build.sh && vendor/mpv/build.sh install" >&2
  exit 1 ;;
esac

echo "downloading $URL"
curl -fL --retry 3 --progress-bar -o "$TMP/$ASSET" "$URL"
actual="$(shasum -a 256 "$TMP/$ASSET" | cut -d' ' -f1)"
if [ "$actual" != "$SHA256" ]; then
  echo "SHA-256 mismatch for $ASSET" >&2
  echo "  expected $SHA256" >&2
  echo "  actual   $actual" >&2
  exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST"
tar -xzf "$TMP/$ASSET" -C "$DEST"

echo
head -1 "$DEST/BUILD-INFO.txt"
echo "installed into $DEST"
