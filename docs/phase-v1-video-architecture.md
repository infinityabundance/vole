# Phase V.1 — Universal Procedural Video Codec: Video Architecture Contract

> Status: **V.1.0 (audit + architecture contract)** — this document is the
> internally coherent contract that V.1.1+ implement against. Per the V.1
> master brief §0 and §243, **no feature coding precedes this document**.
>
> Head: commit `1308f0d` (v0.17.0, Phases A–U sealed). Working tree clean at
> audit time; the full historical suite passes (279 tests, all features,
> dev + release).
>
> This document reconciles the Phase V.1 master brief with the actual sealed
> Phase A–U implementation. Where the brief and the codebase disagree, the
> codebase state is recorded and the decision taken here is explicit
> (`DECIDED`), or explicitly deferred to a named subphase (`DEFERRED`). Claims
> stay inside the §72 discipline of the master brief; established codecs
> already own the individual predictor primitives this programme courts
> (acknowledged in §15 below — VOLE's contribution is the state-first
> architecture and its empirical integration, not those primitives in
> isolation).

---

## 1. The audit (V.1.0 / brief §0)

Executed at `1308f0d` against a clean tree.

| # | Item | Finding |
|---|---|---|
| 1 | Current head | `1308f0d` `release: bump to v0.17.0 (Phase U …)`; `origin/main` == local |
| 2 | `PROJECT_STATE.md` | Ledger complete A→U; no in-progress phase; "next" is the post-U research programme (V.1 video / V.2 audio) |
| 3 | Receipts A–U | `docs/phase-a-receipt.md`, `docs/phase-b.md` … `docs/phase-t.md`, `docs/phase-u.md` all present and consistent with git history |
| 4 | Post-U `.vole` grammar | v1 frozen: header 24 B (magic/reserved/version/universe/profile/feature_bits/width/height); records 0x01–0x09; checkpoint 0x03/0x08; intervals 0x04 with transitions 0x21–0x30; residual block kinds 0–2; integrity = trailing BLAKE3. Feature bits: `0x1` external objects (Phase P), `0x2` quantized-content declaration (Phase U). See `docs/format-v1.md`. |
| 5 | Universe/profile model | `UNIVERSE_V1 = 1`, `LIMIT_PROFILE_V1 = 1` (`src/universe.rs`, `src/limits.rs`); v1 header rejects anything else typed |
| 6 | Archive semantics | `src/archive.rs`: `.volea` sidecar manifest (magic `VOLEARC1`, schema 1, self-seal), record index + object/checkpoint hashes + per-frame reconstruction hashes; layered verify; standalone-only; deep verify = one decode pass |
| 7 | Partial materialization | `src/partial.rs` (Phase S): demand plan backward from the view over COPY/MOVE reads of the immediately previous observation; per-level `Region` (merged row spans, saturating); per-level `PartialFrame` boxes; `materialize_view(parsed, idx, view)`; FullFrame == canonical step machinery; audit-scope semantics documented |
| 8 | Native ingest | `src/ingest.rs` (Phase Q): typed `Ingest` over the v1 descriptor encoder; `finish()` re-validates; §53 script format in `src/script.rs` |
| 9 | Transport | `src/transport.rs` (Phase R): `[len][kind][seq][body]` framing over standalone v1 records; receiver rebuilds the canonical prefix and plays it through the normative parser |
| 10 | Perceptual boundaries | `src/lossy.rs` (Phase U): encode-time integer `Q` over raster-origin Gray8; feature bit 0x2 declaration; exact decoder unchanged |
| 11 | APIs assuming Gray8 | **The entire sample domain.** `pixel::Canvas { data: Vec<u8> }`; `PixelFormat::Gray8` only; header carries no pixel-layout field; every object content kind (fill `u8`, raster `Vec<u8>`, index plane `u8`, generator → mod-256 `u8`); state background `u8`; overlay points `(x,y,v:u8)`; palette entries `Vec<u8>`; residual point values `u8`; transform-residual domain checked to `0..=255`; rANS is byte-oriented (depth-agnostic — reusable); archive frame hashes hash Canvas bytes; lossy lattice is defined over `0..=255` |
| 12 | APIs owning a full raster destination | `materialize::materialize_full(state,w,h,limits) -> Canvas` allocates the whole canvas; `decoder::step_frame` requires `prev: &Canvas` (full previous frame); `materialize_all` retains every frame (`Vec<Canvas>`); partial decode allocates per-level boxes and returns an owned crop (`PartialView { canvas }`) |
| 13 | `materialize_all` call sites | Library: `optimize.rs`, `collapse.rs` (equivalence proofs), `inverse.rs` (final verification), `lossy.rs` (reconstruction proof), `transport.rs` receiver (`frames_so_far`), `demo.rs` courts, CLI `verify`/`decode`. Tests/examples: universal as the conformance oracle. **No playback path exists.** |
| 14 | Previous-frame dependencies | One normative family: COPY_RECT/MOVE_RECT (Phase D) read the **immediately previous decoded observation** (snapshot semantics, depth 1). Sequential replay (`step_frame`, `materialize_all`) satisfies it with the prior full frame; `partial.rs` satisfies it per-level with demand-planned boxes; archive deep verify replays sequentially. No temporal prediction beyond this exists |
| 15 | Historical suite | `cargo test` / `--all-features` / `--release --all-features`: 279/279 pass; fmt/check/clippy `-D warnings` clean |
| 16 | Starting record | Commit `1308f0d`, clean tree, rustc 1.98.0, ffmpeg n9.0.1 + ffprobe available on the audit machine (bridge subphase V.1.3 will record versions per run) |

