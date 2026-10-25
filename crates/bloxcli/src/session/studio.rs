//! Persistent Studio editing sessions (Phase 25).
//!
//! `blox studio start <place>` launches a detached daemon
//! (`bloxcli studio-daemon`) that owns one long-lived C++ engine
//! process holding the authoritative edit DataModel in memory.
//! Independent CLI invocations and MCP attach to the same daemon over
//! a Unix socket in `<place>.bloxstudio/`; there is no second
//! DataModel and the place file is never rewritten except by an
//! explicit `studio save` (or `studio stop --save`).
//!
//! Editor state kept in the daemon: the loaded tree, selection,
//! camera, dirty flag, and in-memory undo/redo snapshot stacks.
//! Every mutation also writes an atomic recovery snapshot
//! (`recovery.rbxl`, sequence-numbered in `session.json`) so a killed
//! daemon can be restarted and its unsaved edits recovered without
//! ever touching the original file. Save detects external edits to
//! the place file (content hash recorded at load/last save) and
//! refuses to silently overwrite them.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value as JsonValue};

use crate::app::session::write_mutation;
use crate::protocol::engine::EngineClient;
use crate::protocol::tree::{err, Error, Node, Result};
use crate::rbxio::{read_place, write_place};

pub fn session_dir(place: &Path) -> PathBuf {
    let name = place
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "place".to_string());
    place.with_file_name(format!("{name}.bloxstudio"))
}
pub fn socket_path(place: &Path) -> PathBuf {
    session_dir(place).join("session.sock")
}
fn meta_path(place: &Path) -> PathBuf {
    session_dir(place).join("session.json")
}
fn recovery_path(place: &Path) -> PathBuf {
    session_dir(place).join("recovery.rbxl")
}

fn file_hash(path: &Path) -> u64 {
    // FNV-1a over the file bytes; used only for conflict detection.
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            h
        }
        Err(_) => 0,
    }
}

// -------------------------------------------------------------- client ---

