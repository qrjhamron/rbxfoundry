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

## PlayerGui

At script start, client and single-player runtimes clone StarterGui's
contents wholesale into the local player's `PlayerGui`
(`Players/<player>/PlayerGui`), exactly where Roblox puts them.
LocalScripts execute against the clones; layout, hit-testing and
screenshots prefer PlayerGui over the StarterGui originals whenever a
PlayerGui exists. `Players.LocalPlayer` resolves on client/single
engines and is nil on server engines. Server edits to StarterGui after
cloning do not propagate to clones (Roblox semantics: use RemoteEvents).
StarterPlayer/StarterPack/ReplicatedFirst LocalScripts run in place;
PlayerScripts cloning is not modelled. Solo play injects `Player1`
(like net sessions inject their players); `ResetOnSpawn` is not
re-cloned yet.

`play state` enumerates active tweens (target, properties,
PlaybackState, elapsed/duration). Touch input (`play input --touch`,
`net input --player N --touch`) fires `TouchTap` through the same
native hit-test; net GUI input is per-client by construction.

## Lifecycle

StarterPlayerScripts clones into `Player/PlayerScripts` once at join
and is never re-run on respawn. On death the runtime fires
`Humanoid.Died` (once) and `CharacterRemoving`; after a 1-second
respawn the character returns at spawn with full health and
`CharacterAdded` fires. ScreenGuis whose StarterGui original has
`ResetOnSpawn = true` (the default) are destroyed and re-cloned —
their LocalScripts restart with fresh state — while
`ResetOnSpawn = false` GUIs keep their live state. The character model
instance itself is reused across respawn rather than recreated.

## Known limitations

- StarterPlayer scripts other than StarterPlayerScripts contents run
  in place; character-model recreation on respawn is not modelled.
- UIGradient evaluates at most 8 keypoints and does not model `Offset`.
- The character is part of the 3D scene extraction; GUI is composited in
  screen space only (SurfaceGui/BillboardGui are part of the 3D pass).
