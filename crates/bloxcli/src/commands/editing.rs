//! Editing commands: tree/add/set/save/export/get/find/clone/delete/reparent/group/ungroup/undo/validate.

use std::path::PathBuf;
use serde_json::{json, Value as JsonValue};
use crate::cli::args::{parse_cmd, usage_err, Global};
use crate::app::session::{emit, format_name, load_session, mutate, spawn_engine, write_mutation};
use crate::protocol::tree::{parse_value, Error, Node, PropValue, Result};
use crate::rbxio::{format_for_path, read_place, write_place};
use crate::rbxio::convert::validate_reflection;
use crate::history::History;
use crate::validation::integrity::{blocked_message, check_export_integrity, errors as integrity_errors, warnings as integrity_warnings};

pub fn cmd_tree(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["path", "depth"], &["props"])?;
    let file = parsed.file()?;
    let (mut engine, _format) = load_session(global, &file)?;
    let tree = match parsed.opt("path") {
        Some(path) => engine.get(path)?,
        None => engine.snapshot()?,
    };
    engine.shutdown();
    let max_depth: Option<usize> = match parsed.opt("depth") {
        Some(d) => Some(
            d.parse()
                .map_err(|_| usage_err("--depth must be a non-negative integer"))?,
        ),
        None => None,
    };

    if global.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "ok": true, "command": "tree", "path": file.display().to_string(), "tree": tree })
            )
            .expect("json output")
        );
    } else {
        let mut out = String::new();
        print_node(&tree, 0, parsed.has("props"), true, max_depth, &mut out);
        print!("{out}");
    }
    Ok(())
}

pub fn print_node(node: &Node, depth: usize, props: bool, is_root: bool, max_depth: Option<usize>, out: &mut String) {
    let indent = "  ".repeat(depth);
    if is_root {
        out.push_str(&format!("{} [{}] (#{})\n", node.name, node.class_name, node.referent));
    } else {
        out.push_str(&format!("{indent}{} [{}] (#{})\n", node.name, node.class_name, node.referent));
    }
    if props {
        for (name, value) in &node.properties {
            out.push_str(&format!(
                "{indent}  {name} = {} ({})\n",
                value.display(),
                value.type_name()
            ));
        }
    }
    if let Some(max) = max_depth {
        if depth >= max {
            if !node.children.is_empty() {
                out.push_str(&format!("{}  ... ({} children)\n", indent, node.children.len()));
            }
            return;
        }
    }
    for child in &node.children {
        print_node(child, depth + 1, props, false, max_depth, out);
    }
}

pub fn cmd_add(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["parent", "class", "name"], &[])?;
    let file = parsed.file()?;
    let parent = parsed.required("parent")?.to_string();
    let class = parsed.required("class")?.to_string();
    let name = parsed.opt("name").unwrap_or(&class).to_string();

    let (added, tree) = mutate(global, &file, |engine| engine.add(&parent, &class, &name))?;

    let path = added.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
    emit(
        global,
        format!("added {class} at {path} in {}", file.display()),
        json!({
            "ok": true, "command": "add", "path": file.display().to_string(),
            "instancePath": path, "className": class, "name": name,
            "instances": tree.count(),
        }),
    );
    Ok(())
}

