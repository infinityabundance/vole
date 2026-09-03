# PROJECT_STATE

**Current head:** Phase G sealed (see git log)
**Current phase:** G (exhaustive inverse proceduralization) — SEALED. Next: Phase H (fixed-heuristic vs DSFB-governed search).
**Phase order:** master brief §64, verified against the prior-art §29 lettering:
A → B → C → D → E → F → G → H → I → J → K → L → M → N → O → P → Q → R → S → T → U.
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
materializes 65 exact frames from a 1 820 B stream (raw 14.98 MB).

Phase D: a COPY_RECT/MOVE_RECT frame-referencing op at dependency depth 1 with
canonical snapshot-copy + clipping. Oracle-exact wrap-scroll court; hostile
bounds; noise negative control.

Phase E: persistent integer translation — per-instance `(vx, vy)` applied once
per `AdvanceTranslations` (`position(t+1) = position(t) + (vx, vy)`), wire tags
0x26/0x27, cumulative work budget. 101 exact frames in 1 505 B vs 2 692 B
per-frame `SetPosition` baseline; camera-like translation; static + noise
controls.

Phase F: native deterministic order-0 byte rANS coder owned in-crate
(`src/rans.rs`; scale_bits=14, STATE_L=2^23), largest-remainder model
normalization (512 B inline model), RAW-fallback accounting. Byte parity +
bidirectional cross-decode vs the `ryg-rans-rs` oracle; hostile courts; skew
59×, uniform→RAW.

Phase G (this head): **exhaustive inverse proceduralization** (`src/inverse.rs`)
— a raster→VOLE encoder that per frame exhaustively evaluates RAW · FILL ·
UNCHANGED · EXACT_OBJECT_REF · SPARSE · COPY_RECT · TRANSLATION ·
RANS_RESIDUAL (plus copy+residual and prev-diff composites), byte-validates
every candidate through the normative materializer path, and emits the
complete-cost winner; streams are always decode-verified end-to-end before
return. New wire ops 0x28/0x29 (content-replacement clears) and 0x2a (one-shot
per-frame residual block); new bounds `max_overlay_points`/`max_residual_bytes`
and enforcement of `max_stream_bytes`/`max_checkpoint_distance`; `vole encode`
CLI; per-frame decision records with regret-0 oracle consistency; whole-frame
granularity with documented row-hash-prefiltered large-canvas scroll search.
Courts + evidence + receipt in `tests/phase_g.rs`,
`tests/malformed.rs`, `examples/inverse_proof.rs`,
`evidence/campaigns/phase-g-inverse-1788461583/`, `docs/phase-g.md`.

## In progress

Phase H — fixed-heuristic vs DSFB-governed search over the Phase-G candidate
universe (deterministic sentinels; no stochastic bandits; regret courts
`N_dsfb < N_exhaustive` while `J_dsfb ≈ J_exhaustive`; local rebase). The
decision-record infrastructure from G is the shared court surface.

## Correct, decided, waiting

## Explicit ordering for the remaining ladder (each gate-passed before next)

Phase H DSFB-guided search → Phase I parametric trajectories (bounded
fixed-point: constant/linear/acceleration/piecewise; collapse of repeated
SET_POSITION) → Phase J palettes → Phase K variable regions → Phase L
affine/global → Phase M transform residual → Phase N procedural generators →
Phase O representation re-optimization (`vole optimize`) → Phase P optional
EntropyFS persistence → Phase Q native procedural ingest API → Phase R
procedural transport → Phase S partial materialization → Phase T archive
profile → Phase U perceptual profile (last).

## Failures / uncertainty

No mechanism rejected yet. Measured temporal gaps recorded (not hidden):
per-frame `SetPosition` (26 B) vs authored velocity (14 B from frame 2) on
pans; stable residuals pay one-shot per frame until a re-optimization pass
promotes them to persistent state/content (Phase O); "static after canvas-op"
frames repeat at 38 B until a RAW-capture rebase (Phase O checkpoint/capture
placement). Phase G's whole-frame granularity pays a full-raster declaration
at raster-origin frame 0 / appearance frames (region extraction is Phase K;
native ingest is Phase Q).

## Frozen (format decisions)

v1 `.vole` grammar (docs/format-v1.md), materializer painter semantics, time
model, limits profile 1, integrity trailer, rANS normative constants
(docs/phase-f.md). v1 continues to *extend* per sealed phase (tags 0x21–0x2a)
with old streams re-parsed unchanged.
