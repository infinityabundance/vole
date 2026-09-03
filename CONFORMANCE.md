# Conformance

## Statement

A conforming `.vole` decoder, given a canonical v1 stream and default
limit-profile 1, produces the exact sequence of full Gray8 frames specified by
`SPEC.md`, `docs/format-v1.md`, `docs/materialization.md`, and
`docs/transitions.md`. On hostile input it returns a typed `VoleError` and
never panics/hangs/OOMs.

## Independent oracle

The Phase-A conformance court does **not** merely decode against the encoder's
own expectations. `tests/court.rs` compares the materialized frames against an
**independent reference painter** (`src/demo.rs::reference_painter`) that
shares no blit code, so a bug in the shared `Canvas::blit` cannot pass the
court. Boundary frames are additionally hashed (SHA-256) against an
independently re-derived reference in `proof/`.

## Court status (Phase A)

| Check | Result |
|---|---|
| materialize == independent painter, all 101 frames | PASS |
| first/last frame sample checkpoints under motion model | PASS |
| stored stream is not raster-proportional | PASS |
| hostile-input cases all terminate typed | PASS |
| `cargo fmt`, `check`, `clippy -D warnings`, `test` | PASS |
| Phase I: trajectory materialization == independent closed-form painter, all frames | PASS |
| Phase I: closed-form simulator == normative state stepper (200 random programs) | PASS |
| Phase I: trajectory collapse decode-identical and strictly smaller | PASS |
| Phase J: palette-index materialization == independent palette painter, all frames | PASS |
| Phase J: palette ops (0x06/0x08/0x2d–0x2f) round-trip and hostile forms fail typed | PASS |
| Phase J: palette accounting buckets (state_bytes, index_object_bytes) sum to total | PASS |
| Phase K: variable-region encoder streams decode byte-identical (zero whole-frame rebases on localized change) | PASS |
| Phase K: region exact-ref reuse, DSFB byte-equality, noise RAW negative control | PASS |
| Phase L: affine materialization == independent incremental sampling painter (rotation / zoom / sub-pixel pan / random parameters) | PASS |
| Phase L: Q8 30°-rotation approximation + residual == float-rendered target byte-for-byte | PASS |
| Phase L: affine over palette-index and fill objects exact; work budget + hostile wire forms typed | PASS |
| Phase M: transform residual roundtrip exact (random + gradient blocks; unit courts) | PASS |
| Phase M: transform materialization == target byte-for-byte end-to-end (drift / wrap-ramp / textured) | PASS |
| Phase M: noise stays RAW; tiny diffs never evaluate the family; oracle min-payload invariant holds | PASS |
| Phase M: hostile kind-2 streams typed at parse (id/padding/length/truncation) and materialization (EntropyCorrupt / OutOfBounds) | PASS |
| Phase N: generator objects materialize byte-exact vs independent references (all four kinds, plain + affine + motion) | PASS |
| Phase N: pure-gradient sequences are discovered procedurally (35 245× flagship); noise and wrong-seed controls stay RAW | PASS |
| Phase N: generator+residual closure exact; hostile generator wire forms typed; identity == wire record; accounting sums | PASS |

## Goldens

Sealed format v1 golden streams must decode forever under v1 semantics:
(old) stream hash + expected reconstruction hashes are recorded in evidence.
Any future re-encode or version bump must not reinterpret v1 files.

See `docs/architecture.md`, `PROJECT_STATE.md`, `evidence/campaigns/`.
