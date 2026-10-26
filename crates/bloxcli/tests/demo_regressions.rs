//! Required Demo Regressions — synthetic equivalents of the named
//! demo projects (original sources are not in this repository, and no
//! engine code is special-cased for them):
//! - CROWNWARD: obby platforms, checkpoints UI, HUD transitions.
//! - MOTORWORKS: factory progression, script-spawned products, coins,
//!   purchase button, script-updated GUI.
//! - APEX/GP: camera modes + speedometer HUD driven by live character.
//! Plus Gate 6 TextStroke rendering and a Gate 7 start/stop stress.

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
fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-demo-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn base_place(dir: &PathBuf) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "Workspace", "Part", "Ground");
    set(&f, "Workspace/Ground", "Size", "Vector3", "120,1,120");
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-0.5,0"]);
    set(&f, "Workspace/Ground", "Anchored", "Bool", "true");
    set(&f, "Workspace/Ground", "Color", "Color3", "0.3,0.32,0.35");
    add(&f, "StarterGui", "ScreenGui", "HUD");
    f
}

#[test]
fn crownward_obby_synthetic() {
    let dir = temp("crownward");
    let f = base_place(&dir);
    // Ascending obby steps + a finish platform.
    for i in 1..=5 {
        let name = format!("Step{i}");
        add(&f, "Workspace", "Part", &name);
        set(&f, &format!("Workspace/{name}"), "Size", "Vector3", "8,1,8");
        cli_ok(&["move", &f, "--target", &format!("Workspace/{name}"), "--position",
                  &format!("{},{},{}", i * 10, i, -20)]);
        set(&f, &format!("Workspace/{name}"), "Anchored", "Bool", "true");
        set(&f, &format!("Workspace/{name}"), "Color", "Color3", "0.85,0.6,0.2");
    }
    add(&f, "StarterGui/HUD", "TextLabel", "Stage");
    set(&f, "StarterGui/HUD/Stage", "Size", "UDim2", "0,200,0,40");
    set(&f, "StarterGui/HUD/Stage", "Position", "UDim2", "0,16,0,16");
    set(&f, "StarterGui/HUD/Stage", "Text", "String", "Stage 1");
    set(&f, "StarterGui/HUD/Stage", "BackgroundColor3", "Color3", "0.1,0.1,0.14");
    set(&f, "StarterGui/HUD/Stage", "TextColor3", "Color3", "1,1,1");
    add(&f, "StarterGui/HUD", "TextButton", "Next");
    set(&f, "StarterGui/HUD/Next", "Size", "UDim2", "0,120,0,44");
    set(&f, "StarterGui/HUD/Next", "Position", "UDim2", "0,16,0,64");
    set(&f, "StarterGui/HUD/Next", "Text", "String", "Next Stage");
    add(&f, "StarterGui/HUD", "LocalScript", "Ctl");
    set(&f, "StarterGui/HUD/Ctl", "Source", "String", r#"
        local stage = 1
        script.Parent.Next.MouseButton1Click:Connect(function()
            stage += 1
            script.Parent.Stage.Text = "Stage " .. stage
            print("stage now", stage)
        end)
    "#);
    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "90"]);
    let state = cli_json(&["play", "state", &f]);
    assert_eq!(state["state"]["character"]["grounded"], true, "character grounded on obby spawn: {}", state["state"]);
    let shot = dir.join("obby.png");
    let r = cli_json(&["play", "screenshot", &f, "--output", shot.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probes", "100,70"]);
    let p = &r["result"]["probes"][0]["rgba"];
    assert!(p[0].as_i64().unwrap() < 80, "HUD stage label plate renders: {p}");
    cli_json(&["play", "input", &f, "--click", "76,123"]);
    let logs = cli_json(&["play", "logs", &f]);
    assert!(serde_json::to_string(&logs).unwrap().contains("stage now 2"), "{logs}");
    let state2 = cli_json(&["play", "state", &f]);
    let rects = state2["state"]["gui"]["rects"].as_array().unwrap();
    let stage = rects.iter().find(|x| x["name"].as_str() == Some("Stage")).unwrap();
    assert_eq!(stage["text"], "Stage 2", "HUD transition applied: {stage}");
    cli_json(&["play", "stop", &f]);
}

#[test]
fn motorworks_factory_synthetic() {
    let dir = temp("motorworks");
    let f = base_place(&dir);
    add(&f, "Workspace", "Part", "Conveyor");
    set(&f, "Workspace/Conveyor", "Size", "Vector3", "60,1,6");
    cli_ok(&["move", &f, "--target", "Workspace/Conveyor", "--position", "0,2,-8"]);
    set(&f, "Workspace/Conveyor", "Anchored", "Bool", "true");
    add(&f, "ReplicatedStorage", "IntValue", "Coins");
    set(&f, "ReplicatedStorage/Coins", "Value", "Int32", "0");
    add(&f, "ServerScriptService", "Script", "Factory");
    set(&f, "ServerScriptService/Factory", "Source", "String", r#"
        local coins = game.ReplicatedStorage.Coins
        local n = 0
        while true do
            task.wait(0.5)
            n += 1
            coins.Value += 10
            local p = Instance.new("Part")
            p.Name = "Product" .. n
            p.Size = Vector3.new(2, 2, 2)
            p.Position = Vector3.new(-20 + n * 4, 6, -8)
            p.Color = Color3.new(1, 0.5, 0)
            p.Parent = workspace
            print("tick", n, "coins", coins.Value)
        end
    "#);
    add(&f, "StarterGui/HUD", "TextButton", "Buy");
    set(&f, "StarterGui/HUD/Buy", "Size", "UDim2", "0,140,0,48");
    set(&f, "StarterGui/HUD/Buy", "Position", "UDim2", "0,16,0,16");
    set(&f, "StarterGui/HUD/Buy", "Text", "String", "Buy (30)");
    add(&f, "StarterGui/HUD", "LocalScript", "Shop");
    set(&f, "StarterGui/HUD/Shop", "Source", "String", r#"
        local coins = game.ReplicatedStorage.Coins
        script.Parent.Buy.MouseButton1Click:Connect(function()
            if coins.Value >= 30 then
                coins.Value -= 30
                print("purchased, coins", coins.Value)
            end
        end)
    "#);
    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "260"]);
    let logs = cli_json(&["play", "logs", &f]);
    let text = serde_json::to_string(&logs).unwrap();
    assert!(text.contains("tick 4"), "factory progressed: {text}");
    let state = cli_json(&["play", "state", &f]);
    assert!(state["state"]["bodies"].as_i64().unwrap() >= 6, "spawned products have bodies: {}", state["state"]);
    cli_json(&["play", "input", &f, "--click", "86,76"]);
    let logs2 = cli_json(&["play", "logs", &f]);
    assert!(serde_json::to_string(&logs2).unwrap().contains("purchased"), "{logs2}");
    cli_json(&["play", "stop", &f]);
}

#[test]
fn apex_speedometer_synthetic() {
    let dir = temp("apex");
    let f = base_place(&dir);
    add(&f, "StarterGui/HUD", "TextLabel", "Speedo");
    set(&f, "StarterGui/HUD/Speedo", "Size", "UDim2", "0,220,0,44");
    set(&f, "StarterGui/HUD/Speedo", "Position", "UDim2", "0,16,0,16");
    set(&f, "StarterGui/HUD/Speedo", "Text", "String", "P 0");
    set(&f, "StarterGui/HUD/Speedo", "BackgroundColor3", "Color3", "0.08,0.08,0.1");
    set(&f, "StarterGui/HUD/Speedo", "TextColor3", "Color3", "1,1,1");
    add(&f, "StarterGui/HUD", "LocalScript", "Dash");
    set(&f, "StarterGui/HUD/Dash", "Source", "String", r#"
        local player = game.Players.LocalPlayer
        local label = script.Parent.Speedo
        task.spawn(function()
            while true do
                task.wait(0.1)
                local ch = player.Character
                local hrp = ch and ch:FindFirstChild("HumanoidRootPart")
                if hrp then
                    label.Text = "P " .. math.floor(hrp.Position.Magnitude + 0.5)
                end
            end
        end)
    "#);
    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "30"]);
    // Third-person frame, then walk forward and check the live readout.
    let s1 = dir.join("third.png");
    cli_json(&["play", "screenshot", &f, "--output", s1.to_str().unwrap()]);
    cli_json(&["play", "input", &f, "--keys", "W", "--duration", "1.5"]);
    let state = cli_json(&["play", "state", &f]);
    let rects = state["state"]["gui"]["rects"].as_array().unwrap();
    let speedo = rects.iter().find(|x| x["name"].as_str() == Some("Speedo")).unwrap();
    let text = speedo["text"].as_str().unwrap_or("");
    assert!(text.starts_with("P ") && text != "P 0", "speedometer tracks character: '{text}'");
    let s2 = dir.join("first.png");
    let r2 = cli_json(&["play", "screenshot", &f, "--output", s2.to_str().unwrap()]);
    assert_eq!(r2["result"]["camera"]["position"].as_array().unwrap().len(), 3);
    assert_ne!(std::fs::read(&s1).unwrap(), std::fs::read(&s2).unwrap(), "camera frames differ after movement");
    cli_json(&["play", "stop", &f]);
}

