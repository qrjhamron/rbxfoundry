//! BloxCLI: headless, CLI-only Roblox place editor for AI agents.
//! Phase 1 commands: new, inspect, add, set, save, export, validate.
//! Every mutation runs in the C++ engine (the runtime source of truth);
//! this binary handles CLI parsing and genuine .rbxl/.rbxlx file I/O.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Value as JsonValue};

use bloxcli::convert::validate_reflection;
use bloxcli::engine::{find_engine, EngineClient};
use bloxcli::history::History;
use bloxcli::integrity::{blocked_message, check_export_integrity, errors as integrity_errors, warnings as integrity_warnings};
use bloxcli::integrity::ref_index_path;
use bloxcli::rbxio::{format_for_path, read_model, read_place, write_model, write_place, PlaceFormat};
use bloxcli::tree::{parse_value, parse_vec3, Error, Node, PropValue, Result};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Global {
    json: bool,
    engine: Option<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::from(0),
        Err(err) => {
            eprintln!("error: {}", err.0);
            if err.0.starts_with("usage:") {
                ExitCode::from(2)
            } else {
                ExitCode::from(1)
            }
        }
    }
}

fn usage_err(msg: &str) -> Error {
    Error(format!("usage: {msg} (run 'bloxcli --help')"))
}

fn run(args: &[String]) -> Result<()> {
    let mut global = Global { json: false, engine: None };
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => global.json = true,
            "--engine" => {
                i += 1;
                global.engine = Some(
                    args.get(i)
                        .ok_or_else(|| usage_err("--engine requires a path"))?
                        .clone(),
                );
            }
            "-h" | "--help" if rest.is_empty() => {
                print_help();
                return Ok(());
            }
            "-V" | "--version" if rest.is_empty() => {
                println!("bloxcli {VERSION}");
                return Ok(());
            }
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }
    let Some(command) = rest.first().cloned() else {
        print_help();
        return Err(usage_err("no command given"));
    };
    let cmd_args = &rest[1..];
    match command.as_str() {
        "mcp" => cmd_mcp(&global, cmd_args),
        "run" => cmd_run(&global, cmd_args),
        "play" => cmd_play(&global, cmd_args),
        "play-daemon" => cmd_play_daemon(&global, cmd_args),
        "net" => cmd_net(&global, cmd_args),
        "net-daemon" => cmd_net_daemon(&global, cmd_args),
        "terrain" => cmd_terrain(&global, cmd_args),
        "place" => cmd_place(&global, cmd_args),
        "model" => cmd_model(&global, cmd_args),
        "template" => cmd_template(&global, cmd_args),
        "new" => cmd_new(&global, cmd_args),
        "inspect" | "tree" => cmd_tree(&global, cmd_args),
        "add" => cmd_add(&global, cmd_args),
        "set" => cmd_set(&global, cmd_args),
        "save" => cmd_save(&global, cmd_args),
        "export" => cmd_export(&global, cmd_args),
        "validate" => cmd_validate(&global, cmd_args),
        "get" => cmd_get(&global, cmd_args),
        "find" => cmd_find(&global, cmd_args),
        "clone" => cmd_clone(&global, cmd_args),
        "delete" => cmd_delete(&global, cmd_args),
        "reparent" => cmd_reparent(&global, cmd_args),
        "group" => cmd_group(&global, cmd_args),
        "move" => cmd_move(&global, cmd_args),
        "rotate" => cmd_rotate(&global, cmd_args),
        "scale" => cmd_scale(&global, cmd_args),
        "pivot" => cmd_pivot(&global, cmd_args),
        "undo" => cmd_undo(&global, cmd_args, false),
        "render" => cmd_render(&global, cmd_args),
        "camera" => cmd_camera(&global, cmd_args),
        "ui" => cmd_ui(&global, cmd_args),
        "redo" => cmd_undo(&global, cmd_args, true),
        "help" => {
            print_help();
            Ok(())
        }
        other => Err(usage_err(&format!("unknown command '{other}'"))),
    }
}

