//! Phase 2 integration tests: Studio editing commands, transforms, Model
//! pivots, extended property types, and undo/redo history.
//!
//! Requires the C++ engine (see roundtrip.rs for build instructions).

use std::path::PathBuf;
use std::process::Command;

use bloxcli::rbxio::{read_place, write_place};
use bloxcli::tree::{CFrameValue, Node, PropValue};

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(
        candidate.is_file(),
        "blox-engine not found at {} — build it first: cmake -S . -B build -G Ninja && cmake --build build",
        candidate.display()
    );
    candidate
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--engine")
        .arg(engine_path())
        .args(args)
        .output()
        .expect("run bloxcli")
}

fn cli_ok(args: &[&str]) -> String {
    let out = cli(args);
    assert!(
        out.status.success(),
        "bloxcli {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

fn cli_json(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json")
        .arg("--engine")
        .arg(engine_path())
        .args(args)
        .output()
        .expect("run bloxcli --json");
    assert!(
        out.status.success(),
        "bloxcli --json {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("json output")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-p2-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn cframe(node: &Node) -> CFrameValue {
    match node.properties.get("CFrame") {
        Some(PropValue::CFrame(cf)) => cf.clone(),
        other => panic!("expected CFrame on {}, got {other:?}", node.name),
    }
}

fn assert_pos(node: &Node, x: f32, y: f32, z: f32) {
    let cf = cframe(node);
    assert!(
        (cf.position[0] - x).abs() < 1e-4
            && (cf.position[1] - y).abs() < 1e-4
            && (cf.position[2] - z).abs() < 1e-4,
        "{} position {:?} != ({x}, {y}, {z})",
        node.name,
        cf.position
    );
}

#[test]
fn hierarchy_editing_workflow() {
    let dir = temp_dir("hierarchy");
    let place = dir.join("edit.rbxl");
    let p = place.to_str().unwrap().to_string();

    cli_ok(&["new", &p]);
    // Standard services from Phase 2 exist.
    let (tree, _) = read_place(&place).unwrap();
    for service in [
        "Workspace", "Lighting", "ReplicatedStorage", "ServerStorage",
        "ServerScriptService", "StarterGui", "StarterPlayer", "StarterPack",
    ] {
        assert!(tree.find_path(service).is_ok(), "missing service {service}");
    }

    cli_ok(&["add", &p, "--parent", "Workspace", "--class", "Part", "--name", "A"]);
    cli_ok(&["add", &p, "--parent", "Workspace", "--class", "Part", "--name", "B"]);
    cli_ok(&["move", &p, "--target", "Workspace/A", "--position", "4,0,0"]);
    cli_ok(&["move", &p, "--target", "Workspace/B", "--position", "-4,0,0"]);

    // Clone keeps the transform and gets a fresh referent.
    let cloned = cli_json(&["clone", &p, "--target", "Workspace/A", "--name", "ACopy"]);
    let orig = cli_json(&["get", &p, "--target", "Workspace/A"]);
    assert_ne!(
        cloned["referent"], orig["instance"]["referent"],
        "clone must not share the source referent"
    );
    let (tree, _) = read_place(&place).unwrap();
    assert_pos(tree.find_path("Workspace/ACopy").unwrap(), 4.0, 0.0, 0.0);

    // Group A and B; world transforms must not change.
    cli_ok(&["group", &p, "--targets", "Workspace/A,Workspace/B", "--name", "Pair"]);
    let (tree, _) = read_place(&place).unwrap();
    assert_pos(tree.find_path("Workspace/Pair/A").unwrap(), 4.0, 0.0, 0.0);
    assert_pos(tree.find_path("Workspace/Pair/B").unwrap(), -4.0, 0.0, 0.0);

    // Find by class under the model.
    let found = cli_json(&["find", &p, "--class", "Part", "--under", "Workspace/Pair"]);
    assert_eq!(found["count"], 2);

    // Reparent the clone into the model, then delete the whole model.
    cli_ok(&["reparent", &p, "--target", "Workspace/ACopy", "--parent", "Workspace/Pair"]);
    let deleted = cli_json(&["delete", &p, "--target", "Workspace/Pair"]);
    assert_eq!(deleted["deletedCount"], 4); // Pair + A + B + ACopy
    let (tree, _) = read_place(&place).unwrap();
    assert!(tree.find_path("Workspace/Pair").is_err());
    assert!(tree.find_path("Workspace/A").is_err());

    // Cycle reparenting is rejected and leaves the file untouched.
    cli_ok(&["add", &p, "--parent", "Workspace", "--class", "Model", "--name", "Outer"]);
    cli_ok(&["add", &p, "--parent", "Workspace/Outer", "--class", "Folder", "--name", "Inner"]);
    let before = std::fs::read(&place).unwrap();
    let out = cli(&["reparent", &p, "--target", "Workspace/Outer", "--parent", "Workspace/Outer/Inner"]);
    assert!(!out.status.success(), "cycle reparent was accepted");
    assert_eq!(before, std::fs::read(&place).unwrap(), "failed reparent modified the file");
}

#[test]
fn model_transforms_and_pivot() {
    let dir = temp_dir("transforms");
    let place = dir.join("rig.rbxl");
    let p = place.to_str().unwrap().to_string();

    cli_ok(&["new", &p]);
    cli_ok(&["add", &p, "--parent", "Workspace", "--class", "Model", "--name", "Rig"]);
    cli_ok(&["add", &p, "--parent", "Workspace/Rig", "--class", "Part", "--name", "L"]);
    cli_ok(&["add", &p, "--parent", "Workspace/Rig", "--class", "Part", "--name", "R"]);
    cli_ok(&["set", &p, "--target", "Workspace/Rig/L", "--property", "Size", "--value", "2,2,2"]);
    cli_ok(&["set", &p, "--target", "Workspace/Rig/R", "--property", "Size", "--value", "2,2,2"]);
    cli_ok(&["move", &p, "--target", "Workspace/Rig/L", "--position", "3,0,0"]);
    cli_ok(&["move", &p, "--target", "Workspace/Rig/R", "--position", "-3,0,0"]);

    // Pivot defaults to the bounding-box centre (origin here).
    let pivot = cli_json(&["pivot", &p, "--target", "Workspace/Rig"]);
    let pos = &pivot["pivot"]["value"]["position"];
    assert_eq!(pos[0].as_f64().unwrap(), 0.0);

    // Rotate the model 90° about Y: +X part goes to -Z.
    cli_ok(&["rotate", &p, "--target", "Workspace/Rig", "--angles", "0,90,0"]);
    let (tree, _) = read_place(&place).unwrap();
    assert_pos(tree.find_path("Workspace/Rig/L").unwrap(), 0.0, 0.0, -3.0);
    assert_pos(tree.find_path("Workspace/Rig/R").unwrap(), 0.0, 0.0, 3.0);

    // Uniform model scale about the pivot doubles offsets and sizes.
    cli_ok(&["scale", &p, "--target", "Workspace/Rig", "--factor", "2"]);
    let (tree, _) = read_place(&place).unwrap();
    let l = tree.find_path("Workspace/Rig/L").unwrap();
    assert_pos(l, 0.0, 0.0, -6.0);
    assert_eq!(l.properties.get("Size"), Some(&PropValue::Vector3([4.0, 4.0, 4.0])));

    // Non-uniform model scale is rejected (Roblox Model:ScaleTo is uniform).
    let out = cli(&["scale", &p, "--target", "Workspace/Rig", "--factors", "2,1,2"]);
    assert!(!out.status.success());

    // An explicit pivot changes the centre of rotation.
    cli_ok(&["pivot", &p, "--target", "Workspace/Rig", "--position", "100,0,0"]);
    cli_ok(&["rotate", &p, "--target", "Workspace/Rig", "--angles", "0,90,0"]);
    let (tree, _) = read_place(&place).unwrap();
    let l = tree.find_path("Workspace/Rig/L").unwrap();
    // L was at (0,0,-6); rotating about (100,0,0) by 90°Y must displace it.
    assert!(cframe(l).position[0].abs() > 50.0, "explicit pivot ignored: {:?}", cframe(l).position);

    // Single-part move/rotate/scale still behave.
    cli_ok(&["move", &p, "--target", "Workspace/Rig/R", "--delta", "0,10,0"]);
    let (tree, _) = read_place(&place).unwrap();
    let r = tree.find_path("Workspace/Rig/R").unwrap();
    assert!((cframe(r).position[1] - 10.0).abs() < 1e-4);
    cli_ok(&["scale", &p, "--target", "Workspace/Rig/R", "--size", "1,2,3"]);
    let (tree, _) = read_place(&place).unwrap();
    assert_eq!(
        tree.find_path("Workspace/Rig/R").unwrap().properties.get("Size"),
        Some(&PropValue::Vector3([1.0, 2.0, 3.0]))
    );

    // Transforms on transform-less instances fail loudly.
    let out = cli(&["move", &p, "--target", "Workspace", "--delta", "1,0,0"]);
    assert!(!out.status.success(), "moving Workspace itself should fail (it has parts but is not a Model)");
}

#[test]
fn undo_redo_history() {
    let dir = temp_dir("history");
    let place = dir.join("hist.rbxl");
    let p = place.to_str().unwrap().to_string();

    // Nothing to undo on a fresh file.
    cli_ok(&["new", &p]);
    let out = cli(&["undo", &p]);
    assert!(!out.status.success(), "undo with no history succeeded");

    cli_ok(&["add", &p, "--parent", "Workspace", "--class", "Part", "--name", "One"]);
    cli_ok(&["set", &p, "--target", "Workspace/One", "--property", "Transparency", "--value", "0.5"]);
    let (tree, _) = read_place(&place).unwrap();
    assert_eq!(
        tree.find_path("Workspace/One").unwrap().properties.get("Transparency"),
        Some(&PropValue::Float32(0.5))
    );

    // Undo the set: property returns to the default, part still exists.
    cli_ok(&["undo", &p]);
    let (tree, _) = read_place(&place).unwrap();
    assert_eq!(
        tree.find_path("Workspace/One").unwrap().properties.get("Transparency"),
        Some(&PropValue::Float32(0.0))
    );

    // Undo the add: the part is gone.
    cli_ok(&["undo", &p]);
    let (tree, _) = read_place(&place).unwrap();
    assert!(tree.find_path("Workspace/One").is_err());

    // Redo both steps.
    cli_ok(&["redo", &p]);
    cli_ok(&["redo", &p]);
    let (tree, _) = read_place(&place).unwrap();
    assert_eq!(
        tree.find_path("Workspace/One").unwrap().properties.get("Transparency"),
        Some(&PropValue::Float32(0.5))
    );

    // A fresh mutation clears the redo stack.
    cli_ok(&["undo", &p]);
    cli_ok(&["set", &p, "--target", "Workspace/One", "--property", "Transparency", "--value", "0.75"]);
    let out = cli(&["redo", &p]);
    assert!(!out.status.success(), "redo after a new mutation should be empty");

    // History lives in the sidecar directory, not inside the .rbxl.
    assert!(dir.join("hist.rbxl.bloxhistory").is_dir());
    let bytes = std::fs::read(&place).unwrap();
    assert!(bytes.starts_with(b"<roblox!"));
}

#[test]
fn extended_property_types_roundtrip() {
    let dir = temp_dir("types");
    let place = dir.join("typed.rbxl");
    let p = place.to_str().unwrap().to_string();

    cli_ok(&["new", &p]);
    // GUI element with UDim2 layout properties.
    cli_ok(&["add", &p, "--parent", "StarterGui", "--class", "ScreenGui", "--name", "Gui"]);
    cli_ok(&["add", &p, "--parent", "StarterGui/Gui", "--class", "Frame", "--name", "Panel"]);
    cli_ok(&["set", &p, "--target", "StarterGui/Gui/Panel", "--property", "Size", "--type", "UDim2", "--value", "0.5,10,0.25,20"]);
    cli_ok(&["set", &p, "--target", "StarterGui/Gui/Panel", "--property", "Position", "--type", "UDim2", "--value", "0,5,0,5"]);

    // ParticleEmitter with sequences and a range.
    cli_ok(&["add", &p, "--parent", "Workspace", "--class", "Part", "--name", "EmitterPart"]);
    cli_ok(&["add", &p, "--parent", "Workspace/EmitterPart", "--class", "ParticleEmitter", "--name", "Sparks"]);
    cli_ok(&["set", &p, "--target", "Workspace/EmitterPart/Sparks", "--property", "Color", "--type", "ColorSequence", "--value", "0:1,0,0;1:0,0,1"]);
    cli_ok(&["set", &p, "--target", "Workspace/EmitterPart/Sparks", "--property", "Size", "--type", "NumberSequence", "--value", "0:0,0;1:2,0"]);
    cli_ok(&["set", &p, "--target", "Workspace/EmitterPart/Sparks", "--property", "Lifetime", "--type", "NumberRange", "--value", "0.5,1.5"]);

    // Custom physical properties on a part.
    cli_ok(&["set", &p, "--target", "Workspace/EmitterPart", "--property", "CustomPhysicalProperties", "--type", "PhysicalProperties", "--value", "0.7,0.3,0.5,1,1,1"]);

    let (tree, _) = read_place(&place).unwrap();
    let panel = tree.find_path("StarterGui/Gui/Panel").unwrap();
    assert_eq!(panel.properties.get("Size"), Some(&PropValue::UDim2([0.5, 10.0, 0.25, 20.0])));
    let sparks = tree.find_path("Workspace/EmitterPart/Sparks").unwrap();
    match sparks.properties.get("Color") {
        Some(PropValue::ColorSequence(kps)) => {
            assert_eq!(kps.len(), 2);
            assert_eq!(kps[0].color, [1.0, 0.0, 0.0]);
            assert_eq!(kps[1].time, 1.0);
        }
        other => panic!("Color not a ColorSequence: {other:?}"),
    }
    assert_eq!(sparks.properties.get("Lifetime"), Some(&PropValue::NumberRange([0.5, 1.5])));
    let part = tree.find_path("Workspace/EmitterPart").unwrap();
    match part.properties.get("CustomPhysicalProperties") {
        // The binary format does not round-trip acoustic absorption on all
        // paths; the first five physical fields must survive.
        Some(PropValue::PhysicalProperties(Some(v))) => {
            assert!((v[0] - 0.7).abs() < 1e-6 && (v[1] - 0.3).abs() < 1e-6 && (v[2] - 0.5).abs() < 1e-6);
        }
        other => panic!("CustomPhysicalProperties lost: {other:?}"),
    }
    cli_ok(&["validate", &p]);

    // The same tree converts to XML and back unchanged.
    let xml = dir.join("typed.rbxlx");
    let xml_s = xml.to_str().unwrap().to_string();
    cli_ok(&["export", &p, &xml_s]);
    let (xml_tree, _) = read_place(&xml).unwrap();
    let mut a = tree.clone();
    let mut b = xml_tree.clone();
    fn strip(node: &mut Node) {
        node.referent = 0;
        for child in &mut node.children {
            strip(child);
        }
    }
    strip(&mut a);
    strip(&mut b);
    assert_eq!(a, b, "extended types changed across binary -> XML");
}

#[test]
fn content_migration_properties_roundtrip() {
    // Modern Roblox serializes legacy ContentId properties (MeshId,
    // TextureID, Decal.Texture) as Content under new names (MeshContent,
    // TextureContent). The canonical names must survive a write/read cycle.
    let dir = temp_dir("content");
    let place = dir.join("content.rbxl");
    let p = place.to_str().unwrap().to_string();

    cli_ok(&["new", &p]);
    cli_ok(&["add", &p, "--parent", "Workspace", "--class", "MeshPart", "--name", "Rock"]);
    cli_ok(&["set", &p, "--target", "Workspace/Rock", "--property", "MeshId", "--value", "rbxassetid://12345"]);
    cli_ok(&["set", &p, "--target", "Workspace/Rock", "--property", "TextureID", "--value", "rbxassetid://6789"]);
    cli_ok(&["add", &p, "--parent", "Workspace/Rock", "--class", "Decal", "--name", "Face"]);
    cli_ok(&["set", &p, "--target", "Workspace/Rock/Face", "--property", "Texture", "--value", "rbxassetid://111"]);

    let (tree, _) = read_place(&place).unwrap();
    let rock = tree.find_path("Workspace/Rock").unwrap();
    assert_eq!(
        rock.properties.get("MeshId"),
        Some(&PropValue::ContentId("rbxassetid://12345".to_string()))
    );
    assert_eq!(
        rock.properties.get("TextureID"),
        Some(&PropValue::ContentId("rbxassetid://6789".to_string()))
    );
    assert_eq!(
        tree.find_path("Workspace/Rock/Face").unwrap().properties.get("Texture"),
        Some(&PropValue::ContentId("rbxassetid://111".to_string()))
    );
    let report = cli_json(&["validate", &p]);
    assert_eq!(report["valid"], true, "issues: {}", report["issues"]);
}

#[test]
fn ref_properties_survive() {
    let dir = temp_dir("refs");
    let place = dir.join("refs.rbxl");

    // Build a Model whose PrimaryPart points at its child part.
    let mut root = Node::new("DataModel", "Place");
    let mut workspace = Node::new("Workspace", "Workspace");
    let mut model = Node::new("Model", "Rig");
    model.properties.insert(
        "PrimaryPart".to_string(),
        PropValue::Ref("0/0/0".to_string()), // root -> Workspace(0) -> Rig(0) -> Wheel(0)
    );
    let wheel = Node::new("Part", "Wheel");
    model.children.push(wheel);
    workspace.children.push(model);
    root.children.push(workspace);

    write_place(&place, &root).expect("write ref place");
    let (back, _) = read_place(&place).expect("read ref place");
    let rig = back.find_path("Workspace/Rig").unwrap();
    assert_eq!(
        rig.properties.get("PrimaryPart"),
        Some(&PropValue::Ref("0/0/0".to_string())),
        "PrimaryPart reference was not preserved"
    );

    // Moving the model keeps the reference valid (same target, new transform).
    let place_s = place.to_str().unwrap().to_string();
    cli_ok(&["move", &place_s, "--target", "Workspace/Rig", "--delta", "5,0,0"]);
    let (back, _) = read_place(&place).expect("read after move");
    let wheel = back.find_path("Workspace/Rig/Wheel").unwrap();
    assert_pos(wheel, 5.0, 0.0, 0.0);
    assert_eq!(
        back.find_path("Workspace/Rig").unwrap().properties.get("PrimaryPart"),
        Some(&PropValue::Ref("0/0/0".to_string()))
    );
}
