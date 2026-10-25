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

/// Model files (Phase 17): `.rbxm` (binary) / `.rbxmx` (XML) contain a
/// single non-DataModel root instance instead of place services.
pub fn format_for_model_path(path: &Path) -> Result<PlaceFormat> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rbxm") => Ok(PlaceFormat::Binary),
        Some("rbxmx") => Ok(PlaceFormat::Xml),
        _ => err(format!(
            "'{}' must end in .rbxm (binary) or .rbxmx (XML)",
            path.display()
        )),
    }
}

/// Applies `f` to every non-empty Ref value in a tree (used to shift
/// reference index paths when wrapping/unwrapping a model root).
fn shift_refs(node: &mut Node, f: &dyn Fn(&str) -> String) {
    for value in node.properties.values_mut() {
        if let crate::tree::PropValue::Ref(target) = value {
            if !target.is_empty() {
                *target = f(target);
            }
        }
    }
    for child in node.children.iter_mut() {
        shift_refs(child, f);
    }
}

/// `rbx_xml` silently decodes a `<Ref>` naming an undefined referent
/// as `None`, which would silently lose the reference. Scan the XML
/// text first so such files fail loudly instead (Phase 17).
fn check_xml_referents(bytes: &[u8]) -> Result<()> {
    let text = String::from_utf8_lossy(bytes);
    let mut declared = std::collections::HashSet::new();
    let mut rest = text.as_ref();
    while let Some(pos) = rest.find("referent=\"") {
        let after = &rest[pos + 10..];
        if let Some(end) = after.find('"') {
            declared.insert(after[..end].to_string());
        }
        rest = after;
    }
    let mut rest = text.as_ref();
    while let Some(pos) = rest.find("<Ref") {
        let after = &rest[pos + 4..];
        let Some(gt) = after.find('>') else { break };
        let after_tag = &after[gt + 1..];
        let Some(end) = after_tag.find("</Ref>") else { break };
        let target = after_tag[..end].trim();
        if target != "null" && !target.is_empty() && !declared.contains(target) {
            return err(format!(
                "file contains a reference to undefined referent '{target}'; refusing to silently drop it"
            ));
        }
        rest = &after_tag[end + 6..];
    }
    Ok(())
}

pub fn read_model(path: &Path) -> Result<(Node, PlaceFormat)> {
    if matches!(path.extension().and_then(|e| e.to_str()), Some("rbxl") | Some("rbxlx")) {
        return err(format!(
            "'{}' is a place file; use place commands (or 'model export' from a place)",
            path.display()
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| Error(format!("cannot read {}: {e}", path.display())))?;
    let format = sniff_format(&bytes)?;
    if format == PlaceFormat::Xml {
        check_xml_referents(&bytes)?;
    }
    let dom = decode_dom(&bytes, format)?;
    let wrapper = dom_to_node(&dom)?;
    // rbx-dom synthesizes a DataModel root that is not stored in the
    // file; the model root is its single child. (Multi-root model files
    // are rejected loudly rather than partially imported.)
    if wrapper.children.len() != 1 {
        return err(format!(
            "'{}' has {} top-level instances; only single-root model files are supported",
            path.display(),
            wrapper.children.len()
        ));
    }
    let mut model = wrapper.children.into_iter().next().unwrap();
    if model.class_name == "DataModel" {
        return err(format!("'{}' is a place (DataModel root), not a model file", path.display()));
    }
    // References decode wrapper-relative ("0/0/.."): strip the wrapper
    // level so they are relative to the model root. A reference to the
    // model root itself ("0") becomes "" (None) — see COMPATIBILITY.md.
    shift_refs(&mut model, &|target| {
        target
            .strip_prefix("0/")
            .map(|rest| rest.to_string())
            .unwrap_or_default()
    });
    Ok((model, format))
}

/// Writes a model file atomically after a re-decode self-check, like
/// `write_place`. The subtree root is stored as the file root.
pub fn write_model(path: &Path, node: &Node) -> Result<PlaceFormat> {
    let format = format_for_model_path(path)?;
    // Encode under a synthetic wrapper so the on-disk roots are exactly
    // the model subtree; shift model-relative refs one level down.
    let mut model = node.clone();
    shift_refs(&mut model, &|target| format!("0/{target}"));
    let mut wrapper = Node::new("DataModel", "");
    wrapper.children.push(model);
    let bytes = node_to_bytes(&wrapper, format)?;
    let dom = decode_dom(&bytes, format)?;
    let _roundtrip = dom_to_node(&dom)?;
    write_atomic(path, &bytes)?;
    Ok(format)
}

pub fn read_place(path: &Path) -> Result<(Node, PlaceFormat)> {
    if matches!(path.extension().and_then(|e| e.to_str()), Some("rbxm") | Some("rbxmx")) {
        return err(format!(
            "'{}' is a model file, not a place; use 'model inspect' / 'model import'",
            path.display()
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| Error(format!("cannot read {}: {e}", path.display())))?;
    let format = sniff_format(&bytes)?;
    if format == PlaceFormat::Xml {
        check_xml_referents(&bytes)?;
    }
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
