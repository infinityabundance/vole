# Phase S Receipt — partial materialization: `Rect`/`Tile` views (§16/§37/§66) (SEALED)

## Deliverable

1. **`View::Rect` / `View::Tile`** (`src/view.rs`, §16): typed partial
   materialization targets — an arbitrary axis-aligned sub-rectangle in frame
   coordinates and one tile of a canonical tile grid anchored at the canvas
   origin. `View::clip` computes the in-canvas `ViewBox` intersection with
   checked arithmetic; zero-size geometry and out-of-canvas requests are typed.
   Views are requests, never stream syntax: **no wire-format change.**
2. **`vole_video::partial`** (`src/partial.rs`, §37/§66): demand-planned
   partial decode. Because every interval canvas op (COPY_RECT / MOVE_RECT /
   residual) reads **only** the previous frame and overpaints the freshly
   materialized base, the value of frame `t` at any position is determined by
   the base state paint, the last canvas op covering it (→ frame `t−1`'s
   value at the op source), or the op itself (residual). A backward **demand
   plan** therefore collects, per level, exactly the region of frame `t−1`
   that frame `t` reads; a forward replay then applies state transitions and
   paints only demanded regions (background / instances / overlay / ops).
   Levels with empty demand are skipped entirely. Demand regions are merged
   per-row spans that **saturate to the whole canvas** beyond a bookkeeping
   budget — a sound over-approximation that keeps hostile streams in the
   whole-frame memory class and is never output-unsafe.
3. **Canonical whole-frame path**: `View::FullFrame` (and any full-canvas
   box) replays the canonical step machinery (`materialize_full` +
   `decoder::step_frame`) — byte- and error-identical with whole-frame decode
   by construction. Sub-frame views validate everything contributing to the
   region (residual containers are fully decoded/validated; bounds against the
   canvas, never the partial box) and paint only demanded samples — the
   documented **audit-scope boundary**: content that never contributes to a
   view is not audited; whole-frame decode remains the canonical audit path.
4. **`PartialStats`**: painted sample writes (base/copy/residual), levels
   materialized, frames replayed, distinct **objects touched** (decode-time
   analogue of object fetches), peak per-level raster memory, demand
   bookkeeping.
5. **API**: `Decoder::materialize_view(idx, view)` and
   `materialize::materialize(state, View::Rect/Tile)` (state-level crop).

## Courts (`tests/phase_s.rs`, 14 tests)

| Court | Result |
|---|---|
| View geometry: full/rect/tile clipping, overhang, fully-outside → typed, zero-size → `DimensionTooLarge`, kinds | PASS |
| Parity on every frame of every stream (copy-chain, palette+sparse+RAW/rANS residuals, affine rotation, generator drift, 12 deterministic random movies): 4 random views per frame each byte-equal to the whole-frame crop; `FullFrame` view == canonical whole-frame decode | PASS |
| Decoder API: `materialize_view` == `materialize` crop for FullFrame/whole-box/Rect/Tile on every frame | PASS |
| Tile grid partitions a frame exactly (each tile == crop; tile areas sum to the canvas) | PASS |
| State-level `Rect`/`Tile` == whole-frame state crop (frame 0) | PASS |
| 1920×1080 viewport tracking (41 frames): exactly one level painted per decode — 56,400 samples (260×140 bg + 200×100 sprite) vs ≥ 2,073,600/level whole-frame; objects touched == 1 (a 600×900 decoration never intersects the viewport); 2.72% of the whole-frame lane | PASS |
| COPY_RECT chain: 20×56 viewport inside a panned band — cross-frame demand propagates backward, stays near the region (102,544 samples over 13 frames), byte-exact | PASS |
| Audit-scope hostile: in-view index poison → `OutOfBounds` (identical to whole-frame); out-of-view poison → clean samples (documented sampling contract); unsorted residual → `NonCanonicalEncoding` for any view of that frame; out-of-range idx/view/zero-size typed | PASS |
| Stats consistency: residual writes counted only inside the region; painted == base+copy+residual; frames replayed/levels/peak reported | PASS |
| Pathological 40-interval × 6 overlapping-copy stream: exact and bounded (span budget saturation) | PASS |
| Full-canvas-box views route through the canonical step machinery (byte-identical with canvas ops present) | PASS |

## Malformed-input court (partial domain)

Fully outside / zero-size / out-of-range-frame views; index poison inside vs
outside the region; unsorted residuals; overlapping-copy planning pressure —
all typed, never a panic or unbounded memory.

## Empirical court / evidence

`examples/partial_proof.rs` (release), 1920×1080 Gray8 41-frame sprite track +
copy-chain + tile-grid courts (numbers from
`evidence/campaigns/phase-s-partial-1788545992/partial-proof.log`):

```
whole-frame sequential decode (41 frames): 14.5 ms  (writes ≥ 85,017,600 samples = 81 MiB)
random access frame 40: whole 13.5 ms; viewport (260x140) 0.068 ms (198×)
  viewport painted 56,400 samples (whole-frame lower bound 85,017,600)
  peak working raster 36,400 samples (1.755% of the 2,073,600-sample frame)
  objects touched 1 (only the tracked sprite)
all-41 viewport frames: 3.4 ms total, 2,312,400 samples (2.720% of the whole-frame lane)
scroll-chain viewport lane: 102,544 samples total; every view byte-equal to the crop
tile grid 24x24 of frame 8 (96x96): 16 tiles partition the frame exactly
partial proof: OK
```

Evidence: `evidence/campaigns/phase-s-partial-1788545992/`
(`partial-proof.log`, `environment.json`, `summary.json`).

## Recorded, not hidden

* Views are requests, not wire syntax: no format-v1 constant changed; every
  existing stream and golden untouched.
* Partial decode replays state transitions for levels `0..=idx` exactly like
  whole-frame decode; the measured win is the raster lane (2.72% on the 1080p
  court; 198× random-access latency on the release example). Levels with empty
  demand are skipped entirely.
* The demand plan is an over-approximation (union over all copy destinations,
  including shadowed ones; residuals add no demand) — always exact-output-safe;
  span bookkeeping saturates to the whole canvas past the budget.
* Documented audit boundary: a sub-frame view validates content contributing
  to its region; whole-frame decode (and `View::FullFrame`) remains the
  canonical audit path, byte- and error-identical by construction.
* Synthetic authored courts only: decode-work numbers, no natural-video claim
  (§72). §66 direct scanout stays a hypothesis; correct partial materialization
  is its prerequisite and is what this phase proves and measures.
* Residual decode work on a demanded level is whole-container (validation
  parity); residual-heavy frames therefore see less relative saving — measured
  honestly by `residual_samples_written` and the painted totals.

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0 warnings) ·
`cargo test` (249, dev) · `cargo test --all-features` (251) ·
`cargo test --release --all-features` (251) · hostile courts · Phase-S court ·
evidence receipt (`evidence/campaigns/phase-s-partial-…/`) · docs updated
(`materialization.md`, `empirical-status.md`, `CONFORMANCE.md`,
`PROJECT_STATE.md`, README).

## Verdict

```
SEALED
```