### Structural gaps the brief exposes (reconciled)

1. **Sample domain is monochrome 8-bit.** There is no plane model, bit depth
   > 8, subsampling, chroma location, color description, alpha, or HDR —
   because v1 froze exactly Gray8 (§11/§14–§25 of the brief).
2. **Time is an integer interval index, not a media timeline.** `Interval(u64)`
   anchors every stream at frame 0 with no duration, no rational PTS, no VFR
   (§10–§12).
3. **One canvas per stream.** No epochs, no resolution/layout change mid-stream
   (§13).
4. **One global state, one checkpoint, single interval chain.** No
   multi-track, no bidirectional temporal structure, no reference management
   (§70–§72).
5. **Materialization owns its destination.** `materialize_full` allocates;
   presentation does not exist (§111–§116).
6. **The representation ladder stops at transform/rANS/RAW.** No subpixel
   motion, local-motion regions, spatial/cross-component prediction, or grain
   hypotheses (§44, §57–§76, §82–§86).
7. **No foreign ingest.** Everything is synthetic or raw Gray8
   (§31–§41).
8. **Canvas memory scales with full-frame samples** even in partial decode
   (per-level boxes are demand-bounded but still owned crops; §164–§176's
   caller-owned target refactor is new).

The brief's §45 observation is accurate and is adopted as a design rule: the
existing families (UNCHANGED, EXACT_REF, SPARSE, COPY_RECT, TRANSLATION,
TRAJECTORY, PALETTE, REGIONS, AFFINE, TRANSFORM_RESIDUAL, GENERATOR, rANS/RAW)
are **generalized, not rewritten**, and v1 behavior must remain an exact
specialization.

---

## 2. Decisions of this contract

Conventions: `DECIDED` = V.1.1+ implements against this; `DEFERRED(subphase)`
= explicitly open until the named subphase; `RECORDED` = measured fact.

### 2.1 Format evolution: v2 + universe v2, v1 permanent (DECIDED)

v1's frozen 24-byte header has no field that can declare a pixel layout, bit
depth, plane geometry, color description, or timeline base, and the brief
forbids contorting v1 (§8). **VOLE introduces format v2 (`format_version = 2`)
and a video universe v2 (`universe_id = 2`) for the V.1 programme.** The v1
parser, decoder, materializer, goldens, and every A–U court remain untouched
and permanently supported (same crate, same binary, dispatch on the header).
No old `.vole` stream acquires new interpretation.

