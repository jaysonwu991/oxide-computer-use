#!/usr/bin/env bash
# Sets the crate version from a release tag.
#
# The version in Cargo.toml is what a local build reports. Release CI calls this
# with the pushed tag before it builds, so the binary in an archive reports the
# release it came from. Refreshing Cargo.lock afterwards is the caller's job
# (`cargo update --workspace`).
#
# Usage: scripts/set-version.sh v1.2.3
set -euo pipefail

raw="${1:-}"
if [ -z "$raw" ]; then
    echo "usage: $(basename "$0") <version|tag>" >&2
    exit 2
fi

version="${raw#v}"
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'; then
    echo "not a version: ${raw}" >&2
    exit 2
fi

manifest="$(cd "$(dirname "$0")/.." && pwd)/Cargo.toml"
temporary="${manifest}.set-version"
trap 'rm -f "$temporary"' EXIT

# The first `version = "…"` is the package's; a dependency's follows it.
awk -v version="$version" '
    !patched && /^version = "/ { sub(/^version = ".*"$/, "version = \"" version "\""); patched = 1 }
    { print }
' "$manifest" > "$temporary"

if ! grep -qx "version = \"${version}\"" "$temporary"; then
    echo "did not find the package version in ${manifest}" >&2
    exit 1
fi

first_version="$(awk '/^version = "/ { print; exit }' "$temporary")"
if [ "$first_version" != "version = \"${version}\"" ]; then
    echo "the first version line in ${manifest} is not the package's: ${first_version}" >&2
    exit 1
fi

mv "$temporary" "$manifest"
printf '%s: version %s\n' "$manifest" "$version"
