#!/bin/bash
# Memory usage test for OpenCarrier
#
# This script measures memory usage of the serve mode

set -e

echo "=== OpenCarrier Memory Usage Test ==="

OPENCARRIER=${OPENCARRIER:-target/release/opencarrier}

if [ ! -f "$OPENCARRIER" ]; then
    echo "Building release binary..."
    cargo build --release -p opencarrier-cli
fi

# Start opencarrier serve in background, keep it running with a pipe
mkfifo /tmp/opencarrier_input_$$ 2>/dev/null || true
"$OPENCARRIER" serve < /tmp/opencarrier_input_$$ &
OC_PID=$!

# Open the pipe for writing to keep it alive
exec 3>/tmp/opencarrier_input_$$

# Wait for startup
sleep 3

if kill -0 $OC_PID 2>/dev/null; then
    # Get memory usage in KB (RSS - Resident Set Size)
    MEM_KB=$(ps -o rss= -p $OC_PID 2>/dev/null || echo "0")
    MEM_MB=$((MEM_KB / 1024))

    # Get virtual memory in KB
    VMEM_KB=$(ps -o vsz= -p $OC_PID 2>/dev/null || echo "0")
    VMEM_MB=$((VMEM_KB / 1024))

    echo "Process ID: $OC_PID"
    echo "RSS Memory: ${MEM_MB}MB (${MEM_KB}KB)"
    echo "Virtual Memory: ${VMEM_MB}MB (${VMEM_KB}KB)"
    echo ""

    # Cleanup
    exec 3>&-
    kill $OC_PID 2>/dev/null || true
    wait $OC_PID 2>/dev/null || true
    rm -f /tmp/opencarrier_input_$$

    if [ $MEM_MB -lt 50 ]; then
        echo "✅ Memory usage OK (< 50MB)"
        exit 0
    else
        echo "⚠️  Memory usage high (>= 50MB)"
        echo ""
        echo "Note: Memory usage may vary based on loaded skills and models."
        echo "The kernel loads 60 bundled skills on startup."
        exit 1
    fi
else
    echo "❌ Failed to start opencarrier serve"
    rm -f /tmp/opencarrier_input_$$
    exit 1
fi
