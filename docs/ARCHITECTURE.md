# Architecture

RBXFoundry is two programs with one source of truth:

```
AI agent / user
      |  CLI (JSON)            MCP (stdio JSON-RPC)
      v                        v
  bloxcli (Rust)  --------  same binary, `mcp` subcommand
      |  versioned JSON-RPC over stdio pipes (protocol "1")
      v
  blox-engine (C++20)  — authoritative DataModel, Luau VM, Jolt
                         physics, Vulkan renderer, GUI engine
```

The pre-refactor audit (file sizes, coupling, god objects) is in
[ARCHITECTURE_AUDIT.md](ARCHITECTURE_AUDIT.md).

## C++ side (`engine/`)

Headers live under `include/blox/<domain>/`, sources under
`src/<domain>/`. CMake builds one static library per domain; the
`target_link_libraries` graph below is the enforced boundary — a domain
cannot link what is not listed.

```
core ──< datamodel ──< terrain ──< physics ──┐
              │  ▲                              │
              │  └── gui ──< rendering ──┐      ├──< scripting ──┐
              │                          └──────┴────────────────┴──< ipc ──< blox-engine (src/main.cpp)
```

| Library | Sources | May link |
| --- | --- | --- |
| `blox_core` | `src/core/json.cpp` (`core/json`, `core/variant`, `core/brickcolor`) | — |
| `blox_datamodel` | `src/datamodel/datamodel.cpp` (tree, properties, snapshot, validation), `reflection.cpp` (class/property defaults), `transforms.cpp` (move/rotate/scale/pivot, hierarchy) | core |
| `blox_terrain` | `src/terrain/terrain.cpp` (voxel grid, sculpt) | datamodel |
| `blox_physics` | `src/physics/physics.cpp` (Jolt world, character, raycast/queries; `-fno-rtti`) | datamodel, terrain, Jolt |
| `blox_gui` | `src/gui/text.cpp` (FreeType chains, glyph atlas, measure/rasterize), `src/gui/layout.cpp` (UDim2 layout, RichText, validation, software rasterizer) | datamodel |
| `blox_rendering` | `src/rendering/scene.cpp` + `mesh.cpp` (DataModel -> Scene), `renderer.cpp` (Vulkan scene + UI pipelines), `image.cpp` (PNG I/O, image diff) | datamodel, terrain, gui, Vulkan/PNG |
| `blox_scripting` | `src/scripting/runtime.cpp` (VM lifecycle, scheduler, debugger, `Runtime` API), `bindings.cpp` (Instance/datatype/service Luau bindings), `script_tools.cpp` (Luau Frontend) | datamodel, physics, terrain, gui, Luau |
| `blox_ipc` | `src/ipc/server.cpp` (JSON-RPC framing, dispatch order, protocol gate) + `handlers_*.cpp` per method family | all of the above |
| `blox-engine` | `src/main.cpp` (30 lines: `--version`/`--help`, construct `EngineServer`, run) | ipc only |

Key boundaries:

- **DataModel is the authoritative runtime state.** Edit and runtime
  models are separate `DataModel` copies owned by `ipc::EngineServer`;
  scripts/physics mutate only the runtime copy unless committed.
- **IPC is separate from engine operations.** Handlers
  (`handlers_datamodel`, `handlers_runtime`, `handlers_physics`,
  `handlers_debug`, `handlers_terrain`, `handlers_render`,
  `handlers_script`) only translate JSON <-> engine calls and return
  `EngineServer::Reply`; framing, the `initialize` gate and the
  has-model gate live in `server.cpp`. Dispatch order and every method
  name/payload/error code are unchanged (protocol `"1"`).
- **Physics sync boundary** is explicit: `PhysicsWorld::step` writes
  CFrames into the runtime DataModel; only the physics IPC handler
  interleaves physics stepping with `Runtime::step`, touch events and
  Humanoid state.
- **Luau bindings vs scheduling:** `bindings.cpp` owns all `lua_CFunction`
  bindings; `runtime.cpp` owns thread scheduling, signals plumbing,
  debugger hooks and registration. They share only
  `src/scripting/runtime_internal.hpp`.
- **GUI separable components:** typography (`text.cpp`), layout +
  validation + software rasterization (`layout.cpp`); `gui/ui.hpp` no
  longer includes the renderer header. The Vulkan UI pipeline stays in
  `rendering/renderer.cpp` and consumes a laid-out `UiScene`.
- **Networking** (RemoteEvent/Function routing) lives in the scripting
  runtime (`Runtime::deliver_remote` etc.) with transport owned by the
  Rust session daemons; replication logic never touches sockets.
