# PROJECT_STATE

**Current head:** Phase F sealed (see git log)
**Current phase:** F (native rANS entropy floor) — SEALED. Next: Phase G (exhaustive inverse-proceduralization court).
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

Phases A–F are SEALED.

Phase F: native deterministic order-0 byte rANS coder owned in-crate
(`src/rans.rs`; scale_bits=14, STATE_L=2^23, per-symbol x_max renorm, LIFO
decode), deterministic largest-remainder model normalization (512 B inline
model), RAW-fallback accounting (RANS iff model+encoded < raw). Byte parity +
bidirectional cross-decode vs the `ryg-rans-rs` oracle over an adversarial
corpus; hostile length/truncation/corruption courts; measured skew 59×,
single-symbol 499×, uniform→RAW. Evidence + receipt
(`evidence/campaigns/phase-f-…` / `docs/phase-f.md`).

Phase D: a COPY_RECT/MOVE_RECT frame-referencing op at dependency depth 1 with
canonical snapshot-copy + clipping. Oracle-exact 96×96 wrap-scroll court (two
rects per interval; none of the intermediate frames is reproducible from the
immutable painter State); hostile bounds in parser and encoder; noise negative
control (prior-frame-uncorrelated content cannot be COPY-encoded — encoders
must fall back, lossless authority holds). Evidence + receipt in
`evidence/campaigns/phase-d-…` / `docs/phase-d.md`.

Courts: moving-rect 1920×1080, 101 frames — VOLE stores 2,692 B (state +
transitions); raw full-frame sequence would be 209,433,600 B. Materialized
frames verified byte-exact against an independent painter; header-semantic and
integrity gates asserted typed. Court/Hostile tests, `cargo fmt/check/clippy
-D warnings/test` all pass.

## In progress

Phase G — exhaustive inverse-proceduralization: a raster encoder that tests
RAW, FILL, UNCHANGED, EXACT_REF, SPARSE, COPY_RECT, TRANSLATION and
RANS_RESIDUAL candidates, materializing each, building the exact residual,
and picking the complete-cost winner (oracle); the rANS floor from Phase F is
the RANS_RESIDUAL payload primitive.

## Correct, decided, waiting

## Explicit ordering for the remaining ladder (each gate-passed before next)

Phase F native rANS entropy floor →
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

Concrete **next** step from this commit: Phase G — build the exhaustive raster
inverse-proceduralizer: for each region/frame evaluate candidate families
(RAW, FILL, UNCHANGED, EXACT_REF/object reuse, SPARSE, COPY_RECT, integer
TRANSLATION, RANS_RESIDUAL), validate every candidate through the normative
materializer with an exact residual, and select by complete persisted-byte
cost under the typed accounting model; seal with oracle-regret campaign and
receipt exactly as Phases A–F were.

## Failures / uncertainty

None recorded yet (Phase A has no negative-result court beyond noise controls,
which are Phase-C+). Open questions are listed on each future phase entry
rather than asserted here.

## Frozen (format decisions)

v1 `.vole` grammar (docs/format-v1.md), materializer painter, time model,
limits profile 1, integrity trailer.
