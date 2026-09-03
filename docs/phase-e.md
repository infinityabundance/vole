# Phase E Receipt — integer translation (SEALED)

*Correcting the phase order: the master brief's §64 plan places **Phase E
(Integer translation)** between Phase D (2D copy/move) and Phase F (normative
entropy floor). Phase F was previously implemented and sealed out of order due
to an erroneous ladder summary; this receipt seals Phase E in its proper place.
The canonical order is A → B → C → D → **E** → F → G → …*

## Mechanism

Persistent **integer translation** as first-class procedural state: an
instance may carry a translation `(vx, vy)` so that

```
position(t+1) = position(t) + (vx, vy)
```

is applied once per explicit `AdvanceTranslations` transition — *not* as
codec-local block motion and *not* as repeated per-frame coordinate payloads.
State semantics:

* `SetVelocity { id, vx, vy }` sets the persistent translation (removed when
  `(0,0)`); unknown instance → typed error.
* `AdvanceTranslations` applies every active translation once with checked
  arithmetic (one O(instances) pass).
* Translation state lives in `State` (instance id → `(vx, vy)`), is part of
  checkpoint clones/replay, and is bounded by a new cumulative work limit
  `Limits.max_transition_work` enforced in both the encoder validator and the
  byte parser, so hostile streams cannot force unbounded advance work.

Wire format v1 (evolving): transition tags `0x26` (`SetVelocity`) and `0x27`
(`AdvanceTranslations`).

## Courts (`tests/phase_e.rs`, 8 tests; all pass)

* moving object (constant velocity): 101 exact frames, byte-identical to an
  independent painter (interior/trailing-edge sample checks);
* persistent translation stream is strictly smaller than the equivalent
  per-frame `SetPosition` stream, and both decode to the identical sequence;
* camera-like translation (large region, whole-pixel translation);
* static control: zero translation ⇒ all frames identical;
* noise negative control: a translation hypothesis that cannot reproduce the
  target is rejected by the exactness gate (constant trajectory accepted,
  random walk rejected);
* unknown-instance velocity → typed error;
* hostile work budget rejected by both encoder and parser (fast, typed).

## Measured (evidence/campaigns/phase-e-translation-…)

| court | frames | stream | per-frame `SetPosition` | raw all frames |
|---|---|---|---|---|
| moving object (vx=2, vy=1) | 101 | 1 505 B | 2 692 B (1.79×) | 209 433 600 B |
| camera-like (large region) | 201 | 2 905 B | — | 185 241 600 B |
| static control | 101 | identical frames | — | — |

All exact-vs-reference checks pass (`exact=true`). The persistent-translation
representation stores one `SetVelocity` + one tiny `AdvanceTranslations` per
interval — never per-frame coordinate payloads, never frame rasters.

## Adopted / rejected

Adopted: persistent per-instance integer translation + explicit advance;
O(instances) advance; cumulative work budget at encoder and parser.
Rejected: implicit time-driven advance on empty intervals (would corrupt the
sealed Phase-B unchanged lane); floating-point trajectories (Phase I).

## Verdict

```
SEALED
```
