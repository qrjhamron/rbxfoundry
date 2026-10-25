//! Phase 25 gate tests: persistent Studio sessions.
//!
//! Verifies one long-lived engine serves many independent CLI
//! processes, unsaved state stays in memory (never in the file),
//! undo/redo works across invocations, MCP observes the same state,
//! explicit save round-trips, and a killed daemon recovers unsaved
//! edits without corrupting the original.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value as JsonValue};

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") {
        if !p.is_empty() { return PathBuf::from(p); }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(candidate.is_file(), "blox-engine not found at {}", candidate.display());
    candidate
}

fn cli_json(args: &[&str]) -> JsonValue {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json").arg("--engine").arg(engine_path()).args(args)
        .output().expect("run bloxcli");
    assert!(out.status.success(), "bloxcli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("json output")
}

fn cli_ok(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--engine").arg(engine_path()).args(args)
        .output().expect("run bloxcli");
    assert!(out.status.success(), "bloxcli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-p25-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

struct SessionGuard(PathBuf);
impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
            .arg("--engine").arg(engine_path())
            .args(["studio", "stop", &self.0.to_string_lossy()])
            .output();
    }
}

#[test]
fn persistent_session_gate() {
    let dir = temp_dir("gate");
    let place = dir.join("game.rbxl");
    cli_ok(&["new", &place.to_string_lossy(), "--name", "P25"]);
    let _guard = SessionGuard(place.clone());
    let f = place.to_string_lossy().to_string();

    let started = cli_json(&["studio", "start", &f]);
    assert_eq!(started["state"]["dirty"], false);
    let session_id = started["state"]["sessionId"].as_str().unwrap().to_string();

    // 100 sequential CLI operations against one persistent engine.
    cli_json(&["studio", "add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Counter"]);
    for i in 0..99 {
        let v = format!("{}", i as f64 / 100.0);
        cli_json(&["studio", "set", &f, "--target", "Workspace/Counter", "--property", "Transparency", "--value", &v]);
    }
    let status = cli_json(&["studio", "status", &f]);
    assert_eq!(status["state"]["sessionId"], session_id, "same daemon served all operations");
    assert_eq!(status["state"]["dirty"], true);

    // Unsaved state is in memory only: the file has no Counter yet.
    let tree = cli_ok(&["tree", &f]);
    assert!(!tree.contains("Counter"), "unsaved edits must not touch the file");

    // Selection + camera persist in the session.
    cli_json(&["studio", "select", &f, "--targets", "Workspace/Counter"]);
    cli_json(&["studio", "camera", &f, "--position", "10,10,10", "--look-at", "0,0,0"]);
    let status = cli_json(&["studio", "status", &f]);
    assert_eq!(status["state"]["selection"][0], "Workspace/Counter");
    assert_eq!(status["state"]["camera"]["position"][0], 10.0);

    // Undo/redo across invocations (Transparency 0.98 -> undo -> 0.97).
    let before = cli_json(&["studio", "get", &f, "--target", "Workspace/Counter"]);
    assert!((before["instance"]["properties"]["Transparency"]["value"].as_f64().unwrap() - 0.98).abs() < 1e-5);
    cli_json(&["studio", "undo", &f]);
    let undone = cli_json(&["studio", "get", &f, "--target", "Workspace/Counter"]);
    assert!((undone["instance"]["properties"]["Transparency"]["value"].as_f64().unwrap() - 0.97).abs() < 1e-5, "undo stepped one op");
    cli_json(&["studio", "redo", &f]);
    let redone = cli_json(&["studio", "get", &f, "--target", "Workspace/Counter"]);
    assert!((redone["instance"]["properties"]["Transparency"]["value"].as_f64().unwrap() - 0.98).abs() < 1e-5);

    // Explicit save writes equivalent data; reload sees it.
    cli_json(&["studio", "save", &f]);
    let status = cli_json(&["studio", "status", &f]);
    assert_eq!(status["state"]["dirty"], false);
    let tree = cli_ok(&["tree", &f]);
    assert!(tree.contains("Counter"), "saved edits are in the file");
    cli_json(&["studio", "stop", &f]);
    let (node, _) = bloxcli::rbxio::read_place(&place).expect("read saved place");
    let counter = node.find_path("Workspace/Counter").expect("Counter saved");
    match counter.properties.get("Transparency") {
        Some(bloxcli::tree::PropValue::Float64(v)) => assert!((*v - 0.98).abs() < 1e-5),
        Some(bloxcli::tree::PropValue::Float32(v)) => assert!((*v as f64 - 0.98).abs() < 1e-4),
        other => panic!("unexpected Transparency {other:?}"),
    }
}

#[test]
fn killed_daemon_recovers_unsaved_edits() {
    let dir = temp_dir("crash");
    let place = dir.join("game.rbxl");
    cli_ok(&["new", &place.to_string_lossy(), "--name", "Crash"]);
    let f = place.to_string_lossy().to_string();
    cli_json(&["studio", "start", &f]);
    cli_json(&["studio", "add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Unsaved"]);
    let meta: JsonValue = serde_json::from_str(&std::fs::read_to_string(dir.join("game.rbxl.bloxstudio/session.json")).unwrap()).unwrap();
    let pid = meta["pid"].as_i64().unwrap();
    // Simulate unexpected termination.
    assert!(Command::new("kill").arg("-9").arg(pid.to_string()).status().unwrap().success());
    std::thread::sleep(std::time::Duration::from_millis(300));
    let status = cli_json(&["studio", "status", &f]);
    assert_eq!(status["live"], false, "dead daemon detected");
    // Original file is uncorrupted and still lacks the unsaved part.
    let tree = cli_ok(&["tree", &f]);
    assert!(!tree.contains("Unsaved"));
    bloxcli::rbxio::read_place(&place).expect("original still decodes");
    // Restart recovers the unsaved edit from the recovery snapshot.
    let restarted = cli_json(&["studio", "start", &f]);
    assert_eq!(restarted["state"]["recovered"], true);
    let got = cli_json(&["studio", "get", &f, "--target", "Workspace/Unsaved"]);
    assert_eq!(got["instance"]["name"], "Unsaved");
    cli_json(&["studio", "stop", &f]);
    let _ = json!(null);
}

#[test]
fn mcp_observes_same_studio_session() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command as Cmd, Stdio};
    let dir = temp_dir("mcp");
    let place = dir.join("game.rbxl");
    cli_ok(&["new", &place.to_string_lossy(), "--name", "McpP25"]);
    let f = place.to_string_lossy().to_string();
    cli_json(&["studio", "start", &f]);
    cli_json(&["studio", "add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Shared"]);
    cli_json(&["studio", "set", &f, "--target", "Workspace/Shared", "--property", "Transparency", "--value", "0.25"]);

    let mut child = Cmd::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--engine").arg(engine_path()).arg("mcp").arg("--root").arg(&dir)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().expect("spawn mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut id = 0;
    let mut call = |stdin: &mut std::process::ChildStdin, stdout: &mut BufReader<std::process::ChildStdout>, tool: &str, args: JsonValue| -> JsonValue {
        id += 1;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": args}});
        writeln!(stdin, "{req}").unwrap(); stdin.flush().unwrap();
        let mut line = String::new(); stdout.read_line(&mut line).unwrap();
        let resp: JsonValue = serde_json::from_str(&line).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().expect("tool text");
        serde_json::from_str(text).expect("tool json")
    };
    // initialize first (id 0 handled separately to keep closure ids simple)
    writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
    stdin.flush().unwrap();
    let mut line = String::new(); stdout.read_line(&mut line).unwrap();
    let got = call(&mut stdin, &mut stdout, "studio_session_get", json!({"path": "game.rbxl", "target": "Workspace/Shared"}));
    assert_eq!(got["instance"]["name"], "Shared");
    let v = got["instance"]["properties"]["Transparency"]["value"].as_f64().unwrap();
    assert!((v - 0.25).abs() < 1e-4, "MCP sees CLI session value, got {v}");
    drop(stdin);
    let _ = child.wait();
    cli_json(&["studio", "stop", &f]);
}
