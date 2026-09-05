# VOLE — Video Object Layer Engine

<p align="center">
  <img src="assets/vole.png" alt="VOLE logo" width="313">
</p>

> de Beer, R. (2026). VOLE: Procedural Video Storage and Transport by
> Deterministic State Materialization - Broad Prior-Art Technical Disclosure and
> Research Architecture (Version v1.0). Zenodo.
> <https://doi.org/10.5281/zenodo.22284396>

**Native-Rust procedural video storage, transport, inverse-proceduralization,
and deterministic materialization.**

> **Store the deterministic explanation. Encode what the explanation cannot
> reproduce. Materialize raster samples only when needed.**

VOLE is **not** primarily a conventional block codec. Its central idea is that
a video may be represented as bounded, deterministic **procedural state** and
its **evolution**, with raster frames being *materialized views* of that state
that are produced on demand, plus the explicit **residual** information that a
procedural explanation cannot reproduce:

```
G_{t+1} = Φ(U, G_t, Δ_t)      state evolves deterministically
F_t     = M(U, G_t, V) ⊕ R_t  frames are views ⊕ residual
```

Three concerns are kept permanently distinct:

| Concern | Role |
|---------|------|
| **VOLE** (this crate) | Normative procedural representation, state-transition semantics, materialization, residual reconstruction, standalone `.vole` format |
| **EntropyFS** | Optional representation-aware persistence substrate (content-addressed sharing, GC) — *never required for `.vole` playback* |
| **DSFB** | Zero-authority encoder search governance — *never in normative decode* |

A standalone `.vole` stream must decode **without** DSFB, EntropyFS, ML, a
network, or external codec libraries.

## Repository layout (illustrative; one crate)

```
Cargo.toml            single native Rust crate (no micro-workspace)
src/                  normative implementation (#![forbid(unsafe_code)])
docs/                 architecture, format, courts, ADRs
tests/                conformance + hostile-input courts
evidence/campaigns/   reproducible, timestamped evidence
research/             prior-art paper and briefs
proof/                sealed Phase-A first proof artifacts
```

## Phase-A proof (sealed)

`vole demo moving-rect proof/court-moving-rect.vole` writes a 1920×1080 Gray8
stream storing **one 200×100 object, one instance, one checkpoint, and 100
SET_POSITION transitions** — and *no* per-frame raster. Materializing it
reproduces **101 exact frames** (verified byte-for-byte against an independent
painter, including boundary frames under SHA-256 in `proof/`).

| Quantity | Value |
|---|---|
| `.vole` stream size | 2,692 bytes |
| frames materialized | 101 |
| raw raster for all 101 frames | 209,433,600 bytes |
| single-frame raw | 2,073,600 bytes |
| reconstruction | byte-exact (courts + hashes) |

See `docs/architecture.md`, `tests/court.rs`, `tests/malformed.rs`, and
`docs/transitions.md` for the full court and hostile-input record.

## Sealed phase surface (Phases A–U)

Each phase is sealed only after the full gate
(fmt/check/clippy `-D warnings`/test dev+release), hostile-input courts, an
empirical court, an evidence receipt, and docs. All streams are Gray8 v1,
reconstruction is byte-exact through the normative decoder, and evidence
lives in `docs/phase-*.md` + `evidence/campaigns/`.

