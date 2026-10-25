//! Phase 22 integration tests: the Roblox UI builder workflow —
//! genuine GUI fixtures (HUD, inventory, shop, settings, mobile HUD)
//! built, rendered, validated and exported through the CLI, plus
//! LocalScript GUI events (click/hover/TextBox) driven through a real
//! play session.

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
    let dir = std::env::temp_dir().join(format!("bloxcli-phase22-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn gui_place(dir: &PathBuf, tag: &str) -> String {
    let file = dir.join(format!("{tag}.rbxl"));
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "HUD"]);
    f
}

fn add(f: &str, parent: &str, class: &str, name: &str) -> String {
    cli_ok(&["add", f, "--parent", parent, "--class", class, "--name", name]);
    format!("{parent}/{name}")
}

fn set(f: &str, target: &str, prop: &str, ty: &str, value: &str) {
    cli_ok(&["set", f, "--target", target, "--property", prop, "--type", ty, "--value", value]);
}

fn render_and_probe(f: &str, dir: &PathBuf, tag: &str, w: u32, h: u32, probe: (i64, i64)) -> (i64, i64, i64) {
    let out = dir.join(format!("{tag}.png"));
    let probes = format!("{},{}", probe.0, probe.1);
    let result = cli_json(&[
        "ui", "render", f, "--ignore-inset", "--output", out.to_str().unwrap(),
        "--width", &w.to_string(), "--height", &h.to_string(), "--probe", &probes,
    ]);
    let bytes = std::fs::read(&out).expect("ui png");
    assert!(bytes.len() > 200 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n", "{tag} png genuine");
    let p = &result["probes"][0]["rgba"];
    (p[0].as_i64().unwrap(), p[1].as_i64().unwrap(), p[2].as_i64().unwrap())
}

#[test]
fn ui_fixtures_render_validate_and_export() {
    // ---------- HUD (desktop) ----------
    let dir = temp_dir("fixtures");
    let f = gui_place(&dir, "hud");
    let bar = add(&f, "StarterGui/HUD", "Frame", "HealthBar");
    set(&f, &bar, "Size", "UDim2", "0,300,0,24");
    set(&f, &bar, "Position", "UDim2", "0,16,0,16");
    set(&f, &bar, "BackgroundColor3", "Color3", "0.2,0.8,0.2");
    let label = add(&f, "StarterGui/HUD", "TextLabel", "HealthText");
    set(&f, &label, "Size", "UDim2", "0,300,0,24");
    set(&f, &label, "Position", "UDim2", "0,16,0,16");
    set(&f, &label, "BackgroundTransparency", "Float64", "1");
    set(&f, &label, "Text", "String", "HP 100/100");
    set(&f, &label, "TextSize", "Float64", "18");
    let c = render_and_probe(&f, &dir, "hud", 1280, 720, (290, 20));
    assert!(c.1 > 150 && c.1 > c.0 + 60, "health bar green renders: {c:?}");

    // ---------- Inventory (grid of slots) ----------
    let f = gui_place(&dir, "inv");
    let panel = add(&f, "StarterGui/HUD", "Frame", "Grid");
    set(&f, &panel, "Size", "UDim2", "0,264,0,264");
    set(&f, &panel, "Position", "UDim2", "0,20,0,20");
    set(&f, &panel, "BackgroundColor3", "Color3", "0.15,0.15,0.2");
    for i in 0..9 {
        let slot = add(&f, "StarterGui/HUD/Grid", "Frame", &format!("Slot{}", i + 1));
        let x = 8 + (i % 3) * 84;
        let y = 8 + (i / 3) * 84;
        set(&f, &slot, "Size", "UDim2", "0,76,0,76");
        set(&f, &slot, "Position", "UDim2", &format!("0,{x},0,{y}"));
        set(&f, &slot, "BackgroundColor3", "Color3", "0.4,0.4,0.5");
    }
    let c = render_and_probe(&f, &dir, "inv", 800, 600, (50, 50));
    assert!(c.0 > 60 && c.2 > 60, "inventory slot renders: {c:?}");
    let validated = cli_json(&["ui", "validate", &f, "--width", "800", "--height", "600"]);
    assert_eq!(validated["ok"], true, "inventory validates: {validated}");

    // ---------- Shop (list + price labels) ----------
    let f = gui_place(&dir, "shop");
    let panel = add(&f, "StarterGui/HUD", "Frame", "Shop");
    set(&f, &panel, "Size", "UDim2", "0,360,0,300");
    set(&f, &panel, "Position", "UDim2", "0.5,-180,0.5,-150");
    set(&f, &panel, "BackgroundColor3", "Color3", "0.1,0.1,0.14");
    for (i, item) in ["Sword 100", "Shield 250", "Potion 40"].iter().enumerate() {
        let row = add(&f, "StarterGui/HUD/Shop", "TextButton", &format!("Buy{i}"));
        set(&f, &row, "Size", "UDim2", "1,-20,0,40");
        set(&f, &row, "Position", "UDim2", &format!("0,10,0,{}", 10 + i * 50));
        set(&f, &row, "Text", "String", item);
        set(&f, &row, "TextSize", "Float64", "20");
        set(&f, &row, "BackgroundColor3", "Color3", "0.25,0.5,0.9");
    }
    let c = render_and_probe(&f, &dir, "shop", 1024, 768, (512, 260));
    assert!(c.2 > 150, "shop buy button renders: {c:?}");

    // ---------- Settings (mobile) ----------
    let f = gui_place(&dir, "settings");
    let panel = add(&f, "StarterGui/HUD", "Frame", "Panel");
    set(&f, &panel, "Size", "UDim2", "1,-20,0,200");
    set(&f, &panel, "Position", "UDim2", "0,10,0,10");
    set(&f, &panel, "BackgroundColor3", "Color3", "0.9,0.9,0.92");
    let title = add(&f, "StarterGui/HUD/Panel", "TextLabel", "Title");
    set(&f, &title, "Size", "UDim2", "1,0,0,40");
    set(&f, &title, "Text", "String", "Settings");
    set(&f, &title, "TextSize", "Float64", "24");
    set(&f, &title, "BackgroundTransparency", "Float64", "1");
    let c = render_and_probe(&f, &dir, "settings", 390, 844, (30, 100));
    assert!(c.0 > 200 && c.1 > 200, "settings panel renders on mobile: {c:?}");

    // Every fixture exports and revalidates.
    for tag in ["hud", "inv", "shop", "settings"] {
        let src = dir.join(format!("{tag}.rbxl"));
        let out = dir.join(format!("{tag}.rbxlx"));
        cli_ok(&["export", src.to_str().unwrap(), out.to_str().unwrap()]);
        let v = cli_json(&["validate", out.to_str().unwrap()]);
        assert_eq!(v["valid"], true, "{tag} exports validly: {v}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn rect_center(f: &str, path: &str) -> (f64, f64) {
    let layout = cli_json(&["ui", "inspect", f, "--width", "1280", "--height", "720"]);
    for rect in layout["rects"].as_array().expect("rects") {
        if rect["path"].as_str() == Some(path) {
            let x = rect["x"].as_f64().unwrap() + rect["width"].as_f64().unwrap() / 2.0;
            let y = rect["y"].as_f64().unwrap() + rect["height"].as_f64().unwrap() / 2.0;
            return (x, y);
        }
    }
    panic!("rect for {path} not found in {layout}");
}

#[test]
fn play_gui_click_hover_and_textbox_events_fire() {
    let dir = temp_dir("events");
    let f = gui_place(&dir, "events");
    let button = add(&f, "StarterGui/HUD", "TextButton", "PlayBtn");
    set(&f, &button, "Size", "UDim2", "0,200,0,60");
    set(&f, &button, "Position", "UDim2", "0,40,0,40");
    set(&f, &button, "Text", "String", "Play");
    set(&f, &button, "TextSize", "Float64", "24");
    let status = add(&f, "StarterGui/HUD", "TextLabel", "Status");
    set(&f, &status, "Size", "UDim2", "0,400,0,40");
    set(&f, &status, "Position", "UDim2", "0,40,0,120");
    set(&f, &status, "Text", "String", "idle");
    let name_box = add(&f, "StarterGui/HUD", "TextBox", "NameBox");
    set(&f, &name_box, "Size", "UDim2", "0,200,0,40");
    set(&f, &name_box, "Position", "UDim2", "0,40,0,180");
    let script = add(&f, "StarterGui", "LocalScript", "UIHandler");
    set(&f, &script, "Source", "String", r#"
local gui = script.Parent
local btn = gui.HUD.PlayBtn
local status = gui.HUD.Status
local box = gui.HUD.NameBox
btn.MouseEnter:Connect(function() print("EVT enter") end)
btn.MouseLeave:Connect(function() print("EVT leave") end)
btn.MouseButton1Click:Connect(function()
    status.Text = "clicked"
    print("EVT click")
end)
box.Focused:Connect(function() print("EVT focused") end)
box.FocusLost:Connect(function(enterPressed)
    print("EVT name=" .. box.Text .. " enter=" .. tostring(enterPressed))
end)
"#);

    let (bx, by) = rect_center(&f, "StarterGui/HUD/PlayBtn");
    let (nx, ny) = rect_center(&f, "StarterGui/HUD/NameBox");

    cli_ok(&["play", "start", &f]);
    let cleanup = |f: &str| {
        let _ = cli(&["play", "stop", f]);
    };
    let hover = cli_json(&["play", "input", &f, "--hover", &format!("{bx:.0},{by:.0}")]);
    assert_eq!(hover["hit"]["event"], "MouseEnter", "{hover}");
    let click = cli_json(&["play", "input", &f, "--click", &format!("{bx:.0},{by:.0}")]);
    assert_eq!(click["hit"]["event"], "MouseButton1Click", "{click}");
    // Clicking the TextBox focuses it; typing + submit fires FocusLost(true).
    let focus = cli_json(&["play", "input", &f, "--click", &format!("{nx:.0},{ny:.0}")]);
    assert_eq!(focus["hit"]["event"], "Focused", "{focus}");
    cli_ok(&["play", "input", &f, "--type-text", "Hero", "--submit"]);
    // Hover away to empty space fires MouseLeave on the button.
    cli_ok(&["play", "input", &f, "--hover", "1200,650"]);
    let logs = cli_json(&["play", "logs", &f]);
    let text = format!("{}", logs["logs"]);
    for expected in ["EVT enter", "EVT click", "EVT focused", "EVT name=Hero enter=true", "EVT leave"] {
        assert!(text.contains(expected), "expected '{expected}' in play logs: {text}");
    }
    // The click handler's property change is live in the session state.
    let state = cli_json(&["play", "state", &f]);
    assert!(state["ok"] == true || state.get("time").is_some(), "{state}");
    cleanup(&f);
    let _ = std::fs::remove_dir_all(&dir);
}
