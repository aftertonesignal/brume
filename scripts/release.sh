#!/usr/bin/env bash
#
# scripts/release.sh — cut a Brume release.
#
# Procedure (matches BUILD_AND_RELEASE.md §5a):
#
#   1. Verify clean working tree on `main`.
#   2. Bump [workspace.package] version in Cargo.toml.
#   3. Promote [Unreleased] in CHANGELOG.md to [X.Y.Z] - YYYY-MM-DD,
#      reseed an empty [Unreleased] block above it, and update the
#      reference-link footer.
#   4. Open $EDITOR on CHANGELOG.md so the release-cutter can edit
#      the freshly-promoted block (skipped with --no-edit).
#   5. Stage Cargo.toml + Cargo.lock + CHANGELOG.md and commit as
#      "release: cut X.Y.Z".
#   6. Create the lightweight tag vX.Y.Z (-s for signed if --sign).
#
# Dry-run by default. Pass --apply to actually mutate state.
# Print the post-script manual steps when finished.
#
# Usage:
#   scripts/release.sh 0.3.0                # dry-run
#   scripts/release.sh 0.3.0 --apply        # mutate state
#   scripts/release.sh 0.3.0 --apply --sign # signed tag
#   scripts/release.sh 0.3.0 --apply --no-edit
#
set -euo pipefail

# ------------------------------------------------------------------ args

VERSION=""
APPLY=0
NO_EDIT=0
SIGN=0

usage() {
  sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//; /^set -e/d'
  exit "${1:-2}"
}

[ $# -eq 0 ] && usage 2

while [ $# -gt 0 ]; do
  case "$1" in
    --apply)   APPLY=1 ;;
    --no-edit) NO_EDIT=1 ;;
    --sign)    SIGN=1 ;;
    -h|--help) usage 0 ;;
    -*)        echo "error: unknown flag: $1" >&2; usage 2 ;;
    *)
      if [ -n "$VERSION" ]; then
        echo "error: extra positional arg: $1" >&2; usage 2
      fi
      VERSION="$1"
      ;;
  esac
  shift
done

if [ -z "$VERSION" ]; then
  echo "error: target version required (e.g. 0.3.0)" >&2
  usage 2
fi

# Reject leading 'v' — we add that ourselves at tag time.
if [[ "$VERSION" == v* ]]; then
  echo "error: pass version as 0.3.0, not v0.3.0 (the v prefix is added at tag time)" >&2
  exit 2
fi

# Loose SemVer check; reject obvious typos but allow pre-release suffixes.
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$ ]]; then
  echo "error: '$VERSION' is not a SemVer X.Y.Z (or X.Y.Z-prerelease)" >&2
  exit 2
fi

TAG="v$VERSION"
TODAY="$(date +%Y-%m-%d)"

# ------------------------------------------------------------------ helpers

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

CARGO_TOML="$REPO_ROOT/Cargo.toml"
CHANGELOG="$REPO_ROOT/CHANGELOG.md"

note()  { printf '  %s\n' "$*"; }
step()  { printf '\n==> %s\n' "$*"; }
plan()  { [ "$APPLY" -eq 1 ] && printf '    [apply] %s\n' "$*" || printf '    [dry]   %s\n' "$*"; }
abort() { echo "error: $*" >&2; exit 1; }

run_or_show() {
  if [ "$APPLY" -eq 1 ]; then "$@"; else printf '          $ %s\n' "$*"; fi
}

# ------------------------------------------------------------------ preflight

step "preflight"

if [ "$APPLY" -eq 1 ]; then
  note "mode: APPLY (mutating state)"
else
  note "mode: dry-run (no changes will be made; pass --apply to mutate)"
fi
note "target: $VERSION  (tag: $TAG, date: $TODAY)"

# Must be inside a git repo.
git rev-parse --git-dir >/dev/null 2>&1 || abort "not a git repository"

# Branch must be main.
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [ "$BRANCH" != "main" ]; then
  abort "current branch is '$BRANCH'; cut releases from main"
