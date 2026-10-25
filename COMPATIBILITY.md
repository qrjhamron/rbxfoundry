# Compatibility Matrix

Honest status of BloxCLI against Roblox Studio. "Supported" means
implemented **and** covered by automated tests in this repository. Anything
not listed as supported should be assumed unsupported. No 1:1 compatibility
is claimed anywhere; Studio itself has not been run against these files
(Linux, offline environment), so format compatibility rests on the upstream
rbx-dom serializers, byte-level verification, and round-trip tests.

Last updated: Phases 4.5/5/6/8 (MCP, Luau runtime, physics + playtest,
mesh v5). Earlier milestones: Phases 3A/3B/3C and 4 (rendering + UI).

Status vocabulary used below: **VERIFIED** (covered by automated tests in
this repo), **PARTIAL** (works for a documented subset), **UNSUPPORTED**
(deliberately absent; fails or reports loudly), **UNTESTED** (present as
data/API surface but without behavioural tests). Every VERIFIED and
PARTIAL claim is backed by *internal* tests only: this environment has no
Roblox Studio installation, so **no status here is a Studio comparison
result**. Where behaviour was modelled on Roblox documentation or open
references, that is stated; nothing is claimed 1:1.

## Place files

| Area | Status | Notes |
|---|---|---|
| `.rbxl` read | Supported | Via `rbx_binary` 3.0.1. Format sniffed from the `<roblox!` magic, not the extension. |
| `.rbxl` write | Supported | Genuine binary: `<roblox!` header, INST/PROP/PRNT/END chunks. Verified byte-level in tests. |
| `.rbxlx` read/write | Supported | Via `rbx_xml` 3.0.1. |
| `.rbxl` <-> `.rbxlx` conversion | Supported | `export`; tree-equality tested both directions. |
| `.rbxm`/`.rbxmx` model files | PARTIAL (internal tests) | Phase 17: `model export/inspect/import` for single-root subtrees. References inside the subtree are rewritten to relative index paths; references leaving the subtree (or pointing at the model root itself, which decodes as `None`) fail the export loudly. Multi-root model files are rejected. |
| DataModel root name | Not stored | Place files do not store it (upstream convention); reads synthesize `"Place"`. |
| Unknown classes | Preserved | Kept verbatim; `validate` warns. Serializers may reject value types they cannot encode on unknown classes (e.g. `Attributes`) — that fails loudly. |
| Unknown properties | Preserved | Kept verbatim; `validate` warns. |
| Unsupported property types | **Hard error** | Still unsupported: `Region3`/`Region3int16`, `MaterialColors`, `UniqueId`, `SecurityCapabilities`, `NetAssetRef`, object-referencing `Content`. (`Font` is supported.) Load fails naming class/property/type instead of dropping data. |
| Content migration (MeshId→MeshContent, Decal.Texture→TextureContent, Sound.SoundId→AudioContent, …) | Supported | Current Studio files decode with `Content` values under the new serialized names; BloxCLI maps them back to the canonical `ContentId` properties (`MeshId`, `TextureID`, `Texture`, `SoundId`) and writes them with reflection-driven migration. Object-referencing `Content` values fail loudly. Empty content reads as an empty `ContentId`. |
| Extended property types (Phase 2) | Supported | `Vector2`, `Vector2int16`, `Vector3int16`, `UDim`, `UDim2`, `NumberRange`, `Rect`, `Ray`, `Faces`, `Axes`, `PhysicalProperties`, `ColorSequence`, `NumberSequence`, `EnumItem`, and `Ref` (child-index path, rewired on write; e.g. `Model.PrimaryPart`). All covered by binary/XML round-trip tests. |
| Attributes | Supported | Under the canonical `Attributes` property, on reflection-known classes (serializer requirement). String attributes round-trip (shared type ID 0x02 normalized). Attribute values must be Roblox-representable (no Int64). |
| Tags | Supported | `Tags` property; raw-byte fallback decoding for reflection-unknown classes. |
| `Color3` precision | Format-limited | Binary format stores Color3 as 3 bytes; values quantize to 1/255. Same as Roblox. |

## DataModel (C++ engine)

