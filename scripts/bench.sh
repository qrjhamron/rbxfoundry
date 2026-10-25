#!/usr/bin/env bash
# Micro-benchmarks on a moderately complex generated place.
# Prints wall-clock timings; results are machine-specific (software
# Vulkan here). Usage: scripts/bench.sh [path-to-bloxcli]
set -euo pipefail
cd "$(dirname "$0")/.."

BLOX="${1:-./target/debug/bloxcli}"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
F="${WORK}/bench.rbxl"

"${BLOX}" new "${F}" --template baseplate >/dev/null
# Scatter 200 parts to approximate a moderately complex scene.
"${BLOX}" run "${F}" --duration 0 --commit --eval "
for i = 1, 200 do
    local p = Instance.new('Part')
    p.Size = Vector3.new(4, 4, 4)
    p.Position = Vector3.new((i % 14) * 8 - 52, 6 + (i % 5) * 5, math.floor(i / 14) * 8 - 52)
    p.Anchored = i % 3 ~= 0
    p.Parent = workspace
end
print('built 200 parts')" >/dev/null

timeit() {
    local label="$1"; shift
    local start end
    start="$(date +%s.%N)"
    "$@" >/dev/null
    end="$(date +%s.%N)"
    printf "%-28s %7.2f s\n" "${label}" "$(echo "${end} - ${start}" | bc)"
}

echo "== RBXFoundry benchmarks (${F}) =="
timeit "render 800x600 (200 parts)" "${BLOX}" render "${F}" --output "${WORK}/s.png" --width 800 --height 600 --camera "60,40,60" --look-at "0,5,0"
timeit "ui render 1920x1080" "${BLOX}" ui render "${F}" --output "${WORK}/u.png" --width 1920 --height 1080
timeit "export rbxl -> rbxlx" "${BLOX}" export "${F}" "${WORK}/bench.rbxlx"
timeit "validate" "${BLOX}" validate "${F}"
timeit "tree (213 instances)" "${BLOX}" tree "${F}"
"${BLOX}" play start "${F}" >/dev/null
timeit "play step x240 (physics)" "${BLOX}" play step "${F}" --steps 240
"${BLOX}" play stop "${F}" >/dev/null
echo "done (host: $(uname -srm))"
