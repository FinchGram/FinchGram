#!/usr/bin/env bash
# Publishes a release. Bumps the version, commits, tags v<version> with the release notes as the
# tag message, pushes, then watches the GitHub Actions run that builds the app on a clean runner
# and publishes it as a release of this repository (.github/workflows/release.yml).
# Nothing is built on this machine.
#
#   scripts/release.sh patch              # 0.1.0 -> 0.1.1
#   scripts/release.sh minor              # 0.1.1 -> 0.2.0
#   scripts/release.sh major              # 0.2.0 -> 1.0.0
#   scripts/release.sh 0.2.0              # exactly this version
#   scripts/release.sh patch -m "notes"   # release notes; default: the commit subjects since the previous release
#   scripts/release.sh patch --no-wait    # return right after pushing instead of watching the run
#
# Needs a clean git tree on main and `gh` logged in (brew install gh && gh auth login).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RELEASES_REPO="FinchGram/FinchGram"   # this repository; must match RELEASES_REPO in src/update.rs
WORKFLOW="release.yml"
APP_NAME="FinchGram"

usage() { sed -n '2,15p' "$0" >&2; exit 1; }

BUMP="${1:-}"
[ -n "$BUMP" ] || usage
shift
NOTES=""
WAIT=1
while [ $# -gt 0 ]; do
  case "$1" in
    -m) NOTES="${2:?-m needs the notes}"; shift 2 ;;
    --no-wait) WAIT=0; shift ;;
    *) usage ;;
  esac
done

cd "$ROOT"

# ---- preconditions ---------------------------------------------------------------------------
command -v gh >/dev/null || { echo "gh is not installed: brew install gh && gh auth login" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "gh is not logged in: gh auth login" >&2; exit 1; }
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$BRANCH" = "main" ] || { echo "releases are made from main; this is $BRANCH" >&2; exit 1; }
if [ -n "$(git status --porcelain)" ]; then
  echo "the git tree is not clean; commit first:" >&2
  git status --short >&2
  exit 1
fi

# ---- version ---------------------------------------------------------------------------------
CURRENT="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
IFS=. read -r MAJ MIN PAT <<<"$CURRENT"
case "$BUMP" in
  major) VERSION="$((MAJ + 1)).0.0" ;;
  minor) VERSION="$MAJ.$((MIN + 1)).0" ;;
  patch) VERSION="$MAJ.$MIN.$((PAT + 1))" ;;
  [0-9]*.[0-9]*.[0-9]*) VERSION="$BUMP" ;;
  *) usage ;;
esac
TAG="v$VERSION"
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "tag $TAG already exists" >&2; exit 1
fi
if gh release view "$TAG" --repo "$RELEASES_REPO" >/dev/null 2>&1; then
  echo "release $TAG already exists in $RELEASES_REPO" >&2; exit 1
fi
PREV_TAG="$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)"

if [ "$VERSION" != "$CURRENT" ]; then
  # Only the first `version = "..."` line is the package version.
  perl -0777 -pi -e "s/^version = \"[^\"]*\"/version = \"$VERSION\"/m" Cargo.toml
  # Brings Cargo.lock's own entry up to date without building anything. Not --offline: that
  # needs every platform's dependencies in the local cache, even ones never built here.
  cargo metadata --format-version 1 >/dev/null
  echo "version: $CURRENT -> $VERSION"
fi

# ---- notes: they become the tag message, which the workflow publishes ------------------------
if [ -z "$NOTES" ] && [ -n "$PREV_TAG" ]; then
  NOTES="$(git log --format='- %s' "$PREV_TAG..HEAD")"
fi
[ -n "$NOTES" ] || NOTES="$APP_NAME $VERSION."

# ---- commit, tag, push -----------------------------------------------------------------------
# Always a new commit, so that the push to main triggers the workflow. The tag goes up first:
# the workflow runs on the branch push and looks for a v* tag on the commit it builds.
git add Cargo.toml Cargo.lock
git commit -q --allow-empty -m "Release $TAG"
git tag -a "$TAG" -m "$NOTES"
git push -q origin "$TAG"
git push -q origin HEAD
SHA="$(git rev-parse HEAD)"
echo "pushed $TAG ($SHA); GitHub Actions builds and publishes it"
[ "$WAIT" = 1 ] || exit 0

# ---- watch the run ---------------------------------------------------------------------------
RUN_ID=""
for _ in $(seq 1 30); do
  RUN_ID="$(gh run list --workflow "$WORKFLOW" --commit "$SHA" --json databaseId --jq '.[0].databaseId' 2>/dev/null || true)"
  [ -n "$RUN_ID" ] && break
  sleep 2
done
[ -n "$RUN_ID" ] || { echo "cannot find the workflow run yet; watch it with: gh run list --workflow $WORKFLOW" >&2; exit 1; }
echo "run: $(gh run view "$RUN_ID" --json url --jq .url)"
if ! gh run watch "$RUN_ID" --exit-status; then
  echo "the run failed; details: gh run view $RUN_ID --log-failed" >&2
  exit 1
fi
echo
echo "released $TAG: https://github.com/$RELEASES_REPO/releases/tag/$TAG"
