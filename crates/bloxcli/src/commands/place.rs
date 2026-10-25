//! Place commands: new/place/template/diff/imagediff/parity.

use std::path::PathBuf;
use serde_json::{json, Value as JsonValue};
use crate::cli::args::{parse_cmd, usage_err, Global};
use crate::app::session::{emit, format_name, spawn_engine};
use crate::protocol::tree::{Error, Node, PropValue, Result};
use crate::rbxio::{format_for_path, read_place, write_place};
use crate::validation::integrity::{check_export_integrity, errors as integrity_errors, warnings as integrity_warnings};
use crate::commands::model::read_any;

pub fn cmd_new(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["name", "template"], &["force"])?;
    let file = parsed.file()?;
    format_for_path(&file)?;
    if file.exists() && !parsed.has("force") {
        return Err(Error(format!(
            "'{}' already exists; refusing to overwrite (pass --force)",
            file.display()
        )));
    }
    let default_name = file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Place".to_string());
    let name = parsed.opt("name").unwrap_or(&default_name).to_string();
    let template = parsed.opt("template").unwrap_or("empty").to_string();
    if crate::templates::find(&template).is_none() {
        return Err(usage_err(&format!(
            "unknown template '{template}' (available: {})",
            crate::templates::registry()
                .iter()
                .map(|t| t.name)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let mut engine = spawn_engine(global)?;
    engine.new_datamodel(&name)?;
    // The standard place service set (Phase 2). Everything else can be
    // added by class name with `add`.
    for (class, service_name) in [
        ("Workspace", "Workspace"),
        ("Lighting", "Lighting"),
        ("ReplicatedStorage", "ReplicatedStorage"),
        ("ServerStorage", "ServerStorage"),
        ("ServerScriptService", "ServerScriptService"),
        ("StarterGui", "StarterGui"),
        ("StarterPlayer", "StarterPlayer"),
        ("StarterPack", "StarterPack"),
        ("Teams", "Teams"),
        ("SoundService", "SoundService"),
    ] {
        engine.add("game", class, service_name)?;
    }
    crate::templates::apply(&mut engine, &template)?;
    let tree = engine.snapshot()?;
    let engine_version = engine.engine_version.clone();
    engine.shutdown();

    let format = write_place(&file, &tree)?;
    emit(
        global,
        format!(
            "created {} ({}) with the standard service set, {} instance(s)",
            file.display(),
            format_name(format),
            tree.count()
        ),
        json!({
            "ok": true, "command": "new", "path": file.display().to_string(),
            "format": format_name(format), "placeName": name, "template": template,
            "instances": tree.count(), "engineVersion": engine_version,
        }),
    );
    Ok(())
}

/// Records the file's current bytes in undo history, then writes the new
/// tree. Used by every mutating command.
pub fn cmd_place(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("place needs a subcommand: info"));
    };
    match sub.as_str() {
        "info" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let (node, format) = read_place(&file)?;
            let int_prop = |name: &str| -> i64 {
                match node.properties.get(name) {
                    Some(PropValue::Int64(v)) => *v,
                    Some(PropValue::Int32(v)) => *v as i64,
                    _ => 0,
                }
            };
            let str_prop = |name: &str| -> String {
                match node.properties.get(name) {
                    Some(PropValue::String(v)) => v.clone(),
                    _ => String::new(),
                }
            };
            let place_id = int_prop("PlaceId");
            let game_id = str_prop("GameId");
            let place_version = int_prop("PlaceVersion");
            let job_id = str_prop("JobId");
            let metadata_state = if place_id == 0 && game_id.is_empty() {
                "offline-unpublished (PlaceId=0, no GameId; Roblox does not store published place/universe ids inside a local .rbxl, so none are invented)"
            } else {
                "stored in canonical tree root properties (not serialized into RBXL binary/XML; reloaded files report offline-unpublished unless re-supplied)"
            };
            let services: Vec<String> = node.children.iter().map(|c| c.name.clone()).collect();
            let terrain = node
                .find_path("Workspace/Terrain")
                .ok()
                .map(|terrain| {
                    let voxels = matches!(
                        terrain.properties.get("BloxVoxels"),
                        Some(PropValue::BinaryString(v)) if !v.is_empty()
                    );
                    json!({
                        "present": true,
                        "voxelPayload": if voxels { "BloxVoxels (BTR1)" } else { "empty" },
                    })
                })
                .unwrap_or_else(|| json!({"present": false}));
            let info = json!({
                "path": file.display().to_string(),
                "format": format_name(format),
                "name": node.name,
                "className": node.class_name,
                "instances": node.count(),
                "services": services,
                "metadata": {
                    "placeId": place_id,
                    "gameId": game_id,
                    "placeVersion": place_version,
                    "jobId": job_id,
                    "creatorId": int_prop("CreatorId"),
                    "state": metadata_state,
                },
                "terrain": terrain,
            });
            emit(
                global,
                format!(
                    "{}: {} ({}), {} instance(s), PlaceId {} ({})",
                    file.display(),
                    node.name,
                    format_name(format),
                    node.count(),
                    place_id,
                    if place_id == 0 { "offline-unpublished" } else { "supplied metadata" }
                ),
                json!({"ok": true, "command": "place info", "info": info}),
            );
            Ok(())
        }
        "manifest" => {
            // Sidecar manifest (Phase 19): records offline metadata
            // that genuine .rbxl files cannot carry (published
            // PlaceId/UniverseId live in Roblox's online metadata).
            // Nothing is injected into the binary/XML place file.
            let parsed = parse_cmd(rest, &["output"], &[])?;
            let file = parsed.file()?;
            let (node, format) = read_place(&file)?;
            let output = parsed.opt("output").map(PathBuf::from).unwrap_or_else(|| {
                let mut name = file.file_name().unwrap_or_default().to_os_string();
                name.push(".manifest.json");
                file.with_file_name(name)
            });
            let mut class_counts = serde_json::Map::new();
            fn count_classes(node: &Node, map: &mut serde_json::Map<String, JsonValue>) {
                let entry = map
                    .entry(node.class_name.clone())
                    .or_insert_with(|| json!(0));
                *entry = json!(entry.as_i64().unwrap_or(0) + 1);
                for child in &node.children {
                    count_classes(child, map);
                }
            }
            count_classes(&node, &mut class_counts);
            let place_dir = file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
            let guard = check_export_integrity(&node, &place_dir);
            let get_i = |name: &str| match node.properties.get(name) {
                Some(PropValue::Int64(v)) => *v,
                Some(PropValue::Int32(v)) => *v as i64,
                _ => 0,
            };
            let get_s = |name: &str| match node.properties.get(name) {
                Some(PropValue::String(v)) => v.clone(),
                _ => String::new(),
            };
            let manifest = json!({
                "manifestVersion": 1,
                "generatedBy": "RBXFoundry place manifest (sidecar; not part of the .rbxl file)",
                "placeFile": file.display().to_string(),
                "format": format_name(format),
                "fileBytes": std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0),
                "instances": node.count(),
                "services": node.children.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                "classCounts": class_counts,
                "metadata": {
                    "placeId": get_i("PlaceId"),
                    "gameId": get_s("GameId"),
                    "placeVersion": get_i("PlaceVersion"),
                    "jobId": get_s("JobId"),
                    "creatorId": get_i("CreatorId"),
                    "state": "offline-unpublished unless explicitly supplied; Roblox-issued ids are never invented",
                },
                "integrity": {
                    "errors": integrity_errors(&guard).len(),
                    "warnings": integrity_warnings(&guard).len(),
                },
            });
            let text = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
            std::fs::write(&output, text).map_err(|e| Error(format!("cannot write {}: {e}", output.display())))?;
            emit(
                global,
                format!("wrote sidecar manifest {} for {} ({} instance(s))", output.display(), file.display(), node.count()),
                json!({"ok": true, "command": "place manifest", "output": output.display().to_string(), "manifest": manifest}),
            );
            Ok(())
        }
        other => Err(usage_err(&format!("unknown place subcommand '{other}' (info, manifest)"))),
    }
}

