//! Cross-phase integration: persistent Studio session edits, Tween +
//! DataStore in Play-style runs, save/reopen verification (Phases
//! 25+27B+29). Internal verification only; Studio parity UNTESTED.

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

#[test]
fn studio_edit_tween_datastore_reopen() {
    let dir = std::env::temp_dir().join(format!("bloxcli-xphase-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f, "--name", "XPhase"]);
    // Persistent studio session: edit + save through the daemon.
    cli_json(&["studio", "start", &f]);
    cli_json(&["studio", "add", &f, "--parent", "Workspace", "--class", "Part", "--name", "QuestPart"]);
    cli_json(&["studio", "save", &f]);
    cli_json(&["studio", "stop", &f]);
    // Script: tween a GUI panel and record progression in DataStore.
    cli_ok(&["add", &f, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "Hud"]);
    cli_ok(&["add", &f, "--parent", "StarterGui/Hud", "--class", "Frame", "--name", "Panel"]);
    cli_ok(&["set", &f, "--target", "StarterGui/Hud/Panel", "--property", "Position", "--type", "UDim2", "--value", "0,0,0,0"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value", r#"
        local ds = game:GetService("DataStoreService"):GetDataStore("Progress")
        ds:IncrementAsync("level", 1)
        local panel = game.StarterGui.Hud.Panel
        game:GetService("TweenService"):Create(panel, TweenInfo.new(0.1, Enum.EasingStyle.Linear), {Position = UDim2.new(0.5, 0, 0, 0)}):Play()
        print("quest part", workspace.QuestPart.Name)
    "#]);
    let run = cli_json(&["run", &f, "--duration", "0.3", "--step", "0.05"]);
    let logs = serde_json::to_string(&run).unwrap();
    assert!(logs.contains("quest part QuestPart"), "{logs}");
    // Reopen (fresh engine process): progression persisted.
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value",
        "print('level restored', game:GetService('DataStoreService'):GetDataStore('Progress'):GetAsync('level'))"]);
    let run2 = cli_json(&["run", &f, "--duration", "0.05"]);
    let logs2 = serde_json::to_string(&run2).unwrap();
    assert!(logs2.contains("level restored 1"), "{logs2}");
    // Exported file still decodes with all instances intact.
    let (node, _) = bloxcli::rbxio::read_place(&file).expect("reopen place");
    assert!(node.find_path("Workspace/QuestPart").is_ok());
    assert!(node.find_path("StarterGui/Hud/Panel").is_ok());
}
