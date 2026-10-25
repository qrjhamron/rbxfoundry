//! Production MCP (Model Context Protocol) server over stdio.
//!
//! The server drives the same persistent C++ engine process and the same
//! `.rbxl`/`.rbxlx` files as the CLI — there is no second DataModel.
//! Mutating tools auto-save through the same history-recording atomic
//! write path the CLI uses, so MCP and CLI state never diverge.
//!
//! Protocol: JSON-RPC 2.0, newline-delimited, stdout is reserved for
//! protocol frames only (all diagnostics go to stderr). Filesystem access
//! is confined to the configured root directory.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value as JsonValue};

use crate::engine::EngineClient;
use crate::history::History;
use crate::rbxio::{read_place, write_place};
use crate::tree::{base64_encode, err, parse_value, Error, Result};

/// Local error conversion (the crate Error type has no From impls).
trait MapErr<T> {
    fn mx(self) -> Result<T>;
}
impl<T, E: std::fmt::Display> MapErr<T> for std::result::Result<T, E> {
    fn mx(self) -> Result<T> {
        self.map_err(|e| Error(e.to_string()))
    }
}

const SUPPORTED_PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct McpServer {
    root: PathBuf,
    engine_path: PathBuf,
    engine: Option<EngineClient>,
    open_path: Option<PathBuf>,
    session_camera: Option<JsonValue>,
}

pub fn run_server(root: PathBuf, engine_path: PathBuf) -> Result<()> {
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir().mx()?.join(root)
    };
    let root = root.canonicalize().unwrap_or(root);
    let mut server = McpServer {
        root,
        engine_path,
        engine: None,
        open_path: None,
        session_camera: None,
    };
    server.serve()
}

