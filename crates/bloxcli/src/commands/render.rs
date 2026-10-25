//! Render and UI commands: render/camera/ui.*.

use std::path::Path;
use serde_json::{json, Value as JsonValue};
use crate::cli::args::{parse_cmd, usage_err, CmdArgs, Global};
use crate::app::session::{emit, load_session};
use crate::protocol::tree::{parse_vec3, Error, Result};

pub fn parse_probe(text: &str) -> Result<Vec<(i64, i64)>> {
    let mut out = Vec::new();
    for part in text.split(';') {
        let part = part.trim();
        if part.is_empty() { continue; }
        let (x, y) = part.split_once(',').ok_or_else(|| usage_err("--probe expects 'x,y;x,y'"))?;
        out.push((
            x.trim().parse().map_err(|_| usage_err("probe x must be an integer"))?,
            y.trim().parse().map_err(|_| usage_err("probe y must be an integer"))?,
        ));
    }
    Ok(out)
}

pub fn cmd_render(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(
        args,
        &["output", "width", "height", "camera", "look-at", "up", "fov", "preset",
          "quality", "assets", "exposure", "probe", "shadow-size"],
        &["ortho", "no-shadows"],
    )?;
    let file = parsed.file()?;
    let output = parsed.opt("output").map(|s| s.to_string())
        .ok_or_else(|| usage_err("render requires --output <file.png>"))?;
    let width: u32 = parsed.opt("width").unwrap_or("800").parse()
        .map_err(|_| usage_err("--width must be an integer"))?;
    let height: u32 = parsed.opt("height").unwrap_or("600").parse()
        .map_err(|_| usage_err("--height must be an integer"))?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(usage_err("--width/--height must be within 1..=8192"));
    }
    let place_dir = file.parent().map(|p| p.to_string_lossy().to_string())
        .filter(|s| !s.is_empty()).unwrap_or_else(|| ".".to_string());
    let assets = parsed.opt("assets").map(|s| s.to_string())
        .unwrap_or_else(|| place_dir.clone());

    let (mut engine, _format) = load_session(global, &file)?;
    let mut params = json!({
        "width": width, "height": height, "outputPath": output,
        "assetsDir": assets, "placeDir": place_dir,
        "shadows": !parsed.has("no-shadows"),
    });
    if let Some(q) = parsed.opt("quality") {
        if !["low", "medium", "high"].contains(&q) {
            engine.shutdown();
            return Err(usage_err("--quality must be low, medium, or high"));
        }
        params["quality"] = json!(q);
    }
    if let Some(preset) = parsed.opt("preset") {
        if !["default", "isometric", "front", "top", "side", "current"].contains(&preset) {
            engine.shutdown();
            return Err(usage_err("--preset must be default, isometric, front, top, side, or current"));
        }
        params["preset"] = json!(preset);
    }
    let mut camera = json!({});
    let mut has_camera = false;
    if let Some(c) = parsed.opt("camera") {
        camera["position"] = json!(parse_vec3(c)?);
        has_camera = true;
    }
    if let Some(c) = parsed.opt("look-at") {
        camera["lookAt"] = json!(parse_vec3(c)?);
        has_camera = true;
    }
    if let Some(c) = parsed.opt("up") {
        camera["up"] = json!(parse_vec3(c)?);
        has_camera = true;
    }
    if let Some(f) = parsed.opt("fov") {
        camera["fovDegrees"] = json!(f.parse::<f32>().map_err(|_| usage_err("--fov must be a number"))?);
        has_camera = true;
    }
    if parsed.has("ortho") {
        camera["orthographic"] = json!(true);
        has_camera = true;
    }
    if has_camera {
        params["camera"] = camera;
    }
    if let Some(e) = parsed.opt("exposure") {
        params["exposure"] = json!(e.parse::<f32>().map_err(|_| usage_err("--exposure must be a number"))?);
    }
    if let Some(probe) = parsed.opt("probe") {
        let probes = parse_probe(probe)?;
        params["probePixels"] = json!(probes.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>());
    }
    let result = engine.render_screenshot(params)?;
    engine.shutdown();

    // The engine claims to have written a PNG: verify before reporting success.
    let bytes = std::fs::read(&output)
        .map_err(|e| Error(format!("engine did not produce '{}': {e}", output)))?;
    if bytes.len() < 33 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(Error(format!("'{output}' is not a valid PNG (bad signature, {} bytes)", bytes.len())));
    }
    let png_w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let png_h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if png_w != width || png_h != height {
        return Err(Error(format!("PNG dimensions {png_w}x{png_h} do not match requested {width}x{height}")));
    }
    let human = format!(
        "rendered {} ({}x{}) in {:.1} ms on {} — {} draw calls, {} triangles, {} parts{}",
        output, width, height,
        result.get("renderMs").and_then(|v| v.as_f64()).unwrap_or(0.0),
        result.get("device").and_then(|v| v.as_str()).unwrap_or("unknown device"),
        result.get("drawCalls").and_then(|v| v.as_i64()).unwrap_or(0),
        result.get("triangles").and_then(|v| v.as_i64()).unwrap_or(0),
        result.get("parts").and_then(|v| v.as_i64()).unwrap_or(0),
        {
            let missing = result.get("missingAssets").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            if missing > 0 { format!(", WARNING: {missing} missing asset(s) reported in JSON") } else { String::new() }
        }
    );
    emit(global, human, json!({
        "ok": true, "command": "render", "path": file.display().to_string(),
        "output": output, "width": width, "height": height,
        "device": result.get("device").cloned().unwrap_or(json!(null)),
        "drawCalls": result.get("drawCalls").cloned().unwrap_or(json!(0)),
        "triangles": result.get("triangles").cloned().unwrap_or(json!(0)),
        "renderMs": result.get("renderMs").cloned().unwrap_or(json!(0)),
        "parts": result.get("parts").cloned().unwrap_or(json!(0)),
        "camera": result.get("camera").cloned().unwrap_or(json!(null)),
        "skipped": result.get("skipped").cloned().unwrap_or(json!([])),
        "missingAssets": result.get("missingAssets").cloned().unwrap_or(json!([])),
        "warnings": result.get("warnings").cloned().unwrap_or(json!([])),
        "probes": result.get("probes").cloned().unwrap_or(json!([])),
    }));
    Ok(())
}

