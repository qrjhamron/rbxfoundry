//! Export integrity guard (Phase 17).
//!
//! Checks a canonical tree *before* any `.rbxl`/`.rbxlx` bytes are
//! written, so a place (or model export) can never silently lose data:
//!
//! * errors block the write: dangling instance references, corrupted
//!   terrain voxel payloads, instance references inside Attributes
//!   (Roblox forbids them there);
//! * warnings are reported but do not block: scripts without Source,
//!   `rbxassetid://` content with no local file (the id is preserved
//!   verbatim — never substituted), duplicate sibling names (paths
//!   become ambiguous).
//!
//! The guard runs on the canonical tree — the same data the
//! serializers consume — so its verdicts cannot drift from the writer.

use std::path::Path;

use serde_json::{json, Value as JsonValue};

use crate::protocol::tree::{base64_decode, Node, PropValue};

pub struct IntegrityIssue {
    pub severity: &'static str, // "error" | "warning"
    pub path: String,
    pub message: String,
}

impl IntegrityIssue {
    pub fn to_json(&self) -> JsonValue {
        json!({ "severity": self.severity, "path": self.path, "message": self.message, "source": "integrity" })
    }
}

/// Resolves a canonical reference value to an absolute child-index path
/// from `root` (root itself is `[]`). Canonical refs are either
/// `name:Service/Child/...` name paths or raw `0/2/1` index paths.
pub fn ref_index_path(root: &Node, target: &str) -> Option<Vec<usize>> {
    if target == "." {
        return Some(Vec::new()); // self-reference to the root
    }
    if let Some(rest) = target.strip_prefix("name:") {
        let mut cur = root;
        let mut path = Vec::new();
        for part in rest.split('/').filter(|s| !s.is_empty()) {
            let idx = cur.children.iter().position(|c| c.name == part)?;
            path.push(idx);
            cur = &cur.children[idx];
        }
        return Some(path);
    }
    // Index path form: every segment must be a valid child index.
    let mut cur = root;
    let mut path = Vec::new();
    for part in target.split('/').filter(|s| !s.is_empty()) {
        let idx: usize = part.parse().ok()?;
        cur = cur.children.get(idx)?;
        path.push(idx);
    }
    Some(path)
}

pub fn node_at<'a>(root: &'a Node, index_path: &[usize]) -> Option<&'a Node> {
    let mut cur = root;
    for &idx in index_path {
        cur = cur.children.get(idx)?;
    }
    Some(cur)
}

fn check_node(
    root: &Node,
    node: &Node,
    display_path: &str,
    place_dir: &Path,
    issues: &mut Vec<IntegrityIssue>,
) {
    // Duplicate sibling names make name paths ambiguous.
    {
        let mut seen = std::collections::HashSet::new();
        for child in &node.children {
            if !seen.insert(child.name.clone()) {
                issues.push(IntegrityIssue {
                    severity: "warning",
                    path: display_path.to_string(),
                    message: format!(
                        "duplicate sibling name '{}' — name paths to these instances are ambiguous",
                        child.name
                    ),
                });
            }
        }
    }

    if matches!(node.class_name.as_str(), "Script" | "LocalScript" | "ModuleScript") {
        let has_source = matches!(
            node.properties.get("Source"),
            Some(PropValue::String(s)) if !s.trim().is_empty()
        );
        if !has_source {
            issues.push(IntegrityIssue {
                severity: "warning",
                path: display_path.to_string(),
                message: format!("{} '{}' has no Source — it will do nothing in any runtime", node.class_name, node.name),
            });
        }
    }

    if node.class_name == "Terrain" {
        if let Some(PropValue::BinaryString(payload)) = node.properties.get("BloxVoxels") {
            if !payload.is_empty() {
                match base64_decode(payload) {
                    Ok(bytes) => {
                        let bytes = &bytes[..];
                        let valid = bytes.len() >= 8
                            && &bytes[0..4] == b"BTR1"
                            && {
                                let count = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
                                bytes.len() == 8 + count * 15
                            };
                        if !valid {
                            issues.push(IntegrityIssue {
                                severity: "error",
                                path: display_path.to_string(),
                                message: "Terrain BloxVoxels payload is corrupted (expected BTR1: magic + u32 count + 15 bytes/voxel); refusing to export it".to_string(),
                            });
                        }
                    }
                    Err(_) => issues.push(IntegrityIssue {
                        severity: "error",
                        path: display_path.to_string(),
                        message: "Terrain BloxVoxels is not valid base64; refusing to export corrupted terrain".to_string(),
                    }),
                }
            }
        }
    }

    for (name, value) in &node.properties {
        match value {
            PropValue::Ref(target) => {
                if !target.is_empty() && ref_index_path(root, target).is_none() {
                    issues.push(IntegrityIssue {
                        severity: "error",
                        path: display_path.to_string(),
                        message: format!(
                            "property '{name}' references '{target}', which does not exist in this place — export would write a dangling reference"
                        ),
                    });
                }
            }
            PropValue::Attributes(map) => {
                for (attr, attr_value) in map {
                    if matches!(attr_value, PropValue::Ref(_)) {
                        issues.push(IntegrityIssue {
                            severity: "error",
                            path: display_path.to_string(),
                            message: format!(
                                "attribute '{attr}' holds an instance reference — Roblox attributes cannot reference instances and serializers reject this"
                            ),
                        });
                    }
                }
            }
            PropValue::ContentId(content) => {
                if let Some(id) = content.strip_prefix("rbxassetid://") {
                    let id = id.trim();
                    if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                        let local = ["png", "jpg", "mesh", "ogg", "mp3"]
                            .iter()
                            .any(|ext| place_dir.join(format!("{id}.{ext}")).is_file());
                        if !local {
                            issues.push(IntegrityIssue {
                                severity: "warning",
                                path: display_path.to_string(),
                                message: format!(
                                    "property '{name}' references rbxassetid://{id} with no local file '{id}.*' next to the place — the id is preserved verbatim and will only resolve where the asset is available"
                                ),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for child in &node.children {
        check_node(root, child, &format!("{display_path}/{}", child.name), place_dir, issues);
    }
}

/// Runs every integrity check over `root`. `place_dir` is used only to
/// test whether referenced assets exist locally.
pub fn check_export_integrity(root: &Node, place_dir: &Path) -> Vec<IntegrityIssue> {
    let mut issues = Vec::new();
    check_node(root, root, &root.name, place_dir, &mut issues);
    issues
}

pub fn errors(issues: &[IntegrityIssue]) -> Vec<&IntegrityIssue> {
    issues.iter().filter(|i| i.severity == "error").collect()
}

pub fn warnings(issues: &[IntegrityIssue]) -> Vec<&IntegrityIssue> {
    issues.iter().filter(|i| i.severity == "warning").collect()
}

/// Error message used when the guard blocks a write.
pub fn blocked_message(issues: &[IntegrityIssue]) -> String {
    let mut msg = String::from("export integrity guard blocked the write (no file was modified):");
    for issue in errors(issues) {
        msg.push_str(&format!("\n  - [{}] {}", issue.path, issue.message));
    }
    msg
}
