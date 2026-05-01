#!/usr/bin/env bash
# OpenCarrier installer — works on Linux, macOS, WSL
# Usage: curl -sSf https://opencarrier.sh | sh
#
# Environment variables:
#   OPENCARRIER_INSTALL_DIR  — custom install directory (default: ~/.opencarrier/bin)
#   OPENCARRIER_VERSION      — install a specific version tag (default: latest)

set -euo pipefail

REPO="yinnho/opencarrier"
INSTALL_DIR="${OPENCARRIER_INSTALL_DIR:-$HOME/.opencarrier/bin}"

detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)
    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) echo "  Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    case "$OS" in
        linux) PLATFORM="${ARCH}-unknown-linux-gnu" ;;
        darwin) PLATFORM="${ARCH}-apple-darwin" ;;
        mingw*|msys*|cygwin*)
            echo ""
            echo "  For Windows, use PowerShell instead:"
            echo "    irm https://opencarrier.sh/install.ps1 | iex"
            echo ""
            echo "  Or download the .msi installer from:"
            echo "    https://github.com/$REPO/releases/latest"
            echo ""
            echo "  Or install via cargo:"
            echo "    cargo install --git https://github.com/$REPO opencarrier-cli"
            exit 1
            ;;
        *) echo "  Unsupported OS: $OS"; exit 1 ;;
    esac
}

