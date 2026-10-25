//! Runtime commands: run/exec/script.

use std::path::PathBuf;
use serde_json::{json, Value as JsonValue};
use crate::cli::args::{parse_cmd, usage_err, Global};
use crate::app::session::{emit, load_session, spawn_engine, write_mutation};
use crate::protocol::tree::{parse_vec3, Error, PropValue, Result};
use crate::rbxio::{read_place, write_place};
use crate::commands::sessions::place_arg;

pub fn cmd_run(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(
        args,
        &["duration", "step", "eval", "save-runtime", "render", "ui", "camera", "look-at"],
        &["allow-errors", "commit"],
    )?;
    let file = parsed.file()?;
    let duration: f64 = parsed.opt("duration").unwrap_or("2").parse()
        .map_err(|_| usage_err("--duration must be a number of seconds"))?;
    let step: f64 = parsed.opt("step").unwrap_or("0.05").parse()
        .map_err(|_| usage_err("--step must be a number of seconds"))?;
    if step <= 0.0 || duration < 0.0 {
        return Err(usage_err("--step must be > 0 and --duration >= 0"));
    }
    let (mut engine, _format) = load_session(global, &file)?;
    engine.call("runtime/begin", json!({}))?;
    let mut state = engine.call("runtime/runScripts", json!({}))?;
    if let Some(source) = parsed.opt("eval") {
        state = engine.call("runtime/eval", json!({"source": source, "name": "cli-eval"}))?;
        if state.get("ok").and_then(|v| v.as_bool()) == Some(false) && !parsed.has("allow-errors") {
            let error = state.get("error").and_then(|v| v.as_str()).unwrap_or("eval failed");
            engine.shutdown();
            return Err(Error(format!("--eval chunk failed: {error}")));
        }
    }
    let steps = ((duration / step).ceil() as i64).max(0);
    if steps > 0 {
        state = engine.call("runtime/step", json!({"dt": step, "steps": steps}))?;
    }
    let tree = {
        let snap = engine.call("runtime/snapshot", json!({}))?;
        let tree_json = snap.get("tree").ok_or_else(|| Error("runtime snapshot missing tree".into()))?;
        crate::protocol::engine::node_from_value(tree_json)?
    };
    let logs = state.get("logs").cloned().unwrap_or(json!([]));
    let errors = state.get("errors").cloned().unwrap_or(json!([]));
    let error_count = errors.as_array().map(|a| a.len()).unwrap_or(0);

    let mut saved_to: Option<String> = None;
    if parsed.has("commit") {
        write_mutation(&file, &tree)?;
        saved_to = Some(file.display().to_string());
    } else if let Some(out) = parsed.opt("save-runtime") {
        write_place(&PathBuf::from(out), &tree)?;
        saved_to = Some(out.to_string());
    }
    let mut rendered_to: Option<String> = None;
    let mut ui_to: Option<String> = None;
    if parsed.opt("render").is_some() || parsed.opt("ui").is_some() {
        // Load the runtime result back as the session model for rendering.
        engine.load(&tree)?;
        if let Some(out) = parsed.opt("render") {
            let place_dir = file.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let mut params = json!({
                "width": 1280, "height": 720, "outputPath": out,
                "assetsDir": place_dir, "placeDir": place_dir, "shadows": true,
            });
            let mut camera = json!({});
            let mut has_camera = false;
            if let Some(c) = parsed.opt("camera") { camera["position"] = json!(parse_vec3(c)?); has_camera = true; }
            if let Some(c) = parsed.opt("look-at") { camera["lookAt"] = json!(parse_vec3(c)?); has_camera = true; }
            if !has_camera { params["preset"] = json!("isometric"); } else { params["camera"] = camera; }
            engine.render_screenshot(params)?;
            rendered_to = Some(out.to_string());
        }
        if let Some(out) = parsed.opt("ui") {
            let mut params = json!({"width": 1280, "height": 720});
            params["outputPath"] = json!(out);
            engine.ui_rpc("ui/render", params)?;
            ui_to = Some(out.to_string());
        }
    }
    let time = state.get("time").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let threads = state.get("threads").and_then(|v| v.as_i64()).unwrap_or(0);
    engine.shutdown();

    let human = {
        let mut lines = vec![format!(
            "ran {} for {time:.2}s of simulated time ({threads} live thread(s))",
            file.display()
        )];
        if let Some(arr) = logs.as_array() {
            for entry in arr {
                lines.push(format!(
                    "[{}] {}: {}",
                    entry.get("level").and_then(|v| v.as_str()).unwrap_or("print"),
                    entry.get("source").and_then(|v| v.as_str()).unwrap_or(""),
                    entry.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                ));
            }
        }
        if let Some(arr) = errors.as_array() {
            for entry in arr {
                lines.push(format!(
                    "ERROR {}: {}",
                    entry.get("source").and_then(|v| v.as_str()).unwrap_or(""),
                    entry.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                ));
            }
        }
        if let Some(path) = &saved_to { lines.push(format!("runtime state saved to {path}")); }
        if let Some(path) = &rendered_to { lines.push(format!("rendered runtime scene to {path}")); }
        if let Some(path) = &ui_to { lines.push(format!("rendered runtime UI to {path}")); }
        lines.join("
")
    };
    emit(global, human, json!({
        "ok": error_count == 0, "command": "run", "path": file.display().to_string(),
        "time": time, "threads": threads, "logs": logs, "errors": errors,
        "savedTo": saved_to, "renderedTo": rendered_to, "uiRenderedTo": ui_to,
        "instances": tree.count(),
    }));
    if error_count > 0 && !parsed.has("allow-errors") {
        return Err(Error(format!("{error_count} script error(s) during run (see output); pass --allow-errors to exit 0")));
    }
    Ok(())
}

pub fn cmd_exec(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["code"], &[])?;
    let file = parsed.file()?;
    let code = parsed.required("code")?.to_string();
    let (mut engine, _format) = load_session(global, &file)?;
    engine.call("runtime/begin", json!({}))?;
    let state = engine.call("runtime/eval", json!({"source": code, "name": "command-bar"}))?;
    let logs = state.get("logs").cloned().unwrap_or_else(|| json!([]));
    let errors = state.get("errors").cloned().unwrap_or_else(|| json!([]));
    let snap = engine.call("runtime/snapshot", json!({}))?;
    let tree_json = snap.get("tree").ok_or_else(|| Error("runtime snapshot missing tree".into()))?;
    let tree = crate::protocol::engine::node_from_value(tree_json)?;
    engine.shutdown();
    if errors.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
        return Err(Error(format!("command bar Luau failed (edit tree NOT saved): {errors}")));
    }
    write_mutation(&file, &tree)?;
    emit(
        global,
        format!("command bar executed in {} (edit context, committed)", file.display()),
        json!({
            "ok": true, "command": "exec", "path": file.display().to_string(),
            "logs": logs, "instances": tree.count(),
        }),
    );
    Ok(())
}

