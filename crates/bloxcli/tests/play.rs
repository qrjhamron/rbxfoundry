//! Phase 6 integration tests: persistent play sessions with a Jolt-driven
//! character. Movement assertions use fixed steps and generous tolerances
//! (physics is deterministic per-build, but we assert behaviour, not bits).

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(candidate.is_file(), "blox-engine not found at {}", candidate.display());
    candidate
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--engine")
        .arg(engine_path())
        .args(args)
        .output()
        .expect("run bloxcli")
}

fn cli_ok(args: &[&str]) -> String {
    let out = cli(args);
    assert!(out.status.success(), "bloxcli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

fn cli_json(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json")
        .arg("--engine")
        .arg(engine_path())
        .args(args)
        .output()
        .expect("run bloxcli --json");
    assert!(out.status.success(), "bloxcli --json {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
    serde_json::from_slice(&out.stdout).expect("json output")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-play-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Stops the session on drop so daemons never leak from a failed test.
struct Session(PathBuf);
impl Session {
    fn state(&self) -> serde_json::Value {
        cli_json(&["play", "state", self.0.to_str().unwrap()])["state"].clone()
    }
    fn pos(&self) -> (f64, f64, f64) {
        let state = self.state();
        let p = state["character"]["position"].as_array().unwrap();
        (p[0].as_f64().unwrap(), p[1].as_f64().unwrap(), p[2].as_f64().unwrap())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = cli(&["play", "stop", self.0.to_str().unwrap()]);
    }
}

fn build_world(dir: &std::path::Path) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ground"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Size", "--value", "80,2,80"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-1,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Color", "--value", "0.35,0.75,0.35"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "SpawnLocation", "--name", "Spawn"]);
    cli_ok(&["move", &f, "--target", "Workspace/Spawn", "--position", "0,0.5,10"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Wall"]);
    cli_ok(&["set", &f, "--target", "Workspace/Wall", "--property", "Size", "--value", "24,8,1"]);
    cli_ok(&["move", &f, "--target", "Workspace/Wall", "--position", "0,4,-8"]);
    cli_ok(&["set", &f, "--target", "Workspace/Wall", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Crate"]);
    cli_ok(&["set", &f, "--target", "Workspace/Crate", "--property", "Size", "--value", "3,3,3"]);
    cli_ok(&["move", &f, "--target", "Workspace/Crate", "--position", "8,12,0"]);
    f
}

#[test]
fn character_spawns_walks_jumps_and_stops_at_walls() {
    let dir = temp_dir("walk");
    let f = build_world(&dir);
    cli_ok(&["play", "start", &f]);
    let session = Session(PathBuf::from(&f));

    // Settle onto the spawn.
    cli_ok(&["play", "step", &f, "--steps", "60"]);
    let (x0, y0, z0) = session.pos();
    let state = session.state();
    assert_eq!(state["character"]["grounded"], true, "character grounded after settling: {state}");
    assert!(y0 < 4.0, "character stands near the ground, y={y0}");

    // Walk forward (-Z) for one second at WalkSpeed 16.
    cli_ok(&["play", "input", &f, "--keys", "W", "--duration", "1"]);
    let (x1, _y1, z1) = session.pos();
    let walked = z0 - z1;
    assert!(walked > 11.0 && walked < 19.0,
            "walked ~16 studs forward (got {walked}: {x0},{z0} -> {x1},{z1})");
    assert!(x1.abs() < 1.5, "no lateral drift, x={x1}");

    // Jump: brief airborne phase, then back on the ground.
    cli_ok(&["play", "input", &f, "--keys", "Space", "--duration", "0.12"]);
    cli_ok(&["play", "step", &f, "--steps", "8"]);
    let mid = session.state();
    assert_eq!(mid["character"]["grounded"], false, "character leaves the ground when jumping: {mid}");
    cli_ok(&["play", "step", &f, "--steps", "120"]);
    let landed = session.state();
    assert_eq!(landed["character"]["grounded"], true, "character lands after the jump: {landed}");

    // Keep walking into the wall: it must block, not tunnel.
    cli_ok(&["play", "input", &f, "--keys", "W", "--duration", "2"]);
    let (_x2, _y2, z2) = session.pos();
    assert!(z2 > -7.6 && z2 < -5.5, "wall at z=-8 (face -7.5) stops the character, z={z2}");

    // The unanchored crate fell and rests on the ground.
    let state_file = dir.join("game.rbxl.bloxplay").join("state.rbxl");
    let crate_pos = cli_json(&["get", state_file.to_str().unwrap(),
                               "--target", "Workspace/Crate", "--property", "Position"]);
    let cy = crate_pos["value"]["value"][1].as_f64().unwrap();
    assert!(cy > 0.8 && cy < 2.2, "crate fell from 12 and rests on the ground, y={cy}");

    // Screenshot from the live session shows the world.
    let shot = dir.join("play.png");
    cli_ok(&["play", "screenshot", &f, "--output", shot.to_str().unwrap()]);
    let bytes = std::fs::read(&shot).unwrap();
    assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn play_scripts_run_and_react() {
    let dir = temp_dir("scripts");
    let f = build_world(&dir);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Gameplay"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Gameplay", "--property", "Source",
             "--type", "String", "--value", r#"
print("gameplay script started")
local platform = Instance.new("Part")
platform.Name = "ScriptPlatform"
platform.Size = Vector3.new(8, 1, 8)
platform.Anchored = true
platform.CFrame = CFrame.new(0, 0.5, -20)
platform.Parent = workspace
local frames = 0
game:GetService("RunService").Heartbeat:Connect(function(dt)
    frames += 1
    if frames == 30 then
        workspace:SetAttribute("heartbeatFrames", frames)
        print("heartbeat observed")
    end
end)
"#]);
    cli_ok(&["play", "start", &f]);
    let session = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "90"]);
    let logs = cli_json(&["play", "logs", &f]);
    let text = serde_json::to_string(&logs).unwrap();
    assert!(text.contains("gameplay script started"), "session logs include script output: {text}");
    assert!(text.contains("heartbeat observed"), "Heartbeat fired during play steps: {text}");
    // The script-created platform exists in the live world snapshot.
    let state_file = dir.join("game.rbxl.bloxplay").join("state.rbxl");
    let platform = cli(&["get", state_file.to_str().unwrap(), "--target", "Workspace/ScriptPlatform"]);
    assert!(platform.status.success(), "script-created part is in the play world");
    // But NOT in the edit-mode place file.
    let original = cli(&["get", &f, "--target", "Workspace/ScriptPlatform"]);
    assert!(!original.status.success(), "play state stays out of the edit file until commit");
    drop(session);

    // Commit merges the runtime world back into the place file.
    cli_ok(&["play", "start", &f]);
    let session = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "30"]);
    drop(session);
    // (Session::drop stopped without commit; start once more and commit.)
    cli_ok(&["play", "start", &f]);
    cli_ok(&["play", "step", &f, "--steps", "30"]);
    cli_ok(&["play", "stop", &f, "--commit"]);
    let committed = cli(&["get", &f, "--target", "Workspace/Player"]);
    assert!(committed.status.success(), "committed place contains the character model");
}

#[test]
fn r15_spawns_and_session_lifecycle_is_strict() {
    let dir = temp_dir("lifecycle");
    let f = build_world(&dir);
    // No session yet: state/input/stop all fail cleanly.
    assert!(!cli(&["play", "state", &f]).status.success());
    cli_ok(&["play", "start", &f, "--rig", "R15"]);
    let session = Session(PathBuf::from(&f));
    assert_eq!(session.state()["rig"], "R15");
    // A second start is refused while one is live.
    assert!(!cli(&["play", "start", &f]).status.success(), "double start rejected");
    // R15 rig has the segmented parts.
    let state_file = dir.join("game.rbxl.bloxplay").join("state.rbxl");
    cli_ok(&["play", "step", &f, "--steps", "30"]);
    let upper = cli(&["get", state_file.to_str().unwrap(), "--target", "Workspace/Player/UpperTorso"]);
    assert!(upper.status.success(), "R15 character has UpperTorso");
    drop(session);
    // After stop, the session is gone.
    assert!(!cli(&["play", "state", &f]).status.success(), "state fails after stop");
}