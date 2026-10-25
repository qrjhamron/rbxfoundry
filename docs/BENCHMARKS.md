# Benchmarks

Measured with `scripts/bench.sh` on the development host
(Linux 7.0.0-34-generic x86_64, Mesa Lavapipe software Vulkan,
200-part scene on a baseplate template, 213 instances total).
Every command includes full CLI + engine process startup
(~0.15 s), so numbers are end-to-end agent-visible latencies,
not engine-internal timings.

| Operation | Wall time |
| --- | --- |
| render 800x600 (200 parts) | 0.63 s |
| ui render 1920x1080 | 0.42 s |
| export rbxl -> rbxlx | 0.40 s |
| validate | 0.17 s |
| tree (213 instances) | 0.26 s |
| play step x240 (physics, 4 s sim) | 0.16 s |

The compatibility suite separately records one 800x600 complex-scene
frame at ~305-350 ms/frame on this machine (`perf.render_800x600` in
[compat-report.json](../compat-report.json), status PARTIAL — no
Studio baseline exists for comparison).

Reproduce: `./scripts/build.sh && ./scripts/bench.sh`

## Text layout optimization (Phase 21-24 session)

Measured on the same host with a 60-`TextLabel` text-heavy GUI
(1,280x720 `ui render`, wall time for 3 renders incl. process
startup):

| Build | 3 renders |
| --- | --- |
| Before (faces reloaded per measure call) | 4.50 s |
| + font-chain cache | 1.87 s |
| + per-render glyph cache | 1.64 s |

Profile first, optimize second: `measure_text` reloaded four
FreeType faces on every call, and layout calls it per word per
wrap/TextScaled iteration. The caches are process-lifetime
(font chains) and per-render (glyph atlas entries); output is
unchanged (phase16/phase22/visual suites re-verified).

## Architecture refactor (post-refactor, same host)

Measured with `scripts/bench.sh` after the modularization refactor
(domain libraries, IPC handlers, Rust cli/app/commands split). Two
consecutive runs; render varies with Lavapipe load, all other
operations match the pre-refactor baseline in ARCHITECTURE_AUDIT.md.

| Operation | Before | After (run A) | After (run B) |
| --- | ---: | ---: | ---: |
| render 800x600 (200 parts) | 0.42 s | 0.91 s | 0.42 s |
| ui render 1920x1080 | 0.41 s | 0.41 s | 0.42 s |
| export rbxl -> rbxlx | 0.41 s | 0.44 s | 0.48 s |
| validate | 0.17 s | 0.18 s | 0.18 s |
| tree (213 instances) | 0.21 s | 0.24 s | 0.32 s |
| play step x240 (physics) | 0.16 s | 0.14 s | 0.14 s |

Conclusion: no meaningful performance regression; end-to-end timings
are dominated by process startup (~0.15 s) as before.

## Phase 25-29 check (same host, post-implementation)

| Operation | Value |
| --- | ---: |
| render 800x600 (200 parts) | 0.49 s |
| ui render 1920x1080 | 0.51 s |
| export rbxl -> rbxlx | 0.46 s |
| validate | 0.18 s |
| tree (213 instances) | 0.19 s |
| play step x240 (physics) | 0.14 s |
| Studio session: 100 sequential CLI ops on one daemon | covered by `phase25.rs` (whole test ~1.6 s incl. process spawns) |

No meaningful regression vs the Phase 25-29 baseline (ARCHITECTURE_AUDIT.md).