v2 inherits the v1 *principles* wholesale (manual little-endian canonical wire,
mandatory fail-closed feature bits, integrity trailer, explicit-advance state
machine, typed `Limits`, non-Turing-complete materializer, residual closure,
archive/transport/store layering as *independent* of the media model where
possible) and extends the *ontology* (media descriptor, epochs, timeline,
component planes, richer predictor/residual families). The exact v2 byte
grammar is **not frozen by this document**; it is specified in
`docs/format-v2.md` during V.1.1–V.1.2 and frozen at the end of V.1.2 by the
usual gate. The v2 header must keep the same first fields as v1 (magic,
reserved, `format_version`, `universe_id`, `limit_profile`, `feature_bits`)
so version/universe dispatch is a pure read of the existing prefix.

### 2.2 Two clocks, one state machine (DECIDED)

The procedural state machine keeps the v1 discipline: **explicit interval
advances only** (`Interval(u64)`), never implicit stepping. V.1 adds a
separate, declarative **media timeline** that maps state-evolution ordinals to
rational presentation times. The timeline never changes what the state machine
computes; it only says *when* an observation is presented and *how long* it
lasts. This keeps Φ deterministic and VFR a mapping concern:

```
state ordinals (intervals, explicit Δ application)
        │  timeline: ordinal → (PTS, duration)
        ▼
canonical video observation sequence (presentation order)
```

Rational time primitives (all integer, checked):

```rust
pub struct TimeBase { pub numerator: u32, pub denominator: u32 } // seconds = n/d
pub struct Pts { pub value: i64, pub tb: TimeBase }              // signed, checked rescale
pub struct Duration { pub value: i64, pub tb: TimeBase }         // ≥ 1 tick; VFR = per-observation
```

No floating-point timestamps anywhere normative. Decode order of a foreign
compressed source is provenance only (§11); the canonical track is
presentation-ordered.

### 2.3 Epochs (DECIDED)

A v2 stream carries one or more **video epochs**. Each epoch declares the full
media interpretation of its observations: canvas geometry, pixel layout,
per-plane geometry, bit depth, subsampling, chroma sample location, color
description (primaries/transfer/matrix/range), field structure, SAR,
orientation, and the timeline base of its first observation. Observations are
tagged with their epoch; a change of any declared property is an epoch
boundary in the stream, never a silent rescale (§13). V.1.1 defines the epoch
record; synthetic epoch-change courts (V.1.1/V.1.19) and the dynamic-resolution
court (§208) exercise it.

### 2.4 Component-plane model (DECIDED for V.1.1–V.1.2; grammar frozen V.1.2 end)

Canonical internal storage is **planar, tight, no stride padding**, LE:

- ≤ 8 active bits per sample → `u8` plane (1 byte/sample);
- > 8 active bits → `u16` plane, little-endian on the wire, active bits exact,
  unused high bits canonicalized to 0 at import and rejected if nonzero on the
  wire;
- ≥ 32-bit float sample sources (F16/F32) initially fall to an **exact
  opaque raw-bit plane** representation (§17) — never silently quantized;
  procedural float analysis is out of scope until integer video is sealed.

Layouts are a canonical registry (`PixelLayoutId`), not free-form geometry:

```
GRAY · YUV 4:0:0 · 4:2:0 · 4:2:2 · 4:4:4 · YUVA · GBR · GBRA ·
RGB · BGR · RGBA · BGRA · ARGB · ABGR · NV12/NV21 · P010/P016 ·
YUYV422 · UYVY422 · PAL8
```

Packed external layouts (NV12, P010, YUYV422, UYVY422, PAL8, RGBA…) are
**reversibly unpacked** into canonical planes at import; decoder stride
padding is never preserved (§18). `Component { Y, Cb, Cr, R, G, B, A, Gray,
Other(u16) }` names planes; subsample_x/y are **geometry** with exact
ceil/floor rules (odd-dimension courts: 1×1, 3×3, 1919×1079, 1921×1081),
chroma sample location is preserved explicitly (§19–§20), and SAR is stored
exactly and never resampled on import (§27).

