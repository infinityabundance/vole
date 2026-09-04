# Phase O Receipt — representation re-optimization, `vole optimize` (SEALED)

## Deliverable

`vole optimize <in.vole> <out.vole>` (and `optimize::optimize_stream`) searches
a decoded stream for **equivalence-preserving representation rewrites** (§44):
a candidate is accepted only when the rebuilt stream is **strictly smaller**
(`J(D1) < J(D0)`) **and** decodes to byte-identical frames with the normative
decoder (`M(D0) == M(D1)`, proven by full decode — never trusted). Rewrites
apply one at a time to a fixpoint (the stream strictly shrinks, so the loop
terminates).

## Rewrite families (deterministic order per iteration)

1. **velocity collapse** — runs of consecutive single-`SetPosition` groups
   with a constant delta (measured from the pre-run position) become one
   `SetVelocity` + one `AdvanceTranslations` per frame: `13 + len` bytes vs
   `13·len`. Strictly cheaper than the trajectory descriptor for pure linear
   runs (by the descriptor difference), and proven safe against every
   multi-instance advance hazard by the decode proof;
2. **trajectory collapse** — the Phase-I parametric pass (accel / piecewise
   runs velocity cannot serve), reused as-is;
3. **residual promotion** — a maximal run of identical one-shot point-residual
   blocks becomes one persistent sparse overlay and the rest of the run rides
   the unchanged lane. This closes the *recorded* Phase-G/K gap "stable
   residuals pay one-shot per frame until Phase O promotes them";
4. **generator substitution** — a declared raster object whose samples are
   exactly a bounded program (deterministic fits + byte-for-byte render
   check) is re-declared as that generator: the declaration stores the
   program, never the samples;
5. **duplicate merge** — byte-identical object declarations share one record;
   checkpoint and interval references are remapped.

## Contract

* `decoded_before_hash == decoded_after_hash` — `OptimizeReport.exact`
  re-checks it at the end of every run;
* never grows (asserted over every court stream);
* palette-bearing streams are preserved verbatim (the rebuild path re-emits
  objects/instances/intervals only; recorded limitation, never a silent
  change);
* noise is never substituted (seed discovery is unbounded search, §21).

## Courts (`tests/phase_o.rs`, 8 tests; all pass, decode-identical)

* velocity collapse on a 12-frame linear run (strictly smaller than the
  trajectory-only fixpoint; second pass is a fixpoint);
* accel runs collapse via trajectory;
* repeated identical residual → promotion to the unchanged lane
  (residual_bytes fall to zero);
* raster gradient/checker objects → generator substitution; noise raster is a
  fixpoint (never substituted);
* duplicate objects merge to one declaration (object bytes fall);
* palette streams preserved verbatim;
* never-grow invariant across earlier-phase stream shapes (inverse-gradient
  encoder output, noise RAW output, affine court, static content);
* hostile/truncated input is typed.

## Measured (evidence/campaigns/phase-o-optimize-1788483022, release)

| court | before | after | saved | rewrites | exact |
|---|---|---|---|---|---|
| 100-frame linear run, 1920×1080 (textured tile) | 22 691 B | 21 504 B | 1 187 B | velocity_collapse | ✓ |
| 40-frame accel run 192×128 | 1 387 B | 695 B | 49.9% | trajectory + generator | ✓ |
| stable 40-point residual × 30 frames | 36 277 B | 856 B | 97.6% | residual_promotion + generator | ✓ |
| full-canvas raster gradient decl | 24 667 B | 101 B | 99.6% | generator_substitution | ✓ |
| eight identical tile declarations | 33 062 B | 213 B | 99.4% | generator + duplicate merge | ✓ |
| inverse-gradient encoder output (negative) | 926 B | 926 B | 0 | — | ✓ |
| noise encoder output (negative) | 24 680 B | 24 680 B | 0 | — | ✓ |

CLI: the Phase-A proof stream (101 per-frame `SetPosition` groups, 2 692 B)
optimizes to **1 505 B** via velocity collapse — exactly the Phase-E
per-frame-velocity baseline, with `vole verify` confirming identical decode.

## Recorded, not hidden

* copy decomposition, checkpoint placement, and per-frame entropy-model
  retuning remain re-encode territory; on the current encoder output the
  court measures them as zero-savings fixpoints (the encoder already emits
  near-optimal representations for the families optimize knows — a measured
  result, not a claim);
* palette streams preserved verbatim (documented).

## Verdict

```
SEALED
```
