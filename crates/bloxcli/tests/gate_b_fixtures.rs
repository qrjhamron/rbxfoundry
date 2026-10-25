//! Gate B fixture acceptance: animated inventory GUI driven by a real
//! LocalScript through the actual play input path (click -> signal ->
//! TweenService -> stepped screenshots), plus desktop/mobile renders
//! and RBXL export/reimport integrity for the same fixture.

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
fn temp_dir(tag: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("bloxcli-gatebfx-{}-{tag}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }
fn add(f: &str, parent: &str, class: &str, name: &str) { cli_ok(&["add", f, "--parent", parent, "--class", class, "--name", name]); }
fn set(f: &str, target: &str, prop: &str, ty: &str, value: &str) { cli_ok(&["set", f, "--target", target, "--property", prop, "--type", ty, "--value", value]); }

#[test]
fn animated_inventory_fixture_end_to_end() {
    let dir = temp_dir("inventory");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "StarterGui", "ScreenGui", "HUD");
    // Open button (top-left) and an offscreen inventory panel.
    add(&f, "StarterGui/HUD", "TextButton", "OpenButton");
    set(&f, "StarterGui/HUD/OpenButton", "Size", "UDim2", "0,120,0,44");
    set(&f, "StarterGui/HUD/OpenButton", "Position", "UDim2", "0,16,0,16");
    set(&f, "StarterGui/HUD/OpenButton", "Text", "String", "Inventory");
    set(&f, "StarterGui/HUD/OpenButton", "BackgroundColor3", "Color3", "0.15,0.35,0.85");
    add(&f, "StarterGui/HUD", "Frame", "Inventory");
    set(&f, "StarterGui/HUD/Inventory", "Size", "UDim2", "0,320,0,240");
    set(&f, "StarterGui/HUD/Inventory", "Position", "UDim2", "0,-340,0,80");
    set(&f, "StarterGui/HUD/Inventory", "BackgroundColor3", "Color3", "0.85,0.2,0.2");
    set(&f, "StarterGui/HUD/Inventory", "Visible", "Bool", "false");
    add(&f, "StarterGui/HUD/Inventory", "TextLabel", "Title");
    set(&f, "StarterGui/HUD/Inventory/Title", "Text", "String", "Items");
    set(&f, "StarterGui/HUD/Inventory/Title", "Size", "UDim2", "1,0,0,36");
    add(&f, "StarterGui", "LocalScript", "UIHandler");
    set(&f, "StarterGui/UIHandler", "Source", "String", r#"
        local gui = script.Parent.HUD
        local panel = gui.Inventory
        local open = false
        gui.OpenButton.MouseButton1Click:Connect(function()
            open = not open
            panel.Visible = true
            local goal = open and UDim2.new(0, 16, 0, 80) or UDim2.new(0, -340, 0, 80)
            local tween = game:GetService("TweenService"):Create(panel,
                TweenInfo.new(0.6, Enum.EasingStyle.Quad, Enum.EasingDirection.Out),
                {Position = goal})
            tween.Completed:Connect(function(state)
                print("PANEL STATE", state, math.floor(panel.Position.X.Offset + 0.5))
            end)
            tween:Play()
        end)
    "#);

    // Static renders at desktop and mobile sizes are genuine PNGs and
    // validate cleanly.
    for (tag, w, h) in [("desktop", "1920", "1080"), ("mobile", "390", "844")] {
        let out = dir.join(format!("{tag}.png"));
        let r = cli_json(&["ui", "render", &f, "--ignore-inset", "--output", out.to_str().unwrap(), "--width", w, "--height", h, "--probe", "400,200"]);
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.len() > 200 && &bytes[0..4] == b"\x89PNG", "{tag} png");
        let _ = r;
    }
    let v = cli_json(&["ui", "validate", &f]);
    assert_eq!(v["valid"], true, "fixture validates: {v}");

    // Play: real input path. Screenshot before, click the button,
    // step in fixed increments, screenshot after the tween.
    cli_json(&["play", "start", &f]);
    let shot_a = dir.join("play-a.png");
    cli_json(&["play", "screenshot", &f, "--output", shot_a.to_str().unwrap(), "--width", "1280", "--height", "720"]);
    let click = cli_json(&["play", "input", &f, "--click", "76,74", "--width", "1280", "--height", "720"]);
    assert_eq!(click["hit"]["path"], "StarterGui/HUD/OpenButton", "native hit-test found the button: {click}");
    cli_json(&["play", "step", &f, "--steps", "30"]);
    let shot_mid = dir.join("play-mid.png");
    cli_json(&["play", "screenshot", &f, "--output", shot_mid.to_str().unwrap(), "--width", "1280", "--height", "720"]);
    cli_json(&["play", "step", &f, "--steps", "30"]);
    let shot_b = dir.join("play-b.png");
    cli_json(&["play", "screenshot", &f, "--output", shot_b.to_str().unwrap(), "--width", "1280", "--height", "720"]);
    let logs = cli_json(&["play", "logs", &f]);
    // Commit the runtime state so the tweened GUI can be rendered and
    // probed through the normal UI pipeline.
    cli_json(&["play", "stop", &f, "--commit"]);
    let text = serde_json::to_string(&logs).unwrap();
    assert!(text.contains("PANEL STATE Completed 16"), "tween completed at goal offset 16: {text}");
    for shot in [&shot_a, &shot_mid, &shot_b] {
        let bytes = std::fs::read(shot).unwrap();
        assert!(bytes.len() > 200 && &bytes[0..4] == b"\x89PNG", "play screenshot genuine: {shot:?}");
    }
    // After commit, the panel renders at its tweened on-screen spot:
    // probe its centre (16+160, 80+36+120) through ui render.
    let probe_out = dir.join("after.png");
    let render = cli_json(&["ui", "render", &f, "--ignore-inset", "--output", probe_out.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probe", "176,236"]);
    let rgba = &render["probes"][0]["rgba"];
    assert!(rgba[0].as_i64().unwrap() > 150 && rgba[1].as_i64().unwrap() < 110,
        "committed panel renders red at its goal position: {rgba}");

    // Export/reimport: the fixture (scripts included) survives.
    let exported = dir.join("export.rbxlx");
    cli_ok(&["export", &f, exported.to_str().unwrap()]);
    let (node, _) = bloxcli::rbxio::read_place(&exported).expect("reimport exported fixture");
    let panel = node.find_path("StarterGui/HUD/Inventory").expect("panel reimported");
    assert!(panel.properties.contains_key("Position"));
    let script = node.find_path("StarterGui/UIHandler").expect("script reimported");
    match script.properties.get("Source") {
        Some(bloxcli::tree::PropValue::String(s)) => assert!(s.contains("TweenInfo.new"), "script source preserved"),
        other => panic!("script Source: {other:?}"),
    }
}

fn render_probe(f: &str, dir: &PathBuf, tag: &str, w: &str, h: &str, probe: &str) -> serde_json::Value {
    let out = dir.join(format!("{tag}.png"));
    let r = cli_json(&["ui", "render", f, "--ignore-inset", "--output", out.to_str().unwrap(), "--width", w, "--height", h, "--probe", probe]);
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.len() > 200 && &bytes[0..4] == b"\x89PNG", "{tag} png genuine");
    r
}
fn check_export(dir: &PathBuf, f: &str, must_contain: &[&str]) {
    let exported = dir.join("reexport.rbxlx");
    cli_ok(&["export", f, exported.to_str().unwrap()]);
    let (node, _) = bloxcli::rbxio::read_place(&exported).expect("reimport fixture");
    let text = format!("{:?}", node);
    for needle in must_contain { assert!(text.contains(needle), "export contains {needle}"); }
}

#[test]
fn shop_settings_mobile_menu_fixtures() {
    // ---------- Shop: grid of item cards with prices ----------
    let dir = temp_dir("shop");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "StarterGui", "ScreenGui", "HUD");
    add(&f, "StarterGui/HUD", "Frame", "Shop");
    set(&f, "StarterGui/HUD/Shop", "Size", "UDim2", "0,480,0,320");
    set(&f, "StarterGui/HUD/Shop", "Position", "UDim2", "0,40,0,40");
    set(&f, "StarterGui/HUD/Shop", "BackgroundColor3", "Color3", "0.12,0.12,0.16");
    add(&f, "StarterGui/HUD/Shop", "UIGridLayout", "Grid");
    set(&f, "StarterGui/HUD/Shop/Grid", "CellSize", "UDim2", "0,100,0,120");
    for i in 1..=4 {
        let card = format!("Item{i}");
        add(&f, "StarterGui/HUD/Shop", "TextButton", &card);
        set(&f, &format!("StarterGui/HUD/Shop/{card}"), "Text", "String", &format!("Sword {i}\n$ {i}00"));
        set(&f, &format!("StarterGui/HUD/Shop/{card}"), "BackgroundColor3", "Color3", "0.2,0.5,0.9");
    }
    let r = render_probe(&f, &dir, "shop-desktop", "1920", "1080", "90,100");
    assert!(r["probes"][0]["rgba"][2].as_i64().unwrap() > 100, "shop card renders blue-ish: {r}");
    let v = cli_json(&["ui", "validate", &f]);
    assert_eq!(v["valid"], true, "shop validates: {v}");
    check_export(&dir, &f, &["Item4", "UIGridLayout"]);

    // ---------- Settings: list layout with toggles ----------
    let dir = temp_dir("settings");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "StarterGui", "ScreenGui", "HUD");
    add(&f, "StarterGui/HUD", "ScrollingFrame", "Settings");
    set(&f, "StarterGui/HUD/Settings", "Size", "UDim2", "0,360,0,400");
    set(&f, "StarterGui/HUD/Settings", "Position", "UDim2", "0,30,0,30");
    add(&f, "StarterGui/HUD/Settings", "UIListLayout", "List");
    for name in ["Music", "Effects", "Shadows"] {
        add(&f, "StarterGui/HUD/Settings", "TextButton", name);
        set(&f, &format!("StarterGui/HUD/Settings/{name}"), "Size", "UDim2", "1,0,0,48");
        set(&f, &format!("StarterGui/HUD/Settings/{name}"), "Text", "String", name);
        set(&f, &format!("StarterGui/HUD/Settings/{name}"), "BackgroundColor3", "Color3", "0.25,0.25,0.3");
    }
    let r = render_probe(&f, &dir, "settings-desktop", "1366", "768", "100,60");
    assert!(r["probes"][0]["rgba"][0].as_i64().unwrap() > 40, "settings row renders: {r}");
    check_export(&dir, &f, &["ScrollingFrame", "Shadows"]);

    // ---------- Mobile interface at 390x844 ----------
    let dir = temp_dir("mobile");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "StarterGui", "ScreenGui", "HUD");
    add(&f, "StarterGui/HUD", "TextButton", "Jump");
    set(&f, "StarterGui/HUD/Jump", "Size", "UDim2", "0,84,0,84");
    set(&f, "StarterGui/HUD/Jump", "Position", "UDim2", "1,-100,1,-120");
    set(&f, "StarterGui/HUD/Jump", "Text", "String", "JUMP");
    set(&f, "StarterGui/HUD/Jump", "BackgroundColor3", "Color3", "0.9,0.55,0.15");
    let json = cli_json(&["ui", "inspect", &f, "--device", "mobile"]);
    let jump = json["rects"].as_array().unwrap().iter().find(|r| r["path"].as_str().unwrap().ends_with("/Jump")).expect("jump rect");
    assert!((jump["x"].as_f64().unwrap() - 290.0).abs() < 3.0, "mobile jump anchored bottom-right: {jump}");
    let r = render_probe(&f, &dir, "mobile", "390", "844", "330,760");
    assert!(r["probes"][0]["rgba"][0].as_i64().unwrap() > 150, "mobile button renders orange: {r}");
    check_export(&dir, &f, &["Jump"]);

    // ---------- Animated menu (tween verified on scheduler time) ----------
    let dir = temp_dir("menu");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "StarterGui", "ScreenGui", "HUD");
    add(&f, "StarterGui/HUD", "Frame", "Menu");
    set(&f, "StarterGui/HUD/Menu", "Size", "UDim2", "0,260,0,180");
    set(&f, "StarterGui/HUD/Menu", "Position", "UDim2", "0.5,0,0,-200");
    set(&f, "StarterGui/HUD/Menu", "AnchorPoint", "Vector2", "0.5,0");
    set(&f, "StarterGui/HUD/Menu", "BackgroundColor3", "Color3", "0.1,0.7,0.4");
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    set(&f, "ServerScriptService/Main", "Source", "String", r#"
        local menu = game.StarterGui.HUD.Menu
        game:GetService("TweenService"):Create(menu, TweenInfo.new(1, Enum.EasingStyle.Back, Enum.EasingDirection.Out),
            {Position = UDim2.new(0.5, 0, 0, 60)}):Play()
    "#);
    let out = dir.join("runtime.rbxl");
    cli_json(&["run", &f, "--duration", "0.5", "--step", "0.05", "--save-runtime", out.to_str().unwrap()]);
    let (mid, _) = bloxcli::rbxio::read_place(&out).expect("mid runtime");
    let menu_mid = mid.find_path("StarterGui/HUD/Menu").expect("menu mid");
    let mid_y = match menu_mid.properties.get("Position") { Some(bloxcli::tree::PropValue::UDim2(v)) => v[3], _ => panic!("mid Position") };
    assert!(mid_y > 60.0, "menu mid-flight Back overshoot past goal: y={mid_y}");
    let out2 = dir.join("runtime2.rbxl");
    cli_json(&["run", &f, "--duration", "1.2", "--step", "0.05", "--save-runtime", out2.to_str().unwrap()]);
    let (done, _) = bloxcli::rbxio::read_place(&out2).expect("done runtime");
    let menu_done = done.find_path("StarterGui/HUD/Menu").expect("menu done");
    let done_y = match menu_done.properties.get("Position") { Some(bloxcli::tree::PropValue::UDim2(v)) => v[3], _ => panic!("done Position") };
    assert!((done_y - 60.0).abs() < 1.0, "menu lands at goal: y={done_y}");
}
