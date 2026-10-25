//! Persistent playtest sessions (Phase 6).
//!
//! `blox play start` launches a detached daemon (`bloxcli play-daemon`)
//! that owns one engine process: the Luau runtime runs on a copy of the
//! place, Jolt physics steps the character and dynamic parts, and every
//! command is applied to that live session over a Unix socket. The
//! session directory `<place>.bloxplay/` holds the socket, the latest
//! runtime snapshot (`state.rbxl`, rewritten after every command) and
//! the daemon log. Nothing here teleports the character: movement comes
//! from the physics character controller responding to held keys.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value as JsonValue};

use crate::protocol::engine::{node_from_value, EngineClient};
use crate::history::History;
use crate::rbxio::{read_place, write_place};
use crate::protocol::tree::{err, Error, Result};

pub const FIXED_DT: f64 = 1.0 / 60.0;

pub fn session_dir(place: &Path) -> PathBuf {
    let name = place
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "place".to_string());
    place.with_file_name(format!("{name}.bloxplay"))
}

pub fn socket_path(place: &Path) -> PathBuf {
    session_dir(place).join("session.sock")
}

fn state_path(place: &Path) -> PathBuf {
    session_dir(place).join("state.rbxl")
}

// -------------------------------------------------------------- client ---

pub fn send_request(place: &Path, request: &JsonValue) -> Result<JsonValue> {
    let socket = socket_path(place);
    let stream = UnixStream::connect(&socket).map_err(|e| {
        Error(format!(
            "no live play session for '{}' ({}): {e}; start one with `blox play start`",
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
        return err("play daemon closed the connection without responding");
    }
    serde_json::from_str(&response).map_err(|e| Error(format!("bad daemon response: {e}")))
}

pub fn session_alive(place: &Path) -> bool {
    if !socket_path(place).exists() {
        return false;
    }
    send_request(place, &json!({"cmd": "ping"})).is_ok()
}

/// Spawns the detached daemon and waits for its socket to answer.
pub fn start_daemon(
    engine_path: &Path,
    place: &Path,
    rig: &str,
    force: bool,
) -> Result<JsonValue> {
    let dir = session_dir(place);
    if session_alive(place) {
        if !force {
            return err(format!(
                "a play session is already running for '{}' (stop it first, or pass force)",
                place.display()
            ));
        }
        let _ = send_request(place, &json!({"cmd": "stop", "commit": false}));
        std::thread::sleep(Duration::from_millis(200));
    }
    std::fs::create_dir_all(&dir).map_err(|e| Error(e.to_string()))?;
    let socket = socket_path(place);
    let _ = std::fs::remove_file(&socket);
    let log_file = std::fs::File::create(dir.join("daemon.log")).map_err(|e| Error(e.to_string()))?;
    let log_err = log_file.try_clone().map_err(|e| Error(e.to_string()))?;
    let exe = std::env::current_exe().map_err(|e| Error(e.to_string()))?;
    // Detached in its own session so the daemon survives this CLI call.
    let mut cmd = std::process::Command::new("setsid");
    cmd.arg("-f")
        .arg(exe)
        .arg("--engine")
        .arg(engine_path)
        .arg("play-daemon")
        .arg(place)
        .arg("--rig")
        .arg(rig)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_err));
    let status = cmd.status().map_err(|e| Error(format!("failed to launch play daemon (setsid): {e}")))?;
    if !status.success() {
        return err("play daemon launcher exited with an error; see daemon.log");
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if session_alive(place) {
            return send_request(place, &json!({"cmd": "state"}));
        }
        if Instant::now() > deadline {
            let log = std::fs::read_to_string(dir.join("daemon.log")).unwrap_or_default();
            return err(format!(
                "play daemon did not come up within 20s; daemon.log tail: {}",
                log.chars().rev().take(400).collect::<String>().chars().rev().collect::<String>()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// -------------------------------------------------------------- daemon ---

struct Daemon {
    engine: EngineClient,
    place: PathBuf,
    rig: String,
    yaw: f64,   // camera yaw, radians
    pitch: f64, // camera pitch, radians (positive = look down)
    first_person: bool,
    held_keys: Vec<String>,
    logs: Vec<JsonValue>,
    log_cursor: usize,
}

pub fn run_daemon(engine_path: &Path, place: &Path, rig: &str) -> Result<()> {
    let dir = session_dir(place);
    std::fs::create_dir_all(&dir).map_err(|e| Error(e.to_string()))?;
    let (node, _format) = read_place(place)?;
    let mut engine = EngineClient::spawn(engine_path)?;
    engine.load(&node)?;
    engine.call("runtime/begin", json!({}))?;
    let started = engine.call("runtime/runScripts", json!({}))?;
    let physics = engine.call("physics/begin", json!({"rig": rig}))?;

    let mut daemon = Daemon {
        engine,
        place: place.to_path_buf(),
        rig: rig.to_string(),
        yaw: physics["character"]["yaw"].as_f64().unwrap_or(0.0),
        pitch: 0.35,
        first_person: false,
        held_keys: Vec::new(),
        logs: Vec::new(),
        log_cursor: 0,
    };
    daemon.absorb_logs(&started);
    daemon.persist_snapshot()?;

    let socket = socket_path(place);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).map_err(|e| Error(e.to_string()))?;
    eprintln!("play daemon ready for {} (rig {})", place.display(), rig);
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        match daemon.handle(stream) {
            Ok(true) => break, // stop requested
            Ok(false) => {}
            Err(e) => eprintln!("play daemon: request failed: {e}"),
        }
    }
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

impl Daemon {
    fn absorb_logs(&mut self, state: &JsonValue) {
        if let Some(logs) = state.get("logs").and_then(|v| v.as_array()) {
            for entry in logs.iter().skip(self.log_cursor) {
                self.logs.push(entry.clone());
            }
            self.log_cursor = logs.len();
            if self.logs.len() > 500 {
                let excess = self.logs.len() - 500;
                self.logs.drain(0..excess);
            }
        }
    }

    fn persist_snapshot(&mut self) -> Result<()> {
        let snap = self.engine.call("runtime/snapshot", json!({}))?;
        let tree_json = snap.get("tree").ok_or_else(|| Error("runtime snapshot missing tree".into()))?;
        let tree = node_from_value(tree_json)?;
        write_place(&state_path(&self.place), &tree)?;
        Ok(())
    }

    fn state_json(&mut self) -> Result<JsonValue> {
        let state = self.engine.call("physics/state", json!({}))?;
        Ok(json!({
            "playing": true,
            "rig": self.rig,
            "time": state.get("time"),
            "bodies": state.get("bodies"),
            "character": state.get("character"),
            "camera": {"yawDeg": self.yaw.to_degrees(), "pitchDeg": self.pitch.to_degrees(),
                       "mode": if self.first_person { "first-person" } else { "third-person" }},
            "heldKeys": self.held_keys,
            "logCount": self.logs.len(),
        }))
    }

    fn apply_keys(&mut self, keys: &[String], steps: i64) -> Result<JsonValue> {
        // Feed UserInputService transitions.
        let mut changes = json!({});
        for key in keys {
            if !self.held_keys.contains(key) {
                changes[key] = json!(true);
            }
        }
        for key in &self.held_keys {
            if !keys.contains(key) {
                changes[key] = json!(false);
            }
        }
        if changes.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            self.engine.call("runtime/keys", json!({"keys": changes}))?;
        }
        self.held_keys = keys.to_vec();
        let has = |name: &str| keys.iter().any(|k| k == name);
        let forward = (if has("W") { 1.0 } else { 0.0 }) - (if has("S") { 1.0 } else { 0.0 });
        let right = (if has("D") { 1.0 } else { 0.0 }) - (if has("A") { 1.0 } else { 0.0 });
        let result = self.engine.call("physics/step", json!({
            "dt": FIXED_DT,
            "steps": steps,
            "input": {"move": [right, forward], "jump": has("Space"), "yaw": self.yaw},
        }))?;
        self.absorb_logs(&result);
        self.persist_snapshot()?;
        let mut out = self.state_json()?;
        out["events"] = result.get("events").cloned().unwrap_or(json!([]));
        out["errors"] = result.get("errors").cloned().unwrap_or(json!([]));
        Ok(out)
    }

    fn handle(&mut self, stream: UnixStream) -> Result<bool> {
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| Error(e.to_string()))?);
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| Error(e.to_string()))?;
        let request: JsonValue = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = json!({"ok": false, "error": format!("bad request: {e}")});
                writeln!(writer, "{resp}").map_err(|e| Error(e.to_string()))?;
                return Ok(false);
            }
        };
        let cmd = request.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        let (response, stop) = match self.dispatch(cmd, &request) {
            Ok((resp, stop)) => (resp, stop),
            Err(e) => (json!({"ok": false, "error": e.to_string()}), false),
        };
        writeln!(writer, "{response}").map_err(|e| Error(e.to_string()))?;
        Ok(stop)
    }

    fn dispatch(&mut self, cmd: &str, request: &JsonValue) -> Result<(JsonValue, bool)> {
        match cmd {
            "ping" => Ok((json!({"ok": true, "pong": true}), false)),
            "state" => Ok((self.state_json()?, false)),
            "step" => {
                let steps = request.get("steps").and_then(|v| v.as_i64()).unwrap_or(1).max(0);
                let keys = self.held_keys.clone();
                Ok((self.apply_keys(&keys, steps)?, false))
            }
            "gui" => {
                // Pointer/text input against the runtime GUI (Phase 22).
                let kind = request.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                let width = request.get("width").and_then(|v| v.as_i64()).unwrap_or(1280);
                let height = request.get("height").and_then(|v| v.as_i64()).unwrap_or(720);
                let result = match kind {
                    "click" => self.engine.call("runtime/click", json!({
                        "x": request.get("x").cloned().unwrap_or(json!(0)),
                        "y": request.get("y").cloned().unwrap_or(json!(0)),
                        "width": width, "height": height,
                    }))?,
                    "hover" => self.engine.call("runtime/hover", json!({
                        "x": request.get("x").cloned().unwrap_or(json!(0)),
                        "y": request.get("y").cloned().unwrap_or(json!(0)),
                        "width": width, "height": height,
                    }))?,
                    "typeText" => self.engine.call("runtime/typeText", json!({
                        "text": request.get("text").cloned().unwrap_or(json!("")),
                        "submit": request.get("submit").cloned().unwrap_or(json!(false)),
                    }))?,
                    other => return err(format!("unknown gui input kind '{other}'")),
                };
                self.absorb_logs(&result);
                Ok((result, false))
            }
            "input" => {
                if let Some(yaw) = request.get("yawDeg").and_then(|v| v.as_f64()) {
                    self.yaw = yaw.to_radians();
                }
                if let Some(pitch) = request.get("pitchDeg").and_then(|v| v.as_f64()) {
                    self.pitch = pitch.to_radians().clamp(-1.2, 1.2);
                }
                if let Some(fp) = request.get("firstPerson").and_then(|v| v.as_bool()) {
                    self.first_person = fp;
                }
                let keys: Vec<String> = request
                    .get("keys")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|k| k.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                let duration = request.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let steps = (duration * 60.0).round() as i64;
                let result = self.apply_keys(&keys, steps.max(0))?;
                // Keys are held for the requested duration only: release
                // them afterwards (plain `step` continues key-free).
                if !self.held_keys.is_empty() {
                    let mut release = json!({});
                    for key in &self.held_keys {
                        release[key] = json!(false);
                    }
                    self.engine.call("runtime/keys", json!({"keys": release}))?;
                    self.held_keys.clear();
                }
                Ok((result, false))
            }
            "screenshot" => Ok((self.screenshot(request)?, false)),
            "logs" => Ok((json!({"ok": true, "logs": self.logs}), false)),
            "stop" => {
                let commit = request.get("commit").and_then(|v| v.as_bool()).unwrap_or(false);
                if commit {
                    let snap = self.engine.call("runtime/snapshot", json!({}))?;
                    let tree_json = snap.get("tree").ok_or_else(|| Error("snapshot missing tree".into()))?;
                    let tree = node_from_value(tree_json)?;
                    let mut history = History::open(&self.place)?;
                    history.record(&self.place)?;
                    write_place(&self.place, &tree)?;
                }
                // The engine exits when the daemon process ends (stdin
                // closes); respond first, then let run_daemon return.
                Ok((json!({"ok": true, "stopped": true, "committed": commit}), true))
            }
            other => Err(Error(format!("unknown play command '{other}'"))),
        }
    }

    fn screenshot(&mut self, request: &JsonValue) -> Result<JsonValue> {
        let output = request
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error("screenshot requires 'output'".into()))?;
        let width = request.get("width").and_then(|v| v.as_i64()).unwrap_or(1280);
        let height = request.get("height").and_then(|v| v.as_i64()).unwrap_or(720);
        let state = self.engine.call("physics/state", json!({}))?;
        let character = state.get("character").cloned().unwrap_or(json!({}));
        let pos = character.get("position").and_then(|v| v.as_array())
            .ok_or_else(|| Error("character not present".into()))?;
        let (cx, cy, cz) = (pos[0].as_f64().unwrap_or(0.0), pos[1].as_f64().unwrap_or(0.0), pos[2].as_f64().unwrap_or(0.0));
        let (sy, cyaw) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        // Forward the camera faces (yaw 0 = -Z, pitch > 0 looks down).
        let forward = (-sy * cp, -sp, -cyaw * cp);
        let (eye, look) = if self.first_person {
            ((cx, cy + 2.2, cz), (cx + forward.0 * 10.0, cy + 2.2 + forward.1 * 10.0, cz + forward.2 * 10.0))
        } else {
            (
                (cx - forward.0 * 11.0, cy + 3.0 - forward.1 * 11.0, cz - forward.2 * 11.0),
                (cx, cy + 1.5, cz),
            )
        };
        // Render from the runtime snapshot: swap the edit model (the
        // runtime model is untouched by datamodel/load).
        let snap = self.engine.call("runtime/snapshot", json!({}))?;
        let tree_json = snap.get("tree").ok_or_else(|| Error("snapshot missing tree".into()))?;
        let tree = node_from_value(tree_json)?;
        self.engine.load(&tree)?;
        let place_dir = self.place.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let result = self.engine.render_screenshot(json!({
            "width": width, "height": height, "outputPath": output,
            "assetsDir": place_dir, "placeDir": place_dir, "shadows": true,
            "camera": {"position": [eye.0, eye.1, eye.2], "lookAt": [look.0, look.1, look.2]},
        }))?;
        Ok(json!({
            "ok": true, "output": output, "width": width, "height": height,
            "camera": {"position": [eye.0, eye.1, eye.2], "lookAt": [look.0, look.1, look.2]},
            "character": character,
            "device": result.get("device"),
        }))
    }
}