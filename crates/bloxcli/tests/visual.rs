//! Phase 11 integration tests: terrain voxels (storage, rendering,
//! collision, Luau API), dynamic lights, SurfaceAppearance PBR maps,
//! Beam/ParticleEmitter rendering and TextService measurement.
//! All assertions use real renders, physics state and Luau logs.

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
    let dir = std::env::temp_dir().join(format!("bloxcli-visual-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn probe(json: &serde_json::Value, x: i64, y: i64) -> (i64, i64, i64) {
    for p in json["probes"].as_array().expect("probes array") {
        if p["x"].as_i64() == Some(x) && p["y"].as_i64() == Some(y) {
            let rgba = p["rgba"].as_array().expect("rgba");
            return (rgba[0].as_i64().unwrap(), rgba[1].as_i64().unwrap(), rgba[2].as_i64().unwrap());
        }
    }
    panic!("probe ({x},{y}) missing from {json}");
}

struct Session(PathBuf);
impl Session {
    fn state(&self) -> serde_json::Value {
        cli_json(&["play", "state", self.0.to_str().unwrap()])["state"].clone()
    }
    fn logs(&self) -> String {
        let value = cli_json(&["play", "logs", self.0.to_str().unwrap()]);
        value["logs"].as_array().map(|arr| {
            arr.iter().map(|e| e["message"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join("\n")
        }).unwrap_or_default()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = cli(&["play", "stop", self.0.to_str().unwrap()]);
    }
}

#[test]
fn terrain_fill_renders_collides_and_round_trips() {
    let dir = temp_dir("terrain");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    let filled = cli_json(&["terrain", "fill", &f, "--center", "0,2,0", "--size", "32,4,32", "--material", "Grass"]);
    assert_eq!(filled["result"]["voxels"], 64, "{filled}");
    let info = cli_json(&["terrain", "info", &f]);
    assert_eq!(info["result"]["voxels"], 64);
    assert_eq!(info["result"]["materialsByEnumValue"]["1280"], 64);

    // The voxel payload survives a round trip through the file.
    let tree = cli_json(&["tree", &f]);
    assert!(format!("{}", tree["tree"]).contains("BloxVoxels"), "voxels stored on Terrain");

    // Terrain renders as green voxel geometry.
    let out = dir.join("terrain.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,26,34", "--look-at", "0,2,0",
        "--probe", "400,330",
    ]);
    let (r, g, b) = probe(&json, 400, 330);
    assert!(g > r + 15 && g > b + 10, "terrain grass colour at centre, got ({r},{g},{b})");

    // Character physics stands on terrain (no Part ground exists).
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "SpawnLocation", "--name", "Spawn"]);
    cli_ok(&["move", &f, "--target", "Workspace/Spawn", "--position", "0,4.5,8"]);
    cli_ok(&["set", &f, "--target", "Workspace/Spawn", "--property", "CanCollide", "--type", "Bool", "--value", "false"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Dig"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Dig", "--property", "Source", "--type", "String",
             "--value", "local t = workspace:WaitForChild('Terrain')\nt:FillBall(Vector3.new(16, 6, 16), 5, Enum.Material.Sand)\nlocal mats, occ = t:ReadVoxels(CFrame.new(16, 6, 16), Vector3.new(4, 4, 4))\nprint('VOXEL occ=' .. string.format('%.2f', occ[1][1][1]))\nprint('VOXEL mat=' .. tostring(mats[1][1][1].Name))"]);
    cli_ok(&["play", "start", &f]);
    let session = Session(PathBuf::from(&f));
    cli_ok(&["play", "step", &f, "--steps", "90"]);
    let state = session.state();
    assert_eq!(state["character"]["grounded"], true, "character grounded on terrain: {state}");
    let y = state["character"]["position"][1].as_f64().unwrap();
    assert!(y > 5.5 && y < 7.5, "character stands on the 4-stud terrain top, y={y}");
    let logs = session.logs();
    assert!(logs.contains("VOXEL occ=1.00"), "{logs}");
    assert!(logs.contains("VOXEL mat=Sand"), "{logs}");
    cli_ok(&["play", "stop", &f]);
}

