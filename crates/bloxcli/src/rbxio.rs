//! Genuine `.rbxl` (binary) and `.rbxlx` (XML) file I/O via the upstream
//! `rbx_binary` / `rbx_xml` serializers from the rbx-dom project. BloxCLI
//! never writes JSON into a `.rbxl` file.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use rbx_dom_weak::WeakDom;

use crate::convert::{dom_to_node, node_to_dom};
use crate::tree::{err, Error, Node, Result};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaceFormat {
    Binary, // .rbxl
    Xml,    // .rbxlx
}

pub fn format_for_path(path: &Path) -> Result<PlaceFormat> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rbxl") => Ok(PlaceFormat::Binary),
        Some("rbxlx") => Ok(PlaceFormat::Xml),
        _ => err(format!(
            "'{}' must end in .rbxl (binary) or .rbxlx (XML)",
            path.display()
        )),
    }
}

/// Sniffs the format from file contents instead of trusting the extension:
/// binary places start with the `<roblox!` magic, XML places with `<` of an
/// XML document containing `roblox`.
pub fn sniff_format(bytes: &[u8]) -> Result<PlaceFormat> {
    if bytes.starts_with(b"<roblox!") {
        Ok(PlaceFormat::Binary)
    } else if bytes
        .windows(6)
        .take(64)
        .any(|w| w == b"roblox")
        || bytes.starts_with(b"<")
    {
        Ok(PlaceFormat::Xml)
    } else {
        err("file is neither a Roblox binary place (<roblox! magic) nor a Roblox XML place".to_string())
    }
}

pub fn read_place(path: &Path) -> Result<(Node, PlaceFormat)> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error(format!("cannot read {}: {e}", path.display())))?;
    let format = sniff_format(&bytes)?;
    let dom = decode_dom(&bytes, format)?;
    let node = dom_to_node(&dom)?;
    if node.class_name != "DataModel" {
        return err(format!(
            "place root must be a DataModel, found '{}' (model files are not places)",
            node.class_name
        ));
    }
    Ok((node, format))
}

pub fn decode_dom(bytes: &[u8], format: PlaceFormat) -> Result<WeakDom> {
    match format {
        PlaceFormat::Binary => rbx_binary::from_reader(Cursor::new(bytes))
            .map_err(|e| Error(format!("invalid .rbxl binary file: {e}"))),
        PlaceFormat::Xml => rbx_xml::from_reader_default(Cursor::new(bytes))
            .map_err(|e| Error(format!("invalid .rbxlx XML file: {e}"))),
    }
}

pub fn encode_dom(dom: &WeakDom, format: PlaceFormat) -> Result<Vec<u8>> {
    // Place convention (per rbx-dom): the DataModel root itself is not stored
    // in the file; its children (the services) are the serialization roots.
    // Deserializers synthesize a fresh DataModel root on read.
    let root = dom.root();
    let refs: Vec<rbx_dom_weak::types::Ref> = if root.class.to_string() == "DataModel" {
        root.children().to_vec()
    } else {
        vec![dom.root_ref()]
    };
    let mut bytes = Vec::new();
    match format {
        PlaceFormat::Binary => {
            rbx_binary::to_writer(&mut bytes, dom, &refs)
                .map_err(|e| Error(format!("failed to serialize .rbxl: {e}")))?;
        }
        PlaceFormat::Xml => {
            rbx_xml::to_writer_default(&mut bytes, dom, &refs)
                .map_err(|e| Error(format!("failed to serialize .rbxlx: {e}")))?;
        }
    }
    Ok(bytes)
}

pub fn node_to_bytes(node: &Node, format: PlaceFormat) -> Result<Vec<u8>> {
    let dom = node_to_dom(node)?;
    encode_dom(&dom, format)
}

/// Writes atomically: serialize to a sibling temp file, then rename, so a
/// failure can never truncate the user's existing place file.
pub fn write_place(path: &Path, node: &Node) -> Result<PlaceFormat> {
    let format = format_for_path(path)?;
    let bytes = node_to_bytes(node, format)?;
    // Verify our own output before touching the destination.
    let dom = decode_dom(&bytes, format)?;
    let _roundtrip = dom_to_node(&dom)?;
    write_atomic(path, &bytes)?;
    Ok(format)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = temp_sibling(path);
    std::fs::write(&tmp, bytes)
        .map_err(|e| Error(format!("cannot write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error(format!("cannot replace {}: {e}", path.display()))
    })?;
    Ok(())
}

fn temp_sibling(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "place".to_string());
    name.push_str(&format!(".tmp-{}", std::process::id()));
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}
