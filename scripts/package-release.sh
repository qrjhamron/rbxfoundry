#!/usr/bin/env bash
# Builds optimized binaries and assembles a self-contained Linux
# release tarball in dist/:
#   rbxfoundry-<version>-linux-x86_64/
#     bin/bloxcli, bin/blox   (CLI; finds the engine next to itself)
#     bin/blox-engine         (C++ engine)
#     bin/fonts/              (DejaVu Sans + Bold, OFL-licensed)
#     examples/               (example project builder + README)
#     README.md COMPATIBILITY.md LICENSE
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(grep -m1 '^version = ' crates/bloxcli/Cargo.toml | sed 's/.*\"\(.*\)\".*/\1/')"
NAME="rbxfoundry-${VERSION}-linux-x86_64"
DIST="dist/${NAME}"

echo "== Configuring + building C++ engine (Release) =="
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build

echo "== Building Rust CLI (release) =="
cargo build --release --workspace

echo "== Assembling ${DIST} =="
rm -rf "${DIST}"
mkdir -p "${DIST}/bin/fonts" "${DIST}/examples"

cp target/release/bloxcli "${DIST}/bin/bloxcli"
cp build/engine/blox-engine "${DIST}/bin/blox-engine"
ln -sf bloxcli "${DIST}/bin/blox"
strip "${DIST}/bin/bloxcli" "${DIST}/bin/blox-engine" || true

# Bundle the fonts the engine looks for in <bin>/fonts (see
# find_default_font). DejaVu is OFL-licensed and redistributable.
for font in DejaVuSans.ttf DejaVuSans-Bold.ttf; do
    for dir in /usr/share/fonts/truetype/dejavu /usr/share/fonts/dejavu; do
        if [ -f "${dir}/${font}" ]; then cp "${dir}/${font}" "${DIST}/bin/fonts/"; break; fi
    done
done
if [ ! -f "${DIST}/bin/fonts/DejaVuSans.ttf" ]; then
    echo "warning: DejaVuSans.ttf not found on this system; tarball relies on target fonts or BLOX_FONT" >&2
fi

cp examples/README.md examples/make_examples.sh "${DIST}/examples/"
cp README.md COMPATIBILITY.md LICENSE "${DIST}/"

tar -czf "dist/${NAME}.tar.gz" -C dist "${NAME}"
echo
echo "Release tarball: dist/${NAME}.tar.gz"
ls -la "dist/${NAME}.tar.gz"
