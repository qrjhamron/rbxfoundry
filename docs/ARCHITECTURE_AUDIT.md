# RBXFoundry Architecture Audit

Date: 2026-10-25 (pre-refactor baseline, commit `fc3fb71`).
Scope: every C++ source/header under `engine/`, every Rust source under
`crates/bloxcli/src/`, build files, tests. No behaviour was changed to
produce this audit.

## 1. Baseline (measured, this host)

| Check | Result |
| --- | --- |
| C++ build (`cmake --build build`, Ninja, Release) | clean, no work |
| CTest | 2/2 passed (`engine_datamodel`, `engine_scene`) |
| Rust (`cargo test --workspace`) | **100 passed, 0 failed** (21 integration files + lib/bin) |
| Bench (`scripts/bench.sh`, 200-part place, Lavapipe) | render 800x600 0.42 s; ui 1920x1080 0.41 s; export 0.41 s; validate 0.17 s; tree 0.21 s; play step x240 0.16 s |

Per-file Rust test counts: animation 4, compat 1, mcp 5, net 2,
phase13 1, phase14 6, phase16 4, phase17 5, phase18 4, phase19 4,
phase2 6, phase21 5, phase22 2, phase23 3, phase24 3, phase4 8,
play 3, render 17, roundtrip 6, runtime 6, visual 5. Total 100.

## 2. Source inventory (lines, pre-refactor)

### C++ (`engine/`, 14,976 lines incl. headers)

| File | Lines | Role today |
| --- | ---: | --- |
| `src/runtime.cpp` | 4,561 | Luau VM: userdata, marshalling, Instance bindings, signals, tweens, services, remotes, animation, pathfinding, datatypes, scheduler, debugger, `Runtime` impl |
| `src/ui.cpp` | 1,832 | FreeType text (decode, fallback chain, atlas, measure) + GUI layout + RichText + software rasterizer |
| `src/renderer.cpp` | 1,600 | Vulkan device/pipelines, scene render, PNG encode/decode, image diff, UI pipeline |
| `src/main.cpp` | 1,570 | `main()` + one 1,400-line dispatch loop (63 `method ==` branches) over every IPC domain |
| `src/physics.cpp` | 1,227 | Jolt world build, constraints, character rig/pose, raycast/queries |
| `src/datamodel.cpp` | 914 | Reflection table (~300 lines), instance tree, property sync, hierarchy, transforms |
| `src/scene.cpp` | 766 | DataModel -> `Scene` extraction, part geometry, SurfaceGui |
| `src/mesh.cpp` | 406 | Mesh/texture asset loading |
| `src/terrain.cpp` | 292 | Voxel grid codec + sculpt ops |
| `src/json.cpp` | 252 | Minimal JSON value + parser |
| `src/script_tools.cpp` | 127 | Luau Frontend bridge |
| `include/blox/*.hpp` (11 files, flat) | 1,520 | All public headers in one flat namespace directory |

### Rust (`crates/bloxcli/src/`, 9,314 lines)

| File | Lines | Role today |
| --- | ---: | --- |
| `main.rs` | 3,219 | Binary entry, global flags, help text, `CmdArgs`, engine session helpers, **and ~40 `cmd_*` command implementations** |
| `net.rs` | 1,086 | Client/server session daemon + client helpers |
| `mcp.rs` | 754 | MCP stdio server, sandbox, engine session, all tool dispatch + tool schemas |
| `convert.rs` | 685 | Canonical tree <-> WeakDom, reflection coercion |
| `tree.rs` | 613 | Canonical tree / `PropValue` wire contract, value parsing |
| `play.rs` | 406 | Playtest daemon + client helpers |
| `engine.rs` | 316 | Engine JSON-RPC client |
| `rbxio.rs` | 270 | `.rbxl/.rbxlx/.rbxm` I/O, atomic writes |
| `diff.rs` | 252 | Semantic DataModel diff, image diff client |
| `completion.rs` | 234 | Luau completion |
| `integrity.rs` | 216 | Export guard |
| `debug.rs` | 192 | Debugger daemon |
| `history.rs` | 180 | Undo/redo snapshots |
| `templates.rs` | 101 | Place templates |
| `lib.rs` | 13 | Flat module list |

Tests: 21 files, 5,098 lines. Engine tests: `test_datamodel.cpp` (204),
`test_scene.cpp` (205).

