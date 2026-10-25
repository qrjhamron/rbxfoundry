//! Phase 3 integration tests: real headless Vulkan rendering (3A geometry
//! and cameras, 3B lighting/shadows/materials, 3C meshes/textures/assets).
//!
//! Every test renders actual PNGs through the C++ engine and probes pixels
//! from the same frame via `--probe`. Requires the engine binary and a
//! Vulkan driver (Mesa Lavapipe in CI containers).

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(
        candidate.is_file(),
        "blox-engine not found at {} — build it first: cmake -S . -B build -G Ninja && cmake --build build",
        candidate.display()
    );
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
    assert!(
        out.status.success(),
        "bloxcli {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
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
    assert!(
        out.status.success(),
        "bloxcli --json {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    serde_json::from_slice(&out.stdout).expect("json output")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-render-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Ground plane + one red block (the "asym" calibration scene).
fn build_block_scene(dir: &std::path::Path) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ground"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Size", "--value", "40,2,40"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-1,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Color", "--value", "0.35,0.75,0.35"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Block"]);
    cli_ok(&["set", &f, "--target", "Workspace/Block", "--property", "Size", "--value", "4,4,4"]);
    cli_ok(&["move", &f, "--target", "Workspace/Block", "--position", "10,2,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Block", "--property", "Color", "--value", "1,0.1,0.1"]);
    f
}

fn probe(json: &serde_json::Value, x: i64, y: i64) -> (i64, i64, i64) {
    for p in json["probes"].as_array().expect("probes array") {
        if p["x"].as_i64() == Some(x) && p["y"].as_i64() == Some(y) {
            let rgba = p["rgba"].as_array().expect("rgba");
            return (
                rgba[0].as_i64().unwrap(),
                rgba[1].as_i64().unwrap(),
                rgba[2].as_i64().unwrap(),
            );
        }
    }
    panic!("probe ({x},{y}) missing from {json}");
}

fn render_json(file: &str, out: &std::path::Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "render", file,
        "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,10,30", "--look-at", "0,1,0",
    ];
    args.extend_from_slice(extra);
    cli_json(&args)
}

fn assert_png(path: &std::path::Path, w: u32, h: u32) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("png exists");
    assert!(bytes.len() > 100, "png non-trivial ({} bytes)", bytes.len());
    assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), w, "PNG width");
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), h, "PNG height");
    bytes
}

// ---------------------------------------------------------------- 3A -----

#[test]
fn geometry_occlusion_and_sky_probes() {
    let dir = temp_dir("geom");
    let f = build_block_scene(&dir);
    let out = dir.join("a.png");
    let json = render_json(&f, &out, &["--probe", "400,100;400,400;550,300"]);
    assert_png(&out, 800, 600);
    assert_eq!(json["parts"], 2);
    let (r, g, b) = probe(&json, 400, 100);
    assert!(b > r && b > 150, "sky probe is sky-blue, got ({r},{g},{b})");
    let (r, g, b) = probe(&json, 400, 400);
    assert!(g > 150 && g > r + 50 && g > b + 50, "ground probe is lit green, got ({r},{g},{b})");
    let (r, g, b) = probe(&json, 550, 300);
    assert!(r > g + 50 && r > b + 50, "block probe is red-dominant (occludes ground), got ({r},{g},{b})");
}

#[test]
fn rendering_is_deterministic() {
    let dir = temp_dir("determinism");
    let f = build_block_scene(&dir);
    let a = dir.join("a.png");
    let b = dir.join("b.png");
    render_json(&f, &a, &[]);
    render_json(&f, &b, &[]);
    assert_eq!(assert_png(&a, 800, 600), assert_png(&b, 800, 600),
               "identical scene+camera must produce byte-identical PNGs");
}

#[test]
fn camera_move_changes_the_frame() {
    let dir = temp_dir("camera");
    let f = build_block_scene(&dir);
    let front = dir.join("front.png");
    let json_front = render_json(&f, &front, &["--probe", "550,300"]);
    let behind = dir.join("behind.png");
    let json_behind = cli_json(&[
        "render", &f, "--output", behind.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,10,-30", "--look-at", "0,1,0", "--probe", "550,300",
    ]);
    let front_px = probe(&json_front, 550, 300);
    let behind_px = probe(&json_behind, 550, 300);
    assert_ne!(front_px, behind_px, "orbiting the camera must change pixels");
    let (r, g, b) = behind_px;
    assert!(g >= r, "from behind, the probe sees ground/sky, not the red face: ({r},{g},{b})");
}

