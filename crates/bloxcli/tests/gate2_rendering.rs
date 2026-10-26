//! Gate 2 — typography shaping (HarfBuzz) and UIGradient completeness
//! (arbitrary keypoints, Offset), verified through the real renderer.

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
fn add(f: &str, parent: &str, class: &str, name: &str) { cli_ok(&["add", f, "--parent", parent, "--class", class, "--name", name]); }
fn set(f: &str, target: &str, prop: &str, ty: &str, value: &str) { cli_ok(&["set", f, "--target", target, "--property", prop, "--type", ty, "--value", value]); }

fn label_place(dir: &PathBuf, text: &str) -> String {
    let file = dir.join("g.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "StarterGui", "ScreenGui", "HUD");
    add(&f, "StarterGui/HUD", "TextLabel", "L");
    set(&f, "StarterGui/HUD/L", "Size", "UDim2", "0,400,0,60");
    set(&f, "StarterGui/HUD/L", "BackgroundTransparency", "Float64", "1");
    set(&f, "StarterGui/HUD/L", "TextSize", "Float64", "32");
    set(&f, "StarterGui/HUD/L", "Text", "String", text);
    f
}
fn quads(f: &str, dir: &PathBuf) -> i64 {
    let out = dir.join("o.png");
    let r = cli_json(&["ui", "render", f, "--output", out.to_str().unwrap(), "--width", "800", "--height", "450"]);
    assert_eq!(r["warnings"], serde_json::json!([]), "no font warnings: {r}");
    r["quadCount"].as_i64().unwrap()
}
fn auto_width(f: &str) -> f64 {
    set(f, "StarterGui/HUD/L", "AutomaticSize", "Enum", "1"); // X
    set(f, "StarterGui/HUD/L", "Size", "UDim2", "0,0,0,60");
    let r = cli_json(&["ui", "inspect", f]);
    r["rects"].as_array().unwrap().iter()
        .find(|x| x["name"].as_str() == Some("L")).unwrap()["width"].as_f64().unwrap()
}

#[test]
fn harfbuzz_shaping_ligatures_kerning_combining_arabic() {
    let mk = |tag: &str, text: &str| {
        let dir = std::env::temp_dir().join(format!("bloxcli-type-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (label_place(&dir, text), dir)
    };
    let (fi, d1) = mk("fi", "fi");
    assert_eq!(quads(&fi, &d1), 1, "'fi' shapes to a single ligature glyph");
    let (ab, d2) = mk("ab", "ab");
    assert_eq!(quads(&ab, &d2), 2, "'ab' stays two glyphs");

    let (av, d3) = mk("av", "AV");
    let (a, d4) = mk("a", "A");
    let (v, d5) = mk("v", "V");
    let (w_av, w_a, w_v) = (auto_width(&av), auto_width(&a), auto_width(&v));
    assert!(w_av < w_a + w_v - 0.5, "kerning tightens AV: {w_av} vs {} + {w_v}", w_a);

    let (e, d6) = mk("e", "e");
    let (ec, d7) = mk("ec", "e\u{301}");
    let (w_e, w_ec) = (auto_width(&e), auto_width(&ec));
    assert!((w_ec - w_e).abs() < 1.5, "combining mark adds no advance: e={w_e} e+mark={w_ec}");
    assert!(quads(&ec, &d7) >= 1, "combining sequence renders");

    let (ar, d8) = mk("ar", "\u{633}\u{644}\u{627}\u{645}");
    assert!(quads(&ar, &d8) >= 3, "Arabic shapes and renders (no missing-glyph warnings)");
}

fn gradient_runtime(dir: &PathBuf, set_script: &str) -> String {
    let file = dir.join("g.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "StarterGui", "ScreenGui", "HUD");
    add(&f, "StarterGui/HUD", "Frame", "Panel");
    set(&f, "StarterGui/HUD/Panel", "Position", "UDim2", "0,40,0,40");
    set(&f, "StarterGui/HUD/Panel", "Size", "UDim2", "0,200,0,120");
    set(&f, "StarterGui/HUD/Panel", "BackgroundColor3", "Color3", "1,1,1");
    add(&f, "StarterGui/HUD/Panel", "UIGradient", "Grad");
    add(&f, "ServerScriptService", "Script", "Set");
    set(&f, "ServerScriptService/Set", "Source", "String", set_script);
    let out = dir.join("runtime.rbxl");
    cli_json(&["run", &f, "--duration", "0.05", "--save-runtime", out.to_str().unwrap()]);
    out.to_str().unwrap().to_string()
}
fn probe_row(f: &str, dir: &PathBuf, xs: &[i64], y: i64) -> Vec<(i64, i64, i64)> {
    let out = dir.join("p.png");
    let probes = xs.iter().map(|x| format!("{x},{y}")).collect::<Vec<_>>().join(";");
    let r = cli_json(&["ui", "render", f, "--ignore-inset", "--output", out.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probe", &probes]);
    r["probes"].as_array().unwrap().iter().map(|p| {
        let a = p["rgba"].as_array().unwrap();
        (a[0].as_i64().unwrap(), a[1].as_i64().unwrap(), a[2].as_i64().unwrap())
    }).collect()
}

#[test]
fn gradient_twelve_keypoints_and_offset() {
    let dir = std::env::temp_dir().join(format!("bloxcli-grad12-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Twelve alternating keypoints: bands must keep alternating past
    // the old 8-stop cap (stop 9/10/11 still evaluated).
    let f = gradient_runtime(&dir, r#"
        local kps = {}
        for i = 0, 11 do
            local c = (i % 2 == 0) and Color3.new(1, 0, 0) or Color3.new(0, 0, 1)
            table.insert(kps, ColorSequenceKeypoint.new(i / 11, c))
        end
        game.StarterGui.HUD.Panel.Grad.Color = ColorSequence.new(kps)
    "#);
    let xs: Vec<i64> = (0..12).map(|i| 40 + ((i as f64 + 0.5) * 200.0 / 12.0) as i64).collect();
    let px = probe_row(&f, &dir, &xs, 100);
    for (i, c) in px.iter().enumerate() {
        if i % 2 == 0 {
            assert!(c.0 > 120 && c.2 < 120, "band {i} red: {c:?}");
        } else {
            assert!(c.2 > 100 && c.0 < 150, "band {i} blue-ish: {c:?}");
        }
    }

    // Offset (0.5, 0): the ramp shifts right; the midpoint stays red.
    let dir2 = std::env::temp_dir().join(format!("bloxcli-gradoff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir2);
    std::fs::create_dir_all(&dir2).unwrap();
    let f2 = gradient_runtime(&dir2, r#"
        local g = game.StarterGui.HUD.Panel.Grad
        g.Color = ColorSequence.new({
            ColorSequenceKeypoint.new(0, Color3.new(1, 0, 0)),
            ColorSequenceKeypoint.new(1, Color3.new(0, 0, 1)),
        })
        g.Offset = Vector2.new(0.5, 0)
    "#);
    let px = probe_row(&f2, &dir2, &[60, 140, 220], 100);
    assert!(px[0].0 > 150 && px[0].2 < 100, "offset left stays red: {:?}", px[0]);
    assert!(px[1].0 > 120, "offset midpoint still red-dominant: {:?}", px[1]);
    assert!(px[2].2 > 60, "offset right starts blending: {:?}", px[2]);
}