## 3. Findings

### 3.1 Oversized files and functions

- `engine/src/runtime.cpp` (4,561 lines, ~247 functions in one anonymous
  namespace + `Runtime::Impl`). One translation unit owns at least eight
  responsibilities (see section map below). Any change recompiles all of
  it; reviewers cannot isolate scheduler vs binding changes.
- `engine/src/main.cpp`: `main()` is ~1,420 lines. It owns process state
  (`model`, `runtime_model`, `runtime`, `physics`, `renderer`,
  `outgoing_remotes`, `initialized`, `has_model`) and dispatches every
  method family inline. Adding a method means editing the god loop.
- `crates/bloxcli/src/main.rs` (3,219 lines): ~40 `cmd_*` functions plus
  parsing, help, session and mutation plumbing in the binary crate root.
  The library (`lib.rs`) cannot reuse any command logic; MCP duplicates
  parts of it (see 3.5).
- `engine/src/ui.cpp` (1,832): text engine (lines ~1-440) and layout
  engine (~441-1,832) share one file and one anonymous namespace only
  through `measure_text`/`rasterize_text`.
- `engine/src/renderer.cpp` (1,600): Vulkan rendering, PNG file I/O and
  the Phase-18 image-diff metric are independent concerns in one file;
  `render()` alone spans ~540 lines.
- `engine/src/datamodel.cpp`: the static reflection table occupies the
  first ~390 lines before any `DataModel` logic.

Runtime section map (`runtime.cpp`, by banner comment): userdata
(30-101), `Runtime::Impl` (102-201), signal firing (230-259), helpers
(260-364), network marshalling (365-593), variant<->Lua (594-857),
instances (858-1182), instance methods (1183-1495), signals (1496-1564),
tweens (1565-1647), services (1648-1757), `task.*` (1758-1800),
`Instance.new`/`require` (1801-1883), remotes (1884-2087), animation
(2088-2228), ContextActionService (2229-2298), PhysicsService
(2299-2377), PathfindingService (2378-~2660), Humanoid/Animator
(~2660-2860), terrain bindings, TextService, globals, datatypes
(Vector3/CFrame/Color3/Vector2/UDim/EnumItem), scheduler + debugger
hooks, registration, `Runtime` methods (from ~4040).

### 3.2 God objects / excessive branching

- C++ IPC: 63 string comparisons on `method` in one loop, nested
  `runtime/` (15), `physics/` (10), `debug/`, `terrain/`, `instance/`
  families. No per-domain handler boundary; every handler captures the
  same locals via lambdas (`respond`, `respond_error`, `runtime_state`).
- `Runtime::Impl` aggregates threads, signals, tweens, networking,
  animation tracks, debugger, pathfinding, GUI focus, key state and
  humanoid states. It is the runtime god object; bindings, scheduler and
  debugger all mutate it directly (acceptable internally, but the file
  layout hides the boundaries).
- Rust `run()` dispatch: one match over ~30 command names delegating to
  functions that each re-parse flags with `parse_cmd`.

### 3.3 Mixed responsibilities

- `main.cpp` (C++): IPC framing, protocol handshake, DataModel ops,
  runtime session lifecycle, physics stepping glue (touch events ->
  Luau signals, Humanoid state sync), terrain ops, rendering, UI and
  validation. IPC handling is not separate from engine operations.
- `renderer.cpp`: PNG encoding (`write_png`, `load_png_rgba`,
  `image_diff`) is used by non-render callers (parity) but lives inside
  the Vulkan renderer TU; the UI pipeline is appended at line 1156 with
  a mid-file `#include "blox/ui.hpp"`.
- `scene.cpp` depends on `ui.hpp` (SurfaceGui rasterization) and
  `terrain.hpp`; `runtime.cpp` depends on `physics.hpp`, `terrain.hpp`,
  `ui.hpp`. Rendering-adjacent code thereby depends on GUI internals.
- Rust `main.rs`: `cmd_model` contains referent-rewriting algorithms
  (`display_to_index`, `rewrite_refs_for_model_export/import`) that are
  file-format logic, not CLI logic. `cmd_place` builds manifests;
  `cmd_parity` renders HTML. All sit beside argument parsing.