fn print_help() {
    println!(
        "bloxcli {VERSION} — headless Roblox place editor for AI agents (Phases 1-4)\n\
         \n\
         USAGE:\n\
         \x20   bloxcli [--json] [--engine <path>] <command> [options]\n\
         \n\
         COMMANDS:\n\
         \x20   new <file.rbxl|.rbxlx> [--name <PlaceName>] [--force]\n\
         \x20       Create a new place with a Workspace.\n\
         \x20   inspect <file> [--path <Workspace/Part>] [--props]\n\
         \x20       Print the instance tree.\n\
         \x20   add <file> --parent <path> --class <ClassName> [--name <Name>]\n\
         \x20       Add an instance (with engine reflection defaults).\n\
         \x20   set <file> --target <path> --property <Name> --value <value> [--type <Type>]\n\
         \x20       Set a typed property. Value grammar by type: Bool true|false;\n\
         \x20       Vector3/Color3 'x,y,z'; CFrame 'x,y,z' or 12 numbers; Enum number\n\
         \x20       or Material name; numbers decimal; String raw text.\n\
         \x20   save <file>\n\
         \x20       Load, validate in the engine, and rewrite the file canonically.\n\
         \x20   export <input> <output> [--force]\n\
         \x20       Convert between .rbxl and .rbxlx (or re-encode the same format).\n\
         \x20       An integrity guard blocks writes with dangling references or\n\
         \x20       corrupted terrain; unresolved assets warn but are preserved.\n\
         \x20   model export <place> --target <path> --output <file.rbxm|.rbxmx> [--force]\n\
         \x20   model inspect <file.rbxm|.rbxmx>\n\
         \x20   model import <file.rbxm|.rbxmx> <place> --parent <path>\n\
         \x20   validate <file>\n\
         \x20       Decode and validate; exit code 1 when invalid.\n\
         \x20   get <file> --target <path> [--property <Name>]\n\
         \x20       Read one instance or one typed property (Position is derived).\n\
         \x20   tree <file> [--path <path>] [--depth <n>] [--props]\n\
         \x20       Print the hierarchy with stable referents (#id).\n\
         \x20   find <file> [--name <Name>] [--class <Class>] [--under <path>] [--contains]\n\
         \x20       Search instances by name and/or class.\n\
         \x20   clone <file> --target <path> [--name <Name>] [--parent <path>]\n\
         \x20   delete <file> --target <path>\n\
         \x20   reparent <file> --target <path> --parent <path>\n\
         \x20   group <file> --targets <a,b,...> [--name <Name>] [--parent <path>]\n\
         \x20       Group instances under a new Model (world transforms preserved).\n\
         \x20   move <file> --target <path> (--delta 'x,y,z' [--relative] | --position 'x,y,z')\n\
         \x20   rotate <file> --target <path> --angles 'rx,ry,rz' [--world]\n\
         \x20       Degrees, intrinsic XYZ. Models rotate about their pivot.\n\
         \x20   scale <file> --target <path> (--factor <f> | --factors 'x,y,z' | --size 'x,y,z')\n\
         \x20       Models scale uniformly about their pivot (Model:ScaleTo semantics).\n\
         \x20   pivot <file> --target <Model> [--position 'x,y,z']\n\
         \x20       Get the effective pivot, or set WorldPivot.\n\
         \x20   undo <file> / redo <file>\n\
         \x20       File-level history in '<file>.bloxhistory/' (100 snapshots).\n\
         \n\
         RENDERING (Phase 3 — real Vulkan, headless):\n\
         \x20   render <file> --output <scene.png> [--width <px>] [--height <px>]\n\
         \x20       [--camera 'x,y,z'] [--look-at 'x,y,z'] [--fov <deg>] [--ortho]\n\
         \x20       [--preset default|isometric|front|top|side|current]\n\
         \x20       [--quality low|medium|high] [--shadow-size <px>] [--no-shadows]\n\
         \x20       [--exposure <f>] [--assets <dir>] [--probe 'x,y;x,y']\n\
         \x20       Renders Workspace geometry to a genuine PNG from the framebuffer.\n\
         \x20       --probe also returns exact frame pixels as JSON for agents.\n\
         \x20   camera <file>\n\
         \x20       Report Workspace.CurrentCamera if present, scene bounds and a\n\
         \x20       suggested framing camera.\n\
         \n\
         UI (Phase 4 — Roblox GUI layout + rasterisation):\n\
         \x20   ui render|screenshot <file> --output <ui.png>\n\
         \x20       [--device desktop|laptop|tablet|mobile] [--width <px>] [--height <px>]\n\
         \x20       [--gui <StarterGui/ScreenGui>] [--camera 'x,y,z'] [--look-at 'x,y,z']\n\
         \x20       [--with-scene] [--probe 'x,y;x,y']\n\
         \x20       Draws actual GUI instances (UDim2/AnchorPoint/layouts/text).\n\
         \x20   ui inspect <file> [--device <d>] [--width <px>] [--height <px>]\n\
         \x20       Computed absolute rects, visibility and render order per element.\n\
         \x20   ui validate <file> [...]  Flag offscreen/clipped/zero-size elements.\n\
         \x20   ui hit-test <file> --x <px> --y <px> [...]\n\
         \x20       Topmost element at a point (hit resolution only; use `run`\n\
         \x20       or `play` to execute Luau handlers).\n\
         \n\
         RUNTIME (Phase 5 — official Luau VM):\n\
         \x20   run <file> [--duration <sec>] [--step <sec>] [--eval <luau>]\n\
         \x20       [--save-runtime <out.rbxl>] [--render <png>] [--ui <png>]\n\
         \x20       [--commit] [--allow-errors]\n\
         \x20       Executes Script/LocalScript/ModuleScript code on a runtime\n\
         \x20       copy of the place (edit state is never touched unless\n\
         \x20       --commit). Script errors carry tracebacks and fail the\n\
         \x20       command unless --allow-errors is given.\n\
         \n\
         PLAYTEST (Phase 6 — Jolt physics + character):\n\
         \x20   play start <file> [--rig R6|R15] [--force]\n\
         \x20   play step <file> [--steps <n>|--duration <sec>]\n\
         \x20   play input <file> --keys W,A,S,D,Space [--duration <sec>]\n\
         \x20       [--yaw <deg>] [--pitch <deg>] [--first-person]\n\
         \x20   play state <file> | play logs <file>\n\
         \x20   play screenshot <file> --output <png>\n\
         \x20   play stop <file> [--commit]\n\
         \x20       Persistent sessions (daemon + <file>.bloxplay/ state). The\n\
         \x20       agent drives the actual physics character; no teleporting.\n\
         \n\
         MCP (Phase 4.5):\n\
         \x20   mcp [--root <dir>]\n\
         \x20       Model Context Protocol server over stdio exposing editor,\n\
         \x20       renderer, UI, runtime and play tools. Filesystem access is\n\
         \x20       confined to the root directory.\n\
         \n\
         GLOBAL OPTIONS:\n\
         \x20   --json            Emit structured JSON for agents.\n\
         \x20   --engine <path>   Path to blox-engine (else $BLOX_ENGINE or build tree).\n\
         \x20   --help, --version\n\
         \n\
         Paths address instances by name, e.g. Workspace/Baseplate. Duplicate\n\
         sibling names make a path ambiguous and are reported as errors."
    );
}

