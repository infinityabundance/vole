# Phase V.1.5 Receipt — global video structure (V.1 video programme, contract
# `docs/phase-v1-video-architecture.md` §2.8; master brief §61–§63, §248;
# `docs/format-v2.md` deliberately re-frozen with the V.1.5 global-motion
# extension)
# (SEALED)

## Deliverable

V.1.5 adds the **global video structure** layer of the representation ladder:
whole-plane prediction from the immediately previous observation through a
canonical fixed-point map, with encoder proposals that estimate translation /
rotzoom / affine models over dense raster content (`brief §248`: "global
translation proposal; rotzoom proposal; affine proposal — over real video.
Keep fixed-point normative materialization").

* **Normative `GlobalPredict` canvas op** (`media/global.rs` + `core.rs` +
  `wire.rs`): destination `(x, y)` samples the previous materialized
  observation at `((a·x + b·y + c) >> s, (d·x + e·y + f) >> s)` (signed
  floor; `s` ∈ the **map-shift registry** {Q8, Q12, Q16} — brief §62) and is
  painted only when that source lies inside the previous plane; otherwise the
  sample keeps the interval's fresh state render (the sealed `CopyRect` clip
  rule). The arithmetic is the v1 Phase-L integer rule generalized to a
  declared precision — **never floating point in normative materialization**.
  The op is a one-shot canvas op (snapshot semantics, dependency depth 1)
  under **feature bit `0x2`** (tag `0x32`); per-materialization work is capped
  by the new `Limits.max_motion_work` (whole-plane warps cannot multiply
  unbounded — §214 motion-bomb discipline), and hostile records (unknown
  shift byte, out-of-domain coefficients, op without the bit) fail closed
  typed. Additive: files without the bit keep their exact bytes and goldens.
* **Estimator** (`media/global.rs::estimate_global`, encoder-only — f64 is
  permitted there and only there, §100): a deterministic bounded pipeline —
  integer translation over a **2× box-downsample pyramid** (coarse window,
  then ±1 refinement per level; ≤ 24 coarse candidates at the top) scoring
  mean absolute difference; then a **rotzoom** damped least-squares fit
  (zoom/rotation about the frame center + translation, numeric-Jacobian
  Gauss–Newton over a ≤ 4096-point grid with Levenberg damping) and an
  **affine** six-parameter fit (two deterministic starts: from the rotzoom
  result and from the translation), all pruned by the within-tolerance share
  an integer translation explains (an exact pan needs no refinement; a scene
  cut / noise field is hopeless for any model). Everything proposed is
  **quantized** into a canonical [`GlobalMap`] and only lands in `.vole`
  after normative simulation + exact residual + complete-byte cost agree.
* **Encoder families** (`media/encode.rs`): `global_translation` /
  `global_rotzoom` / `global_affine` candidates per interval — warp the
  previous observation over the committed render (normative mirror), close
  the mismatch with the **cheaper of the sparse / transform residuals**, and
  pick the least complete bytes against every other family (RAW/SPARSE stay
  sentinels). Per winning class the map is priced at **every registry
  precision** (or a forced one via `EncodeOptions`), and the cheapest wins —
  ties prefer the lower precision (§62: never assume more precision wins);
  the chosen per-shift bytes are reported (`EncodeReport.map_shift_*`), never
  assumed. `EncodeOptions { map_shift, disable_global }` expose the precision
  and family ablations. Motion is estimated **per plane** (chroma planes on
  their own subsampled grids — independent-plane doctrine, contract §2.6);
  decoder-side shared visual-motion state across components is designed
  across V.1.5–V.1.7 and lands with the container surface of V.1.6/V.1.7
  (recorded, not hidden).

## Courts

`tests/phase_v1_5.rs` (20) + `src/media` unit courts (global 8 · encode 6). |
Result
|---|---|
| `GlobalPredict` == whole-plane `CopyRect` for pure integer translations at Q8 (byte-identical materialization over the same prev) | PASS |
| Warp semantics against a **naive per-sample expectation oracle** (prev at the mapped source when in bounds, the fresh state render otherwise) for a quarter-turn, a 2× zoom, and a general Q8 map | PASS |
| Precision registry: a translation map materializes identically at Q8/Q12/Q16 and at 8/10/16-bit depths (keep-base rule included) | PASS |
| Encoder — camera pan of dense natural-like content (96×64 Gray8, 6 obs, +2/+1 per frame): 5/5 intervals `global_translation`, total 6 643 B vs 30 975 B RAW floor (4.66×), sample-exact through the wire, map-shift accounting == total; the pan run without the global family costs **exactly the RAW floor** (the V.1.5 classes own the pan) | PASS |
| Encoder — continuous zoom-in (low-contrast smooth content, 1.5 %/frame): global records beat RAW (18 673 B < 20 735 B); the §62 precision court reports **identical bytes at Q8/Q12/Q16** on this footage (tie → Q8) and identical deterministic re-runs | PASS |
| Encoder — 10-bit YUV420 multiplane pan (Y +4/+2, chroma +2/+1): all 15 plane-intervals `global_translation` on each plane's own grid, 11 619 B vs 23 805 B RAW (2.05×), sample-exact | PASS |
| Negative controls: iid noise → 0 global observations, bytes == the RAW floor exactly; a hard scene cut stops the run (4 pan intervals only) | PASS |
| Static tail after a pan is held from the previous observation by an identity warp (greedy per-interval economics; temporal-span promotion is V.1.11) | PASS |
| v2 wire (global-motion extension): minimal/additive feature bits (`0x2` alone for global content; `0x3` with family surface; header without the bit parses the V.1.2/V.1.4 bytes unchanged); **V.1.5 extension golden pinned** (`2791d62289d601a59ce0d1f0884738a6f4d939657cc438666f0a72500ecbbae9`); hostile corpus typed never a panic — op without the bit, unknown shift byte, out-of-domain coefficients (writer + reader), unknown feature bits, truncations and flips | PASS |
| Work cap: 64 whole-plane warps in one interval ⇒ `MaterializationBudgetExceeded` under a small `max_motion_work`; the default envelope materializes the same stream | PASS |
| Estimator boundedness + determinism (96×64 pair: 1.71×10⁶ compared samples vs ≈10⁹ exhaustive; identical hypotheses/work on re-run); unit courts recover an exact integer pan (single translation hypothesis) and a 1.05× bilinear zoom (rotzoom/affine near ground truth) | PASS |
| Full A–U / V.1.1–V.1.4 regression (dev / all-features / release) 0 failures; v1 goldens + the V.1.2 and V.1.4 golden bytes unchanged | PASS |

