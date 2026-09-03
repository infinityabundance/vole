# Phase D Receipt — 2D COPY_RECT / MOVE_RECT (SEALED)

## Mechanism

`COPY_RECT` / `MOVE_RECT` are first-class v1 transitions (`0x24`/`0x25`) that
copy a declared rectangle from the **immediately previous decoded frame** into
the current base frame. This is a true *frame-referencing materialization op*
at bounded dependency depth 1 — it is not a codec-only heuristic. Semantic
order per interval:

1. paint persistent `State` (background + instances + persistent overlay);
2. apply each COPY_RECT/MOVE_RECT in stream order from the *prior* final
   frame with canonical **snapshot-copy** (source read from the distinct prior
   canvas ⇒ no aliasing) and per-sample clipping;
3. enqueue the finished frame; the next interval references it.

`STATE` is unchanged by these ops, so a frame sequence can differ across
intervals purely from recycling prior output pixels — exactly the mechanism
screen scroll needs. A COPY-bearing stream is sequential (replay from the
checkpoint), and seek cost is deferred explicitly to the transport phase.

## Hostile bounds (both parser and encoder enforce)

`w, h ≥ 1`; coords within ±2^24; `w·h ≤ max_copy_area`. Out-of-limit / zero /
non-representable geometry is a typed `NonCanonicalEncoding` /
`MaterializationBudgetExceeded`; never a panic or unbounded allocation.

## Courts (`tests/phase_d.rs`, 7/7 pass)

- dual-marker scenario: `SetPosition` + COPY_RECT-from-frame0 both reproduce
  exactly;
- **wrap-scroll court**: whole 96×96 canvas scrolled up `S=3` rows/interval as
  **two COPY_RECTs**. Frames match an *independent* row-permutation oracle
  `row_y(t) = initRow[(y+t·S) mod H]` — none of the intermediate frames is
  reproducible to the immutable painter State, so COPY_RECT is load-bearing;
- MOVE_RECT parses/materializes; snapshot+clip unit test; oversized and
  zero-size geometries rejected;
- **noise negative control**: prior-frame uncorrelated content cannot be
  losslessly produced by COPY (reuse == 0); the composited raster does not hide
  a mismatch, so an exactness-gated encoder must fall back to literal/RAW
  rather than corrupt (lossless authority holds).

## Measured (evidence/campaigns/phase-d-scroll-…)

96×96, S=3, 12 intervals → 13 oracle-verified exact frames. Stream 10 063 B
contains ONE initial raster (declared as an immutable object) plus 12
intervals of two size-independent descriptors; raw-all frames = 119 808 B.
Incremental scroll cost scales with descriptors (rect count), **not** with
canvas area.

## Negative result recorded

Fairness note (not hidden): on the court the initial screen is stored once as a
full immutable object (~9.2 KB of the 10 KB); the genuine Phase-D win is that
12 subsequently-varying frames add only rect descriptors — no frame raster is
stored per interval. A large document screen amortizing one initial raster over
many scroll steps is where this dominates; tiny screens with few frames let
descriptor overhead dominate (documented, not erased).

## Adopted / rejected

Adopted: COPY_RECT (wrapper form for MOVE_RECT carried this phase), snapshot
semantics, depth-1 dependency discipline, encoder+parser hostile bounds.
Rejected: MOVE_RECT destination-clear mask (deferred), transient operators not
needed for the oracle-exact court and pushed to the terminal/editor phase if a
use case demands them.

## Verdict

```
SEALED
```
