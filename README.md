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

## Build / test gate (each sealed phase must pass)

```
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## CLI (Phase A–L surface)

```
vole demo moving-rect [out.vole]
vole encode --width W --height H [--frames N] in.raw out.vole
vole decode <in.vole> [outdir]
vole verify <in.vole>
vole bench
```

`vole encode` is the Phase-G raster-origin path: `in.raw` is a concatenated
Gray8 sequence; the exhaustive inverse proceduralizer per frame evaluates
RAW/FILL/UNCHANGED/EXACT_OBJECT_REF/SPARSE/COPY_RECT/TRANSLATION/
RANS_RESIDUAL candidates, validates every candidate byte-exactly, emits the
complete-cost winner, and decode-verifies the stream end-to-end before
writing it.

Example (sealed Phase-G evidence, see `docs/phase-g.md`):

```
# 101 frames of a box gliding over a light background:
vole encode --width 1920 --height 1080 --frames 101 box.raw box.vole
#   -> 2,076,291 B (.vole) vs 209,438,191 B (VOLE raster-only) vs
#      209,433,600 B raw: frame 0 = one full-raster declaration, then
#      every interval is a 26 B integer-translation state evolution.
```

## Status

Phase-level status is maintained in `PROJECT_STATE.md` and
`docs/empirical-status.md`. Mechanisms are **never marked implemented/adopted
on the basis of intent**; only measured, courted mechanisms are.

## License

Dual-licensed under the MIT or Apache-2.0 terms, at your option.