Color description preserves primaries/transfer/matrix/range/chroma location
and HDR static metadata (mastering display colour volume, content light level)
when the source signals them; missing metadata is `UNSPECIFIED`, never
guessed (§21–§24). Orientation and interlace metadata are stored as
interpretation, never baked into samples (§26, §28). Alpha is an independent
exact sample component; presentation compositing is presentation policy
(§25). Known typed side data is a bounded registry
(`KNOWN_TYPED` / `OPAQUE_PRESERVED` bounded / `UNSUPPORTED` → refuse if
mandatory) (§29).

### 2.5 Exactness is multi-dimensional (DECIDED)

One vague `exact = true` is replaced by separate flags recorded per import and
per verification (§30):

```
sample_exact · timeline_exact · color_metadata_exact · orientation_exact ·
auxiliary_metadata_exact · source_bitstream_exact
```

`source_bitstream_exact` is only true when `--archive-source` stored the
original compressed bytes (§236) — it is archival provenance and never counts
as procedural efficiency. The lossless VOLE target is a **canonical hash**
(§40): a VOLE-owned, domain-separated BLAKE3 over epoch description + layout +
color description + per-observation (PTS, duration, plane geometry, canonical
sample bytes); SHA-256 is additionally recorded in evidence where useful.

### 2.6 Independent-plane correctness first; shared geometry later (DECIDED)

V.1.2 generalizes the existing families to the canonical domain by the §46
path: each plane is proceduralized independently (Y, then Cb, then Cr — or
R/G/B) through the generalized versions of the sealed v1 families, and
independent-plane materialization must be byte-exact and courted before any
cross-plane hypothesis exists. §47's **shared motion geometry** (one visual
motion state; decoder derives per-component coordinates through exact
subsampling/chroma-location rules with declared signed rounding — never bare
`dx / 2`) is designed in V.1.5–V.1.7 and lands only behind courts. Cross-plane
linear prediction (§76) is a V.1.8 ablation candidate, never a default.

### 2.7 Object identity generalizes (DECIDED)

Content identity (§52) hashes the **canonical object record**: domain/layout
tag + geometry + per-plane geometry + sample bit depth + canonical samples or
program — never host memory, never foreign packed bytes unless that packing
*is* the canonical record. BLAKE3 stays the identity primitive. Temporal
object identity remains structural/economic (§53): recurring exact patches,
repeating textures, rectangular regions, procedural fields — no ML, no
semantic labels. Phase-P store sharing extends naturally: the object record
grammar is layout-aware, so cross-video sharing continues to work by exact
identity without change of mechanism.

### 2.8 The representation ladder (DECIDED ordering discipline)

The inverse compiler's representation space is the brief's §44 ladder. Its
evaluation order is the §98/§293 staging (regime → global → large-region →
local → palette/generator/spatial → residual → entropy/RAW), implemented as a
bounded candidate DAG (§92–§93) whose leaves are the generalized sealed
families plus the new families, each of which must **earn itself** in an
ablation court before it becomes a default. New families and their entry
subphases:

| Family | Subphase | Entry condition |
|---|---|---|
| Generalized v1 families over planes/depths | V.1.4 | multiplane exactness sealed |
| Global translation/rotzoom/affine proposals | V.1.5 | fixed-point normative materialization, Q8→Q12/Q16 by court (§62) |
| Local region translation / bounded motion field | V.1.6 | region/motion-count limits; motion-bomb courts (§214) |
| Subpixel motion + committed integer interpolation tables | V.1.7 | filter tables generated once, integer, committed + hashed (§58–§59); exhaustive filter/vector property courts |
| Spatial intra + cross-component predictors | V.1.8 | complete-byte ablations (§74–§76) |
| Film grain (hypothesis + exact residual) | V.1.9 | AFGS1/AV1 studied as prior art (§85); lossless model `R = F − (Base + Grain(G))` measured against plain residual/RAW (§82–§84); never claimed without the residual |
| Transform/residual floor strengthening (8×8/16×16, skip, contexts) | V.1.10 | reversible-transform property tests + coefficient-range proofs (§89–§90) |
| Bidirectional prediction | deferred until archive/index support bounded forward deps | §70–§72 cost discipline; streaming damage must be priced |
| Mesh/warp, dense flow | NOT before V.1.11 oracle shows need | §67–§69: flow is proposal only |

