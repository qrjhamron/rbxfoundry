# Troubleshooting

## `blox-engine not found`

The CLI locates the engine via `--engine <path>`, `$BLOX_ENGINE`, a
`blox-engine` binary next to the CLI executable, or
`build/engine/blox-engine` in the workspace. Build first:

```bash
./scripts/build.sh
```

Or point at it explicitly: `bloxcli --engine /path/to/blox-engine <cmd>`.

## Rendering fails / Vulkan errors

Rendering is genuine Vulkan. On machines without a GPU, Mesa's
Lavapipe software driver works:

```bash
sudo apt-get install -y mesa-vulkan-drivers libvulkan-dev
```

Check with `vulkaninfo --summary` (package `vulkan-tools`) that a
device is listed. Screenshots are real framebuffer readbacks; if the
render command succeeds, the PNG is genuine.

## Text does not render / "no usable TTF font found"

Install DejaVu (`sudo apt-get install -y fonts-dejavu-core`), set
`BLOX_FONT=/path/to/font.ttf`, pass `--font <path>` to `ui render`, or
use a release tarball (it bundles DejaVu under `fonts/`). Text renders
with FreeType; codepoints no installed font covers are drawn as
`.notdef` boxes and reported in the render `warnings` — install a
broader font (e.g. `fonts-noto-cjk`) for CJK coverage.

## Missing mesh/texture assets

`rbxassetid://` content is never downloaded and never faked. Put local
files next to the place file (or pass `--assets <dir>`): numeric ids
resolve as `<id>.mesh` / `<id>.png`. Unresolved assets are listed in
render output `missing_assets` and flagged by `validate`
(export integrity guard warnings).

## A command refuses to write my file

That is the export integrity guard working: dangling instance
references, corrupted terrain payloads, or references inside
Attributes block `save`/`export` before anything is overwritten.
`blox validate <file> --json` lists the exact issues. For XML places,
also check for `<Ref>` values naming referents that do not exist —
those are refused at load instead of being silently dropped.

## Build failures

- `glslangValidator` missing: `sudo apt-get install -y glslang-tools`.
- FreeType/Vulkan/PNG dev packages: see the list in
  [../README.md](../README.md#build-linux).
- CMake ≥ 3.20 and a C++20 compiler (g++ ≥ 11 or clang ≥ 14) are
  required. Rust: stable ≥ 1.70.

## MCP server issues

Run `bloxcli mcp` over stdio only; logs never go to stdout (they would
corrupt the protocol). Filesystem access is restricted to the approved
project root passed at server start. If a client cannot connect, check
that the configured command is the built `bloxcli` binary and that the
engine is discoverable (above).