pub fn cmd_script(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("script needs a subcommand: analyze|complete"));
    };
    let sub_args: Vec<String> = rest.to_vec();
    match sub.as_str() {
        "analyze" => {
            let parsed = parse_cmd(&sub_args, &["target", "name"], &[])?;
            let file = place_arg(&parsed)?;
            let (source, name) = if file.extension().map(|e| e == "luau" || e == "lua").unwrap_or(false) {
                let text = std::fs::read_to_string(&file)
                    .map_err(|e| Error(format!("cannot read {}: {e}", file.display())))?;
                (text, parsed.opt("name").unwrap_or("Main").to_string())
            } else {
                let target = parsed.opt("target")
                    .ok_or_else(|| usage_err("script analyze on a place requires --target <script path>"))?;
                let (node, _format) = read_place(&file)?;
                let script = node.find_path(target).map_err(|e| Error(e.0))?;
                match script.properties.get("Source") {
                    Some(PropValue::String(src)) => (src.clone(), target.to_string()),
                    _ => return Err(Error(format!("'{target}' has no Source to analyze"))),
                }
            };
            let mut engine = spawn_engine(global)?;
            let result = engine.call("script/analyze", json!({"source": source, "name": name}));
            engine.shutdown();
            let result = result?;
            let diagnostics = result["diagnostics"].clone();
            let errors = diagnostics.as_array().map(|a| a.iter().filter(|d| d["severity"] == "error").count()).unwrap_or(0);
            emit(
                global,
                format!("{} diagnostic(s), {} error(s) for {}", diagnostics.as_array().map(|a| a.len()).unwrap_or(0), errors, name),
                json!({"ok": errors == 0, "command": "script analyze", "name": name, "diagnostics": diagnostics}),
            );
            if errors > 0 {
                Err(Error(format!("{errors} error(s) found by the Luau frontend")))
            } else {
                Ok(())
            }
        }
        "complete" => {
            let parsed = parse_cmd(&sub_args, &["target", "line", "column"], &[])?;
            let file = place_arg(&parsed)?;
            let target = parsed.opt("target")
                .ok_or_else(|| usage_err("script complete requires --target <script path>"))?;
            let line: usize = parsed.opt("line").unwrap_or("1").parse()
                .map_err(|_| usage_err("--line must be a number"))?;
            let column: usize = parsed.opt("column").unwrap_or("1").parse()
                .map_err(|_| usage_err("--column must be a number"))?;
            let (node, _format) = read_place(&file)?;
            let script = node.find_path(target).map_err(|e| Error(e.0))?;
            let source = match script.properties.get("Source") {
                Some(PropValue::String(src)) => src.clone(),
                _ => return Err(Error(format!("'{target}' has no Source"))),
            };
            let items = crate::diagnostics::completion::completions(&source, line, column, &script.class_name, &node);
            let json_items: Vec<JsonValue> = items.iter().map(|c| json!({
                "label": c.label, "kind": c.kind, "detail": c.detail,
            })).collect();
            emit(
                global,
                format!("{} completion(s) at {target}:{line}:{column}", json_items.len()),
                json!({"ok": true, "command": "script complete", "target": target, "line": line, "column": column, "completions": json_items}),
            );
            Ok(())
        }
        other => Err(usage_err(&format!("unknown script subcommand '{other}' (analyze|complete)"))),
    }
}
