#!/bin/sh
# Remit CLI installer
# Usage: curl -fsSL https://remit.md/install.sh | sh
#
# Environment variables:
#   REMIT_INSTALL_DIR  - Installation directory (default: /usr/local/bin)

set -e

REPO="remit-md/remit-cli"
INSTALL_DIR="${REMIT_INSTALL_DIR:-/usr/local/bin}"

# Detect OS
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  linux)  TARGET_OS="unknown-linux-gnu" ;;
  darwin) TARGET_OS="apple-darwin" ;;
  *)
    echo "Error: Unsupported OS: $OS"
    echo "For Windows, use: scoop install remit-md/scoop-bucket/remit"
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64|amd64)   TARGET_ARCH="x86_64" ;;
  aarch64|arm64)   TARGET_ARCH="aarch64" ;;
  *)
    echo "Error: Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

TARGET="${TARGET_ARCH}-${TARGET_OS}"

# Get latest release version
VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"v\(.*\)".*/\1/')

if [ -z "$VERSION" ]; then
  echo "Error: Failed to determine latest version"
  exit 1
fi

URL="https://github.com/${REPO}/releases/download/v${VERSION}/remit-${TARGET}.tar.gz"

echo "Installing remit v${VERSION} for ${TARGET}..."

# Create install dir if needed
if [ ! -d "$INSTALL_DIR" ]; then
  echo "Creating ${INSTALL_DIR} (may need sudo)..."
  mkdir -p "$INSTALL_DIR" 2>/dev/null || sudo mkdir -p "$INSTALL_DIR"
fi

# Download and extract
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

curl -fsSL "$URL" -o "${TMPDIR}/remit.tar.gz"
tar xzf "${TMPDIR}/remit.tar.gz" -C "$TMPDIR"

# Install binary
if [ -w "$INSTALL_DIR" ]; then
  mv "${TMPDIR}/remit" "${INSTALL_DIR}/remit"
else
  sudo mv "${TMPDIR}/remit" "${INSTALL_DIR}/remit"
fi
chmod +x "${INSTALL_DIR}/remit"

# Write install config
mkdir -p "${HOME}/.remit"
cat > "${HOME}/.remit/config.toml" << EOF
[install]
method = "curl"
installed_at = "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
EOF

echo ""
echo "Installed remit v${VERSION} to ${INSTALL_DIR}/remit"
echo ""
echo "Get started:"
echo "  remit signer init    # Create a wallet"
echo "  remit --help          # See all commands"
