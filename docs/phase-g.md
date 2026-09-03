# Phase G Receipt — exhaustive inverse proceduralization (SEALED)

## Deliverable

The first true **raster-origin VOLE encoder**: `src/inverse.rs` consumes an
observed Gray8 raster sequence and, per frame, exhaustively evaluates bounded
procedural candidate programs, reproduces each candidate's frame through the
normative materializer path, keeps only byte-exact explanations, and emits the
complete-cost winner. Every produced stream is decoded with the normative
decoder and verified frame-for-frame against the input raster before it is
returned (`report.exact`); the encoder returns a typed error rather than an
unverified stream.

## Normative format additions (v1 continues to evolve; old streams unchanged)

| Tag | Transition | Semantics |
|---|---|---|
| 0x28 | `ClearInstances` | drop every live instance (ids freed) — full-content replacement |
| 0x29 | `ClearOverlay` | drop every persistent overlay point |
| 0x2a | `Residual { block }` | per-frame residual **canvas op**: Phase-F self-describing block whose decoded bytes are a canonical strict-sorted in-canvas sparse point list; applied in op order after COPY/MOVE; one-shot and stateless (`F = M(state) ⊕_ρ R`) |

New bounds: `Limits.max_overlay_points` (persistent overlay cap),
`Limits.max_residual_bytes` (decoded residual bound, wire slack +1024),
`Limits.max_stream_bytes` and `Limits.max_checkpoint_distance` are now
*enforced* at parse (stream length, interval count) and mirrored in the
encoder validator (`src/format.rs`, `src/limits.rs`, `src/encoder.rs`).
`rans::check_block` gives parse-time structural validation of residual blocks
without forcing an entropy decode of frames that may never be materialized.

## The exhaustive candidate space (whole-frame granularity; Phase K adds regions)

Per frame ≥ 1 the encoder evaluates: UNCHANGED (zero-transition lane) · FILL /
clear-only resets · EXACT_OBJECT_REF (content-addressed reuse) · RAW (raster
object sentinel, always present) · SPARSE (persistent overlay commit) ·
one-shot RESIDUAL / RANS_RESIDUAL (Phase-F coded block) · COPY_RECT (toroidal
wrap scrolls; screen-scroll = rect + residual strip; prev-frame diff) ·
TRANSLATION (whole-pixel instance translation, window `|dx|,|dy| ≤ r`). Every
candidate is a declarative program (state transitions + canvas ops); validity
is established by materializing its expected frame with the same normative
primitives the decoder runs and comparing byte-for-byte with the target;
winners are the minimum persisted-byte program, tie-broken deterministically
by enumeration order. Candidate counts and per-family aggregates are recorded
per frame (§28 decision records). Background choice sweeps a deterministic set
({0,255} ∪ frame-0 corners ∪ global mode) and keeps the cheapest complete run.

Gates (documented on `SearchSpace`): canvases ≤ 16K samples run the full 1D
scroll scan; larger canvases row-hash-prefilter the 1D scroll families
(equality is always byte-verified before acceptance; exactness never depends
on a hash). 2D toroidal scrolls enumerate fully when the candidate count is
≤ 4096.

## Courts

`tests/phase_g.rs` (13 tests; all pass, byte-exact vs independent generators):

- static identical sequence → `unchanged` (13 B intervals); amortized cost
  dominated only by the frame-0 object;
- blinking pixel over a static base → `sparse` (1 residual point/interval);
- cycling uniform panels → fill/object reuse (`raw` first use, then
  `exact_ref`), never raster-proportional;
- diagonal pan → `translation` per frame; the VOLE raster-only baseline is
  > 8× larger on the same input;
- whole-canvas wrap scroll → `copy_rect`, exact against an independent
  row-permutation oracle;
- screen scroll with brand-new rows → `copy_rect`/`copy_residual`, never RAW;
- glitch-then-revert → `sparse` corrections;
- noise negative control → `raw` every frame with < 15% overhead;
- object appearing over a static scene → measured rebase (RAW declaration at
  the appearance frame, then the unchanged lane);
- oracle record integrity: winner payload == min over every evaluated family
  (regret 0), aggregate counts consistent, every winner materialized-exact;
- background sweep picks the scene background;
- single-frame and mismatched-canvas handling (typed errors);
- large-canvas (320×64 > gate) hash-prefiltered scroll search stays exact.

`tests/malformed.rs` Phase-G hostile courts (6 new): unsorted residual points,
out-of-canvas residual points, residual length bombs, truncated RANS-kind
blocks, malformed kind bytes — all typed errors, never panics; clear ops
round-trip and free ids for reuse.

## Measured (evidence/campaigns/phase-g-inverse-1788461583, release)

| court | canvas | frames | vole | raw | winner families | exact |
|---|---|---|---|---|---|---|
| §76 moving box | 1920×1080 | 101 | 2 076 291 B | 209 433 600 B | translation (100×26 B) | ✓ |
| static desktop | 1920×1080 | 240 | 2 076 798 B | 497 664 000 B | raw×1 + unchanged×239 (13 B) | ✓ |
| structural timeline | 640×360 | 89 | 244 481 B | 20 505 600 B | translation×39, rans_residual×5, unchanged×43, … | ✓ |
| screen scroll | 96×96 | 21 | 35 347 B | 193 536 B | copy_residual×20 | ✓ |
| cycling panels | 192×108 | 60 | 42 894 B | 1 244 160 B | exact_ref reuse | ✓ |
| noise (negative) | 64×64 | 12 | 49 738 B | 49 152 B | raw×12 (+1.2%) | ✓ |

Flagship (§76) measured split: VOLE procedural 2 076 291 B vs VOLE raster-only
209 438 191 B (100.9×) vs raw 209 433 600 B. Frame 0 pays one full-raster
object declaration (2 073 613 B) — the honest whole-frame-granularity rebase
cost of inverse proceduralization; every one of the 100 following intervals is
a 26 B integer-translation state evolution (frame 1: family `translation`,
payload 26 B, 152 candidates evaluated, 125 valid). The authored Phase-E
stream for the same content is 1 505 B total; the difference is precisely the
frame-0 raster tax that Phase K (region extraction) and Phase Q (native
procedural ingest) exist to remove.

## Honest accounting & scope notes

- Procedural-fraction (§32) values are low on these whole-frame courts (≈
  0.001–0.03) because frame-0 whole-raster declarations dominate the bytes;
  the *marginal* cost per interval is bytes-tiny (13–26 B). This is recorded,
  not hidden.
- Per-frame decisions are greedy and independent. Measured temporal gaps
  (velocity setup vs per-frame `SetPosition`, persistent-overlay promotion of
  stable residuals, checkpoint/RAW-capture timing, "static after canvas-op"
  repeat frames at 38 B) are the documented Phase O re-optimization surface,
  not silently optimized here.
- Decoder size does not grow with encoder intelligence: the stream only ever
  contains the winner program; all candidate machinery is encoder-side.

## Verdict

```
SEALED
```
