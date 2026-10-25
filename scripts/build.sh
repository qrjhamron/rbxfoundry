#!/usr/bin/env bash
# Builds the C++ engine and the Rust CLI.
set -euo pipefail
cd "$(dirname "$0")/.."

cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
cargo build --workspace

echo
echo "Built:"
echo "  engine: build/engine/blox-engine"
echo "  cli:    target/debug/bloxcli  (or target/release/bloxcli with --release)"
