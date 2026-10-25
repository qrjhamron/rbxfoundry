//! Multi-context client/server simulation sessions (Phase 9).
//!
//! `blox net start` launches a detached daemon (`bloxcli net-daemon`)
//! that owns N+1 engine processes: one authoritative server and one
//! client per simulated player. Each peer has its own DataModel copy
//! and its own Luau runtime context (server Scripts only run on the
//! server, LocalScripts only on their client, `Players.LocalPlayer`
//! only resolves on clients). Peers never share memory: RemoteEvent /
//! RemoteFunction calls are marshalled to JSON by the sending engine,
//! queued here with a configurable latency, and delivered through the
//! receiving engine's `runtime/netDeliver` RPC. Server state reaches
//! clients through explicit snapshot-diff replication
//! (create/set/delete ops), never the reverse — the server is
//! authoritative. The session directory `<place>.bloxnet/` holds the
//! socket, per-peer snapshots and the daemon log.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value as JsonValue};

use crate::protocol::engine::{node_from_value, EngineClient};
use crate::history::History;
use crate::rbxio::{read_place, write_place};
use crate::protocol::tree::{err, Error, Node, PropValue, Result};

pub const FIXED_DT: f64 = 1.0 / 60.0;

pub fn session_dir(place: &Path) -> PathBuf {
    let name = place
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "place".to_string());
    place.with_file_name(format!("{name}.bloxnet"))
}

pub fn socket_path(place: &Path) -> PathBuf {
    session_dir(place).join("session.sock")
}

// -------------------------------------------------------------- client ---

pub fn send_request(place: &Path, request: &JsonValue) -> Result<JsonValue> {
    let socket = socket_path(place);
    let stream = UnixStream::connect(&socket).map_err(|e| {
        Error(format!(
            "no live net session for '{}' ({}): {e}; start one with `blox net start`",
            place.display(),
            socket.display()
        ))
    })?;
    let mut writer = stream.try_clone().map_err(|e| Error(e.to_string()))?;
    let mut line = serde_json::to_string(request).map_err(|e| Error(e.to_string()))?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .map_err(|e| Error(e.to_string()))?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| Error(e.to_string()))?;
    if response.trim().is_empty() {
        return err("net daemon closed the connection without responding");
    }
    serde_json::from_str(&response).map_err(|e| Error(format!("bad daemon response: {e}")))
}

pub fn session_alive(place: &Path) -> bool {
    if !socket_path(place).exists() {
        return false;
    }
    send_request(place, &json!({"cmd": "ping"})).is_ok()
}

