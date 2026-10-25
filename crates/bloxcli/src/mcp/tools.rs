//! MCP tool dispatch: one match over tool names, grouped by domain
//! (studio, instance, scene/camera, ui, play, net, runtime, script).
//! Every mutating tool persists through the same history-recording
//! atomic write as the CLI (see `McpServer::persist`).

use std::path::{Path, PathBuf};
use serde_json::{json, Value as JsonValue};
use crate::history::History;
use crate::rbxio::{read_place, write_place};
use crate::protocol::tree::{base64_encode, err, parse_value, Error, Result};
use crate::mcp::server::{MapErr, McpServer};

impl McpServer {
    pub(crate) fn call_tool(&mut self, name: &str, args: &JsonValue) -> Result<Vec<JsonValue>> {
        // Everything except open/new needs a place in the session.
        const NEEDS_PLACE: &[&str] = &[
            "studio_save", "studio_export", "studio_tree", "studio_inspect",
            "instance_create", "instance_get", "instance_set", "instance_delete",
            "instance_clone", "instance_reparent", "instance_transform", "instance_ungroup",
            "scene_render", "camera_control", "ui_render", "ui_inspect", "runtime_run", "runtime_export",
            "ui_validate", "studio_undo", "studio_redo", "script_analyze",
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
            "instance_ungroup" => {
                let target = str_arg(args, "target")?;
                let parent = target.rsplit_once('/')
                    .map(|(p, _)| p.to_string())
                    .ok_or_else(|| crate::protocol::tree::Error("ungroup target must be inside a parent".into()))?;
                let node = self.engine()?.get(target)?;
                if node.children.is_empty() {
                    return err(format!("'{target}' has no children to ungroup"));
                }
                let mut moved = 0;
                for child in &node.children {
                    self.engine()?.reparent(&format!("{target}/{}", child.name), &parent)?;
                    moved += 1;
                }
                self.engine()?.delete(target)?;
                let saved = self.persist()?;
                text(json!({"ungrouped": target, "moved": moved, "saved": saved}))
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
                let state = crate::session::play::start_daemon(&self.engine_path, &place, rig, bool_arg(args, "force"))?;
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
                let state = crate::session::play::send_request(&place, &request)?;
                text(state)
            }
            "play_screenshot" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let output = self.render_output_path(args, "mcp-play.png")?;
                let result = crate::session::play::send_request(&place, &json!({
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
                let state = crate::session::net::start_daemon(&self.engine_path, &place, players as usize, latency, rig, bool_arg(args, "force"))?;
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
                let state = crate::session::net::send_request(&place, &request)?;
                if state.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                    return err(format!("net command failed: {}", state.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")));
                }
                text(state)
            }
            "net_screenshot" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let output = self.render_output_path(args, "mcp-net.png")?;
                let result = crate::session::net::send_request(&place, &json!({
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
            "script_analyze" => {
                // Official Luau frontend (parser + type solver + lint)
                // over a script's Source in the open place.
                let target = str_arg(args, "target")?;
                let snapshot = self.engine()?.snapshot()?;
                let script = snapshot.find_path(target).map_err(|e| crate::protocol::tree::Error(e.0))?;
                let source = match script.properties.get("Source") {
                    Some(crate::protocol::tree::PropValue::String(src)) => src.clone(),
                    _ => return err(format!("'{target}' has no Source to analyze")),
                };
                let result = self.engine()?.call(
                    "script/analyze",
                    json!({"source": source, "name": target}),
                )?;
                text(json!({"target": target, "diagnostics": result["diagnostics"]}))
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
                let tree = crate::protocol::engine::node_from_value(tree_json)?;
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
            "studio_session_start" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let state = crate::session::studio::start_daemon(&self.engine_path, &place, bool_arg(args, "force"))?;
                text(state)
            }
            "studio_session_status" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let state = crate::session::studio::send_request(&place, &json!({"cmd": "status"}))?;
                text(state)
            }
            "studio_session_get" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let state = crate::session::studio::send_request(&place, &json!({"cmd": "get", "target": str_arg(args, "target")?}))?;
                text(state)
            }
            "studio_session_set" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let state = crate::session::studio::send_request(&place, &json!({"cmd": "set", "target": str_arg(args, "target")?, "property": str_arg(args, "property")?, "value": args.get("value").cloned().ok_or_else(|| Error("set needs value".into()))?}))?;
                text(state)
            }
            "studio_session_save" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let state = crate::session::studio::send_request(&place, &json!({"cmd": "save", "force": bool_arg(args, "force")}))?;
                text(state)
            }
            "studio_session_stop" => {
                let place = self.sandbox_path(str_arg(args, "path")?)?;
                let state = crate::session::studio::send_request(&place, &json!({"cmd": "stop", "save": bool_arg(args, "save")}))?;
                text(state)
            }
            other => err(format!("unknown tool '{other}'")),
        }
    }

    pub(crate) fn ui_params(&self, args: &JsonValue) -> Result<JsonValue> {
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

    pub(crate) fn tool_camera_control(&mut self, args: &JsonValue) -> Result<Vec<JsonValue>> {
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

    pub(crate) fn render_output_path(&self, args: &JsonValue, default_name: &str) -> Result<PathBuf> {
        let raw = args.get("output").and_then(|v| v.as_str()).unwrap_or(default_name);
        let path = self.sandbox_path(raw)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).mx()?;
        }
        Ok(path)
    }

    pub(crate) fn tool_scene_render(&mut self, args: &JsonValue) -> Result<Vec<JsonValue>> {
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

    pub(crate) fn tool_ui_render(&mut self, args: &JsonValue) -> Result<Vec<JsonValue>> {
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