// ---------- argument helpers ----------

struct CmdArgs {
    positional: Vec<String>,
    flags: std::collections::HashMap<String, Option<String>>,
}

fn parse_cmd(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<CmdArgs> {
    let mut out = CmdArgs {
        positional: Vec::new(),
        flags: std::collections::HashMap::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(stripped) = arg.strip_prefix("--") {
            let (key, inline_value) = match stripped.split_once('=') {
                Some((k, v)) => (k.to_string(), Some(v.to_string())),
                None => (stripped.to_string(), None),
            };
            if value_flags.contains(&key.as_str()) {
                let value = match inline_value {
                    Some(v) => Some(v),
                    None => {
                        i += 1;
                        Some(
                            args.get(i)
                                .ok_or_else(|| usage_err(&format!("--{key} requires a value")))?
                                .clone(),
                        )
                    }
                };
                out.flags.insert(key, value);
            } else if bool_flags.contains(&key.as_str()) {
                out.flags.insert(key, None);
            } else {
                return Err(usage_err(&format!("unknown option '--{key}'")));
            }
        } else {
            out.positional.push(arg.clone());
        }
        i += 1;
    }
    Ok(out)
}

impl CmdArgs {
    fn opt(&self, key: &str) -> Option<&str> {
        self.flags.get(key).and_then(|v| v.as_deref())
    }
    fn has(&self, key: &str) -> bool {
        self.flags.contains_key(key)
    }
    fn required(&self, key: &str) -> Result<&str> {
        self.opt(key)
            .ok_or_else(|| usage_err(&format!("missing required --{key}")))
    }
    fn file(&self) -> Result<PathBuf> {
        self.positional
            .first()
            .map(PathBuf::from)
            .ok_or_else(|| usage_err("missing <file> argument"))
    }
}

// ---------- engine sessions ----------

fn spawn_engine(global: &Global) -> Result<EngineClient> {
    let path = find_engine(global.engine.as_deref())?;
    EngineClient::spawn(&path)
}

/// Loads a place file into a fresh engine process. The engine holds the
/// authoritative tree for the rest of the command.
fn load_session(global: &Global, file: &Path) -> Result<(EngineClient, PlaceFormat)> {
    let (node, format) = read_place(file)?;
    let mut engine = spawn_engine(global)?;
    engine.load(&node)?;
    Ok((engine, format))
}

fn emit(global: &Global, human: String, payload: JsonValue) {
    if global.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).expect("json output")
        );
    } else {
        println!("{human}");
    }
}