impl McpServer {
    fn serve(&mut self) -> Result<()> {
        let stdin = std::io::stdin();
        let mut lines = stdin.lock().lines();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        while let Some(line) = lines.next() {
            let line = line.mx()?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let request: JsonValue = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    Self::write(&mut out, &json!({
                        "jsonrpc": "2.0", "id": null,
                        "error": {"code": -32700, "message": format!("parse error: {e}")}
                    }))?;
                    continue;
                }
            };
            if let Some(response) = self.handle(&request) {
                Self::write(&mut out, &response)?;
            }
        }
        Ok(())
    }

    fn write(out: &mut impl Write, value: &JsonValue) -> Result<()> {
        let text = serde_json::to_string(value).mx()?;
        out.write_all(text.as_bytes()).mx()?;
        out.write_all(b"\n").mx()?;
        out.flush().mx()?;
        Ok(())
    }

    fn result(id: &JsonValue, value: JsonValue) -> JsonValue {
        json!({"jsonrpc": "2.0", "id": id, "result": value})
    }

    fn rpc_error(id: &JsonValue, code: i64, message: &str) -> JsonValue {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
    }

    fn handle(&mut self, request: &JsonValue) -> Option<JsonValue> {
        let id = request.get("id").cloned().unwrap_or(JsonValue::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let is_notification = request.get("id").is_none();
        match method {
            "initialize" => {
                let requested = request
                    .pointer("/params/protocolVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("2025-03-26");
                let version = if SUPPORTED_PROTOCOLS.contains(&requested) {
                    requested
                } else {
                    "2025-03-26"
                };
                Some(Self::result(&id, json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "bloxcli", "version": env!("CARGO_PKG_VERSION")},
                })))
            }
            "ping" => Some(Self::result(&id, json!({}))),
            "tools/list" => Some(Self::result(&id, json!({"tools": tool_definitions()}))),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(json!({}));
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call_tool(name, &args) {
                    Ok(content) => Some(Self::result(&id, json!({"content": content, "isError": false}))),
                    Err(e) => Some(Self::result(&id, json!({
                        "content": [{"type": "text", "text": format!("error: {e}")}],
                        "isError": true,
                    }))),
                }
            }
            _ if is_notification => None,
            _ => Some(Self::rpc_error(&id, -32601, &format!("method not found: {method}"))),
        }
    }

    // ------------------------------------------------------------ paths ---

    /// Resolves a tool path inside the sandbox root. Existing files are
    /// canonicalised; for new files the parent directory must resolve
    /// inside the root.
    fn sandbox_path(&self, raw: &str) -> Result<PathBuf> {
        let candidate = if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            self.root.join(raw)
        };
        let resolved = if candidate.exists() {
            candidate.canonicalize().mx()?
        } else if let Some(parent) = candidate.parent() {
            if parent.exists() {
                let parent = parent.canonicalize().mx()?;
                parent.join(candidate.file_name().ok_or_else(|| Error("invalid path".into()))?)
            } else {
                candidate.clone()
            }
        } else {
            candidate.clone()
        };
        if !resolved.starts_with(&self.root) {
            return err(format!(
                "path '{}' escapes the MCP project root '{}'",
                raw,
                self.root.display()
            ));
        }
        Ok(resolved)
    }

    // ----------------------------------------------------------- engine ---

    fn engine(&mut self) -> Result<&mut EngineClient> {
        if self.engine.is_none() {
            self.engine = Some(EngineClient::spawn(&self.engine_path)?);
        }
        Ok(self.engine.as_mut().unwrap())
    }

    fn require_open(&self) -> Result<PathBuf> {
        self.open_path
            .clone()
            .ok_or_else(|| Error("no place is open; call studio_open or studio_new first".into()))
    }

    /// Snapshots the engine and writes the open place atomically (with an
    /// undo-history record), exactly like a CLI mutation.
    fn persist(&mut self) -> Result<JsonValue> {
        let path = self.require_open()?;
        let tree = self.engine()?.snapshot()?;
        let mut history = History::open(&path)?;
        history.record(&path)?;
        write_place(&path, &tree)?;
        Ok(json!({"saved": true, "path": path.display().to_string(), "instances": tree.count()}))
    }

    fn reload_open(&mut self) -> Result<()> {
        let path = self.require_open()?;
        let (node, _format) = read_place(&path)?;
        self.engine()?.load(&node)?;
        Ok(())
    }

    // ------------------------------------------------------------ tools ---

    fn call_tool(&mut self, name: &str, args: &JsonValue) -> Result<Vec<JsonValue>> {
        // Everything except open/new needs a place in the session.
        const NEEDS_PLACE: &[&str] = &[
            "studio_save", "studio_export", "studio_tree", "studio_inspect",
            "instance_create", "instance_get", "instance_set", "instance_delete",
            "instance_clone", "instance_reparent", "instance_transform",
            "scene_render", "camera_control", "ui_render", "ui_inspect", "runtime_run", "runtime_export",
            "ui_validate", "studio_undo", "studio_redo",
        ];
        if NEEDS_PLACE.contains(&name) && self.open_path.is_none() {
            return err("no place is open; call studio_open or studio_new first");
        }
        match name {
            "studio_open" => {
                let path = self.sandbox_path(str_arg(args, "path")?)?;
                let (node, format) = read_place(&path)?;
                self.engine()?.load(&node)?;
                self.open_path = Some(path.clone());
                text(json!({
                    "path": path.display().to_string(),
                    "format": format!("{format:?}"),
                    "instances": node.count(),
                }))
            }
            "studio_new" => {
                let path = self.sandbox_path(str_arg(args, "path")?)?;
                if path.exists() && !bool_arg(args, "force") {
                    return err(format!("'{}' already exists (pass force=true)", path.display()));
                }
                let place_name = args.get("name").and_then(|v| v.as_str()).unwrap_or("Place");
                {
                    let engine = self.engine()?;
                    engine.new_datamodel(place_name)?;
                    for class in ["Workspace", "Lighting", "ReplicatedStorage", "ServerStorage",
                                  "ServerScriptService", "StarterGui", "StarterPlayer",
                                  "StarterPack", "Teams", "SoundService"] {
                        engine.add("game", class, class)?;
                    }
                }
                self.open_path = Some(path);
                let saved = self.persist()?;
                text(json!({"created": true, "saved": saved}))
            }
            "studio_save" => text(self.persist()?),
            "studio_export" => {
                let output = self.sandbox_path(str_arg(args, "output")?)?;
                let tree = self.engine()?.snapshot()?;
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent).mx()?;
                }
                write_place(&output, &tree)?;
                text(json!({"exported": output.display().to_string(), "instances": tree.count()}))
            }
            "studio_tree" => {
                let tree = match args.get("path").and_then(|v| v.as_str()) {
                    Some(path) => self.engine()?.get(path)?,
                    None => self.engine()?.snapshot()?,
                };
                text(serde_json::to_value(&tree).mx()?)
            }
            "studio_inspect" | "instance_get" => {
                let node = self.engine()?.get(str_arg(args, "target")?)?;
                text(serde_json::to_value(&node).mx()?)
            }
            "instance_create" => {
                let parent = str_arg(args, "parent")?;
                let class = str_arg(args, "className")?;
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or(class);
                let added = self.engine()?.add(parent, class, name)?;
                let saved = self.persist()?;
                text(json!({"added": added, "saved": saved}))
            }
            "instance_set" => {
                let target = str_arg(args, "target")?;
                let property = str_arg(args, "property")?;
                let raw = str_arg(args, "value")?;
                let type_name = match args.get("type").and_then(|v| v.as_str()) {
                    Some(t) => t.to_string(),
                    None => {
                        let node = self.engine()?.get(target)?;
                        match node.properties.get(property) {
                            Some(existing) => existing.type_name().to_string(),
                            None => {
                                return err(format!(
                                    "property '{property}' does not exist on '{target}'; pass an explicit 'type'"
                                ))
                            }
                        }
                    }
                };
                let value = parse_value(&type_name, raw, property)?;
                self.engine()?.set(target, property, &value)?;
                let saved = self.persist()?;
                text(json!({"set": {"target": target, "property": property, "value": value}, "saved": saved}))
            }
            "instance_delete" => {
                let removed = self.engine()?.delete(str_arg(args, "target")?)?;
                let saved = self.persist()?;
                text(json!({"removed": removed, "saved": saved}))
            }
            "instance_clone" => {
                let target = str_arg(args, "target")?;
                let parent = args.get("parent").and_then(|v| v.as_str());
                let name = args.get("name").and_then(|v| v.as_str());
                let cloned = self.engine()?.clone_instance(target, name, parent)?;
                let saved = self.persist()?;
                text(json!({"cloned": cloned, "saved": saved}))
            }
            "instance_reparent" => {
                let moved = self.engine()?.reparent(str_arg(args, "target")?, str_arg(args, "parent")?)?;
                let saved = self.persist()?;
                text(json!({"reparented": moved, "saved": saved}))
            }
            "instance_transform" => {
                let target = str_arg(args, "target")?;
                let mut applied = Vec::new();
                if let Some(delta) = args.get("moveDelta").and_then(|v| v.as_array()) {
                    let d = vec3_of(delta)?;
                    self.engine()?.move_by(target, d, bool_arg(args, "relative"))?;
                    applied.push("moveDelta");
                }
                if let Some(angles) = args.get("rotate").and_then(|v| v.as_array()) {
                    let a = vec3_of(angles)?;
                    self.engine()?.rotate(target, a, bool_arg(args, "world"))?;
                    applied.push("rotate");
                }
                if let Some(factor) = args.get("scaleFactor").and_then(|v| v.as_f64()) {
                    self.engine()?.scale_factor(target, factor as f32)?;
                    applied.push("scaleFactor");
                }
                if applied.is_empty() {
                    return err("instance_transform needs moveDelta, rotate, or scaleFactor");
                }
                let saved = self.persist()?;
                text(json!({"applied": applied, "saved": saved}))
            }
            "scene_render" => self.tool_scene_render(args),
            "camera_control" => self.tool_camera_control(args),
            "ui_render" => self.tool_ui_render(args),
            "ui_inspect" => {
                let params = self.ui_params(args)?;
                let result = self.engine()?.ui_rpc("ui/layout", params)?;
                text(result)
            }
            "ui_validate" => {
                let params = self.ui_params(args)?;
                let result = self.engine()?.ui_rpc("ui/validate", params)?;
                text(result)
            }
            "play_start" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let rig = args.get("rig").and_then(|v| v.as_str()).unwrap_or("R6");
                let state = crate::play::start_daemon(&self.engine_path, &place, rig, bool_arg(args, "force"))?;
                text(state)
            }
            "play_step" | "play_input" | "play_state" | "play_logs" | "play_stop" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let request = match name {
                    "play_step" => json!({"cmd": "step", "steps": args.get("steps").and_then(|v| v.as_i64()).unwrap_or(1)}),
                    "play_input" => json!({
                        "cmd": "input",
                        "keys": args.get("keys").cloned().unwrap_or(json!([])),
                        "duration": args.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.5),
                        "yawDeg": args.get("yawDeg"), "pitchDeg": args.get("pitchDeg"),
                        "firstPerson": args.get("firstPerson"),
                    }),
                    "play_state" => json!({"cmd": "state"}),
                    "play_logs" => json!({"cmd": "logs"}),
                    _ => json!({"cmd": "stop", "commit": bool_arg(args, "commit")}),
                };
                let state = crate::play::send_request(&place, &request)?;
                text(state)
            }
            "play_screenshot" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let output = self.render_output_path(args, "mcp-play.png")?;
                let result = crate::play::send_request(&place, &json!({
                    "cmd": "screenshot",
                    "output": output.to_string_lossy(),
                    "width": args.get("width").and_then(|v| v.as_i64()).unwrap_or(1280),
                    "height": args.get("height").and_then(|v| v.as_i64()).unwrap_or(720),
                }))?;
                if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                    return err(format!("play screenshot failed: {}", result.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")));
                }
                let mut content = image_result(&output, &result)?;
                content.insert(0, json!({"type": "text", "text": serde_json::to_string(&result).mx()?}));
                Ok(content)
            }
            "net_start" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let rig = args.get("rig").and_then(|v| v.as_str()).unwrap_or("R6");
                let players = args.get("players").and_then(|v| v.as_i64()).unwrap_or(2);
                if players < 1 || players > 8 {
                    return err("players must be between 1 and 8");
                }
                let latency = args.get("latencyMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let state = crate::net::start_daemon(&self.engine_path, &place, players as usize, latency, rig, bool_arg(args, "force"))?;
                text(state)
            }
            "net_step" | "net_input" | "net_state" | "net_logs" | "net_stop" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let request = match name {
                    "net_step" => json!({"cmd": "step", "steps": args.get("steps").and_then(|v| v.as_i64()).unwrap_or(1)}),
                    "net_input" => json!({
                        "cmd": "input",
                        "player": args.get("player").cloned().unwrap_or(json!(1)),
                        "keys": args.get("keys").cloned().unwrap_or(json!([])),
                        "duration": args.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.5),
                        "yawDeg": args.get("yawDeg"),
                    }),
                    "net_state" => json!({"cmd": "state"}),
                    "net_logs" => json!({"cmd": "logs"}),
                    _ => json!({"cmd": "stop", "commit": bool_arg(args, "commit")}),
                };
                let state = crate::net::send_request(&place, &request)?;
                if state.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                    return err(format!("net command failed: {}", state.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")));
                }
                text(state)
            }
            "net_screenshot" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let output = self.render_output_path(args, "mcp-net.png")?;
                let result = crate::net::send_request(&place, &json!({
                    "cmd": "screenshot",
                    "player": args.get("player").cloned().unwrap_or(json!(1)),
                    "output": output.to_string_lossy(),
                    "width": args.get("width").and_then(|v| v.as_i64()).unwrap_or(1280),
                    "height": args.get("height").and_then(|v| v.as_i64()).unwrap_or(720),
                }))?;
                if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                    return err(format!("net screenshot failed: {}", result.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")));
                }
                let mut content = image_result(&output, &result)?;
                content.insert(0, json!({"type": "text", "text": serde_json::to_string(&result).mx()?}));
                Ok(content)
            }
            "runtime_run" => {
                let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(2.0);
                let step = args.get("step").and_then(|v| v.as_f64()).unwrap_or(0.05);
                self.engine()?.call("runtime/begin", json!({}))?;
                let mut state = self.engine()?.call("runtime/runScripts", json!({}))?;
                if let Some(source) = args.get("eval").and_then(|v| v.as_str()) {
                    state = self.engine()?.call("runtime/eval", json!({"source": source, "name": "mcp-eval"}))?;
                }
                let steps = ((duration / step.max(0.001)).ceil() as i64).max(0);
                if steps > 0 {
                    state = self.engine()?.call("runtime/step", json!({"dt": step, "steps": steps}))?;
                }
                text(json!({
                    "time": state.get("time"), "threads": state.get("threads"),
                    "logs": state.get("logs"), "errors": state.get("errors"),
                    "note": "runtime state is separate from the edit place; use runtime_export to persist it",
                }))
            }
            "runtime_export" => {
                let output = self.sandbox_path(str_arg(args, "output")?)?;
                let snap = self.engine()?.call("runtime/snapshot", json!({}))?;
                let tree_json = snap.get("tree").ok_or_else(|| Error("runtime snapshot missing tree".into()))?;
                let tree = crate::engine::node_from_value(tree_json)?;
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent).mx()?;
                }
                write_place(&output, &tree)?;
                text(json!({"exported": output.display().to_string(), "instances": tree.count()}))
            }
            "studio_undo" => {
                let path = self.require_open()?;
                let mut history = History::open(&path)?;
                let outcome = history.undo(&path)?;
                self.reload_open()?;
                text(json!({"undo": outcome, "canUndo": history.can_undo(), "canRedo": history.can_redo()}))
            }
            "studio_redo" => {
                let path = self.require_open()?;
                let mut history = History::open(&path)?;
                let outcome = history.redo(&path)?;
                self.reload_open()?;
                text(json!({"redo": outcome, "canUndo": history.can_undo(), "canRedo": history.can_redo()}))
            }
            other => err(format!("unknown tool '{other}'")),
        }
    }

    fn ui_params(&self, args: &JsonValue) -> Result<JsonValue> {
        let (width, height) = match args.get("device").and_then(|v| v.as_str()) {
            Some("desktop") => (1920, 1080),
            Some("laptop") => (1366, 768),
            Some("tablet") => (1280, 800),
            Some("mobile") => (390, 844),
            Some(other) => return err(format!("unknown device '{other}'")),
            None => (
                args.get("width").and_then(|v| v.as_i64()).unwrap_or(1280),
                args.get("height").and_then(|v| v.as_i64()).unwrap_or(720),
            ),
        };
        let mut params = json!({"width": width, "height": height});
        if let Some(gui) = args.get("gui").and_then(|v| v.as_str()) {
            params["gui"] = json!(gui);
        }
        Ok(params)
    }

    fn tool_camera_control(&mut self, args: &JsonValue) -> Result<Vec<JsonValue>> {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("inspect") {
            "inspect" => {
                let place = self.require_open()?;
                let dir = place.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                let result = self.engine()?.render_camera(Some(&dir), Some(&dir))?;
                text(result)
            }
            "set" => {
                let mut camera = json!({});
                if let Some(p) = args.get("position") { camera["position"] = p.clone(); }
                if let Some(p) = args.get("lookAt") { camera["lookAt"] = p.clone(); }
                if let Some(f) = args.get("fovDegrees") { camera["fovDegrees"] = f.clone(); }
                self.session_camera = Some(camera.clone());
                text(json!({"sessionCamera": camera}))
            }
            other => err(format!("unknown camera action '{other}'")),
        }
    }

    fn render_output_path(&self, args: &JsonValue, default_name: &str) -> Result<PathBuf> {
        let raw = args.get("output").and_then(|v| v.as_str()).unwrap_or(default_name);
        let path = self.sandbox_path(raw)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).mx()?;
        }
        Ok(path)
    }

    fn tool_scene_render(&mut self, args: &JsonValue) -> Result<Vec<JsonValue>> {
        self.require_open()?;
        let output = self.render_output_path(args, "mcp-scene.png")?;
        let width = args.get("width").and_then(|v| v.as_i64()).unwrap_or(1280);
        let height = args.get("height").and_then(|v| v.as_i64()).unwrap_or(720);
        let place = self.require_open()?;
        let dir = place.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let mut params = json!({
            "width": width, "height": height,
            "outputPath": output.to_string_lossy(),
            "assetsDir": dir, "placeDir": dir, "shadows": true,
        });
        if let Some(preset) = args.get("preset").and_then(|v| v.as_str()) {
            params["preset"] = json!(preset);
        }
        let mut camera = self.session_camera.clone().unwrap_or(json!({}));
        let mut has_camera = self.session_camera.is_some();
        for key in ["position", "lookAt", "fovDegrees"] {
            if let Some(v) = args.get(key) {
                camera[key] = v.clone();
                has_camera = true;
            }
        }
        // render/screenshot expects position/lookAt as objects inside "camera".
        if has_camera {
            params["camera"] = camera;
        }
        let result = self.engine()?.render_screenshot(params)?;
        image_result(&output, &result)
    }

    fn tool_ui_render(&mut self, args: &JsonValue) -> Result<Vec<JsonValue>> {
        self.require_open()?;
        let output = self.render_output_path(args, "mcp-ui.png")?;
        let mut params = self.ui_params(args)?;
        params["outputPath"] = json!(output.to_string_lossy());
        let result = self.engine()?.ui_rpc("ui/render", params)?;
        image_result(&output, &result)
    }
}

