//! Phase 16 integration tests: RichText rendering (color spans,
//! underline), ImageLabel nine-slice (ScaleType=Slice) and
//! multi-device UI screenshots. All assertions use real rendered
//! pixels via the engine's probe output.

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
    let dir = std::env::temp_dir().join(format!("bloxcli-phase16-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn probe_map(json: &serde_json::Value) -> std::collections::HashMap<(i64, i64), (i64, i64, i64)> {
    let mut map = std::collections::HashMap::new();
    for p in json["probes"].as_array().expect("probes array") {
        let rgba = p["rgba"].as_array().expect("rgba");
        map.insert(
            (p["x"].as_i64().unwrap(), p["y"].as_i64().unwrap()),
            (rgba[0].as_i64().unwrap(), rgba[1].as_i64().unwrap(), rgba[2].as_i64().unwrap()),
        );
    }
    map
}

fn probe_str(points: &[(i64, i64)]) -> String {
    points.iter().map(|(x, y)| format!("{x},{y}")).collect::<Vec<_>>().join(";")
}

fn is_red(c: (i64, i64, i64)) -> bool {
    c.0 > 150 && c.0 > c.1 + 60 && c.0 > c.2 + 60
}

fn gui_place(dir: &PathBuf) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "HUD"]);
    f
}

fn add_label(f: &str, name: &str, text: &str, rich: bool) {
    let path = format!("StarterGui/HUD/{name}");
    cli_ok(&["add", f, "--parent", "StarterGui/HUD", "--class", "TextLabel", "--name", name]);
    cli_ok(&["set", f, "--target", &path, "--property", "Size", "--type", "UDim2", "--value", "0,400,0,60"]);
    cli_ok(&["set", f, "--target", &path, "--property", "Position", "--type", "UDim2", "--value", "0,0,0,0"]);
    cli_ok(&["set", f, "--target", &path, "--property", "BackgroundTransparency", "--type", "Float64", "--value", "1"]);
    cli_ok(&["set", f, "--target", &path, "--property", "TextSize", "--type", "Float64", "--value", "30"]);
    cli_ok(&["set", f, "--target", &path, "--property", "Text", "--type", "String", "--value", text]);
    cli_ok(&["set", f, "--target", &path, "--property", "RichText", "--type", "Bool", "--value", if rich { "true" } else { "false" }]);
}

fn ui_render(f: &str, out: &PathBuf, w: u32, h: u32, probes: &[(i64, i64)]) -> serde_json::Value {
    let probe_arg = probe_str(probes);
    cli_json(&[
        "ui", "render", f, "--ignore-inset",
        "--output", out.to_str().unwrap(),
        "--width", &w.to_string(), "--height", &h.to_string(),
        "--probe", &probe_arg,
    ])
}

