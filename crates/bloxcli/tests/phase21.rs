//! Phase 21 integration tests: Studio toolset — ungroup, bulk
//! property editing, grid/angle snapping, terrain editor operations,
//! and the edit-context command bar (`exec`).

use std::path::PathBuf;
use std::process::Command;

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

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-phase21-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn new_place(dir: &PathBuf) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    f
}

#[test]
fn ungroup_moves_children_to_parent() {
    let dir = temp_dir("ungroup");
    let f = new_place(&dir);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Model", "--name", "House"]);
    cli_ok(&["add", &f, "--parent", "Workspace/House", "--class", "Part", "--name", "Wall"]);
    cli_ok(&["add", &f, "--parent", "Workspace/House", "--class", "Part", "--name", "Roof"]);
    let result = cli_json(&["ungroup", &f, "--target", "Workspace/House"]);
    assert_eq!(result["moved"], 2, "{result}");
    let tree = cli_json(&["tree", &f, "--path", "Workspace"]);
    let text = format!("{tree}");
    assert!(text.contains("Wall") && text.contains("Roof"), "children survive: {tree}");
    assert!(!text.contains("House"), "container removed: {tree}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bulk_set_applies_to_all_targets() {
    let dir = temp_dir("bulkset");
    let f = new_place(&dir);
    for name in ["A", "B", "C"] {
        cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", name]);
    }
    let result = cli_json(&[
        "set", &f, "--targets", "Workspace/A,Workspace/B,Workspace/C",
        "--property", "Anchored", "--type", "Bool", "--value", "true",
    ]);
    assert_eq!(result["targets"].as_array().unwrap().len(), 3, "{result}");
    for name in ["A", "B", "C"] {
        let got = cli_json(&["get", &f, "--target", &format!("Workspace/{name}"), "--property", "Anchored"]);
        assert_eq!(got["value"]["value"], true, "{name} anchored: {got}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn move_and_rotate_snapping() {
    let dir = temp_dir("snap");
    let f = new_place(&dir);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "P"]);
    cli_ok(&["move", &f, "--target", "Workspace/P", "--delta", "3.4,0,0", "--snap", "2"]);
    let got = cli_json(&["get", &f, "--target", "Workspace/P", "--property", "Position"]);
    let pos = got["value"]["value"].as_array().unwrap();
    assert_eq!(pos[0].as_f64().unwrap(), 4.0, "3.4 snapped to nearest 2-stud grid: {got}");
    cli_ok(&["rotate", &f, "--target", "Workspace/P", "--angles", "13,0,0", "--snap-angle", "15"]);
    let cf = cli_json(&["get", &f, "--target", "Workspace/P", "--property", "CFrame"]);
    let text = format!("{cf}");
    // A 15-degree X rotation has a measurable Y/Z axis tilt; 13 degrees
    // snapped to 15 gives sin(15°)=0.259 components.
    assert!(text.contains("0.2588") || text.contains("0.259"), "rotation snapped to 15°: {cf}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn terrain_flatten_smooth_paint() {
    let dir = temp_dir("terrain");
    let f = new_place(&dir);
    cli_ok(&["terrain", "fill", &f, "--center", "0,0,0", "--size", "32,16,32", "--material", "Grass"]);
    let info = cli_json(&["terrain", "info", &f]);
    let before = info["result"]["voxels"].as_i64().unwrap();
    assert!(before > 200, "filled voxels (4-stud voxels): {info}");

    // Paint the grass to sand (material count changes, voxels stay).
    cli_ok(&["terrain", "paint", &f, "--center", "0,0,0", "--size", "32,16,32", "--material", "Sand"]);
    let info = cli_json(&["terrain", "info", &f]);
    assert_eq!(info["result"]["voxels"].as_i64().unwrap(), before, "paint keeps voxels");
    let materials = &info["result"]["materialsByEnumValue"];
    assert!(materials.get("1280").is_none(), "grass (1280) fully repainted: {info}");
    assert_eq!(materials.as_object().unwrap().len(), 1, "one material after paint: {info}");

    // Flatten to half height: strictly fewer voxels remain.
    cli_ok(&["terrain", "flatten", &f, "--center", "0,0,0", "--size", "32,16,32", "--height", "0", "--material", "Sand"]);
    let info = cli_json(&["terrain", "info", &f]);
    let after = info["result"]["voxels"].as_i64().unwrap();
    assert!(after < before, "flatten lowers the terrain: {before} -> {after}");
    assert!(after > 0, "flatten keeps the lower half: {info}");

    // Smoothing a solid block keeps it solid (majority rule) — no crash,
    // voxel count stays in a sane range.
    cli_ok(&["terrain", "smooth", &f, "--center", "0,0,0", "--size", "32,16,32"]);
    let info = cli_json(&["terrain", "info", &f]);
    let smoothed = info["result"]["voxels"].as_i64().unwrap();
    assert!(smoothed > 0 && smoothed <= before, "smooth stays bounded: {smoothed}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exec_command_bar_commits_to_edit_tree() {
    let dir = temp_dir("exec");
    let f = new_place(&dir);
    let result = cli_json(&[
        "exec", &f, "--code",
        "local p = Instance.new('Part')\np.Name = 'FromCommandBar'\np.Position = Vector3.new(1, 2, 3)\np.Parent = workspace\nprint('made ' .. p.Name)",
    ]);
    assert!(format!("{}", result["logs"]).contains("made FromCommandBar"), "{result}");
    // The change is committed to the place file (edit context).
    let tree = cli_json(&["tree", &f, "--path", "Workspace"]);
    assert!(format!("{tree}").contains("FromCommandBar"), "{tree}");

    // A failing command bar reports the error and saves nothing.
    let bad = cli(&["exec", &f, "--code", "error('boom')"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("boom"));
    let _ = std::fs::remove_dir_all(&dir);
}
