# PROJECT_STATE

**Current head:** Phase B sealed (next commit)
**Current phase:** B (persistent object identity) — sealed; next C.
**Format version:** v1 (`.vole`), universe v1, limit-profile 1.

## Completed (measured, courted, sealed)

*Phase A core within one native-Rust crate (no external codec/ML/network):
manual `.vole` v1 writer/parser; Gray8 canvas; object table (fill/raw raster
immutable objects); instance state; single checkpoint; exact restore & replay;
`interval → materialize → FullFrame`; absolute `SetPosition`/`CreateInstance`
transitions; BLAKE3 integrity trailer; typed `Limits`; hostile-input tests.

Phase B: exact content identity (BLAKE3 over canonical object record), a
content→id reuse registry, and the unchanged-state lane; static court confirms
10 001 identical views at ~13.0 B/frame (raw would be 20.7 GB).*

Courts: moving-rect 1920×1080, 101 frames — VOLE stores 2,692 B (state +
transitions); raw full-frame sequence would be 209,433,600 B. Materialized
frames verified byte-exact against an independent painter; header-semantic and
integrity gates asserted typed. Court/Hostile tests, `cargo fmt/check/clippy
-D warnings/test` all pass.

## In progress

Phase C — sparse mutation & 2D copy/move once gated (order preserved).
Persistent content identity next gains formal reuse across streams (Phase P,
EntropyFS).

## Correct, decided, waiting

Explicit pending ordering for the later ladder (started only after each prior
gate passes): sparse mutation → COPY_RECT/MOVE_RECT → native rANS entropy
floor → exhaustive inverse-proceduralization court → fixed-heuristic vs DSFB
→ parametric trajectories → palettes → variable regions → affine/global →
transform residual → procedural generators → representation re-optimization →
optional EntropyFS persistence → native procedural ingest API → procedural
transport → partial materialization → archive profile → perceptual profile
(last).

## Failures / uncertainty

None recorded yet (Phase A has no negative-result court beyond noise controls,
which are Phase-C+). Open questions are listed on each future phase entry
rather than asserted here.

## Frozen (format decisions)

v1 `.vole` grammar (docs/format-v1.md), materializer painter, time model,
limits profile 1, integrity trailer.
