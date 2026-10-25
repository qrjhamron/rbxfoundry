//! Persistent Luau debug sessions (Phase 23).
//!
//! `blox debug start` launches a detached daemon (`bloxcli
//! debug-daemon`) that owns one engine process with a runtime begun
//! on a copy of the place. Breakpoints live in the daemon and are
//! passed to the engine's real Luau VM debugger (`debug/begin`,
//! `debug/resume`, `debug/state` RPCs backed by `lua_breakpoint`,
//! `lua_singlestep` and `lua_getinfo`/`lua_getlocal`). The session
//! directory `<place>.bloxdebug/` holds only the socket and log.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value as JsonValue};

use crate::engine::EngineClient;
use crate::rbxio::read_place;
use crate::tree::{err, Error, Result};

pub fn session_dir(place: &Path) -> PathBuf {
    let name = place
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "place".to_string());
    place.with_file_name(format!("{name}.bloxdebug"))
}

pub fn socket_path(place: &Path) -> PathBuf {
    session_dir(place).join("session.sock")
}

pub fn send_request(place: &Path, request: &JsonValue) -> Result<JsonValue> {
    let socket = socket_path(place);
    let stream = UnixStream::connect(&socket).map_err(|e| {
        Error(format!(
            "no live debug session for '{}' ({}): {e}; start one with `blox debug start`",
            place.display(),
            socket.display()
        ))
    })?;
    let mut writer = stream.try_clone().map_err(|e| Error(e.to_string()))?;
    let mut line = serde_json::to_string(request).map_err(|e| Error(e.to_string()))?;
    line.push('\n');
    writer.write_all(line.as_bytes()).map_err(|e| Error(e.to_string()))?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).map_err(|e| Error(e.to_string()))?;
    if response.trim().is_empty() {
        return err("debug daemon closed the connection without responding");
    }
    serde_json::from_str(&response).map_err(|e| Error(format!("bad daemon response: {e}")))
}

pub fn session_alive(place: &Path) -> bool {
    socket_path(place).exists() && send_request(place, &json!({"cmd": "ping"})).is_ok()
}

pub fn start_daemon(engine_path: &Path, place: &Path) -> Result<JsonValue> {
    let dir = session_dir(place);
    if session_alive(place) {
        return err(format!(
            "a debug session is already running for '{}' (stop it first)",
            place.display()
        ));
    }
    std::fs::create_dir_all(&dir).map_err(|e| Error(e.to_string()))?;
    let socket = socket_path(place);
    let _ = std::fs::remove_file(&socket);
    let log_file = std::fs::File::create(dir.join("daemon.log")).map_err(|e| Error(e.to_string()))?;
    let log_err = log_file.try_clone().map_err(|e| Error(e.to_string()))?;
    let exe = std::env::current_exe().map_err(|e| Error(e.to_string()))?;
    let mut cmd = std::process::Command::new("setsid");
    cmd.arg("-f")
        .arg(exe)
        .arg("--engine")
        .arg(engine_path)
        .arg("debug-daemon")
        .arg(place)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_err));
    let status = cmd.status().map_err(|e| Error(format!("failed to launch debug daemon (setsid): {e}")))?;
    if !status.success() {
        return err("debug daemon launcher exited with an error; see daemon.log in the session dir");
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if session_alive(place) {
            return send_request(place, &json!({"cmd": "state"}));
        }
        if Instant::now() > deadline {
            let log = std::fs::read_to_string(dir.join("daemon.log")).unwrap_or_default();
            return err(format!("debug daemon did not come up within 20s. daemon.log:\n{log}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn run_daemon(engine_path: &Path, place: &Path) -> Result<()> {
    let dir = session_dir(place);
    std::fs::create_dir_all(&dir).map_err(|e| Error(e.to_string()))?;
    let socket = socket_path(place);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).map_err(|e| Error(e.to_string()))?;

    let (node, _format) = read_place(place)?;
    let mut engine = EngineClient::spawn(engine_path)?;
    engine.load(&node)?;
    engine.call("runtime/begin", json!({}))?;

    let mut breakpoints: Vec<JsonValue> = Vec::new();
    let mut begun = false;

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut line = String::new();
        {
            let mut reader = BufReader::new(stream.try_clone().map_err(|e| Error(e.to_string()))?);
            if reader.read_line(&mut line).is_err() {
                continue;
            }
        }
        let request: JsonValue = serde_json::from_str(&line).unwrap_or(JsonValue::Null);
        let cmd = request["cmd"].as_str().unwrap_or("");
        let response: JsonValue = match cmd {
            "ping" => json!({"ok": true, "pong": true}),
            "breakpoints" => {
                if let Some(list) = request.get("breakpoints").and_then(|v| v.as_array()) {
                    breakpoints = list.clone();
                }
                json!({"ok": true, "breakpoints": breakpoints})
            }
            "run" => {
                // Restart the runtime so every debug run is fresh.
                let restart = engine
                    .call("runtime/reset", json!({}))
                    .and_then(|_| engine.call("runtime/begin", json!({})))
                    .and_then(|_| {
                        engine.call(
                            "debug/begin",
                            json!({"breakpoints": breakpoints}),
                        )
                    });
                match restart {
                    Ok(state) => {
                        begun = true;
                        json!({"ok": true, "state": state})
                    }
                    Err(e) => json!({"ok": false, "error": e.0}),
                }
            }
            "resume" => {
                if !begun {
                    json!({"ok": false, "error": "nothing to resume: run the scripts first (`debug run`)"})
                } else {
                    let mode = request["mode"].as_str().unwrap_or("continue");
                    match engine.call("debug/resume", json!({"mode": mode})) {
                        Ok(state) => json!({"ok": true, "state": state}),
                        Err(e) => json!({"ok": false, "error": e.0}),
                    }
                }
            }
            "state" => {
                if !begun {
                    json!({"ok": true, "state": {"debug": {"paused": false, "finished": false}, "active": true}})
                } else {
                    let frame = request["frame"].as_i64().unwrap_or(0);
                    match engine.call("debug/state", json!({"frame": frame})) {
                        Ok(state) => json!({"ok": true, "state": state}),
                        Err(e) => json!({"ok": false, "error": e.0}),
                    }
                }
            }
            "stop" => {
                engine.shutdown();
                let _ = stream.write_all(b"{\"ok\":true,\"stopped\":true}\n");
                let _ = std::fs::remove_file(&socket);
                return Ok(());
            }
            other => json!({"ok": false, "error": format!("unknown debug daemon command '{other}'")}),
        };
        let mut text = serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        let _ = stream.write_all(text.as_bytes());
    }
    Ok(())
}
