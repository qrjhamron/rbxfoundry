# Play & Net Screenshot Rendering

Play Mode screenshots are rendered from the **live runtime model**, never
from the place file and never by committing.

## Pipeline (single player)

1. The play daemon asks the engine for `runtime/snapshot` (the runtime
   DataModel exactly as scripts/physics left it).
2. The snapshot is loaded into the engine's scratch edit model
   (`datamodel/load` never touches the runtime model).
3. IPC `ui/render` with `withScene: true` renders the 3D scene from that
   snapshot with the play camera, then composites the GUI built from the
   same snapshot (DisplayOrder, topbar inset, ZIndex, clipping, text,
   images) over it in one Vulkan frame.
4. `--probes "x,y;..."` pixel probes are answered from that same frame.
5. The edit model is reloaded from the place file, so a screenshot cannot
   leak runtime mutations into editing. The place file is only written by
   `play stop --commit`.

`play state` additionally reports live GUI computed bounds (rect paths and
positions from the runtime model) next to time/character/camera/bodies.

## Multiplayer

`net screenshot --player N` runs the same pipeline on that client's own
engine: its replicated snapshot, its camera, its live GUI. Each client
runtime keeps independent GUI state in its StarterGui.

## Known limitations

- There is no separate `Players/<player>/PlayerGui` container yet: client
  LocalScripts run against StarterGui and that live state is what renders.
  Scripts addressing `PlayerGui` by path do not resolve.
- Active tweens are observable through rendered frames and state, but are
  not enumerated in `play state`.
- The character is part of the 3D scene extraction; GUI is composited in
  screen space only (SurfaceGui/BillboardGui are part of the 3D pass).