#[test]
fn orthographic_and_preset_cameras_render() {
    let dir = temp_dir("ortho");
    let f = build_block_scene(&dir);
    let out = dir.join("ortho.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "10,2,34", "--look-at", "10,2,0", "--ortho",
        "--probe", "400,300",
    ]);
    assert_png(&out, 800, 600);
    let (r, g, b) = probe(&json, 400, 300);
    assert!(r > g + 40, "orthographic centre probe hits the red block, got ({r},{g},{b})");

    let iso = dir.join("iso.png");
    let json = cli_json(&[
        "render", &f, "--output", iso.to_str().unwrap(),
        "--preset", "isometric", "--probe", "400,300",
    ]);
    assert_png(&iso, 800, 600);
    let (r, g, b) = probe(&json, 400, 300);
    assert!(g > r, "isometric framing centres on the scene (ground), got ({r},{g},{b})");
}

#[test]
fn camera_command_reports_scene_bounds() {
    let dir = temp_dir("cameracmd");
    let f = build_block_scene(&dir);
    let json = cli_json(&["camera", &f]);
    assert_eq!(json["parts"], 2);
    let min = json["sceneBounds"]["min"].as_array().expect("bounds min");
    let max = json["sceneBounds"]["max"].as_array().expect("bounds max");
    assert!(min[0].as_f64().unwrap() <= -20.0 + 0.01, "ground reaches x=-20: {min:?}");
    assert!(max[0].as_f64().unwrap() >= 20.0 - 0.01, "ground reaches x=+20: {max:?}");
}

// ---------------------------------------------------------------- 3B -----

#[test]
fn shadows_darken_the_ground_and_can_be_disabled() {
    let dir = temp_dir("shadow");
    let f = build_block_scene(&dir);
    let on = dir.join("on.png");
    let json_on = render_json(&f, &on, &["--probe", "600,335"]);
    let off = dir.join("off.png");
    let json_off = render_json(&f, &off, &["--no-shadows", "--probe", "600,335"]);
    let (_, g_on, _) = probe(&json_on, 600, 335);
    let (_, g_off, _) = probe(&json_off, 600, 335);
    assert!(g_off - g_on >= 25,
            "block shadow darkens the ground (on={g_on}, off={g_off})");
}

#[test]
fn time_of_day_changes_lighting() {
    let dir = temp_dir("tod");
    let f = build_block_scene(&dir);
    let day = dir.join("day.png");
    let json_day = render_json(&f, &day, &["--probe", "400,400;400,100"]);
    cli_ok(&["set", &f, "--target", "Lighting", "--property", "ClockTime", "--type", "Float64", "--value", "0"]);
    let night = dir.join("night.png");
    let json_night = render_json(&f, &night, &["--probe", "400,400;400,100"]);
    let lum = |p: (i64, i64, i64)| p.0 + p.1 + p.2;
    assert!(lum(probe(&json_day, 400, 400)) > lum(probe(&json_night, 400, 400)) + 120,
            "midday ground is much brighter than midnight ground");
    assert!(lum(probe(&json_day, 400, 100)) > lum(probe(&json_night, 400, 100)) + 150,
            "midday sky is much brighter than midnight sky");
}

#[test]
fn transparency_blends_with_what_is_behind() {
    let dir = temp_dir("glass");
    let f = build_block_scene(&dir);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Glass"]);
    cli_ok(&["set", &f, "--target", "Workspace/Glass", "--property", "Size", "--value", "8,8,0.5"]);
    cli_ok(&["move", &f, "--target", "Workspace/Glass", "--position", "10,2,8"]);
    cli_ok(&["set", &f, "--target", "Workspace/Glass", "--property", "Color", "--value", "0.2,0.4,1"]);
    cli_ok(&["set", &f, "--target", "Workspace/Glass", "--property", "Transparency", "--type", "Float64", "--value", "0.5"]);
    let out = dir.join("glass.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "10,3,30", "--look-at", "10,2,0", "--probe", "400,300",
    ]);
    let (r, g, b) = probe(&json, 400, 300);
    assert!(b >= 60 && r >= 50,
            "blue glass over red block blends (neither pure), got ({r},{g},{b})");
}

