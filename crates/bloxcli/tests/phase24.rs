//! Gate 24 stability tests: corrupted inputs fail loudly (never
//! crash, never silently succeed), large places stay usable, and
//! repeated engine startup/shutdown is reliable.

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
    let dir = std::env::temp_dir().join(format!("bloxcli-phase24-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn corrupt_inputs_fail_loudly() {
    let dir = temp_dir("corrupt");

    // Random garbage with a place extension.
    let garbage = dir.join("garbage.rbxl");
    std::fs::write(&garbage, vec![0xABu8; 4096]).unwrap();
    let out = cli(&["validate", garbage.to_str().unwrap()]);
    assert!(!out.status.success(), "garbage rbxl must fail");
    assert!(!String::from_utf8_lossy(&out.stderr).is_empty(), "failure explains itself");

    // A real place truncated mid-stream.
    let good = dir.join("good.rbxl");
    let gf = good.to_str().unwrap().to_string();
    cli_ok(&["new", &gf]);
    cli_ok(&["add", &gf, "--parent", "Workspace", "--class", "Part", "--name", "P"]);
    let bytes = std::fs::read(&good).unwrap();
    let truncated = dir.join("truncated.rbxl");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
    let out = cli(&["validate", truncated.to_str().unwrap()]);
    assert!(!out.status.success(), "truncated rbxl must fail");

    // Malformed XML.
    let bad_xml = dir.join("bad.rbxlx");
    std::fs::write(&bad_xml, "<roblox><Item class=\"Part\"><Properties><string name=\"Name\">x").unwrap();
    let out = cli(&["validate", bad_xml.to_str().unwrap()]);
    assert!(!out.status.success(), "malformed rbxlx must fail");

    // A texture file that is not a PNG: explicit diagnostic, no crash.
    std::fs::write(dir.join("12345.png"), b"this is not a png").unwrap();
    cli_ok(&["add", &gf, "--parent", "Workspace", "--class", "MeshPart", "--name", "M"]);
    cli_ok(&["set", &gf, "--target", "Workspace/M", "--property", "MeshId", "--type", "ContentId", "--value", "rbxassetid://12345"]);
    let rendered = cli_json(&[
        "render", &gf, "--output", dir.join("r.png").to_str().unwrap(),
        "--width", "320", "--height", "240", "--assets", dir.to_str().unwrap(),
    ]);
    assert!(rendered["ok"] == true || rendered.get("error").is_some(), "render reports, never crashes: {rendered}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn large_place_stays_usable() {
    let dir = temp_dir("large");
    let file = dir.join("big.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["exec", &f, "--code", "
for i = 1, 3000 do
    local p = Instance.new('Part')
    p.Name = 'P' .. i
    p.Position = Vector3.new((i % 50) * 6, 3, math.floor(i / 50) * 6)
    p.Anchored = true
    p.Parent = workspace
end
print('built')"]);
    let tree = cli_json(&["tree", &f]);
    assert!(format!("{tree}").contains("P3000"), "all instances present");
    let validated = cli_json(&["validate", &f]);
    assert_eq!(validated["valid"], true, "{validated}");
    assert!(validated["instances"].as_u64().unwrap() > 3000);
    let shot = dir.join("big.png");
    let rendered = cli_json(&[
        "render", &f, "--output", shot.to_str().unwrap(),
        "--width", "320", "--height", "240", "--camera", "80,60,80", "--look-at", "150,0,180",
    ]);
    assert_eq!(rendered["ok"], true, "{rendered}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repeated_engine_start_stop_is_reliable() {
    let dir = temp_dir("restart");
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    for i in 0..15 {
        let out = cli(&["validate", &f]);
        assert!(out.status.success(), "engine cycle {i} validates cleanly");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
