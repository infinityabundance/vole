# ADR-0003: DSFB is zero authority

*Status: **ADOPTED*** (design; normative decree applies to every phase)

## Context

DSFB is a drift–slew observer over residual trajectories. Useful as encoder
*search* governance; harmful if it could change samples.

## Decision

DSFB shall **never**:
- change reconstructed samples or procedural semantics;
- bypass candidate validation or declare a lossy candidate lossless;
- override exact final-cost selection among evaluated candidates;
- be required to decode a `.vole` stream.

It may only influence *which candidates / how many* the encoder evaluates.

## Consequences

- `src/decoder.rs`, `src/format.rs`, `src/materialize.rs` contain no DSFB path.
- Standalone `.vole` playback is DSFB-free and remains deterministic.
- Later DSFB-guided encoder phases will be gated by regret courts
  (`N_dsfb < N_exhaustive` at equal-or-better `J`) with typed receipts.
