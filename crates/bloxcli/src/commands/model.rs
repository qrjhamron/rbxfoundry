//! Model commands: single-root .rbxm/.rbxmx export/inspect/import with referent rewriting.

use std::path::{Path, PathBuf};
use serde_json::{json, Value as JsonValue};
use crate::cli::args::{parse_cmd, usage_err, Global};
use crate::app::session::{emit, format_name, spawn_engine, write_mutation};
use crate::protocol::tree::{Error, Node, PropValue, Result};
use crate::rbxio::{read_model, read_place, write_model, PlaceFormat};
use crate::validation::integrity::{blocked_message, check_export_integrity, errors as integrity_errors, warnings as integrity_warnings, ref_index_path};

pub fn display_to_index(root: &Node, path: &str) -> Result<Vec<usize>> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut cur = root;
    let mut index_path = Vec::new();
    let mut iter = parts.iter().peekable();
    if let Some(first) = iter.peek() {
        if **first == "game" || **first == root.name.as_str() || **first == root.class_name.as_str() {
            iter.next();
        }
    }
    for part in iter {
        let matches: Vec<usize> = cur
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| &c.name == part)
            .map(|(i, _)| i)
            .collect();
        match matches.len() {
            0 => return Err(Error(format!("no instance named '{part}' under '{}'", cur.name))),
            1 => {
                index_path.push(matches[0]);
                cur = &cur.children[matches[0]];
            }
            n => {
                return Err(Error(format!(
                    "path '{path}' is ambiguous: {n} siblings named '{part}'"
                )))
            }
        }
    }
    Ok(index_path)
}

pub fn join_index_path(path: &[usize]) -> String {
    path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("/")
}

/// Rewrites every non-empty Ref in a cloned model subtree from
/// place-absolute form to model-relative index paths. References that
/// leave the subtree fail the export; references to the subtree root
/// itself become the "." self-reference token (model roots are
/// serialized, so this round-trips losslessly).
pub fn rewrite_refs_for_model_export(
    place_root: &Node,
    subtree_abs: &[usize],
    node: &mut Node,
    display: &str,
) -> Result<()> {
    for (name, value) in node.properties.iter_mut() {
        if let PropValue::Ref(target) = value {
            if target.is_empty() {
                continue;
            }
            let abs = ref_index_path(place_root, target).ok_or_else(|| {
                Error(format!(
                    "property '{name}' on '{display}' references '{target}', which does not exist"
                ))
            })?;
            if !abs.starts_with(subtree_abs) {
                return Err(Error(format!(
                    "property '{name}' on '{display}' references '{target}' outside the exported subtree; model export would lose it"
                )));
            }
            let rel = &abs[subtree_abs.len()..];
            if rel.is_empty() {
                // Reference to the model root itself: distinct "."
                // self-reference token (model roots are serialized).
                *target = ".".to_string();
            } else {
                *target = join_index_path(rel);
            }
        }
    }
    for child in node.children.iter_mut() {
        rewrite_refs_for_model_export(place_root, subtree_abs, child, &format!("{display}/{}", child.name))?;
    }
    Ok(())
}

/// Rewrites model-relative refs to place-absolute index paths after a
/// graft at `graft_abs` (the index path the model root will occupy).
pub fn rewrite_refs_for_model_import(model_root: &Node, graft_abs: &[usize], node: &mut Node, display: &str) -> Result<()> {
    for (name, value) in node.properties.iter_mut() {
        if let PropValue::Ref(target) = value {
            if target.is_empty() {
                continue;
            }
            let rel = ref_index_path(model_root, target).ok_or_else(|| {
                Error(format!(
                    "property '{name}' on '{display}' references '{target}', which does not exist inside the model"
                ))
            })?;
            let mut abs = graft_abs.to_vec();
            abs.extend_from_slice(&rel);
            *target = join_index_path(&abs);
        }
    }
    for child in node.children.iter_mut() {
        rewrite_refs_for_model_import(model_root, graft_abs, child, &format!("{display}/{}", child.name))?;
    }
    Ok(())
}

