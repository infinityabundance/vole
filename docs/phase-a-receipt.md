# Phase A Receipt — bounded procedural core

*Sealed 2026 (head recorded in evidence environment.json).*

## Implemented

Native-Rust single crate (`vole`, one workspace — none). Gray8 pixel model.
Canonical, manual, little-endian `.vole` v1 writer+parser. Universe v1 binding;
limit-profile 1 (`Limits`). Object table with immutable fill/raster objects.
Instance state (mutable by transition). Single checkpoint; exact restore and
`interval → FullFrame` materialization. Absolute `SetPosition`, `CreateInstance`.
BLAKE3 integrity trailer. Typed error surface. `#![forbid(unsafe_code)]`
throughout.

Modules: `src/{checked,error,limits,universe,time,pixel,object,state,transition,
materialize,format,integr,encoder,decoder,demo}.rs`. Public docs: README, SPEC,
CONFORMANCE, SECURITY, docs/architecture, docs/{procedural-state-graph,
materialization, format-v1, transitions, residuals, information-accounting,
entropyfs, dsfb-search, transport, empirical-method, empirical-status}, ADR 0001–0005.

## Normative format changes

Format v1 (universe v1, profile 1) grammar fixed and documented in
`docs/format-v1.md`. No later phase may reinterpret v1 golden streams.

## Normative semantics changes

Deterministic painter (background fill then ordered clipped instance blits) as
`SPEC.md`/`docs/materialization.md`; absolute integer `(x,y)` i64 domain with a
2^24 wire bound.

## Tests

`cargo test --all-features` passes: unit zero; conformance `tests/court.rs` 4;
hostile `tests/malformed.rs` 11. Gate lane:
`cargo fmt --check`, `cargo check --all-targets`,
`cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
⇒ all pass.

## Malformed-input court

covered: truncated stream, wrong magic, tampered universe, non-zero feature
bits, oversized dimensions, integrity tampering, body bit flips, undeclared
object reference, duplicate ids, checkpoint-less/empty, trailing garbage —
all terminate typed, never panic.

## Empirical court

`vole demo moving-rect`: 1920×1080 Gray8 over 100 intervals, stored as one
200×100 object + one instance + one checkpoint + 100 `SetPosition`
transitions. Stream = **2,692 B**; materialized exact **101 frames** (raw
sequence would be **209,433,600 B**; single frame 2,073,600 B). Reconstructed
frames, including frame 0 and frame 100, are byte-identical to an independent
reference painter and to independently re-derived SHA-256 references.

## Measured results

See `evidence/campaigns/phase-a-…/summary.json` (only reproducible evidence).

## Negative controls

Phase A intentionally contains no mechanism whose private win is being
protected; the noise/random raster controls belong to Phase C+, and the
no-raster repetition proof (stream ≪ raw) is the Phase-A negative control
against "this is just a better frame codec": the stored bytes scale with
declared state cost, not 101 whole rasters.

## Mechanisms adopted

Persistent immutable object + mutable instance; single checkpoint + forward
replay; interval transitions (`SET_POSITION`/`CREATE_INSTANCE`); integrity
trailer; manual wire format; deterministic painter; typed Limits.

## Mechanisms rejected

None this phase (no hypothesis failed that belongs to Phase A's scope). Later
phase rejections will be recorded here without erasure.

## Open uncertainty

* rANS-free file densities cannot yet be compared on raster-origin content
  (Phase D+).
* The default background-fill painter has not been tested against a broad
  content corpus (later phases).
* One-crate structure is untested at large scale; reviewed at Phase-G+.

## Verdict

```
SEALED
```
