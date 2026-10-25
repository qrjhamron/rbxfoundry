# Phase 27 — Physics & Humanoid Fidelity

External Studio parity: `UNTESTED`.

## Implemented this phase
- **PhysicalProperties (27.1, PARTIAL):** `CustomPhysicalProperties`
  density/friction/elasticity (array or object form) now drive Jolt
  mass, friction and restitution for dynamic and anchored bodies
  (previously density-only with fixed 0.5/0.0). `AssemblyLinearVelocity`
  / `AssemblyAngularVelocity` are applied as initial body velocities.
  Material-default property tables and FrictionWeight/ElasticityWeight
  pair blending remain unimplemented (weights stored, not blended).
- **Contacts (27.3, PARTIAL):** `Touched` plus new `TouchEnded` from
  real contact-pair lifecycle (pair set diff per physics step in the
  IPC step loops), gated by `CanTouch` on both parts. Detection remains
  engine-AABB overlap of Jolt bodies (narrow-phase manifolds, contact
  points/normals and CollisionGroup filtering are not exposed; sensors
  without colliders do not generate events). `CanCollide` false parts
  still have no body.
- **TweenService (27B, INTERNAL_VERIFIED for covered behaviour):**
  see GUI_FIDELITY.md. Physics/tween share the scheduler clock.

## Pre-existing (unchanged)
Jolt bodies for Part/Ball/Cylinder/Wedge/CornerWedge/Mesh/Union (box
approximations per COMPATIBILITY.md), constraints (Weld through
Torque where listed), ApplyImpulse family, Raycast/Blockcast/
Spherecast/Shapecast and bounds queries, Humanoid state machine
including climbing (TrussPart/ladder wedge), swimming in Water
terrain, seats, moving platforms — all covered by play/phase14-19
suites. Gate 27 scenario list items 1-12 remain covered by those
suites plus `phase27.rs` (falling/elastic contact); specific new
tests this phase: elastic ball `Touched`+`TouchEnded`.

## Tests
`crates/bloxcli/tests/phase27.rs` (tween + elastic contact), existing
play/phase14/16/17/19 suites.