#[test]
fn point_light_illuminates_dark_scene() {
    let dir = temp_dir("lights");
    let build = |with_light: bool| -> String {
        let file = dir.join(if with_light { "lit.rbxl" } else { "dark.rbxl" });
        let f = file.to_str().unwrap().to_string();
        cli_ok(&["new", &f]);
        cli_ok(&["set", &f, "--target", "Lighting", "--property", "ClockTime", "--type", "Float64", "--value", "0"]);
        cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Floor"]);
        cli_ok(&["set", &f, "--target", "Workspace/Floor", "--property", "Size", "--value", "30,1,30"]);
        cli_ok(&["move", &f, "--target", "Workspace/Floor", "--position", "0,-0.5,0"]);
        cli_ok(&["set", &f, "--target", "Workspace/Floor", "--property", "Color", "--value", "0.9,0.9,0.9"]);
        cli_ok(&["set", &f, "--target", "Workspace/Floor", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
        if with_light {
            cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Lamp"]);
            cli_ok(&["set", &f, "--target", "Workspace/Lamp", "--property", "Size", "--value", "1,1,1"]);
            cli_ok(&["move", &f, "--target", "Workspace/Lamp", "--position", "0,5,0"]);
            cli_ok(&["add", &f, "--parent", "Workspace/Lamp", "--class", "PointLight", "--name", "Glow"]);
            cli_ok(&["set", &f, "--target", "Workspace/Lamp/Glow", "--property", "Color", "--value", "1,0.2,0.1"]);
            cli_ok(&["set", &f, "--target", "Workspace/Lamp/Glow", "--property", "Range", "--type", "Float64", "--value", "30"]);
            cli_ok(&["set", &f, "--target", "Workspace/Lamp/Glow", "--property", "Brightness", "--type", "Float64", "--value", "2"]);
        }
        f
    };
    let render_probe = |f: &str| -> (i64, i64, i64) {
        let out = dir.join("shot.png");
        let json = cli_json(&[
            "render", f, "--output", out.to_str().unwrap(),
            "--width", "800", "--height", "600",
            "--camera", "0,14,18", "--look-at", "0,0,0",
            "--probe", "400,330",
        ]);
        probe(&json, 400, 330)
    };
    let dark = render_probe(&build(false));
    let lit = render_probe(&build(true));
    assert!(lit.0 > dark.0 + 40, "point light brightens the floor: dark={dark:?} lit={lit:?}");
    assert!(lit.0 > lit.2 + 20, "point light colour tints the floor red: {lit:?}");
}

#[test]
fn surface_appearance_pbr_maps_affect_rendering() {
    let dir = temp_dir("pbr");
    let file = dir.join("pbr.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "MeshPart", "--name", "Cube"]);
    cli_ok(&["set", &f, "--target", "Workspace/Cube", "--property", "MeshId", "--type", "ContentId", "--value", "cube_v5.mesh"]);
    cli_ok(&["set", &f, "--target", "Workspace/Cube", "--property", "Size", "--value", "6,6,6"]);
    cli_ok(&["move", &f, "--target", "Workspace/Cube", "--position", "0,3,0"]);
    cli_ok(&["add", &f, "--parent", "Workspace/Cube", "--class", "SurfaceAppearance", "--name", "SA"]);
    for (prop, val) in [("ColorMap", "halves.png"), ("NormalMap", "checker.png"),
                        ("RoughnessMap", "checker.png"), ("MetalnessMap", "checker.png")] {
        cli_ok(&["set", &f, "--target", "Workspace/Cube/SA", "--property", prop, "--type", "ContentId", "--value", val]);
    }
    let render = |f: &str| -> serde_json::Value {
        let out = dir.join("pbr.png");
        cli_json(&[
            "render", f, "--output", out.to_str().unwrap(),
            "--width", "800", "--height", "600",
            "--camera", "7,6,9", "--look-at", "0,3,0",
            "--assets", fixtures_dir().to_str().unwrap(),
            "--probe", "400,250;400,300;400,350;300,300;500,300;350,275",
        ])
    };
    let with_maps = render(&f);
    // Same scene without the extra maps must render differently.
    for prop in ["NormalMap", "RoughnessMap", "MetalnessMap"] {
        cli_ok(&["set", &f, "--target", "Workspace/Cube/SA", "--property", prop, "--type", "ContentId", "--value", ""]);
    }
    let without_maps = render(&f);
    let mut max_diff = 0i64;
    for (x, y) in [(400, 250), (400, 300), (400, 350), (300, 300), (500, 300), (350, 275)] {
        let a = probe(&with_maps, x, y);
        let b = probe(&without_maps, x, y);
        max_diff = max_diff.max((a.0 - b.0).abs()).max((a.1 - b.1).abs()).max((a.2 - b.2).abs());
    }
    assert!(max_diff > 8, "PBR maps change the rendered pixels (max channel diff {max_diff})");
    // A missing map is reported, never faked.
    cli_ok(&["set", &f, "--target", "Workspace/Cube/SA", "--property", "NormalMap", "--type", "ContentId", "--value", "nope_normal.png"]);
    let missing = render(&f);
    let missing_text = format!("{}", missing["missingAssets"]);
    assert!(missing_text.contains("nope_normal.png"), "missing map reported: {missing_text}");
}

#[test]
fn beam_and_particles_render() {
    let dir = temp_dir("beamfx");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ground"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Size", "--value", "60,2,60"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-1,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    // Author attachments/beam/emitter through real Luau, committed back.
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Build"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Build", "--property", "Source", "--type", "String", "--value", r#"
local function post(name, x)
    local p = Instance.new("Part")
    p.Name = name p.Size = Vector3.new(1, 8, 1) p.Position = Vector3.new(x, 4, 0)
    p.Anchored = true p.Parent = workspace
    local a = Instance.new("Attachment") a.Parent = p
    return a
end
local a0 = post("PostA", -8)
local a1 = post("PostB", 8)
local beam = Instance.new("Beam")
beam.Attachment0 = a0 beam.Attachment1 = a1 beam.Parent = workspace
local emitter = Instance.new("ParticleEmitter")
emitter.Rate = 40 emitter.Parent = workspace.PostA
"#]);
    let run = cli_json(&["run", &f, "--duration", "0.2", "--commit"]);
    assert_eq!(run["ok"], true, "{run}");
    let out = dir.join("beam.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,5,22", "--look-at", "0,4,0",
        "--probe", "400,270;400,300",
    ]);
    let beam_px = probe(&json, 400, 270);
    let total: i64 = beam_px.0 + beam_px.1 + beam_px.2;
    assert!(total > 240, "beam visible across the frame centre: {beam_px:?}");
    assert!(json["drawCalls"].as_i64().unwrap_or(0) > 10, "particles add draw calls: {json}");
}

#[test]
fn text_service_measures_and_wraps() {
    let dir = temp_dir("text");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value", r#"
local ts = game:GetService("TextService")
local wide = ts:GetTextSize("Hello world from BloxCLI", 20, Enum.Font.Gotham, Vector2.new(10000, 100))
local narrow = ts:GetTextSize("Hello world from BloxCLI", 20, Enum.Font.Gotham, Vector2.new(70, 1000))
print(string.format("TEXT wide=%.0fx%.0f narrow=%.0fx%.0f", wide.X, wide.Y, narrow.X, narrow.Y))
assert(wide.X > 100, "text has real width")
assert(narrow.Y > wide.Y, "narrow frame wraps to more lines")
"#]);
    let run = cli_json(&["run", &f, "--duration", "0.2"]);
    assert_eq!(run["ok"], true, "{run}");
    let logs = format!("{}", run["logs"]);
    assert!(logs.contains("TEXT wide="), "{logs}");
}