**Bidirectional prediction is explicitly not in the V.1.1–V.1.10 critical
path.** The brief itself makes it conditional (§70); VOLE's single-checkpoint
interval chain plus the archive index is the prerequisite, and the streaming
cost discipline of §72 applies before any byte saving is accepted.

### 2.9 Deterministic search & cost (DECIDED — reuse, don't rebuild)

DSFB stays zero-authority and non-normative (§106–§110); it governs
family/order/breadth over the expanded candidate set with the same
φ/ω/α model, rotating deterministic sentinels (RAW + incumbent + cheap
predictor floor), no stochastic exploration. Exhaustive stays the oracle
(§97). The complete byte cost `J_B` (§94) is the Phase-A §31 accounting
generalized with new buckets (motion descriptors, predictor data, grain
parameters, per-plane objects), and profile-aware physical cost uses
integer/fixed rational weights (§95) — never nondeterministic float tie
breaks. `vole optimize` (§105) generalizes Phase O with the same two-gate
rule: `M(D0) == M(D1)` (now multi-plane exact) and `J(D1) < J(D0)`; offline
optimization stays separate from online ingest (§42–§43, §104).

### 2.10 Direct materialization & presentation (DECIDED architecture; backend DEFERRED to V.1.16)

The V.1 refactor introduces **caller-owned materialization targets**:

- canonical domain: `materialize_into(target, view, scratch)` — the primitive;
  historical `materialize_full(...) -> Canvas` etc. become compatibility
  wrappers that allocate then delegate (§111–§112). This mirrors exactly how
  Phase S already bounds per-level boxes; the new step generalizes
  Region/PartialFrame painting to write **into caller memory** and generalizes
  the painter from one Gray8 plane to per-plane writes with shared geometry.
- presentation domain: `VideoPresentationTarget` (§114) — final-surface
  semantics: extent, format, writable region, damage submission. The **headless
  in-memory target comes first** (V.1.14, §164) and is the CI/hash/profiling
  oracle; the native CPU surface backend is selected at V.1.16 after a
  re-evaluation of the current Rust backend landscape (winit+softbuffer
  remains the leading candidate because `Buffer::age()`, writable presentation
  buffers, and damage submission line up with §126–§132 — but the choice is
  **not frozen here**).
- fused path: the tile pipeline of §115–§116 — output tile → canonical
  component regions → Phase-S demand plan → bounded source tiles → integer
  presentation projection → direct final-surface write. **Damage**
  (what changed, from conservative per-state-source derivation, §122–§124)
  and **demand** (what a predictor must read, §118–§121) stay separate graphs;
  the demand planner is Phase S generalized, never rewritten. Filter halos are
  declared per predictor (§119–§120); chroma demand follows exact subsampling
  (§121); COPY snapshot semantics survive unmodified (§152).
- buffer age (§126–§130): age 0 → full direct redraw; age 1 → refresh
  `damage(prev→cur)`; age > 1 → bounded presentation history, refresh the
  union; unproven provenance → full redraw, never a guess. `present_with_damage`
  used where supported; platform copies are disclosed separately from
  VOLE-owned copies (§131–§132).
- **Directness gate**: during normal native playback
  `full_canonical_frame_allocs == 0` and `full_RGB_staging_allocs == 0`, with
  bounded tile/dependency/transform/residual scratch measured by
  `VideoDirectReport` (§196) and allocation instrumentation (§166). Full-damage
  natural video writes ≈ whole surface *through tiles* — that does not fail
  directness (§167).

Presentation projection (§134–§139) is a declared, integer, standards-derived
fixed-point YCbCr→RGB (matrix/range policy explicit, courted against an
independent high-precision reference; chroma upsampling filters frozen;
byte-order courted on the actual target format). HDR canonical preservation is
independent of SDR preview: `HDR_CANONICAL_EXACT` /
`HDR_NATIVE_PRESENTATION_UNAVAILABLE` / `SDR_PREVIEW_AVAILABLE` is an honest
classification, never silent tone-mapping of stored media (§139). Scaling,
orientation, and deinterlacing are presentation policies, separated from
canonical hashes (§140–§143).