pub fn cmd_camera(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["assets"], &[])?;
    let file = parsed.file()?;
    let place_dir = file.parent().map(|p| p.to_string_lossy().to_string())
        .filter(|s| !s.is_empty()).unwrap_or_else(|| ".".to_string());
    let assets = parsed.opt("assets").map(|s| s.to_string()).unwrap_or_else(|| place_dir.clone());
    let (mut engine, _format) = load_session(global, &file)?;
    let result = engine.render_camera(Some(&assets), Some(&place_dir))?;
    engine.shutdown();
    let human = if result.get("hasCurrentCamera").and_then(|v| v.as_bool()) == Some(true) {
        format!(
            "current camera '{}' at {:?}; {} parts, scene bounds {:?} .. {:?}",
            result.get("currentCameraName").and_then(|v| v.as_str()).unwrap_or("CurrentCamera"),
            result.pointer("/currentCamera/position").cloned().unwrap_or(json!(null)),
            result.get("partCount").and_then(|v| v.as_i64()).unwrap_or(0),
            result.pointer("/sceneBounds/min").cloned().unwrap_or(json!(null)),
            result.pointer("/sceneBounds/max").cloned().unwrap_or(json!(null)),
        )
    } else {
        format!(
            "no CurrentCamera in Workspace; suggested framing camera at {:?} ({} parts)",
            result.pointer("/suggestedCamera/position").cloned().unwrap_or(json!(null)),
            result.get("partCount").and_then(|v| v.as_i64()).unwrap_or(0),
        )
    };
    emit(global, human, json!({
        "ok": true, "command": "camera", "path": file.display().to_string(),
        "hasCurrentCamera": result.get("hasCurrentCamera").cloned().unwrap_or(json!(false)),
        "currentCameraName": result.get("currentCameraName").cloned().unwrap_or(json!(null)),
        "currentCamera": result.get("currentCamera").cloned().unwrap_or(json!(null)),
        "suggestedCamera": result.get("suggestedCamera").cloned().unwrap_or(json!(null)),
        "sceneBounds": result.get("sceneBounds").cloned().unwrap_or(json!(null)),
        "parts": result.get("partCount").cloned().unwrap_or(json!(0)),
        "draws": result.get("drawCount").cloned().unwrap_or(json!(0)),
    }));
    Ok(())
}

