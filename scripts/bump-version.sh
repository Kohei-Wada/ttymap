#!/usr/bin/env bash
# Bump the workspace version. Updates the two Cargo.toml entries that
# don't auto-resolve via `version.workspace = true` and refreshes
# Cargo.lock. Stops short of commit / tag / push — those are deliberate.
#
# Usage: scripts/bump-version.sh 0.1.1

set -euo pipefail

if [ $# -ne 1 ]; then
    echo "usage: $0 <new-version>  (e.g. 0.1.1, 0.2.0-rc.1)" >&2
    exit 2
fi

NEW="$1"

# semver-ish: MAJOR.MINOR.PATCH with optional `-prerelease`. Build
# metadata (`+...`) is intentionally not allowed since cargo's
# behaviour around it is fiddly.
if ! [[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "error: '$NEW' is not a valid version (expected MAJOR.MINOR.PATCH[-pre])" >&2
    exit 2
fi

cd "$(git rev-parse --show-toplevel)"

CUR=$(grep -E '^version = "' Cargo.toml | head -1 | sed -E 's/version = "(.*)"/\1/')
if [ "$CUR" = "$NEW" ]; then
    echo "already at $NEW; nothing to do" >&2
    exit 0
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "error: working tree is dirty; commit or stash first" >&2
    exit 1
fi

# Workspace source of truth.
sed -i.bak -E "s/^version = \"$CUR\"$/version = \"$NEW\"/" Cargo.toml

# `ttymap-engine` dep in the binary crate also pins by version (so
# `cargo publish` succeeds); keep it in lock-step with the workspace.
sed -i.bak -E "s/(ttymap-engine = \{ path = \"\.\.\/ttymap-engine\", version = )\"$CUR\"/\1\"$NEW\"/" ttymap-tui/Cargo.toml

rm -f Cargo.toml.bak ttymap-tui/Cargo.toml.bak

# `cargo check` is enough to refresh Cargo.lock and prove the bump
# compiles — full build wastes time pre-tag.
cargo check --workspace --quiet

echo
echo "bumped: $CUR -> $NEW"
echo
echo "next steps:"
echo "  git diff                                  # review"
echo "  git commit -am 'chore: bump version to $NEW'"
echo "  git tag v$NEW"
echo "  git push && git push origin v$NEW         # triggers release.yml"