#[test]
fn text_stroke_renders_outline() {
    let dir = temp("stroke");
    let f = base_place(&dir);
    add(&f, "StarterGui/HUD", "TextLabel", "Title");
    set(&f, "StarterGui/HUD/Title", "Size", "UDim2", "0,320,0,80");
    set(&f, "StarterGui/HUD/Title", "Position", "UDim2", "0,40,0,40");
    set(&f, "StarterGui/HUD/Title", "Text", "String", "Hi");
    set(&f, "StarterGui/HUD/Title", "TextSize", "Float64", "48");
    set(&f, "StarterGui/HUD/Title", "TextColor3", "Color3", "1,1,1");
    set(&f, "StarterGui/HUD/Title", "BackgroundColor3", "Color3", "0.05,0.05,0.08");
    set(&f, "StarterGui/HUD/Title", "TextStrokeColor3", "Color3", "1,0,0");
    set(&f, "StarterGui/HUD/Title", "TextStrokeTransparency", "Float64", "0");
    let shot = dir.join("stroke.png");
    let probes: Vec<String> = (40..140).step_by(3).map(|x| format!("{x},80")).collect();
    let r = cli_json(&["ui", "render", &f, "--ignore-inset", "--output", shot.to_str().unwrap(),
        "--width", "800", "--height", "450", "--probe", &probes.join(";")]);
    let arr = r["probes"].as_array().unwrap();
    let px: Vec<(i64, i64, i64)> = arr.iter().map(|p| {
        let a = p["rgba"].as_array().unwrap();
        (a[0].as_i64().unwrap(), a[1].as_i64().unwrap(), a[2].as_i64().unwrap())
    }).collect();
    assert!(px.iter().any(|c| c.0 > 120 && c.1 < 90 && c.2 < 90), "red stroke pixels on scanline: {px:?}");
    assert!(px.iter().any(|c| c.0 > 200 && c.1 > 200 && c.2 > 200), "white fill pixels on scanline: {px:?}");
}

#[test]
fn gate7_repeated_sessions_are_stable() {
    let dir = temp("stress");
    let f = base_place(&dir);
    add(&f, "ServerScriptService", "Script", "Stress");
    set(&f, "ServerScriptService/Stress", "Source", "String", r#"
        for i = 1, 300 do
            local p = Instance.new("Part")
            p.Position = Vector3.new(i % 17, 8, -10)
            p.Parent = workspace
            if i % 3 == 0 then p:Destroy() end
        end
        print("stress done")
    "#);
    for cycle in 0..3 {
        cli_json(&["play", "start", &f]);
        cli_json(&["play", "step", &f, "--steps", "60"]);
        let logs = cli_json(&["play", "logs", &f]);
        assert!(serde_json::to_string(&logs).unwrap().contains("stress done"), "cycle {cycle}: {logs}");
        let shot = dir.join(format!("cycle{cycle}.png"));
        let r = cli_json(&["play", "screenshot", &f, "--output", shot.to_str().unwrap(), "--width", "640", "--height", "360"]);
        assert_eq!(r["result"]["ok"], true, "cycle {cycle} screenshot");
        cli_json(&["play", "stop", &f]);
    }
}
