# Phase 25 — Persistent Studio Sessions

Implemented (`INTERNAL_VERIFIED`; external Studio parity `UNTESTED`).

## Design
`blox studio start <place>` launches a detached daemon
(`bloxcli studio-daemon`, via `setsid`) that owns one long-lived C++
`blox-engine` process holding the authoritative edit DataModel in
memory. CLI invocations and MCP attach over a Unix socket in
`<place>.bloxstudio/` (stateless one-line JSON protocol; the same
pattern as play/net/debug daemons). There is no second DataModel and
the place file is never rewritten except by explicit save.

## Commands
```
blox studio start <file> [--force]
blox studio status|attach <file>
blox studio save <file> [--force]
blox studio stop <file> [--save] [--force]
blox studio undo|redo <file>
blox studio get <file> --target <path>
blox studio set <file> --target <path> --property <name> --value <v> [--type <T>]
blox studio add <file> --parent <path> --class <Class> --name <Name>
blox studio delete <file> --target <path>
blox studio select <file> --targets <path,path>
blox studio camera <file> [--position x,y,z --look-at x,y,z --fov deg]
```
MCP tools: `studio_session_start/status/get/set/save/stop` attach to
the same daemon (see `mcp/schemas.rs`).

## State and guarantees
- Editor state in the daemon: tree, selection, camera, dirty flag,
  sequence number, in-memory undo/redo snapshot stacks (100 deep).
- Every mutation writes an atomic recovery snapshot
  (`<place>.bloxstudio/recovery.rbxl`, seq in `session.json`).
- A killed daemon is detected (socket stale, `status` reports it);
  restarting restores unsaved edits from recovery (`recovered: true`)
  without overwriting the original file.
- Save uses the shared `app::session::write_mutation` (history +
  atomic write) and refuses to overwrite a place file whose content
  hash changed since load/last save unless `--force`.
- A second `studio start` against a live session fails instead of
  creating a competing writer; `--force` stops the old daemon first.

## Tests
`crates/bloxcli/tests/phase25.rs`: 100 sequential CLI ops on one
engine (same session id), unsaved isolation from the file,
selection/camera persistence, undo/redo across processes, explicit
save round-trip, MCP observing identical values, kill -9 recovery.
Play/net daemons are unchanged and still covered by their suites.

## Not implemented
- Multi-writer sessions or remote (non-local) attach.
- Editor state beyond selection/camera (e.g. open script tabs).
- Runtime-mode state inside the studio session (use `blox play`/`net`).
