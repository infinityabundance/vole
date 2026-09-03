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
| J | `SetPalette` (0x2d) / `PatchPalette` (0x2e) / `BindPalette` (0x2f) + palette-table record (0x06) + palette-binding checkpoint (0x08) + palette-index object (0x05) | mutable palette state; palette-index planes render through per-instance bound palettes |
| L | `SetAffine` (0x30) | attach a canonical Q8 fixed-point affine placement to one instance (pan / zoom / rotation / camera-like transform as persistent state; see semantics below) |

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

## Phase-J palette semantics (normative, deterministic)

A **palette** is a bounded mutable table of Gray8 entries; a
**palette-index object** is immutable content whose bytes are one-byte
indices. Materialization renders an index object by resolving every index
through the entries of the palette **bound to the painting instance**
(`M(state)`: `indices ∘ entries(bound_palette)`). A missing binding/palette
is `UnknownPalette`; an index at or beyond the palette length is
`OutOfBounds` — typed, deterministic, never a wrap.

* `0x06`/`0x2d` lay down / replace a whole palette (`1..=256` entries, id ≠ 0);
* `0x2e` patches entries — `(index, value)` pairs, strictly ascending, in
  range;
* `0x2f` binds/unbinds an instance to a palette; bindings die with their
  instances (`0x28`), palettes persist;
* the checkpoint variant `0x08` carries interval-0 bindings so palette
  content renders from frame 0 (plain `0x03` checkpoints stay byte-identical
  for streams without bindings).

Because the index plane never changes while palette entries are mutable
state, color animation (accent blinking, full color drift) is a *tiny state
mutation* — `24–28 B/interval` on the Phase-J courts — never a raster or
index-plane rewrite. Palette content at rest costs the ordinary 13 B/frame
unchanged lane.

## Phase-L affine placement semantics (normative, integer, exact)

An **affine placement** (tag `0x30`) replaces the plain `(x, y)` placement of
one instance with a canonical fixed-point 2D map: destination pixel `(x, y)`
samples the object at

```text
(su, sv) = ((a·x + b·y + c) >> 8, (d·x + e·y + f) >> 8)
```

where `a..f` are Q8 coefficients (one source pixel = 256 units; the signed
`>> 8` is floor division — the canonical rounding). No floating point exists
anywhere in the normative path. The sample inside the object rectangle paints
it; a sample outside leaves the underlying canvas (lower instances or the
background); an overflowing accumulation is `ArithmeticOverflow` (typed,
never a wrap). Object-kind semantics are unchanged: fill value, raster
sample, or bound-palette-entry lookup for index objects.

* whole-pixel translation, integer multiples of 90° rotation, and integer
  zooms are *exact* in Q8 (integer coefficients); general rotation/zoom/pan
  parameters are Q8 approximations whose exactness gap is closed by the
  residual algebra (`F = M(state) ⊕_ρ R`, §22) — the Phase-L court closes a
  30° approximation exactly with a bounded sparse correction;
* the identity affine (`a = e = 256`, rest `0`) deactivates and is never
  stored; while attached, the plain placement `(x, y)` is dormant (the
  affine's translation lives in `c`/`f`) and is restored on deactivation;
* affine, velocity (`0x26/0x27`), and trajectory (`0x2b/0x2c`) state on one
  instance are mutually exclusive (attaching one removes the others);
  affines die with their instances (`0x28`);
* painting scans the whole canvas, so cumulative per-materialization affine
  sample work is capped by `Limits.max_affine_work` (typed
  `MaterializationBudgetExceeded`; parse stays cheap — the bound bites only
  where the work would actually happen).

A Q8 camera-like move is therefore *state*, not a sequence of rasters: the
Phase-L rotating-tile flagship stores 81 frames of rotation as one object +
one instance + one `SetAffine` per interval, and every frame is byte-verified
against an independent painter with a structurally different sampling loop.

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