pub fn cmd_set(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "targets", "property", "value", "type"], &[])?;
    let file = parsed.file()?;
    // Multi-selection bulk edit: --targets a,b,c applies one typed
    // property to every listed instance (Phase 21).
    let targets: Vec<String> = if let Some(list) = parsed.opt("targets") {
        list.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
    } else {
        vec![parsed.required("target")?.to_string()]
    };
    if targets.is_empty() {
        return Err(usage_err("set requires --target <path> or --targets <a,b,c>"));
    }
    let target = targets[0].clone();
    let property = parsed.required("property")?.to_string();
    let raw_value = parsed.required("value")?.to_string();

    let (mut engine, _format) = load_session(global, &file)?;
    // Determine the value type: explicit --type, else the existing
    // property's type, else the engine reflection default visible after
    // `add`. Renaming via Name is always a String.
    let type_name = if property == "Name" {
        "String".to_string()
    } else if let Some(t) = parsed.opt("type") {
        t.to_string()
    } else {
        let instance = engine.get(&target)?;
        match instance.properties.get(&property) {
            Some(existing) => existing.type_name().to_string(),
            None => {
                engine.shutdown();
                return Err(Error(format!(
                    "property '{property}' does not exist on '{target}'; pass --type \
                     (Bool|Int32|Int64|Float32|Float64|String|ContentId|Vector3|Color3|CFrame|Enum|BrickColor|Tags)"
                )));
            }
        }
    };
    let value = parse_value(&type_name, &raw_value, &property)?;
    for t in &targets {
        engine.set(t, &property, &value)?;
    }
    let tree = engine.snapshot()?;
    engine.shutdown();
    write_mutation(&file, &tree)?;

    emit(
        global,
        format!("set {}.{property} = {} on {} instance(s) in {}", targets.join(", "), value.display(), targets.len(), file.display()),
        json!({
            "ok": true, "command": "set", "path": file.display().to_string(),
            "target": target, "targets": targets, "property": property, "value": value,
        }),
    );
    Ok(())
}

pub fn cmd_save(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &[], &[])?;
    let file = parsed.file()?;
    let (mut engine, format) = load_session(global, &file)?;
    let report = engine.validate()?;
    let tree = engine.snapshot()?;
    engine.shutdown();
    if report.get("valid").and_then(|v| v.as_bool()) != Some(true) {
        return Err(Error(format!(
            "'{}' failed engine validation; not rewriting it. Issues: {}",
            file.display(),
            report.get("issues").cloned().unwrap_or(JsonValue::Null)
        )));
    }
    let place_dir = file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let guard = check_export_integrity(&tree, &place_dir);
    if !integrity_errors(&guard).is_empty() {
        return Err(Error(blocked_message(&guard)));
    }
    let guard_warnings: Vec<JsonValue> = integrity_warnings(&guard).iter().map(|i| i.to_json()).collect();
    write_mutation(&file, &tree)?;
    emit(
        global,
        format!(
            "saved {} ({}) — {} instance(s), canonical rewrite verified by re-decode",
            file.display(),
            format_name(format),
            tree.count()
        ),
        json!({
            "ok": true, "command": "save", "path": file.display().to_string(),
            "format": format_name(format), "instances": tree.count(),
            "integrityWarnings": guard_warnings,
        }),
    );
    Ok(())
}

pub fn cmd_export(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &[], &["force"])?;
    if parsed.positional.len() != 2 {
        return Err(usage_err("export requires <input> and <output>"));
    }
    let input = PathBuf::from(&parsed.positional[0]);
    let output = PathBuf::from(&parsed.positional[1]);
    format_for_path(&output)?;
    if output.exists() && !parsed.has("force") {
        return Err(Error(format!(
            "'{}' already exists; refusing to overwrite (pass --force)",
            output.display()
        )));
    }
    let (mut engine, in_format) = load_session(global, &input)?;
    let tree = engine.snapshot()?;
    engine.shutdown();
    let place_dir = input.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let guard = check_export_integrity(&tree, &place_dir);
    if !integrity_errors(&guard).is_empty() {
        return Err(Error(blocked_message(&guard)));
    }
    let guard_warnings: Vec<JsonValue> = integrity_warnings(&guard).iter().map(|i| i.to_json()).collect();
    let out_format = write_place(&output, &tree)?;
    emit(
        global,
        format!(
            "exported {} ({}) -> {} ({}) — {} instance(s)",
            input.display(),
            format_name(in_format),
            output.display(),
            format_name(out_format),
            tree.count()
        ),
        json!({
            "ok": true, "command": "export",
            "input": input.display().to_string(), "output": output.display().to_string(),
            "inputFormat": format_name(in_format), "outputFormat": format_name(out_format),
            "instances": tree.count(),
            "integrityWarnings": guard_warnings,
        }),
    );
    Ok(())
}


