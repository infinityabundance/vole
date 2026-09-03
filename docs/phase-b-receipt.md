# Phase B Receipt — persistent object identity

*Establishing object/content identity and the amortized unchanged-state lane.*

## Implemented

Precise immutable content identity mapping a persistent object to a BLAKE3
digest over its canonical record bytes (`src/identity.rs`), a content→ids
registry for object reuse/dedup counting, and an explicit representation of
"nothing happened" as zero-transition interval groups whose frames are
first-class unchanged *views* over persistent object state.

## Tests / courts (all pass, `tests/phase_b.rs`)

- same content ⇒ equal identity; different content ⇒ different identity;
  digest deterministic and 64-letter hex;
- reuse registry counts distinct contents vs total ids;
- static persistent scene: 10 001 frames are all byte-identical to the
  checkpoint view;
- accounting shows a 10 000-interval unchanged stream costs ~13.0 B/frame
  (measured, not a "zero-byte magic" claim) while the equivalent raw raster
  sequence is 20.7 GB.

## Marginal court summary

Static push-pu-lane content: .vole `130,092 B` for 10 001 exact frames of a
1920×1080 UI strip; raw would be `20,738,073,600 B`. Amortized per unchanged
frame ≈ 13.0 B. Reproducible campaign under `evidence/campaigns/phase-b-…/`.

## Negative control

Random/unchanged-hostile objects (content that never repeats) remain out of
scope until the entropy floor exists; recorded as pending in the ledger (Phase
D) rather than fabricated here.

## Mechanisms adopted / rejected

Adopted: exact content identity; same-content reuse registration; unchanged
lane. Rejected: none in this phase's scope.

## Verdict

```
SEALED
```
