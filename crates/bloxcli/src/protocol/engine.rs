//! Client for the persistent C++ `blox-engine` process.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over the child's stdin/stdout.
//! The first request is always `initialize` with protocol version "1"; a
//! version mismatch is a hard error.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value as JsonValue};

use crate::protocol::tree::{err, Error, Node, Result};

pub const PROTOCOL_VERSION: &str = "1";

pub struct EngineClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    pub engine_version: String,
}

/// Locates the engine binary: explicit path, `$BLOX_ENGINE`, a sibling of the
/// CLI executable, `./build/engine/blox-engine`, or the workspace build tree.
pub fn find_engine(explicit: Option<&str>) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = explicit {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(path) = std::env::var("BLOX_ENGINE") {
        if !path.is_empty() {
            candidates.push(PathBuf::from(path));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("blox-engine"));
        }
    }
    candidates.push(PathBuf::from("build/engine/blox-engine"));
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine"),
    );
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    err(format!(
        "could not find the blox-engine binary (looked for: {}). \
         Build it with: cmake -S . -B build -G Ninja && cmake --build build, \
         or pass --engine <path> / set $BLOX_ENGINE",
        candidates
            .iter()
            .map(|c| c.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

impl EngineClient {
    pub fn spawn(engine_path: &std::path::Path) -> Result<Self> {
        let mut child = Command::new(engine_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| Error(format!("failed to start engine {}: {e}", engine_path.display())))?;
        let stdin = child.stdin.take().expect("engine stdin");
        let stdout = BufReader::new(child.stdout.take().expect("engine stdout"));
        let mut client = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
            engine_version: String::new(),
        };
        let hello = client.call(
            "initialize",
            json!({ "protocolVersion": PROTOCOL_VERSION }),
        )?;
        client.engine_version = hello
            .get("engineVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(client)
    }

    pub fn call(&mut self, method: &str, params: JsonValue) -> Result<JsonValue> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = serde_json::to_string(&request).expect("request serialization");
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| Error(format!("failed to write to engine: {e}")))?;
        let mut response_line = String::new();
        let read = self
            .stdout
            .read_line(&mut response_line)
            .map_err(|e| Error(format!("failed to read from engine: {e}")))?;
        if read == 0 {
            return err("engine closed its output stream unexpectedly");
        }
        let response: JsonValue = serde_json::from_str(response_line.trim())
            .map_err(|e| Error(format!("engine sent invalid JSON: {e}")))?;
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown engine error");
            return err(format!("engine error in '{method}': {message}"));
        }
        Ok(response.get("result").cloned().unwrap_or(JsonValue::Null))
    }

    pub fn new_datamodel(&mut self, name: &str) -> Result<Node> {
        let result = self.call("datamodel/new", json!({ "name": name }))?;
        node_from_json(result.get("tree").ok_or_else(|| Error("engine omitted tree".into()))?)
    }

    pub fn load(&mut self, tree: &Node) -> Result<()> {
        let tree_json = serde_json::to_value(tree)
            .map_err(|e| Error(format!("failed to serialize tree: {e}")))?;
        self.call("datamodel/load", json!({ "tree": tree_json }))?;
        Ok(())
    }

    pub fn snapshot(&mut self) -> Result<Node> {
        let result = self.call("datamodel/snapshot", json!({}))?;
        node_from_json(result.get("tree").ok_or_else(|| Error("engine omitted tree".into()))?)
    }

    pub fn add(&mut self, parent: &str, class_name: &str, name: &str) -> Result<JsonValue> {
        self.call(
            "instance/add",
            json!({ "parent": parent, "className": class_name, "name": name }),
        )
    }

    pub fn get(&mut self, target: &str) -> Result<Node> {
        let result = self.call("instance/get", json!({ "target": target }))?;
        let inst = result
            .get("instance")
            .ok_or_else(|| Error("engine omitted instance".into()))?;
        node_from_json(inst)
    }

    pub fn set(
        &mut self,
        target: &str,
        property: &str,
        value: &crate::protocol::tree::PropValue,
    ) -> Result<JsonValue> {
        let value_json = serde_json::to_value(value)
            .map_err(|e| Error(format!("failed to serialize value: {e}")))?;
        self.call(
            "instance/set",
            json!({ "target": target, "property": property, "value": value_json }),
        )
    }

    pub fn validate(&mut self) -> Result<JsonValue> {
        self.call("validate", json!({}))
    }

    pub fn clone_instance(
        &mut self,
        target: &str,
        name: Option<&str>,
        parent: Option<&str>,
    ) -> Result<JsonValue> {
        let mut params = json!({ "target": target });
        if let Some(name) = name {
            params["name"] = json!(name);
        }
        if let Some(parent) = parent {
            params["parent"] = json!(parent);
        }
        self.call("instance/clone", params)
    }

    pub fn delete(&mut self, target: &str) -> Result<JsonValue> {
        self.call("instance/delete", json!({ "target": target }))
    }

    pub fn reparent(&mut self, target: &str, parent: &str) -> Result<JsonValue> {
        self.call(
            "instance/reparent",
            json!({ "target": target, "parent": parent }),
        )
    }

    pub fn group(
        &mut self,
        targets: &[String],
        name: &str,
        parent: Option<&str>,
    ) -> Result<JsonValue> {
        let mut params = json!({ "targets": targets, "name": name });
        if let Some(parent) = parent {
            params["parent"] = json!(parent);
        }
        self.call("instance/group", params)
    }

    pub fn find(
        &mut self,
        name: Option<&str>,
        class_name: Option<&str>,
        substring: bool,
        root: Option<&str>,
    ) -> Result<JsonValue> {
        let mut params = json!({ "substring": substring });
        if let Some(name) = name {
            params["name"] = json!(name);
        }
        if let Some(class_name) = class_name {
            params["className"] = json!(class_name);
        }
        if let Some(root) = root {
            params["root"] = json!(root);
        }
        self.call("instance/find", params)
    }

    pub fn move_by(&mut self, target: &str, delta: [f32; 3], relative: bool) -> Result<JsonValue> {
        self.call(
            "instance/move",
            json!({ "target": target, "delta": delta, "relative": relative }),
        )
    }

    pub fn rotate(&mut self, target: &str, angles: [f32; 3], world_space: bool) -> Result<JsonValue> {
        self.call(
            "instance/rotate",
            json!({ "target": target, "angles": angles, "worldSpace": world_space }),
        )
    }

    pub fn scale_factor(&mut self, target: &str, factor: f32) -> Result<JsonValue> {
        self.call("instance/scale", json!({ "target": target, "factor": factor }))
    }

    pub fn scale_factors(&mut self, target: &str, factors: [f32; 3]) -> Result<JsonValue> {
        self.call(
            "instance/scale",
            json!({ "target": target, "factors": factors }),
        )
    }

    /// Gets a Model's effective pivot, or sets it when `value` is given.
    pub fn pivot(
        &mut self,
        target: &str,
        value: Option<&crate::protocol::tree::PropValue>,
    ) -> Result<JsonValue> {
        let mut params = json!({ "target": target });
        if let Some(value) = value {
            params["value"] = serde_json::to_value(value)
                .map_err(|e| Error(format!("failed to serialize pivot: {e}")))?;
        }
        self.call("instance/pivot", params)
    }

    pub fn render_camera(
        &mut self,
        assets_dir: Option<&str>,
        place_dir: Option<&str>,
    ) -> Result<JsonValue> {
        let mut params = json!({});
        if let Some(a) = assets_dir {
            params["assetsDir"] = json!(a);
        }
        if let Some(p) = place_dir {
            params["placeDir"] = json!(p);
        }
        self.call("render/camera", params)
    }

    /// Renders the loaded DataModel to a PNG at `output_path` (written by
    /// the engine; the CLI verifies the PNG afterwards).
    pub fn render_screenshot(&mut self, params: JsonValue) -> Result<JsonValue> {
        self.call("render/screenshot", params)
    }

    pub fn ui_rpc(&mut self, method: &str, params: JsonValue) -> Result<JsonValue> {
        self.call(method, params)
    }

    pub fn shutdown(mut self) {
        let _ = self.call("shutdown", json!({}));
        let _ = self.child.wait();
    }
}

/// Converts an engine tree JSON value into a Node (public for the
/// runtime/play session code paths).
pub fn node_from_value(value: &JsonValue) -> Result<Node> {
    node_from_json(value)
}

fn node_from_json(value: &JsonValue) -> Result<Node> {
    serde_json::from_value(value.clone())
        .map_err(|e| Error(format!("engine sent a malformed tree: {e}")))
}
