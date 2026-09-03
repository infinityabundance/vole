# Phase M Receipt — deterministic integer transform residual floor (SEALED)

## Deliverable

When procedural state cannot explain a **dense smooth residual**
economically, VOLE now behaves like a conventional coder: the per-frame
residual field is partitioned into aligned `4×4` blocks, decorrelated by a
normative **integer lifting DCT**, and entropy-coded as a new residual block
kind. This is the "transform residual floor" of the ablation ladder — added
without leaving the lossless domain (no quantization, no floating point).

## Normative format changes

* residual block **kind `2`** (tag `0x2a`): the signed field `target − base`
  over aligned `4×4` blocks (edge blocks zero-padded);
  `[2][tfm=0][mask][u32 dc_len][u32 ac_len][dc container][ac container]` —
  mask bit per block (LSB-first, padding bits must be 0), DC coefficients
  (4 zigzag `u32 LE` bytes per coded block) and AC coefficients (60 bytes
  per coded block) in row-major block order, each in a standard Phase-F
  RAW/rANS container;
* transform id `0` (TSF-4×4 v1); unknown ids fail closed typed;
* decoder: inverse-transform each coded block (normative lifting DCT) and
  **add** the reconstruction to the canvas; a result outside `0..=255` is
  `OutOfBounds`;
* no new tag, no new `Limits` field (payload bounded by `max_residual_bytes`;
  decode work bounded by the canvas block count);
* accounting: inline entropy models are a **sub-bucket** (`model_bytes`,
  excluded from `residual_bytes`) so the ten buckets sum to the stream
  length exactly. This fixes a latent double count (rANS point residuals
  previously counted their 512 B model twice) — recorded, not hidden.

## The transform (normative, exact, integer-only)

The 1-D DCT-II stage is built from butterflies and two rotations, each
rotation factored into three reversible Q8 lifting steps
(`A = −tan(θ/2)`, `B = sin θ` in units of 1/256, signed `>> 8` is floor):

* even part (`θ = −π/4`): `A = 106`, `B = −181`;
* odd part (`θ = −π/8`): `A = 51`, `B = −98`.

Each lifting step is inverted by subtracting the *same* floor term, so the
whole map is an invertible integer transform with no division ambiguity; the
butterfly halves are always exact on canonical streams
(`inverse(forward(x)) == x`, asserted over thousands of random blocks and by
every end-to-end decode). The 2-D transform is separable: forward rows then
columns (`C = T·X·Tᵀ`); inverse columns then rows.

## Courts (`tests/phase_m.rs`, 9 tests + 7 transform unit courts; all pass)

* brightness-drift flagship: dense whole-canvas smooth deltas are served by
  the transform floor on every interval, strictly cheaper than the RAW reset
  and the point residual on the same frame, byte-exact end-to-end;
* full-range drifting ramp (Gray8 wrap) and textured drift: transform floor
  appears among the winners, streams decode byte-identical;
* noise negative control: transform never wins; RAW stays;
* sparse gate: single-pixel blinks never evaluate the transform family;
* oracle invariant: winner payload == min over every evaluated family;
* same-delta comparison: the transform block is a fraction of the Phase-G
  point container;
* hostile courts at parse (unknown transform id, padding bits, length
  disagreement, truncation — typed before the trailer) and at materialization
  (rANS-model corruption → `EntropyCorrupt`; huge-coefficient RAW payload →
  `OutOfBounds`), plus crate-internal apply-layer courts.

## Measured (evidence/campaigns/phase-m-transform-1788475312, release)

| court | frames | result | exact |
|---|---|---|---|
| brightness-drift flagship 1920×1080 (curved non-scrollable base, +1..+8) | 9 | 2 632 475 B total (7× vs raw); **69 848 B/interval** vs 2 073 645 B RAW reset (29.7×) and 10 467 936 B point residual (150×); winners `raw×1 transform_residual×8` | ✓ |
| same-delta 480×270 (dense smooth) | — | transform block 5 906 B vs 549 268 B point container (**93×**) | ✓ |
| textured drift 128×96 | 8 | transform winner present; buckets sum to stream | ✓ |
| wrap-ramp drift 160×120 | 7 | transform winner present; every frame exact | ✓ |
| noise 64×48 (negative) | 6 | 18 748 B, winners `raw×6` | ✓ |
| strategies 160×120 (Exhaustive vs FixedHeuristic) | 7 | both 29 575 B, J 1.000 — the floor is probe-reachable | ✓ |

## Recorded, not hidden

* 4×4 blocks only in v1; block-size extension and per-coefficient-position
  contexts are Phase-O surface (v1 uses one order-0 byte model per container);
* a transform residual cannot win on tiny diffs (gate) or on noise
  (measured): the floor is one more bounded candidate the cost court may
  reject;
* Phase H re-measurement: the fixed-heuristic scroll-by-7 court no longer
  measures raw-rebase count — the transform floor absorbs dense scroll frames
  at ~950 B — so the probe-blind stream is now measured by cost
  (23 385 B vs the copy-serving 2 883 B = 8.1×; pre-Phase-M it was 11.5× by
  rebase). Copy blindness of the fixed probe is unchanged.

## Verdict

```
SEALED
```
