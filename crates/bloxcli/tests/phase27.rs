//! Phase 27B tests: real TweenService behaviour — TweenInfo, easing
//! mathematics (direction-aware), UDim2 GUI property interpolation,
//! delay/repeat/reverse lifecycle, deterministic scheduler stepping.

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") { if !p.is_empty() { return PathBuf::from(p); } }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(candidate.is_file(), "blox-engine not found at {}", candidate.display());
    candidate
}
fn cli_ok(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--engine").arg(engine_path()).args(args).output().expect("run bloxcli");
    assert!(out.status.success(), "bloxcli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8")
}
fn cli_json(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--json").arg("--engine").arg(engine_path()).args(args).output().expect("run bloxcli");
    assert!(out.status.success(), "bloxcli --json {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("json")
}
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-p27-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Runs a server Script tweening StarterGui/Panel.Position (UDim2).
/// Returns the saved runtime Position complex value [xs, xo, ys, yo].
fn tween_position_after(duration: &str, source: &str) -> Vec<f64> {
    let dir = temp_dir("tween");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "Hud"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/Hud", "--class", "Frame", "--name", "Panel"]);
    cli_ok(&["set", &f, "--target", "StarterGui/Hud/Panel", "--property", "Position", "--type", "UDim2", "--value", "0,0,0,0"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value", source]);
    let out = dir.join("runtime.rbxl");
    cli_json(&["run", &f, "--duration", duration, "--step", "0.05", "--save-runtime", out.to_str().unwrap()]);
    let (node, _) = bloxcli::rbxio::read_place(&out).expect("runtime snapshot");
    let panel = node.find_path("StarterGui/Hud/Panel").expect("panel");
    match panel.properties.get("Position") {
        Some(bloxcli::tree::PropValue::UDim2(v)) => v.iter().map(|x| *x as f64).collect(),
        other => panic!("expected UDim2 Position, got {other:?}"),
    }
}

#[test]
fn tweeninfo_easing_and_udim2_interpolation() {
    // Sine Out at alpha 0.5 => sin(pi/4) ~= 0.7071 of the way to X scale 1.
    let pos = tween_position_after("0.5", r#"
        local panel = game.StarterGui.Hud.Panel
        local info = TweenInfo.new(1, Enum.EasingStyle.Sine, Enum.EasingDirection.Out)
        local tween = game:GetService("TweenService"):Create(panel, info, {Position = UDim2.new(1, 0, 0, 100)})
        print("state before", tween.PlaybackState)
        tween:Play()
        print("state after play", tween.PlaybackState)
    "#);
    assert!((pos[0] - 0.7071).abs() < 0.03, "SineOut(0.5) X scale ~= 0.707, got {pos:?}");
    assert!((pos[3] - 70.71).abs() < 3.0, "SineOut(0.5) Y offset ~= 70.7, got {pos:?}");

    // Quad In at alpha 0.5 => 0.25.
    let pos = tween_position_after("0.5", r#"
        local panel = game.StarterGui.Hud.Panel
        local tween = game:GetService("TweenService"):Create(panel, TweenInfo.new(1, Enum.EasingStyle.Quad, Enum.EasingDirection.In), {Position = UDim2.new(1, 0, 0, 0)})
        tween:Play()
    "#);
    assert!((pos[0] - 0.25).abs() < 0.03, "QuadIn(0.5) ~= 0.25, got {pos:?}");

    // Completed tween lands exactly on the goal.
    let pos = tween_position_after("1.2", r#"
        local panel = game.StarterGui.Hud.Panel
        local tween = game:GetService("TweenService"):Create(panel, TweenInfo.new(1, Enum.EasingStyle.Linear), {Position = UDim2.new(1, 0, 0, 100)})
        tween.Completed:Connect(function(state) print("completed state", state) end)
        tween:Play()
    "#);
    assert!((pos[0] - 1.0).abs() < 1e-4 && (pos[3] - 100.0).abs() < 1e-3, "completed at goal, got {pos:?}");
}

#[test]
fn tween_delay_repeat_reverse() {
    // Delay 0.5s of a 1s tween, sampled at 0.25s: not yet moved.
    let pos = tween_position_after("0.25", r#"
        local panel = game.StarterGui.Hud.Panel
        local tween = game:GetService("TweenService"):Create(panel, TweenInfo.new(1, Enum.EasingStyle.Linear, Enum.EasingDirection.Out, 0, false, 0.5), {Position = UDim2.new(1, 0, 0, 0)})
        tween:Play()
    "#);
    assert!(pos[0].abs() < 1e-4, "delayed tween has not moved, got {pos:?}");

    // Reverses with RepeatCount 1: at 1.5s (halfway back) => 0.5.
    let pos = tween_position_after("1.5", r#"
        local panel = game.StarterGui.Hud.Panel
        local tween = game:GetService("TweenService"):Create(panel, TweenInfo.new(1, Enum.EasingStyle.Linear, Enum.EasingDirection.Out, 1, true, 0), {Position = UDim2.new(1, 0, 0, 0)})
        tween:Play()
    "#);
    assert!((pos[0] - 0.5).abs() < 0.06, "reversed halfway back ~= 0.5, got {pos:?}");
}

#[test]
fn elastic_ball_touched_and_touchended() {
    let dir = temp_dir("touch");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ground"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Size", "--value", "80,2,80"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-1,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "CustomPhysicalProperties", "--type", "PhysicalProperties", "--value", "1,0.0,0.95,1,1,1"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ball"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ball", "--property", "Size", "--value", "2,2,2"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ball", "--position", "0,10,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ball", "--property", "CustomPhysicalProperties", "--type", "PhysicalProperties", "--value", "1,0.0,0.95,1,1,1"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Watch"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Watch", "--property", "Source", "--type", "String", "--value",
        "local ball = workspace.Ball\nball.Touched:Connect(function(other) print('HIT ' .. other.Name) end)\nball.TouchEnded:Connect(function(other) print('END ' .. other.Name) end)\n"]);
    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "240"]);
    let logs = cli_json(&["play", "logs", &f]);
    let text = serde_json::to_string(&logs).unwrap();
    cli_json(&["play", "stop", &f]);
    assert!(text.contains("HIT Ground"), "Touched fired on impact: {text}");
    assert!(text.contains("END Ground"), "TouchEnded fired after elastic separation: {text}");
}