fi
note "branch: $BRANCH"

# Working tree must be clean.
if ! git diff --quiet || ! git diff --cached --quiet; then
  abort "working tree has uncommitted changes; commit or stash first"
fi
if [ -n "$(git ls-files --others --exclude-standard)" ]; then
  abort "working tree has untracked files; clean up or .gitignore them first"
fi
note "working tree: clean"

# Tag must not already exist (locally or on the remote).
if git rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null; then
  abort "tag $TAG already exists locally"
fi
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  abort "tag $TAG already exists on origin"
fi
note "tag $TAG: available"

# Required files exist.
[ -f "$CARGO_TOML" ] || abort "missing $CARGO_TOML"
[ -f "$CHANGELOG"  ] || abort "missing $CHANGELOG"

# Read current workspace version. The first version = "..." line
# under [workspace.package] is the source of truth.
CURRENT="$(awk '
  /^\[workspace\.package\]/ { in_ws = 1; next }
  /^\[/                     { in_ws = 0 }
  in_ws && /^version = "/   { match($0, /"[^"]+"/); print substr($0, RSTART+1, RLENGTH-2); exit }
' "$CARGO_TOML")"

if [ -z "$CURRENT" ]; then
  abort "could not parse current version from [workspace.package] in Cargo.toml"
fi
note "current workspace version: $CURRENT"

if [ "$CURRENT" = "$VERSION" ]; then
  abort "current version is already $VERSION; nothing to bump"
fi

# ------------------------------------------------------------------ Cargo.toml

step "bump Cargo.toml [workspace.package] version: $CURRENT -> $VERSION"

if [ "$APPLY" -eq 1 ]; then
  # Match only the version line under [workspace.package], not later
  # version = "..." lines under [workspace.dependencies].
  awk -v target="$VERSION" '
    /^\[workspace\.package\]/  { in_ws = 1; print; next }
    /^\[/                       { in_ws = 0 }
    in_ws && /^version = "/    { print "version = \"" target "\""; next }
                                { print }
  ' "$CARGO_TOML" > "$CARGO_TOML.tmp"
  mv "$CARGO_TOML.tmp" "$CARGO_TOML"
  plan "Cargo.toml updated"
else
  plan "would update Cargo.toml: version = \"$VERSION\""
fi

# Refresh Cargo.lock so its workspace-crate entries reflect the new version.
step "refresh Cargo.lock"
if [ "$APPLY" -eq 1 ]; then
  if command -v cargo >/dev/null 2>&1; then
    cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace
    plan "Cargo.lock refreshed via cargo update --workspace"
  else
    note "cargo not on PATH; skipping Cargo.lock refresh (CI will catch the drift)"
  fi
else
  plan "would run: cargo update --workspace"
fi

# ------------------------------------------------------------------ CHANGELOG.md

step "promote [Unreleased] -> [$VERSION] - $TODAY in CHANGELOG.md"

# Sanity: the file must contain an [Unreleased] header and a footer
# reference link for it. If either is missing, refuse rather than
# guessing.
grep -q '^## \[Unreleased\]'           "$CHANGELOG" || abort "no '## [Unreleased]' header in CHANGELOG.md"
grep -q '^\[Unreleased\]: '            "$CHANGELOG" || abort "no '[Unreleased]: ...' footer link in CHANGELOG.md"