install() {
    detect_platform

    echo ""
    echo "  OpenCarrier Installer"
    echo "  =================="
    echo ""

    # Get latest version
    if [ -n "${OPENCARRIER_VERSION:-}" ]; then
        VERSION="$OPENCARRIER_VERSION"
        echo "  Using specified version: $VERSION"
    else
        echo "  Fetching latest release..."
        VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | head -1 | cut -d '"' -f 4)
    fi

    if [ -z "$VERSION" ]; then
        echo "  Could not determine latest version."
        echo "  Install from source instead:"
        echo "    cargo install --git https://github.com/$REPO opencarrier-cli"
        exit 1
    fi

    URL="https://github.com/$REPO/releases/download/$VERSION/opencarrier-$PLATFORM.tar.gz"
    CHECKSUM_URL="$URL.sha256"
    HUB_DL_URL="https://hub.aginx.net/api/releases/$VERSION/download/opencarrier-$PLATFORM.tar.gz"
    HUB_CHECKSUM_URL="https://hub.aginx.net/api/releases/$VERSION/download/opencarrier-$PLATFORM.tar.gz.sha256"

    echo "  Installing OpenCarrier $VERSION for $PLATFORM..."
    mkdir -p "$INSTALL_DIR"

    # Download to temp
    TMPDIR=$(mktemp -d)
    ARCHIVE="$TMPDIR/opencarrier.tar.gz"
    CHECKSUM_FILE="$TMPDIR/checksum.sha256"

    cleanup() { rm -rf "$TMPDIR"; }
    trap cleanup EXIT

    # Try GitHub first, then Hub mirror (for China / no-GitHub access)
    DL_FROM=""
    if curl -fsSL --connect-timeout 5 "$URL" -o "$ARCHIVE" 2>/dev/null; then
        DL_FROM="github"
    elif curl -fsSL --connect-timeout 10 "$HUB_DL_URL" -o "$ARCHIVE" 2>/dev/null; then
        DL_FROM="hub"
        CHECKSUM_URL="$HUB_CHECKSUM_URL"
        echo "  Downloaded from Hub mirror (hub.aginx.net)."
    else
        echo "  Download failed from both GitHub and Hub."
        echo "  Install from source instead:"
        echo "    cargo install --git https://github.com/$REPO opencarrier-cli"
        exit 1
    fi

    # Verify checksum if available
    if curl -fsSL "$CHECKSUM_URL" -o "$CHECKSUM_FILE" 2>/dev/null; then
        EXPECTED=$(cut -d ' ' -f 1 < "$CHECKSUM_FILE")
        if command -v sha256sum &>/dev/null; then
            ACTUAL=$(sha256sum "$ARCHIVE" | cut -d ' ' -f 1)
        elif command -v shasum &>/dev/null; then
            ACTUAL=$(shasum -a 256 "$ARCHIVE" | cut -d ' ' -f 1)
        else
            ACTUAL=""
        fi
        if [ -n "$ACTUAL" ]; then
            if [ "$EXPECTED" != "$ACTUAL" ]; then
                echo "  Checksum verification FAILED!"
                echo "    Expected: $EXPECTED"
                echo "    Got:      $ACTUAL"
                exit 1
            fi
            echo "  Checksum verified."
        else
            echo "  No sha256sum/shasum found, skipping checksum verification."
        fi
    fi

    # Extract
    tar xzf "$ARCHIVE" -C "$INSTALL_DIR"
    chmod +x "$INSTALL_DIR/opencarrier"

    # Ad-hoc codesign on macOS (prevents SIGKILL on Apple Silicon)
    # Must strip extended attributes (com.apple.quarantine) BEFORE signing,
    # otherwise the signature is computed over the quarantine xattr and macOS
    # rejects it as "Code Signature Invalid" → SIGKILL.
    if [ "$OS" = "darwin" ]; then
        if command -v xattr &>/dev/null; then
            xattr -cr "$INSTALL_DIR/opencarrier" 2>/dev/null || true
        fi
        if command -v codesign &>/dev/null; then
            if ! codesign --force --sign - "$INSTALL_DIR/opencarrier"; then
                echo ""
                echo "  Warning: ad-hoc code signing failed."
                echo "  On Apple Silicon, the binary may be killed (SIGKILL) by Gatekeeper."
                echo "  Try manually: xattr -cr $INSTALL_DIR/opencarrier && codesign --force --sign - $INSTALL_DIR/opencarrier"
                echo ""
            fi
        fi
    fi

    # Add to PATH — detect the user's login shell
    USER_SHELL="${SHELL:-}"
    # Fallback: check /etc/passwd if $SHELL is unset (e.g. minimal containers)
    if [ -z "$USER_SHELL" ] && command -v getent &>/dev/null; then
        USER_SHELL=$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)
    fi
    if [ -z "$USER_SHELL" ] && [ -f /etc/passwd ]; then
        USER_SHELL=$(grep "^$(id -un):" /etc/passwd 2>/dev/null | cut -d: -f7)
    fi

    SHELL_RC=""
    case "$USER_SHELL" in
        */zsh)  SHELL_RC="$HOME/.zshrc" ;;
        */bash) SHELL_RC="$HOME/.bashrc" ;;
        */fish) SHELL_RC="$HOME/.config/fish/config.fish" ;;
    esac
    # Also check for config files if shell detection failed.
    # Check bash/zsh first (more common defaults), fish last — avoids
    # writing to config.fish for users who merely have Fish installed.
    if [ -z "$SHELL_RC" ]; then
        if [ -f "$HOME/.bashrc" ]; then
            SHELL_RC="$HOME/.bashrc"
        elif [ -f "$HOME/.zshrc" ]; then
            SHELL_RC="$HOME/.zshrc"
        elif [ -f "$HOME/.config/fish/config.fish" ]; then
            SHELL_RC="$HOME/.config/fish/config.fish"
        fi
    fi

    if [ -n "$SHELL_RC" ] && ! grep -q "opencarrier" "$SHELL_RC" 2>/dev/null; then
        # Determine syntax from the TARGET FILE, not $USER_SHELL — this
        # prevents Bash syntax from ever being written to config.fish even
        # when shell detection mis-identifies the user's shell.
        case "$SHELL_RC" in
            */config.fish)
                mkdir -p "$(dirname "$SHELL_RC")"
                echo "fish_add_path \"$INSTALL_DIR\"" >> "$SHELL_RC"
                ;;
            *)
                echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$SHELL_RC"
                ;;
        esac
        echo "  Added $INSTALL_DIR to PATH in $SHELL_RC"
    fi

    # Verify installation
    if "$INSTALL_DIR/opencarrier" --version >/dev/null 2>&1; then
        INSTALLED_VERSION=$("$INSTALL_DIR/opencarrier" --version 2>/dev/null || echo "$VERSION")
        echo ""
        echo "  OpenCarrier installed successfully! ($INSTALLED_VERSION)"
    else
        echo ""
        echo "  OpenCarrier binary installed to $INSTALL_DIR/opencarrier"
    fi

    echo ""
    echo "  Get started:"
    echo "    opencarrier init"
    echo ""
    echo "  The setup wizard will guide you through provider selection"
    echo "  and configuration."
    echo ""
}

install
