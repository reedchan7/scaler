#!/usr/bin/env bash
#
# bump-version.sh — set the scaler crate version in Cargo.toml,
# refresh Cargo.lock, and prepend a CHANGELOG.md placeholder.
#
# Usage:
#   scripts/bump-version.sh                  # patch +1 (default)
#   scripts/bump-version.sh patch            # patch +1
#   scripts/bump-version.sh minor            # minor +1, patch=0
#   scripts/bump-version.sh major            # major +1, minor=0, patch=0
#   scripts/bump-version.sh 1.2.3            # set explicit version
#
# Env:
#   CARGO=cargo-nightly                       # override cargo binary
#   SKIP_LOCK=1                               # skip Cargo.lock refresh
#   SKIP_CHANGELOG=1                          # skip CHANGELOG insert
#
# Exit codes:
#   0  ok
#   1  invalid argument
#   2  unable to parse current version
#   3  Cargo.lock refresh failed

set -euo pipefail

CARGO="${CARGO:-cargo}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO_TOML="$ROOT/Cargo.toml"
CARGO_LOCK="$ROOT/Cargo.lock"
CHANGELOG="$ROOT/CHANGELOG.md"

arg="${1:-patch}"

# Read the [package] version. Stops at the first match so dependency versions
# in [dependencies] sections are never picked up.
current="$(awk '
    /^\[package\]/ { in_pkg = 1; next }
    /^\[/          { in_pkg = 0 }
    in_pkg && /^version[[:space:]]*=/ {
        gsub(/^version[[:space:]]*=[[:space:]]*"/, "")
        gsub(/".*$/, "")
        print
        exit
    }
' "$CARGO_TOML")"

if [[ -z "$current" ]]; then
    echo "ERROR: cannot parse current version from $CARGO_TOML" >&2
    exit 2
fi

# Validate the current version follows semver MAJOR.MINOR.PATCH so the bump
# math has something to work with.
if ! [[ "$current" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
    echo "ERROR: current version '$current' is not semver MAJOR.MINOR.PATCH" >&2
    exit 2
fi
major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"
patch="${BASH_REMATCH[3]}"

# Compute the new version. If the argument looks like a semver, treat it as
# an explicit version; otherwise treat it as a bump keyword.
if [[ "$arg" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    new="$arg"
else
    case "$arg" in
        major)
            new="$((major + 1)).0.0"
            ;;
        minor)
            new="${major}.$((minor + 1)).0"
            ;;
        patch | "")
            new="${major}.${minor}.$((patch + 1))"
            ;;
        *)
            echo "ERROR: argument must be 'patch' / 'minor' / 'major' or an explicit X.Y.Z (got: $arg)" >&2
            exit 1
            ;;
    esac
fi

if [[ "$new" == "$current" ]]; then
    echo "version is already $new, nothing to do" >&2
    exit 0
fi

echo "$current → $new"

# Update Cargo.toml in place. macOS sed needs the empty -i argument.
if [[ "$(uname)" == "Darwin" ]]; then
    sed -i '' -E "s/^version[[:space:]]*=[[:space:]]*\"$current\"/version = \"$new\"/" "$CARGO_TOML"
else
    sed -i -E "s/^version[[:space:]]*=[[:space:]]*\"$current\"/version = \"$new\"/" "$CARGO_TOML"
fi

# Verify the substitution actually happened (defensive — sed exit status alone
# doesn't tell us whether the pattern matched).
verify="$(awk '
    /^\[package\]/ { in_pkg = 1; next }
    /^\[/          { in_pkg = 0 }
    in_pkg && /^version[[:space:]]*=/ {
        gsub(/^version[[:space:]]*=[[:space:]]*"/, "")
        gsub(/".*$/, "")
        print
        exit
    }
' "$CARGO_TOML")"
if [[ "$verify" != "$new" ]]; then
    echo "ERROR: failed to update Cargo.toml (still says '$verify')" >&2
    exit 2
fi
echo "  Cargo.toml: $new"

# Refresh Cargo.lock so the package version stamp matches. We use --offline
# first because the version bump alone shouldn't need a network round trip;
# fall back to a normal update if --offline rejects the operation.
if [[ -f "$CARGO_LOCK" && "${SKIP_LOCK:-0}" != "1" ]]; then
    if ! "$CARGO" update -p scaler --offline >/dev/null 2>&1; then
        if ! "$CARGO" update -p scaler >/dev/null 2>&1; then
            echo "ERROR: failed to refresh Cargo.lock" >&2
            exit 3
        fi
    fi
    echo "  Cargo.lock: refreshed"
fi

# Prepend a CHANGELOG entry placeholder right under the title.
if [[ -f "$CHANGELOG" && "${SKIP_CHANGELOG:-0}" != "1" ]]; then
    tmp="$(mktemp)"
    awk -v ver="$new" '
        BEGIN { inserted = 0 }
        # Insert immediately after the first blank line that follows the title.
        !inserted && NR > 1 && /^## / {
            print "## " ver
            print ""
            print "- TODO: describe what changed in " ver
            print ""
            inserted = 1
        }
        { print }
    ' "$CHANGELOG" > "$tmp"
    mv "$tmp" "$CHANGELOG"
    echo "  CHANGELOG.md: placeholder inserted for $new"
fi

if [[ -f "$CHANGELOG" ]]; then
    add_files="Cargo.toml Cargo.lock CHANGELOG.md"
    edit_step="
  1. edit CHANGELOG.md and fill in the $new section"
    next_n=2
else
    add_files="Cargo.toml Cargo.lock"
    edit_step=""
    next_n=1
fi

cat <<NEXT

next steps:${edit_step}
  ${next_n}. git add ${add_files}
  $((next_n + 1)). git commit -m "chore: bump version to $new"
  $((next_n + 2)). git tag v$new
  $((next_n + 3)). git push origin main v$new
NEXT
