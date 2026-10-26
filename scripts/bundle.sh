#!/usr/bin/env bash
# Builds a self-contained FinchGram.app in dist/, with finchgram-tdlib and libmpv inside.
# Needs vendor/tdlib/bin/, vendor/mpv/bin/ and vendor/fonts/ (run scripts/fetch-tdlib.sh,
# scripts/fetch-mpv.sh and scripts/fetch-fonts.sh once first).
#
# The release workflow runs this on a clean runner (.github/workflows/release.yml); locally it is
# only for trying the packaged app, never for a release. Without FINCHGRAM_API_ID and
# FINCHGRAM_API_HASH in the environment the app has no Telegram API credentials (docs/conventions.md).
#
# Signing (scripts/sign.sh, run at the end) is ad hoc by default: a bundle for this machine only.
# The release workflow bundles ad hoc on a Blacksmith runner, then signs the bundle again with the
# Developer ID on GitHub's own runner (.github/workflows/release.yml). To sign here with a
# certificate of your own:
#   SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" scripts/bundle.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_NAME="FinchGram"
BIN="finchgram"
BUNDLE_ID="com.finchgram.FinchGram"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
MIN_MACOS="12.0" # keep in sync with MACOS_MIN in vendor/tdlib/build.sh

TDLIB="$ROOT/vendor/tdlib/bin"
MPV="$ROOT/vendor/mpv/bin"
FONTS="$ROOT/vendor/fonts"
ICONS="$ROOT/ui/logo/png"
APP="$ROOT/dist/$APP_NAME.app"
MACOS="$APP/Contents/MacOS"
FRAMEWORKS="$APP/Contents/Frameworks"
RESOURCES="$APP/Contents/Resources"

for f in finchgram-tdlib LICENSE-TDLib.txt LICENSE-OpenSSL.txt BUILD-INFO.txt; do
  [ -e "$TDLIB/$f" ] || { echo "missing $TDLIB/$f - run scripts/fetch-tdlib.sh first" >&2; exit 1; }
done
for f in libmpv.2.dylib BUILD-INFO.txt LICENSE-mpv.txt; do
  [ -e "$MPV/$f" ] || { echo "missing $MPV/$f - run scripts/fetch-mpv.sh first" >&2; exit 1; }
done
ls "$FONTS"/*-OFL.txt >/dev/null 2>&1 || { echo "missing $FONTS - run scripts/fetch-fonts.sh first" >&2; exit 1; }

cargo build --release --manifest-path "$ROOT/Cargo.toml" --bin "$BIN"
# The executable holds the 39 MB of fonts once (src/fonts.rs); a copy that is far larger holds
# them twice, as v0.3.25 did when they were consts, and is not shipped.
BYTES=$(stat -f %z "$ROOT/target/release/$BIN")
# The binary was 92 MB at v0.4.22 and 103 MB at v0.5.0 (the UI's three themes grow with each round);
# the fonts in it twice (the v0.3.25 mistake, src/fonts.rs) would add 39 MB more.
if [ "$BYTES" -gt 130000000 ]; then
  echo "$BIN is $BYTES bytes, more than it should be: are the fonts in it twice (src/fonts.rs)?" >&2
  exit 1
fi

rm -rf "$APP"
mkdir -p "$MACOS" "$FRAMEWORKS" "$RESOURCES"
cp "$ROOT/target/release/$BIN" "$MACOS/$BIN"
# The app only ever looks for finchgram-tdlib next to its own executable (src/telegram/process.rs),
# and for libmpv in its Frameworks folder (its run path, build.rs).
cp "$TDLIB/finchgram-tdlib" "$MACOS/"
cp "$MPV/libmpv.2.dylib" "$FRAMEWORKS/"
# FinchGram's licence, and those of TDLib and OpenSSL with how finchgram-tdlib was built; those of
# the fonts compiled into the executable (SIL OFL 1.1) and of the icons (MIT).
cp "$ROOT/LICENSE" "$RESOURCES/LICENSE.txt"
cp "$TDLIB/LICENSE-TDLib.txt" "$TDLIB/LICENSE-OpenSSL.txt" "$RESOURCES/"
cp "$TDLIB/BUILD-INFO.txt" "$RESOURCES/finchgram-tdlib-BUILD-INFO.txt"
# libmpv's: mpv, FFmpeg, libplacebo, libass, FreeType, FriBidi, HarfBuzz.
cp "$MPV"/LICENSE-*.txt "$RESOURCES/"
cp "$MPV/BUILD-INFO.txt" "$RESOURCES/libmpv-BUILD-INFO.txt"
for licence in "$FONTS"/*-OFL.txt; do
  cp "$licence" "$RESOURCES/LICENSE-Fonts-$(basename "$licence")"
done
cp "$ROOT/ui/icons/LICENSE" "$RESOURCES/LICENSE-Phosphor-Icons.txt"

# The app icon, from the logo's own sizes in ui/logo/png/ (16 px is the silhouette the logo asks
# for below 20 px). iconutil is part of macOS.
ICON_TMP="$(mktemp -d)"
ICONSET="$ICON_TMP/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  cp "$ICONS/FinchGram-icon-$size.png" "$ICONSET/icon_${size}x${size}.png"
  cp "$ICONS/FinchGram-icon-$((size * 2)).png" "$ICONSET/icon_${size}x${size}@2x.png"
done
iconutil --convert icns "$ICONSET" --output "$RESOURCES/AppIcon.icns"
rm -rf "$ICON_TMP"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key><string>$BIN</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>$MIN_MACOS</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.social-networking</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>FinchGram uses the microphone for voice messages, and for voice input when you use it: then what it hears goes to the speech service you chose, under your own key, to become text.</string>
</dict>
</plist>
PLIST

# Signed by scripts/sign.sh: ad hoc, or with SIGN_IDENTITY from the environment.
"$ROOT/scripts/sign.sh" "$APP"

echo
echo "built: $APP ($(du -sh "$APP" | cut -f1))"