fn format_name(format: PlaceFormat) -> &'static str {
    match format {
        PlaceFormat::Binary => "rbxl (binary)",
        PlaceFormat::Xml => "rbxlx (XML)",
    }
}

// ---------- commands ----------

fn cmd_play_daemon(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["rig"], &[])?;
    let file = parsed.file()?;
    let rig = parsed.opt("rig").unwrap_or("R6").to_string();
    let engine_path = find_engine(global.engine.as_deref())?;
    bloxcli::play::run_daemon(&engine_path, &file, &rig)
}

fn cmd_play(global: &Global, args: &[String]) -> Result<()> {
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
            let state = bloxcli::play::start_daemon(&engine_path, &file, rig, parsed.has("force"))?;
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
            let state = bloxcli::play::send_request(&file, &json!({"cmd": "step", "steps": steps}))?;
            emit(global,
                 format!("stepped {} — character at {:?}, state {}",
                         file.display(),
                         state.pointer("/character/position").cloned().unwrap_or(json!(null)),
                         state.pointer("/character/state").and_then(|v| v.as_str()).unwrap_or("?")),
                 json!({"ok": true, "command": "play step", "state": state}));
            Ok(())
        }
        "input" => {
            let parsed = parse_cmd(rest, &["keys", "duration", "yaw", "pitch"], &["first-person", "third-person"])?;
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
            let state = bloxcli::play::send_request(&file, &request)?;
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
            let state = bloxcli::play::send_request(&file, &json!({"cmd": "state"}))?;
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
            let parsed = parse_cmd(rest, &["output", "width", "height"], &[])?;
            let file = parsed.file()?;
            let output = parsed.opt("output").ok_or_else(|| usage_err("play screenshot requires --output <file.png>"))?;
            let width: i64 = parsed.opt("width").unwrap_or("1280").parse().map_err(|_| usage_err("--width must be an integer"))?;
            let height: i64 = parsed.opt("height").unwrap_or("720").parse().map_err(|_| usage_err("--height must be an integer"))?;
            let result = bloxcli::play::send_request(&file, &json!({
                "cmd": "screenshot", "output": output, "width": width, "height": height}))?;
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
            let result = bloxcli::play::send_request(&file, &json!({"cmd": "logs"}))?;
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
            let result = bloxcli::play::send_request(&file, &json!({"cmd": "stop", "commit": parsed.has("commit")}))?;
            emit(global,
                 format!("play session stopped for {}{}", file.display(),
                         if parsed.has("commit") { " (runtime state committed to the place file)" } else { "" }),
                 json!({"ok": true, "command": "play stop", "result": result}));
            Ok(())
        }
        other => Err(usage_err(&format!("unknown play subcommand '{other}'"))),
    }
}


