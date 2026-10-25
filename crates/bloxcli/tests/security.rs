//! Gate A1 adversarial MCP filesystem-security tests.
//!
//! Every escape must be refused with a sandbox error and must not
//! create or overwrite anything outside the MCP root: `..` chains
//! (including through not-yet-existing directories), absolute paths,
//! and symlink components pointing outside.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value as JsonValue};

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") { if !p.is_empty() { return PathBuf::from(p); } }
    let c = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(c.is_file(), "engine at {}", c.display());
    c
}

struct Mcp { child: Child, stdin: std::process::ChildStdin, stdout: BufReader<std::process::ChildStdout>, id: u64 }
impl Mcp {
    fn spawn(root: &PathBuf) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
            .arg("--engine").arg(engine_path()).arg("mcp").arg("--root").arg(root)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().expect("spawn mcp");
        let mut m = Mcp { stdin: child.stdin.take().unwrap(), stdout: BufReader::new(child.stdout.take().unwrap()), child, id: 0 };
        m.request("initialize", json!({"protocolVersion": "2025-03-26"}));
        m
    }
    fn request(&mut self, method: &str, params: JsonValue) -> JsonValue {
        self.id += 1;
        writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})).unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn call(&mut self, tool: &str, args: JsonValue) -> JsonValue {
        self.request("tools/call", json!({"name": tool, "arguments": args}))["result"].clone()
    }
    fn shutdown(mut self) { drop(self.stdin); let _ = self.child.wait(); }
}

#[test]
fn mcp_filesystem_escapes_are_refused() {
    let base = std::env::temp_dir().join(format!("bloxcli-sec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let mut mcp = Mcp::spawn(&root);

    // Legit nested creation inside the root works.
    let ok = mcp.call("studio_new", json!({"path": "sub/game.rbxl", "name": "Safe"}));
    assert_eq!(ok["isError"], false, "nested create inside root: {ok}");
    assert!(root.join("sub/game.rbxl").is_file());

    for (tool, args) in [
        ("studio_new", json!({"path": "../escape.rbxl", "name": "X"})),
        ("studio_new", json!({"path": "a/../../escape.rbxl", "name": "X"})),
        ("studio_new", json!({"path": "link/evil.rbxl", "name": "X"})),
        ("studio_new", json!({"path": outside.join("abs.rbxl").to_string_lossy(), "name": "X"})),
        ("studio_export", json!({"output": "a/../../../escape-export.rbxl"})),
        ("studio_export", json!({"output": "link/exported.rbxl"})),
        ("studio_open", json!({"path": "../escape.rbxl"})),
    ] {
        let result = mcp.call(tool, args.clone());
        assert_eq!(result["isError"], true, "{tool} {args} must be refused");
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("escape"), "refusal names the sandbox: {text}");
    }
    assert!(!base.join("escape.rbxl").exists(), "no file created above the root");
    assert!(!base.join("escape-export.rbxl").exists(), "export did not escape");
    assert!(outside.read_dir().unwrap().next().is_none(), "nothing written through the symlink: {:?}", outside.read_dir().unwrap().collect::<Vec<_>>());
    mcp.shutdown();
}
