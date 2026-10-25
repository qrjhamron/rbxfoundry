//! Conversion between the canonical engine tree ([`Node`]) and `rbx_dom_weak`.
//!
//! Anything BloxCLI cannot represent is a hard error naming the class,
//! property, and type. Data is never silently dropped.

use std::collections::{BTreeMap, HashMap};

use rbx_dom_weak::{types::Ref, InstanceBuilder, WeakDom};
use rbx_types::{
    Attributes, Axes, BinaryString, BrickColor, CFrame, Color3, ColorSequence,
    ColorSequenceKeypoint, ContentId, CustomPhysicalProperties, Enum, EnumItem, Faces, Matrix3,
    Font, FontStyle, FontWeight, NumberRange, NumberSequence, NumberSequenceKeypoint, PhysicalProperties, Ray, Rect, Tags,
    UDim, UDim2, Variant, Vector2, Vector2int16, Vector3, Vector3int16,
};

use crate::protocol::tree::{
    base64_decode, base64_encode, err, CFrameValue, ColorKeypoint, Node, NumberKeypoint, PropValue,
    Result,
};

/// Builds a WeakDom whose root is the canonical tree's root.
pub fn node_to_dom(root: &Node) -> Result<WeakDom> {
    let mut ref_map: HashMap<String, Ref> = HashMap::new();
    let builder = node_to_builder(root, "", &mut ref_map)?;
    let mut dom = WeakDom::new(builder);
    fix_ref_properties(root, root, &mut dom, "", &ref_map)?;
    Ok(dom)
}

/// Second pass after the WeakDom exists: rewrites `Ref` properties (stored in
/// the canonical tree as child-index paths) to real WeakDom referents.
/// Resolves a "name:Workspace/Part/Child" cross-tree reference to the
/// child-index path of the same node in `root`'s tree.
fn resolve_name_ref(root: &Node, target: &str) -> Option<String> {
    let rest = target.strip_prefix("name:")?;
    let mut cur = root;
    let mut index_path = String::new();
    for part in rest.split('/').filter(|s| !s.is_empty()) {
        let idx = cur.children.iter().position(|c| c.name == part)?;
        if !index_path.is_empty() {
            index_path.push('/');
        }
        index_path.push_str(&idx.to_string());
        cur = &cur.children[idx];
    }
    Some(index_path)
}

fn fix_ref_properties(
    root: &Node,
    node: &Node,
    dom: &mut WeakDom,
    index_path: &str,
    ref_map: &HashMap<String, Ref>,
) -> Result<()> {
    let self_ref = *ref_map
        .get(index_path)
        .ok_or_else(|| crate::protocol::tree::Error(format!("internal: no referent for '{index_path}'")))?;
    let ref_props: Vec<(String, String)> = node
        .properties
        .iter()
        .filter_map(|(name, value)| match value {
            PropValue::Ref(target) => Some((name.clone(), target.clone())),
            _ => None,
        })
        .collect();
    for (name, target_path) in ref_props {
        let target_ref = if target_path.is_empty() {
            Ref::none()
        } else if target_path == "." {
            // Self-reference to the tree root (distinct from None).
            *ref_map.get("").ok_or_else(|| {
                crate::protocol::tree::Error("internal: no referent for the root".to_string())
            })?
        } else if let Some(index_path) = resolve_name_ref(root, &target_path) {
            *ref_map.get(&index_path).ok_or_else(|| {
                crate::protocol::tree::Error(format!(
                    "property '{name}' on '{}' references instance '{target_path}', which no longer exists",
                    node.name
                ))
            })?
        } else {
            *ref_map.get(&target_path).ok_or_else(|| {
                crate::protocol::tree::Error(format!(
                    "property '{name}' on '{}' references instance '{target_path}', which no longer exists",
                    node.name
                ))
            })?
        };
        let inst = dom
            .get_by_ref_mut(self_ref)
            .ok_or_else(|| crate::protocol::tree::Error("internal: WeakDom instance missing".into()))?;
        inst.properties
            .insert(name.as_str().into(), Variant::Ref(target_ref));
    }
    for (i, child) in node.children.iter().enumerate() {
        let child_path = join_index(index_path, i);
        fix_ref_properties(root, child, dom, &child_path, ref_map)?;
    }
    Ok(())
}