### 2.11 Streaming decoder, seek, playback discipline (DECIDED shape; DEFERRED V.1.17–V.1.18)

`PlaybackDecoder<R: Read+Seek>` (§144) reads through the Phase-T index and
checkpoints, keeps a bounded object cache (never a completed-frame cache,
§148–§149), prefetches bounded future groups (§150), and exposes
`advance_to(timestamp)` + `present_into(&mut target, view, scratch)` (§146).
Player code never calls `materialize_all` and never exposes
`next_frame() -> Vec<u8>` as the core abstraction (§145, §228). Seek is
index → nearest legal checkpoint → restore → bounded replay → direct present
(§155–§156) with p50/p95/p99 metrics (§157). Scheduling uses presentation
timestamps; winit `WaitUntil` deadline semantics are the model (§158–§159);
late presentations may skip *display observations* but never required *state
transitions* (§160). Pause, frame-step, resize, occlusion follow §140, §162,
§163. Streaming file access replaces whole-file reads for playback (§147,
§260); `>4 GiB` and very-long-timeline courts (§209–§211) guard the offset and
timestamp widths.

### 2.12 Foreign bridge (DECIDED architecture; DEFERRED V.1.3)

Foreign decode exists **only at import/export** (§31): `ffmpeg`/`ffprobe` as
subprocesses via `std::process::Command` with individual arguments — never
shell strings (§32); no ffmpeg-sys/libav* normative dependency. The import
pipeline: ffprobe manifest (§38) → software reference decode (never hardware
as the reference, §33) with silent transforms disabled (§34) and pixel-format
retention researched per the `-pix_fmt +` semantics (§35) → **narrow NUT pipe
bridge** (§36–§37: main header, stream header, time bases, video packet
timestamps/framing, raw-video tags, checksums, bounded lengths — unsupported
bridge features are typed errors) → framehash oracle (§39: independent
per-observation digest over the SAME canonical sample layout; mismatch ⇒
`IMPORT FAIL`) → canonicalizer (packed→planes, depth exact, padding dropped)
→ streaming inverse ingest (§42: bounded frame/byte/time lookahead, discard
when unneeded) → atomic `.vole` write (temp sibling + parse/integrity/decode
verify + rename, §220). Deterministic stream selection (§41), child-process
limits (wall/idle time, stdout/stderr, sample/observation/dimension bounds,
clean kill, §217), local-file-only default with no network URLs unless an
explicit `--allow-network` exists (§218). The NUT reader is a *narrow*
implementation of exactly what VOLE's controlled bridge emits — never a second
multimedia framework (§37).

### 2.13 Export (DECIDED default; DEFERRED V.1.21)

`vole export movie.vole out.mkv` streams canonical observations through NUT to
an FFmpeg encoder; default lossless target Matroska + FFV1 v3 where the
canonical layout is exactly supported (§232); layout/depth-preserving by
default, otherwise `UNSUPPORTED_EXACT_EXPORT` (§233); export-hash court
per observation (§234); `--reproducible` bitexact mode where the foreign
toolchain supports it (§235). Playback and export are different boundaries
(§1): export intentionally materializes conventional raster observations.

### 2.14 Verification, inspection, CLI (DECIDED surface; DEFERRED to the subphases that build each piece)

```
vole probe-media input.mkv            (V.1.3, §41)
vole import  input.mkv movie.vole     (V.1.19, atomic, canonical-hash verified)
vole verify  movie.vole --full        (canonical hash + exactness dimensions)
vole play    movie.vole               (V.1.16–V.1.18, direct)
vole inspect movie.vole --video|--analysis   (V.1.23, §237–§238)
vole export  movie.vole out.mkv       (V.1.21)
```

`vole inspect --analysis` is representation accounting at interval granularity
(§238) — never "semantic AI explanation". Research-only debug visualizations
(§239) are non-normative.

