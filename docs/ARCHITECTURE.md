# Architecture

RBXFoundry is two programs with one source of truth:

```
AI agent / user
      |  CLI (JSON)            MCP (stdio JSON-RPC)
      v                        v
  bloxcli (Rust)  --------  same binary, `mcp` subcommand
      |  versioned JSON-RPC over stdio pipes
      v
  blox-engine (C++20)  — authoritative DataModel, Luau VM, Jolt
                         physics, Vulkan renderer, GUI engine
```

## Rust side (`crates/bloxcli/src`)

| Module | Responsibility |
| --- | --- |
| `main.rs` | CLI dispatch; every command emits strict JSON with `--json` |
| `tree.rs` | Canonical instance tree (`Node`/`PropValue`) — the wire + file model |
| `rbxio.rs` | Genuine `.rbxl`/`.rbxlx`/`.rbxm`/`.rbxmx` read/write (rbx-dom), atomic writes, XML referent pre-scan |
| `convert.rs` | Canonical tree <-> WeakDom conversion, reflection coercion, defaults |
| `integrity.rs` | Pre-write export guard (dangling refs, corrupt terrain, unresolved assets) |
| `diff.rs` | Semantic DataModel diff (Parity Lab) |
| `templates.rs` | Place templates (empty/baseplate/classic-baseplate) |
| `engine.rs` | Engine process client (JSON-RPC), typed wrappers over RPC |
| `play.rs` / `net.rs` / `debug.rs` | Detached session daemons (unix sockets): playtest, client/server sim, Luau debugger |
| `mcp.rs` | MCP tools — thin wrappers over the same engine/CLI operations, sandboxed to a project root |
| `completion.rs` | Reflection-database-driven Luau completion |
| `history.rs` | Undo snapshots for file mutations |

Rules: CLI and MCP invoke the same underlying operations; file writes
are atomic (temp + rename) and guarded by `integrity.rs`; the C++
engine is spawned per command or held by a session daemon.

## C++ side (`engine/`)

| File | Responsibility |
| --- | --- |
| `src/datamodel.cpp` + `include/blox/datamodel.hpp` | Instance tree, reflection metadata, property semantics (incl. Color<->BrickColor sync) |
| `src/runtime.cpp` | Luau VM runtime: scheduler, signals, services, Humanoid hooks, debugger (lua_breakpoint/single-step) |
| `src/script_tools.cpp` | Official Luau Frontend bridge (`script analyze`) |
| `src/physics.cpp` | Jolt integration, character controller, collision groups |
| `src/renderer.cpp` | Vulkan offscreen renderer, PNG I/O, image diff |
| `src/ui.cpp` | GUI layout + FreeType text (UTF-8, fallback chains, glyph atlas) |
| `src/scene.cpp`, `src/mesh.cpp`, `src/terrain.cpp` | Scene building, mesh loading, voxel terrain |
| `src/main.cpp` | JSON-RPC server: dispatches `datamodel/*`, `runtime/*`, `physics/*`, `terrain/*`, `ui/*`, `render/*`, `debug/*`, `script/*`, `parity/*` |
| `include/blox/brickcolor.hpp` | Shared BrickColor palette (generated from rbx_types data) |

The engine owns runtime state; edit/play isolation is structural
(edit model vs runtime model, explicit `--commit`).

## Invariants

- No silent data loss: unknown classes/properties are preserved;
  unrepresentable content fails loudly before any file is written.
- Determinism: fixed-step physics, seeded-free rendering paths,
  reproducible screenshots on a fixed build.
- Offline: no network calls anywhere; `rbxassetid://` content resolves
  only to local files and is reported when missing.