fn join_index(parent: &str, index: usize) -> String {
    if parent.is_empty() {
        index.to_string()
    } else {
        format!("{parent}/{index}")
    }
}

fn node_to_builder(
    node: &Node,
    index_path: &str,
    ref_map: &mut HashMap<String, Ref>,
) -> Result<InstanceBuilder> {
    let mut builder = InstanceBuilder::new(node.class_name.clone()).with_name(node.name.clone());
    ref_map.insert(index_path.to_string(), builder.referent());
    // Keep the BloxPivot attribute in sync with WorldPivot (see above).
    let mut properties_owned;
    let properties: &BTreeMap<String, PropValue> = if node.class_name == "Model" {
        properties_owned = node.properties.clone();
        let world_pivot = properties_owned.get("WorldPivot").cloned();
        match &world_pivot {
            Some(pivot @ PropValue::CFrame(_)) => {
                let is_identity = *pivot
                    == PropValue::CFrame(CFrameValue::identity_at(0.0, 0.0, 0.0));
                if is_identity {
                    if let Some(PropValue::Attributes(attrs)) =
                        properties_owned.get_mut("Attributes")
                    {
                        attrs.remove("BloxPivot");
                    }
                } else {
                    let entry = properties_owned
                        .entry("Attributes".to_string())
                        .or_insert_with(|| PropValue::Attributes(BTreeMap::new()));
                    if let PropValue::Attributes(attrs) = entry {
                        attrs.insert("BloxPivot".to_string(), pivot.clone());
                    }
                }
            }
            _ => {}
        }
        &properties_owned
    } else {
        &node.properties
    };
    for (name, value) in properties {
        // Ref properties are wired up in fix_ref_properties once every
        // referent exists.
        if matches!(value, PropValue::Ref(_)) {
            continue;
        }
        let variant = prop_to_variant(&node.class_name, name, value)?;
        let variant = coerce_for_reflection(&node.class_name, name, variant);
        builder.add_property(name.clone(), variant);
    }
    for (i, child) in node.children.iter().enumerate() {
        builder.add_child(node_to_builder(child, &join_index(index_path, i), ref_map)?);
    }
    Ok(builder)
}

/// Converts a WeakDom into the canonical tree. The WeakDom root (a DataModel
/// for place files) becomes the canonical root. Canonical referents are
/// assigned in traversal order, matching the engine's load order, so an
/// instance keeps its referent across load/snapshot of an unchanged tree.
pub fn dom_to_node(dom: &WeakDom) -> Result<Node> {
    // Map every WeakDom referent to its child-index path for Ref properties.
    let mut ref_paths: HashMap<Ref, String> = HashMap::new();
    collect_ref_paths(dom, dom.root_ref(), "", &mut ref_paths);
    let mut counter: i64 = 0;
    let mut node = instance_to_node(dom, dom.root(), &ref_paths, &mut counter)?;
    // Place files do not store the DataModel root's name; deserializers
    // synthesize the root with an empty name.
    if node.name.is_empty() {
        node.name = "Place".to_string();
    }
    Ok(node)
}

fn collect_ref_paths(dom: &WeakDom, referent: Ref, index_path: &str, out: &mut HashMap<Ref, String>) {
    out.insert(referent, index_path.to_string());
    if let Some(inst) = dom.get_by_ref(referent) {
        for (i, &child) in inst.children().iter().enumerate() {
            collect_ref_paths(dom, child, &join_index(index_path, i), out);
        }
    }
}