pub fn cmd_ui(global: &Global, args: &[String]) -> Result<()> {
    // Implemented in the Phase 4 block below (kept here so unknown `ui`
    // subcommands fail with usage instead of "unknown command").
    ui_dispatch(global, args)
}

// ---------- Phase 2 commands ----------

pub fn device_size(device: Option<&str>, width: Option<&str>, height: Option<&str>) -> Result<(u32, u32)> {
    if let Some(device) = device {
        return Ok(match device {
            "desktop" => (1920, 1080),
            "laptop" => (1366, 768),
            "tablet" => (1280, 800),
            "mobile" => (390, 844),
            other => return Err(usage_err(&format!("unknown --device '{other}' (desktop, laptop, tablet, mobile)"))),
        });
    }
    let w: u32 = width.unwrap_or("1280").parse().map_err(|_| usage_err("--width must be an integer"))?;
    let h: u32 = height.unwrap_or("720").parse().map_err(|_| usage_err("--height must be an integer"))?;
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err(usage_err("--width/--height must be within 1..=8192"));
    }
    Ok((w, h))
}

pub fn ui_base_params(file: &Path, parsed: &CmdArgs, width: u32, height: u32) -> JsonValue {
    let place_dir = file.parent().map(|p| p.to_string_lossy().to_string())
        .filter(|s| !s.is_empty()).unwrap_or_else(|| ".".to_string());
    let assets = parsed.opt("assets").map(|s| s.to_string()).unwrap_or_else(|| place_dir.clone());
    let mut params = json!({
        "width": width, "height": height,
        "assetsDir": assets, "placeDir": place_dir,
        "ignoreInset": parsed.has("ignore-inset"),
    });
    if let Some(gui) = parsed.opt("gui") {
        params["gui"] = json!(gui);
    }
    if let Some(font) = parsed.opt("font") {
        params["fontPath"] = json!(font);
    }
    params
}

pub fn ui_dispatch(global: &Global, args: &[String]) -> Result<()> {
    let Some(sub) = args.first().cloned() else {
        return Err(usage_err("ui requires a subcommand: render, screenshot, inspect, validate, hit-test"));
    };
    let sub_args = &args[1..];
    match sub.as_str() {
        "render" | "screenshot" => ui_render_cmd(global, sub_args),
        "inspect" => ui_inspect_cmd(global, sub_args),
        "validate" => ui_validate_cmd(global, sub_args),
        "hit-test" => ui_hit_test_cmd(global, sub_args),
        other => Err(usage_err(&format!("unknown ui subcommand '{other}'"))),
    }
}

