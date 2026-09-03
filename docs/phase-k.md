# Phase K Receipt — variable regions in the inverse encoder (SEALED)

## Deliverable

The raster-origin inverse proceduralizer gains a **variable-region family**
(`src/inverse.rs`): per frame it partitions the target/base diff into tiles of
a granularity and, for every diff-bearing tile, declares the tile's diff
**bounding box** — a *rectangular* region of any aspect — as an immutable
object holding the target's own sub-rectangle, painted above the base by a
fresh `CreateInstance`. Granularity ladder 64 → 32 → 16 → 8; the cheapest
complete cover (or the whole-frame reset, sparse, residual, translation, copy
families) wins the per-frame cost court. Localized change therefore **never
needs a whole-canvas declaration**, and identical region content is reused by
exact BLAKE3 identity with zero declaration bytes.

## Normative format changes

None. Phase K is encoder-side search over the existing normative state model
(objects of arbitrary box geometry + instances at arbitrary positions already
exist since Phase A); `.vole` grammar, materializer, and decoder are
untouched. Old streams and all earlier encoder behavior on content those
families serve remain byte-stable except where the new family genuinely wins.

## Bounded candidate space (documented gates)

* regions are evaluated only when a per-frame diff exists and is ≤ a quarter
  of the canvas (a larger diff cannot beat the whole-frame reset sentinel —
  a full-canvas region is a reset plus a redundant create);
* at most 256 rectangles per candidate;
* a candidate is skipped when any changed sample is shadowed by a persistent
  overlay point (overlay paints above every instance; those frames are the
  residual/sparse families' business);
* Full mode walks the 64→32→16→8 ladder; Probe mode evaluates the fixed 16
  granularity only (fixed heuristic / DSFB rotating sweep).

Candidate validity is analytic (rectangles cover every diff sample by
construction and re-paint only target-exact content; unchanged samples under a
rectangle are re-painted with their own value), and the encoder's standing
invariant still holds: every committed winner is re-materialized and compared
byte-for-byte with the target, and the whole stream is decode-verified
end-to-end before it is returned.

## Courts (`tests/phase_k.rs`, 9 tests; all pass, byte-exact end-to-end)

* localized changes never rebase the whole frame: 40 region frames, zero
  `raw` declarations after frame 0;
* region representation is not raster-proportional (well under ⅛ of the raw
  sequence);
* exact-ref region reuse: an alternating glyph area is served by two objects
  reused across 35 frames with zero declaration bytes at the reuse floor
  (envelope + one create);
* granularity/rectangular content (full-width banner + dense block + 8×8
  cell) stays exact with no rebases;
* DSFB governance: guided search is byte-identical to the exhaustive oracle
  on steady region content with strictly fewer candidates, and reaches the
  same per-frame winners;
* overlay-shadowing negative control: a blinking dense block stays in the
  sparse family (regions cannot paint through the overlay) — exact;
* noise negative control: the diff gate skips regions; every frame stays RAW
  with bounded overhead;
* oracle invariant: winner payload == minimum best payload over every
  evaluated family; region winners' interval bytes == envelope + 17 B ×
  created rectangles;
* bounded candidate space: at most 4 region candidates per frame.

## Measured (evidence/campaigns/phase-k-regions-<ts>, release)

| court | frames | result | exact |
|---|---|---|---|
| localized flag 1920×1080 (200×120 clock/frame + full-width status bar) | 41 | 3 237 461 B total; **26× vs raw**; winners `raw×1 regions×40`; **rebases after frame 0 = 0** | ✓ |
| reuse court 160×120 (glyph alternation) | 37 | exhaustive 26 309 B (`raw×1 regions×36`); DSFB **J 1.000** at N 0.378×; fixed heuristic J 1.036 (probe-granularity blindness, measured) | ✓ |
| rectangular shapes 128×96 | 17 | `regions×16`, 96 region creates, no rebases | ✓ |
| noise 48×32 (negative) | 12 | `raw×12`, bounded overhead | ✓ |

The frame-0 whole-canvas declaration (≈ 2.07 MB at 1920×1080) remains — the
base must be established once; Phase Q (native ingest) and archive profiles
address it further. Region *instances* persist in state (a region stays
repaired until over-painted or cleared); long-horizon instance retirement and
region+residual composites (dense region with a sparse dust remainder) are
recorded Phase-O re-optimization surface, not hidden.

## Verdict

```
SEALED
```