fn instance_to_node(
    dom: &WeakDom,
    inst: &rbx_dom_weak::Instance,
    ref_paths: &HashMap<Ref, String>,
    counter: &mut i64,
) -> Result<Node> {
    *counter += 1;
    let mut node = Node::new(&inst.class.to_string(), &inst.name);
    node.referent = *counter;
    for (name, value) in &inst.properties {
        // Roblox's Content migration: legacy ContentId properties (MeshId,
        // Decal.Texture, Sound.SoundId, ...) serialize under new Content
        // properties (MeshContent, TextureContent, AudioContent, ...).
        // Files written by current Studio therefore decode with Content
        // values under the new names; map them back to the canonical
        // ContentId property so tools see one stable name.
        if let Variant::Content(content) = value {
            let class = inst.class.to_string();
            let serialized_name = name.to_string();
            let canonical =
                migrated_content_canonical(&class, &serialized_name).unwrap_or(serialized_name);
            let prop = match content.as_uri() {
                Some(uri) => PropValue::ContentId(uri.to_string()),
                None if content.as_object().is_none() => PropValue::ContentId(String::new()),
                _ => {
                    return err(format!(
                        "property '{canonical}' on class '{class}' is an object-referencing Content value, which cannot be represented yet; refusing to drop it"
                    ))
                }
            };
            node.properties.insert(canonical, prop);
            continue;
        }
        match variant_to_prop_ref(&inst.class.to_string(), &name.to_string(), value, ref_paths) {
            Ok(prop) => {
                node.properties.insert(name.to_string(), prop);
            }
            Err(e) => return Err(e),
        }
    }
    // Model.WorldPivot is a canonical-only property: Roblox file serializers
    // do not store it. BloxCLI persists an explicitly set pivot in the
    // `BloxPivot` attribute (a plain, Studio-visible CFrame attribute);
    // restore it as WorldPivot here. See COMPATIBILITY.md.
    if node.class_name == "Model" && !node.properties.contains_key("WorldPivot") {
        if let Some(PropValue::Attributes(attrs)) = node.properties.get("Attributes") {
            if let Some(pivot @ PropValue::CFrame(_)) = attrs.get("BloxPivot") {
                node.properties
                    .insert("WorldPivot".to_string(), pivot.clone());
            }
        }
    }
    for &child_ref in inst.children() {
        let child = dom
            .get_by_ref(child_ref)
            .ok_or_else(|| crate::protocol::tree::Error("WeakDom contains a dangling child referent".into()))?;
        node.children.push(instance_to_node(dom, child, ref_paths, counter)?);
    }
    Ok(node)
}

fn variant_to_prop(class: &str, property: &str, value: &Variant) -> Result<PropValue> {
    variant_to_prop_inner(class, property, value, false)
}

/// Like `variant_to_prop`, but resolves `Ref` variants through the
/// child-index path map of the source WeakDom.
fn variant_to_prop_ref(
    class: &str,
    property: &str,
    value: &Variant,
    ref_paths: &HashMap<Ref, String>,
) -> Result<PropValue> {
    if let Variant::Ref(target) = value {
        if target.is_none() {
            return Ok(PropValue::Ref(String::new()));
        }
        return match ref_paths.get(target) {
            // The root's own path is "" (see collect_ref_paths); a
            // reference to it is a distinct self-reference token, not
            // None (which is the empty string).
            Some(path) if path.is_empty() => Ok(PropValue::Ref(".".to_string())),
            Some(path) => Ok(PropValue::Ref(path.clone())),
            None => err(format!(
                "property '{property}' on class '{class}' references an instance outside the place; refusing to drop it"
            )),
        };
    }
    variant_to_prop(class, property, value)
}

