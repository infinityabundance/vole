# Phase N Receipt — bounded procedural generators (SEALED)

## Deliverable

A **procedural generator** is an immutable *content program*: an object whose
samples are **computed** at materialization instead of stored. The stream
therefore carries the program — a bounded, deterministic, integer-only
description — never the raster. Four v1 programs:

```text
gradient  v = (base + sx·x + sy·y) mod 256
checker   a when ((x/cell)+(y/cell)) is even, else b        (floor division)
periodic  v = (base + sx·(x mod p) + sy·(y mod p)) mod 256
noise     seeded integer hash of (x, y)                     (splitmix64 finalizer)
```

All arithmetic is integer; generator work is exactly the painted box (the
same class as a raster blit); there is no state and no hidden iteration. The
discipline of §21 holds: nothing is claimed free — a noise seed is
author-only (the source knows it) and **never discovered** by the inverse
encoder (unbounded search must not masquerade as a win; measured negative
control).

## Normative format changes

* object tag `0x07` (procedural generator): `id w h program` where `program`
  is `kind u8` + parameters (gradient/checker 7-10 B, periodic 14 B, noise
  9 B);
* canonical domains: `|sx|,|sy| ≤ 2^24`, `1 ≤ cell, period ≤ 4096`; unknown
  kinds and out-of-domain parameters are typed `NonCanonicalEncoding` at
  parse;
* content identity: BLAKE3 over `0x07 w h program` — identical to the wire
  record, so generator content reuses through the exact-identity registry
  like every other object;
* materialization: `paint_generator` computes samples of the clipped box
  (plain and affine placements both resolve generator content by sampling the
  program at the source coordinate);
* no new `Limits` field (work == painted area, bounded by the existing object
  geometry limits);
* accounting: generator declarations are a `generator_object_bytes`
  sub-bucket of `object_bytes` (the ten buckets still sum exactly).

## Encoder discovery (whole-frame, Phase N)

The inverse encoder probes a small deterministic set of *content-derived*
programs: a gradient fit measured from the origin edges, a checker fit over a
bounded cell lattice `{1,2,4,8,16,32}`, and a periodic-sawtooth fit over a
bounded period lattice `{2,4,…,256}`. Every fit is spot-checked on `O(w+h)`
samples (cheap prefilter, mirroring the row-hash copy prefilter) and then
**validated by rendering the normative field and comparing byte-for-byte** —
a candidate can never win on appearance. An inexact fit is admissible only
as a `generator_residual` candidate whose exact correction is counted (gate:
the fit must explain ≥ 15/16 of the pixels, else RAW is the honest floor).
Noise is never fitted. `FramePlan.generators`: Full / Probe (gradient only,
fixed heuristic and DSFB sweep) / Off; DSFB promotes the family when
`generator` or `generator_residual` wins.

## Courts (`tests/phase_n.rs`, 10 tests + generator unit courts; all pass)

* every kind materializes byte-exact vs structurally different independent
  references (wrapping-`&` arithmetic vs `rem_euclid`);
* generator tiles compose with `SetPosition` motion and affine rotation —
  exact vs independent painters;
* raster-origin discovery: a drifting pure-gradient sequence is explained
  procedurally on every frame (`generator` winners) with the stream far from
  raster-proportional;
* residual closure: a gradient + dust (off the fit rows) is served by
  `generator_residual` when it first appears after a non-explanatory base —
  the exact correction is counted and the frame stays a fraction of RAW;
* noise + wrong-seed negative controls: RAW stays, no generator family ever
  passes its fit; authored seeded noise is a tiny stream (the source knows
  the seed);
* identity: generator content id == BLAKE3 over the wire record; equal
  programs share, differing programs differ;
* hostile wire: out-of-domain slope, unknown kind, oversized box, truncation
  — all typed at parse;
* accounting buckets sum with a generator declaration
  (`generator_object_bytes == object_bytes`, zero stored raster samples);
* strategies: Exhaustive == FixedHeuristic == DSFB byte-identical on
  gradient discovery, `N_dsfb ≤ N_exhaustive`;
* structural detail is never hidden: a moving bar over a gradient backdrop
  always costs real bytes (never free behind the generator).

## Measured (evidence/campaigns/phase-n-generators-1788476951, release)

| court | result | exact |
|---|---|---|
| drifting-gradient flagship 1920×1080, 12 frames | **706 B** total, 35 245× vs raw (24 883 200 B); winners `generator×12` | ✓ |
| authored full-HD frame per kind | 98–105 B (≈ 20 000× vs the 2 073 600 B raster) | ✓ |
| noise negative 192×128 | 24 667 B, winner `raw` (bounded overhead) | ✓ |
| closure 1920×160 (gradient + inverted band) | frame-1 5 860 B (transform floor closes the 2-D band); generator+residual closure wins on the dust court | ✓ |

## Recorded, not hidden

* whole-frame generator discovery only in v1; generic/rectangular fits are
  Phase-O/Q surface;
* behavioral re-measurement: pure wrap-ramp content that Phase M's court used
  to exhibit the transform floor is now explained procedurally (the phase_m
  full-range-ramp assertion was updated to the post-N reality — `generator`
  winners, all frames byte-exact). The transform floor still wins on
  curved/non-generator dense deltas, and Phase-M courts there are unchanged;
* seeded noise is author-only: the flattening tax for noise is
  *structural* (unknowable seed), measured as RAW — this is the §21/§63
  negative control, not a claim about compression.

## Verdict

```
SEALED
```
