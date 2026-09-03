# PROJECT_STATE

**Current head:** Phase H sealed (see git log)
**Current phase:** H (fixed-heuristic vs DSFB-guided search) — SEALED. Next: Phase I (parametric trajectories).
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

Phase D: COPY_RECT/MOVE_RECT frame-referencing ops at dependency depth 1 with
canonical snapshot-copy + clipping; oracle-exact wrap-scroll court; hostile
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

Phase G: exhaustive inverse proceduralization (`src/inverse.rs`) — a
raster→VOLE encoder that per frame exhaustively evaluates RAW · FILL ·
UNCHANGED · EXACT_OBJECT_REF · SPARSE · COPY_RECT · TRANSLATION ·
RANS_RESIDUAL (plus copy+residual and prev-diff composites), byte-validates
every candidate through the normative materializer path, and emits the
complete-cost winner; streams always decode-verified end-to-end. Wire ops
0x28/0x29 (content-replacement clears) and 0x2a (one-shot per-frame residual
block); bounds `max_overlay_points`/`max_residual_bytes` and enforcement of
`max_stream_bytes`/`max_checkpoint_distance`; `vole encode` CLI; per-frame
decision records (regret-0 oracle consistency). Receipt + evidence:
`docs/phase-g.md`, `evidence/campaigns/phase-g-inverse-1788461583/`.

Phase H (this head): three search strategies over the same candidate universe
— **Exhaustive** (oracle, still the default) · **FixedHeuristic** (constant
plan) · **DsfbGuided** (non-normative deterministic trust model in
`src/dsfb.rs`: recent-winner active set, per-family `φ`, drift `ω`, regime
flag `α`, full broaden on regime/slew, deterministic rotating sweep every 6th
frame; no stochastic bandits). Decision records now include `search_work` and
`dsfb_diag`. Measured: `N_dsfb ≤ 0.18× N_exhaustive` with byte-identical
`J_dsfb == J_exhaustive` on steady courts; across four regime changes
(static→wrap→noise→pan) `J_dsfb = 1.055×` oracle with 0–1 frame recovery
latency and 36 → 15 measured rebase events vs the fixed heuristic; the fixed
heuristic's constant probe set misses scroll-by-7 entirely (`J = 11.5×`).
Receipt + evidence: `docs/phase-h.md`,
`evidence/campaigns/phase-h-dsfb-1788464563/`.

## In progress

Phase I — bounded parametric trajectories (constant/linear/acceleration/
piecewise-linear integer/fixed-point): collapse repeated `SET_POSITION`
runs and measured steady translations into a single trajectory descriptor
only when materialization stays exact and complete cost falls. The Phase-H
decision records and the DSFB governor are the natural court + search surface.

## Correct, decided, waiting

## Explicit ordering for the remaining ladder (each gate-passed before next)

Phase I parametric trajectories → Phase J palettes → Phase K variable regions →
Phase L affine/global → Phase M transform residual → Phase N procedural
generators → Phase O representation re-optimization (`vole optimize`) → Phase
P optional EntropyFS persistence → Phase Q native procedural ingest API →
Phase R procedural transport → Phase S partial materialization → Phase T
archive profile → Phase U perceptual profile (last).

## Failures / uncertainty

No mechanism rejected yet. Measured, recorded gaps (not hidden): whole-frame
granularity pays full-raster declarations at raster-origin frame 0 / rebase
frames (region extraction is Phase K, native ingest Phase Q); per-frame
`SetPosition` (26 B) vs authored velocity (14 B from frame 2) on pans
(trajectory collapse is Phase I); stable residuals pay one-shot per frame
until a re-optimization pass promotes them (Phase O); "static after canvas-op"
frames repeat at 38 B until a RAW-capture rebase (Phase O); DSFB can miss a
cheaper family with no slew/regime signal for at most one small interval
before the rotating sweep or a following signal recovers it (Phase H receipt).

## Frozen (format decisions)

v1 `.vole` grammar (docs/format-v1.md), materializer painter semantics, time
model, limits profile 1, integrity trailer, rANS normative constants
(docs/phase-f.md). v1 continues to *extend* per sealed phase (tags 0x21–0x2a)
with old streams re-parsed unchanged.
