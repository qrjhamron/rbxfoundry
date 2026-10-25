# Phase 28 — Multiplayer, Replication & Camera

External Studio parity: `UNTESTED`.

## Existing architecture (verified by net/phase24 suites)
- Detached server + up to 4 client engine processes (`blox net`),
  each with its own DataModel, camera, input and GUI state
  (`LocalPlayer` present on clients, nil on server).
- Replication: per-client visible-tree snapshots with handshake,
  declared property mirrors (type-checked, unknown props rejected),
  instance creation/destruction/reparenting, attribute/tag sync,
  deletion propagation and per-peer sequence/latency simulation
  (`--latency`, deterministic virtual clock).
- RemoteEvents (FireServer/FireClient/FireAllClients) and
  RemoteFunctions with **yielding handlers**: invocation ids,
  pending-thread parking (`pending_invokes`), resume on completion,
  multiple concurrent invokes, context checks (InvokeServer only
  from clients, InvokeClient only from server). No engine-wide block.
- Network ownership: server-authoritative physics with
  `AssemblyNetworkOwnership` handling as documented in
  COMPATIBILITY.md; clients cannot write server properties outside
  replication mirrors.

## Camera (PARTIAL)
Per-client `CurrentCamera` with Scriptable/Fixed camera types,
CFrame/FieldOfView control from Luau and `play screenshot --camera`
framing presets. Roblox PlayerModule-style default cameras
(first/third person follow, zoom, occlusion modes) are clean-room
partial: character-relative follow exists in play sessions; the full
default camera module behaviours are not reproduced.

## Remainder
Baseline resync after arbitrary (non-declared) property divergence,
prediction/reconciliation beyond ownership rules, and full camera
occlusion modes remain open. See COMPATIBILITY.md for statuses.
