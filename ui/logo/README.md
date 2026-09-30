# FinchGram logo

[中文](README.zh-Hans.md)

Brand colour: indigo `#4a5fc1`.

## svg/

- `FinchGram-icon.svg`: the app icon, a white bird on an indigo rounded square (the main version)
- `FinchGram-icon-dark.svg`, `-light.svg`, `-mono.svg`: on a dark background, on a light background,
  single colour
- `FinchGram-icon-16.svg`: for 16–20 px only, the silhouette alone (no eye, no wing line)
- `FinchGram-mark.svg`, `-black.svg`, `-white.svg`: the bird on its own, to stand next to text
- `FinchGram-mark-16.svg`: the mark as a silhouette, for small sizes

## png/

- `FinchGram-icon-16.png` … `-1024.png`: the app icon in every size (16 px is the silhouette);
  scripts/bundle.sh makes the macOS icon from them
- `FinchGram-mark-*-1024.png`: the mark on a transparent background

## Rules

- Never on a round background, and never in Telegram's blue (Telegram's API terms, 2.4: no official
  Telegram logo).
- Below 20 px, always the -16 silhouette.
- Leave space around the mark of at least a quarter of the bird's height.
- The app's name is written FinchGram (capital F and G), without "Telegram".

The vector outlines were traced from the original PNG; before a final release their curves are
worth a pass in a vector editor.