pub fn ui_render_cmd(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(
        args,
        &["output", "width", "height", "device", "gui", "assets", "camera", "look-at", "probe", "font"],
        &["ignore-inset", "with-scene"],
    )?;
    let file = parsed.file()?;
    let output = parsed.opt("output").map(|s| s.to_string())
        .ok_or_else(|| usage_err("ui render requires --output <file.png>"))?;
    let (width, height) = device_size(parsed.opt("device"), parsed.opt("width"), parsed.opt("height"))?;
    let (mut engine, _format) = load_session(global, &file)?;
    let mut params = ui_base_params(&file, &parsed, width, height);
    params["outputPath"] = json!(output);
    params["withScene"] = json!(parsed.has("with-scene"));
    let mut camera = json!({});
    let mut has_camera = false;
    if let Some(c) = parsed.opt("camera") { camera["position"] = json!(parse_vec3(c)?); has_camera = true; }
    if let Some(c) = parsed.opt("look-at") { camera["lookAt"] = json!(parse_vec3(c)?); has_camera = true; }
    if has_camera { params["camera"] = camera; }
    if let Some(probe) = parsed.opt("probe") {
        let probes = parse_probe(probe)?;
        params["probePixels"] = json!(probes.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>());
    }
    let result = engine.ui_rpc("ui/render", params)?;
    engine.shutdown();

    let bytes = std::fs::read(&output)
        .map_err(|e| Error(format!("engine did not produce '{output}': {e}")))?;
    if bytes.len() < 33 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(Error(format!("'{output}' is not a valid PNG")));
    }
    emit(
        global,
        format!(
            "rendered UI {} ({}x{}) in {:.1} ms on {} — {} quads{}",
            output, width, height,
            result.get("renderMs").and_then(|v| v.as_f64()).unwrap_or(0.0),
            result.get("device").and_then(|v| v.as_str()).unwrap_or("unknown"),
            result.get("quadCount").and_then(|v| v.as_i64()).unwrap_or(0),
            {
                let w = result.get("warnings").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                if w > 0 { format!(", {w} warning(s) (see JSON)") } else { String::new() }
            }
        ),
        json!({
            "ok": true, "command": "ui render", "path": file.display().to_string(),
            "output": output, "width": width, "height": height,
            "device": result.get("device").cloned().unwrap_or(json!(null)),
            "drawCalls": result.get("drawCalls").cloned().unwrap_or(json!(0)),
            "quadCount": result.get("quadCount").cloned().unwrap_or(json!(0)),
            "rectCount": result.get("rectCount").cloned().unwrap_or(json!(0)),
            "renderMs": result.get("renderMs").cloned().unwrap_or(json!(0)),
            "warnings": result.get("warnings").cloned().unwrap_or(json!([])),
            "probes": result.get("probes").cloned().unwrap_or(json!([])),
        }),
    );
    Ok(())
}

pub fn ui_inspect_cmd(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["width", "height", "device", "gui", "assets"], &["ignore-inset"])?;
    let file = parsed.file()?;
    // The target GUI (e.g. a ScreenGui path) may be the positional arg after the file.
    let (width, height) = device_size(parsed.opt("device"), parsed.opt("width"), parsed.opt("height"))?;
    let (mut engine, _format) = load_session(global, &file)?;
    let params = ui_base_params(&file, &parsed, width, height);
    let result = engine.ui_rpc("ui/layout", params)?;
    engine.shutdown();
    let rects = result.get("rects").cloned().unwrap_or(json!([]));
    let human = {
        let mut lines = vec![format!("UI layout for {} at {}x{}:", file.display(), width, height)];
        if let Some(arr) = rects.as_array() {
            for rect in arr {
                let visible = rect.get("visible").and_then(|v| v.as_bool()).unwrap_or(false);
                lines.push(format!(
                    "  {} [{}] @ ({:.0},{:.0}) {:.0}x{:.0} z={:.0} order={}{}",
                    rect.get("path").and_then(|v| v.as_str()).unwrap_or("?"),
                    rect.get("className").and_then(|v| v.as_str()).unwrap_or("?"),
                    rect.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    rect.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    rect.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    rect.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    rect.get("zindex").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    rect.get("order").and_then(|v| v.as_i64()).unwrap_or(0),
                    if visible { "" } else { " (hidden)" },
                ));
            }
        }
        lines.push(format!("{} warning(s)", result.get("warnings").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)));
        lines.join("\n")
    };
    emit(global, human, json!({
        "ok": true, "command": "ui inspect", "path": file.display().to_string(),
        "width": width, "height": height, "rects": rects,
        "warnings": result.get("warnings").cloned().unwrap_or(json!([])),
    }));
    Ok(())
}