- Rust `mcp.rs`: transport (stdio loop), sandboxing, engine lifecycle,
  persistence, tool dispatch and tool JSON schemas in one file.

### 3.4 Dependencies and coupling (C++)

Include graph (public headers):

```
core:      json.hpp <- variant.hpp <- datamodel.hpp
datamodel: datamodel.hpp -> json, variant
rendering: renderer.hpp (leaf); scene.hpp -> datamodel, renderer
gui:       ui.hpp -> datamodel, renderer
physics:   physics.hpp -> datamodel
terrain:   terrain.hpp -> datamodel
scripting: runtime.hpp -> datamodel; script_tools.hpp (leaf)
brickcolor.hpp (leaf, header-only palette)
```

TU graph: `main.cpp` includes all 9 headers (fan-in hub).
`scene.cpp` -> brickcolor, terrain, ui. `renderer.cpp` -> ui (UI
pipeline). `runtime.cpp` -> physics, terrain, ui. `physics.cpp` ->
terrain (voxel collision). No circular includes exist today; the risk
is hub coupling through `main.cpp` and cross-domain includes from
`runtime.cpp`/`scene.cpp`, which are genuine runtime needs (Raycast,
SurfaceGui) and must become explicit narrow interfaces, not be hidden.

All engine code links into one static library `blox_engine_lib`;
CMake enforces no subsystem boundaries. `physics.cpp` needs
`-fno-rtti` (Jolt), currently a per-file property — a boundary marker
that a `physics` target should own.

### 3.5 Duplicated logic (Rust CLI vs MCP vs daemons)

- Persistence sequence `snapshot -> History::open/record ->
  write_place` is open-coded in `main.rs` (`mutate`, ~10 call sites),
  `mcp.rs` (`persist`, undo/redo), `play.rs` and `net.rs` daemon commit
  paths. One application service should own it.
- Engine session setup (`find_engine` + `EngineClient::spawn` + `load`)
  is repeated as `spawn_engine`/`load_session` in `main.rs`, lazily in
  `mcp.rs::engine()`, and inside each daemon (`play`, `net`, `debug`).
- MCP tool schemas are built with `serde_json::from_str(&format!(...))`
  + `unwrap()` (11 sites in `tool_definitions`): correct today, but a
  production panic path on unchecked string construction.
- `net.rs` (1,086) and `play.rs` (406) share daemon scaffolding
  (socket path, state file, request loop) with divergent copies.

### 3.6 Ownership, lifetime, global state

