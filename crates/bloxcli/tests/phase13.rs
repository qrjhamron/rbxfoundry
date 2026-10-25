//! Phase 13 tests: place templates, place metadata, `place info`,
//! `template inspect`, and Luau DataModel metadata semantics.
//! Template Studio values are UNTESTED (no golden fixture); these tests
//! verify RBXFoundry's own documented template structure and behavior.

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") {
        if !p.is_empty() { return PathBuf::from(p); }
    }
    let c = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(c.is_file(), "engine missing at {}", c.display());
    c
}
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--engine").arg(engine_path()).args(args).output().unwrap()
}
fn cli_ok(args: &[&str]) -> String {
    let o = cli(args);
    assert!(o.status.success(), "bloxcli {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}
fn cli_json(args: &[&str]) -> serde_json::Value {
    let o = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--json").arg("--engine").arg(engine_path()).args(args).output().unwrap();
    assert!(o.status.success(), "bloxcli --json {args:?}: {}\n{}", String::from_utf8_lossy(&o.stderr), String::from_utf8_lossy(&o.stdout));
    serde_json::from_slice(&o.stdout).unwrap()
}
fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bloxcli-p13-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn find_node<'a>(n: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    if n["name"].as_str() == Some(name) { return Some(n); }
    for c in n["children"].as_array()? { if let Some(f) = find_node(c, name) { return Some(f); } }
    None
}

#[test]
fn templates_place_info_and_metadata() {
    let dir = temp_dir("templates");
    // Registry is inspectable.
    let list = cli_json(&["template", "list"]);
    let names: Vec<&str> = list["templates"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["empty", "baseplate", "classic-baseplate"]);
    let inspect = cli_json(&["template", "inspect", "baseplate"]);
    assert!(inspect["template"]["status"].as_str().unwrap().contains("UNTESTED"));

    // Baseplate structure.
    let file = dir.join("MyGame.rbxl");
    let f = file.to_str().unwrap().to_string();
    let created = cli_json(&["new", &f, "--template", "baseplate"]);
    assert_eq!(created["template"], "baseplate");
    let tree = cli_json(&["tree", &f]);
    let baseplate = find_node(&tree["tree"], "Baseplate").expect("Baseplate");
    assert_eq!(baseplate["className"], "Part");
    assert_eq!(baseplate["properties"]["Size"]["value"], serde_json::json!([512.0, 16.0, 512.0]));
    assert_eq!(baseplate["properties"]["CFrame"]["value"]["position"][1], -8.0);
    assert!(find_node(&tree["tree"], "SpawnLocation").is_some());
    assert!(find_node(&tree["tree"], "Terrain").is_some());
    assert!(find_node(&tree["tree"], "Camera").is_some());

    // Place info reports honest offline metadata.
    let info = cli_json(&["place", "info", &f]);
    assert_eq!(info["info"]["metadata"]["placeId"], 0);
    assert_eq!(info["info"]["metadata"]["gameId"], "");
    assert!(info["info"]["metadata"]["state"].as_str().unwrap().contains("offline-unpublished"));
    assert!(info["info"]["services"].as_array().unwrap().iter().any(|s| s == "Workspace"));

    // Luau DataModel metadata semantics.
    let run = cli_json(&["run", &f, "--duration", "0.1", "--eval",
        "assert(game.PlaceId == 0 and game.GameId == '' and game.PlaceVersion == 0 and game.JobId == '' and game.CreatorId == 0)\nprint('META OK')"]);
    assert!(format!("{}", run["logs"]).contains("META OK"), "{run}");
    let ro = cli(&["run", &f, "--duration", "0.1", "--eval", "game.PlaceId = 123"]);
    assert!(!ro.status.success());
    assert!(String::from_utf8_lossy(&ro.stderr).contains("read-only"));

    // Baseplate renders and spawns a grounded character.
    let shot = dir.join("baseplate.png");
    cli_ok(&["render", &f, "--output", shot.to_str().unwrap(), "--width", "640", "--height", "400", "--camera", "20,14,24", "--look-at", "0,0,0"]);
    let bytes = std::fs::read(&shot).unwrap();
    assert!(bytes.len() > 1000 && &bytes[0..4] == b"\x89PNG");
    cli_ok(&["play", "start", &f]);
    cli_ok(&["play", "step", &f, "--steps", "90"]);
    let state = cli_json(&["play", "state", &f]);
    assert_eq!(state["state"]["character"]["grounded"], true, "{state}");
    let y = state["state"]["character"]["position"][1].as_f64().unwrap();
    assert!(y > 3.0 && y < 4.0, "character stands on the SpawnLocation pad (top y=1), centre y={y}");
    cli_ok(&["play", "stop", &f]);

    // Classic template differs only by the studded top surface.
    let classic = dir.join("Classic.rbxl");
    let cf = classic.to_str().unwrap().to_string();
    cli_json(&["new", &cf, "--template", "classic-baseplate"]);
    let ctree = cli_json(&["tree", &cf]);
    let cb = find_node(&ctree["tree"], "Baseplate").unwrap();
    assert_eq!(cb["properties"]["TopSurface"]["value"], 1);

    // Default `new` remains the empty template (no regression).
    let plain = dir.join("Plain.rbxl");
    let pf = plain.to_str().unwrap().to_string();
    cli_json(&["new", &pf]);
    let ptree = cli_json(&["tree", &pf]);
    assert!(find_node(&ptree["tree"], "Baseplate").is_none());
}
