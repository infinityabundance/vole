# Phase H Receipt — Exhaustive vs FixedHeuristic vs DsfbGuided (SEALED)

## Deliverable

Three search strategies over **one shared candidate universe** (Phase G's
whole-frame families), implemented as a strategy plug-in on the Phase-G
encoder (`src/inverse.rs`) plus the non-normative DSFB governor
(`src/dsfb.rs`):

* **Exhaustive** — full per-frame evaluation (the search-quality oracle; this
  remains the default `EncodeOptions.strategy`, so Phase-G behavior and
  streams are unchanged).
* **FixedHeuristic** — a constant, history-free per-frame plan: the
  deterministic sentinels + the full translation window + a fixed small copy
  probe (toroidal wraps and vertical screen scrolls by 1..3).
* **DsfbGuided** — a deterministic trust model: the recent-window winner set
  (the *active* families) receives the full budget for its cheap families;
  COPY_RECT is evaluated as a *probe replay of the previous winner's rect
  ops*; regime detection `α` arms a full broaden frame when the winner leaves
  the active set or the payload slews past 3× the trailing median; a
  deterministic rotating sweep (every 6th frame) re-probes the non-active
  families so a silent regime change is found within a bounded latency. No
  stochastic bandits and no randomness anywhere.

DSFB may only reorder and budget candidate evaluation (§26): every evaluated
candidate is still byte-validated through the normative materializer path by
the Phase-G machinery, the exact final cost comparison among evaluated
candidates is untouched, RAW can never be suppressed, and nothing in
`src/dsfb.rs` is reachable from the decoder.

## Normative format changes

None. Phase H adds encoder-side search only. The one Phase-G refactor folded
the per-object exact-ref library scan into a single content-addressed reset
candidate (identical valid set and costs; reuse detection is now exact via
the BLAKE3 content registry).

## Decision records (§28)

`FrameDecision` now also carries: `search_work` (candidate count + weighted
pixel scans; deterministic) and `dsfb_diag` (winner, payload, active set,
per-family `φ`, drift `ω` EWMA, regime flag `α`, broaden flag, cumulative
evaluated count, frames since regime) under the guided strategy. Oracle
regret is computed per frame against the exhaustive run's winner payloads in
the courts (0 on steady segments; bounded and measured across regime
switches).

## Courts (`tests/phase_h.rs`, 6 tests; all pass, byte-exact end-to-end)

- Steady diagonal pan: `N_dsfb < N_exhaustive` (0.132×) with **byte-identical
  streams** (`J = 1.000`); fixed heuristic also matches oracle bytes on this
  content; per-frame payloads all 26 B.
- Steady wrap-by-7 (outside the fixed probe window): DSFB byte-identical to
  the oracle (`J = 1.000`, `N = 0.048×`) with zero rebases; the fixed
  heuristic rebases **every** frame (`J = 11.5×`, 24 raw declarations) — the
  measured cost of a constant probe set that cannot see the regime.
- Static + blink: `J = 1.000`, `N = 0.073×`, winner families match the oracle
  per frame (unchanged lane, then sparse).
- Regime court (static textured scene → wrap-by-7 → noise → whole-scene pan):
  all streams exact; DSFB total `J = 1.055×` the oracle (bounded, per-switch
  whole-frame rebase penalties only), `N = 0.179×`; guided recovery latency
  per oracle regime switch measured at 0–1 frames (log);
  `raw_rebases(fixed) = 36 > raw_rebases(dsfb) = 15` (the 15 are the noise
  segment's unavoidable RAW frames + one switch frame).
- Diagnostics: `α` armed at regime switches and quiescent on steady tails;
  winner families reach `φ = 1.0` on steady tails; `ω` decays toward 0.
- Determinism: every strategy reproduces byte-identical streams across runs.

## Measured (evidence/campaigns/phase-h-dsfb-1788464563, release)

| court | strategy | vole B | candidates | work | J/oracle | N/oracle | rebases |
|---|---|---|---|---|---|---|---|
| steady pan 64×64 (40f) | exhaustive | 5 201 | 11 038 | 44.9M | 1.000 | 1.000 | 0 |
| | fixed | 5 201 | 1 600 | 6.6M | 1.000 | 0.145 | 0 |
| | dsfb | 5 201 | 1 458 | 6.0M | 1.000 | 0.132 | 0 |
| wrap-by-7 40×32 (25f) | exhaustive | 2 883 | 34 609 | 44.3M | 1.000 | 1.000 | 0 |
| | fixed | 33 171 | 985 | 1.3M | 11.506 | 0.028 | 24 |
| | dsfb | 2 883 | 1 653 | 2.2M | 1.000 | 0.048 | 0 |
| static+blink 32×24 (32f) | exhaustive | 1 486 | 27 464 | 21.1M | 1.000 | 1.000 | 0 |
| | dsfb | 1 486 | 1 997 | 1.5M | 1.000 | 0.073 | 0 |
| regime 32×32 (78f) | exhaustive | 18 240 | 90 661 | 92.8M | 1.000 | 1.000 | 14 |
| | fixed | 40 372 | 3 112 | 3.2M | 2.213 | 0.034 | 36 |
| | dsfb | 19 246 | 16 266 | 16.7M | 1.055 | 0.179 | 15 |

Primary DSFB criterion confirmed: `N_dsfb < N_exhaustive` (0.05–0.18×) while
`J_dsfb == J_exhaustive` byte-for-byte on steady content, and `J_dsfb = 1.055×`
oracle across four consecutive regime changes with 0–1 frame recovery
latency. All streams decode byte-exact.

## Open / recorded (not hidden)

- DSFB's per-frame byte cost can exceed the oracle by a bounded amount when a
  *cheaper* representation exists only inside an unevaluated family and no
  slew/regime signal fires (e.g. a 26 B translation frame where a 14 B
  clear-instances unwind was available): the measured gap is at most one
  small interval per occurrence, self-heals on the following frame, and the
  rotating sweep bounds silent-regime recovery.
- Whole-frame granularity still pays full-raster declarations at raster-origin
  frame 0 and at unavoidable rebases (noise segments): region extraction
  (Phase K) and native ingest (Phase Q) target those costs.
- Search-work is a deterministic sample-scan estimate, not wall time; wall
  time is reported in the log as measured, never asserted.

## Verdict

```
SEALED
```
