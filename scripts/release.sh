#!/usr/bin/env bash
set -euo pipefail

# hnfc — release helper
# Usage: ./scripts/release.sh 0.2.0 [--dry-run] [--no-push] [--force] [--allow-dirty]
#        ./scripts/release.sh v0.2.0
# Does:
#   1. validates semver & that tag doesn't already exist
#   2. bumps version in Cargo.toml (+ Cargo.lock) — the stored version
#   3. git commit + annotated tag vX.Y.Z
#   4. git push (branch + tag) -> triggers .github/workflows/release.yml
#
# Flags:
#   --dry-run     show what would happen, don't write/commit/push
#   --no-push     commit+tag locally but don't push
#   --force       allow bumping to same or lower version
#   --allow-dirty allow running with uncommitted changes
#   --remote NAME git remote to push to (default: origin)
#   --branch NAME branch to push (default: current branch / main)

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REMOTE="origin"
BRANCH=""
DRY_RUN=0
NO_PUSH=0
FORCE=0
ALLOW_DIRTY=0
VERSION=""

usage() {
  cat <<'USAGE'
Usage: scripts/release.sh <version> [options]

  version            semver like 0.2.0 or v0.2.0 (stored without 'v' in Cargo.toml, tag is vX.Y.Z)

Options:
  --dry-run          don't write files, commit, tag or push — just print plan
  --no-push          commit and tag locally, but don't push
  --force            allow same or lower version (skip version-order check)
  --allow-dirty      allow with uncommitted changes / untracked files
  --remote NAME      git remote (default: origin)
  --branch NAME      branch to push (default: current branch)
  -h, --help         show this help

Examples:
  scripts/release.sh 0.2.0
  scripts/release.sh v0.2.0 --dry-run
  scripts/release.sh 0.3.0 --no-push
  HNFC_VERSION=0.2.0 scripts/release.sh   # also reads $HNFC_VERSION / $VERSION env

Workflow: tag push triggers .github/workflows/release.yml which builds
  linux (amd64/arm64 musl), macOS (amd64/arm64), Windows (amd64) and creates the GitHub Release.
USAGE
}

# allow VERSION from env as fallback if not given as arg
ENV_VERSION="${HNFC_VERSION:-${VERSION:-}}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --dry-run) DRY_RUN=1; shift ;;
    --no-push) NO_PUSH=1; shift ;;
    --force) FORCE=1; shift ;;
    --allow-dirty) ALLOW_DIRTY=1; shift ;;
    --remote) REMOTE="$2"; shift 2 ;;
    --remote=*) REMOTE="${1#*=}"; shift ;;
    --branch) BRANCH="$2"; shift 2 ;;
    --branch=*) BRANCH="${1#*=}"; shift ;;
    --) shift; break ;;
    -*) echo "Unknown flag: $1" >&2; usage >&2; exit 1 ;;
    *)
      if [[ -z "$VERSION" ]]; then VERSION="$1"; else echo "Extra arg: $1" >&2; exit 1; fi
      shift ;;
  esac
done

if [[ -z "$VERSION" ]]; then VERSION="$ENV_VERSION"; fi
if [[ -z "$VERSION" ]]; then echo "error: version required" >&2; usage >&2; exit 1; fi

# normalize: strip leading v, trim
VERSION="${VERSION#v}"
VERSION="$(echo "$VERSION" | tr -d '[:space:]')"

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
  echo "error: version '$VERSION' is not semver (expected like 0.2.0 or 1.0.0-rc.1)" >&2
  exit 1
fi

TAG="v$VERSION"

if [[ -z "$BRANCH" ]]; then
  BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)"
fi

# checks
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "error: not a git repo" >&2; exit 1
fi
if ! git remote get-url "$REMOTE" >/dev/null 2>&1; then
  echo "error: remote '$REMOTE' not found. Remotes:" >&2
  git remote -v >&2
  exit 1
fi

# uncommitted / untracked check
if [[ "$ALLOW_DIRTY" -eq 0 ]]; then
  if ! git diff --quiet 2>/dev/null; then
    echo "error: working tree has unstaged changes. Commit or stash, or pass --allow-dirty." >&2
    git status --short >&2
    exit 1
  fi
  if ! git diff --cached --quiet 2>/dev/null; then
    echo "error: index has staged changes. Commit or stash, or pass --allow-dirty." >&2
    git status --short >&2
    exit 1
  fi
fi

