//! Gate 1/2/5 (Runtime & Rendering Integration): Play Mode and net
//! screenshots must composite the LIVE GUI (as mutated by scripts in
//! the runtime model) over the 3D world from the same runtime state,
//! without committing anything to the place file.

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") { if !p.is_empty() { return PathBuf::from(p); } }
    let c = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(c.is_file(), "engine at {}", c.display());
    c
}
fn cli_ok(a: &[&str]) -> String { let o = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--engine").arg(engine_path()).args(a).output().unwrap(); assert!(o.status.success(), "{a:?}: {}", String::from_utf8_lossy(&o.stderr)); String::from_utf8(o.stdout).unwrap() }
fn cli_json(a: &[&str]) -> serde_json::Value { let o = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--json").arg("--engine").arg(engine_path()).args(a).output().unwrap(); assert!(o.status.success(), "{a:?}: {}", String::from_utf8_lossy(&o.stderr)); serde_json::from_slice(&o.stdout).unwrap() }

fn add(f: &str, parent: &str, class: &str, name: &str) {
    cli_ok(&["add", f, "--parent", parent, "--class", class, "--name", name]);
}
fn set(f: &str, target: &str, prop: &str, ty: &str, value: &str) {
    cli_ok(&["set", f, "--target", target, "--property", prop, "--type", ty, "--value", value]);
}
fn probe(v: &serde_json::Value, i: usize) -> (i64, i64, i64) {
    let probes = v["result"]["probes"].as_array().unwrap_or_else(|| panic!("probes in result: {v}"));
    let rgba = probes[i]["rgba"].as_array().unwrap();
    (rgba[0].as_i64().unwrap(), rgba[1].as_i64().unwrap(), rgba[2].as_i64().unwrap())
}
fn is_blue(c: (i64, i64, i64)) -> bool { c.2 > 150 && c.0 < 100 && c.1 < 100 }

fn hud_place(dir: &PathBuf) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "Workspace", "Part", "Ground");
    set(&f, "Workspace/Ground", "Size", "Vector3", "80,1,80");
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-0.5,0"]);
    set(&f, "Workspace/Ground", "Anchored", "Bool", "true");
    set(&f, "Workspace/Ground", "Color", "Color3", "0.35,0.35,0.38");
    add(&f, "StarterGui", "ScreenGui", "HUD");
    add(&f, "StarterGui/HUD", "Frame", "Panel");
    set(&f, "StarterGui/HUD/Panel", "Position", "UDim2", "0,40,0,40");
    set(&f, "StarterGui/HUD/Panel", "Size", "UDim2", "0,200,0,120");
    // File state is RED; the live runtime state (below) is BLUE. A
    // screenshot showing blue proves it rendered the runtime model.
    set(&f, "StarterGui/HUD/Panel", "BackgroundColor3", "Color3", "1,0,0");
    f
}

