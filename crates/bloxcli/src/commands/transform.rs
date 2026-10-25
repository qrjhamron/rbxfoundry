//! Transform commands: move/rotate/scale/pivot.

use serde_json::json;
use crate::cli::args::{parse_cmd, usage_err, Global};
use crate::app::session::{emit, load_session, mutate, write_mutation};
use crate::protocol::tree::{parse_value, parse_vec3, Error, PropValue, Result};

pub fn cmd_move(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "delta", "position", "snap"], &["relative"])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    let snap: f32 = parsed
        .opt("snap")
        .unwrap_or("0")
        .parse()
        .map_err(|_| usage_err("--snap must be a number of studs (0 disables snapping)"))?;
    if snap < 0.0 {
        return Err(usage_err("--snap must be >= 0"));
    }
    let snap3 = |v: [f32; 3]| -> [f32; 3] {
        if snap > 0.0 {
            v.map(|c| (c / snap).round() * snap)
        } else {
            v
        }
    };
    if parsed.opt("delta").is_none() && parsed.opt("position").is_none() {
        return Err(usage_err("move requires --delta 'x,y,z' (or --position for an absolute BasePart position)"));
    }
    // Absolute positioning is only well-defined for a single BasePart.
    if let Some(position) = parsed.opt("position") {
        let wanted = snap3(parse_vec3(position)?);
        let (mut engine, _format) = load_session(global, &file)?;
        let instance = engine.get(&target)?;
        let current = match instance.properties.get("CFrame") {
            Some(PropValue::CFrame(cf)) => cf.position,
            _ => {
                engine.shutdown();
                return Err(Error(format!(
                    "'{target}' has no CFrame; --position only applies to BaseParts"
                )));
            }
        };
        let delta = [
            wanted[0] - current[0],
            wanted[1] - current[1],
            wanted[2] - current[2],
        ];
        let result = engine.move_by(&target, delta, false);
        let tree = match result {
            Ok(_) => engine.snapshot()?,
            Err(e) => {
                engine.shutdown();
                return Err(e);
            }
        };
        engine.shutdown();
        write_mutation(&file, &tree)?;
        emit(
            global,
            format!("moved {target} to ({}, {}, {}) in {}", wanted[0], wanted[1], wanted[2], file.display()),
            json!({
                "ok": true, "command": "move", "path": file.display().to_string(),
                "target": target, "position": wanted,
            }),
        );
        return Ok(());
    }
    let delta = snap3(parse_vec3(parsed.opt("delta").unwrap())?);
    let relative = parsed.has("relative");
    let (_result, _tree) = mutate(global, &file, |engine| {
        engine.move_by(&target, delta, relative)
    })?;
    emit(
        global,
        format!("moved {target} by ({}, {}, {}){} in {}", delta[0], delta[1], delta[2], if relative { " (local space)" } else { "" }, file.display()),
        json!({
            "ok": true, "command": "move", "path": file.display().to_string(),
            "target": target, "delta": delta, "relative": relative,
        }),
    );
    Ok(())
}

pub fn cmd_rotate(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "angles", "snap-angle"], &["world"])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    let angles_raw = parsed.required("angles")?;
    let mut angles = parse_vec3(angles_raw)?;
    let snap_angle: f32 = parsed
        .opt("snap-angle")
        .unwrap_or("0")
        .parse()
        .map_err(|_| usage_err("--snap-angle must be a number of degrees (0 disables)"))?;
    if snap_angle > 0.0 {
        angles = angles.map(|a| (a / snap_angle).round() * snap_angle);
    }
    let world = parsed.has("world");
    let (_result, _tree) = mutate(global, &file, |engine| {
        engine.rotate(&target, angles, world)
    })?;
    emit(
        global,
        format!("rotated {target} by ({}, {}, {}) degrees{} in {}", angles[0], angles[1], angles[2], if world { " (world space)" } else { "" }, file.display()),
        json!({
            "ok": true, "command": "rotate", "path": file.display().to_string(),
            "target": target, "angles": angles, "worldSpace": world,
        }),
    );
    Ok(())
}

pub fn cmd_scale(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "factor", "factors", "size"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    if let Some(size_raw) = parsed.opt("size") {
        // Absolute BasePart Size: expressed through the property system so
        // type checking and history behave exactly like `set`.
        let size = parse_value("Vector3", size_raw, "Size")?;
        let (mut engine, _format) = load_session(global, &file)?;
        let result = engine.set(&target, "Size", &size);
        let tree = match result {
            Ok(_) => engine.snapshot()?,
            Err(e) => {
                engine.shutdown();
                return Err(e);
            }
        };
        engine.shutdown();
        write_mutation(&file, &tree)?;
        emit(
            global,
            format!("scaled {target} to Size {} in {}", size.display(), file.display()),
            json!({
                "ok": true, "command": "scale", "path": file.display().to_string(),
                "target": target, "size": size,
            }),
        );
        return Ok(());
    }
    if let Some(factor_raw) = parsed.opt("factor") {
        let factor: f32 = factor_raw
            .parse()
            .map_err(|_| usage_err("--factor must be a number"))?;
        let (_r, _t) = mutate(global, &file, |engine| engine.scale_factor(&target, factor))?;
        emit(
            global,
            format!("scaled {target} by factor {factor} in {}", file.display()),
            json!({
                "ok": true, "command": "scale", "path": file.display().to_string(),
                "target": target, "factor": factor,
            }),
        );
        return Ok(());
    }
    if let Some(factors_raw) = parsed.opt("factors") {
        let factors = parse_vec3(factors_raw)?;
        let (_r, _t) = mutate(global, &file, |engine| engine.scale_factors(&target, factors))?;
        emit(
            global,
            format!("scaled {target} by factors ({}, {}, {}) in {}", factors[0], factors[1], factors[2], file.display()),
            json!({
                "ok": true, "command": "scale", "path": file.display().to_string(),
                "target": target, "factors": factors,
            }),
        );
        return Ok(());
    }
    Err(usage_err("scale requires --factor F, --factors 'x,y,z', or --size 'x,y,z'"))
}

pub fn cmd_pivot(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "position"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    if let Some(position_raw) = parsed.opt("position").map(|s| s.to_string()) {
        let pos = parse_vec3(&position_raw)?;
        let value = PropValue::CFrame(crate::protocol::tree::CFrameValue::identity_at(pos[0], pos[1], pos[2]));
        let (result, _tree) = mutate(global, &file, |engine| {
            engine.pivot(&target, Some(&value))
        })?;
        emit(
            global,
            format!("set pivot of {target} to ({}, {}, {}) in {}", pos[0], pos[1], pos[2], file.display()),
            json!({
                "ok": true, "command": "pivot", "path": file.display().to_string(),
                "target": target, "pivot": result.get("pivot").cloned().unwrap_or(json!(null)),
            }),
        );
    } else {
        let (mut engine, _format) = load_session(global, &file)?;
        let result = engine.pivot(&target, None)?;
        engine.shutdown();
        let pivot = result.get("pivot").cloned().unwrap_or(json!(null));
        emit(
            global,
            format!("pivot of {target}: {}", serde_json::to_string(&pivot).unwrap_or_default()),
            json!({
                "ok": true, "command": "pivot", "path": file.display().to_string(),
                "target": target, "pivot": pivot,
            }),
        );
    }
    Ok(())
}