fn text(value: JsonValue) -> Result<Vec<JsonValue>> {
    Ok(vec![json!({"type": "text", "text": serde_json::to_string(&value).mx()?})])
}

fn image_result(path: &Path, result: &JsonValue) -> Result<Vec<JsonValue>> {
    let bytes = std::fs::read(path).mx()?;
    if bytes.len() < 33 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return err(format!("renderer did not produce a valid PNG at {}", path.display()));
    }
    Ok(vec![
        json!({"type": "text", "text": serde_json::to_string(&json!({
            "output": path.display().to_string(),
            "device": result.get("device"),
            "drawCalls": result.get("drawCalls"),
            "triangles": result.get("triangles"),
            "parts": result.get("parts"),
            "quadCount": result.get("quadCount"),
            "missingAssets": result.get("missingAssets"),
            "warnings": result.get("warnings"),
        })).mx()?}),
        json!({"type": "image", "data": base64_encode(&bytes), "mimeType": "image/png"}),
    ])
}

fn str_arg<'a>(args: &'a JsonValue, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error(format!("missing required string argument '{key}'")))
}

fn bool_arg(args: &JsonValue, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn vec3_of(values: &[JsonValue]) -> Result<[f32; 3]> {
    if values.len() != 3 {
        return err("expected an array of 3 numbers");
    }
    let mut out = [0f32; 3];
    for (i, v) in values.iter().enumerate() {
        out[i] = v.as_f64().ok_or_else(|| Error("expected numbers".into()))? as f32;
    }
    Ok(out)
}

// ----------------------------------------------------------- tool schemas --

fn schema(properties: JsonValue, required: &[&str]) -> JsonValue {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn tool(name: &str, description: &str, input_schema: JsonValue) -> JsonValue {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

const PATH_PROP: &str = "\"path\": {\"type\": \"string\", \"description\": \"Place file inside the project root (.rbxl/.rbxlx)\"}";

fn tool_definitions() -> Vec<JsonValue> {
    let path_only = serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}}}")).unwrap();
    vec![
        tool("studio_open", "Open an existing .rbxl/.rbxlx place (inside the MCP project root) into the persistent engine session.", schema(path_only.clone(), &["path"])),
        tool("studio_new", "Create a new place with the standard Roblox service set and save it.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"name\": {{\"type\": \"string\"}}, \"force\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("studio_save", "Save the open place atomically (records an undo point).", schema(json!({}), &[])),
        tool("studio_export", "Export the open place to another .rbxl/.rbxlx path.", schema(json!({"output": {"type": "string"}}), &["output"])),
        tool("studio_tree", "Return the full instance tree (or the subtree at 'path') as JSON.", schema(json!({"path": {"type": "string"}}), &[])),
        tool("studio_inspect", "Return one instance with properties and children.", schema(json!({"target": {"type": "string"}}), &["target"])),
        tool("instance_create", "Create an instance (engine reflection defaults) under a parent path. Auto-saves.", schema(
            json!({"parent": {"type": "string"}, "className": {"type": "string"}, "name": {"type": "string"}}), &["parent", "className"])),
        tool("instance_get", "Return one instance with properties and children.", schema(json!({"target": {"type": "string"}}), &["target"])),
        tool("instance_set", "Set a typed property from its string form (e.g. Color3 '1,0,0', UDim2 '0.5,0,0.5,0'). 'type' is inferred from the existing property when omitted. Auto-saves.", schema(
            json!({"target": {"type": "string"}, "property": {"type": "string"}, "value": {"type": "string"}, "type": {"type": "string"}}),
            &["target", "property", "value"])),
        tool("instance_delete", "Delete an instance subtree. Auto-saves.", schema(json!({"target": {"type": "string"}}), &["target"])),
        tool("instance_clone", "Deep-clone an instance with fresh referents. Auto-saves.", schema(
            json!({"target": {"type": "string"}, "parent": {"type": "string"}, "name": {"type": "string"}}), &["target"])),
        tool("instance_reparent", "Move an instance under a new parent. Auto-saves.", schema(
            json!({"target": {"type": "string"}, "parent": {"type": "string"}}), &["target", "parent"])),
        tool("instance_transform", "Apply moveDelta [x,y,z], rotate [rx,ry,rz] degrees, and/or scaleFactor to a part or model. Auto-saves.", schema(
            json!({"target": {"type": "string"}, "moveDelta": {"type": "array", "items": {"type": "number"}},
                   "rotate": {"type": "array", "items": {"type": "number"}}, "scaleFactor": {"type": "number"},
                   "relative": {"type": "boolean"}, "world": {"type": "boolean"}}), &["target"])),
        tool("scene_render", "Render the open place's Workspace with the real Vulkan renderer. Returns a PNG image. Camera defaults to the session camera (camera_control) or the isometric framing.", schema(
            json!({"output": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"},
                   "preset": {"type": "string"}, "position": {"type": "array", "items": {"type": "number"}},
                   "lookAt": {"type": "array", "items": {"type": "number"}}, "fovDegrees": {"type": "number"}}), &[])),
        tool("camera_control", "action=inspect reports CurrentCamera, scene bounds and suggested framing; action=set stores the session camera used by scene_render.", schema(
            json!({"action": {"type": "string", "enum": ["inspect", "set"]},
                   "position": {"type": "array", "items": {"type": "number"}},
                   "lookAt": {"type": "array", "items": {"type": "number"}}, "fovDegrees": {"type": "number"}}), &[])),
        tool("ui_render", "Rasterise the open place's GUI instances to a PNG image at a device preset or custom size.", schema(
            json!({"output": {"type": "string"}, "device": {"type": "string", "enum": ["desktop", "laptop", "tablet", "mobile"]},
                   "width": {"type": "integer"}, "height": {"type": "integer"}, "gui": {"type": "string"}}), &[])),
        tool("ui_inspect", "Computed absolute GUI layout rects, visibility and render order (JSON).", schema(
            json!({"device": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"}, "gui": {"type": "string"}}), &[])),
        tool("ui_validate", "Validate GUI layout: offscreen (error), clipped, zero-size and invisible elements.", schema(
            json!({"device": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"}, "gui": {"type": "string"}}), &[])),
        tool("runtime_run", "Run the open place's Luau scripts in the real runtime (separate from edit state): starts Script/LocalScript instances, optionally evals a chunk, then steps simulated time. Returns logs and structured script errors.", schema(
            json!({"duration": {"type": "number"}, "step": {"type": "number"}, "eval": {"type": "string"}}), &[])),
        tool("runtime_export", "Export the current runtime DataModel state to a .rbxl/.rbxlx file.", schema(json!({"output": {"type": "string"}}), &["output"])),
        tool("play_start", "Start a persistent playtest session (Jolt physics + Luau runtime + character) for a place inside the root. rig: R6 or R15.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"rig\": {{\"type\": \"string\", \"enum\": [\"R6\", \"R15\"]}}, \"force\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("play_step", "Advance a play session by fixed 1/60 steps (keys released; use play_input to move).", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"steps\": {{\"type\": \"integer\"}}}}")).unwrap(), &["path"])),
        tool("play_input", "Hold keys (W,A,S,D,Space) for a duration in a play session, optionally setting camera yawDeg/pitchDeg/firstPerson. The character moves through real physics, never teleporting.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"keys\": {{\"type\": \"array\", \"items\": {{\"type\": \"string\"}}}}, \"duration\": {{\"type\": \"number\"}}, \"yawDeg\": {{\"type\": \"number\"}}, \"pitchDeg\": {{\"type\": \"number\"}}, \"firstPerson\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("play_state", "Character position/velocity/health/state, simulation time and camera for a play session.", schema(path_only.clone(), &["path"])),
        tool("play_screenshot", "Screenshot a play session from the character camera. Returns a PNG image.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"output\": {{\"type\": \"string\"}}, \"width\": {{\"type\": \"integer\"}}, \"height\": {{\"type\": \"integer\"}}}}")).unwrap(), &["path"])),
        tool("play_logs", "Luau print/warn output captured during a play session.", schema(path_only.clone(), &["path"])),
        tool("play_stop", "Stop a play session. commit=true writes the runtime world back into the place file (with history); otherwise it is discarded.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"commit\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("net_start", "Start a multi-context net session: one authoritative server plus one client engine per simulated player, separate DataModels/Luau contexts, real RemoteEvent/RemoteFunction message passing, snapshot-diff replication and configurable latency. players 1..8, rig R6/R15.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"players\": {{\"type\": \"integer\"}}, \"latencyMs\": {{\"type\": \"number\"}}, \"rig\": {{\"type\": \"string\", \"enum\": [\"R6\", \"R15\"]}}, \"force\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("net_step", "Advance every net peer by fixed 1/60 steps, delivering due remote messages and server->client replication.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"steps\": {{\"type\": \"integer\"}}}}")).unwrap(), &["path"])),
        tool("net_input", "Hold keys (W,A,S,D,Space) for one simulated player for a duration; only that player's server-side character moves (server-authoritative physics).", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"player\": {{\"type\": \"integer\"}}, \"keys\": {{\"type\": \"array\", \"items\": {{\"type\": \"string\"}}}}, \"duration\": {{\"type\": \"number\"}}, \"yawDeg\": {{\"type\": \"number\"}}}}")).unwrap(), &["path"])),
        tool("net_state", "Net session state: step/time, players, per-player character physics state, server/client runtime state, queued message count and warnings.", schema(path_only.clone(), &["path"])),
        tool("net_screenshot", "Screenshot one client's replicated view from behind its character. Returns a PNG image.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"player\": {{\"type\": \"integer\"}}, \"output\": {{\"type\": \"string\"}}, \"width\": {{\"type\": \"integer\"}}, \"height\": {{\"type\": \"integer\"}}}}")).unwrap(), &["path"])),
        tool("net_logs", "Per-peer Luau logs/errors (each entry tagged with its peer: server/client1/...).", schema(path_only.clone(), &["path"])),
        tool("net_stop", "Stop a net session. commit=true writes the authoritative server world back into the place file (with history).", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"commit\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("studio_undo", "Undo the last saved mutation of the open place and reload it.", schema(json!({}), &[])),
        tool("studio_redo", "Redo a previously undone mutation of the open place and reload it.", schema(json!({}), &[])),
    ]
}