#[test]
fn play_screenshot_composites_live_gui_over_world() {
    let dir = std::env::temp_dir().join(format!("bloxcli-playrender-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = hud_place(&dir);
    add(&f, "ServerScriptService", "Script", "Live");
    set(&f, "ServerScriptService/Live", "Source", "String", r#"
        local panel = game.StarterGui.HUD.Panel
        panel.BackgroundColor3 = Color3.fromRGB(0, 0, 255)
        task.wait(0.2)
        game:GetService("TweenService"):Create(panel, TweenInfo.new(1, Enum.EasingStyle.Linear),
            {Position = UDim2.new(0, 440, 0, 40)}):Play()
        local spawned = Instance.new("Part")
        spawned.Anchored = true
        spawned.Position = Vector3.new(20, 5, 0)
        spawned.Parent = workspace
        print("live ready")
    "#);
    let file_hash_before = std::fs::read(&f).unwrap();

    cli_json(&["play", "start", &f]);
    let shot1 = dir.join("frame1.png");
    let r1 = cli_json(&["play", "screenshot", &f, "--output", shot1.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probes", "140,120;1100,650"]);
    let png1 = std::fs::read(&shot1).unwrap();
    assert!(png1.len() > 1000 && &png1[0..4] == b"\x89PNG", "genuine PNG frame");
    assert!(is_blue(probe(&r1, 0)), "live (blue) panel composited over world: {:?}", probe(&r1, 0));
    let world = probe(&r1, 1);
    assert!(!is_blue(world), "world renders outside the panel: {world:?}");
    assert_eq!(std::fs::read(&f).unwrap(), file_hash_before, "screenshot must not modify the place file");

    // Tween to the goal position on scheduler time, then re-shoot.
    cli_json(&["play", "step", &f, "--steps", "150"]);
    let shot2 = dir.join("frame2.png");
    let r2 = cli_json(&["play", "screenshot", &f, "--output", shot2.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probes", "540,120;140,120"]);
    assert!(is_blue(probe(&r2, 0)), "panel at tween goal: {:?}", probe(&r2, 0));
    assert!(!is_blue(probe(&r2, 1)), "panel left its start position: {:?}", probe(&r2, 1));
    // Live inspectability: GUI bounds + runtime-spawned physics body.
    let state = cli_json(&["play", "state", &f]);
    let rects = state["state"]["gui"]["rects"].as_array().expect("gui rects in play state");
    let panel = rects.iter().find(|r| r["name"].as_str() == Some("Panel")).expect("panel rect");
    assert!((panel["x"].as_f64().unwrap() - 440.0).abs() < 2.0, "panel rect at tween goal: {panel}");
    assert!(state["state"]["bodies"].as_i64().unwrap() >= 2, "runtime-spawned part has a body: {}", state["state"]);
    cli_json(&["play", "stop", &f]);
    assert_eq!(std::fs::read(&f).unwrap(), file_hash_before, "play without commit leaves file intact");
}

#[test]
fn play_click_changes_screenshot_frame() {
    // Gate 3 path in one frame sequence: native click -> LocalScript
    // -> DataModel mutation -> next screenshot shows the new state.
    let dir = std::env::temp_dir().join(format!("bloxcli-playclick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = hud_place(&dir);
    add(&f, "StarterGui/HUD", "TextButton", "Toggle");
    set(&f, "StarterGui/HUD/Toggle", "Position", "UDim2", "0,300,0,40");
    set(&f, "StarterGui/HUD/Toggle", "Size", "UDim2", "0,120,0,50");
    set(&f, "StarterGui/HUD/Toggle", "Text", "String", "TOGGLE");
    set(&f, "StarterGui/HUD/Toggle", "BackgroundColor3", "Color3", "0.2,0.2,0.2");
    add(&f, "StarterGui/HUD", "LocalScript", "Ctl");
    set(&f, "StarterGui/HUD/Ctl", "Source", "String", r#"
        local panel = script.Parent.Panel
        script.Parent.Toggle.MouseButton1Click:Connect(function()
            panel.BackgroundColor3 = Color3.fromRGB(0, 200, 0)
            print("toggled")
        end)
    "#);
    cli_json(&["play", "start", &f]);
    let before = dir.join("before.png");
    let r0 = cli_json(&["play", "screenshot", &f, "--output", before.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probes", "140,120"]);
    assert!(!is_blue(probe(&r0, 0)) && probe(&r0, 0).0 > 150, "panel red before click: {:?}", probe(&r0, 0));
    // Button centre in play layout (36px topbar inset): (360, 40+36+25).
    cli_json(&["play", "input", &f, "--click", "360,101"]);
    let logs = cli_json(&["play", "logs", &f]);
    assert!(serde_json::to_string(&logs).unwrap().contains("toggled"), "click reached LocalScript: {logs}");
    let after = dir.join("after.png");
    let r1 = cli_json(&["play", "screenshot", &f, "--output", after.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probes", "140,120"]);
    let c = probe(&r1, 0);
    assert!(c.1 > 120 && c.0 < 100, "panel green in frame after click: {c:?}");
    cli_json(&["play", "stop", &f]);
}

#[test]
fn net_screenshots_include_each_clients_gui() {
    let dir = std::env::temp_dir().join(format!("bloxcli-netrender-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = hud_place(&dir);
    // Blue in file this time; both clients must show their own GUI.
    set(&f, "StarterGui/HUD/Panel", "BackgroundColor3", "Color3", "0,0,1");
    cli_ok(&["net", "start", &f, "--players", "2", "--latency", "0"]);
    cli_ok(&["net", "step", &f, "--steps", "30"]);
    let mut pngs = Vec::new();
    for player in ["1", "2"] {
        let out = dir.join(format!("client{player}.png"));
        let r = cli_json(&["net", "screenshot", &f, "--player", player, "--output", out.to_str().unwrap(),
            "--width", "1280", "--height", "720", "--probes", "140,120"]);
        assert!(is_blue(probe(&r, 0)), "client {player} GUI composited: {:?}", probe(&r, 0));
        pngs.push(std::fs::read(&out).unwrap());
    }
    assert_ne!(pngs[0], pngs[1], "clients render different viewpoints");
    cli_ok(&["net", "stop", &f]);
}
