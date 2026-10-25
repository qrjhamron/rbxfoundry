//! CLI argument parsing: global flags, per-command flag parsing
//! (`CmdArgs`/`parse_cmd`) and help text. No business logic lives here.

use std::path::PathBuf;
use crate::protocol::tree::{Error, Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Global {
    pub json: bool,
    pub engine: Option<String>,
}

pub fn usage_err(msg: &str) -> Error {
    Error(format!("usage: {msg} (run 'bloxcli --help')"))
}

pub fn print_help() {
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
         \x20   ungroup <file> --target <path>\n\
         \x20       Move a container's children to its parent and delete the container.\n\
         \x20   exec <file> --code <luau>\n\
         \x20       Studio Command Bar: run Luau in the edit context and commit the result.\n\
         \x20   script analyze <file.luau | place --target <script>>\n\
         \x20   script complete <place> --target <script> --line <n> --column <n>\n\
         \x20       Official Luau frontend diagnostics; reflection-based completion.\n\
         \x20   debug start|stop <place>; debug breakpoint add|list|clear; debug run|\n\
         \x20       continue|step-over|step-into|step-out|stack|variables <place>\n\
         \x20       Real Luau VM debugger sessions (breakpoints, stepping, locals).\n\
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

pub struct CmdArgs {
    pub positional: Vec<String>,
    pub flags: std::collections::HashMap<String, Option<String>>,
}

pub fn parse_cmd(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<CmdArgs> {
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
    pub fn opt(&self, key: &str) -> Option<&str> {
        self.flags.get(key).and_then(|v| v.as_deref())
    }
    pub fn has(&self, key: &str) -> bool {
        self.flags.contains_key(key)
    }
    pub fn required(&self, key: &str) -> Result<&str> {
        self.opt(key)
            .ok_or_else(|| usage_err(&format!("missing required --{key}")))
    }
    pub fn file(&self) -> Result<PathBuf> {
        self.positional
            .first()
            .map(PathBuf::from)
            .ok_or_else(|| usage_err("missing <file> argument"))
    }
}

// ---------- engine sessions ----------

