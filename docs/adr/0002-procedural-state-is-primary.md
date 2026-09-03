# ADR-0002: Procedural state is the primary representation

*Status: **ADOPTED** (sealed, Phase A)*

## Context

Compressed-raster sequences are the conventional video ontology. VOLE's value
hypothesis is that `state + transitions + residual` can beat it on suitable
content.

## Decision

The persistent object of a `.vole` stream is procedural state (objects, a
checkpoint, interval transitions). Raster frames are **materialized views** —
produced by the normative materializer only when requested. Residual
information exists as an explicit object that closes the gap between the
state's reconstruction and any exact target.

## Consequences

- Phase A must prove this: a moving box stored as *one object + one instance +
  one checkpoint + transitions* reconstructs exact frames *without* storing any
  of them (see `tests/court.rs`, `proof/`).
- Encoder search (inverse proceduralization) is subordinate to the
  materializer; no candidate is authoritative until exact reconstruction is
  validated.
- It remains a hypothesis to measure — never an assumption — how much content
  this helps.
