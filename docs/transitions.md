# Transitions

## Time model

Time is a monotonically increasing integer `Interval`. A checkpoint anchors
interval `0`; a transition group advances state to a strictly larger interval.
Materializing each integer interval of a decoded stream yields the canonical
view sequence.

Phase A timeline:

```
checkpoint   G_0   = objects + background + instance placements  (frame 0)
interval  1  G_1   = Φ(G_0, SET_POSITION ...)                    (frame 1)
interval  2  G_2   = Φ(G_1, SET_POSITION ...)                    (frame 2)
...
```

## Phase-A operator language

The transition language is intentionally small but the *infrastructure* is
real and reused by later phases. Supported Phase-A operators, with their wire
tags (`docs/format-v1.md`):

| Transition | Effect | Example |
|---|---|---|
| `DeclareObject` (0x11/0x12 via pre-checkpoint records)/`DeclareFill` | add immutable object to object table | baseline objects only in v1 |
| `CreateInstance` (0x21) | add an instance at a paint position | an object appearing |
| `SetPosition` (0x22) | move an instance to an absolute `(x, y)` | a moving box: `x += 2` |

## Later-phase operators (v1 grammar continues to evolve per phase)

| Phase | Transition (tag) | Effect |
|---|---|---|
| C | `PatchSparse` (0x23) | set persistent overlay points above all instances (strict-sorted) |
| D | `CopyRect`/`MoveRect` (0x24/0x25) | canvas ops copying a rectangle from the immediately previous decoded frame onto the frame base |
| E | `SetVelocity` (0x26) + `AdvanceTranslations` (0x27) | per-instance persistent integer translation applied once per advance |
| G | `ClearInstances` (0x28) / `ClearOverlay` (0x29) | full-content replacement: drop every live instance / every overlay point |
| G | `Residual` (0x2a) | per-frame residual block (Phase-F coded payload) applied to the canvas in op order — the `⊕_ρ` residual algebra, one-shot, stateless |
| I | `SetTrajectory` (0x2b) + `AdvanceTrajectories` (0x2c) | per-instance bounded parametric trajectory program (Linear / Accel segments) stepped once per advance; empty program deactivates; exclusive with translation state |

Because Phase A declares all objects before the single checkpoint, interval
groups in v1 contain only `CreateInstance`/`SetPosition`. Later phases broaden
the operator language; the *replay* architecture (`src/format.rs`,
`src/transition.rs`) is unchanged: each transition is applied forward onto one
deterministic `State`, and references are validated as they are applied
(unknown object/instance → typed error).

## Phase-I trajectory semantics (normative, integer, exact)

A **trajectory** is a finite motion program attached to one instance. Time
steps are *advances*: one advance is applied per `AdvanceTrajectories`
transition (the Phase-E choice of *explicit* stepping is kept, so the sealed
unchanged lane is untouched and nothing steps on empty intervals). Segments
run in order; each runs for its declared `steps` advances, then the next
segment starts; when the final segment's steps are exhausted the trajectory
deactivates and the instance stays at its final position.

* `Linear { vx, vy, steps }` — position gains `(vx, vy)` per advance
  (constant velocity; `(0,0)` is an exact hold).
* `Accel { vx0, vy0, ax, ay, steps }` — velocity starts at `(vx0, vy0)` and
  gains `(ax, ay)` *after* each advance. After `t` advances the displacement
  is the exact integer closed form `Δ(t) = t·v0 + a·t·(t−1)/2` (velocity
  during advance `k` is `v0 + k·a`), the discrete-time form of
  `x(t) = x0 + v0·t + ½·a·t²`.

All arithmetic is checked; an overflowing accumulation is a typed error,
never a wrap. Trajectory and translation (`0x26/0x27`) state on one instance
are mutually exclusive. The program is bounded (`max_trajectory_segments`
segments); cumulative trajectory-step work is capped by
`Limits.max_trajectory_work` at parse and encode time.

§43 collapse: many repeated per-frame `SetPosition` transitions (or measured
steady translations) may be replaced by one `SetTrajectory` descriptor plus
per-frame `AdvanceTrajectories` **only if** materialization stays exact
(proven by normative decode of the rebuilt stream) and the complete cost
falls (strictly fewer bytes). Runs shorter than three frames cannot pay for
the descriptor and are left alone.

## §/Semantics that MUST be deterministic

* positions are absolute; `SetPosition` replaces (`SET_POSITION`), not `+=`;
  parametric motion is expressed with explicit trajectory state (Phase I)
  rather than by mutating the absolute-position operator's meaning;
* apply order inside an interval is the order written, and materialization
  after an interval sees exactly the composed result;
* hostile, typed-everywhere: an interval index that is not strictly
  increasing, a duplicate instance id, or a reference to nothing is rejected
  with a typed error during parse (tests/malformed.rs).

## Replay

A decoder restores the checkpoint state, then applies successive interval
groups. `Decoded::materialize_all` and `Decoder` in `src/decoder.rs` replay a
validated stream (re-applications after parse validation are guaranteed not to
produce errors, but are still checked rather than `unwrap`ed on hostile input).
