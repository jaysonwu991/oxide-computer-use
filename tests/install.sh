#!/bin/sh
# `install.sh --check` against a staged release directory: no network, no macOS
# API and no install, so it runs wherever the repository's checks run.
set -eu

script="$(cd "$(dirname "$0")/.." && pwd)/install.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT INT TERM

stage() {
    mkdir -p "$work/releases/latest/download"
    {
        printf 'version: %s\n' "$1"
        printf 'macos-arm64: oxide-computer-use-macos-arm64.tar.gz\n'
        printf 'macos-x64: oxide-computer-use-macos-x64.tar.gz\n'
    } > "$work/releases/latest/download/oxide-computer-use-manifest"
}

mark() {
    mkdir -p "$work/bin"
    printf 'source example/oxide-computer-use\nversion %s\nplatform macos-arm64\nsha256 00\n' "$1" \
        > "$work/bin/.oxide-computer-use-install"
}

check() {
    OXIDE_COMPUTER_USE_RELEASES="file://$work/releases" \
        OXIDE_COMPUTER_USE_INSTALL_DIR="$work/bin" \
        sh "$script" --check >"$work/out" 2>&1
}

fail() {
    printf 'install.sh --check: %s\n' "$1" >&2
    cat "$work/out" >&2 2>/dev/null || true
    exit 1
}

stage v0.2.0
mark 0.2.0
check || fail "an installed copy at the latest version should exit 0"

stage v0.3.0
status=0
check || status=$?
[ "$status" = 1 ] || fail "an available update should exit 1, got ${status}"
grep -q '0\.2\.0' "$work/out" || fail "the installed version should be named"
grep -q '0\.3\.0' "$work/out" || fail "the available version should be named"

rm -f "$work/bin/.oxide-computer-use-install"
status=0
check || status=$?
[ "$status" = 2 ] || fail "no installed copy should exit 2, got ${status}"
grep -q 'without --check' "$work/out" || fail "the message should say how to install first"

stage v0.2.0
mark 0.2.0
rm -rf "$work/releases/latest"
status=0
check || status=$?
[ "$status" = 2 ] || fail "no release manifest should exit 2, got ${status}"
grep -q 'manifest' "$work/out" || fail "the message should name the manifest"

printf 'install.sh --check: current, behind, uninstalled and offline all reported\n'