pub fn ui_validate_cmd(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["width", "height", "device", "gui", "assets"], &["ignore-inset"])?;
    let file = parsed.file()?;
    let (width, height) = device_size(parsed.opt("device"), parsed.opt("width"), parsed.opt("height"))?;
    let (mut engine, _format) = load_session(global, &file)?;
    let params = ui_base_params(&file, &parsed, width, height);
    let result = engine.ui_rpc("ui/validate", params)?;
    engine.shutdown();
    let valid = result.get("valid").and_then(|v| v.as_bool()).unwrap_or(false);
    let issues = result.get("issues").cloned().unwrap_or(json!([]));
    let human = if valid {
        format!("UI VALID {} at {}x{} ({} warning(s))", file.display(), width, height,
                issues.as_array().map(|a| a.len()).unwrap_or(0))
    } else {
        format!("UI INVALID {} at {}x{}: {}", file.display(), width, height,
                serde_json::to_string_pretty(&issues).unwrap_or_default())
    };
    emit(global, human, json!({
        "ok": valid, "command": "ui validate", "path": file.display().to_string(),
        "width": width, "height": height, "valid": valid, "issues": issues,
        "rects": result.get("rects").cloned().unwrap_or(json!([])),
    }));
    if valid { Ok(()) } else { Err(Error("UI validation failed".to_string())) }
}

pub fn ui_hit_test_cmd(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["x", "y", "width", "height", "device", "gui", "assets"], &["ignore-inset"])?;
    let file = parsed.file()?;
    let x: f64 = parsed.required("x")?.parse().map_err(|_| usage_err("--x must be a number"))?;
    let y: f64 = parsed.required("y")?.parse().map_err(|_| usage_err("--y must be a number"))?;
    let (width, height) = device_size(parsed.opt("device"), parsed.opt("width"), parsed.opt("height"))?;
    let (mut engine, _format) = load_session(global, &file)?;
    let params = ui_base_params(&file, &parsed, width, height);
    let result = engine.ui_rpc("ui/layout", params)?;
    engine.shutdown();
    let rects = result.get("rects").cloned().unwrap_or(json!([]));
    // Topmost = greatest render order among visible rects containing the point.
    let mut best: Option<JsonValue> = None;
    for rect in rects.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        if rect.get("visible").and_then(|v| v.as_bool()) != Some(true) { continue; }
        // Containers that only exist to hold GuiObjects are not hit targets.
        if matches!(rect.get("className").and_then(|v| v.as_str()), Some("ScreenGui") | Some("BillboardGui") | Some("SurfaceGui")) { continue; }
        let rx = rect.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let ry = rect.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let rw = rect.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let rh = rect.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
            let order = rect.get("order").and_then(|v| v.as_i64()).unwrap_or(0);
            let best_order = best.as_ref().and_then(|b| b.get("order")).and_then(|v| v.as_i64()).unwrap_or(-1);
            if order >= best_order { best = Some(rect.clone()); }
        }
    }
    match best {
        Some(hit) => {
            let class = hit.get("className").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let would_fire: Vec<&str> = match class.as_str() {
                "TextButton" | "ImageButton" => vec!["MouseButton1Click", "MouseButton1Down", "Activated"],
                "TextBox" => vec!["Focused", "MouseButton1Down"],
                _ => vec!["MouseButton1Down"],
            };
            emit(
                global,
                format!("hit {} [{}] at ({x}, {y}) — simulated events {:?} (no scripts execute yet)",
                        hit.get("path").and_then(|v| v.as_str()).unwrap_or("?"), class, would_fire),
                json!({
                    "ok": true, "command": "ui hit-test", "path": file.display().to_string(),
                    "x": x, "y": y, "hit": true, "instance": hit,
                    "simulated": true,
                    "note": "hit-test/event simulation only; Luau handlers do not execute (runtime arrives in a later phase)",
                    "wouldFire": would_fire,
                }),
            );
            Ok(())
        }
        None => {
            emit(
                global,
                format!("no hit at ({x}, {y})"),
                json!({
                    "ok": true, "command": "ui hit-test", "path": file.display().to_string(),
                    "x": x, "y": y, "hit": false, "instance": null, "simulated": true,
                }),
            );
            Ok(())
        }
    }
}
