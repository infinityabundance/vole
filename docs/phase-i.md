# Phase I Receipt — bounded parametric trajectories (SEALED)

## Deliverable

**Trajectories as first-class procedural state** (`src/trajectory.rs`,
`src/state.rs`, `src/transition.rs`): a finite, deterministic motion program
attached to one instance, made of ordered segments —

* `Linear { vx, vy, steps }` — constant velocity (an exact hold when
  `(vx,vy) == (0,0)`; "constant"/"linear" motion in §20 terms);
* `Accel { vx0, vy0, ax, ay, steps }` — constant acceleration with the exact
  discrete integer semantics `pos += v; v += a` per advance (closed form
  `Δ(t) = t·v0 + a·t·(t−1)/2`, i.e. velocity during advance `k` is
  `v0 + k·a`; the integer form of `x(t) = x0 + v0·t + ½·a·t²`).

One advance is applied per explicit `AdvanceTrajectories` transition — the
Phase-E choice of explicit stepping is preserved so the sealed unchanged lane
is untouched. A program deactivates exactly when its final segment's steps are
exhausted (the instance then stays put); an empty program deactivates
immediately. Trajectory and translation state on one instance are mutually
exclusive. All arithmetic is checked; **no floating point anywhere** in the
normative path.

**Trajectory collapse (§43)** (`src/collapse.rs`): the first
equivalence-preserving re-optimization primitive. A maximal run of interval
groups that each carry exactly one `SetPosition` for the same instance is
rewritten into **one `SetTrajectory` + per-frame `AdvanceTrajectories`** only
when (1) the rebuilt stream is decoded with the normative decoder and is
**byte-identical frame-for-frame** to the original, and (2) the rebuilt stream
is **strictly smaller** in total bytes. Exact fits are tried in canonical
order (linear, acceleration, piecewise-linear); the fit is never trusted —
reconstruction is proven.

## Normative format changes (v1 continues to extend; old streams unchanged)

| Tag | Transition | Semantics |
|---|---|---|
| 0x2b | `SetTrajectory { id, segments }` | attach a bounded program (count 0 = deactivate; clears any translation on the instance) |
| 0x2c | `AdvanceTrajectories` | apply one advance of every active trajectory program |

New bounds: `Limits.max_trajectory_segments` (256 segments per program) and
`Limits.max_trajectory_work` (1 << 22 cumulative trajectory steps, counted
*pre-apply* so a program deactivating on the very step is still counted),
enforced in the parser, the encoder validator, and the writer.

Canonical rules (hostile, typed): `steps ≥ 1`; every signed literal
`|·| ≤ 2^24`; an `Accel` with `(ax,ay) == (0,0)` must be written `Linear`;
two adjacent `Linear` segments with the same velocity must be merged. The
byte accountant (`inverse::account_stream`) and the docs (`format-v1.md`,
`transitions.md`, `procedural-state-graph.md`) are in sync.

## Courts

`tests/phase_i.rs` (14 tests; all pass, byte-exact end-to-end) +
`tests/malformed.rs` (8 new Phase-I hostile courts) + unit tests in
`src/trajectory.rs` / `src/collapse.rs`:

* accelerating §76-analogue flagship (1920×1080, one 200×100 box,
  `v(t) = (2+t, 1)` for 40 intervals): 41 exact frames vs an independent
  closed-form reference painter; interior/trailing sample checks follow the
  analytic positions;
* byte comparison of the *same exact frames* under three representations:
  trajectory (686 B) < per-frame `SetPosition` (1 132 B) < per-frame
  `SetVelocity` rewrite (1 172 B); all three decode to the identical
  sequence; representation is not raster-proportional (raw 85 MB);
* piecewise-linear motion (move → hold → reverse): 61 exact frames, the hold
  really holds;
* deactivation: after the program's duration the state is stationary and the
  unchanged lane resumes;
* exclusivity: trajectory ↔ translation state on one instance; empty-program
  deactivation; unknown-instance typed errors;
* closed-form simulator vs the normative state stepper: 200 random canonical
  programs, positions agree on every advance;
* hostile work budgets rejected by both encoder and parser
  (`MaterializationBudgetExceeded`, fast); oversize programs rejected;
* non-parametric hypotheses rejected: any fit returned for a random walk
  reproduces it exactly when re-simulated (no wrong model is ever accepted);
* raster-origin collapse court: Phase-G greedy encode of an accelerating
  whole-canvas sprite → collapse strictly shrinks the stream and the normative
  decode equals the input raster; raster noise never collapses.

## Measured (evidence/campaigns/phase-i-trajectory-1788466084, release)

| court | frames | representation | bytes | ratio | exact |
|---|---|---|---|---|---|
| accel flagship 1920×1080 | 41 | trajectory (1 set + 40 adv) | 686 B | — | ✓ |
| | | per-frame `SetPosition` | 1 132 B | 1.65× | ✓ |
| | | per-frame `SetVelocity` | 1 172 B | 1.71× | ✓ |
| | | raw all frames | 85 017 600 B | 123 932× | — |
| piecewise 320×200 | 61 | trajectory (3 segments) | 992 B | — | ✓ |
| | | per-frame `SetPosition` | 1 652 B | 1.67× | ✓ |
| static hold 1920×1080 | 201 | zero-velocity program | 2 918 B (14.52 B/frame amortized) | unchanged lane 13 B/frame is cheaper | ✓ |
| raster linear pan 64×64 | 40 | greedy (Phase G) | 5 201 B (interval transitions 1 014 B) | — | ✓ |
| | | collapsed | 4 759 B (interval transitions 572 B) | total 0.915×, interval 0.564× | ✓ |
| raster accel 48×24 | 8 | greedy | 1 425 B (interval 182 B) | — | ✓ |
| | | collapsed (accel fit) | 1 375 B (interval 132 B) | total 0.965×, interval 0.725× | ✓ |
| noise 24×16 (negative) | 12 | RAW | 5 194 B | collapse fixpoint (0 runs) | ✓ |
| random walk (negative) | 41 | authored SetPosition | 1 132 B | collapse fixpoint (no paying fit) | ✓ |

Motion intervals collapse from 26 B/frame (`SetPosition`) to a 14 B/frame
steady state (`13 B envelope + 1 B advance`) plus one amortized descriptor;
on the accelerating raster court the run is short (7 frames), so the 34 B
acceleration descriptor still dominates — measured, not hidden.

## Adopted / rejected / recorded

Adopted: bounded parametric trajectories as first-class state; exact integer
semantics with an explicit closed form; explicit advance op (never implicit
stepping); deactivation at program end; exclusivity with translation state;
canonical program forms; separate `max_trajectory_segments` /
`max_trajectory_work` limits enforced at parser + encoder + writer; collapse
accepts only exact (normative-decode-proven) and strictly-cheaper rewrites.

Rejected: floating-point trajectories (normative path is integer-only);
implicit time-driven advance (would corrupt the sealed unchanged lane);
per-frame coordinate payloads as the motion representation (measured 1.65×
more bytes than the trajectory on identical frames).

Recorded, not hidden: statics must stay in the unchanged lane (an active
zero-velocity trajectory costs 14.5 B/frame vs 13 B/frame — measured);
trajectory descriptors only pay from runs of ≥ 3 frames; short accelerating
runs amortize the descriptor poorly; the collapse pass is one re-optimization
family — Phase O generalizes it into `vole optimize` (with piecewise motion
estimation over rasters and the other §44 families under the same
exact-and-cheaper invariant); the greedy per-frame encoder itself is unchanged
(Phase G/H streams byte-stable).

## Verdict

```
SEALED
```