pub fn send_request(place: &Path, request: &JsonValue) -> Result<JsonValue> {
    let socket = socket_path(place);
    let stream = UnixStream::connect(&socket).map_err(|e| {
        Error(format!(
            "no live studio session for '{}' ({}): {e}; start one with `blox studio start`",
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
        return err("studio daemon closed the connection without responding");
    }
    let value: JsonValue = serde_json::from_str(&response).map_err(|e| Error(format!("bad daemon response: {e}")))?;
    if value.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return err(value.get("error").and_then(|v| v.as_str()).unwrap_or("studio session error").to_string());
    }
    Ok(value)
}

pub fn session_alive(place: &Path) -> bool {
    socket_path(place).exists() && send_request(place, &json!({"cmd": "ping"})).is_ok()
}

pub fn read_meta(place: &Path) -> Option<JsonValue> {
    std::fs::read_to_string(meta_path(place)).ok().and_then(|s| serde_json::from_str(&s).ok())
}

pub fn start_daemon(engine_path: &Path, place: &Path, force: bool) -> Result<JsonValue> {
    let dir = session_dir(place);
    if session_alive(place) {
        if !force {
            return err(format!(
                "a studio session is already running for '{}' (stop it first, or pass --force)",
                place.display()
            ));
        }
        let _ = send_request(place, &json!({"cmd": "stop", "save": false}));
        std::thread::sleep(Duration::from_millis(200));
    }
    std::fs::create_dir_all(&dir).map_err(|e| Error(e.to_string()))?;
    let socket = socket_path(place);
    let _ = std::fs::remove_file(&socket);
    let log_file = std::fs::File::create(dir.join("daemon.log")).map_err(|e| Error(e.to_string()))?;
    let log_err = log_file.try_clone().map_err(|e| Error(e.to_string()))?;
    let exe = std::env::current_exe().map_err(|e| Error(e.to_string()))?;
    let mut cmd = std::process::Command::new("setsid");
    cmd.arg("-f").arg(exe).arg("--engine").arg(engine_path).arg("studio-daemon").arg(place)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_err));
    let status = cmd.status().map_err(|e| Error(format!("failed to launch studio daemon (setsid): {e}")))?;
    if !status.success() {
        return err("studio daemon launcher exited with an error; see daemon.log");
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if session_alive(place) {
            return send_request(place, &json!({"cmd": "status"}));
        }
        if Instant::now() > deadline {
            let log = std::fs::read_to_string(dir.join("daemon.log")).unwrap_or_default();
            return err(format!("studio daemon did not come up within 20s; daemon.log tail: {}", log.chars().rev().take(400).collect::<String>().chars().rev().collect::<String>()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// -------------------------------------------------------------- daemon ---

struct Daemon {
    engine: EngineClient,
    place: PathBuf,
    session_id: String,
    selection: Vec<String>,
    camera: JsonValue,
    dirty: bool,
    recovered: bool,
    seq: u64,
    saved_hash: u64,
    undo: Vec<Node>,
    redo: Vec<Node>,
}

pub fn run_daemon(engine_path: &Path, place: &Path) -> Result<()> {
    let dir = session_dir(place);
    std::fs::create_dir_all(&dir).map_err(|e| Error(e.to_string()))?;
    // Recovery: a previous daemon died with unsaved edits. Restore the
    // recovery snapshot into the new session; never write the place.
    let prev = read_meta(place);
    let was_dirty = prev.as_ref().and_then(|m| m.get("dirty")).and_then(|v| v.as_bool()).unwrap_or(false);
    let recover = was_dirty && recovery_path(place).is_file();
    let (node, _format) = if recover {
        read_place(&recovery_path(place)).map_err(|e| Error(format!("studio recovery snapshot unreadable: {e}")))?
    } else {
        read_place(place)?
    };
    let mut engine = EngineClient::spawn(engine_path)?;
    engine.load(&node)?;
    let session_id = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let mut daemon = Daemon {
        engine,
        place: place.to_path_buf(),
        session_id,
        selection: Vec::new(),
        camera: json!(null),
        dirty: recover,
        recovered: recover,
        seq: prev.as_ref().and_then(|m| m.get("seq")).and_then(|v| v.as_u64()).unwrap_or(0),
        saved_hash: file_hash(place),
        undo: Vec::new(),
        redo: Vec::new(),
    };
    daemon.write_meta()?;
    let socket = socket_path(place);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).map_err(|e| Error(e.to_string()))?;
    eprintln!("studio daemon ready for {} (recovered={})", place.display(), daemon.recovered);
    for stream in listener.incoming() {
        let stream = match stream { Ok(s) => s, Err(_) => continue };
        match daemon.handle(stream) {
            Ok(true) => break,
            Ok(false) => {}
            Err(e) => eprintln!("studio daemon: request failed: {e}"),
        }
    }
    daemon.write_meta()?;
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

impl Daemon {
    fn write_meta(&self) -> Result<()> {
        let meta = json!({
            "sessionId": self.session_id, "pid": std::process::id(),
            "place": self.place.display().to_string(), "dirty": self.dirty,
            "recovered": self.recovered, "seq": self.seq,
            "selection": self.selection, "camera": self.camera,
        });
        let text = serde_json::to_string_pretty(&meta).map_err(|e| Error(e.to_string()))?;
        let tmp = meta_path(&self.place).with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| Error(e.to_string()))?;
        std::fs::rename(&tmp, meta_path(&self.place)).map_err(|e| Error(e.to_string()))?;
        Ok(())
    }

    fn status(&mut self) -> Result<JsonValue> {
        let tree = self.engine.snapshot()?;
        Ok(json!({
            "ok": true, "sessionId": self.session_id, "pid": std::process::id(),
            "place": self.place.display().to_string(), "dirty": self.dirty,
            "recovered": self.recovered, "instances": tree.count(),
            "selection": self.selection, "camera": self.camera,
            "canUndo": !self.undo.is_empty(), "canRedo": !self.redo.is_empty(),
            "undoDepth": self.undo.len(), "redoDepth": self.redo.len(), "seq": self.seq,
        }))
    }

    /// Snapshot before a mutation (undo point), clear redo, mark dirty
    /// and persist the recovery snapshot after the mutation.
    fn before_mutation(&mut self) -> Result<()> {
        let tree = self.engine.snapshot()?;
        self.undo.push(tree);
        if self.undo.len() > 100 { self.undo.remove(0); }
        self.redo.clear();
        Ok(())
    }
    fn after_mutation(&mut self) -> Result<JsonValue> {
        self.dirty = true;
        self.seq += 1;
        let tree = self.engine.snapshot()?;
        write_place(&recovery_path(&self.place), &tree)?;
        self.write_meta()?;
        Ok(json!({"ok": true, "dirty": true, "instances": tree.count(), "seq": self.seq}))
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
        let cmd = request.get("cmd").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let (response, stop) = match self.dispatch(&cmd, &request) {
            Ok((resp, stop)) => (resp, stop),
            Err(e) => (json!({"ok": false, "error": e.to_string()}), false),
        };
        writeln!(writer, "{response}").map_err(|e| Error(e.to_string()))?;
        Ok(stop)
    }

    fn dispatch(&mut self, cmd: &str, req: &JsonValue) -> Result<(JsonValue, bool)> {
        match cmd {
            "ping" => Ok((json!({"ok": true, "pong": true}), false)),
            "status" => Ok((self.status()?, false)),
            "get" => {
                let target = req.get("target").and_then(|v| v.as_str()).ok_or_else(|| Error("get needs target".into()))?;
                let node = self.engine.get(target)?;
                Ok((json!({"ok": true, "instance": serde_json::to_value(&node).map_err(|e| Error(e.to_string()))?}), false))
            }
            "snapshot" => {
                let tree = self.engine.snapshot()?;
                Ok((json!({"ok": true, "tree": serde_json::to_value(&tree).map_err(|e| Error(e.to_string()))?}), false))
            }
            "add" => {
                self.before_mutation()?;
                let r = self.engine.add(req.get("parent").and_then(|v| v.as_str()).unwrap_or(""), req.get("className").and_then(|v| v.as_str()).unwrap_or(""), req.get("name").and_then(|v| v.as_str()).unwrap_or(""))?;
                let mut out = self.after_mutation()?; out["added"] = r; Ok((out, false))
            }
            "set" => {
                let target = req.get("target").and_then(|v| v.as_str()).ok_or_else(|| Error("set needs target".into()))?.to_string();
                let property = req.get("property").and_then(|v| v.as_str()).ok_or_else(|| Error("set needs property".into()))?.to_string();
                let value: crate::protocol::tree::PropValue = serde_json::from_value(req.get("value").cloned().unwrap_or(json!(null))).map_err(|e| Error(format!("bad value: {e}")))?;
                self.before_mutation()?;
                let r = self.engine.set(&target, &property, &value)?;
                let mut out = self.after_mutation()?; out["set"] = r; Ok((out, false))
            }
            "delete" => {
                self.before_mutation()?;
                let r = self.engine.delete(req.get("target").and_then(|v| v.as_str()).unwrap_or(""))?;
                let mut out = self.after_mutation()?; out["deleted"] = r; Ok((out, false))
            }
            "clone" => {
                self.before_mutation()?;
                let r = self.engine.clone_instance(req.get("target").and_then(|v| v.as_str()).unwrap_or(""), req.get("name").and_then(|v| v.as_str()), req.get("parent").and_then(|v| v.as_str()))?;
                let mut out = self.after_mutation()?; out["cloned"] = r; Ok((out, false))
            }
            "reparent" => {
                self.before_mutation()?;
                let r = self.engine.reparent(req.get("target").and_then(|v| v.as_str()).unwrap_or(""), req.get("parent").and_then(|v| v.as_str()).unwrap_or(""))?;
                let mut out = self.after_mutation()?; out["reparented"] = r; Ok((out, false))
            }
            "undo" => {
                let Some(prev) = self.undo.pop() else { return Ok((json!({"ok": true, "undid": false}), false)); };
                let current = self.engine.snapshot()?;
                self.redo.push(current);
                self.engine.load(&prev)?;
                self.dirty = true; self.seq += 1;
                write_place(&recovery_path(&self.place), &prev)?;
                self.write_meta()?;
                Ok((json!({"ok": true, "undid": true, "instances": prev.count()}), false))
            }
            "redo" => {
                let Some(next) = self.redo.pop() else { return Ok((json!({"ok": true, "redid": false}), false)); };
                let current = self.engine.snapshot()?;
                self.undo.push(current);
                self.engine.load(&next)?;
                self.dirty = true; self.seq += 1;
                write_place(&recovery_path(&self.place), &next)?;
                self.write_meta()?;
                Ok((json!({"ok": true, "redid": true, "instances": next.count()}), false))
            }
            "select" => {
                self.selection = req.get("selection").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
                self.write_meta()?;
                Ok((json!({"ok": true, "selection": self.selection}), false))
            }
            "camera" => {
                if let Some(cam) = req.get("camera") { if !cam.is_null() { self.camera = cam.clone(); self.write_meta()?; } }
                Ok((json!({"ok": true, "camera": self.camera}), false))
            }
            "save" => {
                let force = req.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
                let current_hash = file_hash(&self.place);
                if self.place.is_file() && current_hash != self.saved_hash && !force {
                    return err("place file changed on disk since the session loaded it; refusing to overwrite (pass force to override)".to_string());
                }
                let tree = self.engine.snapshot()?;
                write_mutation(&self.place, &tree)?;
                self.saved_hash = file_hash(&self.place);
                self.dirty = false; self.seq += 1;
                write_place(&recovery_path(&self.place), &tree)?;
                self.write_meta()?;
                Ok((json!({"ok": true, "saved": true, "path": self.place.display().to_string(), "instances": tree.count()}), false))
            }
            "validate" => Ok((self.engine.validate()?, false)),
            "stop" => {
                if req.get("save").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let _ = self.dispatch("save", &json!({"cmd": "save", "force": req.get("force").cloned().unwrap_or(json!(false))}))?;
                }
                Ok((json!({"ok": true, "stopped": true}), true))
            }
            other => err(format!("unknown studio command '{other}'")),
        }
    }
}
