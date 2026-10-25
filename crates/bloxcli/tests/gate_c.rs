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
    "#]);
    let out = dir.join("out.rbxl");
    let run = cli_json(&["run", &f, "--duration", "0.05", "--save-runtime", out.to_str().unwrap()]);
    let logs = serde_json::to_string(&run).unwrap();
    assert!(logs.contains("ColorSequence"), "{logs}");
    assert!(logs.contains("range 0.5 9"), "{logs}");
    assert!(logs.contains("phys 2 0.9"), "{logs}");
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