| Area | Status | Notes |
|---|---|---|
| Instance tree, parent/children | Supported | Engine is the runtime source of truth during every command. |
| Typed properties | Supported | 29 value types; see README. |
| Reflection defaults | Partial | Building core: Workspace, Lighting (extended), StarterPlayer, Part/BasePart family (incl. `BrickColor`, `Shape`, `MaterialVariant`, `CustomPhysicalProperties`), MeshPart (+ `MeshId`, `TextureID`), SpawnLocation, Script family, Model, Camera, Humanoid, value objects, Sound, PointLight. Other classes get no engine defaults but serialize with reflection-correct types. |
| Property type checking | Supported | Engine table plus full reflection-database validation in `validate`. Numeric Int/Float pairs are interchangeable. |
| Name paths | Supported | `Workspace/Baseplate`; ambiguous sibling names are errors. |
| Referent identity | Session-stable | Referents are assigned in traversal order identically by the Rust conversion and the engine; unchanged trees keep `#id`s across load/save. Structural edits can renumber later instances. Not Studio GUIDs. |
| Clone / delete / reparent / group | Supported | Fresh referents on clone; cycle and root operations rejected; grouping preserves world transforms. Group parent defaults to the first target's parent. |
| Find | Supported | Exact/substring name and/or class, optionally scoped with `--under`. |
| Services in `new` | Supported | Workspace, Lighting, ReplicatedStorage, ServerStorage, ServerScriptService, StarterGui, StarterPlayer, StarterPack, Teams, SoundService. Any other service can be `add`ed by class name. |
| Model pivot | Supported with caveat | Effective pivot: explicit `WorldPivot`, else bounding-box centre. Roblox serializers do not store `WorldPivot`; BloxCLI persists an explicitly set pivot in a Studio-visible CFrame attribute `BloxPivot` and restores it on load. |
| Transforms | Supported | World-space CFrame math in the engine: BasePart move (world/local), rotate (local/world), per-axis Size scale; Model move/rotate-about-pivot/uniform scale (Roblox `Model:ScaleTo` semantics). Euler order: intrinsic XYZ, degrees. Verified by engine unit tests and CLI integration tests. |
| Derived `Position` | Supported | `get --property Position` reads CFrame translation; `set Position` keeps a stored `Position` property and `CFrame` in sync. |
| Terrain | PARTIAL (internal tests) | Voxel terrain (4-stud voxels) authored via `blox terrain fill|ball|clear|info`, Luau `Terrain:FillBlock/FillBall/Clear/ReadVoxels`; rendered as merged voxel runs, collides as static voxel boxes, characters walk on it. Voxel data is stored in a documented `BloxVoxels` BinaryString (BTR1 encoding) that round-trips losslessly through RBXL. Genuine Studio terrain chunks (proprietary layout, incl. MaterialColors) are NOT decoded — Studio terrain imports as an empty Terrain instance. |
| CSG unions/negations | Unsupported | Preserved as data only; not rendered. |

## Scripting and runtime (Phase 5 — Luau)

The official Luau implementation (vendored `luau-lang/luau` release/744:
`Luau.Compiler` + `Luau.VM`, MIT) executes scripts against a **runtime
copy** of the edit DataModel. `blox run` (and MCP `runtime_run`, and play
sessions) never mutate edit state unless `--commit`/`runtime_export` is
used explicitly.

