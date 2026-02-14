#!/bin/bash
# build-release.sh
# Optimized release build script for low-latency trading system

set -e

echo "Building with maximum optimizations..."

# Set RUSTFLAGS for native CPU optimizations
export RUSTFLAGS="-C target-cpu=native -C opt-level=3 -C link-arg=-fuse-ld=lld"

# Build release binary
cargo build --release

echo "Build complete!"
echo ""
echo "Running clippy checks..."
cargo clippy --release -- -D warnings

echo ""
echo "Binary location: target/release/arbitrage2"
echo ""
echo "To install flamegraph profiler (first time only):"
echo "  cargo install flamegraph"
echo ""
echo "To profile with flamegraph, run:"
echo "  cargo flamegraph --release --bin arbitrage2"
