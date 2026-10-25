//! MCP tool JSON schemas (`tools/list` payload). Names and schemas
//! here are the public MCP contract; do not rename without updating
//! clients.

use serde_json::{json, Value as JsonValue};

fn schema(properties: JsonValue, required: &[&str]) -> JsonValue {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn tool(name: &str, description: &str, input_schema: JsonValue) -> JsonValue {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

const PATH_PROP: &str = "\"path\": {\"type\": \"string\", \"description\": \"Place file inside the project root (.rbxl/.rbxlx)\"}";

pub fn tool_definitions() -> Vec<JsonValue> {
    let path_only = serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}}}")).unwrap();
    vec![
        tool("studio_open", "Open an existing .rbxl/.rbxlx place (inside the MCP project root) into the persistent engine session.", schema(path_only.clone(), &["path"])),
        tool("studio_new", "Create a new place with the standard Roblox service set and save it.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"name\": {{\"type\": \"string\"}}, \"force\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("studio_save", "Save the open place atomically (records an undo point).", schema(json!({}), &[])),
        tool("studio_export", "Export the open place to another .rbxl/.rbxlx path.", schema(json!({"output": {"type": "string"}}), &["output"])),
        tool("studio_tree", "Return the full instance tree (or the subtree at 'path') as JSON.", schema(json!({"path": {"type": "string"}}), &[])),
        tool("studio_inspect", "Return one instance with properties and children.", schema(json!({"target": {"type": "string"}}), &["target"])),
        tool("instance_create", "Create an instance (engine reflection defaults) under a parent path. Auto-saves.", schema(
            json!({"parent": {"type": "string"}, "className": {"type": "string"}, "name": {"type": "string"}}), &["parent", "className"])),
        tool("instance_get", "Return one instance with properties and children.", schema(json!({"target": {"type": "string"}}), &["target"])),
        tool("instance_set", "Set a typed property from its string form (e.g. Color3 '1,0,0', UDim2 '0.5,0,0.5,0'). 'type' is inferred from the existing property when omitted. Auto-saves.", schema(
            json!({"target": {"type": "string"}, "property": {"type": "string"}, "value": {"type": "string"}, "type": {"type": "string"}}),
            &["target", "property", "value"])),
        tool("instance_delete", "Delete an instance subtree. Auto-saves.", schema(json!({"target": {"type": "string"}}), &["target"])),
        tool("instance_clone", "Deep-clone an instance with fresh referents. Auto-saves.", schema(
            json!({"target": {"type": "string"}, "parent": {"type": "string"}, "name": {"type": "string"}}), &["target"])),
        tool("instance_reparent", "Move an instance under a new parent. Auto-saves.", schema(
            json!({"target": {"type": "string"}, "parent": {"type": "string"}}), &["target", "parent"])),
        tool("instance_ungroup", "Ungroup a Model/Folder: children move to its parent and the container is deleted. Auto-saves.", schema(json!({"target": {"type": "string"}}), &["target"])),
        tool("instance_transform", "Apply moveDelta [x,y,z], rotate [rx,ry,rz] degrees, and/or scaleFactor to a part or model. Auto-saves.", schema(
            json!({"target": {"type": "string"}, "moveDelta": {"type": "array", "items": {"type": "number"}},
                   "rotate": {"type": "array", "items": {"type": "number"}}, "scaleFactor": {"type": "number"},
                   "relative": {"type": "boolean"}, "world": {"type": "boolean"}}), &["target"])),
        tool("scene_render", "Render the open place's Workspace with the real Vulkan renderer. Returns a PNG image. Camera defaults to the session camera (camera_control) or the isometric framing.", schema(
            json!({"output": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"},
                   "preset": {"type": "string"}, "position": {"type": "array", "items": {"type": "number"}},
                   "lookAt": {"type": "array", "items": {"type": "number"}}, "fovDegrees": {"type": "number"}}), &[])),
        tool("camera_control", "action=inspect reports CurrentCamera, scene bounds and suggested framing; action=set stores the session camera used by scene_render.", schema(
            json!({"action": {"type": "string", "enum": ["inspect", "set"]},
                   "position": {"type": "array", "items": {"type": "number"}},
                   "lookAt": {"type": "array", "items": {"type": "number"}}, "fovDegrees": {"type": "number"}}), &[])),
        tool("ui_render", "Rasterise the open place's GUI instances to a PNG image at a device preset or custom size.", schema(
            json!({"output": {"type": "string"}, "device": {"type": "string", "enum": ["desktop", "laptop", "tablet", "mobile"]},
                   "width": {"type": "integer"}, "height": {"type": "integer"}, "gui": {"type": "string"}}), &[])),
        tool("ui_inspect", "Computed absolute GUI layout rects, visibility and render order (JSON).", schema(
            json!({"device": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"}, "gui": {"type": "string"}}), &[])),
        tool("ui_validate", "Validate GUI layout: offscreen (error), clipped, zero-size and invisible elements.", schema(
            json!({"device": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"}, "gui": {"type": "string"}}), &[])),
        tool("script_analyze", "Run the official Luau frontend (syntax + type errors + lint with spans) over a script's Source in the open place. Roblox globals are declared as any.", schema(json!({"target": {"type": "string"}}), &["target"])),
        tool("runtime_run", "Run the open place's Luau scripts in the real runtime (separate from edit state): starts Script/LocalScript instances, optionally evals a chunk, then steps simulated time. Returns logs and structured script errors.", schema(
            json!({"duration": {"type": "number"}, "step": {"type": "number"}, "eval": {"type": "string"}}), &[])),
        tool("runtime_export", "Export the current runtime DataModel state to a .rbxl/.rbxlx file.", schema(json!({"output": {"type": "string"}}), &["output"])),
        tool("play_start", "Start a persistent playtest session (Jolt physics + Luau runtime + character) for a place inside the root. rig: R6 or R15.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"rig\": {{\"type\": \"string\", \"enum\": [\"R6\", \"R15\"]}}, \"force\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("play_step", "Advance a play session by fixed 1/60 steps (keys released; use play_input to move).", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"steps\": {{\"type\": \"integer\"}}}}")).unwrap(), &["path"])),
        tool("play_input", "Hold keys (W,A,S,D,Space) for a duration in a play session, optionally setting camera yawDeg/pitchDeg/firstPerson. The character moves through real physics, never teleporting.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"keys\": {{\"type\": \"array\", \"items\": {{\"type\": \"string\"}}}}, \"duration\": {{\"type\": \"number\"}}, \"yawDeg\": {{\"type\": \"number\"}}, \"pitchDeg\": {{\"type\": \"number\"}}, \"firstPerson\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("play_state", "Character position/velocity/health/state, simulation time and camera for a play session.", schema(path_only.clone(), &["path"])),
        tool("play_screenshot", "Screenshot a play session from the character camera. Returns a PNG image.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"output\": {{\"type\": \"string\"}}, \"width\": {{\"type\": \"integer\"}}, \"height\": {{\"type\": \"integer\"}}}}")).unwrap(), &["path"])),
        tool("play_logs", "Luau print/warn output captured during a play session.", schema(path_only.clone(), &["path"])),
        tool("play_stop", "Stop a play session. commit=true writes the runtime world back into the place file (with history); otherwise it is discarded.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"commit\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("net_start", "Start a multi-context net session: one authoritative server plus one client engine per simulated player, separate DataModels/Luau contexts, real RemoteEvent/RemoteFunction message passing, snapshot-diff replication and configurable latency. players 1..8, rig R6/R15.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"players\": {{\"type\": \"integer\"}}, \"latencyMs\": {{\"type\": \"number\"}}, \"rig\": {{\"type\": \"string\", \"enum\": [\"R6\", \"R15\"]}}, \"force\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("net_step", "Advance every net peer by fixed 1/60 steps, delivering due remote messages and server->client replication.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"steps\": {{\"type\": \"integer\"}}}}")).unwrap(), &["path"])),
        tool("net_input", "Hold keys (W,A,S,D,Space) for one simulated player for a duration; only that player's server-side character moves (server-authoritative physics).", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"player\": {{\"type\": \"integer\"}}, \"keys\": {{\"type\": \"array\", \"items\": {{\"type\": \"string\"}}}}, \"duration\": {{\"type\": \"number\"}}, \"yawDeg\": {{\"type\": \"number\"}}}}")).unwrap(), &["path"])),
        tool("net_state", "Net session state: step/time, players, per-player character physics state, server/client runtime state, queued message count and warnings.", schema(path_only.clone(), &["path"])),
        tool("net_screenshot", "Screenshot one client's replicated view from behind its character. Returns a PNG image.", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"player\": {{\"type\": \"integer\"}}, \"output\": {{\"type\": \"string\"}}, \"width\": {{\"type\": \"integer\"}}, \"height\": {{\"type\": \"integer\"}}}}")).unwrap(), &["path"])),
        tool("net_logs", "Per-peer Luau logs/errors (each entry tagged with its peer: server/client1/...).", schema(path_only.clone(), &["path"])),
        tool("net_stop", "Stop a net session. commit=true writes the authoritative server world back into the place file (with history).", schema(
            serde_json::from_str::<JsonValue>(&format!("{{{PATH_PROP}, \"commit\": {{\"type\": \"boolean\"}}}}")).unwrap(), &["path"])),
        tool("studio_undo", "Undo the last saved mutation of the open place and reload it.", schema(json!({}), &[])),
        tool("studio_redo", "Redo a previously undone mutation of the open place and reload it.", schema(json!({}), &[])),
    ]
}
