//! Session commands: play/net/debug (client + daemon entrypoints) and mcp.

use std::path::PathBuf;
use serde_json::json;
use crate::cli::args::{parse_cmd, usage_err, CmdArgs, Global};
use crate::app::session::emit;
use crate::protocol::engine::find_engine;
use crate::protocol::tree::{Error, Result};

pub fn cmd_play_daemon(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["rig"], &[])?;
    let file = parsed.file()?;
    let rig = parsed.opt("rig").unwrap_or("R6").to_string();
    let engine_path = find_engine(global.engine.as_deref())?;
    crate::session::play::run_daemon(&engine_path, &file, &rig)
}

/// Parses `--probes "x,y;x,y"` into [[x, y], ...] for screenshot
/// pixel probes answered from the same rendered frame.
pub(crate) fn parse_probes(raw: Option<&str>) -> Result<serde_json::Value> {
    let mut out = Vec::new();
    if let Some(raw) = raw {
        for pair in raw.split(';').filter(|p| !p.trim().is_empty()) {
            let mut it = pair.split(',');
            let x: i64 = it.next().and_then(|v| v.trim().parse().ok())
                .ok_or_else(|| usage_err("--probes expects 'x,y;x,y'"))?;
            let y: i64 = it.next().and_then(|v| v.trim().parse().ok())
                .ok_or_else(|| usage_err("--probes expects 'x,y;x,y'"))?;
            out.push(json!([x, y]));
        }
    }
    Ok(json!(out))
}

