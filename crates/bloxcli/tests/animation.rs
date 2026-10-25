//! Phase 10 integration tests: Luau raycasts/spatial queries, Jolt
//! constraints, touch events, Motor6D rig posing and Animator-driven
//! keyframe animation, slope walking and dynamic-body interaction.
//! Everything is asserted through public CLI state, per-session Luau
//! logs and runtime snapshots — no engine internals.

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
    let dir = std::env::temp_dir().join(format!("bloxcli-anim-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

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
    fn logs(&self) -> String {
        let value = cli_json(&["play", "logs", self.0.to_str().unwrap()]);
        value["logs"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|e| format!("{}: {}", e["level"].as_str().unwrap_or(""), e["message"].as_str().unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }
    fn snapshot_tree(&self) -> serde_json::Value {
        let state_file = self
            .0
            .with_file_name(format!(
                "{}.bloxplay/state.rbxl",
                self.0.file_name().unwrap().to_string_lossy()
            ));
        cli_json(&["tree", state_file.to_str().unwrap()])["tree"].clone()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = cli(&["play", "stop", self.0.to_str().unwrap()]);
    }
}

fn find_node<'a>(node: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    if node["name"].as_str() == Some(name) {
        return Some(node);
    }
    for child in node["children"].as_array()? {
        if let Some(found) = find_node(child, name) {
            return Some(found);
        }
    }
    None
}

fn node_pos(node: &serde_json::Value) -> (f64, f64, f64) {
    let cf = &node["properties"]["CFrame"]["value"];
    let pos = cf["position"].as_array().expect("cframe position");
    (
        pos[0].as_f64().unwrap(),
        pos[1].as_f64().unwrap(),
        pos[2].as_f64().unwrap(),
    )
}

fn base_place(dir: &std::path::Path) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ground"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Size", "--value", "80,2,80"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-1,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "SpawnLocation", "--name", "Spawn"]);
    cli_ok(&["move", &f, "--target", "Workspace/Spawn", "--position", "0,0.5,10"]);
    f
}

fn add_script(f: &str, parent: &str, name: &str, source: &str) {
    cli_ok(&["add", f, "--parent", parent, "--class", "Script", "--name", name]);
    cli_ok(&["set", f, "--target", &format!("{parent}/{name}"), "--property", "Source",
             "--type", "String", "--value", source]);
}