## Measured (release, `examples/global_proof.rs`)

Pan run (96×64 Gray8, 6 obs): **6 643 B vs 30 975 B RAW floor (4.66×)** —
global_translation 5 obs / 6 643 B; the ablation without the global family is
**exactly the RAW floor**. Zoom run (64×64, 6 obs, +1.5 %/frame): **18 673 B
< 20 735 B RAW** with 15 127 B of global records; forced Q8 = Q12 = Q16 =
18 673 B and the auto run chose **Q8** (per-shift bytes reported: {8: 15 127})
— the §62 decision on this footage is *more precision does not win*, measured
not assumed. Multiplane pan (10-bit YUV420, 6 obs): **11 619 B vs 23 805 B
RAW (2.05×)**, 15/15 global_translation. Noise control: **12 492 B == RAW
floor**, 0 global observations. Estimator work 1.71×10⁶ samples on a 96×64
pair. V.1.5 extension golden container 624 B, digest
`2791d62289d601a59ce0d1f0884738a6f4d939657cc438666f0a72500ecbbae9` (pinned).

## Recorded, not hidden

* **The v2 grammar is deliberately extended and re-frozen at V.1.5** (feature
  bit `0x2`, op `0x32`, the map-shift registry): additive — old byte streams
  keep their exact meaning and both earlier goldens; hostile corpus grew with
  the surface; the V.1.5 golden is pinned.
* **The rotzoom/affine byte advantage is gated on sub-pixel prediction.** On
  the V.1.5 corpus (pan, zoom) the integer-translation + exact residual model
  wins or ties every interval — with nearest-neighbour sampling, a zoom/rotate
  map and an identity map leave residuals of the same magnitude, so the
  measured winner is `global_translation`. The rotzoom/affine **decoder
  semantics** (arbitrary canonical maps, courted against the naive oracle and
  CopyRect) and the **estimator proposals** (rotzoom/affine recovery courted
  in unit tests) are sealed here; V.1.7's committed integer interpolation is
  the recorded gate for the residual savings the continuous classes exist to
  capture. Nothing is hidden: per-family bytes and per-shift bytes are
  reported for every run.
* **The greedy encoder holds a settled position from the previous
  observation** (an identity warp per interval) rather than re-syncing once —
  per-interval cost is honest (a hold is ~38 B vs a RAW re-sync of the whole
  plane); temporal-span promotion is V.1.11's job (§43/§102–§105).
* **The estimator is proposal-only** (f64, deterministic per machine,
  bounded: pyramid ≤ 24 coarse candidates + ≤ 9 refinements per level, fits
  over ≤ 4096 grid points, pruning by an exact-within-tolerance share). It is
  **not** the §92/§93 candidate DAG (V.1.11) nor DSFB-governed (V.1.12), and
  cross-plane decoder-side shared geometry (§47/§48) is deferred to the
  V.1.6/V.1.7 container work, recorded here as a decision.
* Real-codec *import* of arbitrary files and the full `import → encode`
  pipeline are V.1.19's seal gate; V.1.5 courts "over real video" with dense
  natural-like raster footage (smooth-octave content, camera renders with
  bilinear resampling) and a 10-bit multiplane pan — the encoder itself never
  depends on how its `Picture` inputs were produced.
* Regression: dev 397 / all-features 399 / release 399, 0 failures
  (was 369/371/371 at the V.1.4 seal); v1 goldens and the V.1.2/V.1.4 golden
  bytes unchanged.

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0) ·
`cargo test` (397, dev) · `cargo test --all-features` (399) ·
`cargo test --release --all-features` (399) · Phase-V.1.5 courts (20) ·
global unit courts · precision/ablation/negative controls · extension golden ·
evidence (`evidence/campaigns/phase-v1-5-…/`) · docs updated (`format-v2.md`
re-frozen, `empirical-status.md`, `PROJECT_STATE.md`, `CONFORMANCE.md`,
README).

## Next

V.1.6 — local motion: region translation / bounded motion field over the
plane domain with region/motion-count limits and motion-bomb courts (brief
§249, §64–§66; contract §2.8).

## Verdict

```
SEALED
```
