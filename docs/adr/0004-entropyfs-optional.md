# ADR-0004: EntropyFS is optional persistence

*Status: **ADOPTED***

## Context

Cross-video content-addressed object reuse may materially reduce physical
storage, but must never change the representation's validity or require the
store for playback.

## Decision

EntropyFS is an **optional persistence substrate** layered under a
`ObjectStore`-like abstraction only after standalone VOLE works. The
materializer must not know whether an object came from the `.vole`, EntropyFS,
or memory except through that storage abstraction.

## Consequences

- A standalone `.vole` decodes fully without EntropyFS.
- Objects/shared state are physically accounted (`store-level physical cost`)
  separately from per-stream attribution; shared state is never zero-cost.
- Storage transactions never leak into normative video semantics.

Implementation status: **landed in Phase P** — the `ObjectStore` abstraction,
`EmbeddedStore`, the feature-gated `EntropyFsStore` adapter over the real
entropyfs engine, cross-video exact-object/palette sharing with
physical-vs-declared accounting, GC closure, and the additive external-object
declaration form are sealed (`src/store.rs`, `tests/phase_p.rs`,
`docs/entropyfs.md`, `docs/phase-p.md`). The decision itself predates the
phase and never blocked Phase A.
