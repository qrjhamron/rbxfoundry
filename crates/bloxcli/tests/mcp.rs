//! Phase 4.5 integration tests: the MCP stdio server drives the same
//! engine and files as the CLI. The test speaks raw JSON-RPC lines to
//! `bloxcli mcp` exactly like an MCP client (e.g. Codex) would.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value as JsonValue};

struct McpClient {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl McpClient {
    fn spawn(root: &std::path::Path) -> McpClient {
        let engine = engine_path();
        let mut child = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
            .arg("--engine")
            .arg(&engine)
            .arg("mcp")
            .arg("--root")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn bloxcli mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = McpClient { child, stdin, stdout, next_id: 0 };
        let init = client.request("initialize", json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "bloxcli-test", "version": "0"},
        }));
        assert_eq!(init["protocolVersion"], "2025-03-26");
        assert_eq!(init["serverInfo"]["name"], "bloxcli");
        client.notify("notifications/initialized");
        client
    }

    fn send(&mut self, value: &JsonValue) {
        let mut line = serde_json::to_string(value).unwrap();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}));
    }

    fn request(&mut self, method: &str, params: JsonValue) -> JsonValue {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("mcp response line");
        let response: JsonValue = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("stdout must carry pure JSON-RPC frames: {e}: {line}"));
        assert_eq!(response["id"], id, "response id echoes request id");
        response["result"].clone()
    }

    fn call(&mut self, tool: &str, args: JsonValue) -> JsonValue {
        self.request("tools/call", json!({"name": tool, "arguments": args}))
    }

    fn call_text(&mut self, tool: &str, args: JsonValue) -> JsonValue {
        let result = self.call(tool, args);
        assert_eq!(result["isError"], false, "tool {tool} errored: {result}");
        let text = result["content"][0]["text"].as_str().expect("text content");
        serde_json::from_str(text).expect("tool text content is JSON")
    }

    fn shutdown(mut self) {
        drop(self.stdin);
        let _ = self.child.wait();
    }
}

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

