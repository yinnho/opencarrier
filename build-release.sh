#!/bin/bash
# Build release binaries for multiple platforms
#
# Usage: ./build-release.sh [version]
# Example: ./build-release.sh v0.1.0

set -e

VERSION=${1:-v0.1.0}
OUTPUT_DIR="dist"
BINARY_NAME="opencarrier"

echo "=== Building OpenCarrier Release ${VERSION} ==="
echo ""

# Clean output directory
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

# Build for current platform first (fast)
echo "--- Building for current platform ---"
cargo build --release -p opencarrier-cli
cp target/release/$BINARY_NAME "$OUTPUT_DIR/${BINARY_NAME}-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)"
echo "✅ Current platform build complete"
echo ""

# Cross-compilation targets (requires cross-rs or cargo cross)
# Uncomment if you have cross-compilation set up

# Linux x86_64
# echo "--- Building for Linux x86_64 ---"
# cross build --release --target x86_64-unknown-linux-gnu -p opencarrier-cli
# cp target/x86_64-unknown-linux-gnu/release/$BINARY_NAME "$OUTPUT_DIR/${BINARY_NAME}-linux-x86_64"
# tar czf "$OUTPUT_DIR/${BINARY_NAME}-linux-x86_64.tar.gz" -C "$OUTPUT_DIR" "${BINARY_NAME}-linux-x86_64"
# rm "$OUTPUT_DIR/${BINARY_NAME}-linux-x86_64"
# echo "✅ Linux x86_64 complete"

# Linux aarch64
# echo "--- Building for Linux aarch64 ---"
# cross build --release --target aarch64-unknown-linux-gnu -p opencarrier-cli
# cp target/aarch64-unknown-linux-gnu/release/$BINARY_NAME "$OUTPUT_DIR/${BINARY_NAME}-linux-aarch64"
# tar czf "$OUTPUT_DIR/${BINARY_NAME}-linux-aarch64.tar.gz" -C "$OUTPUT_DIR" "${BINARY_NAME}-linux-aarch64"
# rm "$OUTPUT_DIR/${BINARY_NAME}-linux-aarch64"
# echo "✅ Linux aarch64 complete"

# macOS universal (requires macos)
if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "--- Creating macOS tarball ---"
    tar czf "$OUTPUT_DIR/${BINARY_NAME}-darwin-$(uname -m).tar.gz" -C target/release "$BINARY_NAME"
    echo "✅ macOS tarball complete"
fi

# Generate checksums
echo ""
echo "--- Generating checksums ---"
cd "$OUTPUT_DIR"
if command -v sha256sum &> /dev/null; then
    sha256sum *.tar.gz > checksums.sha256
else
    shasum -a 256 *.tar.gz > checksums.sha256
fi
cd ..
echo "✅ Checksums generated"

# Show results
echo ""
echo "=== Build Complete ==="
echo "Output directory: $OUTPUT_DIR/"
ls -lh "$OUTPUT_DIR/"
echo ""
echo "Files ready for upload to: https://carrier.yinnho.cn/releases/${VERSION}/"