- C++ ownership is mostly sound: `DataModel` owns the tree via
  `unique_ptr<Instance>`, raw `Instance*` are non-owning observers,
  `Runtime`/`PhysicsWorld` borrow `DataModel&`. Hazards: `Runtime`
  exposes `impl_` and `PhysicsWorld` exposes `impl_` publicly ("for
  helpers"); `Renderer` uses raw `Impl*`. `scene.cpp` has a mutable
  `static int surface_counter` (process-global, non-atomic).
  `renderer.cpp` keeps `static` texture cache and `static UiPipeline`
  inside functions — hidden process lifetime, reused across renders.
  `ui.cpp` keeps a `thread_local` font-chain cache plus static
  `FT_Library`. These are documented performance caches, but their
  lifetime is implicit.
- Rust ownership is sound (no `unsafe` in `crates/bloxcli/src`).
  Production `unwrap/expect` sites: 23 outside tests, concentrated in
  `engine.rs` (child stdio handles — infallible after spawn),
  `mcp.rs` schema literals, `rbxio.rs:132` (`next().unwrap()` on a
  wrapper known non-empty), `main.rs` PNG header slicing after a length
  check. Each must be classified, not blanket-removed.

### 3.7 Allocations and copies

- `Json` (C++) stores arrays/objects behind `shared_ptr`; `Variant`
  equality compares `complex_value.dump()` strings. IPC handlers copy
  `Json params` per request and `runtime_state()` rebuilds log/error
  arrays on every call. Acceptable at current scale; noted, not changed
  without profiling.
- Rust `node_from_json` clones the tree `JsonValue` for Serde; every CLI
  mutation round-trips the whole tree through the engine. This is the
  protocol design (snapshot model), preserved as-is.
- Known prior optimisation (documented in BENCHMARKS.md): font-chain
  and glyph caches in `ui.cpp` cut a 60-label render from 4.50 s to
  1.64 s. The refactor must not regress this; caches move with the text
  module intact.

## 4. Proposed boundaries (implemented by this refactor)

### C++ (`engine/include/blox/<domain>/`, `engine/src/<domain>/`)

| Domain | Owns | May depend on |
| --- | --- | --- |
| `core` | `json`, `variant`, `brickcolor` | — |
| `datamodel` | instance tree, reflection, transforms | `core` |
| `reflection` | (folded into `datamodel`: `reflection.cpp` TU) | `core`, `datamodel` header |
| `terrain` | voxel grid + sculpt | `datamodel` |
| `physics` | Jolt world, character, queries | `datamodel`, `terrain` |
| `scripting` | Luau runtime (core/scheduler/debugger), bindings, `script_tools` | `datamodel`, `physics`, `terrain`, `gui` (layout for GUI events, narrow) |
| `gui` | layout, text, rasterize, validate | `datamodel`, `rendering` (scene types) |
| `rendering` | scene extraction, mesh, Vulkan renderer, image I/O | `datamodel`, `terrain`, `gui` (SurfaceGui) |
| `ipc` | JSON-RPC framing, `EngineServer`, per-domain handlers | all domains (composition only) |
| `runtime`/`studio`/`networking`/`animation`/`debugging` | No separate folders: these behaviours live inside `scripting` (scheduler, remotes, animation tracks, debugger) and `ipc` handlers. Creating empty domain folders for them would be scaffolding, so they are documented sub-boundaries within `scripting`/`ipc` instead. | — |

Rules enforced by CMake: one static library per domain with explicit
`target_link_libraries`; `main.cpp` links only `ipc` (composition
root). IPC handlers are `EngineServer` members split by domain;
`main()` only constructs the server and runs its stdio loop.

Splits (no algorithm changes, code moved verbatim where possible):
`datamodel.cpp` -> `datamodel.cpp` + `reflection.cpp` + `transforms.cpp`;
`ui.cpp` -> `gui/text.cpp` + `gui/layout.cpp` (+ software rasterizer in
`layout.cpp`); `renderer.cpp` -> `rendering/renderer.cpp` +
`rendering/image.cpp` (PNG + diff); `runtime.cpp` -> `scripting/runtime.cpp`
(scheduler, debugger, `Runtime` impl) + `scripting/bindings.cpp`
(Instance/datatype/service bindings, registration) sharing
`scripting/runtime_internal.hpp`; `main.cpp` -> `ipc/server.cpp` +
`ipc/handlers_*.cpp` + thin `src/main.cpp`.

### Rust (`crates/bloxcli/src/`)

| Module | Owns |
| --- | --- |
| `cli/` | argument parsing (`args`), help, dispatch to commands |
| `app/` | application services: engine session (`session`), atomic mutation with history (`mutation`) — shared by CLI, MCP and daemons |
| `commands/` | one file per command group (editing, place, render, ui, runtime, sessions, studio extras); pure delegation to `app`/`protocol` |
| `mcp/` | `server` (transport+sandbox+session), `tools` (dispatch), `schemas` (tool definitions) |
| `protocol/` | `engine` (JSON-RPC client), `tree` (canonical contract) |
| `session/` | `play`, `net`, `debug` daemons |
| `rbxio/` | `mod` (place/model I/O), `convert` (WeakDom conversion) |
| `validation/` | `integrity` export guard |
| `diagnostics/` | `diff`, `completion` |
| (root) | `history`, `templates` stay as small leaf modules |

`lib.rs` keeps legacy paths (`bloxcli::engine`, `::tree`, `::rbxio`,
`::integrity`, `::play`, `::net`, `::debug`, `::mcp`, ...) as
re-exports so integration tests and external callers do not break.
`main.rs` becomes a thin entrypoint delegating to `cli::run`.

## 5. Risks and non-goals

- No IPC protocol change (protocol `"1"`, all method names/payloads).
- No MCP tool name/schema change; no CLI flag change.
- No algorithm rewrites: Jolt stepping, Luau scheduling, layout,
  rendering and RBXL conversion move verbatim.
- Character/physics sync boundary stays explicit: physics writes
  CFrames into the runtime DataModel in `PhysicsWorld::step`; IPC
  physics handlers remain the only place that interleaves physics and
  runtime stepping.
