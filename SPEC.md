# VOLE Specification (normative core)

This file is the human-readable normative specification of **format v1 /
Phase A**. It is deliberately kept terse and authoritative; the byte grammar
lives in `docs/format-v1.md`, the painter semantics in
`docs/materialization.md`, and time/transitions in `docs/transitions.md`.
Disagreement between this file and `src/` is a bug in `src/`.

## Scoping

- **Format**: v1 (`.vole`), one crate, native Rust, `#![forbid(unsafe_code)]`.
- **Pixel**: Gray8 only. Canonical full frame = `width × height` (strictly
  packed, row-major, top-to-bottom). No alpha/palette/color conversion.
- **Universe**: v1. **Profile**: limit-profile 1.

## Semantics (lossless)

A decoded stream deterministically yields a sequence of full Gray8 frames:
frame 0 from the checkpoint; every subsequent integer interval from the state
after applying that interval's transition group. For a lossless target the
stream is produced such that `M(U,G_t,V) ⊕ R_t = F_t` sample-for-sample; Phase
A has zero residual for court content.

### Painter (normative)

1. new canvas = `background` everywhere;
2. for each instance in paint order, overwrite its object box (clipped) at its
   i32 `(x, y)`.

### Object identity & validity

* Object ids and instance ids are u32; unused all-zero id is not special.
* Duplicate declarations, references to unknown objects/instances, references
  produced before the referenced instance exists, and non-increasing interval
  indices are typed errors.
* All counts/volumes must satisfy the active Limits.

## Transition set (v1)

`SetPosition{id,x,y}`, `CreateInstance{id,obj,x,y}` (interval bodies);
objects declared before checkpoint via fill/raster records.

## Integrity

Last 32 bytes = BLAKE3 over the whole preceding file; verified after
structural parse. Header-semantic errors surface with their precise reason.

See `tests/court.rs` (conformance oracle: materialize == independent painter)
and `tests/malformed.rs` (hostile court).
