#!/usr/bin/env bash
# Downloads the pinned finchgram-tdlib (TDLib + OpenSSL, built from source by this repository's own
# vendor/tdlib/build.sh) into vendor/tdlib/bin/. The app only ever uses this copy; see
# docs/conventions.md.
#
# To upgrade: change the pinned block of vendor/tdlib/build.sh, push a tag tdlib-<version>-<n>
# (the tdlib workflow builds and publishes it), then change RELEASE, ASSET and SHA256 below
# and TDLIB_VERSION in src/telegram/mod.rs, commit, and every developer re-runs this script.
set -euo pipefail

# ---- pinned version ---------------------------------------------------------------
REPO="FinchGram/FinchGram"                        # this repository on GitHub
RELEASE="tdlib-1.8.67-1"                          # release tag
ASSET="finchgram-tdlib-1.8.67-macos-arm64.tar.gz" # asset name in that release
SHA256="9cf74e246dabc924f29744858f95926a39e2c22ae40f869b649e8061346d0bd2" # from the release's SHA256SUMS
# -----------------------------------------------------------------------------------

URL="https://github.com/$REPO/releases/download/$RELEASE/$ASSET"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/vendor/tdlib/bin"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

case "$SHA256" in *PENDING*)
  echo "SHA256 in $0 is not pinned yet: publish the $RELEASE release first (push the tag)." >&2
  echo "Until then, build finchgram-tdlib here: vendor/tdlib/build.sh && vendor/tdlib/build.sh install" >&2
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
tar -xzf "$TMP/$ASSET" -C "$DEST" finchgram-tdlib LICENSE-TDLib.txt LICENSE-OpenSSL.txt BUILD-INFO.txt

echo
echo "TDLib $("$DEST/finchgram-tdlib" --version)"
echo "installed into $DEST"
