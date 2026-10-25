//! Gate C1 tests: Luau datatype constructors and property round-trips.

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

#[test]
fn datatype_constructors_round_trip() {
    let dir = std::env::temp_dir().join(format!("bloxcli-gatec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("g.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "HUD"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD", "--class", "Frame", "--name", "Panel"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD/Panel", "--class", "UIGradient", "--name", "Grad"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Typed"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value", r#"
        local grad = game.StarterGui.HUD.Panel.Grad
        grad.Color = ColorSequence.new({
            ColorSequenceKeypoint.new(0, Color3.new(1, 0, 0)),
            ColorSequenceKeypoint.new(1, Color3.new(0, 0, 1)),
        })
        grad.Transparency = NumberSequence.new(0.25)
        print("types", typeof(grad.Color), typeof(grad.Transparency))
        print("keypoints", #grad.Color.Keypoints, grad.Color.Keypoints[2].Value.B)
        local range = NumberRange.new(0.5, 9)
        print("range", range.Min, range.Max, typeof(range))
        local part = workspace.Typed
        part.CustomPhysicalProperties = PhysicalProperties.new(2, 0.1, 0.9, 1, 1)
        print("phys", part.CustomPhysicalProperties.Density, part.CustomPhysicalProperties.Elasticity)
        local params = OverlapParams.new()
        params.FilterType = Enum.RaycastFilterType.Exclude
        params.FilterDescendantsInstances = {part}
        params.MaxParts = 5
        print("overlap", params.MaxParts, params.FilterType.Name)
        local ordered = game:GetService("DataStoreService"):GetOrderedDataStore("Scores")
        ordered:SetAsync("a", 30); ordered:SetAsync("b", 10); ordered:SetAsync("c", 20)
        local pages = ordered:GetSortedAsync(true, 2)
        local page = pages:GetCurrentPage()
        print("sorted", page[1].key, page[2].key, pages.IsFinished)
        pages:AdvanceToNextPageAsync()
        local page2 = pages:GetCurrentPage()
        print("sorted2", page2[1].key, pages.IsFinished)
    "#]);
    let out = dir.join("out.rbxl");
    let run = cli_json(&["run", &f, "--duration", "0.05", "--save-runtime", out.to_str().unwrap()]);
    let logs = serde_json::to_string(&run).unwrap();
    assert!(logs.contains("ColorSequence"), "{logs}");
    assert!(logs.contains("range 0.5 9"), "{logs}");
    assert!(logs.contains("phys 2 0.9"), "{logs}");
    assert!(logs.contains("sorted b c"), "{logs}");
    assert!(logs.contains("sorted2 a true"), "{logs}");
    let (node, _) = bloxcli::rbxio::read_place(&out).expect("reopen");
    let part = node.find_path("Workspace/Typed").expect("part");
    match part.properties.get("CustomPhysicalProperties") {
        Some(bloxcli::tree::PropValue::PhysicalProperties(Some(v))) => {
            assert!((v[0] - 2.0).abs() < 1e-4 && (v[2] - 0.9).abs() < 1e-4, "physical props: {v:?}");
        }
        other => panic!("CustomPhysicalProperties round-trip: {other:?}"),
    }
    let grad = node.find_path("StarterGui/HUD/Panel/Grad").expect("gradient");
    assert!(grad.properties.contains_key("Color"), "gradient Color preserved: {:?}", grad.properties.keys().collect::<Vec<_>>());
}

#[test]
fn wedge_collider_is_a_real_slope() {
    // A ball dropped on a WedgePart must slide downhill (-Z here)
    // instead of resting on a box top.
    let dir = std::env::temp_dir().join(format!("bloxcli-gatec-wedge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("g.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "WedgePart", "--name", "Ramp"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ramp", "--property", "Size", "--value", "20,10,20"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ramp", "--position", "0,5,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ramp", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ball"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ball", "--property", "Shape", "--type", "Enum", "--value", "0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ball", "--property", "Size", "--value", "2,2,2"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ball", "--position", "0,14,4"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Watch"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Watch", "--property", "Source", "--type", "String", "--value",
        "task.wait(2.5)\nlocal b = workspace.Ball\nprint(string.format('ball %.2f %.2f %.2f', b.Position.X, b.Position.Y, b.Position.Z))\n"]);
    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "240"]);
    let logs = cli_json(&["play", "logs", &f]);
    cli_json(&["play", "stop", &f]);
    let text = serde_json::to_string(&logs).unwrap();
    let nums: Vec<f64> = text.split("ball ").nth(1).map(|s| s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter_map(|w| w.parse().ok()).collect()).unwrap_or_default();
    assert!(nums.len() >= 3, "ball log: {text}");
    let (y, z) = (nums[1], nums[2]);
    // Box top would rest at y = 10 (wedge top) + 1 = 11 with z ~ 4.
    // A real slope: the ball rolls toward -Z and loses height.
    assert!(z < 1.0 && y < 9.5, "ball slid down the wedge slope: y={y} z={z}");
}
