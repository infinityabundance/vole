# Transitions

## Time model

Time is a monotonically increasing integer `Interval`. A checkpoint anchors
interval `0`; a transition group advances state to a strictly larger interval.
Materializing each integer interval of a decoded stream yields the canonical
view sequence.

Phase A timeline:

```
checkpoint   G_0   = objects + background + instance placements  (frame 0)
interval  1  G_1   = Φ(G_0, SET_POSITION ...)                    (frame 1)
interval  2  G_2   = Φ(G_1, SET_POSITION ...)                    (frame 2)
...
```

## Phase-A operator language

The transition language is intentionally small but the *infrastructure* is
real and reused by later phases. Supported Phase-A operators, with their wire
tags (`docs/format-v1.md`):

| Transition | Effect | Example |
|---|---|---|
| `DeclareObject` (0x11/0x12 via pre-checkpoint records)/`DeclareFill` | add immutable object to object table | baseline objects only in v1 |
| `CreateInstance` (0x21) | add an instance at a paint position | an object appearing |
| `SetPosition` (0x22) | move an instance to an absolute `(x, y)` | a moving box: `x += 2` |

Because Phase A declares all objects before the single checkpoint, interval
groups in v1 contain only `CreateInstance`/`SetPosition`. Later phases broaden
the operator language; the *replay* architecture (`src/format.rs`,
`src/transition.rs`) is unchanged: each transition is applied forward onto one
deterministic `State`, and references are validated as they are applied
(unknown object/instance → typed error).

## §/Semantics that MUST be deterministic

* positions are absolute; `SetPosition` replaces (`SET_POSITION`), not `+=`,
  in Phase A (delta/parametric trajectories arrive in a later phase);
* apply order inside an interval is the order written, and materialization
  after an interval sees exactly the composed result;
* hostile, typed-everywhere: an interval index that is not strictly
  increasing, a duplicate instance id, or a reference to nothing is rejected
  with a typed error during parse (tests/malformed.rs).

## Replay

A decoder restores the checkpoint state, then applies successive interval
groups. `Decoded::materialize_all` and `Decoder` in `src/decoder.rs` replay a
validated stream (re-applications after parse validation are guaranteed not to
produce errors, but are still checked rather than `unwrap`ed on hostile input).