### 2.15 Security, hostile courts, fuzz, property tests (DECIDED to extend; DEFERRED per subphase)

`Limits` gains the §212 envelope as families land (video tracks, plane/observation
sample & byte caps, epochs, motion regions/vectors/work, warp work, transform
blocks/work, generator work, residual bytes, reference depth, replay, tile and
dependency scratch, duration, observation count), plus the specific bombs of
§213–§216 (tiny stream ≠ cheap stream; motion/transform/reference bombs) and
typed media errors (§219: `Bridge*`, `UnsupportedPixelLayout`,
`UnsupportedColorDescription`, `CanonicalHashMismatch`, `VideoEpochViolation`,
`PresentationUnsupported`, … — never strings). Fuzz targets and property tests
follow §221–§225 incl. `decode(encode(canonical)) == canonical`,
partial == crop, direct == reference projection, checkpoint seek ==
sequential, thread-count invariance, and per-family exactness properties.

### 2.16 Evidence, corpora, courts, baselines (DECIDED conventions)

Immutable campaigns `phase-v1-0-audit` … `phase-v1-23-final` (§241); each
records commit/toolchain/hardware/OS/source hashes/bridge versions/commands/
canonical hashes/VOLE hash/decisions/accounting/timings/memory/directness/
failures (§242). Corpora are manifest-tracked (SHA-256, license/provenance,
content class — never filenames, §240) and separated by class (§182–§185).
Baselines: raw canonical video + FFV1 as the preservation baseline + lossless
H.264/HEVC/AV1 (and screen-content modes where available), with full command
receipts (§57 discipline, §188–§190). Negative controls are mandatory:
crypto-random video (§186, §290), already-noisy/artifact-heavy decoded content
(§187), and the flagship courts of §285–§290 (structural, real-video,
direct-codec, high-bit-depth, screen, noise). Real-time classification is
honest (`REALTIME`/`NON_REALTIME`/`UNSUPPORTED`, §198–§199, §279).

---

## 3. Inverse hierarchy and candidate composition (contract for V.1.11)

The hierarchical inverse compiler stages (§98) each emit complete valid
candidates; the candidate DAG bounds composition depth (§92); every candidate
reports dependencies, descriptor bytes, payload bytes, residual bytes, decode
work, reference depth, working memory (§93); every winning candidate is
materialized by the normative materializer, receives an exact per-plane
residual, is compared byte-for-byte to the target observation, and only then
enters the cost court (§29 of the original brief; §96 here). Composite
candidates such as GLOBAL_AFFINE + LOCAL_RESIDUAL, MOTION_REGION +
TRANSFORM_RESIDUAL, GENERATOR + RESIDUAL, PALETTE + SPARSE are bounded
compositions, not an arbitrary expression-tree search. Encoder-side proposal
analysis may use float/pyramids/phase correlation/least squares (§99–§100);
the deterministic encoder mode pins algorithms, iteration counts, candidate
order, threading, and seed policy (§101); stored parameters are always
normative integer/fixed-point, and decode never depends on the proposal side
(§57, §61–§62, §69, §100).

## 4. Film grain discipline (contract for V.1.9)

Grain is a serious court, not "noise". AV1's normative grain synthesis and
AFGS1's codec-independent generalization are prior art to study (§85), with
the VOLE strictness that **lossless closure is mandatory**: for a candidate
grain model `G`, `F̂ = Base + Grain(G)` and `R = F − F̂` exactly (§83), and the
complete cost `B(base) + B(grain params) + B(grain residual)` competes against
ordinary residual coding and RAW. Two separate results are never conflated:
source-native seeded grain (exact generator, zero residual — §84) versus
inferred camera/film grain (statistical hypothesis + measured residual — §82).
An adopted VOLE grain generator defines its integer PRNG, seed, spatial
correlation, luma/chroma relation, block overlap, amplitude curve, clipping,
and temporal seed advancement exactly (§85–§86). If the model does not save
complete bytes on the grain corpus, it loses (§63 of the original brief
discipline).

## 5. Prior-art discipline (contract-wide)

