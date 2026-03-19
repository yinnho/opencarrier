#!/bin/bash
# 本地测试脚本 - 不依赖 GitHub Actions

set -e

echo "=== Running cargo fmt check ==="
cargo fmt --check

echo "=== Running cargo clippy ==="
cargo clippy --workspace -- -D warnings

echo "=== Running cargo check ==="
cargo check --workspace

echo "=== Running tests (本地只运行核心测试) ==="
# 只运行核心 crate 的测试，跳过耗时的集成测试
cargo test -p opencarrier-runtime -p opencarrier-types -p opencarrier-memory

echo "=== All checks passed! ==="
