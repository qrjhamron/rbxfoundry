//! Phase 19 integration tests: Unicode text rendering (UTF-8
//! codepoints, font fallback reporting), model-root self-references,
//! and the offline place sidecar manifest.

use std::path::PathBuf;
use std::process::Command;

use bloxcli::rbxio::{read_place, write_place};
use bloxcli::tree::{Node, PropValue};

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
    let dir = std::env::temp_dir().join(format!("bloxcli-phase19-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn gui_place(dir: &PathBuf, text: &str) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "HUD"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD", "--class", "TextLabel", "--name", "L"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "Size", "--type", "UDim2", "--value", "0,760,0,90"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "TextSize", "--type", "Float64", "--value", "34"]);
    cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "Text", "--type", "String", "--value", text]);
    f
}

fn text_width(f: &str, text: &str) -> f64 {
    // TextService:GetTextSize via the Luau runtime's measured logs is
    // indirect; use `run --eval` printing the Vector2 width.
    let eval = format!(
        "local ts = game:GetService('TextService')\nlocal s = ts:GetTextSize('{text}', 34, Enum.Font.Legacy, Vector2.new(2000, 100))\nprint('WIDTH=' .. string.format('%.1f', s.X))"
    );
    let run = cli_json(&["run", f, "--duration", "0.1", "--eval", &eval]);
    let logs = format!("{}", run["logs"]);
    let start = logs.find("WIDTH=").expect("width logged") + 6;
    let num: String = logs[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    num.parse().expect("numeric width")
}

#[test]
fn unicode_measurement_uses_codepoints_not_bytes() {
    let dir = temp_dir("unicode");
    let f = gui_place(&dir, "x");
    let cafe_accent = text_width(&f, "Café");
    let cafe_plain = text_width(&f, "Cafe");
    assert!(cafe_accent > 20.0 && cafe_plain > 20.0, "widths measured: {cafe_accent} vs {cafe_plain}");
    // é is ONE glyph, so "Café" and "Cafe" are both 4 glyphs wide
    // (within one glyph's width). Byte-oriented rendering made "Café"
    // five glyphs (Ã + ©), measurably wider.
    assert!(
        (cafe_accent - cafe_plain).abs() < cafe_plain * 0.35,
        "accented width {cafe_accent} should be close to plain {cafe_plain}"
    );
    let dash = text_width(&f, "—");
    let dashes = text_width(&f, "——");
    assert!(dash > 10.0, "em dash has real width: {dash}");
    assert!((dashes - 2.0 * dash).abs() < dash * 0.3, "two dashes ≈ 2x one: {dashes} vs {dash}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unicode_renders_and_reports_uncovered_codepoints() {
    let dir = temp_dir("uncover");
    let f = gui_place(&dir, "Café — 中文");
    let out = dir.join("ui.png");
    let result = cli_json(&[
        "ui", "render", &f, "--ignore-inset",
        "--output", out.to_str().unwrap(), "--width", "800", "--height", "160",
    ]);
    let warnings = format!("{}", result["warnings"]);
    // DejaVu covers é/— but not CJK on this system: the renderer must
    // SAY so (drawn as .notdef boxes) instead of failing or faking it.
    assert!(
        warnings.contains("no installed font covers") && warnings.contains("U+4E2D"),
        "uncovered codepoints reported: {warnings}"
    );
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.len() > 1000 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn model_root_self_reference_round_trips() {
    let dir = temp_dir("selfref");
    // House (Model) with an ObjectValue whose Value points at House
    // itself — the model-root self-reference that used to collapse
    // to None on read.
    let mut root = Node::new("DataModel", "Place");
    let mut workspace = Node::new("Workspace", "Workspace");
    let mut house = Node::new("Model", "House");
    let mut link = Node::new("ObjectValue", "SelfLink");
    link.properties
        .insert("Value".to_string(), PropValue::Ref("0/0".to_string())); // Workspace/House
    house.children.push(Node::new("Part", "Wall"));
    house.children.push(link);
    workspace.children.push(house);
    root.children.push(workspace);
    let place = dir.join("game.rbxl");
    write_place(&place, &root).expect("write fixture");
    let f = place.to_str().unwrap().to_string();

    let model_file = dir.join("house.rbxm");
    cli_ok(&["model", "export", &f, "--target", "Workspace/House", "--output", model_file.to_str().unwrap()]);
    let inspected = cli_json(&["model", "inspect", model_file.to_str().unwrap()]);
    assert!(
        format!("{}", inspected["tree"]).contains("\".\""),
        "self-reference stored as '.' token: {}", inspected["tree"]
    );

    let target = dir.join("target.rbxl");
    let tf = target.to_str().unwrap().to_string();
    cli_ok(&["new", &tf]);
    cli_ok(&["model", "import", model_file.to_str().unwrap(), &tf, "--parent", "Workspace"]);
    let (back, _) = read_place(&target).expect("read target");
    let link = back.find_path("Workspace/House/SelfLink").expect("link imported");
    match link.properties.get("Value") {
        Some(PropValue::Ref(v)) if !v.is_empty() => {
            let resolved = bloxcli::integrity::ref_index_path(&back, v).expect("ref resolves");
            let target_node = bloxcli::integrity::node_at(&back, &resolved).expect("target exists");
            assert_eq!(target_node.name, "House", "self-link points at the imported House");
            assert_eq!(target_node.class_name, "Model");
        }
        other => panic!("SelfLink.Value must be a non-empty Ref after round trip, got {other:?}"),
    }
    let validated = cli_json(&["validate", &tf]);
    assert_eq!(validated["valid"], true, "{validated}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn place_manifest_sidecar_records_offline_metadata() {
    let dir = temp_dir("manifest");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f, "--template", "baseplate"]);
    let bytes_before = std::fs::read(&file).unwrap();

    let result = cli_json(&["place", "manifest", &f]);
    let output = result["output"].as_str().unwrap().to_string();
    assert!(output.ends_with("game.rbxl.manifest.json"), "{output}");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).expect("manifest json");
    assert_eq!(manifest["metadata"]["placeId"], 0);
    assert!(manifest["metadata"]["state"].as_str().unwrap().contains("offline"));
    assert_eq!(manifest["integrity"]["errors"], 0);
    assert!(manifest["classCounts"]["Part"].as_u64().unwrap() >= 1);
    assert!(manifest["services"].as_array().unwrap().iter().any(|s| s == "Workspace"));
    // The sidecar is separate: the place file itself is untouched.
    assert_eq!(std::fs::read(&file).unwrap(), bytes_before);
    let _ = std::fs::remove_dir_all(&dir);
}
