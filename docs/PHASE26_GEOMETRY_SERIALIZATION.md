# Phase 26 — RBXL, Terrain & CSG Fidelity

External Studio parity: `UNTESTED`. Statuses below are internal.

## Terrain (`PARTIAL`)
- Storage remains BloxCLI's documented BTR1 voxel chunks on the
  Terrain instance (`Workspace/Terrain.BloxVoxels` + `BloxTerrainMaterials`).
  Genuine Studio binary terrain chunks are **not** decoded; unknown
  terrain payloads are preserved as opaque properties and never
  silently replaced with empty terrain (export guard behaviour is
  unchanged and tested by round-trip suites).
- Luau APIs implemented over the 4-stud grid: `FillBlock`, `FillBall`,
  `FillCylinder`, `FillWedge`, `FillRegion`, `WriteVoxels`,
  `ReadVoxels`, `CopyRegion`/`PasteRegion` (per-Terrain clipboard),
  `Clear`. Only resolution 4 is accepted; other resolutions error
  explicitly. Rotation is ignored for fills (documented).
- `Region3` datatype: `Region3.new(min, max)`, `.Min/.Max/.Size/.CFrame`,
  `:ExpandToGrid(resolution)`, `typeof` = `Region3`.
- CLI sculpt ops (fill/sphere/clear/carve/replace/raise/lower/smooth/
  flatten/grow/erode) unchanged.

## CSG (`UNSUPPORTED`)
- No general CSG boolean engine exists. UnionOperation instances and
  their `MeshData`/physics payloads are preserved opaquely through
  round-trips when present; RBXFoundry does not generate
  Studio-exportable UnionOperation data and never relabels a MeshPart
  as one. This is an explicit remainder, not a stubbed API: no
  `UnionAsync`/`SubtractAsync`/etc. methods are exposed.

## Property/model coverage
- Unchanged from Gate 26 audit: serialized `Region3`, `MaterialColors`,
  `UniqueId`, `SecurityCapabilities`, `NetAssetRef` properties fail
  loudly in `rbxio::convert` instead of being dropped; attributes,
  tags and object references round-trip as before. `.rbxm` root/self/
  internal/external reference behaviour is unchanged (phase2 tests).
- New Luau-only `Region3` values are a runtime datatype, not a
  serialized property type.

## Tests
`crates/bloxcli/tests/phase26.rs`: cylinder/wedge fills read back via
`ReadVoxels`, copy/paste region equality, `WriteVoxels` erase.