- Internal seams are `*_internal.hpp` headers next to their sources
  (datamodel, gui text, rendering image, scripting runtime, ipc) —
  never installed, never public API.

`blox_engine_lib` remains as an INTERFACE umbrella over `blox_ipc` for
tests and external consumers.

## Rust side (`crates/bloxcli/src/`)

| Module | Responsibility |
| --- | --- |
| `main.rs` (7 lines) | Binary entrypoint: forwards to `bloxcli::cli::run_main` (also built as `blox`) |
| `cli/` | `args` (global flags, `CmdArgs`/`parse_cmd`, help), `mod` (dispatch only) |
| `app/` | `session`: engine spawn/load, `emit`, atomic history-recording `write_mutation`/`mutate` — shared by CLI, MCP and daemons |
| `commands/` | One file per command group: `editing`, `transform`, `place`, `render`, `runtime`, `sessions`, `terrain`, `model`. No parsing or persistence logic of their own beyond delegation |
| `mcp/` | `server` (stdio transport, sandbox, engine session, `persist` via `app::session::write_mutation`), `tools` (dispatch by domain), `schemas` (`tools/list` contract) |
| `protocol/` | `engine` (JSON-RPC client, typed wrappers), `tree` (canonical `Node`/`PropValue` wire contract) |
| `session/` | `play`, `net`, `debug` detached session daemons (unix sockets) |
| `rbxio/` | `mod` (`.rbxl/.rbxlx/.rbxm` I/O, atomic temp+rename writes), `convert` (tree <-> WeakDom, reflection coercion) |
| `validation/` | `integrity` pre-write export guard |
| `diagnostics/` | `diff` (semantic + image diff), `completion` (Luau completion) |
| `history.rs`, `templates.rs` | Undo/redo snapshots; place templates (small leaf modules) |

`lib.rs` keeps the pre-refactor flat paths (`bloxcli::engine`,
`::tree`, `::rbxio`, `::integrity`, `::play`, `::net`, `::debug`,
`::convert`, `::diff`, `::completion`) as re-exports — no copies — so
integration tests and external callers are unaffected.

Rules: CLI and MCP invoke the same underlying operations and the same
`app::session` persistence (snapshot -> history record -> atomic
write); file writes are atomic and guarded by
`validation::integrity`; the C++ engine is spawned per command or held
by a session daemon. No production panic paths were added; existing
`expect` sites are limited to process-spawn handles and static MCP
schema literals (audited in ARCHITECTURE_AUDIT.md §3.6).

## Before / after

| Metric | Before | After |
| --- | ---: | ---: |
| `engine/src/main.cpp` | 1,570 lines (god dispatch) | 30 lines |
| Largest C++ TU | `runtime.cpp` 4,561 | `bindings.cpp` 3,116 (bindings only); core runtime 1,271 |
| `crates/bloxcli/src/main.rs` | 3,219 lines | 7 lines |
| Largest Rust file | `main.rs` 3,219 | `commands/editing.rs` 606; `mcp.rs` 754 -> server 209 + tools 465 + schemas 94 |
| C++ build targets | 1 monolithic lib | 8 domain libs + IPC + umbrella |
| Empty/scaffold modules | — | none: every directory above has real sources |

## Verification (this refactor, this host)

- Clean Ninja build in a fresh directory: passes.
- CTest: 2/2 (`engine_datamodel`, `engine_scene`).
- Rust: 100/100 integration tests (same suites as baseline: roundtrip,
  mcp, render, visual, runtime, play, net, phase*, animation, compat).
- IPC smoke: `initialize`/`ping`/`datamodel/new`/`validate`/`shutdown`
  byte-identical shapes.
- Benchmarks (`scripts/bench.sh`, 200-part place): see
  [BENCHMARKS.md](BENCHMARKS.md) — no meaningful regression vs the
  pre-refactor baseline recorded in ARCHITECTURE_AUDIT.md §1.
- `cargo fix` clean (no warnings beyond the pre-existing dual-bin
  note). `rustfmt`/`clippy` are not installed in this environment and
  were not run; `cargo check` passes.

## Invariants

- No silent data loss: unknown classes/properties are preserved;
  unrepresentable content fails loudly before any file is written.
- Determinism: fixed-step physics, seeded-free rendering paths,
  reproducible screenshots on a fixed build.
- Offline: no network calls anywhere; `rbxassetid://` content resolves
  only to local files and is reported when missing.
- Existing projects remain readable without migration; RBXL/RBXM
  import/export, undo/redo and atomic writes are untouched algorithms
  moved verbatim.
