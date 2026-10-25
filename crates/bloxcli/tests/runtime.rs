//! Phase 5 integration tests: the official Luau VM executes real scripts
//! against a runtime copy of the DataModel. Script results are verified
//! through saved runtime snapshots, CLI reads, and rendered pixels — the
//! edit-mode place file must remain untouched unless committed.

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
    assert!(out.status.success(), "bloxcli --json {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("json output")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-lua-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn place_with_script(dir: &std::path::Path, source: &str) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source",
             "--type", "String", "--value", source]);
    f
}

#[test]
fn script_creates_part_and_moves_it_over_simulated_time() {
    let dir = temp_dir("create");
    let f = place_with_script(&dir, r#"
local p = Instance.new("Part")
p.Name = "Mover"
p.Size = Vector3.new(4, 2, 4)
p.Color = Color3.fromRGB(255, 80, 40)
p.CFrame = CFrame.new(0, 1, 0)
p.Anchored = true
p.Parent = workspace
for i = 1, 4 do
    task.wait(0.25)
    p.CFrame = p.CFrame * CFrame.new(0, 1, 0)
end
"#);
    let out = dir.join("after.rbxl");
    let json = cli_json(&["run", &f, "--duration", "1.5", "--step", "0.05",
                          "--save-runtime", out.to_str().unwrap()]);
    assert_eq!(json["ok"], true, "run reports no errors: {json}");
    // The script-created part exists in the runtime snapshot with the
    // post-wait height: started at Y=1, rose 4 studs over the waits.
    let part = cli_json(&["get", out.to_str().unwrap(), "--target", "Workspace/Mover"]);
    assert_eq!(part["instance"]["className"], "Part");
    let pos = cli_json(&["get", out.to_str().unwrap(), "--target", "Workspace/Mover", "--property", "Position"]);
    let y = pos["value"]["value"][1].as_f64().unwrap();
    assert!((y - 5.0).abs() < 0.01, "part rose to Y=5 via task.wait steps, got {y}");
    // Edit state is untouched: the original file has no Mover.
    let original = cli(&["get", &f, "--target", "Workspace/Mover"]);
    assert!(!original.status.success(), "runtime changes must not leak into edit state");
}

#[test]
fn events_gui_changes_and_logs_are_real() {
    let dir = temp_dir("events");
    let f = place_with_script(&dir, r#"
local gui = Instance.new("ScreenGui")
gui.Name = "RuntimeGui"
gui.Parent = game:GetService("StarterGui")
local label = Instance.new("TextLabel")
label.Name = "Status"
label.Text = "waiting"
label.Parent = gui
local seen = {}
workspace.ChildAdded:Connect(function(child)
    table.insert(seen, child.Name)
end)
local beacon = Instance.new("Part")
beacon.Name = "Beacon"
beacon.Parent = workspace
task.wait(0.1)
label.Text = "beacon seen: " .. table.concat(seen, ",")
print("label updated")
"#);
    // StarterGui has no ScreenGui by default in a fresh place; the script
    // creates its own, exactly like a real game script would.
    let out = dir.join("after.rbxl");
    let json = cli_json(&["run", &f, "--duration", "0.5",
                          "--save-runtime", out.to_str().unwrap()]);
    let logs = serde_json::to_string(&json["logs"]).unwrap();
    assert!(logs.contains("label updated"), "script print captured: {logs}");
    let label = cli_json(&["get", out.to_str().unwrap(),
                           "--target", "StarterGui/RuntimeGui/Status", "--property", "Text"]);
    assert_eq!(label["value"]["value"], "beacon seen: Beacon",
               "ChildAdded event drove a real GUI text change");
}

#[test]
fn require_caches_modules_and_detects_recursion() {
    let dir = temp_dir("require");
    let f = place_with_script(&dir, r#"
local mod = require(game:GetService("ReplicatedStorage"):WaitForChild("Lib"))
local again = require(game:GetService("ReplicatedStorage").Lib)
print("same module value:", mod == again, mod.answer)
mod.answer = 100
print("second read:", again.answer)
"#);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage", "--class", "ModuleScript", "--name", "Lib"]);
    cli_ok(&["set", &f, "--target", "ReplicatedStorage/Lib", "--property", "Source",
             "--type", "String", "--value", "print(\"module initialised\")\nreturn { answer = 42 }"]);
    let json = cli_json(&["run", &f, "--duration", "0.2"]);
    let logs = serde_json::to_string(&json["logs"]).unwrap();
    assert_eq!(logs.matches("module initialised").count(), 1,
               "module body executes exactly once (cache): {logs}");
    assert!(logs.contains("same module value: true 42"), "require returns the cached value: {logs}");
    assert!(logs.contains("second read: 100"), "cached module table is shared: {logs}");
}

#[test]
fn runtime_errors_carry_tracebacks_and_fail_the_run() {
    let dir = temp_dir("errors");
    let f = place_with_script(&dir, "local x = 1\nerror('deliberate failure')");
    let out = cli(&["--json", "run", &f, "--duration", "0.2"]);
    assert!(!out.status.success(), "script errors fail the run by default");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("deliberate failure"), "error message reported: {text}");
    assert!(text.contains("Main"), "error names the script: {text}");
    // --allow-errors downgrades to a warning exit code but still reports.
    let ok = cli_json(&["run", &f, "--duration", "0.2", "--allow-errors"]);
    assert_eq!(ok["ok"], false);
    assert_eq!(ok["errors"].as_array().unwrap().len(), 1);
    let traceback = ok["errors"][0]["traceback"].as_str().unwrap();
    assert!(traceback.contains("Main"), "traceback references the script: {traceback}");
}

