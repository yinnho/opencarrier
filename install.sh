#!/bin/sh
# OpenCarrier Installer
# Usage: curl -sSL https://carrier.yinnho.cn/install.sh | sh

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() { echo "${GREEN}[INFO]${NC} $1"; }
warn() { echo "${YELLOW}[WARN]${NC} $1"; }
error() { echo "${RED}[ERROR]${NC} $1"; exit 1; }

# Detect OS
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
case $OS in
    linux)  OS="linux" ;;
    darwin) OS="darwin" ;;
    *)      error "Unsupported OS: $OS" ;;
esac

# Detect Arch
ARCH=$(uname -m)
case $ARCH in
    x86_64|amd64)   ARCH="x86_64" ;;
    aarch64|arm64)  ARCH="aarch64" ;;
    *)              error "Unsupported architecture: $ARCH" ;;
esac

info "Detected: $OS-$ARCH"

# Binary name
BINARY="yinghe"
ARCHIVE="${BINARY}-${OS}-${ARCH}.tar.gz"

# Download URL
BASE_URL="${OPENCARRIER_URL:-https://carrier.yinnho.cn}"
VERSION="${OPENCARRIER_VERSION:-v0.1.0}"
DOWNLOAD_URL="${BASE_URL}/releases/${VERSION}/${ARCHIVE}"

info "Downloading from: $DOWNLOAD_URL"

# Create temp directory
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

# Download
if command -v curl > /dev/null; then
    curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/$ARCHIVE" || error "Download failed"
elif command -v wget > /dev/null; then
    wget -q "$DOWNLOAD_URL" -O "$TMP_DIR/$ARCHIVE" || error "Download failed"
else
    error "curl or wget required"
fi

# Extract
info "Extracting..."
tar xzf "$TMP_DIR/$ARCHIVE" -C "$TMP_DIR" || error "Extract failed"

# Install directory
INSTALL_DIR="${OPENCARRIER_INSTALL:-/usr/local/bin}"

# Check if we can write to install dir
if [ ! -w "$INSTALL_DIR" ]; then
    if command -v sudo > /dev/null; then
        SUDO="sudo"
    else
        error "Cannot write to $INSTALL_DIR and sudo not available"
    fi
else
    SUDO=""
fi

# Install
info "Installing to $INSTALL_DIR..."
$SUDO mv "$TMP_DIR/$BINARY" "$INSTALL_DIR/$BINARY"
$SUDO chmod +x "$INSTALL_DIR/$BINARY"

# Verify
info "Verifying installation..."
"$INSTALL_DIR/$BINARY" --version || warn "Version check failed, but binary installed"

echo ""
echo "${GREEN}✅ OpenCarrier installed successfully!${NC}"
echo ""
echo "Usage:"
echo "  yinghe serve          # Start serve mode (stdin/stdout)"
echo "  yinghe status         # Show status"
echo "  yinghe bind <code>   # Bind with pairing code"
echo "  yinghe --help        # Show help"
echo ""
