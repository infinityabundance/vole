# ADR-0005: Materializer is non-Turing-complete

*Status: **ADOPTED*** (Phase A sealed: no interpreter, no scripts)

## Context

Procedural state is a target for hostile input. Embedding a scripting VM,
unbounded loops/recursion, runtime syscalls, or probabilistic generation would
make materialization unbounded and un-auditable.

## Decision

The materializer executes only a **finite, bounded, deterministic** operator
set. Concepts like object fetch, copy/blit, palette lookup, sparse scatter,
motion compensation, parametric trajectory evaluation, procedural generation,
entropy decode, inverse transform, and residual application are each *fixed
descriptors* whose work is bounded by `Limits`. No description can start an
unbounded loop, recuse past the bounded dependency depth, allocate past its
limit, or call the host.

## Consequences

- Phase A materializer = background fill + ordered clipped blits; bounded by
  `Limits` (see `src/materialize.rs`, `src/limits.rs`).
- Every procedural generator that later phases introduce must have bounded
  work and an explicit budget, and must be entered into the empirical-status
  ledger only through a court (no credit by intent).
- Hostile-input tests (`tests/malformed.rs`) assert typed errors, never panics
  or hangs.
