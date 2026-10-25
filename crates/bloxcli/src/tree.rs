//! Canonical tree JSON shared between the Rust CLI and the C++ engine.
//! This is the IPC contract (protocol "1"); field names must match
//! `engine/include/blox/variant.hpp` and `datamodel.cpp`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Error {}

pub fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum PropValue {
    Bool(bool),
    Int32(i32),
    Int64(i64),
    Float32(f32),
    Float64(f64),
    String(String),
    /// Base64-encoded bytes.
    BinaryString(String),
    Vector3([f32; 3]),
    Color3([f32; 3]),
    CFrame(CFrameValue),
    Enum(u32),
    BrickColor(u32),
    ContentId(String),
    Tags(Vec<String>),
    Attributes(BTreeMap<String, PropValue>),
    // Phase 2 extensions. JSON shapes match engine/include/blox/variant.hpp.
    Vector2([f32; 2]),
    Vector2int16([i16; 2]),
    Vector3int16([i16; 3]),
    /// [scale, offset]
    UDim([f32; 2]),
    /// [xScale, xOffset, yScale, yOffset]
    UDim2([f32; 4]),
    /// [min, max]
    NumberRange([f32; 2]),
    /// [x0, y0, x1, y1]
    Rect([f32; 4]),
    Ray {
        origin: [f32; 3],
        direction: [f32; 3],
    },
    /// Bitmask: Right=1, Top=2, Back=4, Left=8, Bottom=16, Front=32.
    Faces(u32),
    /// Bitmask: X=1, Y=2, Z=4.
    Axes(u32),
    /// `None` is Roblox's `Default`; otherwise [density, friction, elasticity,
    /// frictionWeight, elasticityWeight, acousticAbsorption].
    #[serde(
        serialize_with = "ser_physical_properties",
        deserialize_with = "de_physical_properties"
    )]
    PhysicalProperties(Option<[f32; 6]>),
    ColorSequence(Vec<ColorKeypoint>),
    NumberSequence(Vec<NumberKeypoint>),
    EnumItem {
        #[serde(rename = "enum")]
        enum_type: String,
        value: u32,
    },
    /// Child-index path of the target instance ("0/2/1"); "" means
    /// none; "." is a self-reference to the tree root (serializable in
    /// model files, whose root is stored; in place files the DataModel
    /// root is not stored, so "." only survives canonical round trips).
    Ref(String),
    Font {
        family: String,
        weight: u16,
        style: String,
    },
}

/// Wire form for PhysicalProperties: the string "Default", or a 6-number
/// array for Custom. Matches the C++ engine's Variant JSON.
fn ser_physical_properties<S: serde::Serializer>(
    value: &Option<[f32; 6]>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    match value {
        None => serializer.serialize_str("Default"),
        Some(arr) => arr.serialize(serializer),
    }
}