fn cmd_net_daemon(global: &Global, args: &[String]) -> Result<()> {
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
    bloxcli::net::run_daemon(&engine_path, &file, players, latency, &rig)
}

fn cmd_net(global: &Global, args: &[String]) -> Result<()> {
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
            let state = bloxcli::net::start_daemon(
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
            let state = bloxcli::net::send_request(&file, &json!({"cmd": "step", "steps": steps}))?;
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
            let parsed = parse_cmd(rest, &["player", "keys", "duration", "yaw"], &[])?;
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
            let state = bloxcli::net::send_request(&file, &request)?;
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
            let state = bloxcli::net::send_request(&file, &json!({"cmd": "state"}))?;
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
            let parsed = parse_cmd(rest, &["player", "output", "width", "height"], &[])?;
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
            let result = bloxcli::net::send_request(
                &file,
                &json!({"cmd": "screenshot", "player": parsed.opt("player").unwrap_or("1"),
                        "output": output, "width": width, "height": height}),
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
            let result = bloxcli::net::send_request(&file, &json!({"cmd": "logs"}))?;
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
            let result = bloxcli::net::send_request(
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


fn terrain_material_value(name: &str) -> Result<i64> {
    match name {
        "Air" => Ok(0),
        "Plastic" => Ok(256),
        "Wood" => Ok(512),
        "Slate" | "Rock" => Ok(800),
        "Concrete" => Ok(816),
        "Brick" => Ok(848),
        "Grass" => Ok(1280),
        "Sand" => Ok(1312),
        "Snow" => Ok(1328),
        "Ice" => Ok(1536),
        "Water" => Ok(2048),
        other => Err(usage_err(&format!(
            "unknown terrain material '{other}' (use Grass, Sand, Slate, Concrete, Brick, Wood, Ice, Snow, Water, Air, or a numeric Enum.Material value)"
        ))),
    }
}

fn parse_xyz(text: &str, what: &str) -> Result<[f64; 3]> {
    let parts: Vec<f64> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| usage_err(&format!("{what} must be 'x,y,z'")))?;
    if parts.len() != 3 {
        return Err(usage_err(&format!("{what} must be 'x,y,z'")));
    }
    Ok([parts[0], parts[1], parts[2]])
}

fn cmd_terrain(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("terrain needs a subcommand: fill|ball|clear|info"));
    };
    match sub.as_str() {
        "fill" | "ball" => {
            let parsed = parse_cmd(rest, &["center", "size", "radius", "material"], &[])?;
            let file = parsed.file()?;
            let center = parse_xyz(parsed.opt("center").unwrap_or("0,0,0"), "--center")?;
            let material_name = parsed.opt("material").unwrap_or("Grass");
            let material: i64 = match material_name.parse::<i64>() {
                Ok(v) => v,
                Err(_) => terrain_material_value(material_name)?,
            };
            let size = parse_xyz(parsed.opt("size").unwrap_or("16,4,16"), "--size")?;
            let radius: f64 = parsed
                .opt("radius")
                .unwrap_or("8")
                .parse()
                .map_err(|_| usage_err("--radius must be a number"))?;
            let (result, _tree) = mutate(global, &file, |engine| {
                if sub == "fill" {
                    engine.call("terrain/fill", json!({"center": center, "size": size, "material": material}))
                } else {
                    engine.call("terrain/fillBall", json!({"center": center, "radius": radius, "material": material}))
                }
            })?;
            emit(
                global,
                format!("terrain {} applied to {} ({} voxels total)", sub, file.display(), result["voxels"]),
                json!({"ok": true, "command": format!("terrain {sub}"), "path": file.display().to_string(), "result": result}),
            );
            Ok(())
        }
        "clear" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let (result, _tree) = mutate(global, &file, |engine| engine.call("terrain/clear", json!({})))?;
            emit(
                global,
                format!("terrain cleared in {}", file.display()),
                json!({"ok": true, "command": "terrain clear", "path": file.display().to_string(), "result": result}),
            );
            Ok(())
        }
        "info" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let (mut engine, _format) = load_session(global, &file)?;
            let result = engine.call("terrain/info", json!({}))?;
            engine.shutdown();
            emit(
                global,
                format!(
                    "{}: {} voxels ({} stud voxels)",
                    file.display(),
                    result["voxels"],
                    result["voxelSize"]
                ),
                json!({"ok": true, "command": "terrain info", "path": file.display().to_string(), "result": result}),
            );
            Ok(())
        }
        other => Err(usage_err(&format!("unknown terrain subcommand '{other}'"))),
    }
}

