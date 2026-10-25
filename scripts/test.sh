#!/usr/bin/env bash
# Builds (if needed) and runs every test suite: C++ engine tests via CTest
# and the Rust round-trip/CLI integration tests via Cargo.
set -euo pipefail
cd "$(dirname "$0")/.."

cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build

ctest --test-dir build --output-on-failure
cargo test --workspace

echo
echo "All test suites passed."