pub fn cmd_play(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("play needs a subcommand: start|step|input|state|screenshot|logs|stop"));
    };
    match sub.as_str() {
        "start" => {
            let parsed = parse_cmd(rest, &["rig"], &["force"])?;
            let file = parsed.file()?;
            let rig = parsed.opt("rig").unwrap_or("R6");
            if !["R6", "R15"].contains(&rig) {
                return Err(usage_err("--rig must be R6 or R15"));
            }
            let engine_path = find_engine(global.engine.as_deref())?;
            let state = crate::session::play::start_daemon(&engine_path, &file, rig, parsed.has("force"))?;
            emit(global,
                 format!("play session started for {} (rig {rig}); character at {:?}",
                         file.display(),
                         state.pointer("/character/position").cloned().unwrap_or(json!(null))),
                 json!({"ok": true, "command": "play start", "path": file.display().to_string(), "state": state}));
            Ok(())
        }
        "step" => {
            let parsed = parse_cmd(rest, &["steps", "duration"], &[])?;
            let file = parsed.file()?;
            let steps: i64 = if let Some(d) = parsed.opt("duration") {
                (d.parse::<f64>().map_err(|_| usage_err("--duration must be seconds"))? * 60.0).round() as i64
            } else {
                parsed.opt("steps").unwrap_or("1").parse().map_err(|_| usage_err("--steps must be an integer"))?
            };
            let state = crate::session::play::send_request(&file, &json!({"cmd": "step", "steps": steps}))?;
            emit(global,
                 format!("stepped {} — character at {:?}, state {}",
                         file.display(),
                         state.pointer("/character/position").cloned().unwrap_or(json!(null)),
                         state.pointer("/character/state").and_then(|v| v.as_str()).unwrap_or("?")),
                 json!({"ok": true, "command": "play step", "state": state}));
            Ok(())
        }
        "input" => {
            let parsed = parse_cmd(rest, &["keys", "duration", "yaw", "pitch", "click", "touch", "hover", "wheel", "type-text", "width", "height"], &["first-person", "third-person", "submit"])?;
            let file = parsed.file()?;
            let keys: Vec<&str> = parsed.opt("keys").unwrap_or("").split(',')
                .map(|k| k.trim()).filter(|k| !k.is_empty()).collect();
            let duration: f64 = parsed.opt("duration").unwrap_or("0.5").parse()
                .map_err(|_| usage_err("--duration must be seconds"))?;
            let mut request = json!({"cmd": "input", "keys": keys, "duration": duration});
            if let Some(yaw) = parsed.opt("yaw") {
                request["yawDeg"] = json!(yaw.parse::<f64>().map_err(|_| usage_err("--yaw must be degrees"))?);
            }
            if let Some(pitch) = parsed.opt("pitch") {
                request["pitchDeg"] = json!(pitch.parse::<f64>().map_err(|_| usage_err("--pitch must be degrees"))?);
            }
            if parsed.has("first-person") { request["firstPerson"] = json!(true); }
            if parsed.has("third-person") { request["firstPerson"] = json!(false); }
            // GUI pointer/text input (Phase 22): resolved against the
            // runtime GUI layout and fires real Luau GUI signals.
            let viewport_w: i64 = parsed.opt("width").unwrap_or("1280").parse()
                .map_err(|_| usage_err("--width must be an integer"))?;
            let viewport_h: i64 = parsed.opt("height").unwrap_or("720").parse()
                .map_err(|_| usage_err("--height must be an integer"))?;
            let parse_xy = |raw: &str, flag: &str| -> Result<(f64, f64)> {
                let (x, y) = raw.split_once(',')
                    .ok_or_else(|| usage_err(&format!("{flag} expects 'x,y' in viewport pixels")))?;
                Ok((
                    x.trim().parse().map_err(|_| usage_err(&format!("{flag} x must be a number")))?,
                    y.trim().parse().map_err(|_| usage_err(&format!("{flag} y must be a number")))?,
                ))
            };
            if let Some(click) = parsed.opt("click") {
                let (x, y) = parse_xy(click, "--click")?;
                let state = crate::session::play::send_request(&file, &json!({
                    "cmd": "gui", "kind": "click", "x": x, "y": y,
                    "width": viewport_w, "height": viewport_h,
                }))?;
                emit(global,
                     format!("click ({x}, {y}) -> {} {}", state["hit"]["path"], state["hit"]["event"]),
                     json!({"ok": true, "command": "play input", "hit": state["hit"], "state": state}));
                return Ok(());
            }
            if let Some(touch) = parsed.opt("touch") {
                let (x, y) = parse_xy(touch, "--touch")?;
                let state = crate::session::play::send_request(&file, &json!({
                    "cmd": "gui", "kind": "touch", "x": x, "y": y,
                    "width": viewport_w, "height": viewport_h,
                }))?;
                emit(global,
                     format!("touch ({x}, {y}) -> {} {}", state["hit"]["path"], state["hit"]["event"]),
                     json!({"ok": true, "command": "play input", "hit": state["hit"], "state": state}));
                return Ok(());
            }
            if let Some(hover) = parsed.opt("hover") {
                let (x, y) = parse_xy(hover, "--hover")?;
                let state = crate::session::play::send_request(&file, &json!({
                    "cmd": "gui", "kind": "hover", "x": x, "y": y,
                    "width": viewport_w, "height": viewport_h,
                }))?;
                emit(global,
                     format!("hover ({x}, {y}) -> {} {}", state["hit"]["path"], state["hit"]["event"]),
                     json!({"ok": true, "command": "play input", "hit": state["hit"], "state": state}));
                return Ok(());
            }
            if let Some(wheel) = parsed.opt("wheel") {
                let (x, y) = parse_xy(wheel, "--wheel")?;
                let state = crate::session::play::send_request(&file, &json!({
                    "cmd": "gui", "kind": "wheel", "x": x, "y": y, "delta": 1,
                    "width": viewport_w, "height": viewport_h,
                }))?;
                emit(global,
                     format!("wheel ({x}, {y}) -> {} {}", state["hit"]["path"], state["hit"]["event"]),
                     json!({"ok": true, "command": "play input", "hit": state["hit"], "state": state}));
                return Ok(());
            }
            if let Some(text) = parsed.opt("type-text") {
                let state = crate::session::play::send_request(&file, &json!({
                    "cmd": "gui", "kind": "typeText", "text": text,
                    "submit": parsed.has("submit"),
                }))?;
                emit(global,
                     format!("typed text into focused TextBox (submitted: {})", parsed.has("submit")),
                     json!({"ok": true, "command": "play input", "typed": state["typed"], "state": state}));
                return Ok(());
            }
            let state = crate::session::play::send_request(&file, &request)?;
            emit(global,
                 format!("input {:?} for {duration}s — character at {:?}, {}",
                         keys,
                         state.pointer("/character/position").cloned().unwrap_or(json!(null)),
                         state.pointer("/character/state").and_then(|v| v.as_str()).unwrap_or("?")),
                 json!({"ok": true, "command": "play input", "state": state}));
            Ok(())
        }
        "state" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let state = crate::session::play::send_request(&file, &json!({"cmd": "state"}))?;
            emit(global,
                 format!("playing {} — t={}s, character {:?} velocity {:?} health {} state {}",
                         file.display(),
                         state.get("time").and_then(|v| v.as_f64()).unwrap_or(0.0),
                         state.pointer("/character/position").cloned().unwrap_or(json!(null)),
                         state.pointer("/character/velocity").cloned().unwrap_or(json!(null)),
                         state.pointer("/character/health").and_then(|v| v.as_f64()).unwrap_or(0.0),
                         state.pointer("/character/state").and_then(|v| v.as_str()).unwrap_or("?")),
                 json!({"ok": true, "command": "play state", "state": state}));
            Ok(())
        }
        "screenshot" => {
            let parsed = parse_cmd(rest, &["output", "width", "height", "probes"], &[])?;
            let file = parsed.file()?;
            let output = parsed.opt("output").ok_or_else(|| usage_err("play screenshot requires --output <file.png>"))?;
            let width: i64 = parsed.opt("width").unwrap_or("1280").parse().map_err(|_| usage_err("--width must be an integer"))?;
            let height: i64 = parsed.opt("height").unwrap_or("720").parse().map_err(|_| usage_err("--height must be an integer"))?;
            let result = crate::session::play::send_request(&file, &json!({
                "cmd": "screenshot", "output": output, "width": width, "height": height,
                "probes": parse_probes(parsed.opt("probes"))?}))?;
            if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                return Err(Error(format!("screenshot failed: {}", result.get("error").and_then(|v| v.as_str()).unwrap_or("unknown"))));
            }
            emit(global,
                 format!("play screenshot written to {output}"),
                 json!({"ok": true, "command": "play screenshot", "result": result}));
            Ok(())
        }
        "logs" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let result = crate::session::play::send_request(&file, &json!({"cmd": "logs"}))?;
            let logs = result.get("logs").cloned().unwrap_or(json!([]));
            let human = logs.as_array().map(|arr| arr.iter().map(|entry| format!(
                "[{}] {}: {}",
                entry.get("level").and_then(|v| v.as_str()).unwrap_or("print"),
                entry.get("source").and_then(|v| v.as_str()).unwrap_or(""),
                entry.get("message").and_then(|v| v.as_str()).unwrap_or(""),
            )).collect::<Vec<_>>().join("
")).unwrap_or_default();
            emit(global, human, json!({"ok": true, "command": "play logs", "logs": logs}));
            Ok(())
        }
        "stop" => {
            let parsed = parse_cmd(rest, &[], &["commit"])?;
            let file = parsed.file()?;
            let result = crate::session::play::send_request(&file, &json!({"cmd": "stop", "commit": parsed.has("commit")}))?;
            emit(global,
                 format!("play session stopped for {}{}", file.display(),
                         if parsed.has("commit") { " (runtime state committed to the place file)" } else { "" }),
                 json!({"ok": true, "command": "play stop", "result": result}));
            Ok(())
        }
        other => Err(usage_err(&format!("unknown play subcommand '{other}'"))),
    }
}


