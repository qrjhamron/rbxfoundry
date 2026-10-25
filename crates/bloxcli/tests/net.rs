//! Phase 9 integration tests: real multi-context client/server
//! simulation. Two simulated clients connect to one authoritative
//! server, each with its own DataModel, Luau context and character.
//! Assertions go through the public CLI and per-peer logs/snapshots —
//! never through shared state.

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
        "bloxcli --json {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    serde_json::from_slice(&out.stdout).expect("json output")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-net-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

struct Session(PathBuf);
impl Session {
    fn state(&self) -> serde_json::Value {
        cli_json(&["net", "state", self.0.to_str().unwrap()])["state"].clone()
    }
    fn logs(&self) -> Vec<serde_json::Value> {
        cli_json(&["net", "logs", self.0.to_str().unwrap()])["logs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    fn log_text(&self, peer: &str) -> String {
        self.logs()
            .iter()
            .filter(|e| e["peer"].as_str() == Some(peer))
            .map(|e| {
                format!(
                    "{} {}",
                    e["level"].as_str().unwrap_or(""),
                    e["message"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn char_pos(&self, name: &str) -> (f64, f64, f64) {
        let state = self.state();
        let chars = state["characters"].as_array().expect("characters");
        let c = chars
            .iter()
            .find(|c| c["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("no character {name} in {state}"));
        let p = c["position"].as_array().unwrap();
        (
            p[0].as_f64().unwrap(),
            p[1].as_f64().unwrap(),
            p[2].as_f64().unwrap(),
        )
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = cli(&["net", "stop", self.0.to_str().unwrap()]);
    }
}

const SERVER_SCRIPT: &str = r#"
local players = game:GetService("Players")
local rs = game:GetService("ReplicatedStorage")
print("SERVER LocalPlayer is nil: " .. tostring(players.LocalPlayer == nil))
players.PlayerAdded:Connect(function(player)
    print("SERVER player joined: " .. player.Name)
    player.CharacterAdded:Connect(function(char)
        print("SERVER character added: " .. char.Name)
    end)
end)
local ping = rs:WaitForChild("Ping")
local echo = rs:WaitForChild("Echo")
local last = rs:WaitForChild("LastPing")
ping.OnServerEvent:Connect(function(player, msg)
    print("SERVER ping from " .. player.Name .. ": " .. tostring(msg))
    last.Value = player.Name .. ":" .. tostring(msg)
    ping:FireClient(player, "ack for " .. player.Name)
    ping:FireAllClients("broadcast")
end)
echo.OnServerInvoke = function(player, x)
    return x * 2, "from " .. player.Name
end
"#;

const CLIENT_SCRIPT: &str = r#"
local players = game:GetService("Players")
local lp = players.LocalPlayer
print("CLIENT local player: " .. lp.Name)
print("CLIENT is server context: false")
lp.CharacterAdded:Connect(function(char)
    print("CLIENT character added for " .. lp.Name .. ": " .. char.Name)
end)
local rs = game:GetService("ReplicatedStorage")
local ping = rs:WaitForChild("Ping")
local echo = rs:WaitForChild("Echo")
ping.OnClientEvent:Connect(function(msg)
    print("CLIENT event: " .. tostring(msg))
end)
ping:FireServer("hello")
local doubled, who = echo:InvokeServer(21)
print("CLIENT echo: " .. tostring(doubled) .. " " .. tostring(who))
-- Try to cheat: change the ground locally. The server must not care.
local ground = workspace:WaitForChild("Ground")
ground.Color = Color3.new(1, 0, 0)
print("CLIENT cheated ground color locally")
"#;

fn build_place(dir: &std::path::Path) -> String {
    let file = dir.join("game.rbxl");
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Ground"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Size", "--value", "80,2,80"]);
    cli_ok(&["move", &f, "--target", "Workspace/Ground", "--position", "0,-1,0"]);
    cli_ok(&["set", &f, "--target", "Workspace/Ground", "--property", "Anchored", "--type", "Bool", "--value", "true"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "SpawnLocation", "--name", "Spawn"]);
    cli_ok(&["move", &f, "--target", "Workspace/Spawn", "--position", "0,0.5,10"]);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage", "--class", "RemoteEvent", "--name", "Ping"]);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage", "--class", "RemoteFunction", "--name", "Echo"]);
    cli_ok(&["add", &f, "--parent", "ReplicatedStorage", "--class", "StringValue", "--name", "LastPing"]);
    cli_ok(&["add", &f, "--parent", "ServerScriptService", "--class", "Script", "--name", "ServerMain"]);
    cli_ok(&["set", &f, "--target", "ServerScriptService/ServerMain", "--property", "Source",
             "--type", "String", "--value", SERVER_SCRIPT]);
    // A Beam with Ref properties: replication must translate its
    // child-index refs into portable form (regression: index refs are
    // not portable between server/client trees).
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "PostA"]);
    cli_ok(&["move", &f, "--target", "Workspace/PostA", "--position", "-6,3,0"]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "PostB"]);
    cli_ok(&["move", &f, "--target", "Workspace/PostB", "--position", "6,3,0"]);
    cli_ok(&["run", &f, "--duration", "0.1", "--commit", "--eval",
             "local a0 = Instance.new('Attachment') a0.Parent = workspace.PostA\nlocal a1 = Instance.new('Attachment') a1.Parent = workspace.PostB\nlocal b = Instance.new('Beam') b.Attachment0 = a0 b.Attachment1 = a1 b.Parent = workspace"]);
    cli_ok(&["add", &f, "--parent", "StarterPlayer", "--class", "LocalScript", "--name", "ClientMain"]);
    cli_ok(&["set", &f, "--target", "StarterPlayer/ClientMain", "--property", "Source",
             "--type", "String", "--value", CLIENT_SCRIPT]);
    f
}

#[test]
fn two_clients_server_authority_remotes_and_isolation() {
    let dir = temp_dir("two-clients");
    let f = build_place(&dir);
    cli_ok(&["net", "start", &f, "--players", "2", "--latency", "0"]);
    let session = Session(PathBuf::from(&f));

    // Both players and characters exist; PlayerAdded fired server-side.
    let state = session.state();
    assert_eq!(state["players"], serde_json::json!(["Player1", "Player2"]));
    assert_eq!(state["characters"].as_array().unwrap().len(), 2, "{state}");
    cli_ok(&["net", "step", &f, "--steps", "90"]);

    let server_logs = session.log_text("server");
    let c1_logs = session.log_text("client1");
    let c2_logs = session.log_text("client2");

    // Join + character lifecycle, LocalPlayer semantics.
    assert!(server_logs.contains("SERVER player joined: Player1"), "{server_logs}");
    assert!(server_logs.contains("SERVER player joined: Player2"), "{server_logs}");
    assert!(server_logs.contains("SERVER LocalPlayer is nil: true"), "{server_logs}");
    assert!(server_logs.contains("SERVER character added: Player1"), "{server_logs}");
    assert!(c1_logs.contains("CLIENT local player: Player1"), "{c1_logs}");
    assert!(c2_logs.contains("CLIENT local player: Player2"), "{c2_logs}");
    assert!(c1_logs.contains("CLIENT character added for Player1: Player1"), "{c1_logs}");

    // Script isolation: server script never ran on clients and vice versa.
    assert!(!c1_logs.contains("SERVER ping"), "server code leaked to client1: {c1_logs}");
    assert!(!c2_logs.contains("SERVER ping"), "server code leaked to client2: {c2_logs}");
    assert!(!server_logs.contains("CLIENT local player"), "client code leaked to server: {server_logs}");
    assert!(!server_logs.contains("CLIENT cheated"), "client code leaked to server: {server_logs}");

    // RemoteEvent client->server with sender identity, and the server's
    // replicated StringValue update is visible in the client snapshot.
    assert!(server_logs.contains("SERVER ping from Player1: hello"), "{server_logs}");
    assert!(server_logs.contains("SERVER ping from Player2: hello"), "{server_logs}");

    // Server->client RemoteEvent delivery (targeted and broadcast).
    assert!(c1_logs.contains("CLIENT event: ack for Player1"), "{c1_logs}");
    assert!(c2_logs.contains("CLIENT event: ack for Player2"), "{c2_logs}");
    assert!(c1_logs.contains("CLIENT event: broadcast"), "{c1_logs}");

    // RemoteFunction round trip with multiple return values.
    assert!(c1_logs.contains("CLIENT echo: 42 from Player1"), "{c1_logs}");
    assert!(c2_logs.contains("CLIENT echo: 42 from Player2"), "{c2_logs}");

    let net_dir = dir.join("game.rbxl.bloxnet");
    fn find_node<'a>(node: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
        if node["name"].as_str() == Some(name) {
            return Some(node);
        }
        for child in node["children"].as_array()? {
            if let Some(found) = find_node(child, name) {
                return Some(found);
            }
        }
        None
    }
    let client1 = cli_json(&["tree", net_dir.join("client1.rbxl").to_str().unwrap()]);
    let tree = &client1["tree"];
    let last_ping = find_node(tree, "LastPing").expect("LastPing replicated to client1");
    let value = last_ping["properties"]["Value"]["value"].as_str().unwrap_or("");
    assert!(value.starts_with("Player"), "replicated StringValue wrong: {value}");
    // The other client saw the same replicated world state.
    let client2 = cli_json(&["tree", net_dir.join("client2.rbxl").to_str().unwrap()]);
    let last_ping2 = find_node(&client2["tree"], "LastPing").expect("LastPing replicated to client2");
    assert_eq!(last_ping2["properties"]["Value"]["value"].as_str().unwrap_or(""), value);