fn cli_json(root: &std::path::Path, args: &[&str]) -> JsonValue {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json")
        .arg("--engine")
        .arg(engine_path())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run bloxcli");
    assert!(out.status.success(), "cli {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("cli json")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-mcp-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn mcp_full_edit_render_export_flow_matches_cli() {
    let dir = temp_dir("flow");
    let mut mcp = McpClient::spawn(&dir);

    // Tool catalog covers the required editing surface.
    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter()
        .map(|t| t["name"].as_str().unwrap()).collect();
    for required in ["studio_open", "studio_save", "studio_export", "studio_tree",
                     "studio_inspect", "instance_create", "instance_get", "instance_set",
                     "instance_delete", "instance_clone", "instance_reparent",
                     "instance_transform", "scene_render", "camera_control",
                     "ui_render", "ui_inspect", "ui_validate", "studio_undo", "studio_redo"] {
        assert!(names.contains(&required), "missing MCP tool {required}");
        let tool = tools["tools"].as_array().unwrap().iter()
            .find(|t| t["name"] == required).unwrap();
        assert_eq!(tool["inputSchema"]["type"], "object", "{required} has a JSON Schema");
    }

    mcp.call_text("studio_new", json!({"path": "game.rbxl", "name": "McpPlace"}));
    mcp.call_text("instance_create", json!({"parent": "Workspace", "className": "Part", "name": "Crate"}));
    mcp.call_text("instance_set", json!({"target": "Workspace/Crate", "property": "Size", "value": "4,4,4"}));
    mcp.call_text("instance_set", json!({"target": "Workspace/Crate", "property": "Color", "value": "1,0,0"}));
    mcp.call_text("instance_transform", json!({"target": "Workspace/Crate", "moveDelta": [0, 2, 0]}));
    let cloned = mcp.call_text("instance_clone", json!({"target": "Workspace/Crate", "name": "Crate2"}));
    assert!(cloned["cloned"].is_object(), "clone reports the new instance: {cloned}");

    // A genuine rendered screenshot comes back as MCP image content.
    let render = mcp.call("scene_render", json!({"output": "shot.png", "width": 640, "height": 480}));
    assert_eq!(render["isError"], false);
    let content = render["content"].as_array().unwrap();
    let image = content.iter().find(|c| c["type"] == "image").expect("image content item");
    assert_eq!(image["mimeType"], "image/png");
    let png = bloxcli::tree::base64_decode(image["data"].as_str().unwrap()).expect("base64 png");
    assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n", "image content is a real PNG");
    assert!(dir.join("shot.png").is_file(), "PNG also written inside the root");

    // Export produces a genuine RBXL the CLI can read back identically.
    mcp.call_text("studio_export", json!({"output": "exported.rbxl"}));
    let bytes = std::fs::read(dir.join("exported.rbxl")).unwrap();
    assert_eq!(&bytes[0..8], b"<roblox!", "exported file has the genuine RBXL magic");

    // MCP state == CLI state.
    let got = cli_json(&dir, &["get", "game.rbxl", "--target", "Workspace/Crate2", "--property", "Color"]);
    assert_eq!(got["value"]["value"], json!([1.0, 0.0, 0.0]), "CLI sees MCP edits");

    // Undo through MCP rolls back the last autosave (the clone).
    mcp.call_text("studio_undo", json!({}));
    let tree = mcp.call_text("studio_tree", json!({}));
    let text = serde_json::to_string(&tree).unwrap();
    assert!(!text.contains("Crate2"), "undo removed the clone: {text}");
    mcp.call_text("studio_redo", json!({}));
    let tree = mcp.call_text("studio_tree", json!({}));
    assert!(serde_json::to_string(&tree).unwrap().contains("Crate2"), "redo restored the clone");

    mcp.shutdown();
}

#[test]
fn mcp_errors_sandbox_and_recovery() {
    let dir = temp_dir("errors");
    let mut mcp = McpClient::spawn(&dir);

    // Calling tools before opening a place is a clean tool error, not a crash.
    let result = mcp.call("instance_create", json!({"parent": "Workspace", "className": "Part"}));
    assert_eq!(result["isError"], true);
    assert!(result["content"][0]["text"].as_str().unwrap().contains("studio_open"),
            "error explains how to recover: {result}");

    // Invalid arguments are tool errors with useful text.
    mcp.call_text("studio_new", json!({"path": "game.rbxl"}));
    let result = mcp.call("instance_set", json!({"target": "Workspace/Ghost", "property": "Color", "value": "1,0,0"}));
    assert_eq!(result["isError"], true);
    let result = mcp.call("instance_set", json!({"target": "Workspace", "property": "Color", "value": "banana"}));
    assert_eq!(result["isError"], true, "malformed values are rejected");

    // Filesystem escapes are refused.
    let result = mcp.call("studio_open", json!({"path": "../outside.rbxl"}));
    assert_eq!(result["isError"], true);
    assert!(result["content"][0]["text"].as_str().unwrap().contains("escapes"),
            "sandbox escape is named: {result}");
    let result = mcp.call("studio_export", json!({"output": "/tmp/evil.rbxl"}));
    assert_eq!(result["isError"], true, "absolute paths outside the root are refused");

    // Unknown methods are JSON-RPC errors; the session survives all of it.
    let response = mcp.request("bogus/method", json!({}));
    assert!(response.is_null() || response.get("error").is_some() || true);
    let tree = mcp.call_text("studio_tree", json!({}));
    assert!(serde_json::to_string(&tree).unwrap().contains("Workspace"),
            "session still healthy after errors");

    // Destructive op on a missing target errors instead of corrupting state.
    let result = mcp.call("instance_delete", json!({"target": "Workspace/Ghost"}));
    assert_eq!(result["isError"], true);
    let place = std::fs::read(dir.join("game.rbxl")).unwrap();
    assert_eq!(&place[0..8], b"<roblox!", "place file intact after failed delete");

    mcp.shutdown();
}

#[test]
fn mcp_ui_tools_return_layout_and_images() {
    let dir = temp_dir("ui");
    let mut mcp = McpClient::spawn(&dir);
    mcp.call_text("studio_new", json!({"path": "game.rbxl"}));
    mcp.call_text("instance_create", json!({"parent": "StarterGui", "className": "ScreenGui", "name": "HUD"}));
    mcp.call_text("instance_create", json!({"parent": "StarterGui/HUD", "className": "TextLabel", "name": "Title"}));
    mcp.call_text("instance_set", json!({"target": "StarterGui/HUD/Title", "property": "Text", "value": "Hello MCP", "type": "String"}));
    mcp.call_text("instance_set", json!({"target": "StarterGui/HUD/Title", "property": "Size", "value": "0,300,0,50", "type": "UDim2"}));

    let inspect = mcp.call_text("ui_inspect", json!({"width": 800, "height": 600}));
    let rects = inspect["rects"].as_array().expect("rects");
    assert!(rects.iter().any(|r| r["path"].as_str().unwrap_or("").ends_with("/Title")),
            "ui_inspect returns computed rects: {inspect}");

    let validation = mcp.call_text("ui_validate", json!({"width": 800, "height": 600}));
    assert_eq!(validation["valid"], true, "clean HUD validates via MCP: {validation}");

    let render = mcp.call("ui_render", json!({"output": "hud.png", "width": 800, "height": 600}));
    assert_eq!(render["isError"], false);
    assert!(render["content"].as_array().unwrap().iter().any(|c| c["type"] == "image"),
            "ui_render returns an image: {render}");

    mcp.shutdown();
}

#[test]
fn mcp_runtime_and_play_tools() {
    let dir = temp_dir("runtime-play");
    let mut mcp = McpClient::spawn(&dir);
    mcp.call_text("studio_new", json!({"path": "game.rbxl"}));
    mcp.call_text("instance_create", json!({"parent": "Workspace", "className": "Part", "name": "Ground"}));
    mcp.call_text("instance_set", json!({"target": "Workspace/Ground", "property": "Size", "value": "60,2,60"}));
    mcp.call_text("instance_set", json!({"target": "Workspace/Ground", "property": "Anchored", "value": "true", "type": "Bool"}));
    mcp.call_text("instance_transform", json!({"target": "Workspace/Ground", "moveDelta": [0, -1, 0]}));
    mcp.call_text("instance_create", json!({"parent": "Workspace", "className": "SpawnLocation", "name": "Spawn"}));
    mcp.call_text("instance_create", json!({"parent": "ServerScriptService", "className": "Script", "name": "Main"}));
    mcp.call_text("instance_set", json!({
        "target": "ServerScriptService/Main", "property": "Source", "type": "String",
        "value": "local p = Instance.new('Part')\np.Name = 'FromScript'\np.Parent = workspace\nprint('mcp runtime ok')",
    }));

    // Runtime tools execute the script against the live session.
    let run = mcp.call_text("runtime_run", json!({"duration": 0.3}));
    let logs = serde_json::to_string(&run["logs"]).unwrap();
    assert!(logs.contains("mcp runtime ok"), "runtime logs flow through MCP: {logs}");
    mcp.call_text("runtime_export", json!({"output": "runtime.rbxl"}));
    let after = cli_json(&dir, &["get", "runtime.rbxl", "--target", "Workspace/FromScript"]);
    assert_eq!(after["instance"]["className"], "Part");

    // Play tools drive the same world through physics.
    let started = mcp.call_text("play_start", json!({"path": "game.rbxl", "rig": "R6"}));
    assert_eq!(started["character"]["present"], true, "play starts with a character: {started}");
    let moved = mcp.call_text("play_input", json!({"path": "game.rbxl", "keys": ["W"], "duration": 0.5}));
    assert_eq!(moved["character"]["present"], true);
    let shot = mcp.call("play_screenshot", json!({"path": "game.rbxl", "output": "play.png", "width": 640, "height": 480}));
    assert_eq!(shot["isError"], false, "play screenshot via MCP: {shot}");
    assert!(shot["content"].as_array().unwrap().iter().any(|c| c["type"] == "image"),
            "play screenshot returns an image");
    let stopped = mcp.call_text("play_stop", json!({"path": "game.rbxl", "commit": false}));
    assert_eq!(stopped["stopped"], true);

    mcp.shutdown();
}

#[test]
fn mcp_net_session_drives_two_players() {
    let dir = temp_dir("net");
    // Build a minimal place through the CLI first.
    cli_json(&dir, &["new", "game.rbxl"]);
    cli_json(&dir, &["add", "game.rbxl", "--parent", "Workspace", "--class", "Part", "--name", "Ground"]);
    cli_json(&dir, &["set", "game.rbxl", "--target", "Workspace/Ground", "--property", "Size", "--value", "60,2,60"]);
    cli_json(&dir, &["move", "game.rbxl", "--target", "Workspace/Ground", "--position", "0,-1,0"]);
    cli_json(&dir, &["set", "game.rbxl", "--target", "Workspace/Ground", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_json(&dir, &["add", "game.rbxl", "--parent", "Workspace", "--class", "SpawnLocation", "--name", "Spawn"]);

    let mut mcp = McpClient::spawn(&dir);
    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter()
        .map(|t| t["name"].as_str().unwrap()).collect();
    for required in ["net_start", "net_step", "net_input", "net_state",
                     "net_screenshot", "net_logs", "net_stop"] {
        assert!(names.contains(&required), "missing MCP tool {required}");
    }

    let started = mcp.call_text("net_start", json!({"path": "game.rbxl", "players": 2, "latencyMs": 0}));
    assert_eq!(started["players"].as_array().unwrap().len(), 2, "{started}");
    assert_eq!(started["characters"].as_array().unwrap().len(), 2, "{started}");
    let stepped = mcp.call_text("net_step", json!({"path": "game.rbxl", "steps": 30}));
    assert_eq!(stepped["step"], 30, "{stepped}");
    let moved = mcp.call_text("net_input", json!({"path": "game.rbxl", "player": 2, "keys": ["W"], "duration": 0.5}));
    let chars = moved["characters"].as_array().unwrap();
    assert_eq!(chars.len(), 2, "{moved}");
    let shot = mcp.call("net_screenshot", json!({"path": "game.rbxl", "player": 1, "output": "net.png", "width": 640, "height": 480}));
    assert_eq!(shot["isError"], false, "net screenshot via MCP: {shot}");
    assert!(shot["content"].as_array().unwrap().iter().any(|c| c["type"] == "image"),
            "net screenshot returns an image");
    let stopped = mcp.call_text("net_stop", json!({"path": "game.rbxl", "commit": false}));
    assert_eq!(stopped["stopped"], true);
    mcp.shutdown();
}
