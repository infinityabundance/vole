# PROJECT_STATE

**Current head:** Phase O sealed (see git log)
**Current phase:** O (representation re-optimization) — SEALED. Next: Phase P (EntropyFS persistence).
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

Phase H: three search strategies over the same candidate universe
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

Phase I: bounded **parametric trajectories** as first-class
procedural state (`src/trajectory.rs`, tags 0x2b/0x2c): finite programs of
Linear (constant velocity / exact hold) and Accel (constant acceleration,
exact discrete semantics `pos += v; v += a`, closed form
`Δ(t) = t·v0 + a·t·(t−1)/2`) segments, stepped once per
`AdvanceTrajectories`, deactivating when exhausted, exclusive with translation
state. **Trajectory collapse** (§43, `src/collapse.rs`): repeated per-frame
`SetPosition` runs become one `SetTrajectory` + per-frame advances only when
the rebuilt stream decodes byte-identically (normative proof) and is strictly
smaller. New limits `max_trajectory_segments`/`max_trajectory_work`.
Measured: accel flagship 686 B vs 1 132 B (`SetPosition`) / 1 172 B
(`SetVelocity`) baselines on 41 identical frames (raw 85 017 600 B); piecewise
holds exact; raster linear pan interval transitions 1 014 → 572 B, raster
accel 182 → 132 B, all decode-exact; noise and random walks are collapse
fixpoints; active zero-velocity holds measured at 14.5 B/frame — statics stay
in the 13 B/frame unchanged lane. Receipt + evidence: `docs/phase-i.md`,
`evidence/campaigns/phase-i-trajectory-1788466084/`.

Phase J (this head): **palette state** — palette-index objects (0x05,
immutable one-byte index planes with exact content identity), a mutable
bounded palette table in `G_t` (0x06 pre-checkpoint records +
`SetPalette` 0x2d / `PatchPalette` 0x2e), and per-instance palette bindings
(0x08 checkpoint variant + `BindPalette` 0x2f). Materialization resolves
`indices ∘ entries(bound palette)` with typed errors (`UnknownPalette`,
`OutOfBounds`); `limits.max_palette_entries` (256) / `max_palettes` (4096).
Measured on 1920×1080 UI content: accent cycling **24 B/interval** vs the
204 773 B/interval palette-less sparse floor (8 532×) and 2 073 600 B RAW;
whole-palette rotation **28 B/interval** while every pixel changes; the §55
flattening-tax court measures authored-palette intervals at 288 B vs the
raster-origin inverse encode's 50 016 B (174×) on identical visual frames;
static palette content is free at rest (13 B/frame unchanged lane). Receipt +
evidence: `docs/phase-j.md`, `evidence/campaigns/phase-j-palette-1788469733/`.

Phase K (this head): **variable regions** in the raster-origin encoder
(`src/inverse.rs`, encoder-side only — no wire-format changes). The new
REGIONS family partitions the per-frame diff into tiles of a granularity
(64 → 32 → 16 → 8), declares each diff-bearing tile's rectangular bounding
box as an immutable object holding the target's own sub-rectangle, and paints
it above the base with a fresh instance; repeated region content is reused by
exact BLAKE3 identity with zero declaration bytes. Documented gates: diff ≤ ¼
canvas, ≤ 256 rectangles, no overlay-shadowed samples; DSFB governs the
family (Full ladder / Probe / Off). Measured: 1920×1080 localized-change
Phase K: 40 region frames with **zero whole-frame rebases** after frame 0
(26× vs raw); alternating-glyph region reuse at the 30 B floor across 35
frames; DSFB byte-identical to the oracle at N = 0.378×; fixed-heuristic
probe-granularity blindness measured at J 1.036; noise stays RAW (diff gate).
Receipt + evidence: `docs/phase-k.md`,
`evidence/campaigns/phase-k-regions-…/`.

Phase L (this head): **bounded fixed-point affine / global state** —
pan/zoom/rotation/camera-like transforms are procedural *state*, not
rasters or codec-local block motion (`src/affine.rs`, tag 0x30): a
`SetAffine` transition attaches a canonical Q8 placement
`(su, sv) = ((a·x+b·y+c) >> 8, (d·x+e·y+f) >> 8)` (signed `>> 8` = floor;
no floating point anywhere), integer maps are exact in Q8 and general
rotation/zoom/pan are Q8 approximations whose exactness gap is closed by the
residual algebra. Identity deactivates; affine/velocity/trajectory state on
one instance are mutually exclusive; affines die with their instances; new
limit `max_affine_work` caps per-materialization affine sample work (typed
`MaterializationBudgetExceeded` at materialization). Affine painting keeps
object-kind semantics (fill / raster / bound-palette-index lookup).
Measured: 320×180 rotating-tile flagship — 81 frames as one object + one
instance + one `SetAffine`/interval, **42 B/interval** (618× vs raw),
byte-exact vs an independent incremental painter; the flattening tax of
re-encoding the same rotation through the raster encoder measured at 7×;
Q8 30°-rotation approximation + one persistent sparse correction reproduces
a float-rendered target byte-for-byte (58 of 4 096 tile px — the Q8
camera-map gap is a small edge set); affine over palette-index and fill
objects exact; hostile wire + work-budget courts typed. Receipt + evidence:
`docs/phase-l.md`, `evidence/campaigns/phase-l-affine-…/`.

