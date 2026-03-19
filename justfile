# Justfile - 简化本地开发命令

# 默认运行所有检查
default: fmt-check clippy check test

# 格式化检查
fmt-check:
    cargo fmt --check

# 格式化代码
fmt:
    cargo fmt

# Clippy 检查
clippy:
    cargo clippy --workspace -- -D warnings

# 编译检查
check:
    cargo check --workspace

# 运行核心测试（快速）
test:
    cargo test -p opencarrier-runtime -p opencarrier-types -p opencarrier-memory

# 运行所有测试（较慢）
test-all:
    cargo test --workspace

# 清理构建产物
clean:
    cargo clean

# 本地完整 CI 模拟
ci: fmt-check clippy check test
    @echo "✅ All CI checks passed locally!"
