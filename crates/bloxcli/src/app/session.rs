//! Application services shared by CLI, MCP and session daemons:
//! engine session startup, place loading, output emission and the
//! atomic history-recording mutation helper. Commands must go
//! through these instead of open-coding snapshot/history/write.

use std::path::Path;
use serde_json::Value as JsonValue;
use crate::cli::args::Global;
use crate::protocol::engine::{find_engine, EngineClient};
use crate::protocol::tree::{Node, Result};
use crate::rbxio::{read_place, write_place, PlaceFormat};
use crate::history::History;

pub fn spawn_engine(global: &Global) -> Result<EngineClient> {
    let path = find_engine(global.engine.as_deref())?;
    EngineClient::spawn(&path)
}

/// Loads a place file into a fresh engine process. The engine holds the
/// authoritative tree for the rest of the command.
pub fn load_session(global: &Global, file: &Path) -> Result<(EngineClient, PlaceFormat)> {
    let (node, format) = read_place(file)?;
    let mut engine = spawn_engine(global)?;
    engine.load(&node)?;
    Ok((engine, format))
}

pub fn emit(global: &Global, human: String, payload: JsonValue) {
    if global.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).expect("json output")
        );
    } else {
        println!("{human}");
    }
}

pub fn format_name(format: PlaceFormat) -> &'static str {
    match format {
        PlaceFormat::Binary => "rbxl (binary)",
        PlaceFormat::Xml => "rbxlx (XML)",
    }
}

// ---------- commands ----------

pub fn write_mutation(file: &Path, tree: &Node) -> Result<PlaceFormat> {
    let mut history = History::open(file)?;
    history.record(file)?;
    write_place(file, tree)
}

/// Loads a place into the engine, runs `op`, snapshots, and atomically
/// writes the result (with an undo point). The engine stays the source of
/// truth for the whole operation.
pub fn mutate<F>(global: &Global, file: &Path, op: F) -> Result<(JsonValue, Node)>
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