/// Spawns the detached daemon and waits for its socket to answer.
pub fn start_daemon(
    engine_path: &Path,
    place: &Path,
    players: usize,
    latency_ms: f64,
    rig: &str,
    force: bool,
) -> Result<JsonValue> {
    let dir = session_dir(place);
    if session_alive(place) {
        if !force {
            return err(format!(
                "a net session is already running for '{}' (stop it first, or pass force)",
                place.display()
            ));
        }
        let _ = send_request(place, &json!({"cmd": "stop", "commit": false}));
        std::thread::sleep(Duration::from_millis(200));
    }
    std::fs::create_dir_all(&dir).map_err(|e| Error(e.to_string()))?;
    let socket = socket_path(place);
    let _ = std::fs::remove_file(&socket);
    let log_file = std::fs::File::create(dir.join("daemon.log")).map_err(|e| Error(e.to_string()))?;
    let log_err = log_file.try_clone().map_err(|e| Error(e.to_string()))?;
    let exe = std::env::current_exe().map_err(|e| Error(e.to_string()))?;
    let mut cmd = std::process::Command::new("setsid");
    cmd.arg("-f")
        .arg(exe)
        .arg("--engine")
        .arg(engine_path)
        .arg("net-daemon")
        .arg(place)
        .arg("--players")
        .arg(players.to_string())
        .arg("--latency")
        .arg(latency_ms.to_string())
        .arg("--rig")
        .arg(rig)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_err));
    let status =
        cmd.status().map_err(|e| Error(format!("failed to launch net daemon (setsid): {e}")))?;
    if !status.success() {
        return err("net daemon launcher exited with an error; see daemon.log");
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if session_alive(place) {
            return send_request(place, &json!({"cmd": "state"}));
        }
        if Instant::now() > deadline {
            let log = std::fs::read_to_string(dir.join("daemon.log")).unwrap_or_default();
            return err(format!(
                "net daemon did not come up within 30s; daemon.log tail: {}",
                log.chars()
                    .rev()
                    .take(400)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// -------------------------------------------------------------- daemon ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Peer {
    Server,
    Client(usize),
}

struct InvokeBack {
    peer: Peer,
    invoke_id: i64,
}

struct Message {
    deliver_step: u64,
    target: Peer,
    path: String,
    from_player: Option<String>,
    args: JsonValue,
    invoke: Option<InvokeBack>,
}

#[derive(Clone)]
enum ReplOp {
    Create {
        parent_path: String,
        class_name: String,
        name: String,
        properties: JsonValue,
    },
    Set {
        path: String,
        property: String,
        value: JsonValue,
    },
    Delete {
        path: String,
    },
}

struct QueuedOp {
    deliver_step: u64,
    op: ReplOp,
}

#[derive(Default, Clone)]
struct PlayerInput {
    keys: Vec<String>,
    yaw: f64,
}

struct PeerLogs {
    entries: Vec<JsonValue>,
    cursor: usize,
    error_cursor: usize,
}

impl PeerLogs {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            error_cursor: 0,
        }
    }
}

pub struct NetDaemon {
    server: EngineClient,
    clients: Vec<EngineClient>,
    place: PathBuf,
    rig: String,
    player_names: Vec<String>,
    latency_steps: u64,
    step_count: u64,
    outbox: VecDeque<Message>,
    repl_queue: Vec<QueuedOp>,
    last_server_tree: Node,
    inputs: Vec<PlayerInput>,
    characters: JsonValue,
    server_logs: PeerLogs,
    client_logs: Vec<PeerLogs>,
    warnings: Vec<String>,
}

fn player_tagged(name: &str) -> JsonValue {
    json!({"$t": "Instance", "p": format!("Players/{name}")})
}

/// Builds name paths for every node in the tree (root excluded).
fn index_tree<'a>(node: &'a Node, prefix: &str, out: &mut HashMap<String, &'a Node>) {
    for child in &node.children {
        let path = if prefix.is_empty() {
            child.name.clone()
        } else {
            format!("{prefix}/{}", child.name)
        };
        out.insert(path.clone(), child);
        index_tree(child, &path, out);
    }
}

fn replicated(path: &str) -> bool {
    // Server-only services never replicate (Roblox semantics).
    let first = path.split('/').next().unwrap_or("");
    first != "ServerStorage" && first != "ServerScriptService"
}

/// Maps child-index paths ("0/1/2") to name paths ("Workspace/A/B") so
/// Ref properties can be translated into portable cross-tree form.
fn index_to_name_paths(
    node: &Node,
    index_prefix: &str,
    name_prefix: &str,
    out: &mut HashMap<String, String>,
) {
    for (i, child) in node.children.iter().enumerate() {
        let index_path = if index_prefix.is_empty() {
            i.to_string()
        } else {
            format!("{index_prefix}/{i}")
        };
        let name_path = if name_prefix.is_empty() {
            child.name.clone()
        } else {
            format!("{name_prefix}/{}", child.name)
        };
        out.insert(index_path.clone(), name_path.clone());
        index_to_name_paths(child, &index_path, &name_path, out);
    }
}

/// Ref values are child-index paths in the source tree; clients hold a
/// different tree, so replication rewrites them as name paths (the
/// engine and the RBXL writer both resolve the "name:" form).
fn translate_ref(value: &PropValue, index_to_name: &HashMap<String, String>) -> PropValue {
    if let PropValue::Ref(target) = value {
        if !target.is_empty() && !target.starts_with("name:") {
            if let Some(name_path) = index_to_name.get(target) {
                return PropValue::Ref(format!("name:{name_path}"));
            }
        }
    }
    value.clone()
}

