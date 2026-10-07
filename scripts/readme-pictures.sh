#!/usr/bin/env bash
# Copies the pictures the READMEs show into docs/screenshots/ from the ones the picture test draws
# (`cargo test screenshots -- --ignored`, src/screenshots.rs: every page, with made-up chats).
# Run it after a change to the UI, then commit docs/screenshots/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FROM="$ROOT/target/screenshots"
TO="$ROOT/docs/screenshots"
[ -d "$FROM" ] || { echo "no pictures yet: cargo test screenshots -- --ignored" >&2; exit 1; }

PICTURES=(
  workbench-light-chats
  workbench-dark-chats
  broadsheet-light-chats
  terminal-light-chats
  workbench-light-viewer-photo
  workbench-light-shot-annotated
  workbench-light-shot-send-card
  workbench-light-settings-general
  zh-workbench-chats
)
mkdir -p "$TO"
for name in "${PICTURES[@]}"; do
  cp "$FROM/$name.png" "$TO/$name.png"
done
echo "copied ${#PICTURES[@]} pictures into docs/screenshots/"