| Area | Status | Notes |
|---|---|---|
| Luau compiler/VM | VERIFIED | Real upstream compiler + VM, not an interpreter written for this project. Bytecode compiled with debug info for tracebacks. |
| Script / LocalScript / ModuleScript | VERIFIED (internal tests) | In single-peer `run`/`play`, Scripts run server-side and LocalScripts (StarterGui/StarterPlayer/StarterPack/ReplicatedFirst) run in a second pass in the same process. In `net` sessions contexts are strict: Scripts never execute on clients, LocalScripts never on the server (separate engine processes). ModuleScripts via `require` with caching and recursion detection; require by asset id fails explicitly (offline). |
| Scheduler | VERIFIED | Coroutine threads with `task.wait/spawn/defer/delay/clock`, Heartbeat/Stepped each fixed step. Signal handlers are **deferred** (run when the scheduler processes them, matching Roblox's deferred signal model). |
| Sandbox | VERIFIED | `io`/`loadstring`/`load` removed, `os` reduced to `os.clock`; per-script sandboxed threads; instruction budget via the VM interrupt callback (tight infinite loops error out); 512 MiB memory ceiling. |
| Core API | PARTIAL | `game`/`workspace`/`script`, `Instance.new`, `GetService`, `FindFirstChild(OfClass)`, `WaitForChild`, `GetChildren`, `GetDescendants`, `Clone`, `Destroy`, attributes, tags, `Changed`/`ChildAdded`/`ChildRemoved`/`AncestryChanged`/`Destroying`/`Died` signals, `GetPropertyChangedSignal`. Datatypes Vector2/Vector3/CFrame/Color3/UDim/UDim2/Enum (subset of enums), `typeof`. |
| Services | PARTIAL | RunService (Heartbeat/Stepped; IsServer=true, IsClient=false in this single-peer runtime; BindToRenderStep unsupported-error), TweenService (interpolation for numbers/Vector3/Color3/CFrame-position; easing approximated), CollectionService (tag-backed, genuine), Players (exists; no Player objects until play mode, LocalPlayer=nil), UserInputService (InputBegan/InputEnded/IsKeyDown fed by play input; no real devices). |
| Errors | VERIFIED | Compile and runtime errors reported with Luau tracebacks; `blox run` exits non-zero on script errors unless `--allow-errors`. |
| Client/server simulation (Phase 9) | PARTIAL (internal tests) | `blox net` runs one authoritative server engine + one client engine per player, each with its own DataModel and Luau context. Scripts run only server-side, LocalScripts only on their client; `Players.LocalPlayer` resolves only on clients; PlayerAdded/CharacterAdded fire per peer; RemoteEvent (FireServer/FireClient/FireAllClients) and RemoteFunction (InvokeServer/InvokeClient, multi-return) cross contexts as marshalled JSON with configurable step latency; server state replicates to clients by snapshot diff (ServerStorage/ServerScriptService never replicate); client-side changes never replicate up (server authority). Limitations: replication is whole-property snapshot diff at step granularity (no per-property filtering/network ownership), RemoteFunction handlers that yield mid-call fail the invoke, HttpService/DataStores/Marketplace/TeleportService unsupported, `plugin` API, debugger, `loadstring` unsupported. |
| Not supported | UNSUPPORTED | HttpService, DataStores, Marketplace, TeleportService, network ownership, `plugin` API, debugger, `loadstring`, runtime Luau type-checking (compile-only, like Roblox's non-strict runtime). |
| Play mode (Phase 6) | PARTIAL | Scripts run inside physics play sessions (see below); `Players.LocalPlayer` still nil; character is `Workspace.Player`. |

## MCP server (Phase 4.5)

`bloxcli mcp [--root <dir>]` — a Model Context Protocol server over stdio
(JSON-RPC 2.0; protocol versions 2024-11-05/2025-03-26/2025-06-18
negotiated). Implemented directly against the MCP wire format in Rust
(no official Rust MCP SDK is published on crates.io as of writing; the
protocol surface used — initialize/tools/list/tools/call/ping — is the
stable core). VERIFIED by an in-repo test client speaking raw JSON-RPC,
including screenshot image content, error recovery, session lifecycle
and CLI parity (MCP edits are readable by the CLI and vice versa).

Tools: `studio_open`, `studio_new`, `studio_save`, `studio_export`,
`studio_tree`, `studio_inspect`, `instance_create`, `instance_get`,
`instance_set`, `instance_delete`, `instance_clone`, `instance_reparent`,
`instance_transform`, `scene_render`, `camera_control`, `ui_render`,
`ui_inspect`, `ui_validate`, `studio_undo`, `studio_redo`,
`runtime_run`, `runtime_export`, `play_start`, `play_step`, `play_input`,
`play_state`, `play_screenshot`, `play_logs`, `play_stop`.
Filesystem access is confined to `--root`; stdout carries protocol
frames only. Mutations auto-save through the CLI's history-recording
atomic write. Codex and other MCP clients connect with a standard stdio
server entry (`bloxcli mcp --root /path/to/project`); no auth (local
offline use).

## Physics and playtest (Phase 6)

Jolt Physics v5.2.0 (vendored, MIT) steps the runtime DataModel at a
fixed 1/60s. `blox play start|step|input|state|screenshot|logs|stop`
drives a persistent daemon session; the character is a Jolt
`CharacterVirtual` capsule, so the agent walks/jumps/collides through
real physics — never teleporting. MCP exposes the same as `play_*`
tools.

| Area | Status | Notes |
|---|---|---|
| Rigid bodies | VERIFIED | Anchored = static, unanchored = dynamic (mass = volume × density, CustomPhysicalProperties density honoured). Bodies build once at `physics/begin`; parts created by scripts mid-session are not retro-fitted (documented). |
| Colliders | PARTIAL | Block/Ball/Cylinder exact-ish; Wedge/CornerWedge/MeshPart collide as their bounding box; CanCollide=false parts get no collider. |
| Character | VERIFIED | R6 and R15 rigs spawn at SpawnLocation, walk at Humanoid.WalkSpeed, jump with JumpPower, fall, land, are blocked by walls, stand on platforms, climb 25° slopes. Rig parts are joined by real Motor6D instances (C0/C1 from the rest pose); posing evaluates the joint chain every step. Without script animation, physics writes procedural walk/idle Transforms through the same joints. |
| Humanoid | PARTIAL | WalkSpeed/JumpPower/Health/MaxHealth are live properties (scripts can change them and physics obeys); fall damage + death → Died signal + respawn at spawn. No swimming/climbing/seats; state names limited to Running/Jumping/Freefall/Dead. |
| Cameras | PARTIAL | Third-person follow + first-person from the head, yaw/pitch via `play input --yaw/--pitch`; no Roblox camera scripts/modules, no occlusion handling. |
| Raycast | VERIFIED | Engine RPC `physics/raycast`, Luau `Workspace:Raycast` with RaycastParams (Include/Exclude filters resolved to body allowlists), plus `Workspace:GetPartBoundsInBox`/`GetPartBoundsInRadius` and RPC equivalents. Box queries are axis-aligned (rotation ignored — documented). CanCollide respected. |
| Constraints | PARTIAL (internal tests) | WeldConstraint (Jolt fixed), BallSocketConstraint (point), HingeConstraint (hinge about Attachment0 X axis) built from Attachment/Part refs at `physics/begin`. No constraint motors/limits; Weld/Motor6D are transform joints, not physics constraints. |
| Touch events | PARTIAL (internal tests) | `BasePart.Touched`/`TouchEnded` fire from AABB-overlap transitions involving a dynamic body (not a Jolt contact listener: no hit normal/position, resting contact re-fires only on re-overlap). |
| Animation | PARTIAL (internal tests) | Animator + AnimationTrack: `LoadAnimation` (Animation instance with a KeyframeSequence child, or a KeyframeSequence directly; asset-id animations error explicitly offline), Play/Stop/AdjustSpeed, TimePosition/Length/Looped, keyframe position lerp + quaternion slerp written to Motor6D.Transform and posed through the physics joint chain. No blending between tracks (last writer wins), no fade, Pose.Weight/EasingStyle ignored, default Roblox animate scripts/asset ids unsupported. |
| Differences from Roblox | documented | Character does not push dynamic bodies; no Humanoid solver parity claims; network ownership absent (server simulates all characters in `net` sessions). |


## Multiplayer simulation (Phase 9)

`blox net start|step|input|state|screenshot|logs|stop` runs one
authoritative server engine process plus one client engine process
per simulated player (1–8). Peers share nothing: each has its own
DataModel copy and Luau context. MCP mirrors this as `net_*` tools.

| Area | Status | Notes |
|---|---|---|
| Contexts | VERIFIED (internal tests) | Scripts run only on the server; LocalScripts only on their own client; `Players.LocalPlayer` resolves only on clients (nil on the server); per-peer logs/errors. |
| Players/characters | VERIFIED | Player instances injected per session; PlayerAdded/CharacterAdded fire per peer; each player owns a server-side physics character; clients see replicated character models (`Player.Character` resolves on both sides). |
| RemoteEvent | VERIFIED | FireServer/FireClient/FireAllClients with sender Player identity; values marshalled to JSON (primitives, tables, Vector3/CFrame/Color3/UDim2/EnumItem, Instances by path). Configurable step latency (170 ms tested). |
| RemoteFunction | VERIFIED | InvokeServer/InvokeClient with multiple return values; handlers that yield mid-call, missing handlers, and wrong-context calls fail the invoke with a reported error (documented divergence from Roblox's indefinite yield). |
| Replication | PARTIAL | Server→client whole-property snapshot diff each step (create/set/delete), applied to every client; ServerStorage/ServerScriptService never replicate; Ref properties are translated to portable name-paths. No network ownership, no per-property replication filtering, property *removals* do not propagate. Client-side changes never reach the server (verified: local cheat stays local). |
| Authority | VERIFIED | Physics and world state are server-only; clients render replicated state. `net stop --commit` writes the server world back to the place. |

## Visual fidelity (Phase 11)

| Area | Status | Notes |
|---|---|---|
| Dynamic lights | PARTIAL (internal tests) | PointLight omni + SpotLight/SurfaceLight cones (parent part look vector, Angle/Range/Brightness/Color) in the Vulkan shader, max 8, quadratic falloff; no per-light shadows. Pixel-verified against a dark scene. |
| SurfaceAppearance PBR | PARTIAL (internal tests) | ColorMap + NormalMap (derivative perturbation, no authored tangents) + RoughnessMap (G channel) + MetalnessMap (B channel) on MeshParts; missing maps reported by asset id, never faked. AlphaMode/transparency interplay approximate. |
| Beam / Trail | PARTIAL (internal tests) | Beam renders as an oriented strip between Attachment0/1 world points with its first Color keypoint; Trail uses the same static strip (no motion-history ribbon). Texture/segments/curve unsupported. |
| ParticleEmitter | PARTIAL (internal tests) | Deterministic billboard snapshot: Rate×Lifetime particles (cap 200) with seeded spread, first Size/Transparency/Color keypoints. No accumulation, velocity inheritance, rotation, or texture. |
| TextService | PARTIAL (internal tests) | `TextService:GetTextSize` + `ui/textBounds` RPC use the GUI engine's FreeType measurement (DejaVu substitute — metrics differ from Roblox fonts); greedy word wrap. RichText subset (Phase 16, INTERNAL_VERIFIED): `<b> <i> <u> <font color>` + `<br>` render as styled runs with correct fonts/colours/underlines (pixel-tested); unsupported tags stripped. Text rasterisation is byte-oriented: non-ASCII (e.g. em-dash, emoji) renders incorrectly. |
| Skinning | UNSUPPORTED | Mesh v5 envelopes still skipped with an explicit warning; character animation is joint-transform based, not skinned. |

## Compatibility suite (Phase 12)

`crates/bloxcli/tests/compat.rs` runs a complex fixture (terrain,
house model with attributes, remotes, scripts, HUD) through RBXL→
RBXLX→RBXL round trips, `validate`, `run`, a 2-client net session,
play, desktop/mobile renders and physics/animation presence checks,
and writes a machine-readable report to `compat-report.json` at the
repository root (override with `BLOX_COMPAT_REPORT`). VERIFIED in
that report means an internal automated check passes; there is no
Roblox Studio reference data in this environment, so no check is a
Studio comparison and none is claimed to be.

## Phase 13-16 additions (verified internally, not Studio-compared)

| Area | Status | Evidence |
| --- | --- | --- |
| Templates `empty` / `baseplate` / `classic-baseplate` | PARTIAL (structural) | Registry in `templates.rs`; `new --template`, `template list/inspect`, `place info`. Studio dimensions/colours UNTESTED (no golden `.rbxl` fixture available) |
| Place metadata (`game.PlaceId` etc.) | INTERNAL_VERIFIED | Luau reads default 0/"" for offline places; writes error as read-only. RBXL root metadata is not persisted by the serializers (documented, not silently invented) |
| ContextActionService | INTERNAL_VERIFIED | Bind/Unbind/UnbindAllActions fire from play input (`phase14.rs`) |
| PathfindingService (grid A*) | INTERNAL_VERIFIED | CreatePath/ComputeAsync/GetWaypoints route around obstacles; documented approximation, no navmesh |
| Humanoid:GetState/ChangeState + StateChanged | INTERNAL_VERIFIED | State API driven by physics state |
| PhysicsService collision groups | INTERNAL_VERIFIED | CreateGroup/SetPartCollisionGroup + RegisterCollisionGroupSettable via custom Jolt matrix filter |
| AnimationTrack blending (`Weight`, `AdjustWeight`) | INTERNAL_VERIFIED | Weighted joint blending, lower-weight track contributes measurably |
| Character stair climbing | INTERNAL_VERIFIED | 1-stud steps climbed while walking (`phase14.rs`) |
| RichText subset | INTERNAL_VERIFIED | See TextService row; `screenshots/rich-text.png` |
| ImageLabel nine-slice (`ScaleType=Slice`) | INTERNAL_VERIFIED | Nine quads from `SliceCenter` (Rect) x `SliceScale`, texture size from PNG IHDR; corners unscaled vs Stretch control (pixel-tested). Non-PNG textures fall back to stretch with a warning |
| Multi-device UI screenshots | INTERNAL_VERIFIED | 1920x1080, 1280x800, 390x844 probe-verified anchored layout (`phase16.rs`) |

## Phase 17 additions (verified internally, not Studio-compared)

| Area | Status | Evidence |
| --- | --- | --- |
| Export integrity guard | INTERNAL_VERIFIED | `integrity.rs` runs in `save`/`export`/`model` writes and reports in `validate` (`integrity` JSON summary, `integrityWarnings` on export). Errors block the write before any file is touched: dangling references, corrupted `BloxVoxels` (BTR1) terrain payloads, instance references inside Attributes. Warnings do not block: unresolved `rbxassetid://` content (preserved verbatim), scripts without Source, duplicate sibling names |
| XML undefined referents | INTERNAL_VERIFIED | `rbx_xml` silently decodes a `<Ref>` to an undefined referent as `None`; `read_place`/`read_model` pre-scan XML text and refuse such files loudly |
| `.rbxm`/`.rbxmx` models | INTERNAL_VERIFIED | See Place files table. `phase17.rs`: export->inspect->import round trip keeps PrimaryPart resolving; external references rejected |

## Tooling

| Area | Status | Notes |
|---|---|---|
| CLI commands | Supported | Phase 1 set plus `get`, `tree`, `find`, `clone`, `delete`, `reparent`, `group`, `move`, `rotate`, `scale`, `pivot`, `undo`, `redo` — all tested end-to-end. Binary installed as both `bloxcli` and `blox`. |
| Undo/redo | Supported | File-snapshot history in `<place>.bloxhistory/` (100 cap, decode-checked restores, redo cleared on mutation). Not stored inside the place file. |
| `--json` agent output | Supported | Every command. Errors: stderr text + exit code (1 = error/invalid, 2 = usage). |
| Engine IPC | Supported | Versioned JSON-RPC 2.0, protocol `"1"`, `initialize` handshake enforced. |
| Long-lived daemon across CLI invocations | Unsupported | Engine persists for one command session; files are the durable state. |
| Automated tests | Supported | C++ (`ctest`, 2 suites): DataModel/hierarchy/transforms plus geometry/scene extraction/mesh parsing. Rust (`cargo test`): Phase 1 workflows + genuine-binary byte checks + format round-trips; Phase 2 editing; Phase 3 render tests (pixel probes for geometry/occlusion/cameras/determinism/shadows/time-of-day/transparency/materials, mesh UV orientation, checker mapping, missing-asset reporting, RBXL mesh round-trip, quality presets, named regression scenes); Phase 4 UI tests (exact layout rects, pixel probes incl. rasterised text, hit-testing, offscreen validation, list layout, device presets, SurfaceGui compositing). |

## Dependency evaluation notes

- **rbx-dom** (Rojo ecosystem): **adopted**. `rbx_binary`/`rbx_xml` are the
  most battle-tested open implementations of Roblox's file formats and use
  Roblox's own reflection database (API dump 741) to write Studio-correct
  property types. Writing a second encoder from scratch in Phase 1 would
  have produced a worse-tested clone, not more compatibility.
- **Official Luau**: adopted (Phase 5). Vendored from `luau-lang/luau`
  at release/744 (MIT); `Luau.Compiler` + `Luau.VM` are built from
  `third_party/luau` and linked into the engine.
- **Jolt Physics**: adopted (Phase 6). Vendored from
  `jrouwe/JoltPhysics` v5.2.0 (MIT) in `third_party/JoltPhysics`.
- **rbx_mesh (reference)**: the reverse-engineered mesh layouts (incl.
  v5.00) follow this open-source Rust parser's published format
  knowledge; no Roblox code or assets are used.
- **MCP Rust SDK**: evaluated; no official Rust SDK is published, so the
  server implements the MCP stdio wire protocol directly (see the
  Phase 4.5 note above).
- **Vulkan + Mesa Lavapipe**: adopted for headless rendering. GLSL is
  compiled to SPIR-V at build time with `glslangValidator` and embedded
  in the engine binary; PNG via libpng; text via FreeType with the
  system DejaVu Sans as the legally distributable font.