#[test]
fn sandbox_blocks_host_access_and_runaway_scripts() {
    let dir = temp_dir("sandbox");
    // No filesystem/process access from Luau.
    let f = place_with_script(&dir, "print(io, os.execute, loadstring)");
    let json = cli_json(&["run", &f, "--duration", "0.1"]);
    let logs = serde_json::to_string(&json["logs"]).unwrap();
    assert!(logs.contains("nil nil nil"), "io/loadstring/os.execute are unavailable: {logs}");

    // A tight infinite loop is stopped by the instruction budget and the
    // run still completes (with the error reported).
    let dir2 = temp_dir("runaway");
    let f2 = place_with_script(&dir2, "while true do end");
    let started = std::time::Instant::now();
    let json = cli_json(&["run", &f2, "--duration", "0.2", "--allow-errors"]);
    assert!(started.elapsed().as_secs() < 20, "runaway script was interrupted promptly");
    let errors = serde_json::to_string(&json["errors"]).unwrap();
    assert!(errors.contains("execution limit"), "runaway stopped by budget: {errors}");
}

#[test]
fn heartbeat_task_spawn_and_property_signals() {
    let dir = temp_dir("signals");
    let f = place_with_script(&dir, r#"
local part = Instance.new("Part")
part.Name = "Watched"
part.Parent = workspace
local changes = {}
part.Changed:Connect(function(prop)
    table.insert(changes, prop)
end)
local frames = 0
game:GetService("RunService").Heartbeat:Connect(function(dt)
    frames += 1
end)
task.spawn(function()
    task.wait(0.2)
    part.Color = Color3.new(0, 1, 0)
    part.Name = "Renamed"
    -- Signal handlers are deferred to the scheduler (as in Roblox), so
    -- yield once more before reading what the handler recorded.
    task.wait(0.1)
    workspace:SetAttribute("frames", frames)
    workspace:SetAttribute("changes", table.concat(changes, ","))
end)
"#);
    let out = dir.join("after.rbxl");
    cli_json(&["run", &f, "--duration", "0.6", "--save-runtime", out.to_str().unwrap()]);
    let game = cli_json(&["get", out.to_str().unwrap(), "--target", "Workspace", "--property", "Attributes"]);
    let text = serde_json::to_string(&game).unwrap();
    assert!(text.contains("Color"), "Changed signal observed Color: {text}");
    assert!(text.contains("frames"), "Heartbeat ran during steps: {text}");
    // The watched part was renamed by the spawned thread.
    let renamed = cli(&["get", out.to_str().unwrap(), "--target", "Workspace/Renamed"]);
    assert!(renamed.status.success(), "task.spawn thread renamed the part");
}