fn variant_to_prop_inner(
    class: &str,
    property: &str,
    value: &Variant,
    in_attributes: bool,
) -> Result<PropValue> {
    let ctx = || format!("property '{property}' on class '{class}'");
    Ok(match value {
        Variant::Bool(v) => PropValue::Bool(*v),
        Variant::Int32(v) => PropValue::Int32(*v),
        Variant::Int64(v) => PropValue::Int64(*v),
        Variant::Float32(v) => PropValue::Float32(*v),
        Variant::Float64(v) => PropValue::Float64(*v),
        Variant::String(v) => PropValue::String(v.clone()),
        Variant::BinaryString(v) => {
            let bytes = BinaryString::clone(v).into_vec();
            // In attributes, strings and binary strings share one type ID
            // (0x02) and the decoder yields BinaryString; valid UTF-8 is a
            // string attribute. The serialized bytes are identical either way.
            if in_attributes {
                if let Ok(text) = String::from_utf8(bytes.clone()) {
                    return Ok(PropValue::String(text));
                }
            }
            // `Tags` on a class unknown to reflection decodes as its raw
            // NUL-separated byte encoding; restore the typed form.
            if property == "Tags" {
                if let Ok(tags) = decode_tags(&bytes) {
                    return Ok(PropValue::Tags(tags));
                }
            }
            PropValue::BinaryString(base64_encode(&bytes))
        }
        Variant::SharedString(v) => PropValue::BinaryString(base64_encode(v.data())),
        Variant::Vector3(v) => PropValue::Vector3([v.x, v.y, v.z]),
        Variant::Color3(v) => PropValue::Color3([v.r, v.g, v.b]),
        Variant::Color3uint8(v) => {
            PropValue::Color3([v.r as f32 / 255.0, v.g as f32 / 255.0, v.b as f32 / 255.0])
        }
        Variant::CFrame(cf) => PropValue::CFrame(CFrameValue {
            position: [cf.position.x, cf.position.y, cf.position.z],
            rotation: [
                cf.orientation.x.x, cf.orientation.y.x, cf.orientation.z.x,
                cf.orientation.x.y, cf.orientation.y.y, cf.orientation.z.y,
                cf.orientation.x.z, cf.orientation.y.z, cf.orientation.z.z,
            ],
        }),
        Variant::Enum(v) => PropValue::Enum(v.to_u32()),
        Variant::EnumItem(item) => PropValue::Enum(item.value),
        Variant::BrickColor(v) => PropValue::BrickColor(*v as u16 as u32),
        Variant::ContentId(v) => PropValue::ContentId(v.as_str().to_string()),
        Variant::Tags(v) => PropValue::Tags(v.iter().map(|t| t.to_string()).collect()),
        Variant::Vector2(v) => PropValue::Vector2([v.x, v.y]),
        Variant::Vector2int16(v) => PropValue::Vector2int16([v.x, v.y]),
        Variant::Vector3int16(v) => PropValue::Vector3int16([v.x, v.y, v.z]),
        Variant::UDim(v) => PropValue::UDim([v.scale, v.offset as f32]),
        Variant::UDim2(v) => PropValue::UDim2([v.x.scale, v.x.offset as f32, v.y.scale, v.y.offset as f32]),
        Variant::NumberRange(v) => PropValue::NumberRange([v.min, v.max]),
        Variant::Rect(v) => PropValue::Rect([v.min.x, v.min.y, v.max.x, v.max.y]),
        Variant::Ray(v) => PropValue::Ray {
            origin: [v.origin.x, v.origin.y, v.origin.z],
            direction: [v.direction.x, v.direction.y, v.direction.z],
        },
        Variant::Faces(v) => PropValue::Faces(v.bits() as u32),
        Variant::Axes(v) => PropValue::Axes(v.bits() as u32),
        Variant::PhysicalProperties(v) => match v {
            PhysicalProperties::Default => PropValue::PhysicalProperties(None),
            PhysicalProperties::Custom(c) => PropValue::PhysicalProperties(Some([
                c.density(),
                c.friction(),
                c.elasticity(),
                c.friction_weight(),
                c.elasticity_weight(),
                c.acoustic_absorption(),
            ])),
        },
        Variant::ColorSequence(v) => PropValue::ColorSequence(
            v.keypoints
                .iter()
                .map(|k| ColorKeypoint { time: k.time, color: [k.color.r, k.color.g, k.color.b] })
                .collect(),
        ),
        Variant::NumberSequence(v) => PropValue::NumberSequence(
            v.keypoints
                .iter()
                .map(|k| NumberKeypoint { time: k.time, value: k.value, envelope: k.envelope })
                .collect(),
        ),
        Variant::Font(v) => PropValue::Font {
            family: v.family.clone(),
            weight: v.weight.as_u16(),
            style: match v.style {
                FontStyle::Italic => "Italic".to_string(),
                _ => "Normal".to_string(),
            },
        },
        Variant::Content(v) => match v.as_uri() {
            Some(uri) => PropValue::ContentId(uri.to_string()),
            None if v.as_object().is_none() => PropValue::ContentId(String::new()),
            _ => {
                return err(format!(
                    "{} is an object-referencing Content value, which cannot be represented yet; refusing to drop it",
                    ctx()
                ))
            }
        },
        Variant::Attributes(v) => {
            let mut map = BTreeMap::new();
            for (key, inner) in v.iter() {
                map.insert(
                    key.to_string(),
                    variant_to_prop_inner(class, &format!("{property}.{key}"), inner, true)?,
                );
            }
            PropValue::Attributes(map)
        }
        other => {
            return err(format!(
                "{} has unsupported type {:?} in Phase 1; refusing to drop it",
                ctx(),
                other.ty()
            ))
        }
    })
}