// ---------------------------------------------------------------- 3C -----

fn build_mesh_scene(dir: &std::path::Path, texture: &str) -> String {
    let file = dir.join("mesh.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "MeshPart", "--name", "Quad"]);
    cli_ok(&["set", &f, "--target", "Workspace/Quad", "--property", "MeshId", "--type", "ContentId", "--value", "quad_v2.mesh"]);
    cli_ok(&["set", &f, "--target", "Workspace/Quad", "--property", "TextureID", "--type", "ContentId", "--value", texture]);
    cli_ok(&["set", &f, "--target", "Workspace/Quad", "--property", "Size", "--value", "8,8,1"]);
    f
}

#[test]
fn mesh_texture_uv_orientation() {
    let dir = temp_dir("uv");
    let f = build_mesh_scene(&dir, "halves.png");
    let out = dir.join("uv.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,0,16", "--look-at", "0,0,0",
        "--assets", fixtures_dir().to_str().unwrap(),
        "--probe", "400,220;400,380",
    ]);
    assert_png(&out, 800, 600);
    assert_eq!(json["parts"], 1);
    // halves.png: top half red, bottom half blue (Roblox UV origin bottom-left).
    let (r, g, b) = probe(&json, 400, 220);
    assert!(r >= 40 && r > b + 25, "top of the quad shows the texture's top (red), got ({r},{g},{b})");
    let (r, g, b) = probe(&json, 400, 380);
    assert!(b >= 40 && b > r + 25, "bottom of the quad shows the texture's bottom (blue), got ({r},{g},{b})");
}

#[test]
fn checker_texture_is_not_stretched_to_one_colour() {
    let dir = temp_dir("checker");
    let f = build_mesh_scene(&dir, "checker.png");
    let out = dir.join("checker.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,0,16", "--look-at", "0,0,0",
        "--assets", fixtures_dir().to_str().unwrap(),
        "--probe", "320,220;373,220;480,380",
    ]);
    let lum = |p: (i64, i64, i64)| p.0 + p.1 + p.2;
    let a = lum(probe(&json, 320, 220));
    let b = lum(probe(&json, 373, 220));
    let diag = lum(probe(&json, 480, 380));
    assert!((a - b).abs() > 60, "adjacent checker cells differ (a={a}, b={b})");
    assert!((a - diag).abs() < 40, "diagonal checker cells match (a={a}, diag={diag})");
}

#[test]
fn missing_mesh_asset_is_reported_not_faked() {
    let dir = temp_dir("missing");
    let file = dir.join("missing.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "MeshPart", "--name", "Ghost"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ghost", "--property", "MeshId", "--type", "ContentId", "--value", "rbxassetid://424242"]);
    let out = dir.join("missing.png");
    let json = render_json(&f, &out, &[]);
    let missing = json["missingAssets"].as_array().expect("missingAssets array");
    assert!(
        missing.iter().any(|m| m.as_str().unwrap_or("").contains("424242")),
        "missing asset id is reported explicitly: {missing:?}"
    );
    assert_eq!(json["parts"], 0, "no fake geometry substituted for the missing mesh");
}