if [ "$APPLY" -eq 1 ]; then
  awk -v ver="$VERSION" -v today="$TODAY" '
    BEGIN { promoted = 0; tag_link_done = 0 }

    # Promote the Unreleased header on first sight: emit a fresh
    # empty Unreleased block (inline so we stay portable across
    # BSD awk, which rejects multi-line -v assignments), then the
    # promoted X.Y.Z header.
    /^## \[Unreleased\]/ && !promoted {
      print "## [Unreleased]"
      print ""
      print "### Added"
      print "- (none yet — entries land here per PR / landing during the dev cycle)"
      print ""
      print "### Changed"
      print "- (none yet)"
      print ""
      print "### Fixed"
      print "- (none yet)"
      print ""
      print "## [" ver "] - " today
      promoted = 1
      next
    }

    # Rewrite the [Unreleased] compare-link footer to point at the new tag.
    /^\[Unreleased\]: / {
      line = $0
      sub(/v[0-9A-Za-z.+-]+\.\.\.HEAD/, "v" ver "...HEAD", line)
      print line
      next
    }

    # Insert a new [X.Y.Z]: tag-link above the first existing tag-link.
    /^\[[^]]+\]: .*releases\/tag\/v/ && !tag_link_done {
      new_link = $0
      sub(/v[0-9A-Za-z.+-]+$/, "v" ver, new_link)
      sub(/^\[[^]]+\]:/, "[" ver "]:", new_link)
      print new_link
      print
      tag_link_done = 1
      next
    }

    { print }
  ' "$CHANGELOG" > "$CHANGELOG.tmp"
  mv "$CHANGELOG.tmp" "$CHANGELOG"
  plan "CHANGELOG.md promoted (empty [Unreleased] reseeded; footer links updated)"
else
  plan "would promote ## [Unreleased] -> ## [$VERSION] - $TODAY"
  plan "would reseed empty ## [Unreleased] block at the top"
  plan "would rewrite [Unreleased]: footer to compare/v$VERSION...HEAD"
  plan "would insert [$VERSION]: footer link above the previous version's link"
fi

# ------------------------------------------------------------------ editor

if [ "$NO_EDIT" -eq 0 ] && [ "$APPLY" -eq 1 ]; then
  step "open CHANGELOG.md in \$EDITOR for hand-editing"
  EDITOR_BIN="${EDITOR:-${VISUAL:-vi}}"
  note "editor: $EDITOR_BIN"
  note "fill in real entries under the new [$VERSION] block, save, and exit"
  "$EDITOR_BIN" "$CHANGELOG" || abort "editor exited non-zero; aborting before commit"
elif [ "$NO_EDIT" -eq 1 ]; then
  step "skipping editor (--no-edit)"
else
  step "would open \$EDITOR on CHANGELOG.md (skipped in dry-run)"
fi

# ------------------------------------------------------------------ commit + tag

step "commit + tag"

COMMIT_MSG="release: cut $VERSION"

if [ "$APPLY" -eq 1 ]; then
  git add Cargo.toml Cargo.lock CHANGELOG.md
  git commit -m "$COMMIT_MSG"
  plan "committed: $COMMIT_MSG"

  if [ "$SIGN" -eq 1 ]; then
    git tag -s "$TAG" -m "Brume $VERSION"
    plan "tagged (signed): $TAG"
  else
    git tag "$TAG"
    plan "tagged (lightweight): $TAG"
  fi
else
  plan "would: git add Cargo.toml Cargo.lock CHANGELOG.md"
  plan "would: git commit -m \"$COMMIT_MSG\""
  if [ "$SIGN" -eq 1 ]; then
    plan "would: git tag -s $TAG -m \"Brume $VERSION\""
  else
    plan "would: git tag $TAG"
  fi
fi

# ------------------------------------------------------------------ next steps

step "done"

cat <<EOF

Next steps (manual):

  1. Sanity-check the diff:
       git show HEAD --stat
       git log -1 --format='%s%n%n%b'

  2. Push the commit and tag together:
       git push --follow-tags

  3. The release.yml workflow runs on the v* tag push:
       - matrix-builds the four target binaries
       - extracts the [$VERSION] block from CHANGELOG.md as the body
       - publishes a GitHub Release at refs/tags/$TAG with the four
         binaries attached

  4. Verify on the Releases page once the workflow completes:
       https://github.com/aftertonesignal/brume/releases/tag/$TAG

  5. If anything went wrong, fix forward in $VERSION-next, do not
     delete the tag. (See BUILD_AND_RELEASE.md §5c.)

EOF

if [ "$APPLY" -eq 0 ]; then
  printf '(this was a dry run — re-run with --apply to actually cut the release)\n\n'
fi
