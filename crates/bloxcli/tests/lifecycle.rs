//! Gate 1 — PlayerGui/PlayerScripts lifecycle: join cloning,
//! death -> CharacterRemoving -> respawn -> CharacterAdded, ScreenGui
//! ResetOnSpawn=true re-clones (scripts restart, state resets),
//! ResetOnSpawn=false keeps live state, PlayerScripts run once.

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

fn counter_gui(f: &str, gui: &str, x: i32, reset_on_spawn: bool, tag: &str) {
    add(f, "StarterGui", "ScreenGui", gui);
    if !reset_on_spawn {
        set(f, &format!("StarterGui/{gui}"), "ResetOnSpawn", "Bool", "false");
    }
    add(f, &format!("StarterGui/{gui}"), "Frame", "Panel");
    set(f, &format!("StarterGui/{gui}/Panel"), "Position", "UDim2", &format!("0,{x},0,16"));
    set(f, &format!("StarterGui/{gui}/Panel"), "Size", "UDim2", "0,200,0,120");
    set(f, &format!("StarterGui/{gui}/Panel"), "BackgroundColor3", "Color3", "0.12,0.12,0.16");
    add(f, &format!("StarterGui/{gui}/Panel"), "TextLabel", "Count");
    set(f, &format!("StarterGui/{gui}/Panel/Count"), "Size", "UDim2", "1,0,0,40");
    set(f, &format!("StarterGui/{gui}/Panel/Count"), "Text", "String", "Count 0");
    set(f, &format!("StarterGui/{gui}/Panel/Count"), "TextColor3", "Color3", "1,1,1");
    add(f, &format!("StarterGui/{gui}"), "TextButton", "Inc");
    set(f, &format!("StarterGui/{gui}/Inc"), "Position", "UDim2", &format!("0,{x},0,150"));
    set(f, &format!("StarterGui/{gui}/Inc"), "Size", "UDim2", "0,120,0,44");
    set(f, &format!("StarterGui/{gui}/Inc"), "Text", "String", "Add");
    add(f, &format!("StarterGui/{gui}"), "LocalScript", "Ctl");
    set(f, &format!("StarterGui/{gui}/Ctl"), "Source", "String", &format!(r#"
        print("{tag} gui run")
        local count = 0
        local label = script.Parent.Panel.Count
        script.Parent.Inc.MouseButton1Click:Connect(function()
            count += 1
            label.Text = "Count " .. count
        end)
    "#));
}

#[test]
fn playergui_lifecycle_respawn_reset_semantics() {
    let dir = std::env::temp_dir().join(format!("bloxcli-lifecycle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "Workspace", "Part", "Ground");
    set(&f, "Workspace/Ground", "Size", "Vector3", "80,1,80");
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-0.5,0"]);
    set(&f, "Workspace/Ground", "Anchored", "Bool", "true");

    counter_gui(&f, "HUDReset", 16, true, "reset");
    counter_gui(&f, "HUDKeep", 300, false, "keep");

    add(&f, "StarterPlayer", "StarterPlayerScripts", "ScriptsRoot");
    add(&f, "StarterPlayer/ScriptsRoot", "LocalScript", "PSMain");
    set(&f, "StarterPlayer/ScriptsRoot/PSMain", "Source", "String", r#"
        print("playerscripts run", script:GetFullName())
        local player = game.Players.LocalPlayer
        player.CharacterRemoving:Connect(function() print("removing seen") end)
        player.CharacterAdded:Connect(function() print("added seen") end)
    "#);
    add(&f, "ServerScriptService", "Script", "Killer");
    set(&f, "ServerScriptService/Killer", "Source", "String", r#"
        task.wait(0.8)
        workspace.Player.Humanoid.Health = 0
        print("killed")
    "#);

    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "30"]);
    // Increment both counters through native clicks.
    cli_json(&["play", "input", &f, "--click", "76,208"]);
    cli_json(&["play", "input", &f, "--click", "360,208"]);
    // Death at 0.8s + 1s respawn timer + margin.
    cli_json(&["play", "step", &f, "--steps", "260"]);

    let logs = cli_json(&["play", "logs", &f]);
    let text = serde_json::to_string(&logs).unwrap();
    let count = |needle: &str| text.matches(needle).count();
    assert!(count("playerscripts run") == 1, "PlayerScripts run exactly once (not on respawn): {text}");
    assert!(text.contains("PlayerScripts"), "PSMain runs from the PlayerScripts clone: {text}");
    assert!(count("reset gui run") == 2, "ResetOnSpawn GUI script restarted after respawn: {text}");
    assert!(count("keep gui run") == 1, "ResetOnSpawn=false GUI script not restarted: {text}");
    assert!(text.contains("killed"), "{text}");
    assert!(text.contains("removing seen"), "CharacterRemoving fired: {text}");
    assert!(text.contains("added seen"), "CharacterAdded fired on respawn: {text}");

    let state = cli_json(&["play", "state", &f]);
    assert_eq!(state["state"]["character"]["health"], 100.0, "health restored: {}", state["state"]);
    let rects = state["state"]["gui"]["rects"].as_array().unwrap();
    let label = |gui: &str| rects.iter()
        .find(|r| r["name"].as_str() == Some("Count") && r["path"].as_str().unwrap_or("").contains(gui))
        .unwrap_or_else(|| panic!("{gui} Count rect"))
        ["text"].as_str().unwrap_or("").to_string();
    assert_eq!(label("HUDReset"), "Count 0", "reset GUI state cleared");
    assert_eq!(label("HUDKeep"), "Count 1", "kept GUI state preserved");

    // Screenshot shows the live post-respawn PlayerGui without commit.
    let shot = dir.join("post.png");
    let r = cli_json(&["play", "screenshot", &f, "--output", shot.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probes", "100,100;400,100"]);
    let probes = r["result"]["probes"].as_array().unwrap();
    for p in probes {
        let rgba = p["rgba"].as_array().unwrap();
        assert!(rgba[0].as_i64().unwrap() < 90, "post-respawn GUI panel renders: {rgba:?}");
    }
    cli_json(&["play", "stop", &f]);
}

#[test]
fn seat_sit_and_stand() {
    let dir = std::env::temp_dir().join(format!("bloxcli-seat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "Workspace", "Part", "Ground");
    set(&f, "Workspace/Ground", "Size", "Vector3", "80,1,80");
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-0.5,0"]);
    set(&f, "Workspace/Ground", "Anchored", "Bool", "true");
    add(&f, "Workspace", "Seat", "Bench");
    set(&f, "Workspace/Bench", "Size", "Vector3", "6,1,6");
    cli_ok(&["move", &f, "--target", "Workspace/Bench", "--position", "0,0.5,0"]);
    set(&f, "Workspace/Bench", "Anchored", "Bool", "true");
    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "150"]);
    let state = cli_json(&["play", "state", &f]);
    assert_eq!(state["state"]["character"]["state"], "Seated",
        "character sits on the Seat it landed on: {}", state["state"]);
    cli_json(&["play", "input", &f, "--keys", "Space", "--duration", "0.3"]);
    // Checked immediately: left alone, the character would land on the
    // bench again and re-sit (correct seat behaviour).
    let state2 = cli_json(&["play", "state", &f]);
    assert_ne!(state2["state"]["character"]["state"], "Seated",
        "jump stands the character up: {}", state2["state"]);
    cli_json(&["play", "stop", &f]);
}

#[test]
fn gate6_studio_to_play_lifecycle_workflow() {
    // Studio session edit -> save -> play (die/respawn/screenshot) ->
    // stop -> edit state intact -> reopen verifies saved data.
    let dir = std::env::temp_dir().join(format!("bloxcli-gate6-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "Workspace", "Part", "Ground");
    set(&f, "Workspace/Ground", "Size", "Vector3", "80,1,80");
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-0.5,0"]);
    set(&f, "Workspace/Ground", "Anchored", "Bool", "true");
    counter_gui(&f, "HUDReset", 16, true, "reset");
    add(&f, "ServerScriptService", "Script", "Killer");
    set(&f, "ServerScriptService/Killer", "Source", "String", r#"
        task.wait(0.8)
        workspace.Player.Humanoid.Health = 0
    "#);
    // Edit through the persistent Studio session and save.
    cli_json(&["studio", "start", &f]);
    let st = cli_json(&["studio", "status", &f]);
    assert_eq!(st["ok"], true, "{st}");
    cli_json(&["studio", "stop", &f]);
    let saved_hash = std::fs::read(&f).unwrap();

    cli_json(&["play", "start", &f]);
    cli_json(&["play", "step", &f, "--steps", "240"]);
    let state = cli_json(&["play", "state", &f]);
    assert_eq!(state["state"]["character"]["health"], 100.0, "respawned: {}", state["state"]);
    let shot = dir.join("gate6.png");
    let r = cli_json(&["play", "screenshot", &f, "--output", shot.to_str().unwrap(),
        "--width", "1280", "--height", "720", "--probes", "100,100"]);
    assert!(r["result"]["probes"][0]["rgba"][0].as_i64().unwrap() < 90, "live post-respawn GUI in frame");
    cli_json(&["play", "stop", &f]);
    assert_eq!(std::fs::read(&f).unwrap(), saved_hash, "edit file intact after play");
    let (node, _) = bloxcli::rbxio::read_place(&file).expect("reopen saved place");
    node.find_path("StarterGui/HUDReset/Panel/Count").expect("saved GUI hierarchy intact");
}

#[test]
fn union_operation_opaque_data_preserved() {
    // Gate 3: an uninterpreted UnionOperation must survive round-trips
    // with its properties intact (never silently emptied or converted).
    let dir = std::env::temp_dir().join(format!("bloxcli-union-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    add(&f, "Workspace", "UnionOperation", "CSGPart");
    set(&f, "Workspace/CSGPart", "Size", "Vector3", "6,4,6");
    cli_ok(&["move", &f, "--target", "Workspace/CSGPart", "--position", "0,4,0"]);
    let out = dir.join("out.rbxlx");
    cli_ok(&["export", &f, out.to_str().unwrap()]);
    let back = dir.join("back.rbxl");
    cli_ok(&["export", out.to_str().unwrap(), back.to_str().unwrap()]);
    let (node, _) = bloxcli::rbxio::read_place(&back).expect("reimport");
    let u = node.find_path("Workspace/CSGPart").expect("union survives round-trip");
    assert_eq!(u.class_name, "UnionOperation");
    match u.properties.get("Size") {
        Some(bloxcli::tree::PropValue::Vector3(v)) => {
            assert!((v[0] - 6.0).abs() < 0.01 && (v[1] - 4.0).abs() < 0.01, "size preserved: {v:?}");
        }
        other => panic!("Size: {other:?}"),
    }
}
