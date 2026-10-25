# GUI Fidelity (27A–27F)

External Studio parity: `UNTESTED`. Support matrix is in
COMPATIBILITY.md; this document records behaviour and gaps.

## 27A Fonts (`PARTIAL`)
FreeType with local system-font resolution and fallback chains
(system, FontFace family names, Font enum), cached chains/glyphs,
CJK/Thai codepoint fallback, RichText, TextWrapped/Scaled/Truncate
measurement in `gui/text.cpp` + `gui/layout.cpp`. No HarfBuzz:
shaping is per-codepoint with FreeType kerning between adjacent
glyphs only; no ligatures, grapheme-cluster shaping, bidi reordering
or colour-emoji glyphs. Missing fonts substitute the documented chain
and render visibly instead of failing. Unicode fixtures are covered
by phase21 tests.

## 27B Tweens (`INTERNAL_VERIFIED` for covered behaviour)
- `TweenInfo.new(time, easingStyle, easingDirection, repeatCount,
  reverses, delayTime)` (new; previously missing entirely).
- All documented EasingStyles (Linear, Sine, Quad, Cubic, Quart,
  Quint, Exponential, Circular, Back, Bounce, Elastic) with
  In/Out/InOut direction mathematics.
- Interpolation for numbers, Vector2/Vector3, Color3, CFrame position,
  UDim and UDim2 (GUI Position/Size now tween correctly).
- Lifecycle: Play/Pause/Cancel, Delayed start, RepeatCount (-1 =
  forever), Reverses alternation, Completed event (`"Completed"` /
  `"Cancelled"` string payload; `PlaybackState` returns the state
  name as a string — EnumItem form is a known partial).
- Evaluation is on the runtime scheduler clock (deterministic under
  `run --duration/--step` and play `step --dt`), never wall-clock.
- Tests: `phase27.rs` asserts Sine-Out/Quad-In values at fixed alpha,
  completion exactness, delay, and reverse-halfway values.

## 27C Instances (`PARTIAL`)
Per-class layout/render/event coverage is tracked in
COMPATIBILITY.md. CanvasGroup group transparency and ViewportFrame
(3D-in-GUI) behaviour are as documented there; UIFlexItem is not
implemented and unknown GUI classes/properties are preserved.

## 27D Layout (`INTERNAL_VERIFIED` for covered behaviour)
UDim2/AnchorPoint/AutomaticSize, list/grid/page layouts, constraints,
UIScale, ScrollingFrame canvas + AutomaticCanvasSize/AbsoluteContentSize,
ScreenInsets topbar handling, IgnoreGuiInset, viewport/device sizes
(`ui render --device/--size`, `play screenshot`), ZIndex/DisplayOrder,
ClipsDescendants. Numerically covered by phase21/ui tests.

## 27E Input (`PARTIAL`)
Mouse click/enter/leave, Activated, TextBox focus/edit/submit,
UserInputService keyboard, GUI events through the real scheduler
(`ui/click`, `ui/input`, `ui/hover`, play `input`). Gamepad selection
navigation and MouseMoved/InputChanged streaming are partial; see
COMPATIBILITY.md.

## 27F Rendering (`PARTIAL`)
Software SurfaceGui rasterizer + Vulkan UI pipeline from actual GUI
properties: gradients, strokes, corners, nine-slice, image rects,
rotation, transparency inheritance, SurfaceGui/BillboardGui. sRGB
output is the rasterizer's byte output; no beautification. Screenshot
fixtures: render/visual/phase21 suites.
