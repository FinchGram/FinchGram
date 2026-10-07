#!/usr/bin/env bash
# Downloads the pinned UI fonts into vendor/fonts/. They are compiled into the app (src/fonts.rs),
# so every user sees the same letters whatever is installed on their Mac; see docs/conventions.md.
#
# Source: the official Google Fonts repository (github.com/google/fonts), the same files the design
# was made with: IBM Plex Sans and Mono (the Workbench theme), Source Sans 3 (Broadsheet),
# JetBrains Mono (Terminal), and Noto Sans SC, JP, KR and Arabic for the UI's Chinese, Japanese,
# Korean and Arabic in all three. All of them are under the SIL Open Font License 1.1; the licence
# texts travel with the app (scripts/bundle.sh).
#
# To upgrade: point COMMIT at a newer google/fonts commit, update the SHA-256 values below, commit,
# and every developer re-runs this script.
set -euo pipefail

# ---- pinned version ---------------------------------------------------------------
COMMIT="23e54b51ddffbc7713c583748e3bd86f62b1fa4a"   # google/fonts, 2026-09-24
# local name | path in google/fonts (URL-encoded) | SHA-256
FILES="
IBMPlexSans-Variable.ttf|ofl/ibmplexsans/IBMPlexSans%5Bwdth,wght%5D.ttf|3b031aa4216174205bd8471f88a49b91f093169e9e87bd5262242bc5967fe2e3
IBMPlexMono-Regular.ttf|ofl/ibmplexmono/IBMPlexMono-Regular.ttf|6a3412f058c7d8dfd9170c41e85ade48e5156ecb89356110ca57a0a27734af46
IBMPlexMono-Medium.ttf|ofl/ibmplexmono/IBMPlexMono-Medium.ttf|a9b4c49bb299e05b5f6c481e7fb5e78943d2793249a0c8874ab574a2d1ea6755
IBMPlexMono-SemiBold.ttf|ofl/ibmplexmono/IBMPlexMono-SemiBold.ttf|d3c38e55c78f5b0f28009fddba4834ec503278936a5986032424c9bd2d23aa46
SourceSans3-Variable.ttf|ofl/sourcesans3/SourceSans3%5Bwght%5D.ttf|042fe2cc0b933e328410d7acbd0aa6a1873dca5aef81875f4bc214b08825c7b9
SourceSans3-Italic-Variable.ttf|ofl/sourcesans3/SourceSans3-Italic%5Bwght%5D.ttf|39e3ab05ccd7cb94907c31005bb5bec1d5432f0b096a2b782976e217a540eb6c
JetBrainsMono-Variable.ttf|ofl/jetbrainsmono/JetBrainsMono%5Bwght%5D.ttf|48715a42ec242c21e9f02692891e147d022299a52e48d5e413e1a942193ffeda
NotoSansSC-Variable.ttf|ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf|a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da
NotoSansJP-Variable.ttf|ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf|c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f
NotoSansKR-Variable.ttf|ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf|194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252
NotoSansArabic-Variable.ttf|ofl/notosansarabic/NotoSansArabic%5Bwdth,wght%5D.ttf|63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e
IBMPlex-OFL.txt|ofl/ibmplexsans/OFL.txt|7e6b2818edbd8f6a01ae80641cc8f16a51080d08fb4e532be3a0b6f74adb07da
SourceSans3-OFL.txt|ofl/sourcesans3/OFL.txt|09746787287a289323b0ec3cff4d1a4a801331b82b7207c1e186f5d26619a392
JetBrainsMono-OFL.txt|ofl/jetbrainsmono/OFL.txt|b2fe5e8987594e9ffd1d2ca52a2f5d73eb8335243893c5d6254b5ad69269591d
NotoSans-OFL.txt|ofl/notosanssc/OFL.txt|1c05c68c34f9708415aada51f17e1b0092d2cea709bf4a94cd38114f9e73d7d9
NotoSansArabic-OFL.txt|ofl/notosansarabic/OFL.txt|07fc70bfeb985cc1a87a8587d0a0c80bab11c86c9dc3fd95b6f0cb332f983e96
"
# -----------------------------------------------------------------------------------

BASE="https://raw.githubusercontent.com/google/fonts/$COMMIT"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/vendor/fonts"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

while IFS='|' read -r name path sha; do
  [ -n "$name" ] || continue
  echo "downloading $name"
  curl -fsSL --retry 3 -o "$TMP/$name" "$BASE/$path"
  actual="$(shasum -a 256 "$TMP/$name" | cut -d' ' -f1)"
  if [ "$actual" != "$sha" ]; then
    echo "SHA-256 mismatch for $name" >&2
    echo "  expected $sha" >&2
    echo "  actual   $actual" >&2
    exit 1
  fi
done <<< "$FILES"

mkdir -p "$DEST"
cp "$TMP"/* "$DEST/"
echo
echo "installed $(ls "$DEST" | wc -l | tr -d ' ') files into $DEST ($(du -sh "$DEST" | cut -f1))"