Established codecs already own motion compensation, global/affine motion,
palette coding, intra-block copy, transform coding, and film-grain synthesis;
VOLE claims none of these in isolation (§226). The research contribution under
test is the state-first architecture — persistent deterministic state and its
evolution as the primary representation, with these predictor families as
subordinate, bounded, courted hypotheses under a complete-cost rule — and its
empirical integration across synthetic, screen, and natural corpora. The
language rules of §227–§228 hold: "every supported observation is represented
because unexplained information is retained as exact residual/fallback", and
"no mandatory complete intermediate raster-frame representation during native
playback" — final presentation surfaces still contain samples.

## 6. Sequence of execution (V.1.1 → V.1.23, entry-gated)

Follows brief §293 exactly. Entry gate for each subphase: the previous
subphase's receipt + the relevant courts + the full historical suite still
green. V.1.0 (this document) is the only step that required no feature code.
The first implementation subphase is **V.1.1 (canonical media domain)**:
rational time + epochs + plane/layout registry + bit depths + subsampling +
color/orientation/SAR/interlace/side-data types, exercised on synthetic
canonical vectors — no foreign import yet. V.1.2 then generalizes the core to
YUV444/420/RGB/10-bit multiplane exactness; V.1.3 builds the bridge; V.1.4
generalizes every existing family; V.1.5+ adds the new predictor families in
ladder order; V.1.13+ refactors the materializer to caller-owned targets and
builds directness upward from the headless target.

```
audit + contract (V.1.0)        ◄── this document
canonical media domain (V.1.1)
multiplane core (V.1.2)
import bridge (V.1.3)
existing-family generalization (V.1.4)
global motion (V.1.5) → local motion (V.1.6) → subpixel (V.1.7)
spatial/cross-plane (V.1.8) → film grain (V.1.9) → residual floor (V.1.10)
hierarchical inverse compiler (V.1.11) → DSFB (V.1.12)
target materializer (V.1.13) → headless direct (V.1.14) → damage (V.1.15)
native surface (V.1.16) → streaming decoder (V.1.17) → seek (V.1.18)
real media import (V.1.19) → falsification corpus (V.1.20)
export (V.1.21) → soak (V.1.22) → final seal (V.1.23)
```

Acceptance (§267–§283) is the seal gate of V.1.23 and cannot be satisfied
early: historical compatibility; multiplane exactness (Gray8 + YUV420P8/P10 +
YUV422 + YUV444 + RGB + RGBA); color/HDR/VFR preservation; real compressed
H.264/HEVC/AV1 import; native playback with FFmpeg removed from PATH and the
network disabled; the directness zero-staging gate; reference equality of
canonical and direct-presentation hashes; structural and natural corpora
reported (not predetermined); random control near fallback with bounded
search; honest real-time classification; exact seek; long soak boundedness;
exact lossless export; and no panic/OOM/hang across malformed `.vole`, hostile
bridge data, and bounded foreign-input courts.

## 7. Open items and their owners

| Item | Open until | Notes |
|---|---|---|
| Exact v2 byte grammar | end of V.1.2 | header sketch in §2.1; record grammar in `docs/format-v2.md` |
| Q8 vs Q12 vs Q16 affine precision | V.1.5 court | §62 — never assume more precision wins |
| Subpixel precision set | V.1.7 court | integer/½/¼/⅛/1/16 with full measurements (§60) |
| Filter tables | V.1.7 | committed integer tables + generator + hashes (§59) |
| Bidirectional prediction | post-V.1.10 | prerequisite archive/index support + §72 cost discipline (§70) |
| Native presentation backend | V.1.16 | winit+softbuffer is the leading candidate, not frozen (§133) |
| NUT bridge constants | V.1.3 | narrow reader over VOLE-controlled ffmpeg output (§37) |
| HDR native presentation | post-V.1.16 | honest classification first (§139) |

## 8. Audit evidence

This audit is recorded under `evidence/campaigns/phase-v1-0-audit-*/`
(environment manifest + audit summary), with the commit hash of the V.1.0
milestone in the manifest. The next milestone (V.1.1) starts from this
document and the sealed v0.17.0 tree.