    // The Beam (and its translated refs) replicated to both clients.
    assert!(find_node(&client1["tree"], "Beam").is_some(), "Beam replicated to client1");
    let client2_tree = cli_json(&["tree", net_dir.join("client2.rbxl").to_str().unwrap()]);
    assert!(find_node(&client2_tree["tree"], "Beam").is_some(), "Beam replicated to client2");
    let beam = find_node(&client1["tree"], "Beam").unwrap();
    // The ref must resolve inside the client's own tree (to an
    // Attachment under PostA), whatever textual form it takes there.
    fn resolve_index<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
        let mut cur = root;
        for part in path.split('/').filter(|s| !s.is_empty()) {
            cur = &cur["children"].as_array()?[part.parse::<usize>().ok()?];
        }
        Some(cur)
    }
    let target_path = beam["properties"]["Attachment0"]["value"].as_str().unwrap_or("");
    let target = resolve_index(&client1["tree"], target_path)
        .unwrap_or_else(|| panic!("Beam Attachment0 '{target_path}' resolves on client1"));
    assert_eq!(target["className"].as_str(), Some("Attachment"), "ref target: {target}");
    assert_eq!(target["name"].as_str(), Some("Attachment"));

    // Server authority: the client's local Ground colour cheat never
    // reaches the server (nor the other client).
    let server_tree = cli_ok(&["inspect", net_dir.join("server.rbxl").to_str().unwrap(),
                               "--path", "Workspace/Ground"]);
    assert!(!server_tree.contains("1,0,0") && !server_tree.contains("255, 0, 0"),
            "client cheat reached the server: {server_tree}");

    // Independent character control: only Player1 walks.
    let (_, _, z1_before) = session.char_pos("Player1");
    let (_, _, z2_before) = session.char_pos("Player2");
    cli_ok(&["net", "input", &f, "--player", "1", "--keys", "W", "--duration", "1"]);
    let (_, _, z1_after) = session.char_pos("Player1");
    let (_, _, z2_after) = session.char_pos("Player2");
    assert!(z1_before - z1_after > 8.0, "Player1 walked forward: {z1_before} -> {z1_after}");
    assert!((z2_before - z2_after).abs() < 0.5, "Player2 stayed put: {z2_before} -> {z2_after}");

    // Client screenshot renders that client's replicated world.
    let shot = dir.join("client1.png");
    let result = cli_json(&["net", "screenshot", &f, "--player", "1",
                            "--output", shot.to_str().unwrap()]);
    assert_eq!(result["result"]["ok"], true, "{result}");
    let bytes = std::fs::read(&shot).expect("screenshot file");
    assert!(bytes.len() > 1000 && &bytes[0..4] == b"\x89PNG", "real PNG screenshot");
    cli_ok(&["net", "stop", &f]);
}

#[test]
fn latency_delays_remote_delivery_deterministically() {
    let dir = temp_dir("latency");
    let f = build_place(&dir);
    // 170ms at 60Hz => 11 steps of latency (ceil(170 / 16.67) = 11).
    cli_ok(&["net", "start", &f, "--players", "1", "--latency", "170"]);
    let session = Session(PathBuf::from(&f));
    cli_ok(&["net", "step", &f, "--steps", "3"]);
    let early = session.log_text("server");
    assert!(!early.contains("SERVER ping"), "ping arrived before latency elapsed: {early}");
    let state = session.state();
    assert!(state["queuedMessages"].as_i64().unwrap_or(0) > 0, "messages queued: {state}");
    cli_ok(&["net", "step", &f, "--steps", "40"]);
    let later = session.log_text("server");
    assert!(later.contains("SERVER ping from Player1: hello"), "{later}");
    cli_ok(&["net", "stop", &f]);
}