#[test]
fn visual_regression_indoor_room_and_castle() {
    // Indoor Room: enclosed box room, camera inside looking at the far wall.
    let dir = temp_dir("room");
    let file = dir.join("room.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    let room_part = |f: &str, name: &str, size: &str, pos: &str, color: &str| {
        cli_ok(&["add", f, "--parent", "Workspace", "--class", "Part", "--name", name]);
        cli_ok(&["set", f, "--target", &format!("Workspace/{name}"), "--property", "Size", "--value", size]);
        cli_ok(&["move", f, "--target", &format!("Workspace/{name}"), "--position", pos]);
        cli_ok(&["set", f, "--target", &format!("Workspace/{name}"), "--property", "Color", "--value", color]);
    };
    room_part(&f, "Floor", "20,1,20", "0,-0.5,0", "0.6,0.5,0.4");
    room_part(&f, "Ceiling", "20,1,20", "0,8.5,0", "0.9,0.9,0.9");
    room_part(&f, "BackWall", "20,9,1", "0,4,-10", "0.3,0.4,0.9");
    room_part(&f, "FrontWall", "20,9,1", "0,4,10", "0.3,0.4,0.9");
    room_part(&f, "LeftWall", "1,9,20", "-10,4,0", "0.85,0.8,0.7");
    room_part(&f, "RightWall", "1,9,20", "10,4,0", "0.85,0.8,0.7");
    let out = dir.join("room.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,3.5,6", "--look-at", "0,3.5,-10", "--probe", "400,300;400,560",
    ]);
    assert_png(&out, 800, 600);
    let (r, g, b) = probe(&json, 400, 300);
    assert!(b > r + 20 && b > g, "room centre probe is the blue back wall, got ({r},{g},{b})");
    let (r, g, _b) = probe(&json, 400, 560);
    assert!(r > 60 && g > 40, "room floor probe is warm wood colour, got r={r} g={g}");

    // Castle Exterior: keep + two cylinder towers + wedge roofs.
    let dir = temp_dir("castle");
    let file = dir.join("castle.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    room_part(&f, "Keep", "10,14,10", "0,7,0", "0.62,0.6,0.58");
    room_part(&f, "TowerL", "20,6,6", "-11,10,0", "0.62,0.6,0.58");
    cli_ok(&["set", &f, "--target", "Workspace/TowerL", "--property", "Shape", "--type", "Enum", "--value", "2"]);
    room_part(&f, "TowerR", "20,6,6", "11,10,0", "0.62,0.6,0.58");
    cli_ok(&["set", &f, "--target", "Workspace/TowerR", "--property", "Shape", "--type", "Enum", "--value", "2"]);
    // Roblox cylinders lie along X; rotate them into standing towers.
    cli_ok(&["rotate", &f, "--target", "Workspace/TowerL", "--angles", "0,0,90"]);
    cli_ok(&["rotate", &f, "--target", "Workspace/TowerR", "--angles", "0,0,90"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "WedgePart", "--name", "Roof"]);
    cli_ok(&["set", &f, "--target", "Workspace/Roof", "--property", "Size", "--value", "12,5,12"]);
    cli_ok(&["move", &f, "--target", "Workspace/Roof", "--position", "0,16.5,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Roof", "--property", "Color", "--value", "0.7,0.2,0.15"]);
    let out = dir.join("castle.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "34,20,42", "--look-at", "0,8,0", "--probe", "400,300",
    ]);
    assert_png(&out, 800, 600);
    assert_eq!(json["parts"], 4);
    let (r, g, b) = probe(&json, 400, 300);
    assert!((r - g).abs() < 45 && r > 35 && b < 120, "castle keep probe is stone grey, got ({r},{g},{b})");
}

#[test]
fn materials_change_shading_neon_glows_at_night() {
    let dir = temp_dir("materials");
    let file = dir.join("mats.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    for (name, x, material) in [("PlasticBlock", "-6", "256"), ("NeonBlock", "6", "288")] {
        cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", name]);
        cli_ok(&["set", &f, "--target", &format!("Workspace/{name}"), "--property", "Size", "--value", "6,6,6"]);
        cli_ok(&["move", &f, "--target", &format!("Workspace/{name}"), "--position", &format!("{x},3,-6")]);
        cli_ok(&["set", &f, "--target", &format!("Workspace/{name}"), "--property", "Color", "--value", "0.9,0.3,0.2"]);
        cli_ok(&["set", &f, "--target", &format!("Workspace/{name}"), "--property", "Material", "--type", "Enum", "--value", material]);
    }
    cli_ok(&["set", &f, "--target", "Lighting", "--property", "ClockTime", "--type", "Float64", "--value", "0"]);
    let out = dir.join("mats.png");
    // Camera centred between the blocks: plastic at screen-left, neon right.
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,4,16", "--look-at", "0,3,-6", "--probe", "290,300;510,300",
    ]);
    let lum = |p: (i64, i64, i64)| p.0 + p.1 + p.2;
    let plastic = lum(probe(&json, 290, 300));
    let neon = lum(probe(&json, 510, 300));
    assert!(neon > plastic + 120,
            "Neon material stays emissive-bright at night while Plastic goes dark (neon={neon}, plastic={plastic})");
}