/// Decodes the raw byte form of a Tags value (NUL-separated UTF-8 strings).
fn decode_tags(bytes: &[u8]) -> std::result::Result<Vec<String>, ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    Ok(text
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

pub fn prop_to_variant(class: &str, property: &str, value: &PropValue) -> Result<Variant> {
    Ok(match value {
        PropValue::Bool(v) => Variant::Bool(*v),
        PropValue::Int32(v) => Variant::Int32(*v),
        PropValue::Int64(v) => Variant::Int64(*v),
        PropValue::Float32(v) => Variant::Float32(*v),
        PropValue::Float64(v) => Variant::Float64(*v),
        PropValue::String(v) => Variant::String(v.clone()),
        PropValue::BinaryString(v) => Variant::BinaryString(BinaryString::from(base64_decode(v)?)),
        PropValue::Vector3(v) => Variant::Vector3(Vector3::new(v[0], v[1], v[2])),
        PropValue::Color3(v) => Variant::Color3(Color3::new(v[0], v[1], v[2])),
        PropValue::CFrame(cf) => Variant::CFrame(CFrame::new(
            Vector3::new(cf.position[0], cf.position[1], cf.position[2]),
            Matrix3::new(
                Vector3::new(cf.rotation[0], cf.rotation[3], cf.rotation[6]),
                Vector3::new(cf.rotation[1], cf.rotation[4], cf.rotation[7]),
                Vector3::new(cf.rotation[2], cf.rotation[5], cf.rotation[8]),
            ),
        )),
        PropValue::Enum(v) => Variant::Enum(Enum::from_u32(*v)),
        PropValue::BrickColor(v) => {
            let number = u16::try_from(*v)
                .map_err(|_| crate::protocol::tree::Error(format!("BrickColor number {v} out of range")))?;
            match BrickColor::from_number(number) {
                Some(bc) => Variant::BrickColor(bc),
                None => {
                    return err(format!(
                        "BrickColor number {v} on '{class}' property '{property}' is not a known palette color"
                    ))
                }
            }
        }
        PropValue::ContentId(v) => Variant::ContentId(ContentId::from(v.clone())),
        PropValue::Tags(v) => Variant::Tags(Tags::from(v.clone())),
        PropValue::Vector2(v) => Variant::Vector2(Vector2::new(v[0], v[1])),
        PropValue::Vector2int16(v) => Variant::Vector2int16(Vector2int16::new(v[0], v[1])),
        PropValue::Vector3int16(v) => Variant::Vector3int16(Vector3int16::new(v[0], v[1], v[2])),
        PropValue::UDim(v) => Variant::UDim(UDim::new(v[0], v[1] as i32)),
        PropValue::UDim2(v) => Variant::UDim2(UDim2::new(
            UDim::new(v[0], v[1] as i32),
            UDim::new(v[2], v[3] as i32),
        )),
        PropValue::NumberRange(v) => Variant::NumberRange(NumberRange::new(v[0], v[1])),
        PropValue::Rect(v) => Variant::Rect(Rect::new(
            Vector2::new(v[0], v[1]),
            Vector2::new(v[2], v[3]),
        )),
        PropValue::Ray { origin, direction } => Variant::Ray(Ray::new(
            Vector3::new(origin[0], origin[1], origin[2]),
            Vector3::new(direction[0], direction[1], direction[2]),
        )),
        PropValue::Faces(v) => match Faces::from_bits(*v as u8) {
            Some(f) => Variant::Faces(f),
            None => return err(format!("invalid Faces bitmask {v} on '{class}' property '{property}'")),
        },
        PropValue::Axes(v) => match Axes::from_bits(*v as u8) {
            Some(a) => Variant::Axes(a),
            None => return err(format!("invalid Axes bitmask {v} on '{class}' property '{property}'")),
        },
        PropValue::PhysicalProperties(v) => match v {
            None => Variant::PhysicalProperties(PhysicalProperties::Default),
            Some(p) => Variant::PhysicalProperties(PhysicalProperties::Custom(
                CustomPhysicalProperties::new(p[0], p[1], p[2], p[3], p[4], p[5]),
            )),
        },
        PropValue::ColorSequence(v) => Variant::ColorSequence(ColorSequence {
            keypoints: v
                .iter()
                .map(|k| ColorSequenceKeypoint::new(k.time, Color3::new(k.color[0], k.color[1], k.color[2])))
                .collect(),
        }),
        PropValue::NumberSequence(v) => Variant::NumberSequence(NumberSequence {
            keypoints: v
                .iter()
                .map(|k| NumberSequenceKeypoint::new(k.time, k.value, k.envelope))
                .collect(),
        }),
        PropValue::Font { family, weight, style } => Variant::Font(Font::new(
            family,
            FontWeight::from_u16(*weight).unwrap_or(FontWeight::Regular),
            if style == "Italic" { FontStyle::Italic } else { FontStyle::Normal },
        )),
        PropValue::EnumItem { enum_type, value } => {
            Variant::EnumItem(EnumItem { ty: enum_type.clone(), value: *value })
        }
        PropValue::Ref(_) => {
            // Wired up by fix_ref_properties; a placeholder is never stored
            // because node_to_builder skips Ref properties entirely.
            return err("internal: Ref property reached prop_to_variant".to_string());
        }
        PropValue::Attributes(v) => {
            let mut attrs = Attributes::new();
            for (key, inner) in v {
                attrs.insert(key.clone(), prop_to_variant(class, property, inner)?);
            }
            Variant::Attributes(attrs)
        }
    })
}

/// `rbx_binary`/`rbx_xml` serialize known properties using the type from
/// Roblox's reflection database and reject mismatches. Coerce numeric
/// variants (Float64 <-> Float32, Int64 <-> Int32) to the expected type so
/// engine values always serialize. Unknown classes/properties pass through.
fn coerce_for_reflection(class: &str, property: &str, value: Variant) -> Variant {
    use rbx_types::VariantType;
    let Some(expected) = reflection_type(class, property) else {
        return value;
    };
    match (expected, value) {
        (VariantType::Float32, Variant::Float64(v)) => Variant::Float32(v as f32),
        (VariantType::Float64, Variant::Float32(v)) => Variant::Float64(v as f64),
        (VariantType::Int32, Variant::Int64(v)) => Variant::Int32(v as i32),
        (VariantType::Int64, Variant::Int32(v)) => Variant::Int64(v as i64),
        (VariantType::Enum, Variant::EnumItem(item)) => Variant::Enum(Enum::from_u32(item.value)),
        (VariantType::Content, Variant::ContentId(value)) => {
            Variant::Content(rbx_types::Content::from_uri(value.as_str().to_string()))
        }
        (VariantType::ContentId, Variant::Content(value)) => match value.as_uri() {
            Some(uri) => Variant::ContentId(ContentId::from(uri.to_string())),
            None => Variant::Content(value),
        },
        (_, v) => v,
    }
}

/// If `serialized_name` is the Content property that a canonical ContentId
/// property migrates to (e.g. MeshContent for MeshId), returns the canonical
/// property name. Walks superclasses through the reflection database.
pub fn migrated_content_canonical(class: &str, serialized_name: &str) -> Option<String> {
    let db = rbx_reflection_database::get().ok()?;
    let mut current = Some(class);
    while let Some(name) = current {
        let desc = db.classes.get(name)?;
        for (prop_name, prop) in &desc.properties {
            if prop.data_type.ty() != rbx_types::VariantType::ContentId {
                continue;
            }
            if let rbx_reflection::PropertyKind::Canonical {
                serialization: rbx_reflection::PropertySerialization::Migrate(migration),
            } = &prop.kind
            {
                if migration.new_property_names().contains(&serialized_name) {
                    return Some(prop_name.to_string());
                }
            }
        }
        current = desc.superclass.as_deref();
    }
    None
}

/// Looks up the reflection type for a property, walking superclasses.
pub fn reflection_type(class: &str, property: &str) -> Option<rbx_types::VariantType> {
    let db = rbx_reflection_database::get().ok()?;
    let mut current = Some(class);
    while let Some(name) = current {
        let desc = db.classes.get(name)?;
        if let Some(prop) = desc.properties.get(property) {
            return Some(prop.data_type.ty());
        }
        current = desc.superclass.as_deref();
    }
    None
}

/// Whether Roblox reflection knows this class at all.
pub fn reflection_has_class(class: &str) -> bool {
    match rbx_reflection_database::get() {
        Ok(db) => db.classes.contains_key(class),
        Err(_) => false,
    }
}

#[derive(Debug, Clone)]
pub struct ReflectIssue {
    pub severity: &'static str, // "error" | "warning"
    pub path: String,
    pub message: String,
}

/// Validates a canonical tree against Roblox's official reflection database
/// (rbx_reflection_database, API dump 741): every property on a known class
/// must have the reflected type. Unknown classes/properties are warnings —
/// their data is preserved, never rejected.
pub fn validate_reflection(root: &Node) -> Vec<ReflectIssue> {
    let mut issues = Vec::new();
    validate_node(root, "game", &mut issues);
    issues
}

fn validate_node(node: &Node, path: &str, issues: &mut Vec<ReflectIssue>) {
    let known_class = reflection_has_class(&node.class_name);
    if !known_class {
        issues.push(ReflectIssue {
            severity: "warning",
            path: path.to_string(),
            message: format!("unknown class '{}' (preserved as-is)", node.class_name),
        });
    } else {
        for (name, value) in &node.properties {
            if name == "Name" {
                continue;
            }
            match reflection_type(&node.class_name, name) {
                Some(expected) => {
                    if !types_compatible(expected, value) {
                        issues.push(ReflectIssue {
                            severity: "error",
                            path: path.to_string(),
                            message: format!(
                                "property '{name}' on {} expects {:?}, found {}",
                                node.class_name,
                                expected,
                                value.type_name()
                            ),
                        });
                    }
                }
                None => {
                    // Inherited service/root instances legitimately carry no
                    // extra reflected properties; only warn for real classes
                    // with property tables.
                    issues.push(ReflectIssue {
                        severity: "warning",
                        path: path.to_string(),
                        message: format!(
                            "unknown property '{name}' on class {} (preserved as-is)",
                            node.class_name
                        ),
                    });
                }
            }
        }
    }
    for child in &node.children {
        let child_path = if path == "game" {
            child.name.clone()
        } else {
            format!("{path}/{}", child.name)
        };
        validate_node(child, &child_path, issues);
    }
}

fn types_compatible(expected: rbx_types::VariantType, value: &PropValue) -> bool {
    use rbx_types::VariantType as T;
    let actual = value.type_name();
    let expected_name = format!("{expected:?}");
    if actual == expected_name {
        return true;
    }
    let int_pair = matches!(expected, T::Int32 | T::Int64) && matches!(actual, "Int32" | "Int64");
    let float_pair =
        matches!(expected, T::Float32 | T::Float64) && matches!(actual, "Float32" | "Float64");
    if int_pair || float_pair {
        return true;
    }
    match expected {
        T::Enum => matches!(actual, "Enum" | "EnumItem"),
        T::Color3uint8 => actual == "Color3",
        T::String => matches!(actual, "String" | "BinaryString"),
        T::ContentId => matches!(actual, "ContentId" | "String"),
        T::BinaryString => matches!(actual, "BinaryString" | "String" | "Tags" | "Attributes"),
        T::Ref => actual == "Ref",
        _ => false,
    }
}
