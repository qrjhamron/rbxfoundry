//! Phase 14/15 behavioral tests: ContextActionService, Humanoid state
//! API, AnimationTrack blending, collision groups, stair climbing and
//! PathfindingService — all through real Luau programs and physics.

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") { if !p.is_empty() { return PathBuf::from(p); } }
    let c = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(c.is_file(), "engine missing");
    c
}
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--engine").arg(engine_path()).args(args).output().unwrap()
}
fn cli_ok(args: &[&str]) -> String {
    let o = cli(args);
    assert!(o.status.success(), "bloxcli {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}
fn cli_json(args: &[&str]) -> serde_json::Value {
    let o = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--json").arg("--engine").arg(engine_path()).args(args).output().unwrap();
    assert!(o.status.success(), "bloxcli --json {args:?}: {}\n{}", String::from_utf8_lossy(&o.stderr), String::from_utf8_lossy(&o.stdout));
    serde_json::from_slice(&o.stdout).unwrap()
}
fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bloxcli-p14-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn find_node<'a>(n: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    if n["name"].as_str() == Some(name) { return Some(n); }
    for c in n["children"].as_array()? { if let Some(f) = find_node(c, name) { return Some(f); } }
    None
}
fn node_y(n: &serde_json::Value) -> f64 { n["properties"]["CFrame"]["value"]["position"][1].as_f64().unwrap() }

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
fn add_script(f: &str, name: &str, source: &str) {
    cli_ok(&["add", f, "--parent", "ServerScriptService", "--class", "Script", "--name", name]);
    cli_ok(&["set", f, "--target", &format!("ServerScriptService/{name}"), "--property", "Source", "--type", "String", "--value", source]);
}
struct Session(PathBuf);
impl Session {
    fn logs(&self) -> String {
        let v = cli_json(&["play", "logs", self.0.to_str().unwrap()]);
        v["logs"].as_array().map(|a| a.iter().map(|e| e["message"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
    }
    fn pos(&self) -> (f64, f64, f64) {
        let s = cli_json(&["play", "state", self.0.to_str().unwrap()]);
        let p = s["state"]["character"]["position"].as_array().unwrap();
        (p[0].as_f64().unwrap(), p[1].as_f64().unwrap(), p[2].as_f64().unwrap())
    }
    fn tree(&self) -> serde_json::Value {
        let sf = self.0.with_file_name(format!("{}.bloxplay/state.rbxl", self.0.file_name().unwrap().to_string_lossy()));
        cli_json(&["tree", sf.to_str().unwrap()])["tree"].clone()
    }
}
impl Drop for Session { fn drop(&mut self) { let _ = cli(&["play", "stop", self.0.to_str().unwrap()]); } }

#[test]
fn context_action_service_fires_from_play_input() {
    let dir = temp_dir("cas");
    let f = base_place(&dir);
    add_script(&f, "Bind", r#"
local cas = game:GetService("ContextActionService")
cas:BindAction("Interact", function(name, state, input)
    print("ACTION " .. name .. " " .. state.Name .. " key=" .. input.KeyCode.Name)
end, false, Enum.KeyCode.E)
cas:BindAction("Temp", function() print("TEMP SHOULD NOT FIRE") end, false, Enum.KeyCode.Q)
cas:UnbindAction("Temp")
"#);
    cli_ok(&["play", "start", &f]);
    let s = Session(PathBuf::from(&f));
    cli_ok(&["play", "input", &f, "--keys", "E", "--duration", "0.3"]);
    cli_ok(&["play", "step", &f, "--steps", "10"]);
    let logs = s.logs();
    assert!(logs.contains("ACTION Interact Begin key=E"), "{logs}");
    assert!(!logs.contains("TEMP SHOULD NOT FIRE"), "{logs}");
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn humanoid_state_transitions_are_observable() {
    let dir = temp_dir("state");
    let f = base_place(&dir);
    add_script(&f, "Watch", r#"
local done = false
game:GetService("RunService").Heartbeat:Connect(function()
    if done then return end
    local char = workspace:FindFirstChild("Player")
    if not char then return end
    local humanoid = char:FindFirstChildOfClass("Humanoid")
    if not humanoid then return end
    done = true
    print("GETSTATE " .. humanoid:GetState().Name)
    humanoid.StateChanged:Connect(function(newState, oldState)
        print("STATE " .. oldState.Name .. " -> " .. newState.Name)
    end)
end)
"#);
    cli_ok(&["play", "start", &f]);
    let s = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "60"]);
    cli_ok(&["play", "input", &f, "--keys", "Space", "--duration", "0.25"]);
    cli_ok(&["play", "step", &f, "--steps", "90"]);
    let logs = s.logs();
    assert!(logs.contains("GETSTATE Running"), "{logs}");
    assert!(logs.contains("STATE Running -> Jumping") || logs.contains("STATE Running -> Freefall"), "{logs}");
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn two_tracks_blend_by_weight() {
    let dir = temp_dir("blend");
    let f = base_place(&dir);
    // WaveA raises the arm 45deg; WaveB holds identity. Equal weights
    // must land near 22.5deg: arm offset between rest and full raise.
    for (anim, raised) in [("WaveA", true), ("WaveB", false)] {
        cli_ok(&["add", &f, "--parent", "ReplicatedStorage", "--class", "Animation", "--name", anim]);
        cli_ok(&["add", &f, "--parent", &format!("ReplicatedStorage/{anim}"), "--class", "KeyframeSequence", "--name", "Seq"]);
        cli_ok(&["add", &f, "--parent", &format!("ReplicatedStorage/{anim}/Seq"), "--class", "Keyframe", "--name", "K0"]);
        cli_ok(&["add", &f, "--parent", &format!("ReplicatedStorage/{anim}/Seq/K0"), "--class", "Pose", "--name", "Right Shoulder"]);
        cli_ok(&["add", &f, "--parent", &format!("ReplicatedStorage/{anim}/Seq"), "--class", "Keyframe", "--name", "K1"]);
        cli_ok(&["set", &f, "--target", &format!("ReplicatedStorage/{anim}/Seq/K1"), "--property", "Time", "--type", "Float64", "--value", "0.5"]);
        cli_ok(&["add", &f, "--parent", &format!("ReplicatedStorage/{anim}/Seq/K1"), "--class", "Pose", "--name", "Right Shoulder"]);
        if raised {
            cli_ok(&["set", &f, "--target", &format!("ReplicatedStorage/{anim}/Seq/K1/Right Shoulder"), "--property", "CFrame",
                     "--type", "CFrame", "--value", "0,0,0,1,0,0,0,0.70710678,-0.70710678,0,0.70710678,0.70710678"]);
        }
    }
    add_script(&f, "Blend", r#"
local done = false
game:GetService("RunService").Heartbeat:Connect(function()
    if done then return end
    local char = workspace:FindFirstChild("Player")
    if not char then return end
    local animator = char:FindFirstChildOfClass("Humanoid"):FindFirstChildOfClass("Animator")
    if not animator then return end
    done = true
    local a = animator:LoadAnimation(game.ReplicatedStorage.WaveA)
    local b = animator:LoadAnimation(game.ReplicatedStorage.WaveB)
    a:Play() b:Play()
    print("BLEND weights " .. tostring(a.Weight) .. "/" .. tostring(b.Weight))
end)
"#);
    cli_ok(&["play", "start", &f]);
    let s = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "30"]);
    let tree = s.tree();
    let arm = find_node(&tree, "Right Arm").unwrap();
    let pos = &arm["properties"]["CFrame"]["value"]["position"];
    let dz = pos[2].as_f64().unwrap() - 10.0;
    assert!(dz.abs() > 0.12 && dz.abs() < 0.62, "blended arm offset {dz} between rest (0) and full raise (~0.71)");
    assert!(s.logs().contains("BLEND weights 1/1"));
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn collision_groups_can_disable_pair_collision() {
    let dir = temp_dir("groups");
    let f = base_place(&dir);
    add_script(&f, "Groups", r#"
local physics = game:GetService("PhysicsService")
physics:RegisterCollisionGroup("Ghosts")
physics:RegisterCollisionGroup("Solids")
physics:CollisionGroupSetCollidable("Ghosts", "Solids", false)
print("COLLIDABLE ghost-solid=" .. tostring(physics:CollisionGroupsAreCollidable("Ghosts", "Solids")))
local function crate(name, x, y, group)
    local p = Instance.new("Part")
    p.Name = name p.Size = Vector3.new(2, 2, 2) p.Position = Vector3.new(x, y, 0)
    p.Parent = workspace
    if group then physics:SetPartCollisionGroup(p, group) end
    return p
end
crate("SolidCrate", -10, 1, "Solids")
crate("GhostCrate", -10, 6, "Ghosts")
crate("DefaultLow", 10, 1, nil)
crate("DefaultHigh", 10, 6, nil)
"#);
    cli_ok(&["play", "start", &f]);
    let s = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "150"]);
    let tree = s.tree();
    let ghost_y = node_y(find_node(&tree, "GhostCrate").unwrap());
    let stacked_y = node_y(find_node(&tree, "DefaultHigh").unwrap());
    assert!(ghost_y < 2.0, "ghost crate fell through the solid crate to the ground, y={ghost_y}");
    assert!(stacked_y > 2.4, "default crates still stack, y={stacked_y}");
    assert!(s.logs().contains("COLLIDABLE ghost-solid=false"));
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn character_climbs_stairs() {
    let dir = temp_dir("stairs");
    let f = base_place(&dir);
    for i in 0..5 {
        let name = format!("Step{}", i + 1);
        cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", &name]);
        cli_ok(&["set", &f, "--target", &format!("Workspace/{name}"), "--property", "Size", "--value", "12,1,2"]);
        cli_ok(&["move", &f, "--target", &format!("Workspace/{name}"), "--position",
                 &format!("0,{},{}", 0.5 + i as f64, 4 - 2 * i)]);
        cli_ok(&["set", &f, "--target", &format!("Workspace/{name}"), "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    }
    cli_ok(&["play", "start", &f]);
    let s = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "60"]);
    let (_, y0, z0) = s.pos();
    cli_ok(&["play", "input", &f, "--keys", "W", "--duration", "1.0"]);
    let (_, y1, z1) = s.pos();
    assert!(z1 < z0 - 6.0, "walked forward: z {z0} -> {z1}");
    assert!(y1 > y0 + 1.5, "climbed the staircase: y {y0} -> {y1}");
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn pathfinding_routes_around_a_wall() {
    let dir = temp_dir("path");
    let f = base_place(&dir);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Wall"]);
    cli_ok(&["set", &f, "--target", "Workspace/Wall", "--property", "Size", "--value", "12,6,1"]);
    cli_ok(&["move", &f, "--target", "Workspace/Wall", "--position", "0,3,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Wall", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    add_script(&f, "Nav", r#"
local pf = game:GetService("PathfindingService")
local path = pf:CreatePath({AgentRadius = 2, AgentCanJump = false})
path:ComputeAsync(Vector3.new(0, 0.5, 12), Vector3.new(0, 0.5, -12))
local waypoints = path:GetWaypoints()
print("PATH status=" .. path.Status.Name .. " waypoints=" .. tostring(#waypoints))
local maxx = 0
for _, wp in ipairs(waypoints) do maxx = math.max(maxx, math.abs(wp.Position.X)) end
print("PATH detour maxx=" .. string.format("%.1f", maxx))
local blocked = pf:CreatePath()
blocked:ComputeAsync(Vector3.new(0, 0.5, 0), Vector3.new(0, 0.5, -12))
print("PATH inside wall status=" .. blocked.Status.Name)
"#);
    let run = cli_json(&["run", &f, "--duration", "0.2"]);
    assert_eq!(run["ok"], true, "{run}");
    let logs = format!("{}", run["logs"]);
    assert!(logs.contains("PATH status=Success"), "{logs}");
    assert!(logs.contains("PATH inside wall status=NoPath"), "{logs}");
    // The route must detour around the 12-wide wall (|x| > 6 at crossing).
    let detour = logs.split("detour maxx=").nth(1).and_then(|s| s.split(|c: char| !c.is_ascii_digit() && c != '.').next()).unwrap_or("0");
    assert!(detour.parse::<f64>().unwrap_or(0.0) > 5.0, "detours around the wall: {logs}");
}
