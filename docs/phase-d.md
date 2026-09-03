# Phase D — 2D COPY_RECT / MOVE_RECT (machinery)

## Status

Mechanism: **IMPLEMENTED** (wired writer/parser/decoder/compositor + hostile
bounds). Phase *court*: PARTIAL — the core geometry/precedence court is sealed,
but the domain-winning terminal/editor-scroll court is gated behind a
*transient-patch* operator (so COPY_RECT reuses pixels the pure painter would
otherwise not re-derive), which is next-phase work and is recorded rather than
faked.

## What COPY_RECT is here

A `COPY_RECT` transition copies a **declared rectangle from the immediately
previous decoded full frame** into the current frame's base. It is a true
frame-referencing state materialization op (not a codec-only heuristic): the
replayer keeps the prior frame's canvas as an explicit dependency at depth 1
and the compositor reproduces exactly the same rasters deterministically.

Materialization order per interval (normative):

1. paint the persistent `State` (background, instances) and its persistent
   sparse overlay → base;
2. for each COPY_RECT/MOVE_RECT op (stream order), overwrite the base rectangle
   from the previous final frame, with canonical **snapshot-copy** semantics
   (source samples are read from the prior distinct canvas, so no aliasing) and
   per-sample bounds clipping;
3. return the finished frame; the next interval's ops reference it.

A stream containing COPY_RECT is therefore sequential: `Decoder::materialize(i)`
replays from the checkpoint to `i` (dependency depth is bounded to 1); the cost
of seek is an explicit accounting concern for a later transport phase.

## Bounds (hostile)

`w,h ≥ 1`, coords within `2^24`, `w*h ≤ Limits.max_copy_area` — enforced in both
the byte parser (`format.rs`) and the encoder validator (`encoder.rs`) so no
out-of-limit rectangle is ever serialized (typed `NonCanonicalEncoding` /
`MaterializationBudgetExceeded`).

## MOVE_RECT

MOVE_RECT is accepted and behaves as CopyRect in Phase D; a destination-clear
(~remove-from-source) mask is documented future work, not silently claimed.

## Courts (tests/phase_d.rs — all pass)

- compositor: painter op + CopyRect-from-prior reproduces a hand-verifiable
  dual-marker raster;
- parser accepts MOVE_RECT and materializes;
- snapshot/clipping unit test for `rect_copy`;
- hostile: oversized and zero-size copy rejected. Raw materialization cost of a
  repeating copy op must be tracked with the transient-patch phase.

## Open / next

Transient-patch (the operator that lets scroll content differ per interval yet
reuse prior pixels), real terminal/editor scroll court + its duplicate-object
reuse measurement, then the noise negative control, before Phase D is SEALED.