pub fn cmd_template(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("template needs a subcommand: list|inspect|compare"));
    };
    match sub.as_str() {
        "list" => {
            let templates = crate::templates::registry()
                .iter()
                .map(|t| json!({"name": t.name, "description": t.description, "status": t.status}))
                .collect::<Vec<_>>();
            emit(
                global,
                templates
                    .iter()
                    .map(|t| format!("{} — {}", t["name"].as_str().unwrap_or(""), t["description"].as_str().unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join("\n"),
                json!({"ok": true, "command": "template list", "templates": templates}),
            );
            Ok(())
        }
        "inspect" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let name = parsed
                .positional
                .first()
                .ok_or_else(|| usage_err("template inspect requires a template name"))?;
            let template = crate::templates::find(name)
                .ok_or_else(|| usage_err(&format!("unknown template '{name}'")))?;
            let info = json!({
                "name": template.name,
                "description": template.description,
                "status": template.status,
                "contents": template.contents,
            });
            emit(
                global,
                format!("template {}: {}\nstatus: {}", template.name, template.description, template.status),
                json!({"ok": true, "command": "template inspect", "template": info}),
            );
            Ok(())
        }
        "compare" => {
            // Builds the named template in the engine and semantically
            // diffs it against a supplied place file (e.g. a genuine
            // Studio fixture). Provenance is the caller's: results are
            // UNTESTED as Studio parity until the fixture is a real
            // Studio export.
            let parsed = parse_cmd(rest, &["template"], &[])?;
            let file = parsed.file()?;
            let template_name = parsed.opt("template").unwrap_or("baseplate").to_string();
            if crate::templates::find(&template_name).is_none() {
                return Err(usage_err(&format!("unknown template '{template_name}'")));
            }
            let (fixture, _format) = read_place(&file)?;
            let mut engine = spawn_engine(global)?;
            engine.new_datamodel("Place")?;
            for (class, service_name) in [
                ("Workspace", "Workspace"),
                ("Lighting", "Lighting"),
                ("ReplicatedStorage", "ReplicatedStorage"),
                ("ServerStorage", "ServerStorage"),
                ("ServerScriptService", "ServerScriptService"),
                ("StarterGui", "StarterGui"),
                ("StarterPlayer", "StarterPlayer"),
                ("StarterPack", "StarterPack"),
                ("Teams", "Teams"),
                ("SoundService", "SoundService"),
            ] {
                engine.add("game", class, service_name)?;
            }
            crate::templates::apply(&mut engine, &template_name)?;
            let tree = engine.snapshot()?;
            engine.shutdown();
            let diffs = crate::diagnostics::diff::diff_trees(&tree, &fixture);
            let entries: Vec<JsonValue> = diffs.iter().map(|d| d.to_json()).collect();
            let identical = diffs.is_empty();
            emit(
                global,
                format!(
                    "template {template_name} vs {}: {} difference(s){}",
                    file.display(),
                    diffs.len(),
                    if identical { " (identical)" } else { "" }
                ),
                json!({
                    "ok": identical, "command": "template compare",
                    "template": template_name, "fixture": file.display().to_string(),
                    "status": "UNTESTED (fixture provenance unverified; not a Studio comparison unless the fixture is a genuine Studio export)",
                    "differenceCount": diffs.len(), "differences": entries,
                }),
            );
            if identical {
                Ok(())
            } else {
                Err(Error(format!("{} difference(s) found", diffs.len())))
            }
        }
        other => Err(usage_err(&format!("unknown template subcommand '{other}' (list, inspect, compare)"))),
    }
}

