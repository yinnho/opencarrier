#!/bin/bash
# Performance tests for OpenCarrier (Phase 6)
#
# Requirements:
# - 启动时间 < 100ms
# - 内存占用 < 50MB
#
# Usage: ./tests/performance_test.sh

set -e

echo "=== OpenCarrier Performance Tests ==="
echo ""

# Check if opencarrier binary exists
if ! command -v opencarrier &> /dev/null && [ ! -f target/release/opencarrier ]; then
    echo "Building release binary..."
    cargo build --release -p opencarrier-cli
fi

OPENCARRIER=${OPENCARRIER:-target/release/opencarrier}

# 1. Binary Size Test
echo "--- Binary Size ---"
BINARY_SIZE=$(stat -f%z "$OPENCARRIER" 2>/dev/null || stat --printf="%s" "$OPENCARRIER" 2>/dev/null)
BINARY_SIZE_MB=$((BINARY_SIZE / 1024 / 1024))
echo "Binary size: ${BINARY_SIZE_MB}MB"
if [ $BINARY_SIZE_MB -lt 20 ]; then
    echo "✅ Binary size OK (< 20MB)"
else
    echo "⚠️  Binary size large (${BINARY_SIZE_MB}MB)"
fi
echo ""

# 2. Startup Time Test
echo "--- Startup Time ---"
STARTUP_TIMES=()
for i in {1..5}; do
    START=$(date +%s%N)
    timeout 2 "$OPENCARRIER" --version > /dev/null 2>&1 || true
    END=$(date +%s%N)
    ELAPSED_MS=$(( (END - START) / 1000000 ))
    STARTUP_TIMES+=($ELAPSED_MS)
    echo "Run $i: ${ELAPSED_MS}ms"
done

# Calculate average
SUM=0
for t in "${STARTUP_TIMES[@]}"; do
    SUM=$((SUM + t))
done
AVG=$((SUM / 5))
echo "Average startup time: ${AVG}ms"

if [ $AVG -lt 100 ]; then
    echo "✅ Startup time OK (< 100ms)"
else
    echo "⚠️  Startup time slow (>= 100ms)"
fi
echo ""

# 3. Memory Usage Test (basic check with ps)
echo "--- Memory Usage ---"
# Start opencarrier in background with serve mode
# Note: This requires a valid config, so we skip if it fails
"$OPENCARRIER" serve &
OC_PID=$!
sleep 2

if kill -0 $OC_PID 2>/dev/null; then
    # Get memory usage in KB
    MEM_KB=$(ps -o rss= -p $OC_PID 2>/dev/null || echo "0")
    MEM_MB=$((MEM_KB / 1024))
    echo "Memory usage: ${MEM_MB}MB"

    # Kill the process
    kill $OC_PID 2>/dev/null || true
    wait $OC_PID 2>/dev/null || true

    if [ $MEM_MB -lt 50 ]; then
        echo "✅ Memory usage OK (< 50MB)"
    else
        echo "⚠️  Memory usage high (>= 50MB)"
    fi
else
    echo "⚠️  Could not start opencarrier serve for memory test"
fi
echo ""

# 4. Test Execution Time
echo "--- Test Execution ---"
echo "Running unit tests..."
TEST_START=$(date +%s%N)
cargo test --workspace --quiet 2>&1 | tail -1
TEST_END=$(date +%s%N)
TEST_TIME=$(( (TEST_END - TEST_START) / 1000000000 ))
echo "Test suite completed in ${TEST_TIME}s"
echo ""

# Summary
echo "=== Summary ==="
echo "Binary size: ${BINARY_SIZE_MB}MB"
echo "Avg startup: ${AVG}ms"
echo ""
echo "Performance targets:"
echo "  - Startup < 100ms: $([ $AVG -lt 100 ] && echo '✅ PASS' || echo '⚠️  NEEDS OPTIMIZATION')"
echo "  - Memory < 50MB: (requires running serve mode)"
