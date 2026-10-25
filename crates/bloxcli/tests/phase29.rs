//! Phase 29 tests: offline DataStoreService emulation (persistence
//! across engine restarts, UpdateAsync atomicity, key validation,
//! budget) and HttpService local APIs (JSON, GUID, no-network errors).
//! These are local emulations, never Roblox production services.

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") { if !p.is_empty() { return PathBuf::from(p); } }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(candidate.is_file(), "blox-engine not found at {}", candidate.display());
    candidate
}
fn cli_ok(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--engine").arg(engine_path()).args(args).output().expect("run bloxcli");
    assert!(out.status.success(), "bloxcli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8")
}
fn cli_json(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli")).arg("--json").arg("--engine").arg(engine_path()).args(args).output().expect("run bloxcli");
    assert!(out.status.success(), "bloxcli --json {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("json")
}
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-p29-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}
fn place_with_script(dir: &PathBuf, source: &str) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "Main"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value", source]);
    f
}

#[test]
fn datastore_persists_across_engine_restarts() {
    let dir = temp_dir("persist");
    let f = place_with_script(&dir, r#"
        local ds = game:GetService("DataStoreService"):GetDataStore("PlayerData")
        ds:SetAsync("gold", 42)
        print("inc", ds:IncrementAsync("gold", 8))
        ds:UpdateAsync("gold", function(old) return old + 100 end)
        print("gold", ds:GetAsync("gold"))
        print("removed", ds:RemoveAsync("temp") == nil)
        local ok, err = pcall(function() ds:GetAsync("") end)
        print("empty key errors", not ok)
        print("budget", game:GetService("DataStoreService"):GetRequestBudgetForRequestType(Enum.DataStoreRequestType.GetAsync))
    "#);
    let first = cli_json(&["run", &f, "--duration", "0.05"]);
    let logs = serde_json::to_string(&first).unwrap();
    assert!(logs.contains("gold 150"), "{logs}");
    assert!(logs.contains("empty key errors true"), "{logs}");
    assert!(logs.contains("budget 60"), "{logs}");
    // The store file exists next to the place and holds the value.
    let store = std::fs::read_to_string(dir.join("game.rbxl.bloxdata/datastores.json")).expect("datastore file");
    assert!(store.contains("150"), "store contents: {store}");
    // Second, independent engine process sees the persisted value.
    cli_ok(&["set", &f, "--target", "ServerScriptService/Main", "--property", "Source", "--type", "String", "--value",
        "local ds = game:GetService('DataStoreService'):GetDataStore('PlayerData')\nprint('restored gold', ds:GetAsync('gold'))"]);
    let second = cli_json(&["run", &f, "--duration", "0.05"]);
    let logs = serde_json::to_string(&second).unwrap();
    assert!(logs.contains("restored gold 150"), "second engine run restores: {logs}");
}

#[test]
fn http_service_local_apis_no_network() {
    let dir = temp_dir("http");
    let f = place_with_script(&dir, r#"
        local http = game:GetService("HttpService")
        local encoded = http:JSONEncode({name = "blox", values = {1, 2, 3}})
        print("encoded", encoded)
        local decoded = http:JSONDecode(encoded)
        print("decoded", decoded.name, decoded.values[2])
        local guid = http:GenerateGUID(false)
        print("guid shape", #guid == 36 and guid:sub(9, 9) == "-")
        local ok, err = pcall(function() return http:GetAsync("http://127.0.0.1/never") end)
        print("getasync blocked", not ok and string.find(tostring(err), "offline") ~= nil)
    "#);
    let result = cli_json(&["run", &f, "--duration", "0.05"]);
    let logs = serde_json::to_string(&result).unwrap();
    assert!(logs.contains("decoded blox 2"), "{logs}");
    assert!(logs.contains("guid shape true"), "{logs}");
    assert!(logs.contains("getasync blocked true"), "{logs}");
}
