//! End-to-end Phase 1 tests: CLI workflow, genuine RBXL bytes, format
//! conversion, preservation of unknown data, and failure behaviour.
//!
//! Requires the C++ engine. Build it first:
//!   cmake -S . -B build -G Ninja && cmake --build build
//! (or set $BLOX_ENGINE). The test panics with instructions if it is missing;
//! it never silently skips.

use std::path::{Path, PathBuf};
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
    if candidate.is_file() {
        return candidate;
    }
    panic!(
        "blox-engine not found at {} — build it first: \
         cmake -S . -B build -G Ninja && cmake --build build (or set $BLOX_ENGINE)",
        candidate.display()
    );
}

fn cli(args: &[&str]) -> std::process::Output {
    let engine = engine_path();
    Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--engine")
        .arg(&engine)
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

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn strip_referents(node: &mut Node) {
    node.referent = 0;
    for child in &mut node.children {
        strip_referents(child);
    }
}

#[test]
fn full_workflow_and_format_roundtrip() {
    let dir = temp_dir("workflow");
    let place = dir.join("game.rbxl");
    let place_s = place.to_str().unwrap().to_string();

    cli_ok(&["new", &place_s, "--name", "TestPlace"]);
    cli_ok(&["add", &place_s, "--parent", "Workspace", "--class", "Part", "--name", "Baseplate"]);
    cli_ok(&["add", &place_s, "--parent", "Workspace", "--class", "Folder", "--name", "Props"]);
    cli_ok(&["add", &place_s, "--parent", "Workspace/Props", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &place_s, "--target", "Workspace/Baseplate", "--property", "Size", "--value", "64,1,64"]);
    cli_ok(&["set", &place_s, "--target", "Workspace/Baseplate", "--property", "CFrame", "--value", "0,0.5,0"]);
    cli_ok(&["set", &place_s, "--target", "Workspace/Baseplate", "--property", "Color", "--value", "0.5,0.25,1"]);
    cli_ok(&["set", &place_s, "--target", "Workspace/Baseplate", "--property", "Transparency", "--value", "0.25"]);
    cli_ok(&["set", &place_s, "--target", "Workspace/Baseplate", "--property", "Anchored", "--value", "true"]);
    cli_ok(&["set", &place_s, "--target", "Workspace/Props/Main", "--property", "Source", "--value", "print(\"hello\")"]);
    cli_ok(&["validate", &place_s]);

    // Inspect through the engine and check typed values survived.
    let (tree, _) = read_place(&place).expect("read back place");
    let baseplate = tree.find_path("Workspace/Baseplate").expect("baseplate");
    assert_eq!(baseplate.properties.get("Size"), Some(&PropValue::Vector3([64.0, 1.0, 64.0])));
    assert_eq!(baseplate.properties.get("Transparency"), Some(&PropValue::Float32(0.25)));
    assert_eq!(baseplate.properties.get("Anchored"), Some(&PropValue::Bool(true)));
    match baseplate.properties.get("Color") {
        Some(PropValue::Color3(c)) => {
            // The binary format stores Color3 as three bytes, so values are
            // quantized to 1/255 steps. That is genuine format behaviour.
            assert!((c[0] - 0.5).abs() < 1.0 / 255.0 && (c[2] - 1.0).abs() < 1.0 / 255.0, "got {c:?}");
        }
        other => panic!("unexpected Color: {other:?}"),
    }
    let script = tree.find_path("Workspace/Props/Main").expect("script");
    assert_eq!(
        script.properties.get("Source"),
        Some(&PropValue::String("print(\"hello\")".to_string()))
    );

    // Export to XML and back to binary; canonical trees must match exactly.
    let xml = dir.join("game.rbxlx");
    let xml_s = xml.to_str().unwrap().to_string();
    cli_ok(&["export", &place_s, &xml_s]);
    let (xml_tree, _) = read_place(&xml).expect("read xml place");
    let mut a = tree.clone();
    let mut b = xml_tree.clone();
    strip_referents(&mut a);
    strip_referents(&mut b);
    assert_eq!(a, b, "binary -> XML round-trip changed the tree");

    let place2 = dir.join("game2.rbxl");
    let place2_s = place2.to_str().unwrap().to_string();
    cli_ok(&["export", &xml_s, &place2_s]);
    let (tree2, _) = read_place(&place2).expect("read re-exported binary");
    let mut c = tree2.clone();
    strip_referents(&mut c);
    assert_eq!(a, c, "XML -> binary round-trip changed the tree");

    // save rewrites canonically without changing the tree.
    cli_ok(&["save", &place_s]);
    let (tree3, _) = read_place(&place).expect("read after save");
    let mut d = tree3.clone();
    strip_referents(&mut d);
    assert_eq!(a, d, "save changed the tree");
}

#[test]
fn rbxl_output_is_genuine_binary() {
    let dir = temp_dir("genuine");
    let place = dir.join("real.rbxl");
    let place_s = place.to_str().unwrap().to_string();
    cli_ok(&["new", &place_s]);
    cli_ok(&["add", &place_s, "--parent", "Workspace", "--class", "Part", "--name", "Baseplate"]);

    let bytes = std::fs::read(&place).expect("read rbxl");
    // Header per the Roblox binary format spec (rbx-dom docs/binary.md):
    // magic "<roblox!", signature 89 FF 0D 0A 1A 0A, u16 version 0,
    // i32 class count, i32 instance count, 8 reserved bytes.
    assert!(bytes.starts_with(b"<roblox!"), "missing <roblox! magic");
    assert_eq!(&bytes[8..14], &[0x89, 0xFF, 0x0D, 0x0A, 0x1A, 0x0A], "bad signature");
    assert_eq!(u16::from_le_bytes([bytes[14], bytes[15]]), 0, "bad format version");
    let class_count = i32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let instance_count = i32::from_le_bytes(bytes[20..24].try_into().unwrap());
    // Header counts must agree with the actual tree contents.
    let (tree, _) = read_place(&place).expect("read place for header check");
    let mut classes = std::collections::HashSet::new();
    fn collect(node: &Node, classes: &mut std::collections::HashSet<String>) {
        classes.insert(node.class_name.clone());
        for child in &node.children {
            collect(child, classes);
        }
    }
    collect(&tree, &mut classes);
    classes.remove("DataModel"); // the root itself is not stored in the file
    assert_eq!(class_count as usize, classes.len(), "header class count");
    assert_eq!(instance_count as usize, tree.count() - 1, "header instance count");
    // It must not be JSON with an .rbxl extension.
    assert_ne!(bytes[0], b'{');

    // Walk chunk headers (16 bytes each) and require a terminating END chunk.
    let mut offset = 32usize;
    let mut names = Vec::new();
    loop {
        assert!(offset + 16 <= bytes.len(), "chunk header overruns file");
        let name = std::str::from_utf8(&bytes[offset..offset + 4]).expect("chunk name").to_string();
        let compressed = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let uncompressed = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
        names.push(name.clone());
        offset += 16 + if compressed != 0 { compressed } else { uncompressed };
        if name.trim_end_matches('\0') == "END" {
            break;
        }
        assert!(offset <= bytes.len(), "chunk data overruns file");
    }
    assert!(names.iter().any(|n| n == "INST"), "no INST chunk in {names:?}");
    assert!(names.iter().any(|n| n == "PROP"), "no PROP chunk in {names:?}");
    assert!(names.iter().any(|n| n == "PRNT"), "no PRNT chunk in {names:?}");
}

#[test]
fn unknown_classes_and_properties_are_preserved() {
    let dir = temp_dir("preserve");
    let place = dir.join("custom.rbxl");

    let mut root = Node::new("DataModel", "Place");
    let mut workspace = Node::new("Workspace", "Workspace");
    let mut custom = Node::new("MyCustomClass", "Custom");
    custom
        .properties
        .insert("WeirdProp".to_string(), PropValue::Vector3([1.0, 2.0, 3.0]));
    custom.properties.insert(
        "Blob".to_string(),
        PropValue::BinaryString(bloxcli::tree::base64_encode(b"\x00\x01\x02hello")),
    );
    custom.properties.insert(
        "Tags".to_string(),
        PropValue::Tags(vec!["alpha".to_string(), "beta".to_string()]),
    );
    let mut attrs = std::collections::BTreeMap::new();
    attrs.insert("Health".to_string(), PropValue::Float64(42.0));
    attrs.insert("Title".to_string(), PropValue::String("Boss".to_string()));
    // Roblox stores instance attributes under this exact property name; the
    // binary serializer only writes it on reflection-known classes.
    let mut holder = Node::new("Part", "AttrHolder");
    holder.properties.insert("Attributes".to_string(), PropValue::Attributes(attrs));
    workspace.children.push(custom);
    workspace.children.push(holder);
    root.children.push(workspace);

    write_place(&place, &root).expect("write custom place");
    let (mut back, _) = read_place(&place).expect("read custom place");
    back.name = root.name.clone(); // place files do not store the root name
    let mut a = root.clone();
    let mut b = back.clone();
    strip_referents(&mut a);
    strip_referents(&mut b);
    // The synthesized DataModel root name is not stored in place files.
    b.name = a.name.clone();
    assert_eq!(a, b, "unknown class/property data was not preserved");

    // The same tree must survive a pass through the C++ engine.
    let engine = engine_path();
    let mut client = bloxcli::engine::EngineClient::spawn(&engine).expect("spawn engine");
    client.load(&back).expect("engine load");
    let snapshot = client.snapshot().expect("engine snapshot");
    client.shutdown();
    let mut s = snapshot.clone();
    strip_referents(&mut s);
    assert_eq!(b, s, "engine snapshot changed the tree");
}

#[test]
fn corrupt_and_invalid_files_are_rejected() {
    let dir = temp_dir("corrupt");
    let garbage = dir.join("garbage.rbxl");
    std::fs::write(&garbage, b"{\"not\": \"a real rbxl\"}").unwrap();
    let out = cli(&["validate", garbage.to_str().unwrap()]);
    assert!(!out.status.success(), "validate accepted a JSON file renamed to .rbxl");
    let out = cli(&["inspect", garbage.to_str().unwrap()]);
    assert!(!out.status.success(), "inspect accepted a corrupt file");
}

#[test]
fn failed_mutations_leave_the_file_untouched() {
    let dir = temp_dir("atomic");
    let place = dir.join("safe.rbxl");
    let place_s = place.to_str().unwrap().to_string();
    cli_ok(&["new", &place_s]);
    cli_ok(&["add", &place_s, "--parent", "Workspace", "--class", "Part", "--name", "Baseplate"]);
    let before = std::fs::read(&place).unwrap();

    // Missing parent.
    let out = cli(&["add", &place_s, "--parent", "Workspace/Nope", "--class", "Part"]);
    assert!(!out.status.success());
    // Wrong-typed value.
    let out = cli(&["set", &place_s, "--target", "Workspace/Baseplate", "--property", "Transparency", "--value", "banana"]);
    assert!(!out.status.success());
    // New property without an explicit type.
    let out = cli(&["set", &place_s, "--target", "Workspace/Baseplate", "--property", "BrandNew", "--value", "1"]);
    assert!(!out.status.success());
    // Overwrite protection.
    let out = cli(&["new", &place_s]);
    assert!(!out.status.success(), "new overwrote an existing file without --force");

    let after = std::fs::read(&place).unwrap();
    assert_eq!(before, after, "a failed command modified the place file");

    // The same new property succeeds with an explicit type.
    cli_ok(&["set", &place_s, "--target", "Workspace/Baseplate", "--property", "BrandNew", "--type", "Int64", "--value", "7"]);
    let (tree, _) = read_place(&place).unwrap();
    let part = tree.find_path("Workspace/Baseplate").unwrap();
    assert_eq!(part.properties.get("BrandNew"), Some(&PropValue::Int64(7)));
}

#[test]
fn set_rename_and_json_output() {
    let dir = temp_dir("rename");
    let place = dir.join("named.rbxl");
    let place_s = place.to_str().unwrap().to_string();
    cli_ok(&["new", &place_s]);
    cli_ok(&["add", &place_s, "--parent", "Workspace", "--class", "Part", "--name", "Old"]);
    cli_ok(&["set", &place_s, "--target", "Workspace/Old", "--property", "Name", "--value", "New"]);

    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json")
        .arg("--engine")
        .arg(engine_path())
        .args(["inspect", &place_s, "--path", "Workspace/New"])
        .output()
        .expect("run inspect --json");
    assert!(out.status.success(), "inspect --json failed: {}", String::from_utf8_lossy(&out.stderr));
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("inspect --json output");
    assert_eq!(parsed["tree"]["name"], "New");
    assert_eq!(parsed["tree"]["className"], "Part");
    assert!(Path::new(&place).is_file());
}
