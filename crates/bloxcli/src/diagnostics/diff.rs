//! Semantic DataModel diff (Phase 18, Parity Lab).
//!
//! Compares two canonical trees instance-by-instance and
//! property-by-property. Children are matched by name (with occurrence
//! index for duplicate sibling names); referents are ignored because
//! they are session-local. The output is the machine-readable core of
//! `blox diff` and `blox template compare`.

use serde_json::{json, Value as JsonValue};

use crate::protocol::tree::{Node, PropValue};

pub struct DiffEntry {
    pub kind: &'static str, // added | removed | class_changed | property_added | property_removed | property_changed
    pub path: String,
    pub property: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl DiffEntry {
    pub fn to_json(&self) -> JsonValue {
        let mut value = json!({ "kind": self.kind, "path": self.path });
        if let Some(p) = &self.property {
            value["property"] = json!(p);
        }
        if let Some(b) = &self.before {
            value["before"] = json!(b);
        }
        if let Some(a) = &self.after {
            value["after"] = json!(a);
        }
        value
    }
}

fn child_key(node: &Node, occurrence: usize) -> String {
    if occurrence == 0 {
        node.name.clone()
    } else {
        format!("{}#{occurrence}", node.name)
    }
}

/// Pairs up children of `a` and `b` by (name, occurrence among
/// same-named siblings). Returns (matched pairs, only-in-a, only-in-b).
fn match_children<'a>(
    a: &'a Node,
    b: &'a Node,
) -> (Vec<(&'a Node, &'a Node)>, Vec<&'a Node>, Vec<&'a Node>) {
    let mut b_by_key: std::collections::HashMap<String, &Node> = std::collections::HashMap::new();
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for child in &b.children {
        let occ = {
            let e = counts.entry(child.name.as_str()).or_insert(0);
            let v = *e;
            *e += 1;
            v
        };
        b_by_key.insert(child_key(child, occ), child);
    }
    let mut matched = Vec::new();
    let mut only_a = Vec::new();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    counts.clear();
    for child in &a.children {
        let occ = {
            let e = counts.entry(child.name.as_str()).or_insert(0);
            let v = *e;
            *e += 1;
            v
        };
        let key = child_key(child, occ);
        match b_by_key.get(&key) {
            Some(other) => {
                used.insert(key);
                matched.push((child, *other));
            }
            None => only_a.push(child),
        }
    }
    let mut only_b = Vec::new();
    counts.clear();
    for child in &b.children {
        let occ = {
            let e = counts.entry(child.name.as_str()).or_insert(0);
            let v = *e;
            *e += 1;
            v
        };
        if !used.contains(&child_key(child, occ)) {
            only_b.push(child);
        }
    }
    (matched, only_a, only_b)
}

/// Semantic property equivalence for diffing: tolerates the numeric
/// coercions the serializers legitimately apply (Float32<->Float64,
/// Color3 byte quantization) and the canonical BrickColor->Color
/// migration (Roblox stores part colour as Color3uint8; the palette
/// number is an alias, so a BrickColor equals the Color it maps to).
fn values_equivalent(a: &PropValue, b: &PropValue) -> bool {
    use PropValue::*;
    let eps = 1e-4;
    let close = |x: f64, y: f64| (x - y).abs() <= eps + 1e-4 * y.abs();
    match (a, b) {
        (Float32(x), Float64(y)) | (Float64(y), Float32(x)) => close(*x as f64, *y),
        (Int32(x), Int64(y)) | (Int64(y), Int32(x)) => *x as i64 == *y,
        (Color3(x), Color3(y)) => (0..3).all(|i| (x[i] - y[i]).abs() <= 0.0045),
        (BrickColor(n), Color3(c)) | (Color3(c), BrickColor(n)) => {
            // Equal when the palette colour matches, or when the
            // BrickColor is exactly what snapping this Color to the
            // palette produces (Roblox derives one from the other).
            let exact = match rbx_types::BrickColor::from_number(*n as u16) {
                Some(palette) => {
                    let rgb = palette.to_color3uint8();
                    (rgb.r as f32 / 255.0 - c[0]).abs() <= 0.0045
                        && (rgb.g as f32 / 255.0 - c[1]).abs() <= 0.0045
                        && (rgb.b as f32 / 255.0 - c[2]).abs() <= 0.0045
                }
                None => false,
            };
            if exact {
                return true;
            }
            let mut best: Option<(u32, f32)> = None;
            for number in 0..=1032u16 {
                if let Some(palette) = rbx_types::BrickColor::from_number(number) {
                    let rgb = palette.to_color3uint8();
                    let dist = (rgb.r as f32 / 255.0 - c[0]).powi(2)
                        + (rgb.g as f32 / 255.0 - c[1]).powi(2)
                        + (rgb.b as f32 / 255.0 - c[2]).powi(2);
                    if best.map(|(_, d)| dist < d).unwrap_or(true) {
                        best = Some((number as u32, dist));
                    }
                }
            }
            best.map(|(number, _)| number) == Some(*n)
        }
        _ => a == b,
    }
}

