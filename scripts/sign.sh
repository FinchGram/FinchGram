#!/usr/bin/env bash
# Signs FinchGram.app (dist/FinchGram.app, or the bundle given): the helper and the library inside
# first, then the app with its entitlements (the microphone: scripts/FinchGram.entitlements).
#
# scripts/bundle.sh runs it at the end of every bundle, ad hoc unless SIGN_IDENTITY says otherwise.
# The release workflow bundles on a Blacksmith runner (ad hoc), then signs the same bundle again
# here with the Developer ID on GitHub's own runner, so that the certificate never leaves GitHub
# (.github/workflows/release.yml, docs/conventions.md, Release first):
#   SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" scripts/sign.sh dist/FinchGram.app
# A Developer ID gets the hardened runtime and a timestamp too: they are for notarization.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="${1:-$ROOT/dist/FinchGram.app}"
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
MACOS="$APP/Contents/MacOS"
FRAMEWORKS="$APP/Contents/Frameworks"

[ -d "$APP" ] || { echo "no app at $APP - run scripts/bundle.sh first" >&2; exit 1; }

SIGN_OPTS=()
case "$SIGN_IDENTITY" in
  "Developer ID Application:"*) SIGN_OPTS=(--options runtime --timestamp) ;;
esac
codesign --force --sign "$SIGN_IDENTITY" "${SIGN_OPTS[@]+"${SIGN_OPTS[@]}"}" "$MACOS/finchgram-tdlib"
codesign --force --sign "$SIGN_IDENTITY" "${SIGN_OPTS[@]+"${SIGN_OPTS[@]}"}" "$FRAMEWORKS/libmpv.2.dylib"
codesign --force --sign "$SIGN_IDENTITY" "${SIGN_OPTS[@]+"${SIGN_OPTS[@]}"}" --entitlements "$ROOT/scripts/FinchGram.entitlements" "$APP"
codesign --verify --deep --strict "$APP"
codesign -d --entitlements - "$APP" 2>/dev/null | grep -q "com.apple.security.device.audio-input" || { echo "the app was signed without the microphone's entitlement" >&2; exit 1; }

echo "signed with $SIGN_IDENTITY; designated requirement: $(codesign -d -r- "$APP" 2>/dev/null | sed -n 's/^designated => //p')"