fn de_physical_properties<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<[f32; 6]>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Text(String),
        Numbers([f32; 6]),
    }
    match Wire::deserialize(deserializer)? {
        Wire::Text(text) if text == "Default" => Ok(None),
        Wire::Text(other) => Err(serde::de::Error::custom(format!(
            "invalid PhysicalProperties value '{other}'"
        ))),
        Wire::Numbers(arr) => Ok(Some(arr)),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorKeypoint {
    pub time: f32,
    pub color: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumberKeypoint {
    pub time: f32,
    pub value: f32,
    pub envelope: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFrameValue {
    pub position: [f32; 3],
    /// Row-major rotation basis: [x.x, x.y, x.z, y.x, ...] columns are axes.
    pub rotation: [f32; 9],
}

impl CFrameValue {
    pub fn identity_at(x: f32, y: f32, z: f32) -> Self {
        Self {
            position: [x, y, z],
            rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl PropValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            PropValue::Bool(_) => "Bool",
            PropValue::Int32(_) => "Int32",
            PropValue::Int64(_) => "Int64",
            PropValue::Float32(_) => "Float32",
            PropValue::Float64(_) => "Float64",
            PropValue::String(_) => "String",
            PropValue::BinaryString(_) => "BinaryString",
            PropValue::Vector3(_) => "Vector3",
            PropValue::Color3(_) => "Color3",
            PropValue::CFrame(_) => "CFrame",
            PropValue::Enum(_) => "Enum",
            PropValue::BrickColor(_) => "BrickColor",
            PropValue::ContentId(_) => "ContentId",
            PropValue::Tags(_) => "Tags",
            PropValue::Attributes(_) => "Attributes",
            PropValue::Vector2(_) => "Vector2",
            PropValue::Vector2int16(_) => "Vector2int16",
            PropValue::Vector3int16(_) => "Vector3int16",
            PropValue::UDim(_) => "UDim",
            PropValue::UDim2(_) => "UDim2",
            PropValue::NumberRange(_) => "NumberRange",
            PropValue::Rect(_) => "Rect",
            PropValue::Ray { .. } => "Ray",
            PropValue::Faces(_) => "Faces",
            PropValue::Axes(_) => "Axes",
            PropValue::PhysicalProperties(_) => "PhysicalProperties",
            PropValue::ColorSequence(_) => "ColorSequence",
            PropValue::NumberSequence(_) => "NumberSequence",
            PropValue::EnumItem { .. } => "EnumItem",
            PropValue::Ref(_) => "Ref",
            PropValue::Font { .. } => "Font",
        }
    }

    /// Short human-readable rendering used by `inspect`.
    pub fn display(&self) -> String {
        match self {
            PropValue::Bool(v) => v.to_string(),
            PropValue::Int32(v) => v.to_string(),
            PropValue::Int64(v) => v.to_string(),
            PropValue::Float32(v) => format!("{v}"),
            PropValue::Float64(v) => format!("{v}"),
            PropValue::String(v) => format!("{v:?}"),
            PropValue::BinaryString(v) => format!("<binary, {} base64 chars>", v.len()),
            PropValue::Vector3(v) => format!("({}, {}, {})", v[0], v[1], v[2]),
            PropValue::Color3(v) => format!("({}, {}, {})", v[0], v[1], v[2]),
            PropValue::CFrame(cf) => format!(
                "({}, {}, {}; rot [{}, {}, {}, {}, {}, {}, {}, {}, {}])",
                cf.position[0], cf.position[1], cf.position[2],
                cf.rotation[0], cf.rotation[1], cf.rotation[2], cf.rotation[3],
                cf.rotation[4], cf.rotation[5], cf.rotation[6], cf.rotation[7], cf.rotation[8]
            ),
            PropValue::Enum(v) => v.to_string(),
            PropValue::BrickColor(v) => v.to_string(),
            PropValue::ContentId(v) => format!("{v:?}"),
            PropValue::Tags(v) => format!("{:?}", v),
            PropValue::Attributes(v) => format!("{{{} attributes}}", v.len()),
            PropValue::Vector2(v) => format!("({}, {})", v[0], v[1]),
            PropValue::Vector2int16(v) => format!("({}, {})", v[0], v[1]),
            PropValue::Vector3int16(v) => format!("({}, {}, {})", v[0], v[1], v[2]),
            PropValue::UDim(v) => format!("({}, {})", v[0], v[1]),
            PropValue::UDim2(v) => format!("({}, {}), ({}, {})", v[0], v[1], v[2], v[3]),
            PropValue::NumberRange(v) => format!("{} .. {}", v[0], v[1]),
            PropValue::Rect(v) => format!("({}, {}), ({}, {})", v[0], v[1], v[2], v[3]),
            PropValue::Ray { origin, direction } => {
                format!("origin ({}, {}, {}), direction ({}, {}, {})", origin[0], origin[1], origin[2], direction[0], direction[1], direction[2])
            }
            PropValue::Faces(v) => format!("Faces({v})"),
            PropValue::Axes(v) => format!("Axes({v})"),
            PropValue::PhysicalProperties(None) => "Default".to_string(),
            PropValue::PhysicalProperties(Some(v)) => format!(
                "Custom(density {}, friction {}, elasticity {}, frictionWeight {}, elasticityWeight {}, acousticAbsorption {})",
                v[0], v[1], v[2], v[3], v[4], v[5]
            ),
            PropValue::ColorSequence(v) => format!("ColorSequence({} keypoints)", v.len()),
            PropValue::NumberSequence(v) => format!("NumberSequence({} keypoints)", v.len()),
            PropValue::EnumItem { enum_type, value } => format!("Enum.{enum_type}:{value}"),
            PropValue::Ref(v) => {
                if v.is_empty() { "None".to_string() } else { format!("ref({v})") }
            }
            PropValue::Font { family, weight, style } => format!("Font({family}, {weight}, {style})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub referent: i64,
    #[serde(rename = "className")]
    pub class_name: String,
    pub name: String,
    #[serde(default)]
    pub properties: BTreeMap<String, PropValue>,
    #[serde(default)]
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(class_name: &str, name: &str) -> Self {
        Self {
            referent: 0,
            class_name: class_name.to_string(),
            name: name.to_string(),
            properties: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    pub fn count(&self) -> usize {
        1 + self.children.iter().map(|c| c.count()).sum::<usize>()
    }

    /// Finds a descendant by name path ("Workspace/Baseplate"); the first
    /// segment may be this node's own name, "game", or a child name.
    /// Ambiguous sibling names are an error, mirroring the engine.
    pub fn find_path(&self, path: &str) -> Result<&Node> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut cur = self;
        let mut iter = parts.iter().peekable();
        if let Some(first) = iter.peek() {
            if **first == "game" || **first == self.name.as_str() || **first == self.class_name.as_str() {
                iter.next();
            }
        }
        for part in iter {
            let matches: Vec<&Node> = cur.children.iter().filter(|c| &c.name == part).collect();
            match matches.len() {
                0 => return err(format!("no instance named '{part}' under '{}'", cur.name)),
                1 => cur = matches[0],
                n => {
                    return err(format!(
                        "path '{path}' is ambiguous: {n} siblings named '{part}'"
                    ))
                }
            }
        }
        Ok(cur)
    }

    pub fn find_path_mut(&mut self, path: &str) -> Result<&mut Node> {
        // Re-implemented iteratively because of borrow rules.
        let parts: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let mut start = 0;
        if let Some(first) = parts.first() {
            if first == "game" || *first == self.name || *first == self.class_name {
                start = 1;
            }
        }
        let mut cur = self;
        for part in &parts[start..] {
            let idxs: Vec<usize> = cur
                .children
                .iter()
                .enumerate()
                .filter(|(_, c)| &c.name == part)
                .map(|(i, _)| i)
                .collect();
            match idxs.len() {
                0 => return err(format!("no instance named '{part}' under '{}'", cur.name)),
                1 => cur = &mut cur.children[idxs[0]],
                n => {
                    return err(format!(
                        "path '{path}' is ambiguous: {n} siblings named '{part}'"
                    ))
                }
            }
        }
        Ok(cur)
    }
}

/// Parses the `--value` string form of a property using the declared type.
/// Grammar: Bool `true|false`; numbers decimal; Vector3/Color3 `x,y,z`;
/// CFrame `x,y,z` (identity rotation) or 12 comma-separated numbers
/// (position + 9 rotation); Enum integer or a known Material name;
/// Tags `a,b,c`; String/ContentId raw text.
pub fn parse_value(type_name: &str, text: &str, property: &str) -> Result<PropValue> {
    fn nums(text: &str, n: usize) -> Result<Vec<f32>> {
        let parts: Vec<&str> = text.split(',').map(|s| s.trim()).collect();
        if parts.len() != n {
            return err(format!("expected {n} comma-separated numbers, got {}", parts.len()));
        }
        parts
            .iter()
            .map(|p| {
                p.parse::<f32>()
                    .map_err(|_| Error(format!("'{p}' is not a number")))
            })
            .collect()
    }
    match type_name {
        "Bool" => match text {
            "true" => Ok(PropValue::Bool(true)),
            "false" => Ok(PropValue::Bool(false)),
            _ => err("Bool value must be 'true' or 'false'"),
        },
        "Int32" => text
            .parse::<i32>()
            .map(PropValue::Int32)
            .map_err(|_| Error(format!("'{text}' is not an Int32"))),
        "Int64" => text
            .parse::<i64>()
            .map(PropValue::Int64)
            .map_err(|_| Error(format!("'{text}' is not an Int64"))),
        "Float32" => text
            .parse::<f32>()
            .map(PropValue::Float32)
            .map_err(|_| Error(format!("'{text}' is not a Float32"))),
        "Float64" => text
            .parse::<f64>()
            .map(PropValue::Float64)
            .map_err(|_| Error(format!("'{text}' is not a Float64"))),
        "String" => Ok(PropValue::String(text.to_string())),
        "ContentId" => Ok(PropValue::ContentId(text.to_string())),
        "Vector3" => {
            let v = nums(text, 3)?;
            Ok(PropValue::Vector3([v[0], v[1], v[2]]))
        }
        "Color3" => {
            let v = nums(text, 3)?;
            Ok(PropValue::Color3([v[0], v[1], v[2]]))
        }
        "CFrame" => {
            if text.contains(',') && text.matches(',').count() == 11 {
                let v = nums(text, 12)?;
                Ok(PropValue::CFrame(CFrameValue {
                    position: [v[0], v[1], v[2]],
                    rotation: [v[3], v[4], v[5], v[6], v[7], v[8], v[9], v[10], v[11]],
                }))
            } else {
                let v = nums(text, 3)?;
                Ok(PropValue::CFrame(CFrameValue::identity_at(v[0], v[1], v[2])))
            }
        }
        "Enum" => {
            if let Ok(n) = text.parse::<u32>() {
                return Ok(PropValue::Enum(n));
            }
            if property == "Material" {
                if let Some(n) = material_value(text) {
                    return Ok(PropValue::Enum(n));
                }
            }
            err(format!(
                "'{text}' is not a valid Enum value (use a number{})",
                if property == "Material" { " or a Material name" } else { "" }
            ))
        }
        "BrickColor" => text
            .parse::<u32>()
            .map(PropValue::BrickColor)
            .map_err(|_| Error(format!("'{text}' is not a BrickColor number"))),
        "Tags" => Ok(PropValue::Tags(
            text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        )),
        "Vector2" => {
            let v = nums(text, 2)?;
            Ok(PropValue::Vector2([v[0], v[1]]))
        }
        "UDim" => {
            let v = nums(text, 2)?;
            Ok(PropValue::UDim([v[0], v[1]]))
        }
        "UDim2" => {
            let v = nums(text, 4)?;
            Ok(PropValue::UDim2([v[0], v[1], v[2], v[3]]))
        }
        "NumberRange" => {
            let v = nums(text, 2)?;
            Ok(PropValue::NumberRange([v[0], v[1]]))
        }
        "Rect" => {
            let v = nums(text, 4)?;
            Ok(PropValue::Rect([v[0], v[1], v[2], v[3]]))
        }
        "Faces" => text
            .parse::<u32>()
            .map(PropValue::Faces)
            .map_err(|_| Error(format!("'{text}' is not a Faces bitmask (Right=1, Top=2, Back=4, Left=8, Bottom=16, Front=32)"))),
        "Axes" => text
            .parse::<u32>()
            .map(PropValue::Axes)
            .map_err(|_| Error(format!("'{text}' is not an Axes bitmask (X=1, Y=2, Z=4)"))),
        "PhysicalProperties" => {
            if text == "Default" {
                Ok(PropValue::PhysicalProperties(None))
            } else {
                let v = nums(text, 6).or_else(|_| nums(text, 5))?;
                let mut arr = [0.0f32; 6];
                arr[..v.len()].copy_from_slice(&v);
                Ok(PropValue::PhysicalProperties(Some(arr)))
            }
        }
        "ColorSequence" => {
            // Grammar: "time:r,g,b; time:r,g,b"
            let mut keypoints = Vec::new();
            for part in text.split(';') {
                let (t, rgb) = part
                    .split_once(':')
                    .ok_or_else(|| Error("ColorSequence entries must be 'time:r,g,b'".to_string()))?;
                let v = nums(rgb.trim(), 3)?;
                keypoints.push(ColorKeypoint {
                    time: t.trim().parse().map_err(|_| Error(format!("'{t}' is not a number")))?,
                    color: [v[0], v[1], v[2]],
                });
            }
            Ok(PropValue::ColorSequence(keypoints))
        }
        "NumberSequence" => {
            // Grammar: "time:value,envelope; time:value,envelope"
            let mut keypoints = Vec::new();
            for part in text.split(';') {
                let (t, rest) = part
                    .split_once(':')
                    .ok_or_else(|| Error("NumberSequence entries must be 'time:value,envelope'".to_string()))?;
                let v = nums(rest.trim(), 2)?;
                keypoints.push(NumberKeypoint {
                    time: t.trim().parse().map_err(|_| Error(format!("'{t}' is not a number")))?,
                    value: v[0],
                    envelope: v[1],
                });
            }
            Ok(PropValue::NumberSequence(keypoints))
        }
        "Font" => {
            // Grammar: "family" or "family:weight" (weight 100..900).
            if let Some((family, weight)) = text.split_once(':') {
                let weight: u16 = weight.trim().parse()
                    .map_err(|_| Error(format!("'{weight}' is not a font weight")))?;
                Ok(PropValue::Font { family: family.to_string(), weight, style: "Normal".to_string() })
            } else {
                Ok(PropValue::Font { family: text.to_string(), weight: 400, style: "Normal".to_string() })
            }
        }
        "EnumItem" => {
            // Grammar: "EnumType:value" or just a number (type unknown).
            if let Some((ty, val)) = text.split_once(':') {
                let value = val
                    .parse::<u32>()
                    .map_err(|_| Error(format!("'{val}' is not an enum value")))?;
                Ok(PropValue::EnumItem { enum_type: ty.to_string(), value })
            } else {
                text.parse::<u32>()
                    .map(|value| PropValue::EnumItem { enum_type: String::new(), value })
                    .map_err(|_| Error(format!("'{text}' is not an EnumItem ('EnumType:value')")))
            }
        }
        other => err(format!("setting values of type '{other}' from the CLI text grammar is not supported; use --json via a file-based workflow or a different type")),
    }
}

/// Parses "x,y,z" text into three f32s (used by the transform commands).
pub fn parse_vec3(text: &str) -> Result<[f32; 3]> {
    let parts: Vec<&str> = text.split(',').map(|s| s.trim()).collect();
    if parts.len() != 3 {
        return err(format!("expected 'x,y,z', got '{text}'"));
    }
    let mut out = [0.0f32; 3];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p
            .parse::<f32>()
            .map_err(|_| Error(format!("'{p}' is not a number")))?;
    }
    Ok(out)
}

/// Roblox `Enum.Material` values for the names accepted by `set`.
fn material_value(name: &str) -> Option<u32> {
    Some(match name {
        "Plastic" => 256,
        "SmoothPlastic" => 272,
        "Neon" => 288,
        "Wood" => 512,
        "WoodPlanks" => 528,
        "Marble" => 784,
        "Basalt" => 788,
        "Slate" => 800,
        "CrackedLava" => 804,
        "Concrete" => 816,
        "Limestone" => 820,
        "Granite" => 832,
        "Pavement" => 836,
        "Brick" => 848,
        "Pebble" => 864,
        "Cobblestone" => 880,
        "Rock" => 896,
        "Sandstone" => 912,
        "CorrodedMetal" => 1040,
        "DiamondPlate" => 1056,
        "Foil" => 1072,
        "Metal" => 1088,
        "Grass" => 1280,
        "LeafyGrass" => 1284,
        "Sand" => 1296,
        "Fabric" => 1312,
        "Snow" => 1328,
        "Mud" => 1344,
        "Ground" => 1360,
        "Asphalt" => 1376,
        "Salt" => 1392,
        "Ice" => 1536,
        "Glacier" => 1540,
        "Glass" => 1568,
        "ForceField" => 1584,
        "Air" => 1792,
        "Water" => 2048,
        _ => return None,
    })
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let mut vals = Vec::new();
    for c in text.chars() {
        if c == '=' {
            vals.push(None);
        } else if c.is_whitespace() {
            continue;
        } else if let Some(i) = B64.iter().position(|&b| b == c as u8) {
            vals.push(Some(i as u8));
        } else {
            return err(format!("invalid base64 character '{c}'"));
        }
    }
    if vals.len() % 4 != 0 {
        return err("invalid base64 length".to_string());
    }
    let mut out = Vec::new();
    for chunk in vals.chunks(4) {
        let a = chunk[0].unwrap_or(0) as u32;
        let b = chunk[1].unwrap_or(0) as u32;
        let c = chunk[2].unwrap_or(0) as u32;
        let d = chunk[3].unwrap_or(0) as u32;
        let n = (a << 18) | (b << 12) | (c << 6) | d;
        out.push((n >> 16) as u8);
        if chunk[2].is_some() {
            out.push((n >> 8) as u8);
        }
        if chunk[3].is_some() {
            out.push(n as u8);
        }
    }
    Ok(out)
}
