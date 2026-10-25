//! Phase 17 integration tests: the export integrity guard (dangling
//! references, unresolved assets, corrupted terrain payloads) and
//! `.rbxm` model export / inspect / import.

use std::path::PathBuf;
use std::process::Command;

use bloxcli::rbxio::{read_place, write_place};
use bloxcli::tree::{Node, PropValue};

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(candidate.is_file(), "blox-engine not found at {}", candidate.display());
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
    assert!(out.status.success(), "bloxcli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
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
    assert!(out.status.success(), "bloxcli --json {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
    serde_json::from_slice(&out.stdout).expect("json output")
}

fn cli_json_raw(args: &[&str]) -> (bool, serde_json::Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json")
        .arg("--engine")
        .arg(engine_path())
        .args(args)
        .output()
        .expect("run bloxcli --json");
    let value = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out.status.success(), value)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-phase17-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn place_with(children: Vec<Node>) -> Node {
    let mut root = Node::new("DataModel", "Place");
    let mut workspace = Node::new("Workspace", "Workspace");
    workspace.children = children;
    root.children.push(workspace);
    root
}

#[test]
fn guard_blocks_dangling_reference_export() {
    // (a) Guard logic itself, on a constructed canonical tree whose
    // PrimaryPart points at a child index that does not exist.
    let mut rig = Node::new("Model", "Rig");
    rig.properties.insert("PrimaryPart".to_string(), PropValue::Ref("0/0/9".to_string()));
    rig.children.push(Node::new("Part", "Wheel"));
    let tree = place_with(vec![rig]);
    let issues = bloxcli::integrity::check_export_integrity(&tree, &temp_dir("guardunit"));
    let errors = bloxcli::integrity::errors(&issues);
    assert_eq!(errors.len(), 1, "one dangling-reference error: {:?}", issues.len());
    assert!(errors[0].message.contains("does not exist"), "{}", errors[0].message);
    assert!(errors[0].path.ends_with("Rig"), "{}", errors[0].path);

    // (b) End to end: a hand-written .rbxlx whose PrimaryPart referent
    // (RBX9) is never defined must fail validate/export/save loudly at
    // some layer, never silently, and never modify the file.
    let dir = temp_dir("dangling");
    let place = dir.join("game.rbxlx");
    std::fs::write(&place, r#"<roblox version="4">
  <Item class="Workspace" referent="RBX0">
    <Properties><string name="Name">Workspace</string></Properties>
    <Item class="Model" referent="RBX1">
      <Properties>
        <string name="Name">Rig</string>
        <Ref name="PrimaryPart">RBX9</Ref>
      </Properties>
      <Item class="Part" referent="RBX2">
        <Properties><string name="Name">Wheel</string></Properties>
      </Item>
    </Item>
  </Item>
</roblox>
"#).expect("write xml fixture");
    let f = place.to_str().unwrap().to_string();

    let (ok, report) = cli_json_raw(&["validate", &f]);
    assert!(!ok, "validate must fail on a dangling reference: {report}");
    assert_eq!(report["valid"], false);

    let out = dir.join("out.rbxl");
    let export = cli(&["export", &f, out.to_str().unwrap()]);
    assert!(!export.status.success(), "export must refuse a dangling reference");
    assert!(
        String::from_utf8_lossy(&export.stderr).contains("reference"),
        "error names the reference problem: {}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(!out.exists(), "refused export must not create the output file");

    let before = std::fs::read(&place).unwrap();
    let save = cli(&["save", &f]);
    assert!(!save.status.success(), "save must refuse a dangling reference");
    assert_eq!(std::fs::read(&place).unwrap(), before, "refused save leaves the file untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unresolved_asset_warns_but_is_preserved() {
    let dir = temp_dir("asset");
    let place = dir.join("game.rbxl");
    let f = place.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "MeshPart", "--name", "Rock"]);
    cli_ok(&["set", &f, "--target", "Workspace/Rock", "--property", "MeshId", "--type", "ContentId", "--value", "rbxassetid://123456"]);

    let (ok, report) = cli_json_raw(&["validate", &f]);
    assert!(ok, "unresolved assets are warnings, not errors: {report}");
    assert_eq!(report["valid"], true);
    assert!(report["integrity"]["warnings"].as_u64().unwrap() >= 1, "{report}");
    assert!(format!("{}", report["issues"]).contains("rbxassetid://123456"));

    let out = dir.join("out.rbxl");
    let exported = cli_json(&["export", &f, out.to_str().unwrap()]);
    assert!(
        format!("{}", exported["integrityWarnings"]).contains("rbxassetid://123456"),
        "export reports the unresolved asset: {exported}"
    );
    // The id survives verbatim — never substituted or dropped.
    let (back, _) = read_place(&out).expect("read exported");
    let rock = back.find_path("Workspace/Rock").unwrap();
    assert_eq!(
        rock.properties.get("MeshId"),
        Some(&PropValue::ContentId("rbxassetid://123456".to_string()))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupt_terrain_payload_blocks_export() {
    let dir = temp_dir("terrain");
    let place = dir.join("game.rbxl");
    let f = place.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["terrain", "fill", &f, "--center", "0,2,0", "--size", "8,4,8", "--material", "Grass"]);

    // Corrupt the voxel payload in the file directly (bypasses commands).
    let (mut node, _) = read_place(&place).expect("read filled place");
    let terrain = node.find_path_mut("Workspace/Terrain").expect("terrain exists");
    terrain.properties.insert(
        "BloxVoxels".to_string(),
        PropValue::BinaryString("AAAA".to_string()), // valid base64, not BTR1
    );
    write_place(&place, &node).expect("write corrupted place");

    let out = dir.join("out.rbxl");
    let export = cli(&["export", &f, out.to_str().unwrap()]);
    assert!(!export.status.success(), "corrupt terrain must block export");
    assert!(
        String::from_utf8_lossy(&export.stderr).contains("BTR1"),
        "error names the corruption: {}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(!out.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

fn house_place() -> Node {
    let mut house = Node::new("Model", "House");
    house
        .properties
        .insert("PrimaryPart".to_string(), PropValue::Ref("0/0/0".to_string())); // Workspace/House/Wall
    house.children.push(Node::new("Part", "Wall"));
    house.children.push(Node::new("Part", "Roof"));
    place_with(vec![house])
}

#[test]
fn model_export_inspect_import_round_trip() {
    let dir = temp_dir("model");
    let place = dir.join("game.rbxl");
    write_place(&place, &house_place()).expect("write house place");
    let f = place.to_str().unwrap().to_string();

    let model_file = dir.join("house.rbxm");
    let exported = cli_json(&[
        "model", "export", &f, "--target", "Workspace/House", "--output", model_file.to_str().unwrap(),
    ]);
    assert_eq!(exported["rootClass"], "Model");
    assert_eq!(exported["instances"], 3);
    let bytes = std::fs::read(&model_file).unwrap();
    assert!(bytes.starts_with(b"<roblox!"), "genuine binary model file");

    let inspected = cli_json(&["model", "inspect", model_file.to_str().unwrap()]);
    assert_eq!(inspected["rootClass"], "Model");
    assert_eq!(inspected["name"], "House");
    assert_eq!(inspected["instances"], 3);
    assert!(format!("{}", inspected["tree"]).contains("Wall"));

    // Places refuse model files and vice versa.
    let (ok, _) = cli_json_raw(&["inspect", model_file.to_str().unwrap()]);
    assert!(!ok, "place commands must reject a model file");

    // Import into a fresh place; the PrimaryPart ref must resolve there.
    let target = dir.join("target.rbxl");
    let tf = target.to_str().unwrap().to_string();
    cli_ok(&["new", &tf]);
    let imported = cli_json(&["model", "import", model_file.to_str().unwrap(), &tf, "--parent", "Workspace"]);
    assert_eq!(imported["name"], "House");
    let tree = cli_json(&["tree", &tf]);
    assert!(format!("{}", tree).contains("House"), "house grafted: {tree}");
    let (ok, report) = cli_json_raw(&["validate", &tf]);
    assert!(ok, "imported model validates (refs resolve): {report}");
    assert_eq!(report["integrity"]["errors"], 0);
    let (back, _) = read_place(&target).expect("read target");
    let house = back.find_path("Workspace/House").expect("house present");
    assert!(matches!(house.properties.get("PrimaryPart"), Some(PropValue::Ref(v)) if !v.is_empty()),
        "PrimaryPart preserved through model round trip");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn model_export_rejects_external_references() {
    let dir = temp_dir("modelexternal");
    let place = dir.join("game.rbxl");
    // House.PrimaryPart points at a Part outside the House subtree.
    let mut root = Node::new("DataModel", "Place");
    let mut workspace = Node::new("Workspace", "Workspace");
    let mut house = Node::new("Model", "House");
    house
        .properties
        .insert("PrimaryPart".to_string(), PropValue::Ref("0/1".to_string())); // Workspace/Outside
    house.children.push(Node::new("Part", "Wall"));
    workspace.children.push(house);
    workspace.children.push(Node::new("Part", "Outside"));
    root.children.push(workspace);
    write_place(&place, &root).expect("write fixture");
    let f = place.to_str().unwrap().to_string();

    let model_file = dir.join("house.rbxm");
    let export = cli(&[
        "model", "export", &f, "--target", "Workspace/House", "--output", model_file.to_str().unwrap(),
    ]);
    assert!(!export.status.success(), "external references must block model export");
    assert!(
        String::from_utf8_lossy(&export.stderr).contains("outside the exported subtree"),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(!model_file.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
