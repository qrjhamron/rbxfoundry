//! Gate B fixture acceptance: animated inventory GUI driven by a real
//! LocalScript through the actual play input path (click -> signal ->
//! TweenService -> stepped screenshots), plus desktop/mobile renders
//! and RBXL export/reimport integrity for the same fixture.

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
fn temp_dir(tag: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("bloxcli-gatebfx-{}-{tag}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }
fn add(f: &str, parent: &str, class: &str, name: &str) { cli_ok(&["add", f, "--parent", parent, "--class", class, "--name", name]); }
fn set(f: &str, target: &str, prop: &str, ty: &str, value: &str) { cli_ok(&["set", f, "--target", target, "--property", prop, "--type", ty, "--value", value]); }

#[test]
fn animated_inventory_fixture_end_to_end() {
    let dir = temp_dir("inventory");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "StarterGui", "ScreenGui", "HUD");
    // Open button (top-left) and an offscreen inventory panel.
    add(&f, "StarterGui/HUD", "TextButton", "OpenButton");
    set(&f, "StarterGui/HUD/OpenButton", "Size", "UDim2", "0,120,0,44");
    set(&f, "StarterGui/HUD/OpenButton", "Position", "UDim2", "0,16,0,16");
    set(&f, "StarterGui/HUD/OpenButton", "Text", "String", "Inventory");
    set(&f, "StarterGui/HUD/OpenButton", "BackgroundColor3", "Color3", "0.15,0.35,0.85");
    add(&f, "StarterGui/HUD", "Frame", "Inventory");
    set(&f, "StarterGui/HUD/Inventory", "Size", "UDim2", "0,320,0,240");
    set(&f, "StarterGui/HUD/Inventory", "Position", "UDim2", "0,-340,0,80");
    set(&f, "StarterGui/HUD/Inventory", "BackgroundColor3", "Color3", "0.85,0.2,0.2");
    set(&f, "StarterGui/HUD/Inventory", "Visible", "Bool", "false");
    add(&f, "StarterGui/HUD/Inventory", "TextLabel", "Title");
    set(&f, "StarterGui/HUD/Inventory/Title", "Text", "String", "Items");
    set(&f, "StarterGui/HUD/Inventory/Title", "Size", "UDim2", "1,0,0,36");
    add(&f, "StarterGui", "LocalScript", "UIHandler");
    set(&f, "StarterGui/UIHandler", "Source", "String", r#"
        local gui = script.Parent.HUD
        local panel = gui.Inventory
        local open = false
        gui.OpenButton.MouseButton1Click:Connect(function()
            open = not open
            panel.Visible = true
            local goal = open and UDim2.new(0, 16, 0, 80) or UDim2.new(0, -340, 0, 80)
            local tween = game:GetService("TweenService"):Create(panel,
                TweenInfo.new(0.6, Enum.EasingStyle.Quad, Enum.EasingDirection.Out),
                {Position = goal})
            tween.Completed:Connect(function(state)
                print("PANEL STATE", state, math.floor(panel.Position.X.Offset + 0.5))
            end)
            tween:Play()
        end)
    "#);

    // Static renders at desktop and mobile sizes are genuine PNGs and
    // validate cleanly.
    for (tag, w, h) in [("desktop", "1920", "1080"), ("mobile", "390", "844")] {
        let out = dir.join(format!("{tag}.png"));
        let r = cli_json(&["ui", "render", &f, "--ignore-inset", "--output", out.to_str().unwrap(), "--width", w, "--height", h, "--probe", "400,200"]);
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.len() > 200 && &bytes[0..4] == b"\x89PNG", "{tag} png");
        let _ = r;
    }
    let v = cli_json(&["ui", "validate", &f]);
    assert_eq!(v["valid"], true, "fixture validates: {v}");

    // Play: real input path. Screenshot before, click the button,
    // step in fixed increments, screenshot after the tween.
    cli_json(&["play", "start", &f]);
    let shot_a = dir.join("play-a.png");
    cli_json(&["play", "screenshot", &f, "--output", shot_a.to_str().unwrap(), "--width", "1280", "--height", "720"]);
    let click = cli_json(&["play", "input", &f, "--click", "76,74", "--width", "1280", "--height", "720"]);
    assert_eq!(click["hit"]["path"], "StarterGui/HUD/OpenButton", "native hit-test found the button: {click}");
    cli_json(&["play", "step", &f, "--steps", "30"]);
    let shot_mid = dir.join("play-mid.png");
    cli_json(&["play", "screenshot", &f, "--output", shot_mid.to_str().unwrap(), "--width", "1280", "--height", "720"]);
    cli_json(&["play", "step", &f, "--steps", "30"]);
    let shot_b = dir.join("play-b.png");
    cli_json(&["play", "screenshot", &f, "--output", shot_b.to_str().unwrap(), "--width", "1280", "--height", "720"]);
    let logs = cli_json(&["play", "logs", &f]);
    // Commit the runtime state so the tweened GUI can be rendered and
    // probed through the normal UI pipeline.
    cli_json(&["play", "stop", &f, "--commit"]);
    let text = serde_json::to_string(&logs).unwrap();
    assert!(text.contains("PANEL STATE Completed 16"), "tween completed at goal offset 16: {text}");
    for shot in [&shot_a, &shot_mid, &shot_b] {
        let bytes = std::fs::read(shot).unwrap();
        assert!(bytes.len() > 200 && &bytes[0..4] == b"\x89PNG", "play screenshot genuine: {shot:?}");
    }
    // After commit, the panel renders at its tweened on-screen spot:
    // probe its centre (16+160, 80+36+120) through ui render.
    let probe_out = dir.join("after.png");
    let render = cli_json(&["ui", "render", &f, "--ignore-inset", "--output", probe_out.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probe", "176,236"]);
    let rgba = &render["probes"][0]["rgba"];
    assert!(rgba[0].as_i64().unwrap() > 150 && rgba[1].as_i64().unwrap() < 110,
        "committed panel renders red at its goal position: {rgba}");

    // Export/reimport: the fixture (scripts included) survives.
    let exported = dir.join("export.rbxlx");
    cli_ok(&["export", &f, exported.to_str().unwrap()]);
    let (node, _) = bloxcli::rbxio::read_place(&exported).expect("reimport exported fixture");
    let panel = node.find_path("StarterGui/HUD/Inventory").expect("panel reimported");
    assert!(panel.properties.contains_key("Position"));
    let script = node.find_path("StarterGui/UIHandler").expect("script reimported");
    match script.properties.get("Source") {
        Some(bloxcli::tree::PropValue::String(s)) => assert!(s.contains("TweenInfo.new"), "script source preserved"),
        other => panic!("script Source: {other:?}"),
    }
}
