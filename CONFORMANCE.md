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

## Goldens

Sealed format v1 golden streams must decode forever under v1 semantics:
(old) stream hash + expected reconstruction hashes are recorded in evidence.
Any future re-encode or version bump must not reinterpret v1 files.

See `docs/architecture.md`, `PROJECT_STATE.md`, `evidence/campaigns/`.