// ---------- Phase 23: script IDE + debugger CLI ----------

pub fn cmd_get(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "property"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    let (mut engine, _format) = load_session(global, &file)?;
    let instance = engine.get(&target)?;
    engine.shutdown();

    if let Some(property) = parsed.opt("property") {
        // Derived BasePart properties: Position mirrors CFrame.
        let value = if property == "Position" {
            match instance.properties.get("CFrame") {
                Some(PropValue::CFrame(cf)) => Some(PropValue::Vector3(cf.position)),
                _ => instance.properties.get("Position").cloned(),
            }
        } else if property == "Name" {
            Some(PropValue::String(instance.name.clone()))
        } else if property == "ClassName" {
            Some(PropValue::String(instance.class_name.clone()))
        } else {
            instance.properties.get(property).cloned()
        };
        let Some(value) = value else {
            return Err(Error(format!(
                "property '{property}' not found on {target} ({}); it has: {}",
                instance.class_name,
                instance
                    .properties
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        emit(
            global,
            format!("{}.{} = {} ({})", target, property, value.display(), value.type_name()),
            json!({
                "ok": true, "command": "get", "path": file.display().to_string(),
                "target": target, "property": property, "value": value,
                "className": instance.class_name, "referent": instance.referent,
            }),
        );
    } else {
        emit(
            global,
            format!(
                "{} [{}] (#{}) — {} properties, {} children",
                instance.name,
                instance.class_name,
                instance.referent,
                instance.properties.len(),
                instance.children.len()
            ),
            json!({
                "ok": true, "command": "get", "path": file.display().to_string(),
                "target": target, "instance": instance,
            }),
        );
    }
    Ok(())
}

pub fn cmd_find(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["name", "class", "under"], &["contains"])?;
    let file = parsed.file()?;
    if parsed.opt("name").is_none() && parsed.opt("class").is_none() {
        return Err(usage_err("find requires --name and/or --class"));
    }
    let (mut engine, _format) = load_session(global, &file)?;
    let result = engine.find(
        parsed.opt("name"),
        parsed.opt("class"),
        parsed.has("contains"),
        parsed.opt("under"),
    )?;
    engine.shutdown();
    let matches = result.get("matches").cloned().unwrap_or(json!([]));
    let human = {
        let mut lines = vec![format!("{} match(es) in {}", result.get("count").and_then(|c| c.as_i64()).unwrap_or(0), file.display())];
        if let Some(arr) = matches.as_array() {
            for m in arr {
                lines.push(format!(
                    "  {} [{}] (#{})",
                    m.get("path").and_then(|v| v.as_str()).unwrap_or("?"),
                    m.get("className").and_then(|v| v.as_str()).unwrap_or("?"),
                    m.get("referent").and_then(|v| v.as_i64()).unwrap_or(0)
                ));
            }
        }
        lines.join("\n")
    };
    emit(
        global,
        human,
        json!({
            "ok": true, "command": "find", "path": file.display().to_string(),
            "matches": matches, "count": result.get("count").cloned().unwrap_or(json!(0)),
        }),
    );
    Ok(())
}

pub fn cmd_clone(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "name", "parent"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    let name = parsed.opt("name").map(|s| s.to_string());
    let parent = parsed.opt("parent").map(|s| s.to_string());
    let (result, _tree) = mutate(global, &file, |engine| {
        engine.clone_instance(&target, name.as_deref(), parent.as_deref())
    })?;
    let path = result.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
    emit(
        global,
        format!("cloned {target} -> {path} in {}", file.display()),
        json!({
            "ok": true, "command": "clone", "path": file.display().to_string(),
            "source": target, "instancePath": path,
            "referent": result.get("referent").cloned().unwrap_or(json!(null)),
            "className": result.get("className").cloned().unwrap_or(json!(null)),
        }),
    );
    Ok(())
}

pub fn cmd_delete(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    let (result, tree) = mutate(global, &file, |engine| engine.delete(&target))?;
    let deleted = result.get("deletedCount").and_then(|v| v.as_i64()).unwrap_or(0);
    emit(
        global,
        format!("deleted {target} ({deleted} instance(s)) from {}", file.display()),
        json!({
            "ok": true, "command": "delete", "path": file.display().to_string(),
            "target": target, "deletedCount": deleted, "instances": tree.count(),
        }),
    );
    Ok(())
}

pub fn cmd_reparent(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "parent"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    let parent = parsed.required("parent")?.to_string();
    let (result, _tree) = mutate(global, &file, |engine| engine.reparent(&target, &parent))?;
    let path = result.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
    emit(
        global,
        format!("reparented {target} -> {path} in {}", file.display()),
        json!({
            "ok": true, "command": "reparent", "path": file.display().to_string(),
            "target": target, "instancePath": path,
        }),
    );
    Ok(())
}

pub fn cmd_group(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["targets", "name", "parent"], &[])?;
    let file = parsed.file()?;
    let targets_raw = parsed.required("targets")?;
    let targets: Vec<String> = targets_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if targets.is_empty() {
        return Err(usage_err("group requires at least one --targets path"));
    }
    let name = parsed.opt("name").unwrap_or("Model").to_string();
    let parent = parsed.opt("parent").map(|s| s.to_string());
    let (result, _tree) = mutate(global, &file, |engine| {
        engine.group(&targets, &name, parent.as_deref())
    })?;
    let path = result.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
    emit(
        global,
        format!("grouped {} instance(s) under {path} in {}", targets.len(), file.display()),
        json!({
            "ok": true, "command": "group", "path": file.display().to_string(),
            "instancePath": path, "groupedCount": targets.len(),
        }),
    );
    Ok(())
}

pub fn cmd_ungroup(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    let parent = target
        .rsplit_once('/')
        .map(|(p, _)| p.to_string())
        .ok_or_else(|| usage_err("ungroup target must be inside a parent (e.g. Workspace/Model)"))?;
    let (result, _tree) = mutate(global, &file, |engine| {
        let node = engine.get(&target)?;
        if node.children.is_empty() {
            return Err(Error(format!("'{target}' has no children to ungroup")));
        }
        let mut moved = 0;
        for child in &node.children {
            engine.reparent(&format!("{target}/{}", child.name), &parent)?;
            moved += 1;
        }
        engine.delete(&target)?;
        Ok(json!({ "moved": moved }))
    })?;
    emit(
        global,
        format!("ungrouped {} — {} child instance(s) moved to {parent} in {}", target, result["moved"], file.display()),
        json!({
            "ok": true, "command": "ungroup", "path": file.display().to_string(),
            "target": target, "parent": parent, "moved": result["moved"],
        }),
    );
    Ok(())
}

/// Studio Command Bar equivalent: Luau executed against the EDIT tree
/// (no players, no playtest lifecycle) whose resulting state is
/// committed back to the place file. Distinct from `play` sessions.
pub fn cmd_undo(global: &Global, args: &[String], redo: bool) -> Result<()> {
    let parsed = parse_cmd(args, &[], &[])?;
    let file = parsed.file()?;
    if !file.is_file() {
        return Err(Error(format!("'{}' does not exist", file.display())));
    }
    let mut history = History::open(&file)?;
    let snapshot = if redo {
        history.redo(&file)?
    } else {
        history.undo(&file)?
    };
    // The restored tree must still load into the engine cleanly.
    let (mut engine, _format) = load_session(global, &file)?;
    let tree = engine.snapshot()?;
    engine.shutdown();
    emit(
        global,
        format!(
            "{} {} from snapshot {} — now {} instance(s)",
            if redo { "redid" } else { "undid" },
            file.display(),
            snapshot,
            tree.count()
        ),
        json!({
            "ok": true, "command": if redo { "redo" } else { "undo" },
            "path": file.display().to_string(), "snapshot": snapshot,
            "instances": tree.count(),
            "undoDepth": history.undo_depth(), "redoDepth": history.redo_depth(),
        }),
    );
    Ok(())
}

pub fn cmd_validate(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &[], &[])?;
    let file = parsed.file()?;

    // Decode first: a file that does not parse is invalid, full stop.
    let (node, format) = match read_place(&file) {
        Ok(v) => v,
        Err(e) => {
            emit(
                global,
                format!("INVALID {}: {}", file.display(), e.0),
                json!({ "ok": false, "command": "validate", "path": file.display().to_string(),
                        "valid": false, "error": e.0 }),
            );
            return Err(Error("validation failed".to_string()));
        }
    };
    let mut engine = spawn_engine(global)?;
    if let Err(e) = engine.load(&node) {
        engine.shutdown();
        emit(
            global,
            format!("INVALID {}: engine rejected the tree: {}", file.display(), e.0),
            json!({ "ok": false, "command": "validate", "path": file.display().to_string(),
                    "valid": false, "error": e.0 }),
        );
        return Err(Error("validation failed".to_string()));
    }
    let report = engine.validate()?;
    engine.shutdown();

    // Merge engine issues with reflection-database issues (official Roblox
    // API metadata), deduplicated by path + message.
    let mut merged: Vec<JsonValue> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(arr) = report.get("issues").and_then(|i| i.as_array()) {
        for issue in arr {
            let key = format!(
                "{}|{}",
                issue.get("path").and_then(|v| v.as_str()).unwrap_or(""),
                issue.get("message").and_then(|v| v.as_str()).unwrap_or("")
            );
            if seen.insert(key) {
                merged.push(issue.clone());
            }
        }
    }
    for issue in validate_reflection(&node) {
        let key = format!("{}|{}", issue.path, issue.message);
        if seen.insert(key) {
            merged.push(json!({
                "severity": issue.severity, "path": issue.path, "message": issue.message,
                "source": "reflection",
            }));
        }
    }
    // Export integrity guard (Phase 17): dangling references, corrupted
    // terrain payloads, unresolved assets, source-less scripts.
    let place_dir = file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let guard = check_export_integrity(&node, &place_dir);
    let integrity_json = json!({
        "errors": integrity_errors(&guard).len(),
        "warnings": integrity_warnings(&guard).len(),
    });
    for issue in &guard {
        let key = format!("{}|{}", issue.path, issue.message);
        if seen.insert(key) {
            merged.push(issue.to_json());
        }
    }
    let has_errors = merged
        .iter()
        .any(|i| i.get("severity").and_then(|s| s.as_str()) == Some("error"));
    let valid = !has_errors;
    let issues = JsonValue::Array(merged);
    let human = if valid {
        format!(
            "VALID {} ({}) — {} instance(s), {} warning(s)",
            file.display(),
            format_name(format),
            node.count(),
            issues.as_array().map(|a| a.len()).unwrap_or(0)
        )
    } else {
        format!(
            "INVALID {} ({}) — issues: {}",
            file.display(),
            format_name(format),
            serde_json::to_string_pretty(&issues).unwrap_or_default()
        )
    };
    emit(
        global,
        human,
        json!({
            "ok": valid, "command": "validate", "path": file.display().to_string(),
            "format": format_name(format), "valid": valid,
            "instances": node.count(), "issues": issues,
            "integrity": integrity_json,
        }),
    );
    if valid {
        Ok(())
    } else {
        Err(Error("validation failed".to_string()))
    }
}
