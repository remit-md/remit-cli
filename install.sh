#!/bin/sh
# Remit CLI installer — https://remit.md
# Usage: curl -fsSL https://raw.githubusercontent.com/remit-md/remit-cli/main/install.sh | sh
#
# Environment variables:
#   REMIT_INSTALL_DIR  - Installation directory (default: ~/.remit/bin)

set -eu

REPO="remit-md/remit-cli"
INSTALL_DIR="${REMIT_INSTALL_DIR:-$HOME/.remit/bin}"

main() {
    need_cmd curl
    need_cmd tar

    os="$(detect_os)"
    arch="$(detect_arch)"
    target="$(resolve_target "$os" "$arch")"

    if [ -z "$target" ]; then
        err "unsupported platform: ${os}/${arch}"
    fi

    version="$(get_latest_version)"
    if [ -z "$version" ]; then
        err "failed to fetch latest release version from GitHub"
    fi

    say "Installing remit ${version} (${target})"

    url="https://github.com/${REPO}/releases/download/${version}/remit-${target}.tar.gz"
    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    say "Downloading ${url}"
    curl -fsSL "$url" -o "${tmpdir}/remit.tar.gz"

    say "Extracting..."
    tar xzf "${tmpdir}/remit.tar.gz" -C "$tmpdir"

    mkdir -p "$INSTALL_DIR"
    mv "${tmpdir}/remit" "${INSTALL_DIR}/remit"
    chmod +x "${INSTALL_DIR}/remit"

    say "Installed to ${INSTALL_DIR}/remit"

    # Verify
    if "${INSTALL_DIR}/remit" --version > /dev/null 2>&1; then
        installed_version="$("${INSTALL_DIR}/remit" --version 2>&1)"
        say "Verified: ${installed_version}"
    else
        warn "binary installed but --version check failed"
    fi

    # PATH guidance
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        say ""
        say "Add remit to your PATH by adding this to your shell profile:"
        say ""
        shell_name="$(basename "${SHELL:-/bin/sh}")"
        case "$shell_name" in
            zsh)  profile="~/.zshrc" ;;
            bash) profile="~/.bashrc" ;;
            fish) profile="~/.config/fish/config.fish" ;;
            *)    profile="~/.profile" ;;
        esac
        if [ "$shell_name" = "fish" ]; then
            say "  fish_add_path ${INSTALL_DIR}"
        else
            say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        fi
        say ""
        say "Then restart your shell or run:"
        say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    fi

    say ""
    say "Done! Run 'remit --help' to get started."
}

detect_os() {
    uname_s="$(uname -s)"
    case "$uname_s" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        MINGW*|MSYS*|CYGWIN*)
            err "Windows detected. Use Scoop instead: scoop bucket add remit https://github.com/remit-md/scoop-bucket && scoop install remit"
            ;;
        *)
            err "unsupported OS: ${uname_s}"
            ;;
    esac
}

detect_arch() {
    uname_m="$(uname -m)"
    case "$uname_m" in
        x86_64|amd64)    echo "x86_64" ;;
        aarch64|arm64)   echo "aarch64" ;;
        *)
            err "unsupported architecture: ${uname_m}"
            ;;
    esac
}

resolve_target() {
    _os="$1"
    _arch="$2"
    case "${_os}-${_arch}" in
        linux-x86_64)   echo "x86_64-unknown-linux-musl" ;;
        linux-aarch64)  echo "aarch64-unknown-linux-musl" ;;
        macos-x86_64)   echo "x86_64-apple-darwin" ;;
        macos-aarch64)  echo "aarch64-apple-darwin" ;;
        *)              echo "" ;;
    esac
}

get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
        | grep '"tag_name"' \
        | head -1 \
        | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
}

say() {
    printf 'remit-install: %s\n' "$*"
}

warn() {
    say "WARNING: $*" >&2
}

err() {
    say "ERROR: $*" >&2
    exit 1
}

need_cmd() {
    if ! command -v "$1" > /dev/null 2>&1; then
        err "need '$1' (command not found)"
    fi
}

main "$@"