#[test]
fn quality_presets_render() {
    let dir = temp_dir("quality");
    let f = build_block_scene(&dir);
    for q in ["low", "medium", "high"] {
        let out = dir.join(format!("{q}.png"));
        let json = cli_json(&[
            "render", &f, "--output", out.to_str().unwrap(), "--quality", q,
            "--width", "640", "--height", "480", "--camera", "0,10,30", "--look-at", "0,1,0",
        ]);
        assert_eq!(json["width"], 640, "quality {q} renders at requested size");
        assert_png(&out, 640, 480);
    }
}

#[test]
fn mesh_survives_rbxl_round_trip() {
    let dir = temp_dir("meshrt");
    let f = build_mesh_scene(&dir, "halves.png");
    // Force a rewrite through the serializer, then re-render from the file.
    cli_ok(&["save", &f]);
    let json = cli_json(&["inspect", &f]);
    let text = serde_json::to_string(&json).unwrap();
    assert!(text.contains("quad_v2.mesh"), "MeshId preserved through RBXL round-trip");
    let out = dir.join("rt.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,0,16", "--look-at", "0,0,0",
        "--assets", fixtures_dir().to_str().unwrap(),
        "--probe", "400,220",
    ]);
    let (r, _g, b) = probe(&json, 400, 220);
    assert!(r > b + 25, "mesh still renders textured after round-trip");
}

#[test]
fn mesh_v5_renders_and_skinned_mesh_warns() {
    let dir = temp_dir("meshv5");
    let file = dir.join("v5.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "MeshPart", "--name", "Cube"]);
    cli_ok(&["set", &f, "--target", "Workspace/Cube", "--property", "MeshId", "--type", "ContentId", "--value", "cube_v5.mesh"]);
    cli_ok(&["set", &f, "--target", "Workspace/Cube", "--property", "Size", "--value", "6,6,6"]);
    cli_ok(&["set", &f, "--target", "Workspace/Cube", "--property", "Color", "--value", "0.9,0.5,0.2"]);
    let out = dir.join("v5.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "9,7,12", "--look-at", "0,0,0",
        "--assets", fixtures_dir().to_str().unwrap(),
        "--probe", "400,300",
    ]);
    assert_png(&out, 800, 600);
    assert_eq!(json["parts"], 1, "v5 mesh geometry renders");
    let (r, g, _b) = probe(&json, 400, 300);
    assert!(r > g + 20, "v5 cube shows its orange colour, got ({r},{g})");

    // Skinned v5 renders unskinned but says so explicitly.
    cli_ok(&["set", &f, "--target", "Workspace/Cube", "--property", "MeshId", "--type", "ContentId", "--value", "cube_v5_skinned.mesh"]);
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "9,7,12", "--look-at", "0,0,0",
        "--assets", fixtures_dir().to_str().unwrap(),
    ]);
    let warnings = json["warnings"].as_array().expect("warnings array");
    assert!(warnings.iter().any(|w| w.as_str().unwrap_or("").contains("skinning")),
            "skinned mesh reports the skinning limitation: {warnings:?}");
}

#[test]
fn surface_appearance_colormap_textures_the_mesh() {
    let dir = temp_dir("surfapp");
    let file = dir.join("sa.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "MeshPart", "--name", "Quad"]);
    cli_ok(&["set", &f, "--target", "Workspace/Quad", "--property", "MeshId", "--type", "ContentId", "--value", "quad_v2.mesh"]);
    cli_ok(&["set", &f, "--target", "Workspace/Quad", "--property", "Size", "--value", "8,8,1"]);
    cli_ok(&["add", &f, "--parent", "Workspace/Quad", "--class", "SurfaceAppearance", "--name", "SA"]);
    cli_ok(&["set", &f, "--target", "Workspace/Quad/SA", "--property", "ColorMap", "--type", "ContentId", "--value", "halves.png"]);
    let out = dir.join("sa.png");
    let json = cli_json(&[
        "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,0,16", "--look-at", "0,0,0",
        "--assets", fixtures_dir().to_str().unwrap(),
        "--probe", "400,220;400,380",
    ]);
    let (r, _g, b) = probe(&json, 400, 220);
    assert!(r > b + 25, "SurfaceAppearance ColorMap textures the top red, got ({r},{b})");
    let (r, _g, b) = probe(&json, 400, 380);
    assert!(b > r + 25, "SurfaceAppearance ColorMap textures the bottom blue, got ({r},{b})");
}
