#!/usr/bin/env bash
# Regenerates the deliverable screenshots under screenshots/ using the real
# renderer. Scenes are built entirely through the CLI (no hand-made images).
set -euo pipefail
cd "$(dirname "$0")/.."

BLOX="${1:-target/debug/bloxcli}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p screenshots
cd "$WORK"

# --- Daylight baseplate with a castle (Phase 3A/3B) -------------------------
"$OLDPWD/$BLOX" new castle.rbxl >/dev/null
add_part() {
    "$OLDPWD/$BLOX" add castle.rbxl --parent Workspace --class Part --name "$1" >/dev/null
    "$OLDPWD/$BLOX" set castle.rbxl --target "Workspace/$1" --property Size --value "$2" >/dev/null
    "$OLDPWD/$BLOX" move castle.rbxl --target "Workspace/$1" --position "$3" >/dev/null
    "$OLDPWD/$BLOX" set castle.rbxl --target "Workspace/$1" --property Color --value "$4" >/dev/null
}
add_part Ground 60,2,60 0,-1,0 0.35,0.75,0.35
add_part Keep 10,14,10 0,7,0 0.62,0.6,0.58
add_part TowerL 20,6,6 -11,10,0 0.62,0.6,0.58
add_part TowerR 20,6,6 11,10,0 0.62,0.6,0.58
"$OLDPWD/$BLOX" set castle.rbxl --target Workspace/TowerL --property Shape --type Enum --value 2 >/dev/null
"$OLDPWD/$BLOX" set castle.rbxl --target Workspace/TowerR --property Shape --type Enum --value 2 >/dev/null
# Roblox cylinders lie along X by default; rotate them into standing towers.
"$OLDPWD/$BLOX" rotate castle.rbxl --target Workspace/TowerL --angles "0,0,90" >/dev/null
"$OLDPWD/$BLOX" rotate castle.rbxl --target Workspace/TowerR --angles "0,0,90" >/dev/null
"$OLDPWD/$BLOX" set castle.rbxl --target Lighting --property ClockTime --type Float64 --value 10 >/dev/null
"$OLDPWD/$BLOX" add castle.rbxl --parent Workspace --class WedgePart --name Roof >/dev/null
"$OLDPWD/$BLOX" set castle.rbxl --target Workspace/Roof --property Size --value 12,5,12 >/dev/null
"$OLDPWD/$BLOX" move castle.rbxl --target Workspace/Roof --position 0,16.5,0 >/dev/null
"$OLDPWD/$BLOX" set castle.rbxl --target Workspace/Roof --property Color --value 0.7,0.2,0.15 >/dev/null
"$OLDPWD/$BLOX" render castle.rbxl --output "$OLDPWD/screenshots/castle-day.png" \
    --width 1280 --height 720 --camera 34,22,44 --look-at 0,8,0

# --- Quest HUD at desktop and mobile sizes (Phase 4) ------------------------
"$OLDPWD/$BLOX" new hud.rbxl >/dev/null
"$OLDPWD/$BLOX" add hud.rbxl --parent StarterGui --class ScreenGui --name HUD >/dev/null
"$OLDPWD/$BLOX" add hud.rbxl --parent StarterGui/HUD --class Frame --name Panel >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel --property Position --type UDim2 --value "0.5,0,0.5,0" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel --property Size --type UDim2 --value "0,340,0,190" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel --property AnchorPoint --type Vector2 --value "0.5,0.5" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel --property BackgroundColor3 --type Color3 --value "0.13,0.15,0.22" >/dev/null
"$OLDPWD/$BLOX" add hud.rbxl --parent StarterGui/HUD/Panel --class UICorner --name Round >/dev/null
"$OLDPWD/$BLOX" add hud.rbxl --parent StarterGui/HUD/Panel --class TextLabel --name Title >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Title --property Size --type UDim2 --value "1,0,0,44" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Title --property Text --type String --value "Quest: Slay the Dragon" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Title --property TextColor3 --type Color3 --value "1,0.9,0.4" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Title --property TextSize --type Float64 --value 24 >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Title --property BackgroundTransparency --type Float64 --value 1 >/dev/null
"$OLDPWD/$BLOX" add hud.rbxl --parent StarterGui/HUD/Panel --class TextButton --name Accept >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Accept --property Position --type UDim2 --value "0.5,0,1,-62" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Accept --property Size --type UDim2 --value "0,150,0,46" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Accept --property AnchorPoint --type Vector2 --value "0.5,0" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Accept --property BackgroundColor3 --type Color3 --value "0.2,0.75,0.3" >/dev/null
"$OLDPWD/$BLOX" set hud.rbxl --target StarterGui/HUD/Panel/Accept --property Text --type String --value "Accept" >/dev/null
"$OLDPWD/$BLOX" ui render hud.rbxl --output "$OLDPWD/screenshots/hud-desktop.png" --device desktop
"$OLDPWD/$BLOX" ui render hud.rbxl --output "$OLDPWD/screenshots/hud-mobile.png" --device mobile

echo "Screenshots written to screenshots/"