pub fn cmd_net_daemon(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["players", "latency", "rig"], &[])?;
    let file = parsed.file()?;
    let players: usize = parsed
        .opt("players")
        .unwrap_or("2")
        .parse()
        .map_err(|_| usage_err("--players must be an integer"))?;
    let latency: f64 = parsed
        .opt("latency")
        .unwrap_or("0")
        .parse()
        .map_err(|_| usage_err("--latency must be milliseconds"))?;
    let rig = parsed.opt("rig").unwrap_or("R6").to_string();
    let engine_path = find_engine(global.engine.as_deref())?;
    crate::session::net::run_daemon(&engine_path, &file, players, latency, &rig)
}

pub fn cmd_net(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err(
            "net needs a subcommand: start|step|input|state|screenshot|logs|stop",
        ));
    };
    match sub.as_str() {
        "start" => {
            let parsed = parse_cmd(rest, &["players", "latency", "rig"], &["force"])?;
            let file = parsed.file()?;
            let players: usize = parsed
                .opt("players")
                .unwrap_or("2")
                .parse()
                .map_err(|_| usage_err("--players must be an integer"))?;
            if players == 0 || players > 8 {
                return Err(usage_err("--players must be between 1 and 8"));
            }
            let latency: f64 = parsed
                .opt("latency")
                .unwrap_or("0")
                .parse()
                .map_err(|_| usage_err("--latency must be milliseconds"))?;
            let rig = parsed.opt("rig").unwrap_or("R6");
            if !["R6", "R15"].contains(&rig) {
                return Err(usage_err("--rig must be R6 or R15"));
            }
            let engine_path = find_engine(global.engine.as_deref())?;
            let state = crate::session::net::start_daemon(
                &engine_path,
                &file,
                players,
                latency,
                rig,
                parsed.has("force"),
            )?;
            emit(
                global,
                format!(
                    "net session started for {} ({players} players, {rig}, latency {latency}ms)",
                    file.display()
                ),
                json!({"ok": true, "command": "net start", "path": file.display().to_string(), "state": state}),
            );
            Ok(())
        }
        "step" => {
            let parsed = parse_cmd(rest, &["steps", "duration"], &[])?;
            let file = parsed.file()?;
            let steps: i64 = if let Some(d) = parsed.opt("duration") {
                (d.parse::<f64>()
                    .map_err(|_| usage_err("--duration must be seconds"))?
                    * 60.0)
                    .round() as i64
            } else {
                parsed
                    .opt("steps")
                    .unwrap_or("1")
                    .parse()
                    .map_err(|_| usage_err("--steps must be an integer"))?
            };
            let state = crate::session::net::send_request(&file, &json!({"cmd": "step", "steps": steps}))?;
            if state.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                return Err(Error(format!(
                    "net step failed: {}",
                    state.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                )));
            }
            emit(
                global,
                format!(
                    "net stepped {} — step {}, characters {:?}",
                    file.display(),
                    state.get("step").cloned().unwrap_or(json!(null)),
                    state.get("characters").cloned().unwrap_or(json!(null))
                ),
                json!({"ok": true, "command": "net step", "state": state}),
            );
            Ok(())
        }
        "input" => {
            let parsed = parse_cmd(rest, &["player", "keys", "duration", "yaw", "click", "touch"], &[])?;
            let file = parsed.file()?;
            let player = parsed.opt("player").unwrap_or("1");
            let keys: Vec<&str> = parsed
                .opt("keys")
                .unwrap_or("")
                .split(',')
                .map(|k| k.trim())
                .filter(|k| !k.is_empty())
                .collect();
            let duration: f64 = parsed
                .opt("duration")
                .unwrap_or("0.5")
                .parse()
                .map_err(|_| usage_err("--duration must be seconds"))?;
            let mut request = json!({"cmd": "input", "player": player, "keys": keys, "duration": duration});
            if let Some(yaw) = parsed.opt("yaw") {
                request["yawDeg"] = json!(yaw
                    .parse::<f64>()
                    .map_err(|_| usage_err("--yaw must be degrees"))?);
            }
            if let Some(click) = parsed.opt("click").or_else(|| parsed.opt("touch")) {
                let kind = if parsed.opt("touch").is_some() { "touch" } else { "click" };
                let (x, y) = {
                    let mut it = click.split(',');
                    (it.next().and_then(|v| v.trim().parse::<f64>().ok()).ok_or_else(|| usage_err("--click expects 'x,y'"))?,
                     it.next().and_then(|v| v.trim().parse::<f64>().ok()).ok_or_else(|| usage_err("--click expects 'x,y'"))?)
                };
                let state = crate::session::net::send_request(&file, &json!({
                    "cmd": "gui", "kind": kind, "player": player, "x": x, "y": y}))?;
                emit(global,
                    format!("net {kind} for player {player} -> {} {}", state["hit"]["path"], state["hit"]["event"]),
                    json!({"ok": true, "command": "net input", "hit": state["hit"], "state": state}));
                return Ok(());
            }
            let state = crate::session::net::send_request(&file, &request)?;
            emit(
                global,
                format!("net input for player {player} — characters {:?}", state.get("characters").cloned().unwrap_or(json!(null))),
                json!({"ok": true, "command": "net input", "state": state}),
            );
            Ok(())
        }
        "state" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let state = crate::session::net::send_request(&file, &json!({"cmd": "state"}))?;
            emit(
                global,
                format!(
                    "net session {} — step {}, {} players, characters {:?}",
                    file.display(),
                    state.get("step").cloned().unwrap_or(json!(null)),
                    state.get("players").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
                    state.get("characters").cloned().unwrap_or(json!(null))
                ),
                json!({"ok": true, "command": "net state", "state": state}),
            );
            Ok(())
        }
        "screenshot" => {
            let parsed = parse_cmd(rest, &["player", "output", "width", "height", "probes"], &[])?;
            let file = parsed.file()?;
            let output = parsed
                .opt("output")
                .ok_or_else(|| usage_err("net screenshot requires --output <file.png>"))?;
            let width: i64 = parsed
                .opt("width")
                .unwrap_or("1280")
                .parse()
                .map_err(|_| usage_err("--width must be an integer"))?;
            let height: i64 = parsed
                .opt("height")
                .unwrap_or("720")
                .parse()
                .map_err(|_| usage_err("--height must be an integer"))?;
            let result = crate::session::net::send_request(
                &file,
                &json!({"cmd": "screenshot", "player": parsed.opt("player").unwrap_or("1"),
                        "output": output, "width": width, "height": height,
                        "probes": parse_probes(parsed.opt("probes"))?}),
            )?;
            if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                return Err(Error(format!(
                    "net screenshot failed: {}",
                    result.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                )));
            }
            emit(
                global,
                format!("net screenshot written to {output}"),
                json!({"ok": true, "command": "net screenshot", "result": result}),
            );
            Ok(())
        }
        "logs" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let result = crate::session::net::send_request(&file, &json!({"cmd": "logs"}))?;
            let logs = result.get("logs").cloned().unwrap_or(json!([]));
            let human = logs
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|entry| {
                            format!(
                                "[{}:{}] {}: {}",
                                entry.get("peer").and_then(|v| v.as_str()).unwrap_or("?"),
                                entry.get("level").and_then(|v| v.as_str()).unwrap_or("print"),
                                entry.get("source").and_then(|v| v.as_str()).unwrap_or(""),
                                entry.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            emit(global, human, json!({"ok": true, "command": "net logs", "logs": logs}));
            Ok(())
        }
        "stop" => {
            let parsed = parse_cmd(rest, &[], &["commit"])?;
            let file = parsed.file()?;
            let result = crate::session::net::send_request(
                &file,
                &json!({"cmd": "stop", "commit": parsed.has("commit")}),
            )?;
            emit(
                global,
                format!(
                    "net session stopped for {}{}",
                    file.display(),
                    if parsed.has("commit") {
                        " (server state committed to the place file)"
                    } else {
                        ""
                    }
                ),
                json!({"ok": true, "command": "net stop", "result": result}),
            );
            Ok(())
        }
        other => Err(usage_err(&format!("unknown net subcommand '{other}'"))),
    }
}


