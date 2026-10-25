//! Production MCP (Model Context Protocol) server over stdio.
//!
//! The server drives the same persistent C++ engine process and the same
//! `.rbxl`/`.rbxlx` files as the CLI — there is no second DataModel.
//! Mutating tools auto-save through the same history-recording atomic
//! write path the CLI uses, so MCP and CLI state never diverge.
//!
//! Protocol: JSON-RPC 2.0, newline-delimited, stdout is reserved for
//! protocol frames only (all diagnostics go to stderr). Filesystem access
//! is confined to the configured root directory.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value as JsonValue};

use crate::protocol::engine::EngineClient;
use crate::rbxio::read_place;
use crate::protocol::tree::{err, Error, Result};
use crate::mcp::schemas::tool_definitions;

/// Local error conversion (the crate Error type has no From impls).
pub(crate) trait MapErr<T> {
    fn mx(self) -> Result<T>;
}
impl<T, E: std::fmt::Display> MapErr<T> for std::result::Result<T, E> {
    fn mx(self) -> Result<T> {
        self.map_err(|e| Error(e.to_string()))
    }
}

const SUPPORTED_PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct McpServer {
    pub(crate) root: PathBuf,
    pub(crate) engine_path: PathBuf,
    pub(crate) engine: Option<EngineClient>,
    pub(crate) open_path: Option<PathBuf>,
    pub(crate) session_camera: Option<JsonValue>,
}

pub fn run_server(root: PathBuf, engine_path: PathBuf) -> Result<()> {
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir().mx()?.join(root)
    };
    let root = root.canonicalize().unwrap_or(root);
    let mut server = McpServer {
        root,
        engine_path,
        engine: None,
        open_path: None,
        session_camera: None,
    };
    server.serve()
}

impl McpServer {
    fn serve(&mut self) -> Result<()> {
        let stdin = std::io::stdin();
        let mut lines = stdin.lock().lines();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        while let Some(line) = lines.next() {
            let line = line.mx()?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let request: JsonValue = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    Self::write(&mut out, &json!({
                        "jsonrpc": "2.0", "id": null,
                        "error": {"code": -32700, "message": format!("parse error: {e}")}
                    }))?;
                    continue;
                }
            };
            if let Some(response) = self.handle(&request) {
                Self::write(&mut out, &response)?;
            }
        }
        Ok(())
    }

    fn write(out: &mut impl Write, value: &JsonValue) -> Result<()> {
        let text = serde_json::to_string(value).mx()?;
        out.write_all(text.as_bytes()).mx()?;
        out.write_all(b"\n").mx()?;
        out.flush().mx()?;
        Ok(())
    }

    fn result(id: &JsonValue, value: JsonValue) -> JsonValue {
        json!({"jsonrpc": "2.0", "id": id, "result": value})
    }

    fn rpc_error(id: &JsonValue, code: i64, message: &str) -> JsonValue {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
    }

    fn handle(&mut self, request: &JsonValue) -> Option<JsonValue> {
        let id = request.get("id").cloned().unwrap_or(JsonValue::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let is_notification = request.get("id").is_none();
        match method {
            "initialize" => {
                let requested = request
                    .pointer("/params/protocolVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("2025-03-26");
                let version = if SUPPORTED_PROTOCOLS.contains(&requested) {
                    requested
                } else {
                    "2025-03-26"
                };
                Some(Self::result(&id, json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "bloxcli", "version": env!("CARGO_PKG_VERSION")},
                })))
            }
            "ping" => Some(Self::result(&id, json!({}))),
            "tools/list" => Some(Self::result(&id, json!({"tools": tool_definitions()}))),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(json!({}));
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call_tool(name, &args) {
                    Ok(content) => Some(Self::result(&id, json!({"content": content, "isError": false}))),
                    Err(e) => Some(Self::result(&id, json!({
                        "content": [{"type": "text", "text": format!("error: {e}")}],
                        "isError": true,
                    }))),
                }
            }
            _ if is_notification => None,
            _ => Some(Self::rpc_error(&id, -32601, &format!("method not found: {method}"))),
        }
    }

    // ------------------------------------------------------------ paths ---

    /// Resolves a tool path inside the sandbox root. Existing files are
    /// canonicalised; for new files the parent directory must resolve
    /// inside the root.
    pub(crate) fn sandbox_path(&self, raw: &str) -> Result<PathBuf> {
        let candidate = if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            self.root.join(raw)
        };
        let resolved = if candidate.exists() {
            candidate.canonicalize().mx()?
        } else if let Some(parent) = candidate.parent() {
            if parent.exists() {
                let parent = parent.canonicalize().mx()?;
                parent.join(candidate.file_name().ok_or_else(|| Error("invalid path".into()))?)
            } else {
                candidate.clone()
            }
        } else {
            candidate.clone()
        };
        if !resolved.starts_with(&self.root) {
            return err(format!(
                "path '{}' escapes the MCP project root '{}'",
                raw,
                self.root.display()
            ));
        }
        Ok(resolved)
    }

    // ----------------------------------------------------------- engine ---

    pub(crate) fn engine(&mut self) -> Result<&mut EngineClient> {
        if self.engine.is_none() {
            self.engine = Some(EngineClient::spawn(&self.engine_path)?);
        }
        Ok(self.engine.as_mut().unwrap())
    }

    pub(crate) fn require_open(&self) -> Result<PathBuf> {
        self.open_path
            .clone()
            .ok_or_else(|| Error("no place is open; call studio_open or studio_new first".into()))
    }

    /// Snapshots the engine and writes the open place atomically (with an
    /// undo-history record), exactly like a CLI mutation.
    pub(crate) fn persist(&mut self) -> Result<JsonValue> {
        let path = self.require_open()?;
        let tree = self.engine()?.snapshot()?;
        crate::app::session::write_mutation(&path, &tree)?;
        Ok(json!({"saved": true, "path": path.display().to_string(), "instances": tree.count()}))
    }

    pub(crate) fn reload_open(&mut self) -> Result<()> {
        let path = self.require_open()?;
        let (node, _format) = read_place(&path)?;
        self.engine()?.load(&node)?;
        Ok(())
    }

    // ------------------------------------------------------------ tools ---
}
