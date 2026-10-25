//! Phase 4 integration tests: Roblox GUI layout, rendering, inspection,
//! validation and hit-testing through the real C++ UI engine.
//!
//! Layout expectations are the exact values verified against the engine:
//! at 800x600 the default (inset) ScreenGui occupies (0,36,800,564).

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
    let dir = std::env::temp_dir().join(format!("bloxcli-ui-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// ScreenGui "HUD" with a centred Panel, a Title label and an Accept button.
fn build_hud(dir: &std::path::Path) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "HUD"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD", "--class", "Frame", "--name", "Panel"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel", "--property", "Position", "--type", "UDim2", "--value", "0.5,0,0.5,0"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel", "--property", "Size", "--type", "UDim2", "--value", "0,320,0,180"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel", "--property", "AnchorPoint", "--type", "Vector2", "--value", "0.5,0.5"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel", "--property", "BackgroundColor3", "--type", "Color3", "--value", "0.13,0.15,0.22"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD/Panel", "--class", "TextLabel", "--name", "Title"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Title", "--property", "Size", "--type", "UDim2", "--value", "1,0,0,40"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Title", "--property", "Text", "--type", "String", "--value", "Quest: Slay the Dragon"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Title", "--property", "TextColor3", "--type", "Color3", "--value", "1,0.9,0.4"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Title", "--property", "BackgroundTransparency", "--type", "Float64", "--value", "1"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD/Panel", "--class", "TextButton", "--name", "Accept"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Accept", "--property", "Position", "--type", "UDim2", "--value", "0.5,0,1,-60"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Accept", "--property", "Size", "--type", "UDim2", "--value", "0,140,0,44"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Accept", "--property", "AnchorPoint", "--type", "Vector2", "--value", "0.5,0"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Accept", "--property", "BackgroundColor3", "--type", "Color3", "--value", "0.2,0.75,0.3"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Panel/Accept", "--property", "Text", "--type", "String", "--value", "Accept"]);
    f
}

fn rect(json: &serde_json::Value, path_suffix: &str) -> serde_json::Value {
    json["rects"]
        .as_array()
        .expect("rects array")
        .iter()
        .find(|r| r["path"].as_str().unwrap_or("").ends_with(path_suffix))
        .unwrap_or_else(|| panic!("rect ending '{path_suffix}' missing from {json}"))
        .clone()
}

fn rect_xy(r: &serde_json::Value) -> (f64, f64, f64, f64) {
    (
        r["x"].as_f64().unwrap(),
        r["y"].as_f64().unwrap(),
        r["width"].as_f64().unwrap(),
        r["height"].as_f64().unwrap(),
    )
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

#[test]
fn layout_positions_sizes_and_anchor_points() {
    let dir = temp_dir("layout");
    let f = build_hud(&dir);
    let json = cli_json(&["ui", "inspect", &f, "--width", "800", "--height", "600"]);
    let (x, y, w, h) = rect_xy(&rect(&json, "StarterGui/HUD"));
    assert_eq!((x, y, w, h), (0.0, 36.0, 800.0, 564.0), "ScreenGui gets the default 36px top inset");
    let (x, y, w, h) = rect_xy(&rect(&json, "/Panel"));
    assert_eq!((x, y, w, h), (240.0, 228.0, 320.0, 180.0),
               "centred AnchorPoint (0.5,0.5) Panel: got ({x},{y},{w},{h})");
    let (x, y, w, h) = rect_xy(&rect(&json, "/Accept"));
    assert_eq!((x, y, w, h), (330.0, 348.0, 140.0, 44.0),
               "bottom-centred Accept button: got ({x},{y},{w},{h})");
    let panel = rect(&json, "/Panel");
    let accept = rect(&json, "/Accept");
    assert!(accept["order"].as_i64().unwrap() > panel["order"].as_i64().unwrap(),
            "children render after (above) their parent");
}

#[test]
fn ui_render_probes_panel_button_and_rasterised_text() {
    let dir = temp_dir("render");
    let f = build_hud(&dir);
    let out = dir.join("hud.png");
    // Probe the panel, the button, and a sweep across the Title text band.
    let mut probes = vec!["400,300".to_string(), "400,370".to_string()];
    for x in (252..560).step_by(8) {
        probes.push(format!("{x},247"));
    }
    let probe_arg = probes.join(";");
    let json = cli_json(&[
        "ui", "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600", "--probe", &probe_arg,
    ]);
    let bytes = std::fs::read(&out).expect("ui png exists");
    assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 800);
    assert!(json["quadCount"].as_i64().unwrap() > 10, "backgrounds + glyphs emit quads");
    let (r, g, b) = probe(&json, 400, 300);
    assert!(b > r && r < 90 && b < 110, "panel colour probe, got ({r},{g},{b})");
    let (r, g, b) = probe(&json, 400, 370);
    assert!(g > 120 && g > r + 50, "button colour probe, got ({r},{g},{b})");
    // Real rasterised glyphs: some probes in the Title band must hit the
    // yellow text colour (255,230,102).
    let mut yellow = 0;
    for p in json["probes"].as_array().unwrap() {
        if p["y"].as_i64() == Some(247) {
            let rgba = p["rgba"].as_array().unwrap();
            let (r, g, b) = (rgba[0].as_i64().unwrap(), rgba[1].as_i64().unwrap(), rgba[2].as_i64().unwrap());
            if r > 170 && g > 140 && b < 180 {
                yellow += 1;
            }
        }
    }
    assert!(yellow >= 3, "Title glyphs rasterised (found {yellow} text-coloured probes)");
}

