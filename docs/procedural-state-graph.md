# Procedural State Graph

## Conceptual state

A procedural state at time `t` is (conceptually)

```
G_t = ( O_t  ) objects            immutable declared content (id → object)
      ( I_t  ) instances          mutable-by-transition placements
      ( T_t  ) transforms/trajectories       (added in later phases)
      ( D_t  ) dynamics/generators           (added later)
      ( C_t  ) composition/order             (instance paint order here)
      ( R_t  ) residual bindings             (added later)
      ( M_t  ) models/palettes/dictionaries  (added later)
```

Rust Phase-A domain model (see `src/state.rs`, `src/object.rs`):

* [`Object`] is **immutable** visual content: a Gray8 fill or boxed raster.
* An [`ObjectId`] references an object in the declared object table.
* An [`Instance`] is a mutable placement `(object_id, x, y)`.
* A [`State`] holds `background`, the object table, and ordered instances.

Later phases added to the domain model (`T_t`, transforms/trajectories; `M_t`, models/palettes):

* per-instance persistent integer translation `(vx, vy)` (Phase E: applied
  once per `AdvanceTranslations`);
* per-instance bounded parametric **trajectory programs** (Phase I: finite
  `Linear`/`Accel` segment lists in `src/trajectory.rs`; each program is
  stepped once per `AdvanceTrajectories`, deactivates when exhausted, and is
  mutually exclusive with translation state on the same instance);
* a mutable **palette table** plus per-instance palette bindings (Phase J:
  palette-index objects — immutable one-byte index planes — render through
  the entries of the palette bound to the painting instance, so `M_t` is now
  real state: `G_t` carries `palettes` and `bindings` alongside the object
  table, instances, velocities, and trajectories).

## Immutability vs mutation

* **Objects never change** once declared (`State::declare_object`).
* **Instances change** only through explicit transitions:
  `set_position`, `create_instance`, `set_velocity`/`advance_translations`
  (Phase E), `set_trajectory`/`advance_trajectories` (Phase I); the Phase-B
  unchanged lane handles statics.

Persistent identity is *exact*: no object or instance id is recycled and no
"looks similar" trick grants reuse. Later phases add BLAKE3 content
addressing for immutable content reuse across streams (see
`docs/entropyfs.md` and Phase B ADR drafts) — reuse requires identical bytes,
never appearance.

## Composition

The Phase-A compositor is an ordered painter:

1. fill the whole canonical canvas with `background`;
2. each instance (in **paint order**, append order of creation) overwrites its
   object box (clipped to the canvas) at its `(x, y)`.

Consequences that matter for courts:

* clearing to background each frame is *part of the state semantics*, not a
  codec flag — a moving object therefore "leaves a cleared trail" exactly as a
  simple painter would (`tests/court.rs` checks this);
* object draw order and coordinate origin are normative; correctness does not
  depend on the encoder's search choices.

## Boundedness

`src/limits.rs` bounds every structural quantity (objects, instances,
canvas bytes, transitions per interval, replay depth, stream size). A state
that grows beyond the envelope is rejected with a typed error during parse or
materialization. No untrusted count drives an allocation before validation.