| Phase | Mechanism | Headline measured result |
|---|---|---|
| A | Procedural core: state graph · checkpoint · transitions · materializer · RAW/FILL | one object + one instance + 100 transitions → **101 exact 1920×1080 frames** from 2,692 B (raw 209,433,600 B) |
| B | Persistent object identity (BLAKE3) + unchanged-state lane | 10,001 identical views at ≈ 13 B/frame |
| C | Sparse mutation: persistent overlay + strict-sorted SPARSE_PATCH | blink court: 1,820 B → 65 exact frames |
| D | 2D copy/move (COPY_RECT / MOVE_RECT, dependency depth 1) | oracle-exact wrap-scroll court |
| E | Integer translation (persistent per-instance velocity) | 101 frames in **1,505 B** vs 2,692 B per-frame baseline |
| F | Normative entropy floor: native order-0 rANS | byte-parity vs the `ryg-rans-rs` oracle (deterministic, in-crate) |
| G | Exhaustive inverse proceduralization (`vole encode`) | per-frame RAW/FILL/UNCHANGED/EXACT/SPARSE/COPY/TRANSLATION/rANS court, byte-validated; gliding box 2,076,291 B vs 209 MB raw |
| H | Exhaustive / FixedHeuristic / DsfbGuided search over one universe | DSFB ≤ 0.18× candidates at byte-identical cost (steady); 1.055× oracle across four regime changes |
| I | Bounded parametric trajectories + trajectory collapse | accel flagship 686 B vs 1,132 B baseline (41 identical frames) |
| J | Palette state (index objects, mutable palette table, bindings) | accent cycle **24 B/interval** vs 204,773 B palette-less sparse floor |
| K | Variable regions (64 → 32 → 16 → 8) in the raster encoder | localized change with **zero whole-frame rebases** after frame 0 |
| L | Bounded Q8 fixed-point affine state (pan/zoom/rotation) | rotating tile **42 B/interval** (618× vs raw), byte-exact |
| M | Transform residual floor: reversible integer lifting DCT | brightness drift 69,848 B/interval vs 2,073,645 B RAW reset (29.7×) |
| N | Bounded procedural generators (gradient/checker/periodic/noise) | drifting full-HD gradient: 12 frames in **706 B** (35,245× vs raw) |
| O | Equivalence-preserving re-optimization (`vole optimize`) | five rewrite families, each accepted only when strictly smaller **and** decode-identical; flagship 2,692 → 1,505 B |
| P | Optional content-addressed persistence substrate (`ObjectStore`: `EmbeddedStore` + `EntropyFsStore` adapter, feature `entropyfs-store`) | cross-video exact-object + palette sharing: four videos sharing one logo store it once (unique 7 payloads vs 10 declared; dedup exact at payload level); GC closure measured; store-backed streams (`vole` extern tag) materialize byte-identical with payloads outside the file |
| Q | Native procedural ingest API + §53 script format + the §55 preservation court | authored state persists directly (palette/trajectory/affine/generator); flattening-tax court: same rasters cost 7.7× (palette rotation), 37× (acceleration), 49× (noise-tile rotation), 33× (seeded noise) via rasterize→inverse — interval marginal up to 180× |
| R | Procedural transport (§34–§36): `[len][kind][seq][body]` packets in the five classes OBJECT/CHECKPOINT/INTERVAL/RESIDUAL/INTEGRITY; receiver plays prefixes through the **normative parser** | byte-exact reassembly of a standalone `.vole`; typed loss gaps + retransmission; bounded checkpoint replay; §33: static interval 26 B framed vs 15,360 raster samples — whole 25-frame transport 1,488 framed B < one raster frame; unchanged lane amortizes to 29 B/frame over 225 frames |
| S | Partial materialization (§16/§37/§66): `View::Rect`/`View::Tile` demand-planned decode; `FullFrame` views replay the canonical step machinery | 1920×1080 ×41-frame court: tracking 260×140 viewport paints one level of 56,400 samples = **2.72% of the whole-frame lane**, objects touched 1, random access frame 40 = 0.068 ms vs 13.5 ms whole (198×); every view byte-equal to the whole-frame crop |
| T | Archive profile (§67): self-describing, self-sealed `.volea` manifests — record index (byte-level corruption localization, no decode), object/checkpoint hashes, per-frame reconstruction hashes, pinned universe | corrupted bytes localize to their exact record (header / object / interval `t` / trailer); `vole optimize` rewrites keep all frame hashes identical; manifest overhead 0.4% on raster streams (362.9% on 2.7 KB procedural — measured both ends); FFV1 external harness receipt (synthetic court: VOLE 2,692 B state vs FFV1 105,078 B raster) |
| U | Perceptual profile (§64 Phase-U block): deterministic integer quantization `Q` over raster-origin input — the stream encodes the chosen reconstruction `F̂ = Q(F)` and the **unchanged normative decoder reproduces `F̂` exactly** (loss lives in the declared, encode-time lattice; feature bit `0x2` declares it, MAE/MSE/peak measure it, every stream is decode-proven); `vole quant` CLI | flat panel + 2-bit temporal jitter (480×270 ×17): exact 1,806,807 B (106,282.8 B/frame) → **q3 270 B** (15.9 B/frame, 6,692×) at MAE 1.5; recorded: bytes are *not* monotone in the lattice (q1/q2 > exact), dominated q4 row never chosen, authored content gains nothing from lossy (recorded negative), noise stays RAW — exact profiles intact |