#[test]
fn rich_text_color_span_renders_colored_glyphs() {
    let dir = temp_dir("richcolor");
    let out = dir.join("ui.png");

    // Grid over the "BBBB" segment of `AA <font color="#FF0000">BBBB</font>`.
    let mut grid: Vec<(i64, i64)> = Vec::new();
    for y in (16..=44).step_by(4) {
        for x in (62..=150).step_by(4) {
            grid.push((x, y));
        }
    }

    let f_rich = {
        let dir2 = temp_dir("richcolor-on");
        let f = gui_place(&dir2);
        add_label(&f, "L", "AA <font color=\"#FF0000\">BBBB</font>", true);
        f
    };
    let json = ui_render(&f_rich, &out, 400, 120, &grid);
    let map = probe_map(&json);
    let reds = grid.iter().filter(|p| map.get(p).map(|c| is_red(*c)).unwrap_or(false)).count();
    assert!(reds >= 4, "expected red glyph pixels from RichText color span, found {reds}");

    // Control: identical text with RichText off renders no red span.
    let f_plain = {
        let dir3 = temp_dir("richcolor-off");
        let f = gui_place(&dir3);
        add_label(&f, "L", "AA <font color=\"#FF0000\">BBBB</font>", false);
        f
    };
    let json = ui_render(&f_plain, &out, 400, 120, &grid);
    let map = probe_map(&json);
    let reds_off = grid.iter().filter(|p| map.get(p).map(|c| is_red(*c)).unwrap_or(false)).count();
    assert_eq!(reds_off, 0, "non-RichText label must not render the color span");

    // The file round-trips with RichText + Text preserved.
    let got = cli_json(&["get", &f_rich, "--target", "StarterGui/HUD/L"]);
    assert!(format!("{}", got).contains("#FF0000"), "rich text source preserved: {got}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rich_text_underline_draws_baseline_rule() {
    let dir = temp_dir("richunderline");
    let out = dir.join("ui.png");
    // 100px-tall label, 20px text: baseline ~57.5, underline rule ~60,
    // glyph bottoms sit on the baseline, so row 60 only sees the rule.
    let mut row: Vec<(i64, i64)> = (4..=110).step_by(3).map(|x| (x, 60)).collect();
    row.push((200, 60));

    let f_on = {
        let dir2 = temp_dir("richunderline-on");
        let f = gui_place(&dir2);
        add_label(&f, "L", "<u>UNDER</u>", true);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "Size", "--type", "UDim2", "--value", "0,400,0,100"]);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "TextSize", "--type", "Float64", "--value", "20"]);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "BackgroundTransparency", "--type", "Float64", "--value", "0"]);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "BackgroundColor3", "--type", "Color3", "--value", "1,1,1"]);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "TextColor3", "--type", "Color3", "--value", "1,0,0"]);
        f
    };
    let json = ui_render(&f_on, &out, 400, 120, &row);
    let map = probe_map(&json);
    let hits = row.iter().filter(|p| map.get(p).map(|c| is_red(*c)).unwrap_or(false)).count();
    assert!(hits >= 8, "underline rule pixels expected under UNDER, found {hits}");

    let f_off = {
        let dir3 = temp_dir("richunderline-off");
        let f = gui_place(&dir3);
        add_label(&f, "L", "UNDER", true);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "Size", "--type", "UDim2", "--value", "0,400,0,100"]);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "TextSize", "--type", "Float64", "--value", "20"]);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "BackgroundTransparency", "--type", "Float64", "--value", "0"]);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "BackgroundColor3", "--type", "Color3", "--value", "1,1,1"]);
        cli_ok(&["set", &f, "--target", "StarterGui/HUD/L", "--property", "TextColor3", "--type", "Color3", "--value", "1,0,0"]);
        f
    };
    let json = ui_render(&f_off, &out, 400, 120, &row);
    let map = probe_map(&json);
    let hits_off = row.iter().filter(|p| map.get(p).map(|c| is_red(*c)).unwrap_or(false)).count();
    assert!(hits_off <= 2, "no underline expected without <u>, found {hits_off}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn image_label_nine_slice_keeps_corners_unscaled() {
    let dir = temp_dir("nineslice");
    // The renderer resolves relative image names next to the place file.
    std::fs::copy(fixtures_dir().join("nineslice.png"), dir.join("nineslice.png")).expect("copy fixture");
    let f = gui_place(&dir);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD", "--class", "ImageLabel", "--name", "Panel"]);
    let p = "StarterGui/HUD/Panel";
    cli_ok(&["set", &f, "--target", p, "--property", "Size", "--type", "UDim2", "--value", "0,96,0,96"]);
    cli_ok(&["set", &f, "--target", p, "--property", "Position", "--type", "UDim2", "--value", "0,0,0,0"]);
    cli_ok(&["set", &f, "--target", p, "--property", "BackgroundTransparency", "--type", "Float64", "--value", "1"]);
    cli_ok(&["set", &f, "--target", p, "--property", "Image", "--type", "ContentId", "--value", "nineslice.png"]);
    cli_ok(&["set", &f, "--target", p, "--property", "ScaleType", "--type", "Enum", "--value", "1"]); // Slice
    cli_ok(&["set", &f, "--target", p, "--property", "SliceCenter", "--type", "Rect", "--value", "16,16,32,32"]);
    cli_ok(&["set", &f, "--target", p, "--property", "SliceScale", "--type", "Float64", "--value", "1"]);

    let probes = [(8, 8), (48, 8), (48, 48), (8, 48), (88, 48), (48, 88), (24, 24)];
    let out = dir.join("slice.png");
    let json = ui_render(&f, &out, 200, 200, &probes);
    let m = probe_map(&json);
    let near = |c: (i64, i64, i64), t: (i64, i64, i64)| (c.0 - t.0).abs() < 40 && (c.1 - t.1).abs() < 40 && (c.2 - t.2).abs() < 40;
    assert!(near(m[&(8, 8)], (255, 0, 0)), "corner stays red: {:?}", m[&(8, 8)]);
    assert!(near(m[&(48, 8)], (0, 200, 0)), "top edge green: {:?}", m[&(48, 8)]);
    assert!(near(m[&(48, 48)], (255, 255, 255)), "centre white: {:?}", m[&(48, 48)]);
    assert!(near(m[&(8, 48)], (0, 0, 255)), "left edge blue: {:?}", m[&(8, 48)]);
    assert!(near(m[&(88, 48)], (255, 0, 255)), "right edge magenta: {:?}", m[&(88, 48)]);
    assert!(near(m[&(48, 88)], (255, 200, 0)), "bottom edge yellow: {:?}", m[&(48, 88)]);
    assert!(near(m[&(24, 24)], (255, 255, 255)), "inside 16px corner the centre begins: {:?}", m[&(24, 24)]);

    // Stretch control: the same probe lands inside the scaled corner.
    cli_ok(&["set", &f, "--target", p, "--property", "ScaleType", "--type", "Enum", "--value", "0"]); // Stretch
    let out2 = dir.join("stretch.png");
    let json = ui_render(&f, &out2, 200, 200, &[(24, 24)]);
    let m2 = probe_map(&json);
    assert!(near(m2[&(24, 24)], (255, 0, 0)), "stretch scales the corner: {:?}", m2[&(24, 24)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ui_renders_on_desktop_tablet_and_mobile() {
    let dir = temp_dir("devices");
    let f = gui_place(&dir);
    cli_ok(&["add", &f, "--parent", "StarterGui/HUD", "--class", "Frame", "--name", "Panel"]);
    let p = "StarterGui/HUD/Panel";
    cli_ok(&["set", &f, "--target", p, "--property", "Size", "--type", "UDim2", "--value", "0,200,0,80"]);
    cli_ok(&["set", &f, "--target", p, "--property", "Position", "--type", "UDim2", "--value", "1,-210,1,-90"]);
    cli_ok(&["set", &f, "--target", p, "--property", "BackgroundColor3", "--type", "Color3", "--value", "0.2,0.4,0.9"]);

    for (name, w, h, px, py) in [
        ("desktop", 1920, 1080, 1800, 1030),
        ("tablet", 1280, 800, 1160, 750),
        ("mobile", 390, 844, 280, 790),
    ] {
        let out = dir.join(format!("{name}.png"));
        let json = ui_render(&f, &out, w, h, &[(px, py)]);
        let bytes = std::fs::read(&out).expect("read ui png");
        assert!(bytes.len() > 100 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n", "{name} png valid");
        let m = probe_map(&json);
        let c = m[&(px, py)];
        assert!(c.2 > 150 && c.2 > c.0 + 60, "{name} panel pixel at ({px},{py}) should be blue, got {c:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
