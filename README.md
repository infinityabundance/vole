# VOLE — Video Object Layer Engine

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

## Sealed phase surface (Phases A–O)

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

## Build / test gate (each sealed phase must pass)

```
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## CLI (Phase A–O surface)

```
vole demo moving-rect [out.vole]
vole encode --width W --height H [--frames N] in.raw out.vole
vole decode <in.vole> [outdir]
vole verify <in.vole>
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

**Current head: Phase O sealed** (representation re-optimization); next in the
mandated ladder is **Phase P** (optional EntropyFS persistence), then Q–U.
The phase ledger, mechanism ledger, per-phase receipts, and frozen format
decisions are authoritative and kept current:

```text
PROJECT_STATE.md           current head, phase, next action, failures, frozen format
CONFORMANCE.md             per-phase conformance rows + goldens
SECURITY.md / SPEC.md      limits, hostile-input contract, universe v1

docs/empirical-status.md   mechanism ledger (ADOPTED / RECORDED / PROPOSED / …)
docs/phase-{a..o}.md        sealed phase receipts
docs/architecture.md       format-v1, transitions, residuals, accounting, …
evidence/campaigns/        timestamped, reproducible evidence (never overwritten)
```

Mechanisms are **never marked implemented/adopted on the basis of intent**;
only measured, courted mechanisms are.

## Release

Published on crates.io as [`vole-video`](https://crates.io/crates/vole-video)
(lib `vole_video`; the `vole` binary). Current: **v0.11.x — Phases A–O sealed**.

## License

Dual-licensed under the MIT or Apache-2.0 terms, at your option.