fn cmd_place(global: &Global, args: &[String]) -> Result<()> {
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
        other => Err(usage_err(&format!("unknown place subcommand '{other}'"))),
    }
}

fn cmd_template(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("template needs a subcommand: list|inspect"));
    };
    match sub.as_str() {
        "list" => {
            let templates = bloxcli::templates::registry()
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
            let template = bloxcli::templates::find(name)
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
        other => Err(usage_err(&format!("unknown template subcommand '{other}'"))),
    }
}

fn cmd_run(global: &Global, args: &[String]) -> Result<()> {
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
        bloxcli::engine::node_from_value(tree_json)?
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

fn cmd_mcp(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["root"], &[])?;
    let root = parsed
        .opt("root")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let engine_path = find_engine(global.engine.as_deref())?;
    eprintln!("bloxcli MCP server (root: {})", root.display());
    bloxcli::mcp::run_server(root, engine_path)
}

fn cmd_new(global: &Global, args: &[String]) -> Result<()> {
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
    if bloxcli::templates::find(&template).is_none() {
        return Err(usage_err(&format!(
            "unknown template '{template}' (available: {})",
            bloxcli::templates::registry()
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
    bloxcli::templates::apply(&mut engine, &template)?;
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
fn write_mutation(file: &Path, tree: &Node) -> Result<PlaceFormat> {
    let mut history = History::open(file)?;
    history.record(file)?;
    write_place(file, tree)
}

/// Loads a place into the engine, runs `op`, snapshots, and atomically
/// writes the result (with an undo point). The engine stays the source of
/// truth for the whole operation.
fn mutate<F>(global: &Global, file: &Path, op: F) -> Result<(JsonValue, Node)>
where
    F: FnOnce(&mut EngineClient) -> Result<JsonValue>,
{
    let (mut engine, _format) = load_session(global, file)?;
    let result = match op(&mut engine) {
        Ok(r) => r,
        Err(e) => {
            engine.shutdown();
            return Err(e);
        }
    };
    let tree = engine.snapshot()?;
    engine.shutdown();
    write_mutation(file, &tree)?;
    Ok((result, tree))
}

fn cmd_tree(global: &Global, args: &[String]) -> Result<()> {
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

fn print_node(node: &Node, depth: usize, props: bool, is_root: bool, max_depth: Option<usize>, out: &mut String) {
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

fn cmd_add(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_set(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "property", "value", "type"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
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
    engine.set(&target, &property, &value)?;
    let tree = engine.snapshot()?;
    engine.shutdown();
    write_mutation(&file, &tree)?;

    emit(
        global,
        format!("set {target}.{property} = {} in {}", value.display(), file.display()),
        json!({
            "ok": true, "command": "set", "path": file.display().to_string(),
            "target": target, "property": property, "value": value,
        }),
    );
    Ok(())
}

fn cmd_save(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_export(global: &Global, args: &[String]) -> Result<()> {
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


// ---------- Phase 17: model files (.rbxm/.rbxmx) ----------

/// Absolute child-index path of a display path (same segment rules as
/// `Node::find_path`: a leading `game`/root name/root class is skipped,
/// ambiguous sibling names are errors).
fn display_to_index(root: &Node, path: &str) -> Result<Vec<usize>> {
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

fn join_index_path(path: &[usize]) -> String {
    path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("/")
}

/// Rewrites every non-empty Ref in a cloned model subtree from
/// place-absolute form to model-relative index paths. References that
/// leave the subtree — or point at the subtree root itself, which the
/// file format cannot distinguish from `None` — fail the export.
fn rewrite_refs_for_model_export(
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
                return Err(Error(format!(
                    "property '{name}' on '{display}' references the model root itself, which a model file cannot represent distinctly from None"
                )));
            }
            *target = join_index_path(rel);
        }
    }
    for child in node.children.iter_mut() {
        rewrite_refs_for_model_export(place_root, subtree_abs, child, &format!("{display}/{}", child.name))?;
    }
    Ok(())
}

/// Rewrites model-relative refs to place-absolute index paths after a
/// graft at `graft_abs` (the index path the model root will occupy).
fn rewrite_refs_for_model_import(model_root: &Node, graft_abs: &[usize], node: &mut Node, display: &str) -> Result<()> {
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

fn cmd_model(global: &Global, args: &[String]) -> Result<()> {
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
            let parent_node = bloxcli::integrity::node_at(&place, &parent_abs)
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

// ---------- Phase 3: rendering commands ----------

fn parse_probe(text: &str) -> Result<Vec<(i64, i64)>> {
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

fn cmd_render(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_camera(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_ui(global: &Global, args: &[String]) -> Result<()> {
    // Implemented in the Phase 4 block below (kept here so unknown `ui`
    // subcommands fail with usage instead of "unknown command").
    ui_dispatch(global, args)
}

// ---------- Phase 2 commands ----------

fn cmd_get(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_find(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_clone(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_delete(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_reparent(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_group(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_move(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "delta", "position"], &["relative"])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    if parsed.opt("delta").is_none() && parsed.opt("position").is_none() {
        return Err(usage_err("move requires --delta 'x,y,z' (or --position for an absolute BasePart position)"));
    }
    // Absolute positioning is only well-defined for a single BasePart.
    if let Some(position) = parsed.opt("position") {
        let wanted = parse_vec3(position)?;
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
    let delta = parse_vec3(parsed.opt("delta").unwrap())?;
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

fn cmd_rotate(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "angles"], &["world"])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    let angles_raw = parsed.required("angles")?;
    let angles = parse_vec3(angles_raw)?;
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

fn cmd_scale(global: &Global, args: &[String]) -> Result<()> {
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

fn cmd_pivot(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(args, &["target", "position"], &[])?;
    let file = parsed.file()?;
    let target = parsed.required("target")?.to_string();
    if let Some(position_raw) = parsed.opt("position").map(|s| s.to_string()) {
        let pos = parse_vec3(&position_raw)?;
        let value = PropValue::CFrame(bloxcli::tree::CFrameValue::identity_at(pos[0], pos[1], pos[2]));
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

fn cmd_undo(global: &Global, args: &[String], redo: bool) -> Result<()> {
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

fn cmd_validate(global: &Global, args: &[String]) -> Result<()> {
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

// ---------- Phase 4: UI commands ----------

fn device_size(device: Option<&str>, width: Option<&str>, height: Option<&str>) -> Result<(u32, u32)> {
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

fn ui_base_params(file: &Path, parsed: &CmdArgs, width: u32, height: u32) -> JsonValue {
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
    params
}

fn ui_dispatch(global: &Global, args: &[String]) -> Result<()> {
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

fn ui_render_cmd(global: &Global, args: &[String]) -> Result<()> {
    let parsed = parse_cmd(
        args,
        &["output", "width", "height", "device", "gui", "assets", "camera", "look-at", "probe"],
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

fn ui_inspect_cmd(global: &Global, args: &[String]) -> Result<()> {
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

fn ui_validate_cmd(global: &Global, args: &[String]) -> Result<()> {
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

fn ui_hit_test_cmd(global: &Global, args: &[String]) -> Result<()> {
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