Phase M (this head): **deterministic integer transform residual floor** —
when procedural state cannot explain a dense smooth residual, VOLE behaves
like a conventional coder within the lossless domain (`src/transform.rs`):
the signed residual field is partitioned into aligned 4×4 blocks and
decorrelated by a reversible integer lifting DCT (Q8 lifting rotations for
the −π/4 and −π/8 stages; no floating point, no quantization;
`inverse(forward(x)) == x` for every integer block). Wire: residual block
**kind 2** under tag 0x2a — skip mask + DC/AC zigzag coefficient streams in
standard RAW/rANS containers; decoder inverse-transforms and **adds** the
reconstruction (outside `0..=255` is `OutOfBounds`; unknown transform ids
fail closed). The encoder's TRANSFORM_RESIDUAL family joins the exhaustive
court (gate: `9k ≥ mask+64`) and the fixed-heuristic/DSFB probe (dense
only). Measured: 1920×1080 brightness-drift flagship — **69 848 B/interval**
vs 2 073 645 B RAW reset (29.7×) and 10 467 936 B point residual (150×),
winners `raw×1 transform_residual×8`, all frames byte-exact; same-delta
480×270 transform block 5 906 B vs 549 268 B point container (93×); noise
stays RAW; tiny diffs never evaluate the family; hostile kind-2 courts
typed at parse and materialization. Accounting fix (recorded): inline rANS
models are a `model_bytes` sub-bucket excluded from `residual_bytes`, so the
ten buckets sum exactly. Receipt + evidence: `docs/phase-m.md`,
`evidence/campaigns/phase-m-transform-…/`.