fn diff_node(a: &Node, b: &Node, path: &str, out: &mut Vec<DiffEntry>) {
    if a.class_name != b.class_name {
        out.push(DiffEntry {
            kind: "class_changed",
            path: path.to_string(),
            property: None,
            before: Some(a.class_name.clone()),
            after: Some(b.class_name.clone()),
        });
    }
    // Canonical BrickColor->Color migration: when only one side has
    // BrickColor, compare it against the other side's Color instead of
    // reporting add/remove noise.
    let cross_brick_a = a.properties.contains_key("BrickColor")
        && !b.properties.contains_key("BrickColor")
        && b.properties.contains_key("Color");
    let cross_brick_b = b.properties.contains_key("BrickColor")
        && !a.properties.contains_key("BrickColor")
        && a.properties.contains_key("Color");
    if cross_brick_a
        && !values_equivalent(&a.properties["BrickColor"], &b.properties["Color"])
    {
        out.push(DiffEntry {
            kind: "property_changed",
            path: path.to_string(),
            property: Some("BrickColor/Color".to_string()),
            before: Some(a.properties["BrickColor"].display()),
            after: Some(b.properties["Color"].display()),
        });
    }
    if cross_brick_b
        && !values_equivalent(&b.properties["BrickColor"], &a.properties["Color"])
    {
        out.push(DiffEntry {
            kind: "property_changed",
            path: path.to_string(),
            property: Some("BrickColor/Color".to_string()),
            before: Some(a.properties["Color"].display()),
            after: Some(b.properties["BrickColor"].display()),
        });
    }
    for (name, value) in &a.properties {
        if cross_brick_a && name == "BrickColor" {
            continue;
        }
        match b.properties.get(name) {
            None => out.push(DiffEntry {
                kind: "property_removed",
                path: path.to_string(),
                property: Some(name.clone()),
                before: Some(value.display()),
                after: None,
            }),
            Some(other) if !values_equivalent(value, other) => out.push(DiffEntry {
                kind: "property_changed",
                path: path.to_string(),
                property: Some(name.clone()),
                before: Some(value.display()),
                after: Some(other.display()),
            }),
            _ => {}
        }
    }
    for (name, value) in &b.properties {
        if cross_brick_b && name == "BrickColor" {
            continue;
        }
        if !a.properties.contains_key(name) {
            out.push(DiffEntry {
                kind: "property_added",
                path: path.to_string(),
                property: Some(name.clone()),
                before: None,
                after: Some(value.display()),
            });
        }
    }
    let (matched, only_a, only_b) = match_children(a, b);
    for (ca, cb) in matched {
        diff_node(ca, cb, &format!("{path}/{}", ca.name), out);
    }
    for child in only_a {
        out.push(DiffEntry {
            kind: "removed",
            path: format!("{path}/{}", child.name),
            property: None,
            before: Some(format!("{} '{}' ({} instance(s))", child.class_name, child.name, child.count())),
            after: None,
        });
    }
    for child in only_b {
        out.push(DiffEntry {
            kind: "added",
            path: format!("{path}/{}", child.name),
            property: None,
            before: None,
            after: Some(format!("{} '{}' ({} instance(s))", child.class_name, child.name, child.count())),
        });
    }
}

/// Full semantic diff; empty result means the trees are equivalent
/// (modulo referents and child ordering of same-named siblings).
pub fn diff_trees(a: &Node, b: &Node) -> Vec<DiffEntry> {
    let mut out = Vec::new();
    diff_node(a, b, &a.name, &mut out);
    out
}
