//! `blox studio ...` commands (Phase 25): persistent editing session
//! lifecycle and editing operations against the live session daemon.
//! These delegate to `crate::session::studio`; no engine state lives
//! in the CLI process.

use std::path::PathBuf;
use serde_json::{json, Value as JsonValue};
use crate::cli::args::{parse_cmd, usage_err, Global};
use crate::app::session::emit;
use crate::protocol::engine::find_engine;
use crate::protocol::tree::{parse_value, Error, Result};

fn place_arg(parsed: &crate::cli::args::CmdArgs) -> Result<PathBuf> {
    parsed.file()
}

pub fn cmd_studio_daemon(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &[], &[])?;
    let file = place_arg(&parsed)?;
    let engine_path = find_engine(global.engine.as_deref())?;
    crate::session::studio::run_daemon(&engine_path, &file)
}

pub fn cmd_studio(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("studio needs a subcommand: start|status|attach|save|stop|undo|redo|get|set|add|delete|select|camera"));
    };
    match sub.as_str() {
        "start" => {
            let parsed = parse_cmd(rest, &[], &["force"])?;
            let file = place_arg(&parsed)?;
            let engine_path = find_engine(global.engine.as_deref())?;
            let state = crate::session::studio::start_daemon(&engine_path, &file, parsed.has("force"))?;
            emit(global, format!("studio session started for {} (id {})", file.display(), state.get("sessionId").and_then(|v| v.as_str()).unwrap_or("?")), json!({"ok": true, "command": "studio start", "state": state}));
            Ok(())
        }
        "status" | "attach" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = place_arg(&parsed)?;
            if !crate::session::studio::session_dir(&file).join("session.sock").exists() {
                let meta = crate::session::studio::read_meta(&file);
                emit(global, format!("no live studio session for {}", file.display()), json!({"ok": true, "live": false, "meta": meta}));
                return Ok(());
            }
            match crate::session::studio::send_request(&file, &json!({"cmd": "status"})) {
                Ok(state) => { emit(global, format!("studio session {} for {}: {} instances, dirty={}", state.get("sessionId").and_then(|v| v.as_str()).unwrap_or("?"), file.display(), state.get("instances").cloned().unwrap_or(json!(null)), state.get("dirty").cloned().unwrap_or(json!(null))), json!({"ok": true, "live": true, "state": state})); Ok(()) }
                Err(e) => { emit(global, format!("studio session for {} is stale (socket present, daemon not responding): {e}", file.display()), json!({"ok": true, "live": false, "stale": true, "error": e.to_string()})); Ok(()) }
            }
        }
        "save" => {
            let parsed = parse_cmd(rest, &[], &["force"])?;
            let file = place_arg(&parsed)?;
            let r = crate::session::studio::send_request(&file, &json!({"cmd": "save", "force": parsed.has("force")}))?;
            emit(global, format!("saved {} ({} instances)", file.display(), r.get("instances").cloned().unwrap_or(json!(null))), r);
            Ok(())
        }
        "stop" => {
            let parsed = parse_cmd(rest, &[], &["save", "force"])?;
            let file = place_arg(&parsed)?;
            let r = crate::session::studio::send_request(&file, &json!({"cmd": "stop", "save": parsed.has("save"), "force": parsed.has("force")}))?;
            emit(global, format!("studio session stopped for {}", file.display()), r);
            Ok(())
        }
        "undo" | "redo" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = place_arg(&parsed)?;
            let r = crate::session::studio::send_request(&file, &json!({"cmd": sub}))?;
            emit(global, format!("{sub} for {}: {}", file.display(), r), r);
            Ok(())
        }
        "get" => {
            let parsed = parse_cmd(rest, &["target"], &[])?;
            let file = place_arg(&parsed)?;
            let r = crate::session::studio::send_request(&file, &json!({"cmd": "get", "target": parsed.required("target")?}))?;
            emit(global, format!("{r}"), r);
            Ok(())
        }
        "set" => {
            let parsed = parse_cmd(rest, &["target", "property", "value", "type"], &[])?;
            let file = place_arg(&parsed)?;
            let target = parsed.required("target")?.to_string();
            let property = parsed.required("property")?.to_string();
            let raw = parsed.required("value")?.to_string();
            // Resolve the type from the current value when not given.
            let current = crate::session::studio::send_request(&file, &json!({"cmd": "get", "target": target}))?;
            let type_name = match parsed.opt("type") {
                Some(t) => t.to_string(),
                None => {
                    let inst = current.get("instance").cloned().unwrap_or(json!({}));
                    let existing = inst.get("properties").and_then(|p| p.get(&property)).and_then(|v| v.get("type")).and_then(|v| v.as_str()).unwrap_or("String").to_string();
                    existing
                }
            };
            let value = parse_value(&type_name, &raw, &property)?;
            let r = crate::session::studio::send_request(&file, &json!({"cmd": "set", "target": target, "property": property, "value": serde_json::to_value(&value).map_err(|e| Error(e.to_string()))?}))?;
            emit(global, format!("set {target}.{property} in session for {}", file.display()), r);
            Ok(())
        }
        "add" => {
            let parsed = parse_cmd(rest, &["parent", "class", "name"], &[])?;
            let file = place_arg(&parsed)?;
            let r = crate::session::studio::send_request(&file, &json!({"cmd": "add", "parent": parsed.required("parent")?, "className": parsed.required("class")?, "name": parsed.opt("name").unwrap_or("")}))?;
            emit(global, format!("added instance in session for {}", file.display()), r);
            Ok(())
        }
        "delete" => {
            let parsed = parse_cmd(rest, &["target"], &[])?;
            let file = place_arg(&parsed)?;
            let r = crate::session::studio::send_request(&file, &json!({"cmd": "delete", "target": parsed.required("target")?}))?;
            emit(global, format!("deleted instance in session for {}", file.display()), r);
            Ok(())
        }
        "select" => {
            let parsed = parse_cmd(rest, &["targets"], &[])?;
            let file = place_arg(&parsed)?;
            let targets: Vec<&str> = parsed.opt("targets").unwrap_or("").split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            let r = crate::session::studio::send_request(&file, &json!({"cmd": "select", "selection": targets}))?;
            emit(global, format!("selection for {}: {:?}", file.display(), r.get("selection")), r);
            Ok(())
        }
        "camera" => {
            let parsed = parse_cmd(rest, &["position", "look-at", "fov"], &[])?;
            let file = place_arg(&parsed)?;
            let mut cam = json!({});
            if let Some(p) = parsed.opt("position") { cam["position"] = json!(p.split(',').filter_map(|s| s.trim().parse::<f64>().ok()).collect::<Vec<_>>()); }
            if let Some(p) = parsed.opt("look-at") { cam["lookAt"] = json!(p.split(',').filter_map(|s| s.trim().parse::<f64>().ok()).collect::<Vec<_>>()); }
            if let Some(f) = parsed.opt("fov") { cam["fovDegrees"] = json!(f.parse::<f64>().unwrap_or(70.0)); }
            let req = if cam.as_object().map(|o| o.is_empty()).unwrap_or(true) { json!({"cmd": "camera"}) } else { json!({"cmd": "camera", "camera": cam}) };
            let r = crate::session::studio::send_request(&file, &req)?;
            emit(global, format!("camera for {}: {}", file.display(), r.get("camera").cloned().unwrap_or(json!(null))), r);
            Ok(())
        }
        other => Err(usage_err(&format!("unknown studio subcommand '{other}'"))),
    }
}