pub fn cmd_mcp(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["root"], &[])?;
    let root = parsed
        .opt("root")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let engine_path = find_engine(global.engine.as_deref())?;
    eprintln!("bloxcli MCP server (root: {})", root.display());
    crate::mcp::run_server(root, engine_path)
}

pub fn cmd_debug_daemon(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &[], &[])?;
    let place = parsed.file()?;
    let engine_path = find_engine(global.engine.as_deref())?;
    crate::session::debug::run_daemon(&engine_path, &place)
}

pub fn place_arg(parsed: &CmdArgs) -> Result<PathBuf> {
    parsed.file()
}

pub fn cmd_debug(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("debug needs a subcommand: start|stop|breakpoint|run|continue|step-over|step-into|step-out|stack|variables"));
    };
    let sub_args: Vec<String> = rest.to_vec();
    match sub.as_str() {
        "start" => {
            let parsed = parse_cmd(&sub_args, &[], &[])?;
            let place = place_arg(&parsed)?;
            let engine_path = find_engine(global.engine.as_deref())?;
            let state = crate::session::debug::start_daemon(&engine_path, &place)?;
            emit(
                global,
                format!("debug session started for {} (add breakpoints, then `debug run`)", place.display()),
                json!({"ok": true, "command": "debug start", "path": place.display().to_string(), "state": state}),
            );
            Ok(())
        }
        "stop" => {
            let parsed = parse_cmd(&sub_args, &[], &[])?;
            let place = place_arg(&parsed)?;
            let response = crate::session::debug::send_request(&place, &json!({"cmd": "stop"}))?;
            emit(
                global,
                format!("debug session stopped for {}", place.display()),
                json!({"ok": true, "command": "debug stop", "path": place.display().to_string(), "response": response}),
            );
            Ok(())
        }
        "breakpoint" => {
            // debug breakpoint add <place> <script-path>:<line> | list <place> | clear <place>
            let Some((action, rest2)) = sub_args.split_first() else {
                return Err(usage_err("debug breakpoint needs add|list|clear"));
            };
            let parsed = parse_cmd(rest2, &[], &[])?;
            let place = place_arg(&parsed)?;
            match action.as_str() {
                "add" => {
                    let spec = parsed.positional.get(1).cloned()
                        .filter(|p| p.contains(':'))
                        .or_else(|| parsed.positional.iter().find(|p| p.contains(':')).cloned())
                        .ok_or_else(|| usage_err("debug breakpoint add needs <script-path>:<line>"))?;
                    let (script, line_str) = spec.rsplit_once(':')
                        .ok_or_else(|| usage_err("breakpoint must be <script-path>:<line>"))?;
                    let line: i64 = line_str.parse()
                        .map_err(|_| usage_err("breakpoint line must be a number"))?;
                    let current = crate::session::debug::send_request(&place, &json!({"cmd": "breakpoints"}))?;
                    let mut list = current["breakpoints"].as_array().cloned().unwrap_or_default();
                    list.push(json!({"script": script, "line": line}));
                    let response = crate::session::debug::send_request(&place, &json!({"cmd": "breakpoints", "breakpoints": list}))?;
                    emit(
                        global,
                        format!("breakpoint set at {script}:{line} ({} total)", response["breakpoints"].as_array().map(|a| a.len()).unwrap_or(0)),
                        json!({"ok": true, "command": "debug breakpoint add", "breakpoints": response["breakpoints"]}),
                    );
                    Ok(())
                }
                "list" => {
                    let response = crate::session::debug::send_request(&place, &json!({"cmd": "breakpoints"}))?;
                    emit(
                        global,
                        format!("{} breakpoint(s)", response["breakpoints"].as_array().map(|a| a.len()).unwrap_or(0)),
                        json!({"ok": true, "command": "debug breakpoint list", "breakpoints": response["breakpoints"]}),
                    );
                    Ok(())
                }
                "clear" => {
                    let response = crate::session::debug::send_request(&place, &json!({"cmd": "breakpoints", "breakpoints": []}))?;
                    emit(
                        global,
                        "breakpoints cleared".to_string(),
                        json!({"ok": true, "command": "debug breakpoint clear", "breakpoints": response["breakpoints"]}),
                    );
                    Ok(())
                }
                other => Err(usage_err(&format!("unknown breakpoint action '{other}' (add|list|clear)"))),
            }
        }
        "run" | "continue" | "step-over" | "step-into" | "step-out" => {
            let parsed = parse_cmd(&sub_args, &[], &[])?;
            let place = place_arg(&parsed)?;
            let request = if sub == "run" {
                json!({"cmd": "run"})
            } else {
                let mode = match sub.as_str() {
                    "continue" => "continue",
                    "step-over" => "stepOver",
                    "step-into" => "stepInto",
                    _ => "stepOut",
                };
                json!({"cmd": "resume", "mode": mode})
            };
            let response = crate::session::debug::send_request(&place, &request)?;
            if response["ok"] != json!(true) {
                return Err(Error(format!("debug {sub} failed: {}", response["error"])));
            }
            let dbg = &response["state"]["debug"];
            let summary = if dbg["paused"] == json!(true) {
                format!("paused at {}:{} ({})", dbg["script"], dbg["line"], dbg["reason"])
            } else if dbg["finished"] == json!(true) {
                "execution finished".to_string()
            } else {
                "running (no pause hit)".to_string()
            };
            emit(
                global,
                format!("debug {sub}: {summary}"),
                json!({"ok": true, "command": format!("debug {sub}"), "path": place.display().to_string(), "state": response["state"]}),
            );
            Ok(())
        }
        "stack" | "variables" => {
            let parsed = parse_cmd(&sub_args, &["frame"], &[])?;
            let place = place_arg(&parsed)?;
            let frame: i64 = parsed.opt("frame").unwrap_or("0").parse()
                .map_err(|_| usage_err("--frame must be a number"))?;
            let response = crate::session::debug::send_request(&place, &json!({"cmd": "state", "frame": frame}))?;
            let dbg = &response["state"]["debug"];
            let items = if sub == "stack" { &dbg["stack"] } else { &dbg["variables"] };
            emit(
                global,
                format!("{} {}(s) at {}:{}", items.as_array().map(|a| a.len()).unwrap_or(0), sub, dbg["script"], dbg["line"]),
                json!({"ok": true, "command": format!("debug {sub}"), "path": place.display().to_string(), "debug": dbg}),
            );
            Ok(())
        }
        other => Err(usage_err(&format!("unknown debug subcommand '{other}'"))),
    }
}
