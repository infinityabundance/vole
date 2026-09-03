# VOLE Security

## Threat model

Decoded `.vole` data is treated as **hostile and untrusted**. The decoder must
produce either a valid bounded decode or a typed [`VoleError`]; it must
**never** `panic!`, exhaust the stack, exhaust memory, or hang.

## Posture

- `#![forbid(unsafe_code)]` across the crate. No `unsafe` is present without a
  measured necessity and an explicitly recorded ADR.
- Every untrusted length/count/coordinate/width/height is validated against the
  active [`Limits`](src/limits.rs) *before* it may drive an allocation or a
  loop bound (checked integer arithmetic through `src/checked.rs`).
- No generic deserializer: the manual wire format (`docs/format-v1.md`) has no
  integer-massaging surface for an attacker to abuse via type confusion.
- Unknown **mandatory** features fail closed (`UnsupportedFeature`), as do
  unknown universe, profile, tags, and non-canonical encodings.
- Reference integrity is enforced as data is applied (`UnknownObject`,
  `UnknownInstance`, cycles/self-reference rejected per the limits) and the
  whole file is covered by a BLAKE3 integrity trailer.

## Required hostile cases

Implemented and asserted in `tests/malformed.rs` (Phase-A set):

```
truncated packet, wrong magic, tampered universe, non-zero feature bits,
oversized dimensions, integrity tampering, body bit flips,
reference to undeclared object, duplicate ids, checkpointless/empty, garbage tail
```

Phase-B+ adds: invalid object ID / self-reference / dependency cycle /
reference cycles, huge declared object geometry, non-canonical interval
ordering, entropy overread/underflow, residual-expansion bombs, invalid
checkpoint, and unknown-feature streams — each must yield a typed error.

## Report

Security concerns: file an issue in this repository. This project is research
software; see also `docs/architecture.md` limits.
