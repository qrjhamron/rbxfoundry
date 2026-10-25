//! Terrain commands.

use serde_json::json;
use crate::cli::args::{parse_cmd, usage_err, Global};
use crate::app::session::{emit, load_session, mutate};
use crate::protocol::tree::Result;

pub fn terrain_material_value(name: &str) -> Result<i64> {
    match name {
        "Air" => Ok(0),
        "Plastic" => Ok(256),
        "Wood" => Ok(512),
        "Slate" | "Rock" => Ok(800),
        "Concrete" => Ok(816),
        "Brick" => Ok(848),
        "Grass" => Ok(1280),
        "Sand" => Ok(1312),
        "Snow" => Ok(1328),
        "Ice" => Ok(1536),
        "Water" => Ok(2048),
        other => Err(usage_err(&format!(
            "unknown terrain material '{other}' (use Grass, Sand, Slate, Concrete, Brick, Wood, Ice, Snow, Water, Air, or a numeric Enum.Material value)"
        ))),
    }
}

pub fn parse_xyz(text: &str, what: &str) -> Result<[f64; 3]> {
    let parts: Vec<f64> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| usage_err(&format!("{what} must be 'x,y,z'")))?;
    if parts.len() != 3 {
        return Err(usage_err(&format!("{what} must be 'x,y,z'")));
    }
    Ok([parts[0], parts[1], parts[2]])
}

pub fn cmd_terrain(global: &Global, args: &[String]) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("terrain needs a subcommand: fill|ball|flatten|smooth|paint|clear|info"));
    };
    match sub.as_str() {
        "fill" | "ball" => {
            let parsed = parse_cmd(rest, &["center", "size", "radius", "material"], &[])?;
            let file = parsed.file()?;
            let center = parse_xyz(parsed.opt("center").unwrap_or("0,0,0"), "--center")?;
            let material_name = parsed.opt("material").unwrap_or("Grass");
            let material: i64 = match material_name.parse::<i64>() {
                Ok(v) => v,
                Err(_) => terrain_material_value(material_name)?,
            };
            let size = parse_xyz(parsed.opt("size").unwrap_or("16,4,16"), "--size")?;
            let radius: f64 = parsed
                .opt("radius")
                .unwrap_or("8")
                .parse()
                .map_err(|_| usage_err("--radius must be a number"))?;
            let (result, _tree) = mutate(global, &file, |engine| {
                if sub == "fill" {
                    engine.call("terrain/fill", json!({"center": center, "size": size, "material": material}))
                } else {
                    engine.call("terrain/fillBall", json!({"center": center, "radius": radius, "material": material}))
                }
            })?;
            emit(
                global,
                format!("terrain {} applied to {} ({} voxels total)", sub, file.display(), result["voxels"]),
                json!({"ok": true, "command": format!("terrain {sub}"), "path": file.display().to_string(), "result": result}),
            );
            Ok(())
        }
        "flatten" | "smooth" | "paint" => {
            let parsed = parse_cmd(rest, &["center", "size", "material", "height"], &[])?;
            let file = parsed.file()?;
            let center = parse_xyz(parsed.opt("center").unwrap_or("0,0,0"), "--center")?;
            let size = parse_xyz(parsed.opt("size").unwrap_or("16,16,16"), "--size")?;
            let material_name = parsed.opt("material").unwrap_or("Grass");
            let material: i64 = match material_name.parse::<i64>() {
                Ok(v) => v,
                Err(_) => terrain_material_value(material_name)?,
            };
            let mut params = json!({"center": center, "size": size, "material": material});
            if let Some(h) = parsed.opt("height") {
                params["height"] = json!(h.parse::<f64>().map_err(|_| usage_err("--height must be a number"))?);
            }
            let method = format!("terrain/{sub}");
            let (result, _tree) = mutate(global, &file, |engine| engine.call(&method, params))?;
            emit(
                global,
                format!("terrain {sub} applied to {} ({} voxels total)", file.display(), result["voxels"]),
                json!({"ok": true, "command": format!("terrain {sub}"), "path": file.display().to_string(), "result": result}),
            );
            Ok(())
        }
        "clear" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let (result, _tree) = mutate(global, &file, |engine| engine.call("terrain/clear", json!({})))?;
            emit(
                global,
                format!("terrain cleared in {}", file.display()),
                json!({"ok": true, "command": "terrain clear", "path": file.display().to_string(), "result": result}),
            );
            Ok(())
        }
        "info" => {
            let parsed = parse_cmd(rest, &[], &[])?;
            let file = parsed.file()?;
            let (mut engine, _format) = load_session(global, &file)?;
            let result = engine.call("terrain/info", json!({}))?;
            engine.shutdown();
            emit(
                global,
                format!(
                    "{}: {} voxels ({} stud voxels)",
                    file.display(),
                    result["voxels"],
                    result["voxelSize"]
                ),
                json!({"ok": true, "command": "terrain info", "path": file.display().to_string(), "result": result}),
            );
            Ok(())
        }
        other => Err(usage_err(&format!("unknown terrain subcommand '{other}'"))),
    }
}