## Build / test gate (each sealed phase must pass)

```
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## CLI (Phase A–U surface)

The store substrate (`vole_video::store`), the native procedural ingest API
(`vole_video::ingest`, §53 script format in `vole_video::script`), the
procedural transport layer (`vole_video::transport`, Phase R), the partial
view decoder (`vole_video::partial`, `View::Rect`/`Tile`, Phase S), the
archive profile (`vole_video::archive`, `.volea` manifests, Phase T), and the
perceptual profile (`vole_video::lossy`, Phase U) are library-level. CLI
additions: `vole archive` builds a manifest; `vole verify --archive m.volea`
verifies structurally and deep against it; `vole quant` runs the Phase-U
perceptual profile.

```
vole demo moving-rect [out.vole]
vole encode --width W --height H [--frames N] in.raw out.vole
vole quant --width W --height H --shift S [--rounding halfup|deadzone] [--filter none|box3] in.raw out.vole
vole decode <in.vole> [outdir]
vole verify <in.vole> [--archive m.volea]
vole archive <in.vole> [out.volea]
vole optimize <in.vole> <out.vole>
vole bench
```

`vole encode` is the Phase-G raster-origin path: `in.raw` is a concatenated
Gray8 sequence; the exhaustive inverse proceduralizer per frame evaluates
RAW/FILL/UNCHANGED/EXACT_OBJECT_REF/SPARSE/COPY_RECT/TRANSLATION/
RANS_RESIDUAL/REGIONS/TRANSFORM_RESIDUAL/GENERATOR candidates, validates
every candidate byte-exactly, emits the
complete-cost winner, and decode-verifies the stream end-to-end before
writing it.

`vole archive <in.vole> [out.volea]` (Phase T) builds the archive manifest
(self-description, per-record digests, object/checkpoint hashes, per-frame
reconstruction hashes) for a standalone stream; `vole verify --archive`
verifies the stream against it — structural record digests localize any
corrupted byte to its exact record without raster work, and a deep pass
re-checks every frame's reconstruction hash (see `docs/phase-t.md`).

`vole quant` (Phase U) is the perceptual profile: it quantizes `in.raw`
(Gray8 frames) onto the deterministic integer lattice `2^shift` — optionally
after the canonical `[1 2 1] ≫ 2` pre-filter, with half-up or dead-zone
rounding — encodes the chosen reconstruction `F̂` **exactly** with the
inverse encoder, declares it (feature bit `0x2`), proves the decoder
reproduces `F̂`, and prints the measured bytes + MAE/MSE/peak. The normative
decoder is unchanged and remains exact: lossiness lives entirely in the
declared encode-time quantization (see `docs/phase-u.md`).

`vole optimize <in.vole> <out.vole>` (Phase O) rewrites a decoded stream by
bounded, equivalence-preserving families — velocity/trajectory collapse,
residual promotion, generator substitution, duplicate merge — accepting a
rewrite only when the rebuilt stream is **strictly smaller** and decodes
byte-identically (`M(D0) == M(D1)`, proven).

Example (sealed Phase-G evidence, see `docs/phase-g.md`):

```
# 101 frames of a box gliding over a light background:
vole encode --width 1920 --height 1080 --frames 101 box.raw box.vole
#   -> 2,076,291 B (.vole) vs 209,438,191 B (VOLE raster-only) vs
#      209,433,600 B raw: frame 0 = one full-raster declaration, then
#      every interval is a 26 B integer-translation state evolution.
```

## Status

**Current head: Phase V.1 (video half) V.1.5 sealed** — the mandated ladder
**A → U is complete**; the post-U research programme runs in its own
subphase order. V.1.0 audit + architecture contract
(`docs/phase-v1-video-architecture.md`), V.1.1 canonical media domain
(`src/media/`: rational media time, layout/plane registry with normative
ceil geometry, bit depths 1..=16, color/HDR/side data, epochs), V.1.2
multiplane core + frozen v2 core wire (`src/media/`: sample-domain
`Picture`, independent per-plane programs with the v1 Gray8 specialization
oracle, exact raster-origin ingest floor, and the frozen v2 core grammar in
`docs/format-v2.md`), V.1.3 the **foreign ingest bridge**
(`src/media/bridge/`: bounded argv-only ffmpeg/ffprobe runner, ffprobe
manifest, independent framehash SHA-256 oracle, narrow hostile-safe NUT pipe
reader with exact PTS, reversible source-layout canonicalizer, and
`import_video` proving every canonical observation per frame — FFmpeg/NUT
are non-normative, import-only, and never appear inside `.vole`), V.1.4
the **existing-family generalization** (`src/media/gen.rs` depth-aware
procedural generators; palette-index content + per-plane palette state,
persistent velocity/trajectory motion, Q8 affine placement, and the
Phase-M transform-coded residual in `src/media/core.rs`; the additive v2
family-extension wire under feature bit 0x1 — palette/generator object
kinds, ops `0x29`–`0x31`, initial palette/motion tail; and the per-plane
family encoder `src/media/encode.rs` over the exact floor), and V.1.5 the
**global video structure** (`src/media/global.rs`: the additive v2
global-motion extension under feature bit 0x2 — the `GlobalPredict` canvas
op (tag `0x32`) predicting the whole plane from the previous observation
through a canonical fixed-point map at a declared Q8/Q12/Q16 precision;
and the deterministic bounded translation/rotzoom/affine estimator feeding
global_translation/global_rotzoom/global_affine candidates in the family
encoder, with per-record precision priced and reported) are sealed;
**next is V.1.6 (local motion: region translation / bounded motion field)**.
The phase ledger, mechanism ledger, per-phase receipts, and frozen format
decisions are authoritative and kept current:

```text
PROJECT_STATE.md           current head, phase, next action, failures, frozen format
CONFORMANCE.md             per-phase conformance rows + goldens
SECURITY.md / SPEC.md      limits, hostile-input contract, universe v1

docs/empirical-status.md   mechanism ledger (ADOPTED / RECORDED / PROPOSED / …)
docs/phase-*.md           sealed phase receipts (a–u)
docs/architecture.md       format-v1, transitions, residuals, accounting, …
evidence/campaigns/        timestamped, reproducible evidence (never overwritten)
```

Mechanisms are **never marked implemented/adopted on the basis of intent**;
only measured, courted mechanisms are.

## Release

Published on crates.io as [`vole-video`](https://crates.io/crates/vole-video)
(lib `vole_video`; the `vole` binary). Current: **v0.22.0 — Phases A–U sealed
plus V.1.0–V.1.5** (the full mandated ladder, then the V.1 video programme's
audit, canonical media domain, multiplane core + frozen v2 core wire, foreign
ingest bridge, the existing-family generalization, and the global video
structure: the v2 global-motion extension with the `GlobalPredict` canvas op
and the global_translation/rotzoom/affine encoder families).
The `entropyfs-store` cargo feature (default OFF) links the real EntropyFS
engine adapter; the standalone build never needs it.

## License

Dual-licensed under the MIT or Apache-2.0 terms, at your option.
