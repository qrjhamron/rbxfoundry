//! Gate B tests: GUI fidelity increments — UIFlexItem distribution,
//! grapheme-cluster text width, CFrame tween slerp, mouse wheel.

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
fn temp_dir(tag: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("bloxcli-gateb-{}-{tag}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }
fn rect(json: &serde_json::Value, suffix: &str) -> serde_json::Value {
    json["rects"].as_array().expect("rects").iter().find(|r| r["path"].as_str().unwrap_or("").ends_with(suffix)).unwrap_or_else(|| panic!("rect {suffix} in {json}")).clone()
}
fn gui_place(dir: &PathBuf) -> String {
    let file = dir.join("g.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "HUD"]);
    f
}
fn add(f: &str, parent: &str, class: &str, name: &str) {
    cli_ok(&["add", f, "--parent", parent, "--class", class, "--name", name]);
}
fn set(f: &str, target: &str, prop: &str, ty: &str, value: &str) {
    cli_ok(&["set", f, "--target", target, "--property", prop, "--type", ty, "--value", value]);
}

#[test]
fn uiflexitem_grows_into_free_space() {
    let dir = temp_dir("flex");
    let f = gui_place(&dir);
    add(&f, "StarterGui/HUD", "Frame", "Row");
    set(&f, "StarterGui/HUD/Row", "Size", "UDim2", "0,400,0,50");
    set(&f, "StarterGui/HUD/Row", "Position", "UDim2", "0,0,0,0");
    add(&f, "StarterGui/HUD/Row", "UIListLayout", "List");
    set(&f, "StarterGui/HUD/Row/List", "FillDirection", "Enum", "1");
    add(&f, "StarterGui/HUD/Row", "Frame", "Fixed");
    set(&f, "StarterGui/HUD/Row/Fixed", "Size", "UDim2", "0,100,0,50");
    add(&f, "StarterGui/HUD/Row", "Frame", "Flexy");
    set(&f, "StarterGui/HUD/Row/Flexy", "Size", "UDim2", "0,100,0,50");
    add(&f, "StarterGui/HUD/Row/Flexy", "UIFlexItem", "Flex");
    set(&f, "StarterGui/HUD/Row/Flexy/Flex", "FlexMode", "Enum", "1");
    let json = cli_json(&["ui", "inspect", &f, "--width", "800", "--height", "600", "--ignore-inset"]);
    let flexy = rect(&json, "/Flexy");
    let w = flexy["width"].as_f64().unwrap();
    assert!((w - 300.0).abs() < 2.0, "flex item absorbs free space (100 fixed + 300 grown): w={w}");
    let fixed = rect(&json, "/Fixed");
    assert!((fixed["width"].as_f64().unwrap() - 100.0).abs() < 1.0, "fixed sibling unchanged");
}

#[test]
fn combining_marks_have_zero_width() {
    let dir = temp_dir("grapheme");
    let f = gui_place(&dir);
    for (name, text) in [("Single", "e"), ("Combined", "e\u{301}"), ("Double", "ee")] {
        add(&f, "StarterGui/HUD", "TextLabel", name);
        let t = format!("StarterGui/HUD/{name}");
        set(&f, &t, "Text", "String", text);
        set(&f, &t, "TextSize", "Float64", "30");
        set(&f, &t, "AutomaticSize", "Enum", "1");
        set(&f, &t, "Size", "UDim2", "0,0,0,40");
    }
    let json = cli_json(&["ui", "inspect", &f, "--width", "800", "--height", "600", "--ignore-inset"]);
    let single = rect(&json, "/Single")["width"].as_f64().unwrap();
    let combined = rect(&json, "/Combined")["width"].as_f64().unwrap();
    let double = rect(&json, "/Double")["width"].as_f64().unwrap();
    assert!((combined - single).abs() <= 3.0, "combining mark adds no advance: single={single} combined={combined}");
    assert!(double > single + 5.0, "two base letters are wider: double={double}");
}

#[test]
fn tween_cframe_rotation_slerps() {
    let dir = temp_dir("slerp");
    let file = dir.join("g.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "Workspace", "Part", "Spinner");
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    set(&f, "ServerScriptService/Main", "Source", "String", r#"
        local p = workspace.Spinner
        game:GetService("TweenService"):Create(p, TweenInfo.new(1, Enum.EasingStyle.Linear),
            {CFrame = CFrame.new(0, 0, 0) * CFrame.Angles(0, math.rad(90), 0)}):Play()
    "#);
    let out = dir.join("out.rbxl");
    cli_json(&["run", &f, "--duration", "0.5", "--step", "0.05", "--save-runtime", out.to_str().unwrap()]);
    let (node, _) = bloxcli::rbxio::read_place(&out).expect("runtime out");
    let part = node.find_path("Workspace/Spinner").expect("spinner");
    match part.properties.get("CFrame") {
        Some(bloxcli::tree::PropValue::CFrame(cf)) => {
            // Halfway through a 90-degree Y rotation: cos(45deg) ~ 0.707.
            assert!((cf.rotation[0] - 0.7071).abs() < 0.05, "slerp halfway cos: {:?}", cf.rotation);
        }
        other => panic!("expected CFrame, got {other:?}"),
    }
}

#[test]
fn mouse_wheel_scrolls_scrolling_frame() {
    let dir = temp_dir("wheel");
    let f = gui_place(&dir);
    add(&f, "StarterGui/HUD", "ScrollingFrame", "List");
    set(&f, "StarterGui/HUD/List", "Size", "UDim2", "0,300,0,200");
    set(&f, "StarterGui/HUD/List", "Position", "UDim2", "0,0,0,0");
    add(&f, "StarterGui/HUD/List", "Frame", "Inner");
    set(&f, "StarterGui/HUD/List/Inner", "Size", "UDim2", "0,300,0,600");
    // Pre-scroll so wheel-up has somewhere to go, then wheel down in play.
    set(&f, "StarterGui/HUD/List", "CanvasPosition", "Vector2", "0,100");
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "LocalScript", "--name", "Watch"]);
    set(&f, "StarterGui/Watch", "Source", "String",
        "local list = script.Parent.HUD.List\nlist.InputBegan:Connect(function() print('canvas y', math.floor(list.CanvasPosition.Y + 0.5)) end)\n");
    cli_json(&["play", "start", &f]);
    // Wheel down (delta -1 => canvas moves down 40 from 100 to 140).
    let state = cli_json(&["play", "input", &f, "--wheel", "100,100"]);
    assert_eq!(state["hit"]["event"], "InputBegan", "{state}");
    let logs = cli_json(&["play", "logs", &f]);
    cli_json(&["play", "stop", &f]);
    let text = serde_json::to_string(&logs).unwrap();
    assert!(text.contains("canvas y 60"), "wheel up scrolled 100 -> 60: {text}");
}
