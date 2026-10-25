//! Gate A regression tests: A2 property-type correctness, A3/A4
//! physics synchronization and body identity.

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
fn temp_dir(tag: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("bloxcli-gatea-{}-{tag}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }

#[test]
fn luau_gui_creation_round_trips_with_correct_types() {
    let dir = temp_dir("a2");
    let file = dir.join("g.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value", r#"
        local label = Instance.new("TextLabel")
        label.Name = "Title"
        label.ZIndex = 3
        label.TextSize = 20
        label.BackgroundTransparency = 0.5
        label.Text = "Hello"
        label.Size = UDim2.new(0, 200, 0, 50)
        label.Parent = game.StarterGui
        local part = Instance.new("Part")
        part.Name = "Typed"
        part.Transparency = 0.25
        part.Parent = workspace
    "#]);
    let out = dir.join("out.rbxl");
    let run = cli_json(&["run", &f, "--duration", "0.05", "--save-runtime", out.to_str().unwrap()]);
    assert_eq!(run["ok"], true, "script + export succeed: {run}");
    let (node, _) = bloxcli::rbxio::read_place(&out).expect("reopen exported place");
    let label = node.find_path("StarterGui/Title").expect("label exported");
    assert!(matches!(label.properties.get("ZIndex"), Some(bloxcli::tree::PropValue::Int32(3))), "ZIndex Int32: {:?}", label.properties.get("ZIndex"));
    assert!(matches!(label.properties.get("TextSize"), Some(bloxcli::tree::PropValue::Float32(v)) if (*v - 20.0).abs() < 1e-4), "TextSize Float32: {:?}", label.properties.get("TextSize"));
    // Non-integral assignment to an Int32 property errors in Luau.
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value",
        "local l = Instance.new('TextLabel')\nl.ZIndex = 2.5\n"]);
    let bad = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--json").arg("--engine").arg(engine_path())
        .args(["run", &f, "--duration", "0.05"]).output().unwrap();
    assert!(!bad.status.success(), "non-integral ZIndex assignment must fail");
}

#[test]
fn runtime_spawned_and_destroyed_parts_sync_to_physics() {
    let dir = temp_dir("a3");
    let file = dir.join("g.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ground"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Size", "--value", "80,2,80"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-1,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Spawner"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Spawner", "--property", "Source", "--type", "String", "--value", r#"
        task.wait(0.2)
        local p = Instance.new("Part")
        p.Name = "Dropped"
        p.Size = Vector3.new(2, 2, 2)
        p.Position = Vector3.new(0, 10, 0)
        p.Parent = workspace
        task.wait(2)
        print("dropped y", string.format("%.2f", p.Position.Y))
        workspace.Ground:Destroy()
        task.wait(1)
        print("after destroy y", string.format("%.2f", p.Position.Y))
    "#]);
    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "300"]);
    let logs = cli_json(&["play", "logs", &f]);
    cli_json(&["play", "stop", &f]);
    let text = serde_json::to_string(&logs).unwrap();
    // Spawned part fell and rested on the ground (centre ~1), then fell
    // again after the ground was destroyed (well below rest height).
    let dropped: f64 = text.split("dropped y ").nth(1).and_then(|s| s[..s.find('"')?].parse().ok()).expect(&format!("dropped log in {text}"));
    assert!((dropped - 1.0).abs() < 0.6, "spawned part rests on ground, y={dropped}");
    let after: f64 = text.split("after destroy y ").nth(1).and_then(|s| s[..s.find('"')?].parse().ok()).expect("after destroy log");
    assert!(after < dropped - 2.0, "destroyed ground removes its collider, part falls: y={after}");
}