pub fn cmd_diff(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &[], &[])?;
    if parsed.positional.len() != 2 {
        return Err(usage_err("diff requires two files: <a.rbxl> <b.rbxl> (models too)"));
    }
    let a_path = PathBuf::from(&parsed.positional[0]);
    let b_path = PathBuf::from(&parsed.positional[1]);
    let (a, _) = read_any(&a_path)?;
    let (b, _) = read_any(&b_path)?;
    let diffs = crate::diagnostics::diff::diff_trees(&a, &b);
    let entries: Vec<JsonValue> = diffs.iter().map(|d| d.to_json()).collect();
    let identical = diffs.is_empty();
    emit(
        global,
        if identical {
            format!("{} and {} are semantically identical ({} instance(s))", a_path.display(), b_path.display(), a.count())
        } else {
            format!("{} difference(s) between {} and {}", diffs.len(), a_path.display(), b_path.display())
        },
        json!({
            "ok": identical, "command": "diff",
            "a": a_path.display().to_string(), "b": b_path.display().to_string(),
            "instancesA": a.count(), "instancesB": b.count(),
            "differenceCount": diffs.len(), "differences": entries,
        }),
    );
    if identical {
        Ok(())
    } else {
        Err(Error(format!("{} difference(s) found", diffs.len())))
    }
}

