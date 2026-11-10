#!/bin/sh
# Install the oxide-reins MCP server.
#
#   curl -fsSL https://raw.githubusercontent.com/jaysonwu991/oxide-reins/main/install.sh | sh
#
#   sh install.sh --check     # 0 when the installed copy is the latest release,
#                             # 1 when an update is available, 2 when unknown
#
# Environment overrides:
#   OXIDE_REINS_VERSION      version to install, with or without a leading "v"
#                             (default: the latest release)
#   OXIDE_REINS_INSTALL_DIR  directory to install the binary into
#                             (default: $HOME/.local/bin)
#   OXIDE_REINS_REPO         GitHub repository slug
#                             (default: jaysonwu991/oxide-reins)
#   OXIDE_REINS_RELEASES     base URL the manifest and archives come from
#                             (default: https://github.com/<repo>/releases)
set -eu

REPO="${OXIDE_REINS_REPO:-jaysonwu991/oxide-reins}"
VERSION="${OXIDE_REINS_VERSION:-}"
INSTALL_DIR="${OXIDE_REINS_INSTALL_DIR:-$HOME/.local/bin}"
RELEASES="${OXIDE_REINS_RELEASES:-https://github.com/${REPO}/releases}"
BINARY_NAME="oxide-reins"
MANIFEST_NAME="oxide-reins-manifest"
# A file beside the binary naming the install it came from, so a machine can
# tell a released copy from one built in a checkout.
MARKER_NAME=".oxide-reins-install"

err() {
    printf 'oxide-reins-install: error: %s\n' "$*" >&2
    exit 1
}

refuse() {
    printf 'oxide-reins-install: error: %s\n' "$*" >&2
    exit 2
}

warn() {
    printf 'oxide-reins-install: warning: %s\n' "$*" >&2
}

info() {
    printf 'oxide-reins-install: %s\n' "$*" >&2
}

download() {
    if command -v curl >/dev/null 2>&1; then
        curl --fail --silent --show-error --location "$1" --output "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget --quiet --output-document="$2" "$1"
    else
        err "curl or wget is required"
    fi
}

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl --fail --silent --show-error --location "$1"
    else
        wget --quiet --output-document=- "$1"
    fi
}

detect_platform() {
    os="$(uname -s)"
    arch="$(uname -m)"

    if [ "$os" != "Darwin" ]; then
        err "no ${os} build: this server drives the macOS accessibility and event APIs, and a build for another OS only returns that error"
    fi

    case "$arch" in
        arm64 | aarch64) arch_part="arm64" ;;
        x86_64 | amd64) arch_part="x64" ;;
        *) err "unsupported architecture: $arch" ;;
    esac

    printf 'macos-%s\n' "$arch_part"
}

# Prints the URL of the archive for one platform. A pinned version needs no
# manifest; the latest release is read from one, so the asset names can change
# between releases without breaking this script.
resolve_archive() {
    platform="$1"
    archive="${BINARY_NAME}-${platform}.tar.gz"

    if [ -n "$VERSION" ]; then
        printf '%s/download/v%s/%s\n' "$RELEASES" "${VERSION#v}" "$archive"
        return 0
    fi

    manifest_url="${RELEASES}/latest/download/${MANIFEST_NAME}"
    manifest="$(fetch "$manifest_url" 2>/dev/null)" || manifest=""
    if [ -z "$manifest" ]; then
        err "no release manifest at ${manifest_url}; pass OXIDE_REINS_VERSION=v<version> to pin a release, or build from source"
    fi

    asset="$(printf '%s\n' "$manifest" |
        awk -v key="${platform}:" '$1 == key { print $2 }')"
    if [ -z "$asset" ]; then
        err "the release manifest lists no ${platform} archive: ${manifest_url}"
    fi

    printf '%s/latest/download/%s\n' "$RELEASES" "$asset"
}

sha256_of() {
    file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | cut -d' ' -f1
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$file" | sed 's/^.*= //'
    else
        return 1
    fi
}

# The version a release names, from the manifest the installer already reads.
latest_version() {
    manifest="$(fetch "${RELEASES}/latest/download/${MANIFEST_NAME}" 2>/dev/null)" || manifest=""
    [ -n "$manifest" ] || return 1
    printf '%s\n' "$manifest" | awk '$1 == "version:" { print $2 }'
}

