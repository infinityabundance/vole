# VOLE Architecture

## 1. Position within three systems

The VOLE research program keeps three concerns separate (see ADR-0003, 0004):

```
VOLE      = normative procedural video representation + standalone format
EntropyFS = optional representation-aware persistence substrate
DSFB      = zero-authority encoder search intelligence (residual/governance)
```

This crate implements **VOLE the normative representation**. It is standalone,
self-contained, native Rust, `#![forbid(unsafe_code)]`, and decodes without any
of EntropyFS/DSFB/ML/network/external codecs.

## 2. The invariants we hold

1. **Raster is a view, not the primary stored object.** A `.vole` file stores
   objects, instance state, a checkpoint, and transitions. Full frames are
   produced by the materializer on request.
2. **Lossless is sample-for-sample exact.** For a lossless target,
   `M(U,G,V) ⊕ R = F*`. Candidates that win an encoder court are validated
   through the normative materializer, never assumed.
3. **Every transition is bounded & deterministic.** No scripts, no unbounded
   loops, no Turing-complete materialization. Hostile input yields typed
   errors (never a panic). See `SECURITY.md` and `tests/malformed.rs`.
4. **Information theory binds.** When a procedural explanation is poor the
   residual grows toward the raster size and VOLE falls back to literal /
   entropy-coded representation. Nothing "hides" this (see
   `docs/information-accounting.md`).
5. **Complete accounting.** Storage decisions compare all persisted bytes — not
   just residual bytes — under one typed cost model.

## 3. Data-flow

```
Universe (U)               versioned normative semantics
   │
Procedural State Graph (G) objects, instances, positions, background
   │ Φ (apply Δ)
Transitions (Δ)           SET_POSITION / INSTANCE_CREATE / DECLARE_OBJECT ...
   │
Materializer (M)          state → raster view (FullFrame in Phase A)
   │
View (V) / residual ⊕ R
   │
Raster output (F)
```

Two ingestion paths (both supported by the design; Phase A ships native ingest
and validates decode; raster-origin inverse proceduralization arrives with the
dedicated encoder phases):

```
source procedural state ─► VOLE state ─► transitions ─► materializer   (native)
target raster ─► candidates ─► materialize ─► residual ─► cost court    (inverse)
```

## 4. Phase model

Phases are executed in sequence with evidence gates. See
`PROJECT_STATE.md` for the live ledger and `docs/empirical-status.md` for the
mechanism status table. **A phase is SEALED only when its code, tests,
malformed-input court, empirical court, negative controls, and receipt all
exist and pass.** No mechanism is credited by intent.

## 5. Determinism

Normative materialization defines integer widths, endianness, rounding,
clipping, composition order, and hash canonicalization. It never depends on
unspecified floating-point, platform integer width, or mutable process state.
See `src/materialize.rs` and `docs/materialization.md`.
