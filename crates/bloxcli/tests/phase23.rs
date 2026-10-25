//! Phase 23 integration tests: Luau language tooling (official
//! frontend analysis, reflection-based completion) and the real VM
//! debugger (breakpoints, stepping, stack, locals) over a debug
//! session daemon.

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
    let dir = std::env::temp_dir().join(format!("bloxcli-phase23-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn place_with_script(dir: &PathBuf, source: &str) -> String {
    place_with_script_named(dir, "game.rbxl", source)
}

fn place_with_script_named(dir: &PathBuf, name: &str, source: &str) -> String {
    let file = dir.join(name);
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value", source]);
    f
}

#[test]
fn script_analyze_reports_syntax_and_type_errors() {
    let dir = temp_dir("analyze");
    // Syntax error: unclosed block.
    let bad = dir.join("bad.luau");
    std::fs::write(&bad, "local x = 1\nif x then\n    print(x)\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json")
        .arg("--engine")
        .arg(engine_path())
        .args(["script", "analyze", bad.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success(), "syntax errors fail analyze");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let diags = report["diagnostics"].as_array().unwrap();
    assert!(diags.iter().any(|d| d["kind"] == "syntax" && d["severity"] == "error"), "{report}");
    let syntax = diags.iter().find(|d| d["kind"] == "syntax").unwrap();
    assert!(syntax["line"].as_i64().unwrap() >= 1, "span reported: {syntax}");

    // Type error: string assigned where a number operation happens.
    let typed = dir.join("typed.luau");
    std::fs::write(&typed, "--!strict\nlocal n: number = 'text'\nlocal m = n + 1\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json")
        .arg("--engine")
        .arg(engine_path())
        .args(["script", "analyze", typed.to_str().unwrap()])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        report["diagnostics"].as_array().unwrap().iter().any(|d| d["kind"] == "type"),
        "type error from the official frontend: {report}"
    );

    // Clean Luau passes.
    let good = dir.join("good.luau");
    std::fs::write(&good, "local n: number = 41\nlocal m = n + 1\nprint(m)\n").unwrap();
    let ok = cli_json(&["script", "analyze", good.to_str().unwrap()]);
    assert_eq!(ok["ok"], true, "{ok}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn script_complete_uses_reflection_metadata() {
    let dir = temp_dir("complete");
    let source = "local ws = game:GetService(\"Workspace\")\nlocal p = Instance.new(\"Part\")\nws.\np.";
    let f = place_with_script(&dir, source);
    // Cursor after "ws." (line 3, column 4).
    let result = cli_json(&["script", "complete", &f, "--target", "ServerScriptService/Main", "--line", "3", "--column", "4"]);
    let labels: Vec<&str> = result["completions"].as_array().unwrap().iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"Gravity"), "Workspace property completion: {labels:?}");
    // Cursor after "p." (line 4, column 3) → Part properties.
    let result = cli_json(&["script", "complete", &f, "--target", "ServerScriptService/Main", "--line", "4", "--column", "3"]);
    let labels: Vec<&str> = result["completions"].as_array().unwrap().iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"Anchored") && labels.contains(&"Position"), "Part completions: {labels:?}");
    // Inside Instance.new(" → class names.
    let source2 = "local p = Instance.new(\"Pa";
    let f2 = place_with_script_named(&dir, "game2.rbxl", source2);
    let result = cli_json(&["script", "complete", &f2, "--target", "ServerScriptService/Main", "--line", "1", "--column", "25"]);
    let labels: Vec<&str> = result["completions"].as_array().unwrap().iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"Part"), "class name completion: {labels:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

struct DebugSession(String);
impl Drop for DebugSession {
    fn drop(&mut self) {
        let _ = cli(&["debug", "stop", &self.0]);
    }
}

#[test]
fn debugger_breakpoints_stepping_and_locals() {
    let dir = temp_dir("debugger");
    let source = "local counter = 0\nlocal function bump(x)\n    counter = counter + x\n    return counter\nend\nlocal result = bump(5)\nprint(\"RESULT=\" .. result)";
    let f = place_with_script(&dir, source);
    let _session = DebugSession(f.clone());

    cli_ok(&["debug", "start", &f]);
    cli_ok(&["debug", "breakpoint", "add", &f, "ServerScriptService/Main:6"]);

    let run = cli_json(&["debug", "run", &f]);
    let dbg = &run["state"]["debug"];
    assert_eq!(dbg["paused"], true, "{run}");
    assert_eq!(dbg["line"], 6, "paused on the call line: {run}");
    assert_eq!(dbg["reason"], "breakpoint");

    let into = cli_json(&["debug", "step-into", &f]);
    let dbg = &into["state"]["debug"];
    assert_eq!(dbg["line"], 3, "stepped into bump: {into}");
    assert_eq!(dbg["stack"][0]["function"], "bump", "{into}");
    let vars = format!("{}", dbg["variables"]);
    assert!(vars.contains("\"x\"") && vars.contains("5"), "argument visible: {into}");
    assert!(vars.contains("counter"), "upvalue visible: {into}");

    let over = cli_json(&["debug", "step-over", &f]);
    let dbg = &over["state"]["debug"];
    assert_eq!(dbg["line"], 4, "stepped over the increment: {over}");
    let vars = format!("{}", dbg["variables"]);
    assert!(vars.contains("5"), "counter updated to 5: {over}");

    let out = cli_json(&["debug", "step-out", &f]);
    let dbg = &out["state"]["debug"];
    assert_eq!(dbg["line"], 7, "stepped out to the print: {out}");

    let cont = cli_json(&["debug", "continue", &f]);
    let dbg = &cont["state"]["debug"];
    assert_eq!(dbg["finished"], true, "{cont}");
    assert!(format!("{}", cont["state"]["logs"]).contains("RESULT=5"), "script completed: {cont}");
    let _ = std::fs::remove_dir_all(&dir);
}
