# Phase C Receipt — sparse mutation

*Persistent sparse overlay + canonical sorted patches, exact materialization.*

## Implemented

A sparse overlay layer on procedural [`State`] painted *above* all instances
(authoritative pixel writes) that persists until overwritten, plus the
`PatchSparse` transition carrying **strictly sorted**, canonical `(x,y,v)`
points (unsorted/duplicate lists are a typed `NonCanonicalEncoding`), a v1
wire tag `0x23`, and hostile-input bounds on patch counts
(`≤ max_canvas_bytes`). `SPARSE` + independent-reference courts.

## Courts (tests/phase_c.rs, all pass)

- blink court: 640×360, 64 intervals toggling one pixel; materialized frames
  exact against an independent painter; f=1 overlay 0 / f=2 255 / base 128
  match predictions.
- unsorted sparse ordering rejected (`NonCanonicalEncoding`).
- cost law: sparse representation scales with *changed pixels*, not rasters.

## Measured results (evidence/campaigns/phase-c-…/)

Blink: stream 1,820 B materializes 65 exact frames; raw identical sequence
would be 14,976,000 B (≈ 8 229 ×). Overlay pixel persists deterministically;
only that pixel's value varies per interval.

## Negative controls

`noise`/raster-random controls are deferred to the native entropy floor phase
(recorded, not fabricated). The "blinking equals full frames" straw-man is
rejected by the cost-limit assertion above.

## Adopted / rejected

Adopted: persistent sparse overlay, canonical strict-sorted sparse patch,
exactness under sparse overlay. Rejected: none in scope.

## Verdict

```
SEALED
```
