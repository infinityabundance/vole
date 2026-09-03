# Phase L Receipt — bounded fixed-point affine / global state (SEALED)

## Deliverable

Affine/global transforms become **procedural state** (Phase L of the ablation
ladder: pan, zoom, rotation, camera-like transforms) rather than per-frame
rasters or codec-local block motion. A `SetAffine` transition (tag `0x30`)
attaches a canonical Q8 fixed-point placement to one instance:

```text
(su, sv) = ((a·x + b·y + c) >> 8, (d·x + e·y + f) >> 8)
```

Every destination pixel samples the object through this integer map (signed
`>> 8` is floor division — the canonical rounding); a sample inside the object
rectangle paints it, outside leaves the underlying canvas. There is **no
floating point** in the normative path, and the exactness gap of a Q8
approximation of a continuous camera move is closed by the residual algebra
(`F = M(state) ⊕_ρ R`, §22).

## Normative format changes

* tag `0x30` (`SetAffine`): `iid:u32` + six `i32` Q8 coefficients, each
  `|·| ≤ 2^24` (canonical checks reject out-of-domain coefficients typed at
  parse);
* `Limits.max_affine_work` — cumulative per-materialization affine sample work
  cap (an affine placement scans the whole canvas), default 8 full 1920×1080
  canvases; enforced at materialization (typed
  `MaterializationBudgetExceeded`; parse stays cheap because the work only
  happens where frames are materialized);
* accounting: `account_stream` walks `0x30` (`transition_bytes += 29`).

No other grammar or materializer semantics changed; old streams re-parse
unchanged.

## Semantics (normative)

* identity affine (`a = e = 256`, rest 0) deactivates and is never stored;
  while attached, the plain `(x, y)` placement is dormant (the affine's
  translation lives in `c`/`f`) and is restored on deactivation;
* affine, velocity (`0x26/0x27`), and trajectory (`0x2b/0x2c`) state on one
  instance are mutually exclusive (attaching one removes the others); affines
  die with their instances (`ClearInstances`);
* object-kind semantics under an affine are identical to the plain placement:
  fill value, raster sample, or bound-palette-entry lookup for palette-index
  objects;
* whole-pixel translation, integer multiples of 90° rotation, and integer
  zooms are *exact* in Q8 (integer coefficients); general rotation/zoom/pan
  parameters are Q8 approximations;
* an overflowing accumulation is `ArithmeticOverflow` (typed, never a wrap).

## Courts (`tests/phase_l.rs`, 11 tests; all pass, byte-exact end-to-end)

* quarter-turn rotation is exact and periodic: 41 frames, byte-identical vs an
  **independent incremental painter** (per-row accumulation vs the
  materializer's direct products — a shared arithmetic bug cannot mask a
  mismatch); four quarter turns = identity; the content mark moves with the
  rotation;
* integer 2× zoom and sub-pixel (0.5 px) pan are exact Q8 maps;
* 12 deterministic pseudo-random Q8 parameter sets: materializer and the
  independent painter agree pixel-for-pixel on every frame;
* affine state beats re-encoding the same visual frames: the 40-frame
  rotation as state vs the same frames through the raster-origin encoder —
  affine streams are ≥ 4× smaller (measured 7× on the evidence run);
* residual closure: a float-rendered 30° rotation target is reproduced
  byte-for-byte by the best Q8 approximation + one persistent sparse
  correction (58 of 4 096 tile pixels differ — the Q8 exactness gap of a
  camera map is a small edge set, closed exactly);
* affine over a palette-index object (sampled indices resolve through the
  bound palette) and over a fill object (uniform value under the map) —
  byte-exact vs independent references;
* typed semantics: exclusivity with velocity/trajectory, identity
  deactivation, `UnknownInstance`, out-of-domain coefficients;
* work budget: 9 affine instances on 1920×1080 exceed `max_affine_work` —
  parse succeeds, materialization fails typed (never a panic);
* hostile wire: out-of-domain coefficient patched into a canonical stream
  fails `NonCanonicalEncoding` at parse;
* accounting: buckets sum to `total_bytes == stream length` on an affine
  stream.

## Measured (evidence/campaigns/phase-l-affine-1788473290, release)

| court | frames | result | exact |
|---|---|---|---|
| rotating-tile flagship 320×180 (64×64 tile, quarter turn per interval) | 81 | 7 547 B total; interval cost 42 B = 13 B envelope + 29 B `SetAffine`; **618× vs raw** (4 665 600 B) | ✓ |
| rotation flattening-tax 160×160 (same visual frames via the raster encoder) | 41 | affine state 5 867 B vs 41 077 B re-encoded (7×); discovery is Phase O/Q surface | ✓ |
| zoom + sub-pixel pans 320×180 | 4 | 4 313 B | ✓ |
| residual closure 30° 160×160 | 41 | 5 263 B; 58 residual points (1.4% of tile) close the Q8 gap exactly | ✓ |
| affine over palette-index / fill objects | — | both byte-exact vs independent references | ✓ |

Recorded, not hidden: affine discovery inside the raster-origin inverse
encoder is deliberately absent from Phase L (the flattening tax above is
measured); the encoder's frame court is unchanged and no encoder winner
changed, because affine candidates are authored procedural state in this
phase.

## Verdict

```
SEALED
```