pub fn cmd_model(global: &Global, args: &[String]) -> Result<()> {
    let Some(sub) = args.first().cloned() else {
        return Err(usage_err("model requires a subcommand: export, inspect, import"));
    };
    let sub_args: Vec<String> = args.iter().skip(1).cloned().collect();
    match sub.as_str() {
        "export" => {
            let parsed = parse_cmd(&sub_args, &["output", "target"], &["force"])?;
            let file = parsed.file()?;
            let output = parsed
                .opt("output")
                .map(PathBuf::from)
                .ok_or_else(|| usage_err("model export requires --output <file.rbxm|.rbxmx>"))?;
            let target = parsed
                .opt("target")
                .ok_or_else(|| usage_err("model export requires --target <path>"))?
                .to_string();
            if output.exists() && !parsed.has("force") {
                return Err(Error(format!(
                    "'{}' already exists; refusing to overwrite (pass --force)",
                    output.display()
                )));
            }
            let (place, _format) = read_place(&file)?;
            let subtree_abs = display_to_index(&place, &target)?;
            let target_node = place
                .find_path(&target)
                .map_err(|e| Error(e.0))?;
            if target_node.class_name == "DataModel" {
                return Err(usage_err("model export target must be below the DataModel root"));
            }
            let mut model = target_node.clone();
            rewrite_refs_for_model_export(&place, &subtree_abs, &mut model, &target)?;
            let place_dir = file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
            let guard = check_export_integrity(&model, &place_dir);
            if !integrity_errors(&guard).is_empty() {
                return Err(Error(blocked_message(&guard)));
            }
            let warnings: Vec<JsonValue> =
                integrity_warnings(&guard).iter().map(|i| i.to_json()).collect();
            let out_format = write_model(&output, &model)?;
            emit(
                global,
                format!(
                    "exported model '{}' ({} instance(s)) from {} to {} ({})",
                    model.name,
                    model.count(),
                    file.display(),
                    output.display(),
                    format_name(out_format)
                ),
                json!({
                    "ok": true, "command": "model export",
                    "input": file.display().to_string(), "output": output.display().to_string(),
                    "target": target, "rootClass": model.class_name, "name": model.name,
                    "instances": model.count(), "format": format_name(out_format),
                    "integrityWarnings": warnings,
                }),
            );
            Ok(())
        }
        "inspect" => {
            let parsed = parse_cmd(&sub_args, &[], &[])?;
            let file = parsed.file()?;
            let (node, format) = read_model(&file)?;
            emit(
                global,
                format!(
                    "{} ({}) — model '{}' ({}), {} instance(s)",
                    file.display(),
                    format_name(format),
                    node.name,
                    node.class_name,
                    node.count()
                ),
                json!({
                    "ok": true, "command": "model inspect", "path": file.display().to_string(),
                    "format": format_name(format), "rootClass": node.class_name,
                    "name": node.name, "instances": node.count(), "tree": node,
                }),
            );
            Ok(())
        }
        "import" => {
            let parsed = parse_cmd(&sub_args, &["parent"], &[])?;
            if parsed.positional.len() != 2 {
                return Err(usage_err("model import requires <model.rbxm> and <place.rbxl>"));
            }
            let model_file = PathBuf::from(&parsed.positional[0]);
            let place_file = PathBuf::from(&parsed.positional[1]);
            let parent = parsed
                .opt("parent")
                .ok_or_else(|| usage_err("model import requires --parent <path>"))?
                .to_string();
            let (model, _mf) = read_model(&model_file)?;
            let model_dir = model_file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
            let guard = check_export_integrity(&model, &model_dir);
            if !integrity_errors(&guard).is_empty() {
                return Err(Error(blocked_message(&guard)));
            }
            let (mut place, _pf) = read_place(&place_file)?;
            let parent_abs = display_to_index(&place, &parent)?;
            let parent_node = crate::validation::integrity::node_at(&place, &parent_abs)
                .ok_or_else(|| Error(format!("parent '{parent}' not found")))?;
            let mut graft_abs = parent_abs.clone();
            graft_abs.push(parent_node.children.len());
            let mut graft = model.clone();
            rewrite_refs_for_model_import(&model, &graft_abs, &mut graft, &model.name)?;
            let parent_mut = {
                let mut cur = &mut place;
                for &idx in &parent_abs {
                    cur = &mut cur.children[idx];
                }
                cur
            };
            parent_mut.children.push(graft);
            // The engine is the runtime source of truth: verify the
            // merged tree loads and snapshot its canonical form.
            let mut engine = spawn_engine(global)?;
            let load = engine.load(&place).and_then(|_| engine.snapshot());
            engine.shutdown();
            let tree = load?;
            let place_dir = place_file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
            let guard = check_export_integrity(&tree, &place_dir);
            if !integrity_errors(&guard).is_empty() {
                return Err(Error(blocked_message(&guard)));
            }
            write_mutation(&place_file, &tree)?;
            emit(
                global,
                format!(
                    "imported model '{}' into {} under {} ({} instance(s) total)",
                    model.name,
                    place_file.display(),
                    parent,
                    tree.count()
                ),
                json!({
                    "ok": true, "command": "model import",
                    "model": model_file.display().to_string(),
                    "path": place_file.display().to_string(),
                    "parent": parent, "name": model.name, "rootClass": model.class_name,
                    "instances": tree.count(),
                }),
            );
            Ok(())
        }
        other => Err(usage_err(&format!(
            "unknown model subcommand '{other}' (export, inspect, import)"
        ))),
    }
}

// ---------- Phase 18: Parity Lab commands ----------

/// Reads a place or a model file, chosen by extension.
pub fn read_any(path: &Path) -> Result<(Node, PlaceFormat)> {
    if matches!(path.extension().and_then(|e| e.to_str()), Some("rbxm") | Some("rbxmx")) {
        read_model(path)
    } else {
        read_place(path)
    }
}
