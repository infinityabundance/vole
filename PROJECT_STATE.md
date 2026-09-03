# PROJECT_STATE

**Current head:** `7a14c60` (Phase C sealed)
**Current phase:** C (sparse mutation) — sealed. Next: Phase D (2D COPY_RECT / MOVE_RECT).
**Format version:** v1 (`.vole`), universe v1, limit-profile 1.

## Completed (measured, courted, sealed)

*Phase A core within one native-Rust crate (no external codec/ML/network):
manual `.vole` v1 writer/parser; Gray8 canvas; object table (fill/raw raster
immutable objects); instance state; single checkpoint; exact restore & replay;
`interval → materialize → FullFrame`; absolute `SetPosition`/`CreateInstance`
transitions; BLAKE3 integrity trailer; typed `Limits`; hostile-input tests.

Phase B: exact content identity (BLAKE3 over canonical object record), a
content→id reuse registry, and the unchanged-state lane; static court confirms
10 001 identical views at ~13.0 B/frame (raw would be 20.7 GB).

Phase C: persistent sparse overlay + strict-sorted SPARSE patch; blink court
materializes 65 exact frames from a 1 820 B stream (raw 14.98 MB).*

Courts: moving-rect 1920×1080, 101 frames — VOLE stores 2,692 B (state +
transitions); raw full-frame sequence would be 209,433,600 B. Materialized
frames verified byte-exact against an independent painter; header-semantic and
integrity gates asserted typed. Court/Hostile tests, `cargo fmt/check/clippy
-D warnings/test` all pass.

## In progress

Phase D — 2D copy/move (COPY_RECT/MOVE_RECT) with explicit overlap/clipping
and bounded dependency semantics; then the native entropy floor (rANS).

## Correct, decided, waiting

## Explicit ordering for the remaining ladder (each gate-passed before next)

Phase D 2D COPY_RECT/MOVE_RECT → Phase F native rANS entropy floor →
Phase G exhaustive inverse-proceduralization court → Phase H fixed-heuristic vs
DSFB → Phase I parametric trajectories → Phase J palettes → Phase K variable
regions → Phase L affine/global → Phase M transform residual → Phase N
procedural generators → Phase O representation re-optimization → Phase P
optional EntropyFS persistence → Phase Q native procedural ingest API → Phase R
procedural transport → Phase S partial materialization → Phase T archive profile
→ Phase U perceptual profile (last).

(Phase-Plan numbering above is the master-brief lettering; the *ablation*
letters P0–P16 of §61 fold into these gates with explicit mechanisms, e.g. P0
RAW = our v1 RAW/object base, P4 unchanged = Phase B lane, P5 sparse = Phase C,
P6 COPY_RECT = Phase D.)

Concrete **next** step from this commit: Phase D — add a deterministic
COPY_RECT/MOVE_RECT state transition referencing the *previous materialized
frame* as an explicit (bounded depth) copy source, define canonical overlap
semantics (snapshot source to a temporary to avoid aliasing), enforce clipping
and dependency-depth limits, court a terminal/editor-scroll synthetic, add the
noise negative control, then seal Phase D with a campaign + receipt exactly as
Phases A–C were.

## Failures / uncertainty

None recorded yet (Phase A has no negative-result court beyond noise controls,
which are Phase-C+). Open questions are listed on each future phase entry
rather than asserted here.

## Frozen (format decisions)

v1 `.vole` grammar (docs/format-v1.md), materializer painter, time model,
limits profile 1, integrity trailer.