Phase N (this head): **bounded procedural generators** — an immutable object
may carry a bounded integer *content program* whose samples are computed at
materialization instead of stored (`src/generator.rs`, object tag 0x07):
gradient `(base + sx·x + sy·y) mod 256`, checker (cell parity), periodic
sawtooth with explicit period, and seeded noise (splitmix64 position hash).
Integer only; work == painted box; content identity == BLAKE3 over
`0x07 w h program` so generator content reuses exactly. The inverse encoder
fits a deterministic bounded set of content-derived programs (gradient /
checker cell lattice / periodic period lattice; noise is never fitted — seed
discovery is unbounded search), spot-checks on O(w+h) and validates by
rendering the normative field; an inexact fit is admissible only as
`generator_residual` with its exact correction counted (≥ 15/16 gate).
Measured: 1920×1080 drifting-gradient flagship — 12 frames in **706 B**
(35 245× vs raw), winners `generator×12`, all frames byte-exact; authored
full-HD frames 98–105 B (≈ 20 000× vs the 2 073 600 B raster); noise and
wrong-seed controls stay RAW (unknowable seed is never discovered — the
§21/§63 negative control); generator tiles compose with motion/affine;
hostile wire + identity + accounting courts typed. Recorded re-measurement:
pure wrap-ramp content (Phase M's transform-floor exhibit) is now explained
procedurally. Receipt + evidence: `docs/phase-n.md`,
`evidence/campaigns/phase-n-generators-…/`.

Phase O (this head): **equivalence-preserving representation
re-optimization** (`vole optimize`, `src/optimize.rs`, §44) — a decoded
stream is searched by a bounded rewrite set and the first strictly-smaller,
decode-identical candidate is applied, iterating to a fixpoint: **velocity
collapse** (constant-delta `SetPosition` runs → one `SetVelocity` +
per-frame `AdvanceTranslations`, 13+len vs 13·len); **trajectory collapse**
(Phase I pass reused); **residual promotion** (a run of identical one-shot
point residuals → one persistent sparse overlay + the unchanged lane — the
recorded stable-residual gap is closed); **generator substitution** (raster
objects whose samples are exactly a bounded program are re-declared as
generators); **duplicate merge** (byte-identical objects share one
declaration, references remapped). Every acceptance is proven by full
normative decode (`M(D0)==M(D1)`) and requires strict shrink (`J(D1)<J(D0)`);
never grows; palette streams preserved verbatim. Measured: 100-frame linear
run at 1920×1080 22 691 → 21 504 B (velocity; 13 B/run better than
trajectory); stable 40-point residual × 30 frames 36 277 → 856 B (97.6%);
full-canvas raster gradient decl 24 667 → 101 B; eight identical tiles
33 062 → 213 B; inverse-encoder and noise outputs are zero-savings fixpoints
(honest negatives); the Phase-A proof stream (2 692 B) optimizes to 1 505 B
via the CLI — exactly the Phase-E velocity baseline. Receipt + evidence:
`docs/phase-o.md`, `evidence/campaigns/phase-o-optimize-…/`.

## In progress

(none — Phase O sealed; Phase P is next)

## Correct, decided, waiting

## Explicit ordering for the remaining ladder (each gate-passed before next)

Phase P optional
EntropyFS persistence → Phase Q native procedural ingest API → Phase R
procedural transport → Phase S partial materialization → Phase T archive
profile → Phase U perceptual profile (last).

## Failures / uncertainty

No mechanism rejected yet. Measured, recorded gaps (not hidden): frame 0 and
content-wide rebases still pay one whole-canvas declaration (regions serve
localized change; native ingest is Phase Q); region *instances persist* —
long-horizon instance retirement and encoder-side region+residual composite
discovery (dense region with sparse dust) are open surface (later phases);
the raster-origin encoder has no palette/trajectory/affine *discovery* family
yet — the measured flattening taxes are 174× (palette), 7× (rotation/affine
on the 160×160 court), and trajectory/velocity collapse is a post-pass (Phase
O `vole optimize`); the fixed-heuristic region probe is blind to granularity
(measured J 1.036 on the reuse court); DSFB can miss a cheaper family with no
slew/regime signal for at most one small interval (Phase H receipt);
trajectory descriptors only pay from runs of ≥ 3 frames (Phase I receipt);
active zero-velocity trajectories cost more than the unchanged lane (Phase
I); palettes must be set before they are bound, and index validity is
enforced at materialization (Phase J); an affine placement scans the whole
canvas, so many concurrent affine instances are capped by
`max_affine_work` (8 full canvases) rather than per-instance raster cost
(Phase M); the transform floor is 4×4-only with one order-0 byte model per
DC/AC container — block-size extension and per-coefficient-position contexts
are open surface (Phase M); the accounting fix for inline rANS models
changes the `model_bytes`/`residual_bytes` split of streams carrying rANS
residuals (bucket totals are unchanged; Phase M receipt); the fixed-heuristic
scroll-by-7 court was re-measured after Phase M: the transform floor absorbs
the dense scroll frames at ~950 B, so the probe-blind stream is now measured
by cost (8.1×, was 11.5× by rebase pre-M; copy blindness unchanged) (Phase
M receipt); generator discovery inside the raster encoder is whole-frame only
in v1 — generic/rectangular-region generator fits are Phase-Q surface (Phase
N; Phase O added substitution on *declared* raster objects only); pure
ramps are now explained procedurally, so the Phase-M full-range-ramp court
was re-measured to `generator` winners (recorded in the Phase M/N receipts);
seeded noise is author-only and never discovered by the inverse encoder —
the measured RAW flattening for unknowable noise is structural, not a
compression claim (Phase N receipt).

Closed by Phase O: stable residuals previously paid one-shot per frame for
the life of a repeated difference — residual promotion now converts a run of
identical one-shot blocks into one persistent overlay + the unchanged lane
(measured 36 277 → 856 B on the 30-frame court, Phase O receipt);
copy-decomposition, checkpoint placement, and per-frame entropy-model
retuning were courted and measured as zero-savings fixpoints on current
encoder output (recorded, Phase O receipt).

## Frozen (format decisions)

v1 `.vole` grammar (docs/format-v1.md), materializer painter semantics
(including palette-index resolution, the Phase-L affine source map, the
Phase-M additive transform-residual algebra, and the Phase-N generator
programs), time model (explicit advances
only — never implicit stepping), limits
profile 1, integrity trailer, rANS normative constants (docs/phase-f.md),
transform constants (docs/phase-m.md: kind-2 residual block grammar, lifting
multipliers, transform id 0), generator constants (docs/phase-n.md:
generator kinds, parameter domains, program wire bytes). v1
continues to *extend* per sealed phase (tags 0x21–0x30, residual block kinds
0–2, object tag 0x07) with old streams re-parsed unchanged.
