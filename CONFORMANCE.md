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

## Goldens

Sealed format v1 golden streams must decode forever under v1 semantics:
(old) stream hash + expected reconstruction hashes are recorded in evidence.
Any future re-encode or version bump must not reinterpret v1 files.

See `docs/architecture.md`, `PROJECT_STATE.md`, `evidence/campaigns/`.