# The version recorded beside an installed copy by a previous run.
installed_version() {
    marker="${INSTALL_DIR}/${MARKER_NAME}"
    [ -f "$marker" ] || return 1
    awk '$1 == "version" { print $2 }' "$marker"
}

# 0 current, 1 update available, 2 cannot tell. Versions are compared as the
# release stamp writes them, with any leading "v" removed, so nothing here has
# to understand ordering.
check() {
    installed="$(installed_version)" ||
        refuse "no installed copy in ${INSTALL_DIR}; run this installer without --check first"
    latest="$(latest_version)" ||
        refuse "no release manifest at ${RELEASES}/latest/download/${MANIFEST_NAME}"
    installed="${installed#v}"
    latest="${latest#v}"
    if [ -z "$latest" ]; then
        refuse "the release manifest names no version"
    fi
    if [ "$installed" = "$latest" ]; then
        info "oxide-reins ${installed} is the latest release"
        return 0
    fi
    info "oxide-reins ${installed} is installed; ${latest} is available"
    return 1
}

verify_checksum() {
    file="$1"
    sums="$2"
    expected="$(cut -d' ' -f1 < "$sums" | head -n1)"
    [ -n "$expected" ] || return 0

    if ! actual="$(sha256_of "$file")"; then
        warn "no sha256 tool found; skipping checksum verification"
        return 0
    fi

    [ "$expected" = "$actual" ] || err "checksum verification failed; the download was truncated or replaced"
}

main() {
    case "${1:-}" in
        --check | check)
            check
            exit $?
            ;;
        --help | -h | help)
            sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        "") ;;
        *)
            err "unknown argument: $1 (the only one is --check)"
            ;;
    esac

    command -v uname >/dev/null 2>&1 || err "uname is required"
    command -v tar >/dev/null 2>&1 || err "tar is required"
    command -v mktemp >/dev/null 2>&1 || err "mktemp is required"

    platform="$(detect_platform)"
    url="$(resolve_archive "$platform")"
    archive="${url##*/}"

    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT INT TERM

    info "downloading ${archive}"
    download "$url" "${tmp}/${archive}" || err "download failed: ${url}"

    if download "${url}.sha256" "${tmp}/${archive}.sha256" 2>/dev/null; then
        verify_checksum "${tmp}/${archive}" "${tmp}/${archive}.sha256"
    else
        warn "checksum file unavailable; skipping verification"
    fi

    tar -xzf "${tmp}/${archive}" -C "$tmp"
    [ -f "${tmp}/${BINARY_NAME}" ] || err "the archive does not contain ${BINARY_NAME}"

    # Run the download before moving it into place: a macOS quarantine flag or an
    # archive for the other architecture fails here, not after the install.
    "${tmp}/${BINARY_NAME}" --version >/dev/null 2>&1 ||
        err "the downloaded binary did not run; check the architecture, or clear the download's quarantine flag"

    mkdir -p "$INSTALL_DIR"
    mv -f "${tmp}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod 0755 "${INSTALL_DIR}/${BINARY_NAME}"

    # The version is read back from the installed binary, so the marker names
    # what is here rather than what was asked for.
    installed_version="$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk '{print $2}')" \
        || installed_version=""
    installed_sha="$(sha256_of "${INSTALL_DIR}/${BINARY_NAME}" 2>/dev/null)" \
        || installed_sha=""
    printf 'source %s\nversion %s\nplatform %s\nsha256 %s\n' \
        "$REPO" "$installed_version" "$platform" "$installed_sha" \
        > "${INSTALL_DIR}/${MARKER_NAME}"

    info "installed ${BINARY_NAME} ${installed_version} to ${INSTALL_DIR}/${BINARY_NAME}"

    case ":${PATH:-}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            warn "${INSTALL_DIR} is not on your PATH"
            printf 'oxide-reins-install: add it with: export PATH="%s:$PATH"\n' "$INSTALL_DIR" >&2
            ;;
    esac

    info "next: ${BINARY_NAME} doctor, then ${BINARY_NAME} install --client oxide|claude|codex|cursor|desktop"
}

main "$@"