#[test]
fn hit_test_finds_topmost_and_misses_empty_space() {
    let dir = temp_dir("hit");
    let f = build_hud(&dir);
    let json = cli_json(&["ui", "hit-test", &f, "--x", "400", "--y", "370", "--width", "800", "--height", "600"]);
    assert_eq!(json["hit"], true);
    assert!(json["instance"]["path"].as_str().unwrap().ends_with("/Accept"),
            "button is the topmost hit: {json}");
    assert_eq!(json["simulated"], true, "hit-testing is explicitly simulation, not script execution");
    // Below the Panel (which ends at y=408) only the ScreenGui remains,
    // and containers are not hit targets.
    let json = cli_json(&["ui", "hit-test", &f, "--x", "400", "--y", "480", "--width", "800", "--height", "600"]);
    assert_eq!(json["hit"], false);
    assert!(json["instance"].is_null());
}

#[test]
fn validate_flags_fully_offscreen_elements() {
    let dir = temp_dir("validate");
    let f = build_hud(&dir);
    let ok = cli_json(&["ui", "validate", &f, "--width", "800", "--height", "600"]);
    assert_eq!(ok["valid"], true, "clean HUD validates: {ok}");
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD", "--class", "Frame", "--name", "Lost"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Lost", "--property", "Position", "--type", "UDim2", "--value", "0,900,0,900"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Lost", "--property", "Size", "--type", "UDim2", "--value", "0,50,0,50"]);
    let out = cli(&["--json", "ui", "validate", &f, "--width", "800", "--height", "600"]);
    assert!(!out.status.success(), "offscreen element must fail validation");
    let text = String::from_utf8_lossy(&out.stdout).to_string()
        + &String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("offscreen") || text.contains("Offscreen"),
            "failure explains the offscreen element: {text}");
}

#[test]
fn list_layout_stacks_children() {
    let dir = temp_dir("list");
    let f = build_hud(&dir);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD", "--class", "Frame", "--name", "Menu"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Menu", "--property", "Size", "--type", "UDim2", "--value", "0,200,0,200"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD/Menu", "--class", "UIListLayout", "--name", "Layout"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD/Menu", "--class", "Frame", "--name", "RowA"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Menu/RowA", "--property", "Size", "--type", "UDim2", "--value", "1,0,0,50"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD/Menu", "--class", "Frame", "--name", "RowB"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/Menu/RowB", "--property", "Size", "--type", "UDim2", "--value", "1,0,0,50"]);
    let json = cli_json(&["ui", "inspect", &f, "--width", "800", "--height", "600"]);
    let a = rect(&json, "/RowA");
    let b = rect(&json, "/RowB");
    let dy = b["y"].as_f64().unwrap() - a["y"].as_f64().unwrap();
    assert!((dy - 50.0).abs() <= 1.0, "UIListLayout stacks rows 50px apart, got dy={dy}");
    assert!((b["x"].as_f64().unwrap() - a["x"].as_f64().unwrap()).abs() <= 1.0, "rows share x");
}

