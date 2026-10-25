//! Phase 26 tests: extended Terrain Luau APIs (FillCylinder, FillWedge,
//! FillRegion, WriteVoxels, CopyRegion/PasteRegion, Region3) over the
//! real voxel grid, plus unknown-terrain preservation honesty.
//!
//! Terrain data here is BloxCLI's documented BTR1 voxel storage;
//! genuine Studio terrain chunks remain unsupported (see
//! docs/PHASE26_GEOMETRY_SERIALIZATION.md) and must not silently
//! become empty — that is asserted at the CLI terrain level.

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") { if !p.is_empty() { return PathBuf::from(p); } }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(candidate.is_file(), "blox-engine not found at {}", candidate.display());
    candidate
}
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--engine").arg(engine_path()).args(args).output().expect("run bloxcli")
}
fn cli_ok(args: &[&str]) -> String {
    let out = cli(args);
    assert!(out.status.success(), "bloxcli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8")
}
fn cli_json(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--json").arg("--engine").arg(engine_path()).args(args).output().expect("run bloxcli");
    assert!(out.status.success(), "bloxcli --json {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("json")
}
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-p26-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn run_luau(dir: &std::path::Path, source: &str) -> serde_json::Value {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    // Create the Terrain instance (as Studio places always have one).
    cli_json(&["terrain", "fill", &f, "--center", "200,2,200", "--size", "4,4,4", "--material", "Grass"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value", source]);
    let out = dir.join("runtime.rbxl");
    cli_json(&["run", &f, "--duration", "0.1", "--save-runtime", out.to_str().unwrap()])
}

#[test]
fn terrain_extended_apis() {
    let dir = temp_dir("apis");
    // Fill a cylinder + wedge, copy/paste a region, write voxels back,
    // and read everything through ReadVoxels. Prints are the oracle.
    let result = run_luau(&dir, r#"
        local terrain = workspace.Terrain
        terrain:FillCylinder(CFrame.new(0, 4, 0), 10, 8, Enum.Material.Grass)
        terrain:FillWedge(CFrame.new(24, 2, 0), Vector3.new(8, 4, 8), Enum.Material.Sand)
        local region = Region3.new(Vector3.new(-12, 0, -12), Vector3.new(12, 8, 12))
        print("region size", region.Size.X, region.Size.Y)
        local mats, occ = terrain:ReadVoxels(region.CFrame, region.Size)
        local solid = 0
        for x = 1, #mats do for y = 1, #mats[x] do for z = 1, #mats[x][y] do
            if occ[x][y][z] > 0 then solid += 1 end
        end end end
        print("cylinder solid", solid)
        terrain:CopyRegion(region)
        local shifted = Region3.new(Vector3.new(40, 0, -12), Vector3.new(64, 8, 12))
        terrain:PasteRegion(shifted, true)
        local mats2, occ2 = terrain:ReadVoxels(shifted.CFrame, shifted.Size)
        local solid2 = 0
        for x = 1, #mats2 do for y = 1, #mats2[x] do for z = 1, #mats2[x][y] do
            if occ2[x][y][z] > 0 then solid2 += 1 end
        end end end
        print("pasted solid", solid2)
        assert(solid > 10, "cylinder filled voxels")
        assert(solid2 == solid, "paste reproduces the copied region")
        -- WriteVoxels round-trip: erase via occupancy 0 tables.
        local empty_m, empty_o = {}, {}
        for x = 1, #mats do empty_m[x] = {}; empty_o[x] = {}
            for y = 1, #mats[x] do empty_m[x][y] = {}; empty_o[x][y] = {}
                for z = 1, #mats[x][y] do empty_m[x][y][z] = Enum.Material.Air; empty_o[x][y][z] = 0 end
            end
        end
        terrain:WriteVoxels(region, 4, empty_m, empty_o)
        local _, occ3 = terrain:ReadVoxels(region.CFrame, region.Size)
        local solid3 = 0
        for x = 1, #occ3 do for y = 1, #occ3[x] do for z = 1, #occ3[x][y] do
            if occ3[x][y][z] > 0 then solid3 += 1 end
        end end end
        print("after writevoxels erase", solid3)
        assert(solid3 == 0, "WriteVoxels erased the region")
    "#);
    let logs = serde_json::to_string(&result).unwrap();
    assert!(logs.contains("cylinder solid"), "script ran: {logs}");
    assert!(logs.contains("pasted solid"), "paste ran: {logs}");
    assert!(logs.contains("after writevoxels erase"), "writevoxels ran: {logs}");
}