pub fn cmd_imagediff(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["heatmap", "tolerance"], &[])?;
    if parsed.positional.len() != 2 {
        return Err(usage_err("imagediff requires two PNGs: <a.png> <b.png>"));
    }
    let tolerance: f64 = parsed
        .opt("tolerance")
        .unwrap_or("0")
        .parse()
        .map_err(|_| usage_err("--tolerance must be a number (max differing-pixel fraction)"))?;
    let mut engine = spawn_engine(global)?;
    let result = engine.call(
        "parity/imagediff",
        json!({
            "a": parsed.positional[0], "b": parsed.positional[1],
            "heatmap": parsed.opt("heatmap").unwrap_or(""),
        }),
    );
    engine.shutdown();
    let result = result?;
    let fraction = result["diffFraction"].as_f64().unwrap_or(1.0);
    let within = fraction <= tolerance;
    emit(
        global,
        format!(
            "{} vs {}: MAE {:.2}, RMSE {:.2}, max delta {}, {:.2}% pixels differ{}",
            parsed.positional[0], parsed.positional[1],
            result["mae"].as_f64().unwrap_or(0.0),
            result["rmse"].as_f64().unwrap_or(0.0),
            result["maxDelta"].as_i64().unwrap_or(0),
            fraction * 100.0,
            if within { " (within tolerance)" } else { "" }
        ),
        json!({
            "ok": within, "command": "imagediff",
            "a": parsed.positional[0], "b": parsed.positional[1],
            "tolerance": tolerance, "metrics": result,
        }),
    );
    if within {
        Ok(())
    } else {
        Err(Error(format!("images differ: {fraction:.4} of pixels beyond tolerance {tolerance}")))
    }
}

pub fn cmd_parity(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("parity needs a subcommand: dashboard"));
    };
    match sub.as_str() {
        // Static, offline HTML dashboard rendered from a compat report.
        "dashboard" => {
            let parsed = parse_cmd(rest, &["report", "output"], &[])?;
            let report_path = parsed
                .opt("report")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("compat-report.json"));
            let output = parsed
                .opt("output")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("compat-dashboard.html"));
            let raw = std::fs::read_to_string(&report_path).map_err(|e| {
                Error(format!("cannot read report {}: {e} (run `cargo test --test compat`)", report_path.display()))
            })?;
            let report: JsonValue =
                serde_json::from_str(&raw).map_err(|e| Error(format!("invalid report JSON: {e}")))?;
            let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            let mut rows = String::new();
            if let Some(checks) = report["checks"].as_array() {
                for c in checks {
                    let status = c["status"].as_str().unwrap_or("?");
                    rows.push_str(&format!(
                        "<tr><td>{}</td><td>{}</td><td class=\"s-{}\">{}</td><td>{}</td></tr>\n",
                        esc(c["id"].as_str().unwrap_or("")),
                        esc(c["area"].as_str().unwrap_or("")),
                        esc(&status.to_lowercase().replace('_', "-")),
                        esc(status),
                        esc(c["evidence"].as_str().unwrap_or("")),
                    ));
                }
            }
            let summary = &report["summary"];
            let html = format!(
                "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>RBXFoundry compatibility dashboard</title>\n\
                 <style>body{{font-family:sans-serif;margin:2em}}table{{border-collapse:collapse}}td,th{{border:1px solid #999;padding:4px 8px}}\
                 .s-internal-verified,.s-verified{{background:#c8e6c9}}.s-partial{{background:#fff9c4}}\
                 .s-unsupported{{background:#ffcdd2}}.s-untested{{background:#e0e0e0}}</style></head>\n\
                 <body><h1>RBXFoundry compatibility dashboard</h1>\n\
                 <p>{} checks: {} verified (internal automated checks only), {} partial, {} unsupported, {} untested. \
                 No Roblox Studio reference data was available; nothing here is a Studio comparison.</p>\n\
                 <table><tr><th>Check</th><th>Area</th><th>Status</th><th>Evidence</th></tr>\n{}</table></body></html>\n",
                summary["total"], summary["verified"], summary["partial"], summary["unsupported"], summary["untested"], rows
            );
            std::fs::write(&output, html).map_err(|e| Error(format!("cannot write {}: {e}", output.display())))?;
            emit(
                global,
                format!("wrote compatibility dashboard {} from {}", output.display(), report_path.display()),
                json!({"ok": true, "command": "parity dashboard", "output": output.display().to_string(), "report": report_path.display().to_string()}),
            );
            Ok(())
        }
        other => Err(usage_err(&format!("unknown parity subcommand '{other}' (dashboard)"))),
    }
}