fn diff_trees(old: &Node, new: &Node) -> Vec<ReplOp> {
    let mut index_to_name = HashMap::new();
    index_to_name_paths(new, "", "", &mut index_to_name);
    let mut old_map = HashMap::new();
    let mut new_map = HashMap::new();
    index_tree(old, "", &mut old_map);
    index_tree(new, "", &mut new_map);
    let mut creates: Vec<(usize, ReplOp)> = Vec::new();
    let mut sets: Vec<ReplOp> = Vec::new();
    let mut deletes: Vec<(usize, ReplOp)> = Vec::new();
    for (path, node) in &new_map {
        if !replicated(path) {
            continue;
        }
        match old_map.get(path) {
            None => {
                let parent_path = path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
                let mut props = serde_json::Map::new();
                for (key, value) in &node.properties {
                    if let Ok(v) = serde_json::to_value(translate_ref(value, &index_to_name)) {
                        props.insert(key.clone(), v);
                    }
                }
                creates.push((
                    path.matches('/').count(),
                    ReplOp::Create {
                        parent_path,
                        class_name: node.class_name.clone(),
                        name: node.name.clone(),
                        properties: JsonValue::Object(props),
                    },
                ));
            }
            Some(prev) => {
                if prev.class_name != node.class_name {
                    deletes.push((path.matches('/').count(), ReplOp::Delete { path: path.clone() }));
                    let parent_path = path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
                    let mut props = serde_json::Map::new();
                    for (key, value) in &node.properties {
                        if let Ok(v) = serde_json::to_value(translate_ref(value, &index_to_name)) {
                            props.insert(key.clone(), v);
                        }
                    }
                    creates.push((
                        path.matches('/').count(),
                        ReplOp::Create {
                            parent_path,
                            class_name: node.class_name.clone(),
                            name: node.name.clone(),
                            properties: JsonValue::Object(props),
                        },
                    ));
                } else {
                    for (key, value) in &node.properties {
                        if prev.properties.get(key) != Some(value) {
                            if let Ok(v) = serde_json::to_value(translate_ref(value, &index_to_name)) {
                                sets.push(ReplOp::Set {
                                    path: path.clone(),
                                    property: key.clone(),
                                    value: v,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    for path in old_map.keys() {
        if !replicated(path) {
            continue;
        }
        if !new_map.contains_key(path) {
            deletes.push((path.matches('/').count(), ReplOp::Delete { path: path.clone() }));
        }
    }
    creates.sort_by_key(|(depth, _)| *depth);
    deletes.sort_by_key(|(depth, _)| std::cmp::Reverse(*depth));
    let mut ops: Vec<ReplOp> = creates.into_iter().map(|(_, op)| op).collect();
    ops.extend(sets);
    ops.extend(deletes.into_iter().map(|(_, op)| op));
    ops
}

pub fn run_daemon(
    engine_path: &Path,
    place: &Path,
    players: usize,
    latency_ms: f64,
    rig: &str,
) -> Result<()> {
    let dir = session_dir(place);
    std::fs::create_dir_all(&dir).map_err(|e| Error(e.to_string()))?;
    if players == 0 || players > 8 {
        return err("net sessions support 1..=8 players");
    }
    let (mut node, _format) = read_place(place)?;
    let player_names: Vec<String> = (1..=players).map(|i| format!("Player{i}")).collect();

    // Inject Player instances so every peer's Players service matches.
    {
        let pos = node
            .children
            .iter()
            .position(|c| c.class_name == "Players")
            .unwrap_or_else(|| {
                node.children.push(Node::new("Players", "Players"));
                node.children.len() - 1
            });
        for name in &player_names {
            if !node.children[pos].children.iter().any(|c| &c.name == name) {
                node.children[pos].children.push(Node::new("Player", name));
            }
        }
    }

    let mut server = EngineClient::spawn(engine_path)?;
    server.load(&node)?;
    server.call(
        "runtime/begin",
        json!({"netContext": "server", "playerIndex": -1}),
    )?;

    let mut clients = Vec::new();
    for (i, name) in player_names.iter().enumerate() {
        let mut client = EngineClient::spawn(engine_path)?;
        client.load(&node)?;
        client.call(
            "runtime/begin",
            json!({"netContext": "client", "playerIndex": i, "playerName": name}),
        )?;
        clients.push(client);
    }

    let mut daemon = NetDaemon {
        server,
        clients,
        place: place.to_path_buf(),
        rig: rig.to_string(),
        player_names: player_names.clone(),
        latency_steps: (latency_ms / (FIXED_DT * 1000.0)).ceil() as u64,
        step_count: 0,
        outbox: VecDeque::new(),
        repl_queue: Vec::new(),
        last_server_tree: node.clone(),
        inputs: vec![PlayerInput::default(); players],
        characters: json!([]),
        server_logs: PeerLogs::new(),
        client_logs: (0..players).map(|_| PeerLogs::new()).collect(),
        warnings: Vec::new(),
    };

    // Start scripts first (in Roblox, scripts load before players
    // join, so PlayerAdded handlers connected at startup observe the
    // joins below).
    let started = daemon.server.call("runtime/runScripts", json!({}))?;
    daemon.absorb(Peer::Server, &started);
    daemon.harvest(Peer::Server, &started);
    for i in 0..daemon.clients.len() {
        let started = daemon.clients[i].call("runtime/runScripts", json!({}))?;
        daemon.absorb(Peer::Client(i), &started);
        daemon.harvest(Peer::Client(i), &started);
    }

    // Players join: PlayerAdded on every peer.
    for name in &player_names {
        let args = json!([player_tagged(name)]);
        let resp = daemon.server.call(
            "runtime/fireSignal",
            json!({"path": "Players", "signal": "PlayerAdded", "args": args}),
        )?;
        daemon.absorb(Peer::Server, &resp);
        daemon.harvest(Peer::Server, &resp);
        for i in 0..daemon.clients.len() {
            let resp = daemon.clients[i].call(
                "runtime/fireSignal",
                json!({"path": "Players", "signal": "PlayerAdded", "args": args}),
            )?;
            daemon.absorb(Peer::Client(i), &resp);
            daemon.harvest(Peer::Client(i), &resp);
        }
    }

    // Server physics spawns one character per player.
    let players_json: Vec<JsonValue> = player_names
        .iter()
        .map(|n| json!({"name": n, "rig": rig}))
        .collect();
    let begun = daemon
        .server
        .call("physics/beginMulti", json!({"players": players_json, "assetsDir": std::path::Path::new(place).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()}))?;
    daemon.characters = begun.get("characters").cloned().unwrap_or(json!([]));
    for name in &player_names {
        let resp = daemon.server.call(
            "runtime/fireSignal",
            json!({
                "path": format!("Players/{name}"),
                "signal": "CharacterAdded",
                "args": json!([{"$t": "Instance", "p": format!("Workspace/{name}")}]),
            }),
        )?;
        daemon.absorb(Peer::Server, &resp);
        daemon.harvest(Peer::Server, &resp);
    }

    // Initial replication (immediate): characters appear on clients.
    daemon.replicate(true)?;
    daemon.persist_snapshots()?;

    let socket = socket_path(place);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).map_err(|e| Error(e.to_string()))?;
    eprintln!(
        "net daemon ready for {} ({} players, latency {}ms)",
        place.display(),
        player_names.len(),
        latency_ms
    );
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        match daemon.handle(stream) {
            Ok(true) => break,
            Ok(false) => {}
            Err(e) => eprintln!("net daemon: request failed: {e}"),
        }
    }
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

impl NetDaemon {
    fn absorb(&mut self, peer: Peer, state: &JsonValue) {
        let logs = match peer {
            Peer::Server => &mut self.server_logs,
            Peer::Client(i) => &mut self.client_logs[i],
        };
        let peer_name = match peer {
            Peer::Server => "server".to_string(),
            Peer::Client(i) => format!("client{}", i + 1),
        };
        if let Some(entries) = state.get("logs").and_then(|v| v.as_array()) {
            for entry in entries.iter().skip(logs.cursor) {
                let mut e = entry.clone();
                e["peer"] = json!(peer_name);
                logs.entries.push(e);
            }
            logs.cursor = entries.len();
        }
        if let Some(entries) = state.get("errors").and_then(|v| v.as_array()) {
            for entry in entries.iter().skip(logs.error_cursor) {
                let mut e = entry.clone();
                e["peer"] = json!(peer_name);
                e["level"] = json!("error");
                logs.entries.push(e);
            }
            logs.error_cursor = entries.len();
        }
        if logs.entries.len() > 1000 {
            let excess = logs.entries.len() - 1000;
            logs.entries.drain(0..excess);
        }
    }

    /// Collects buffered outgoing remote calls from an engine response.
    fn harvest(&mut self, from: Peer, state: &JsonValue) {
        let Some(calls) = state.get("remoteCalls").and_then(|v| v.as_array()) else {
            return;
        };
        for call in calls {
            self.route(from, call);
        }
    }

    fn route(&mut self, from: Peer, call: &JsonValue) {
        let path = call.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kind = call.get("kind").and_then(|v| v.as_i64()).unwrap_or(-1);
        let invoke_id = call.get("invokeId").and_then(|v| v.as_i64()).unwrap_or(0);
        let args = call.get("args").cloned().unwrap_or(json!([]));
        let deliver_step = self.step_count + self.latency_steps;
        let target_player = |args: &JsonValue| -> Option<(usize, JsonValue)> {
            let first = args.as_array()?.first()?.clone();
            let p = first.get("p")?.as_str()?;
            let name = p.strip_prefix("Players/")?;
            let idx = self.player_names.iter().position(|n| n == name)?;
            let mut rest = args.as_array()?.clone();
            rest.remove(0);
            Some((idx, JsonValue::Array(rest)))
        };
        match (from, kind) {
            (Peer::Client(i), 0) => self.outbox.push_back(Message {
                deliver_step,
                target: Peer::Server,
                path,
                from_player: Some(self.player_names[i].clone()),
                args,
                invoke: None,
            }),
            (Peer::Client(i), 3) => self.outbox.push_back(Message {
                deliver_step,
                target: Peer::Server,
                path,
                from_player: Some(self.player_names[i].clone()),
                args,
                invoke: Some(InvokeBack {
                    peer: Peer::Client(i),
                    invoke_id,
                }),
            }),
            (Peer::Server, 1) => match target_player(&args) {
                Some((idx, rest)) => self.outbox.push_back(Message {
                    deliver_step,
                    target: Peer::Client(idx),
                    path,
                    from_player: None,
                    args: rest,
                    invoke: None,
                }),
                None => self
                    .warnings
                    .push(format!("FireClient on '{path}' had no valid target Player argument")),
            },
            (Peer::Server, 2) => {
                for i in 0..self.clients.len() {
                    self.outbox.push_back(Message {
                        deliver_step,
                        target: Peer::Client(i),
                        path: path.clone(),
                        from_player: None,
                        args: args.clone(),
                        invoke: None,
                    });
                }
            }
            (Peer::Server, 4) => match target_player(&args) {
                Some((idx, rest)) => self.outbox.push_back(Message {
                    deliver_step,
                    target: Peer::Client(idx),
                    path,
                    from_player: None,
                    args: rest,
                    invoke: Some(InvokeBack {
                        peer: Peer::Server,
                        invoke_id,
                    }),
                }),
                None => self
                    .warnings
                    .push(format!("InvokeClient on '{path}' had no valid target Player argument")),
            },
            (peer, other) => self.warnings.push(format!(
                "dropped remote call kind {other} from {peer:?} on '{path}' (invalid direction for its context)"
            )),
        }
    }

    fn engine(&mut self, peer: Peer) -> &mut EngineClient {
        match peer {
            Peer::Server => &mut self.server,
            Peer::Client(i) => &mut self.clients[i],
        }
    }

    fn deliver_due(&mut self) -> Result<()> {
        let mut pending = Vec::new();
        while let Some(msg) = self.outbox.pop_front() {
            if msg.deliver_step <= self.step_count {
                pending.push(msg);
            } else {
                self.outbox.push_front(msg);
                break;
            }
        }
        // The queue is roughly ordered; also scan the remainder.
        let mut rest: VecDeque<Message> = VecDeque::new();
        while let Some(msg) = self.outbox.pop_front() {
            if msg.deliver_step <= self.step_count {
                pending.push(msg);
            } else {
                rest.push_back(msg);
            }
        }
        self.outbox = rest;
        for msg in pending {
            self.deliver(msg)?;
        }
        Ok(())
    }

    fn deliver(&mut self, msg: Message) -> Result<()> {
        let from_path = msg
            .from_player
            .as_ref()
            .map(|n| format!("Players/{n}"));
        match (msg.target, msg.invoke) {
            (target, None) => {
                let mut params = json!({"path": msg.path, "args": msg.args});
                if let Some(fp) = &from_path {
                    params["fromPlayerPath"] = json!(fp);
                }
                let resp = self.engine(target).call("runtime/netDeliver", params)?;
                self.absorb(target, &resp);
                self.harvest(target, &resp);
            }
            (target, Some(back)) => {
                let mut params = json!({"path": msg.path, "args": msg.args});
                if let Some(fp) = &from_path {
                    params["fromPlayerPath"] = json!(fp);
                }
                let resp = self.engine(target).call("runtime/netInvokeHandler", params)?;
                let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                let mut completion = json!({"invokeId": back.invoke_id, "ok": ok});
                if ok {
                    completion["result"] = resp.get("result").cloned().unwrap_or(json!([]));
                } else {
                    completion["error"] = json!(
                        "RemoteFunction handler missing or errored (see the receiving peer's logs)"
                    );
                }
                let resp2 = self
                    .engine(back.peer)
                    .call("runtime/netCompleteInvoke", completion)?;
                self.absorb(back.peer, &resp2);
                self.harvest(back.peer, &resp2);
            }
        }
        Ok(())
    }

    fn replicate(&mut self, immediate: bool) -> Result<()> {
        let snap = self.server.call("runtime/snapshot", json!({}))?;
        let tree_json = snap
            .get("tree")
            .ok_or_else(|| Error("server snapshot missing tree".into()))?;
        let tree = node_from_value(tree_json)?;
        let ops = diff_trees(&self.last_server_tree, &tree);
        self.last_server_tree = tree;
        let deliver_step = if immediate {
            self.step_count
        } else {
            self.step_count + self.latency_steps
        };
        for op in ops {
            self.repl_queue.push(QueuedOp { deliver_step, op });
        }
        // Apply everything that is due, to every client.
        let mut remaining = Vec::new();
        let mut landed_characters: Vec<String> = Vec::new();
        for queued in std::mem::take(&mut self.repl_queue) {
            if queued.deliver_step > self.step_count {
                remaining.push(queued);
                continue;
            }
            for i in 0..self.clients.len() {
                let resp = match &queued.op {
                    ReplOp::Create {
                        parent_path,
                        class_name,
                        name,
                        properties,
                    } => {
                        let r = self.clients[i].call(
                            "runtime/addInstance",
                            json!({
                                "parentPath": parent_path,
                                "class": class_name,
                                "name": name,
                                "properties": properties,
                            }),
                        );
                        if let ReplOp::Create { name, parent_path, .. } = &queued.op {
                            if parent_path == "Workspace"
                                && self.player_names.iter().any(|n| n == name)
                            {
                                landed_characters.push(name.clone());
                            }
                        }
                        r
                    }
                    ReplOp::Set {
                        path,
                        property,
                        value,
                    } => self.clients[i].call(
                        "runtime/setProperty",
                        json!({"path": path, "property": property, "value": value}),
                    ),
                    ReplOp::Delete { path } => self.clients[i]
                        .call("runtime/removeInstance", json!({"path": path})),
                };
                if let Err(e) = resp {
                    self.warnings
                        .push(format!("replication to client{} failed: {e}", i + 1));
                }
            }
        }
        self.repl_queue = remaining;
        for name in landed_characters {
            for i in 0..self.clients.len() {
                let resp = self.clients[i].call(
                    "runtime/fireSignal",
                    json!({
                        "path": format!("Players/{name}"),
                        "signal": "CharacterAdded",
                        "args": json!([{"$t": "Instance", "p": format!("Workspace/{name}")}]),
                    }),
                )?;
                self.absorb(Peer::Client(i), &resp);
                self.harvest(Peer::Client(i), &resp);
            }
        }
        Ok(())
    }

    fn step_once(&mut self) -> Result<()> {
        self.step_count += 1;
        let inputs: Vec<JsonValue> = self
            .inputs
            .iter()
            .map(|input| {
                let has = |name: &str| input.keys.iter().any(|k| k == name);
                let forward = (if has("W") { 1.0 } else { 0.0 })
                    - (if has("S") { 1.0 } else { 0.0 });
                let right = (if has("D") { 1.0 } else { 0.0 })
                    - (if has("A") { 1.0 } else { 0.0 });
                json!({"move": [right, forward], "jump": has("Space"), "yaw": input.yaw})
            })
            .collect();
        let stepped = self.server.call(
            "physics/stepMulti",
            json!({"dt": FIXED_DT, "steps": 1, "inputs": inputs}),
        )?;
        self.characters = stepped.get("characters").cloned().unwrap_or(json!([]));
        self.absorb(Peer::Server, &stepped);
        self.harvest(Peer::Server, &stepped);
        for i in 0..self.clients.len() {
            let state = self.clients[i].call("runtime/step", json!({"dt": FIXED_DT, "steps": 1}))?;
            self.absorb(Peer::Client(i), &state);
            self.harvest(Peer::Client(i), &state);
        }
        self.deliver_due()?;
        self.replicate(false)?;
        Ok(())
    }

    fn state_json(&mut self) -> Result<JsonValue> {
        let server_state = self.server.call("runtime/state", json!({}))?;
        self.absorb(Peer::Server, &server_state);
        self.harvest(Peer::Server, &server_state);
        let mut client_states = Vec::new();
        for i in 0..self.clients.len() {
            let state = self.clients[i].call("runtime/state", json!({}))?;
            self.absorb(Peer::Client(i), &state);
            self.harvest(Peer::Client(i), &state);
            client_states.push(json!({
                "player": self.player_names[i],
                "time": state.get("time"),
                "threads": state.get("threads"),
            }));
        }
        Ok(json!({
            "playing": true,
            "mode": "net",
            "rig": self.rig,
            "step": self.step_count,
            "time": self.step_count as f64 * FIXED_DT,
            "latencySteps": self.latency_steps,
            "players": self.player_names,
            "characters": self.characters,
            "server": {"time": server_state.get("time"), "threads": server_state.get("threads")},
            "clients": client_states,
            "queuedMessages": self.outbox.len(),
            "warnings": self.warnings,
        }))
    }

    fn persist_snapshots(&mut self) -> Result<()> {
        let dir = session_dir(&self.place);
        let snap = self.server.call("runtime/snapshot", json!({}))?;
        let tree = node_from_value(
            snap.get("tree")
                .ok_or_else(|| Error("server snapshot missing tree".into()))?,
        )?;
        write_place(&dir.join("server.rbxl"), &tree)?;
        for i in 0..self.clients.len() {
            let snap = self.clients[i].call("runtime/snapshot", json!({}))?;
            let tree = node_from_value(
                snap.get("tree")
                    .ok_or_else(|| Error("client snapshot missing tree".into()))?,
            )?;
            write_place(&dir.join(format!("client{}.rbxl", i + 1)), &tree)?;
        }
        Ok(())
    }

    fn handle(&mut self, stream: UnixStream) -> Result<bool> {
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| Error(e.to_string()))?);
        let mut writer = stream;
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| Error(e.to_string()))?;
        let request: JsonValue = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = json!({"ok": false, "error": format!("bad request: {e}")});
                writeln!(writer, "{resp}").map_err(|e| Error(e.to_string()))?;
                return Ok(false);
            }
        };
        let cmd = request.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        let (response, stop) = match self.dispatch(cmd, &request) {
            Ok((resp, stop)) => (resp, stop),
            Err(e) => (json!({"ok": false, "error": e.to_string()}), false),
        };
        writeln!(writer, "{response}").map_err(|e| Error(e.to_string()))?;
        Ok(stop)
    }

    fn player_index(&self, request: &JsonValue) -> Result<usize> {
        if let Some(name) = request.get("player").and_then(|v| v.as_str()) {
            if let Some(i) = self.player_names.iter().position(|n| n == name) {
                return Ok(i);
            }
            // Also accept 1-based numbers as strings.
            if let Ok(n) = name.parse::<usize>() {
                if n >= 1 && n <= self.player_names.len() {
                    return Ok(n - 1);
                }
            }
            return err(format!("unknown player '{name}'"));
        }
        if let Some(n) = request.get("player").and_then(|v| v.as_i64()) {
            if n >= 1 && (n as usize) <= self.player_names.len() {
                return Ok((n - 1) as usize);
            }
        }
        Ok(0)
    }

    fn dispatch(&mut self, cmd: &str, request: &JsonValue) -> Result<(JsonValue, bool)> {
        match cmd {
            "ping" => Ok((json!({"ok": true, "pong": true}), false)),
            "state" => Ok((self.state_json()?, false)),
            "step" => {
                let steps = request
                    .get("steps")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    .max(0);
                for _ in 0..steps {
                    self.step_once()?;
                }
                self.persist_snapshots()?;
                Ok((self.state_json()?, false))
            }
            "input" => {
                let idx = self.player_index(request)?;
                if let Some(yaw) = request.get("yawDeg").and_then(|v| v.as_f64()) {
                    self.inputs[idx].yaw = yaw.to_radians();
                }
                let keys: Vec<String> = request
                    .get("keys")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|k| k.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let duration = request.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let steps = (duration * 60.0).round() as i64;
                self.inputs[idx].keys = keys;
                // Feed the client's own UserInputService too.
                {
                    let mut changes = json!({});
                    for key in &self.inputs[idx].keys {
                        changes[key] = json!(true);
                    }
                    let resp = self.clients[idx].call("runtime/keys", json!({"keys": changes}))?;
                    self.absorb(Peer::Client(idx), &resp);
                }
                for _ in 0..steps.max(0) {
                    self.step_once()?;
                }
                // Release after the requested duration.
                if !self.inputs[idx].keys.is_empty() {
                    let mut release = json!({});
                    for key in &self.inputs[idx].keys {
                        release[key] = json!(false);
                    }
                    let resp = self.clients[idx].call("runtime/keys", json!({"keys": release}))?;
                    self.absorb(Peer::Client(idx), &resp);
                    self.inputs[idx].keys.clear();
                }
                self.persist_snapshots()?;
                Ok((self.state_json()?, false))
            }
            "screenshot" => Ok((self.screenshot(request)?, false)),
            "logs" => {
                let mut all = Vec::new();
                all.extend(self.server_logs.entries.iter().cloned());
                for logs in &self.client_logs {
                    all.extend(logs.entries.iter().cloned());
                }
                Ok((json!({"ok": true, "logs": all}), false))
            }
            "stop" => {
                let commit = request
                    .get("commit")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if commit {
                    let snap = self.server.call("runtime/snapshot", json!({}))?;
                    let tree = node_from_value(
                        snap.get("tree")
                            .ok_or_else(|| Error("server snapshot missing tree".into()))?,
                    )?;
                    let mut history = History::open(&self.place)?;
                    history.record(&self.place)?;
                    write_place(&self.place, &tree)?;
                }
                Ok((json!({"ok": true, "stopped": true, "committed": commit}), true))
            }
            other => Err(Error(format!("unknown net command '{other}'"))),
        }
    }

    fn screenshot(&mut self, request: &JsonValue) -> Result<JsonValue> {
        let idx = self.player_index(request)?;
        let output = request
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error("screenshot requires 'output'".into()))?;
        let width = request.get("width").and_then(|v| v.as_i64()).unwrap_or(1280);
        let height = request.get("height").and_then(|v| v.as_i64()).unwrap_or(720);
        let name = self.player_names[idx].clone();
        let character = self
            .characters
            .as_array()
            .and_then(|chars| {
                chars
                    .iter()
                    .find(|c| c.get("name").and_then(|v| v.as_str()) == Some(name.as_str()))
            })
            .cloned()
            .unwrap_or(json!({}));
        let pos = character
            .get("position")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error("character not present".into()))?;
        let (cx, cy, cz) = (
            pos[0].as_f64().unwrap_or(0.0),
            pos[1].as_f64().unwrap_or(0.0),
            pos[2].as_f64().unwrap_or(0.0),
        );
        let yaw = self.inputs[idx].yaw;
        let (sy, cyaw) = yaw.sin_cos();
        let forward = (-sy, 0.0f64, -cyaw);
        let eye = (
            cx - forward.0 * 11.0,
            cy + 3.0,
            cz - forward.2 * 11.0,
        );
        let look = (cx, cy + 1.5, cz);
        // Render the client's own replicated view of the world.
        let snap = self.clients[idx].call("runtime/snapshot", json!({}))?;
        let tree = node_from_value(
            snap.get("tree")
                .ok_or_else(|| Error("client snapshot missing tree".into()))?,
        )?;
        self.clients[idx].load(&tree)?;
        let place_dir = self
            .place
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        // Composite this client's live GUI over its replicated world,
        // then restore the client's edit model from the place file.
        let probes = request.get("probes").cloned().unwrap_or(json!([]));
        let result = self.clients[idx].call("ui/render", json!({
            "width": width, "height": height, "outputPath": output,
            "assetsDir": place_dir, "placeDir": place_dir, "withScene": true,
            "camera": {"position": [eye.0, eye.1, eye.2], "lookAt": [look.0, look.1, look.2]},
            "probePixels": probes,
        }));
        let (edit_tree, _) = crate::rbxio::read_place(&self.place)?;
        self.clients[idx].load(&edit_tree)?;
        let result = result?;
        Ok(json!({
            "ok": true, "output": output, "width": width, "height": height,
            "player": name,
            "camera": {"position": [eye.0, eye.1, eye.2], "lookAt": [look.0, look.1, look.2]},
            "character": character,
            "device": result.get("device"),
            "quadCount": result.get("quadCount"),
            "probes": result.get("probes"),
        }))
    }
}