CURRENT="$(awk -F'"' '/^\[package\]/{p=1;next} /^\[/{p=0} p && /^version[[:space:]]*=/ {print $2; exit}' Cargo.toml 2>/dev/null || echo "0.0.0")"
if [[ -z "$CURRENT" ]]; then CURRENT="0.0.0"; fi

if [[ "$VERSION" == "$CURRENT" ]]; then
  echo "warn: version $VERSION is same as current $CURRENT" >&2
  if [[ "$FORCE" -eq 0 ]]; then echo "pass --force to allow" >&2; exit 1; fi
fi

# version ordering (sort -V)
if [[ "$FORCE" -eq 0 ]]; then
  SMALLEST="$(printf '%s\n%s\n' "$CURRENT" "$VERSION" | sort -V | head -n1)"
  if [[ "$SMALLEST" == "$VERSION" && "$VERSION" != "$CURRENT" ]]; then
    echo "error: $VERSION < $CURRENT (downgrade). Pass --force to allow." >&2
    exit 1
  fi
fi

if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "error: tag $TAG already exists locally" >&2
  exit 1
fi
if git ls-remote --tags "$REMOTE" 2>/dev/null | grep -q "refs/tags/$TAG$"; then
  echo "error: tag $TAG already exists on remote $REMOTE" >&2
  exit 1
fi

echo "Release plan:"
echo "  current Cargo.toml version : $CURRENT"
echo "  new version                : $VERSION"
echo "  tag                        : $TAG"
echo "  remote / branch            : $REMOTE / $BRANCH"
echo "  dry-run                    : $DRY_RUN"
echo "  no-push                    : $NO_PUSH"
echo ""

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[dry-run] would update Cargo.toml and Cargo.lock, then:"
  echo "  git add Cargo.toml Cargo.lock"
  echo "  git commit -m \"chore: release $TAG\""
  echo "  git tag -a $TAG -m \"$TAG\""
  echo "  git push $REMOTE $BRANCH"
  echo "  git push $REMOTE $TAG"
  echo ""
  echo "[dry-run] Cargo.toml diff preview:"
  awk -v ver="$VERSION" '
    BEGIN{in_pkg=0}
    /^\[package\]/ {in_pkg=1; print; next}
    /^\[/ {in_pkg=0}
    in_pkg && /^version[[:space:]]*=/ { sub(/"[^"]+"/, "\"" ver "\""); in_pkg=0 }
    {print}
  ' Cargo.toml | diff -u Cargo.toml - || true
  exit 0
fi

# --- bump Cargo.toml (only [package] version) ---
awk -v ver="$VERSION" '
  BEGIN{in_pkg=0}
  /^\[package\]/ {in_pkg=1; print; next}
  /^\[/ {in_pkg=0}
  in_pkg && /^version[[:space:]]*=/ { sub(/"[^"]+"/, "\"" ver "\""); in_pkg=0 }
  {print}
' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml

# --- bump Cargo.lock entry for hnfc ---
if [[ -f Cargo.lock ]]; then
  awk -v ver="$VERSION" '
    BEGIN{in_hnfc=0}
    /^\[\[package\]\]/ {in_hnfc=0}
    /^name = "hnfc"/ {in_hnfc=1}
    in_hnfc && /^version = "/ { sub(/"[^"]+"/, "\"" ver "\""); in_hnfc=0 }
    {print}
  ' Cargo.lock > Cargo.lock.tmp && mv Cargo.lock.tmp Cargo.lock
fi

echo "Updated Cargo.toml -> $VERSION"
grep -E '^version =' Cargo.toml | head -1 || true

# verify
NEW_CURRENT="$(awk -F'"' '/^\[package\]/{p=1;next} /^\[/{p=0} p && /^version[[:space:]]*=/ {print $2; exit}' Cargo.toml)"
if [[ "$NEW_CURRENT" != "$VERSION" ]]; then
  echo "error: failed to update Cargo.toml (got $NEW_CURRENT)" >&2
  exit 1
fi

git add Cargo.toml Cargo.lock 2>/dev/null || git add Cargo.toml
# if nothing to commit (e.g. Cargo.lock unchanged and version already set) handle gracefully
if git diff --cached --quiet; then
  echo "No changes to commit (version already $VERSION)."
else
  git commit -m "chore: release $TAG"
  echo "Committed: chore: release $TAG"
fi

# create annotated tag
if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "Tag $TAG already exists locally (created above or pre-existing), skipping tag creation."
else
  git tag -a "$TAG" -m "$TAG"
  echo "Created tag $TAG"
fi

if [[ "$NO_PUSH" -eq 1 ]]; then
  echo ""
  echo "Skipping push (--no-push). To push later:"
  echo "  git push $REMOTE $BRANCH"
  echo "  git push $REMOTE $TAG"
  exit 0
fi

echo ""
echo "Pushing branch and tag to $REMOTE..."
git push "$REMOTE" "$BRANCH"
git push "$REMOTE" "$TAG"

echo ""
echo "Done. Release $TAG pushed — GitHub Actions will build and publish the release."
echo "Watch: https://github.com/$(git remote get-url "$REMOTE" | sed -E 's/.*github.com[:\/](.*)\.git/\1/;s/.*github.com[:\/](.*)/\1/')/actions"
