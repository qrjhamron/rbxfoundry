# BloxCLI

A native Linux, CLI-only, offline-first Roblox Studio alternative designed
for AI coding agents.

> **Status: v0.2.0-beta.1; Phases 1–6, 8–20 implemented** (Phase 7, browser play, is
> intentionally skipped). Editing core, place templates, real headless
> Vulkan rendering, a Roblox GUI engine with RichText and nine-slice
> images, an **MCP server** for AI agents, the **official Luau runtime**,
> client/server simulation, and **Jolt-powered physics playtest** with
> R6/R15 characters. Example output:
> [castle-day.png](screenshots/castle-day.png) ·
> [hud-desktop.png](screenshots/hud-desktop.png) ·
> [hud-mobile.png](screenshots/hud-mobile.png) ·
> [play-r6.png](screenshots/play-r6.png) (regenerate scene/UI shots with
> `./scripts/make_screenshots.sh`). Everything is verified by internal
> automated tests only — this environment has no Roblox Studio, and no
> 1:1 parity is claimed. See [COMPATIBILITY.md](COMPATIBILITY.md).

## Architecture

```
                JSON-RPC 2.0 (newline-delimited, protocol "1")
  bloxcli (Rust)  ───────────────────────────────────────────►  blox-engine (C++20)
  ├─ CLI parsing          stdin/stdout pipes                    ├─ DataModel (source of truth)
  ├─ .rbxl/.rbxlx I/O     ◄───────────────────────────────────  ├─ reflection defaults + validation
  │   via rbx-dom crates                                        ├─ hierarchy ops + transform math
  └─ undo/redo snapshots (.bloxhistory/)                        └─ instance tree, typed properties
```

- **C++20 engine** (`engine/`): owns the DataModel at runtime. Instance
  tree, typed property values, reflection defaults for the building core,
  path resolution (duplicate sibling names are errors, never silent first
  matches), hierarchy operations, world-space CFrame transform math
  (Model pivots, bounding boxes), validation, and snapshot load/save.
  Built with CMake + Ninja. No third-party dependencies.