#[test]
fn device_presets_change_viewport() {
    let dir = temp_dir("device");
    let f = build_hud(&dir);
    for (device, w, h) in [("mobile", 390, 844), ("laptop", 1366, 768), ("desktop", 1920, 1080)] {
        let out = dir.join(format!("{device}.png"));
        cli_ok(&["ui", "render", &f, "--output", out.to_str().unwrap(), "--device", device]);
        let bytes = std::fs::read(&out).expect("device png");
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), w, "{device} width");
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), h, "{device} height");
    }
    // Layout is responsive: the centred Panel moves with the viewport.
    let json = cli_json(&["ui", "inspect", &f, "--device", "mobile"]);
    let (x, _y, w, _h) = rect_xy(&rect(&json, "/Panel"));
    assert!((x - (390.0 - 320.0) / 2.0).abs() <= 1.0 && (w - 320.0).abs() <= 1.0,
            "panel re-centres on mobile width, got x={x} w={w}");
}

#[test]
fn surface_gui_text_label_renders_onto_part() {
    let dir = temp_dir("surface");
    // Dedicated place without a ScreenGui, so the probe sees only the sign.
    let file = dir.join("sign.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Sign"]);
    cli_ok(&["set", &f, "--target", "Workspace/Sign", "--property", "Size", "--value", "10,6,1"]);
    cli_ok(&["move", &f, "--target", "Workspace/Sign", "--position", "0,3,-5"]);
    cli_ok(&["set", &f, "--target", "Workspace/Sign", "--property", "Color", "--value", "1,1,1"]);
    cli_ok(&["add", &f, "--parent", "Workspace/Sign", "--class", "SurfaceGui", "--name", "Face"]);
    cli_ok(&["add", &f, "--parent", "Workspace/Sign/Face", "--class", "TextLabel", "--name", "Label"]);
    cli_ok(&["set", &f, "--target", "Workspace/Sign/Face/Label", "--property", "Size", "--type", "UDim2", "--value", "1,0,1,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Sign/Face/Label", "--property", "BackgroundColor3", "--type", "Color3", "--value", "0.8,0.1,0.1"]);
    cli_ok(&["set", &f, "--target", "Workspace/Sign/Face/Label", "--property", "Text", "--type", "String", "--value", "Spawn"]);
    let out = dir.join("sign.png");
    let json = cli_json(&[
        "ui", "render", &f, "--output", out.to_str().unwrap(),
        "--width", "800", "--height", "600",
        "--camera", "0,3,10", "--look-at", "0,3,-5",
        "--with-scene", "--probe", "400,300",
    ]);
    let (r, g, b) = probe(&json, 400, 300);
    assert!(r > g + 50 && r > b + 50,
            "SurfaceGui label texture visible on the part's front face, got ({r},{g},{b})");
}

#[test]
fn ui_screenshot_alias_and_inspect_json_shape() {
    let dir = temp_dir("alias");
    let f = build_hud(&dir);
    let out = dir.join("shot.png");
    let text = cli_ok(&["ui", "screenshot", &f, "--output", out.to_str().unwrap(), "--width", "640", "--height", "480"]);
    assert!(text.contains("640x480"), "screenshot alias renders: {text}");
    let json = cli_json(&["ui", "inspect", &f, "--width", "800", "--height", "600"]);
    let title = rect(&json, "/Title");
    assert_eq!(title["className"], "TextLabel");
    assert_eq!(title["visible"], true);
}
