//! Place template registry (Phase 13).
//!
//! Templates are applied through the engine (the DataModel stays the
//! source of truth). `empty` is the historical BloxCLI default: the
//! standard service set and nothing else. `baseplate` and
//! `classic-baseplate` reproduce the *structure* of Roblox's templates
//! (Baseplate + SpawnLocation + Terrain + Camera). Their exact Studio
//! property values are UNTESTED — no genuine Studio golden fixture is
//! available in this environment, and the values used here (documented
//! per template in `inspect`) must not be read as verified Studio
//! parity. See COMPATIBILITY.md.

use crate::engine::EngineClient;
use crate::tree::{CFrameValue, PropValue, Result};

pub struct TemplateInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub status: &'static str,
    pub contents: &'static [&'static str],
}

pub fn registry() -> Vec<TemplateInfo> {
    vec![
        TemplateInfo {
            name: "empty",
            description: "Standard service set only (Workspace, Lighting, ReplicatedStorage, ServerStorage, ServerScriptService, StarterGui, StarterPlayer, StarterPack, Teams, SoundService). The default.",
            status: "INTERNAL_VERIFIED (default `blox new` behaviour; covered by the whole test suite)",
            contents: &["services only"],
        },
        TemplateInfo {
            name: "baseplate",
            description: "Modern Baseplate structure: Terrain, Camera, a large Baseplate part whose top is at Y=0, and a centred SpawnLocation.",
            status: "UNTESTED vs Studio (no golden fixture; dimensions/colours are the widely published template values, not measured Studio data)",
            contents: &[
                "Workspace/Terrain (empty voxel terrain)",
                "Workspace/Camera",
                "Workspace/Baseplate (Part 512x16x512, top at Y=0, Anchored)",
                "Workspace/SpawnLocation (12x1x12 at origin, Anchored, Neutral)",
            ],
        },
        TemplateInfo {
            name: "classic-baseplate",
            description: "Classic baseplate structure: same layout as `baseplate` with a studded top surface on the Baseplate part.",
            status: "UNTESTED vs Studio (TopSurface=Studs enum value and colours are unverified approximations)",
            contents: &[
                "Workspace/Terrain (empty voxel terrain)",
                "Workspace/Camera",
                "Workspace/Baseplate (Part 512x16x512, TopSurface=Studs, Anchored)",
                "Workspace/SpawnLocation (12x1x12 at origin, Anchored, Neutral)",
            ],
        },
    ]
}

pub fn find(name: &str) -> Option<TemplateInfo> {
    registry().into_iter().find(|t| t.name == name)
}

fn identity_rotation() -> [f32; 9] {
    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
}

fn cframe(x: f32, y: f32, z: f32) -> PropValue {
    PropValue::CFrame(CFrameValue {
        position: [x, y, z],
        rotation: identity_rotation(),
    })
}

/// Applies a template to a freshly created DataModel in the engine.
/// The standard services must already exist (cmd_new creates them).
pub fn apply(engine: &mut EngineClient, template: &str) -> Result<()> {
    match template {
        "empty" => Ok(()),
        "baseplate" | "classic-baseplate" => {
            engine.add("Workspace", "Terrain", "Terrain")?;
            engine.add("Workspace", "Camera", "Camera")?;
            engine.add("Workspace", "Part", "Baseplate")?;
            engine.set("Workspace/Baseplate", "Size", &PropValue::Vector3([512.0, 16.0, 512.0]))?;
            engine.set("Workspace/Baseplate", "CFrame", &cframe(0.0, -8.0, 0.0))?;
            engine.set("Workspace/Baseplate", "Anchored", &PropValue::Bool(true))?;
            engine.set("Workspace/Baseplate", "Color", &PropValue::Color3([0.356, 0.345, 0.356]))?;
            engine.set("Workspace/Baseplate", "Material", &PropValue::Enum(256))?; // Plastic
            if template == "classic-baseplate" {
                // Enum.SurfaceType.Studs (unverified value, see status).
                engine.set("Workspace/Baseplate", "TopSurface", &PropValue::Enum(1))?;
            }
            engine.add("Workspace", "SpawnLocation", "SpawnLocation")?;
            engine.set("Workspace/SpawnLocation", "Size", &PropValue::Vector3([12.0, 1.0, 12.0]))?;
            engine.set("Workspace/SpawnLocation", "CFrame", &cframe(0.0, 0.5, 0.0))?;
            engine.set("Workspace/SpawnLocation", "Anchored", &PropValue::Bool(true))?;
            engine.set("Workspace/SpawnLocation", "Neutral", &PropValue::Bool(true))?;
            Ok(())
        }
        other => Err(crate::tree::Error(format!(
            "unknown template '{other}' (available: {})",
            registry().iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
        ))),
    }
}