#[test]
fn luau_raycast_and_spatial_queries_hit_real_geometry() {
    let dir = temp_dir("raycast");
    let f = base_place(&dir);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Tower"]);
    cli_ok(&["set", &f, "--target", "Workspace/Tower", "--property", "Size", "--value", "4,20,4"]);
    cli_ok(&["move", &f, "--target", "Workspace/Tower", "--position", "10,10,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Tower", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    add_script(&f, "ServerScriptService", "Probe", r#"
local done = false
game:GetService("RunService").Heartbeat:Connect(function()
    if done then return end
    done = true
    local hit = workspace:Raycast(Vector3.new(10, 30, 0), Vector3.new(0, -100, 0))
    print("RAY hit=" .. tostring(hit ~= nil) .. " inst=" .. (hit and hit.Instance.Name or "none")
        .. " dist=" .. string.format("%.2f", hit and hit.Distance or -1))
    local miss = workspace:Raycast(Vector3.new(-30, 30, -30), Vector3.new(0, -100, 0))
    print("RAY2 ground=" .. (miss and miss.Instance.Name or "none"))
    local params = RaycastParams.new()
    params.FilterDescendantsInstances = {workspace.Tower}
    params.FilterType = Enum.RaycastFilterType.Exclude
    local through = workspace:Raycast(Vector3.new(10, 30, 0), Vector3.new(0, -100, 0), params)
    print("RAY3 through tower hits=" .. (through and through.Instance.Name or "none"))
    local near = workspace:GetPartBoundsInRadius(Vector3.new(10, 10, 0), 12)
    local names = {}
    for _, p in ipairs(near) do table.insert(names, p.Name) end
    table.sort(names)
    print("QUERY radius: " .. table.concat(names, ","))
    local boxed = workspace:GetPartBoundsInBox(CFrame.new(0, 30, 0), Vector3.new(5, 5, 5))
    print("QUERY box count=" .. tostring(#boxed))
end)
"#);
    cli_ok(&["play", "start", &f]);
    let session = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "30"]);
    let logs = session.logs();
    assert!(logs.contains("RAY hit=true inst=Tower dist=10.00"), "{logs}");
    assert!(logs.contains("RAY2 ground=Ground"), "{logs}");
    assert!(logs.contains("RAY3 through tower hits=Ground"), "{logs}");
    assert!(logs.contains("QUERY radius: Ground,Tower"), "{logs}");
    assert!(logs.contains("QUERY box count=0"), "{logs}");
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn constraints_hold_bodies_and_touch_events_fire() {
    let dir = temp_dir("constraints");
    let f = base_place(&dir);
    add_script(&f, "ServerScriptService", "Rig", r#"
local function part(name, pos, anchored)
    local p = Instance.new("Part")
    p.Name = name
    p.Size = Vector3.new(2, 2, 2)
    p.Position = pos
    p.Anchored = anchored
    p.Parent = workspace
    return p
end
-- BallSocket: B hangs from anchored A.
local a = part("AnchorA", Vector3.new(-10, 14, 0), true)
local b = part("BallB", Vector3.new(-10, 10, 0), false)
local a0 = Instance.new("Attachment") a0.CFrame = CFrame.new(0, -1, 0) a0.Parent = a
local a1 = Instance.new("Attachment") a1.CFrame = CFrame.new(0, 1, 0) a1.Parent = b
local socket = Instance.new("BallSocketConstraint")
socket.Attachment0 = a0 socket.Attachment1 = a1 socket.Parent = workspace
-- WeldConstraint: D rigidly attached to anchored C.
local c = part("AnchorC", Vector3.new(10, 14, 0), true)
local d = part("WeldD", Vector3.new(13, 14, 0), false)
local weld = Instance.new("WeldConstraint")
weld.Part0 = c weld.Part1 = d weld.Parent = workspace
-- Hinge: F swings from anchored E.
local e = part("AnchorE", Vector3.new(0, 14, -20), true)
local g = part("HingeF", Vector3.new(0, 10, -20), false)
local e0 = Instance.new("Attachment") e0.Parent = e
local e1 = Instance.new("Attachment") e1.Parent = g
local hinge = Instance.new("HingeConstraint")
hinge.Attachment0 = e0 hinge.Attachment1 = e1 hinge.Parent = workspace
-- Touch: falling crate reports what it lands on.
local crate = part("Crate", Vector3.new(20, 8, 20), false)
crate.Touched:Connect(function(other)
    print("TOUCHED " .. other.Name)
end)
"#);
    cli_ok(&["play", "start", &f]);
    let session = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "180"]);
    let tree = session.snapshot_tree();
    let b = find_node(&tree, "BallB").expect("BallB");
    let (_, by, _) = node_pos(b);
    assert!(by > 6.0 && by < 13.5, "ball-socket body hangs from its anchor, y={by}");
    let d = find_node(&tree, "WeldD").expect("WeldD");
    let (dx, dy, _) = node_pos(d);
    assert!((dx - 13.0).abs() < 0.6 && (dy - 14.0).abs() < 0.6,
            "welded body keeps its welded offset, at ({dx},{dy})");
    let g = find_node(&tree, "HingeF").expect("HingeF");
    let (_, gy, _) = node_pos(g);
    assert!(gy > 11.0 && gy < 15.0, "hinged body stays attached at the hinge point, y={gy}");
    let logs = session.logs();
    assert!(logs.contains("TOUCHED Ground"), "crate touch event fired: {logs}");
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn motor6d_rig_and_animator_keyframes_move_joints() {
    let dir = temp_dir("anim");
    let f = base_place(&dir);
    // Animation asset authored as instances: raise the right arm over 1s.
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage", "--class", "Animation", "--name", "Wave"]);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage/Wave", "--class", "KeyframeSequence", "--name", "Seq"]);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage/Wave/Seq", "--class", "Keyframe", "--name", "K0"]);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage/Wave/Seq/K0", "--class", "Pose", "--name", "Right Shoulder"]);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage/Wave/Seq", "--class", "Keyframe", "--name", "K1"]);
    cli_ok(&["set", &f, "--target", "ReplicatedStorage/Wave/Seq/K1", "--property", "Time", "--type", "Float64", "--value", "1"]);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage/Wave/Seq/K1", "--class", "Pose", "--name", "Right Shoulder"]);
    cli_ok(&["set", &f, "--target", "ReplicatedStorage/Wave/Seq/K1/Right Shoulder", "--property", "CFrame",
             "--type", "CFrame", "--value", "0,0,0,1,0,0,0,0.70710678,-0.70710678,0,0.70710678,0.70710678"]);
    add_script(&f, "ServerScriptService", "Animate", r#"
local started = false
game:GetService("RunService").Heartbeat:Connect(function()
    if started then return end
    local char = workspace:FindFirstChild("Player")
    if not char then return end
    local humanoid = char:FindFirstChildOfClass("Humanoid")
    local animator = humanoid and humanoid:FindFirstChildOfClass("Animator")
    if not animator then return end
    started = true
    local track = animator:LoadAnimation(game.ReplicatedStorage.Wave)
    print("TRACK length=" .. string.format("%.2f", track.Length))
    track:Play()
    print("TRACK playing=" .. tostring(track.IsPlaying))
end)
"#);
    cli_ok(&["play", "start", &f, "--rig", "R6"]);
    let session = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "30"]);
    let logs = session.logs();
    assert!(logs.contains("TRACK length=1.00"), "{logs}");
    assert!(logs.contains("TRACK playing=true"), "{logs}");

    // The rig exists as real instances: Motor6Ds + Animator.
    let tree = session.snapshot_tree();
    let shoulder = find_node(&tree, "Right Shoulder").expect("Right Shoulder Motor6D");
    assert_eq!(shoulder["className"].as_str(), Some("Motor6D"));
    let arm_before = node_pos(find_node(&tree, "Right Arm").unwrap());
    cli_ok(&["play", "step", &f, "--steps", "90"]);
    let tree = session.snapshot_tree();
    let arm_after = node_pos(find_node(&tree, "Right Arm").unwrap());
    let moved = ((arm_after.0 - arm_before.0).powi(2)
        + (arm_after.1 - arm_before.1).powi(2)
        + (arm_after.2 - arm_before.2).powi(2))
    .sqrt();
    assert!(moved > 0.8, "animated arm moved through Motor6D chain: {arm_before:?} -> {arm_after:?}");
    // The joint Transform itself carries the sampled pose.
    let shoulder = find_node(&tree, "Right Shoulder").unwrap();
    let rot = &shoulder["properties"]["Transform"]["value"]["rotation"];
    let identity = serde_json::json!([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert_ne!(rot, &identity, "Motor6D Transform holds the keyframed rotation");
    // Screenshot shows the raised-arm character.
    let shot = dir.join("wave.png");
    cli_ok(&["play", "screenshot", &f, "--output", shot.to_str().unwrap()]);
    let bytes = std::fs::read(&shot).expect("screenshot");
    assert!(bytes.len() > 1000 && &bytes[0..4] == b"\x89PNG");
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn character_walks_up_slope() {
    let dir = temp_dir("slope");
    let f = base_place(&dir);
    // A rotated slab forms a real 25-degree slope in front of the spawn.
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ramp"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ramp", "--property", "Size", "--value", "16,1,30"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ramp", "--position", "0,3,-14"]);
    cli_ok(&["rotate", &f, "--target", "Workspace/Ramp", "--angles", "25,0,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ramp", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_ok(&["play", "start", &f]);
    let session = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "60"]);
    let (_, y0, _) = session.pos();
    // Face the ramp (yaw 0 = -Z) and walk: the character leaves the
    // spawn pad, crosses the flat, then climbs the 25-degree slab.
    cli_ok(&["play", "input", &f, "--keys", "W", "--duration", "2"]);
    let (_, y1, z1) = session.pos();
    assert!(z1 < -12.0, "character advanced along the ramp, z={z1}");
    assert!(y1 > y0 + 3.0, "character climbed the slope: y {y0} -> {y1}");
    cli_ok(&["play", "stop", &f]);
}