- **Rust CLI** (`crates/bloxcli/`): argument parsing, structured `--json`
  output for agents, undo/redo history, and place file I/O through the
  upstream [rbx-dom](https://github.com/rojo-rbx/rbx-dom) crates — the
  same serializers the Rojo ecosystem uses: `rbx_binary` 3.0.1,
  `rbx_xml` 3.0.1, `rbx_dom_weak` 4.2.0, `rbx_types` 3.1.0,
  `rbx_reflection_database` 3.0.1 (Roblox 741). Property types are
  coerced to Roblox's reflection types before writing, so files carry
  Studio-correct property types, and validation checks every property
  against the official reflection database.
- **IPC**: the CLI spawns `blox-engine` per command, performs a versioned
  `initialize` handshake (protocol `"1"`), loads the place into the
  engine, mutates it through the engine, then snapshots and writes the
  file back atomically (temp file + rename; a failed command never
  modifies the file). The engine is a persistent process for the whole
  session and also runs standalone until `shutdown` or EOF. Methods:
  `initialize`, `ping`, `datamodel/new`, `datamodel/load`,
  `datamodel/snapshot`, `instance/add`, `instance/get`, `instance/set`,
  `instance/clone`, `instance/delete`, `instance/reparent`,
  `instance/group`, `instance/find`, `instance/move`, `instance/rotate`,
  `instance/scale`, `instance/pivot`, `validate`,
  `render/screenshot`, `render/camera`, `ui/layout`, `ui/validate`,
  `ui/render`, `shutdown`.
- **Canonical tree JSON** is the shared contract between the two halves
  (see `crates/bloxcli/src/tree.rs` and `engine/include/blox/variant.hpp`).
  Supported value types: `Bool`, `Int32`, `Int64`, `Float32`, `Float64`,
  `String`, `BinaryString` (base64), `Vector3`, `Color3`, `CFrame`,
  `Enum`, `EnumItem`, `BrickColor`, `ContentId`, `Tags`, `Attributes`,
  `Vector2`, `Vector2int16`, `Vector3int16`, `UDim`, `UDim2`,
  `NumberRange`, `Rect`, `Ray`, `Faces`, `Axes`, `PhysicalProperties`,
  `ColorSequence`, `NumberSequence`, and `Ref` (stored as a child-index
  path and rewired to a real referent on write, so e.g.
  `Model.PrimaryPart` survives edits). A file containing any other
  property type fails loudly with the class, property, and type named —
  data is never silently dropped.
- **Undo/redo**: every mutating command first snapshots the place file
  into a sibling `<place>.bloxhistory/` directory (raw file snapshots +
  `state.json`, capped at 100). `undo`/`redo` restore snapshots only
  after they decode successfully. History is file-level, so it works
  across CLI processes; it is not stored inside the `.rbxl`.
- **Referents**: canonical referents are assigned in traversal order on
  load in both the Rust conversion and the engine, so an instance keeps
  its `#id` (shown by `tree`, returned by `get`/`find`) across a
  load/save of an unchanged tree. Structural edits can renumber later
  instances — name paths remain the stable address.

## Build (Linux)

Prerequisites (Ubuntu/Debian):

```bash
sudo apt-get install -y build-essential cmake ninja-build cargo rustc \
    libvulkan-dev libpng-dev libfreetype-dev glslang-tools mesa-vulkan-drivers
```

Rendering needs a Vulkan driver; Mesa's Lavapipe (`mesa-vulkan-drivers`)
provides software Vulkan on machines without a GPU. Shaders are compiled
to SPIR-V by `glslangValidator` at build time and embedded in the engine.

Build and test:

```bash
./scripts/build.sh   # CMake/Ninja for C++, Cargo for Rust
./scripts/test.sh    # CTest (engine) + Cargo integration tests
```

The CLI finds the engine via `--engine <path>`, `$BLOX_ENGINE`, a
`blox-engine` sitting next to the CLI binary, or `build/engine/blox-engine`.
The same binary is also installed as `blox`, so `blox get …` works as
written in the command list.

## Install (release tarball)

Download/extract the Linux tarball (built by `scripts/package-release.sh`,
also produced by CI), then:

```bash
tar -xzf rbxfoundry-0.2.0-beta.1-linux-x86_64.tar.gz
cd rbxfoundry-0.2.0-beta.1-linux-x86_64
./bin/bloxcli --version
./bin/bloxcli new myplace.rbxl --template baseplate
./bin/bloxcli render myplace.rbxl --output scene.png
```

The tarball is self-contained: release binaries plus bundled DejaVu
fonts under `bin/fonts/` (the engine also honours `$BLOX_FONT` and
`ui render --font <path>`). A Vulkan driver is still required for
rendering (Mesa Lavapipe works without a GPU). See
[docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) and
[docs/BENCHMARKS.md](docs/BENCHMARKS.md). Examples:
[examples/README.md](examples/README.md).

## Usage

```bash
B=./target/debug/bloxcli

$B new game.rbxl --name MyPlace           # place with the standard services
$B add game.rbxl --parent Workspace --class Part --name Crate
$B move game.rbxl --target Workspace/Crate --position "10,5,0"
$B clone game.rbxl --target Workspace/Crate --name Crate2
$B group game.rbxl --targets Workspace/Crate,Workspace/Crate2 --name Crates
$B rotate game.rbxl --target Workspace/Crates --angles "0,90,0"
$B set game.rbxl --target Workspace/Crates/Crate --property Material --value Wood
$B find game.rbxl --class Part
$B tree game.rbxl --props
$B validate game.rbxl
$B undo game.rbxl                         # un-does the last mutation
$B export game.rbxl game.rbxlx            # genuine binary <-> XML conversion

$B --json get game.rbxl --target Workspace/Crates --property WorldPivot

$B render game.rbxl --output scene.png --camera "30,25,30" --look-at "0,5,0"
$B camera game.rbxl                       # CurrentCamera / bounds / suggested framing
$B --json render game.rbxl --output s.png --probe "400,300;400,500"  # exact frame pixels

$B ui render game.rbxl --output hud.png --device mobile
$B ui inspect game.rbxl --json            # computed rects, visibility, render order
$B ui validate game.rbxl --json           # offscreen / clipped / zero-size elements
$B ui hit-test game.rbxl --x 250 --y 400 --json  # simulation only; no scripts run
```

Commands:

| Command | Behaviour |
|---|---|
| `new <file> [--name N] [--force]` | New place with the standard service set (Workspace, Lighting, ReplicatedStorage, ServerStorage, ServerScriptService, StarterGui, StarterPlayer, StarterPack, Teams, SoundService). Refuses to overwrite without `--force`. |
| `inspect <file> [--path P] [--props]` | Phase 1 alias of `tree`. |
| `add <file> --parent P --class C [--name N]` | Adds an instance with engine reflection defaults (BasePart family, MeshPart, Model, Folder, SpawnLocation, scripts, value objects, services…). Any class name is accepted. |
| `set <file> --target P --property N --value V [--type T]` | Sets a typed property. Type from the existing property, or `--type` for new ones (includes `UDim`, `UDim2`, `Vector2`, `NumberRange`, `Rect`, `Faces`, `Axes`, `PhysicalProperties`, `ColorSequence` as `t:r,g,b;…`, `NumberSequence` as `t:v,e;…`, `EnumItem` as `Type:value`). `Name` renames; `Position` stays in sync with `CFrame`. |
| `save <file>` | Loads, engine-validates, canonically rewrites. Refuses an invalid tree. |
| `export <in> <out> [--force]` | Converts `.rbxl` <-> `.rbxlx` (or re-encodes the same format). |
| `validate <file>` | Full decode + engine validation + reflection-database type validation. Exit 1 when invalid. |
| `diff <a> <b>` | Semantic, property-level DataModel diff of two places/models (Phase 18). Exit 1 when differences exist. |
| `imagediff <a.png> <b.png> [--heatmap H] [--tolerance F]` | Pixel metrics (MAE/RMSE/diff fraction) + optional heatmap (Phase 18). |
| `model export|inspect|import` | Single-root `.rbxm`/`.rbxmx` handling (Phase 17/19). |
| `place info|manifest <file>` | Place summary / offline sidecar manifest (`.manifest.json`). |
| `template list|inspect|compare` | Template registry; `compare` diffs a template against a supplied fixture file. |
| `parity dashboard` | Static HTML compatibility dashboard from `compat-report.json`. |
| `get <file> --target P [--property N]` | One instance (referent, properties, children) or one typed property. `Position`, `Name`, `ClassName` derived. |
| `tree <file> [--path P] [--depth N] [--props]` | Hierarchy with stable referents (`#id`). |
| `find <file> [--name N] [--class C] [--under P] [--contains]` | Search by exact/substring name and/or class. |
| `clone <file> --target P [--name N] [--parent P]` | Deep-copy with fresh referents. |
| `delete <file> --target P` | Deletes a subtree. Root deletion rejected. |
| `reparent <file> --target P --parent P2` | Moves an instance. Cycles/root moves rejected. |
| `group <file> --targets A,B,… [--name N] [--parent P]` | Groups under a new Model; world transforms preserved. Default parent: first target's parent. |
| `move <file> --target P (--delta 'x,y,z' [--relative] \| --position 'x,y,z')` | BasePart or whole Model. `--relative` = local axes; `--position` absolute (single BasePart). |
| `rotate <file> --target P --angles 'rx,ry,rz' [--world]` | Degrees, intrinsic XYZ. Parts about their centre, Models about their pivot. |
| `scale <file> --target P (--factor F \| --factors 'x,y,z' \| --size 'x,y,z')` | BasePart per-axis, or Model uniform about pivot (`Model:ScaleTo` semantics; non-uniform Model scale rejected). |
| `pivot <file> --target Model [--position 'x,y,z']` | Get effective pivot (explicit `WorldPivot`, else bounding-box centre) or set it. |
| `undo <file>` / `redo <file>` | History in `<file>.bloxhistory/` (100 snapshots). Fresh mutation clears redo. |
| `render <file> --output P.png [--camera 'x,y,z'] [--look-at 'x,y,z'] [--fov D] [--ortho] [--preset P] [--quality low\|medium\|high] [--no-shadows] [--probe 'x,y;…']` | Real headless Vulkan render of Workspace geometry to PNG (Phase 3). `--probe` returns exact frame pixels in JSON. Missing mesh/texture assets are reported, never faked. |
| `camera <file>` | `Workspace.CurrentCamera` if present, scene bounds, suggested framing camera. |
| `ui render\|screenshot <file> --output P.png [--device desktop\|laptop\|tablet\|mobile] [--with-scene]` | Rasterises actual GUI instances (layouts, text, images) at device/custom sizes (Phase 4). |
| `ui inspect <file>` | Computed absolute layout rects, visibility, render order. |
| `ui validate <file>` | Flags offscreen (error), clipped, zero-size, invisible elements. Exit 1 on errors. |
| `ui hit-test <file> --x X --y Y` | Topmost element at a point + events it would fire (hit resolution only; handlers run under `run`/`play`). |
| `run <file> [--duration S] [--eval <luau>] [--save-runtime O] [--render P] [--ui P] [--commit]` | Executes the place's Luau scripts on a runtime copy (official Luau VM), steps simulated time, reports logs + tracebacks. Edit state untouched unless `--commit`. |
| `play start|step|input|state|screenshot|logs|stop <file>` | Persistent physics playtest: Jolt character (R6/R15) driven by held keys, Luau gameplay code live, screenshots from the character camera. |
| `net start|step|input|state|screenshot|logs|stop <file>` | Multi-context client/server simulation (Phase 9): one authoritative server engine plus one client engine per player (`--players 1..8`, `--latency MS`, `--rig R6|R15`). Separate DataModels/Luau contexts, real RemoteEvent/RemoteFunction JSON message passing with latency queues, server→client snapshot-diff replication, per-player server-side physics characters. `net stop --commit` writes the server world back. |
| `terrain fill|ball|clear|info <file>` | Voxel terrain editing (4-stud voxels): fill a block/ball with a material (`--material Grass|Sand|Slate|...`), inspect voxel counts. Stored in the place and round-tripped. |
| `mcp [--root <dir>]` | MCP server over stdio (36 tools: editor, render, UI, runtime, play, net). Filesystem confined to `--root`. |

All commands accept the global `--json` flag. Mutating commands record an
undo snapshot before writing and write atomically.

## Limitations

- **Runtime is Luau, physics is Jolt — both real, both partial.**
  Scripting runs the official Luau VM against a documented subset of
  the Roblox API (no DataStores; client/server split exists in `net`
  sessions — see COMPATIBILITY.md); physics covers rigid bodies and
  character capsules, not constraints or Humanoid solver parity. See
  COMPATIBILITY.md for exact subsets.
- **Rendering is real but not Studio-identical.** Procedural material
  patterns instead of Studio's PBR texture maps, gradient sky instead of
  skybox cubemaps, a single shadow cascade, two-sided rendering (no
  backface culling), DejaVu Sans substituted for Roblox's
  non-redistributable fonts. Mesh v5 geometry renders (skinning does
  not, and says so); online-only assets without a local copy are
  reported, never faked. Full matrix in
  [COMPATIBILITY.md](COMPATIBILITY.md).
- **Models are single-root only.** `.rbxm`/`.rbxmx` export/import handles one subtree root; references leaving the subtree fail loudly. See COMPATIBILITY.md.
- **Property types.** The canonical set above covers the common Studio
  property types. Files using still-unsupported types (e.g.
  `Region3`, `MaterialColors`, object-referencing `Content`) fail loudly
  at load with class/property/type named rather than losing data.
  `Color3` quantizes to 1/255 in the binary format, as in Roblox.
- **Model pivots.** Roblox file serializers do not store
  `Model.WorldPivot`. BloxCLI persists an explicitly set pivot in a plain
  CFrame **attribute named `BloxPivot`** (visible and removable in
  Studio) and restores it as `WorldPivot` on load. Without one, the
  effective pivot is the descendant-parts bounding-box centre. Deleting
  that attribute in Studio resets the pivot to the bounding-box centre.
- **Referents** are file-order identifiers, not Studio GUIDs; structural
  edits can renumber later instances. Use name paths as the stable
  address. Duplicate sibling names make paths ambiguous (reported, never
  silently resolved).
- **Undo history** is per place file and snapshot-based; concurrent CLI
  writers are last-writer-wins.
- **Reflection defaults** cover the building core. Other classes get
  correct file types via the reflection database on write, but no
  engine-side default property set. Terrain and CSG geometry are not
  generated (preserved as data only). Luau/Jolt are compiled from
  vendored sources in `third_party/` (official Luau release/744,
  Jolt Physics v5.2.0; both MIT-licensed).
- Studio itself has never been run against these files in this
  environment; compatibility is established through the upstream rbx-dom
  serializers, the official reflection database, byte-level checks, and
  round-trip tests — not through a Studio comparison.

