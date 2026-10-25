#!/usr/bin/env bash
# Builds the example project (castle scene + HUD + a Luau gameplay
# script) with the RBXFoundry CLI and renders a real screenshot of it.
# Usage: examples/make_examples.sh [path-to-bloxcli]
set -euo pipefail

BLOX="$(readlink -f "${1:-$(dirname "$0")/../target/debug/bloxcli}")"
cd "$(dirname "$0")"
OUT="castle.rbxl"

rm -f "${OUT}" castle.png ui.png
"${BLOX}" new "${OUT}" --force >/dev/null

# A small castle on a baseplate.
"${BLOX}" add "${OUT}" --parent Workspace --class Part --name Baseplate >/dev/null
"${BLOX}" set "${OUT}" --target Workspace/Baseplate --property Size --type Vector3 --value "64,2,64" >/dev/null
"${BLOX}" move "${OUT}" --target Workspace/Baseplate --position "0,-1,0" >/dev/null
"${BLOX}" set "${OUT}" --target Workspace/Baseplate --property Anchored --type Bool --value true >/dev/null
for tower in "TowerL -14,6,-14" "TowerR 14,6,-14"; do
    set -- ${tower}
    "${BLOX}" add "${OUT}" --parent Workspace --class Part --name "$1" >/dev/null
    "${BLOX}" set "${OUT}" --target "Workspace/$1" --property Size --type Vector3 --value "6,14,6" >/dev/null
    "${BLOX}" move "${OUT}" --target "Workspace/$1" --position "$2" >/dev/null
    "${BLOX}" set "${OUT}" --target "Workspace/$1" --property Anchored --type Bool --value true >/dev/null
done
"${BLOX}" add "${OUT}" --parent Workspace --class SpawnLocation --name Spawn >/dev/null
"${BLOX}" move "${OUT}" --target Workspace/Spawn --position "0,0.5,12" >/dev/null

# A HUD built from genuine GUI instances (incl. RichText).
"${BLOX}" add "${OUT}" --parent StarterGui --class ScreenGui --name HUD >/dev/null
"${BLOX}" add "${OUT}" --parent StarterGui/HUD --class TextLabel --name Title >/dev/null
"${BLOX}" set "${OUT}" --target StarterGui/HUD/Title --property Size --type UDim2 --value "1,0,0,60" >/dev/null
"${BLOX}" set "${OUT}" --target StarterGui/HUD/Title --property TextSize --type Float64 --value 28 >/dev/null
"${BLOX}" set "${OUT}" --target StarterGui/HUD/Title --property RichText --type Bool --value true >/dev/null
"${BLOX}" set "${OUT}" --target StarterGui/HUD/Title --property Text --type String --value 'Castle <b>Defense</b> — wave <font color="#FF4040">3</font>' >/dev/null

# Luau gameplay: a day/night cycle driven by the real runtime.
"${BLOX}" add "${OUT}" --parent ServerScriptService --class Script --name DayCycle >/dev/null
"${BLOX}" set "${OUT}" --target ServerScriptService/DayCycle --property Source --type String --value "local lighting = game:GetService('Lighting')
while true do
    lighting.ClockTime = lighting.ClockTime + 0.5
    task.wait(0.5)
end" >/dev/null

"${BLOX}" render "${OUT}" --output castle.png --width 960 --height 600 --camera "34,24,38" --look-at "0,4,0" >/dev/null
"${BLOX}" ui render "${OUT}" --ignore-inset --output ui.png --width 960 --height 600 >/dev/null
"${BLOX}" validate "${OUT}" >/dev/null
echo "example built: examples/${OUT} (+ castle.png, ui.png)"
