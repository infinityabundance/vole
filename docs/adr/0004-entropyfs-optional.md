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
- Storage transactions never leak into normative video semantics. EntropyFS
  integration is future Phase-P/P+ work, tracked in `PROJECT_STATE.md`; it
  does not block Phase A.
