# Format v1 (.vole)

This document is the normative, byte-level wire format implemented by
`src/format.rs`, `src/checked.rs`, and `src/integr.rs`. It is written by the
encoder and read by the decoder; **there is no generic serialization**
(no serde/bincode/postcard/CBOR…). All multi-byte integers are **little
endian**.

## Stream (top-level)

```
File      := Header ObjectDecl* Checkpoint Interval* Integrity
Numbering: all fields fixed-width unless noted.
```

### Header (24 bytes)

| offset | size | field | notes |
|---|---|---|---|
| 0 | 4 | magic `"VOLE"` | ASCII |
| 4 | 1 | reserved | must be `0x00` |
| 5 | 2 | format_version | must equal `1` |
| 7 | 4 | universe_id | must equal `UNIVERSE_V1` |
| 11 | 1 | limit_profile | must equal `1` (only v1 profile) |
| 12 | 4 | feature_bits | must be `0` (fail closed) |
| 16 | 4 | canvas width | samples/row |
| 20 | 4 | canvas height | rows |

Unknown universe/profile/feature/version ⇒ `Unsupported*` typed error.

### Object declarations (before the checkpoint)

```
0x01 obj:u32 w:u32 h:u32 (w*h) gray-sample   // raw raster object
0x02 obj:u32 w:u32 h:u32 v:u8                // uniform fill object
0x05 obj:u32 w:u32 h:u32 (w*h) index-byte    // palette-index raster (Phase J)
0x06 pal:u32 len:u32 (len) entry:u8          // palette-table declaration (Phase J)
```

* `w*h` must not exceed the active `max_object_bytes` (also for index planes).
* `0x05` samples are **palette indices** (not Gray8 samples); they render only
  through a palette bound to the painting instance (see the checkpoint and
  `0x2f`).
* `0x06` initializes mutable palette state before the checkpoint: `len` in
  `1..=max_palette_entries`, `pal` ≠ 0, at most `max_palettes` palettes.
* Duplicate `obj`/`pal` ids ⇒ `DuplicateId`.
* After the checkpoint tag, object/palette-declaration bytes are
  non-canonical (`NonCanonicalEncoding`).

### Checkpoint

```
0x03 bg:u8 n:u32  ( iid:u32 oid:u32 x:i32 y:i32 )^n           // no bindings
0x08 bg:u8 n:u32  ( iid:u32 oid:u32 x:i32 y:i32 pal:u32 )^n   // with palette bindings
```

* Instances are in paint order; `oid` must already be declared
  (`UnknownObject`), `iid` unique (`DuplicateId`).
* `0x08` (Phase J) additionally carries each instance's palette binding
  (`pal = 0` means unbound). A bound palette must already be declared
  (`UnknownPalette`). Streams without any binding use `0x03`; old files never
  contain `0x08`.
* `n` ≤ `max_instances`.
* Exactly one checkpoint per v1 stream.

### Interval

```
0x04 t:u64 m:u32  Transition^m
```

* `t` must be > the previous interval and > 0 ⇒ else `NonConsecutiveInterval`.
* The interval count from the checkpoint is bounded by
  `Limits.max_checkpoint_distance` (`CheckpointOutOfEnvelope`).
* `m` ≤ `max_transitions_per_interval`.
* Transitions:
  * `0x21 iid:u32 oid:u32 x:i32 y:i32` — create instance.
  * `0x22 iid:u32 x:i32 y:i32` — set instance position (absolute).
  * `0x23 n:u32 (x:i32 y:i32 v:u8)^n` — sparse overlay patch (sorted;
    cumulative overlay points bounded by `max_overlay_points`).
  * `0x24 sx:i32 sy:i32 w:u32 h:u32 dx:i32 dy:i32` — COPY_RECT from the prior
    decoded frame (Phase D machinery).
  * `0x25 sx:i32 sy:i32 w:u32 h:u32 dx:i32 dy:i32` — MOVE_RECT (Phase D).
  * `0x26 iid:u32 vx:i32 vy:i32` — set a persistent integer translation on an
    instance (Phase E).
  * `0x27` — advance every active translation once: `position += (vx, vy)`
    (Phase E).
  * `0x28` — clear every live instance; instance ids are freed for reuse
    (Phase G content replacement).
  * `0x29` — clear every persistent overlay point (Phase G).
  * `0x2a len:u32 block` — per-frame residual (Phase G). `block` is a
    Phase-F self-describing payload (`rans::encode_block`: kind u8 + out_len
    u64 + inline model? + payload) whose decoded bytes are a canonical,
    strict-sorted, in-canvas sparse point list `(x:i32 y:i32 v:u8)*` (9 bytes
    per point). `len ≤ max_residual_bytes`; the block is structurally
    validated at parse time and decoded only when the frame it appears in is
    materialized. The residual is a **canvas op** applied in listed order
    after any COPY_RECT/MOVE_RECT: it is one-shot for its frame and never
    mutates persistent state.
  * `0x2b iid:u32 count:u32 seg*` — attach a bounded parametric trajectory
    program to an instance (Phase I). `count == 0` deactivates any active
    trajectory on the instance. Trajectory and translation (`0x26`) state on
    one instance are mutually exclusive (attaching one removes the other).
    Each segment is one of:
    * kind `0x00` (linear): `vx:i32 vy:i32 steps:u64` — the position gains
      `(vx, vy)` per advance for `steps` advances;
    * kind `0x01` (accel): `vx0:i32 vy0:i32 ax:i32 ay:i32 steps:u64` — the
      velocity starts at `(vx0, vy0)` and gains `(ax, ay)` after each of
      `steps` advances (discrete `pos += v; v += a`; closed form
      `Δ(t) = t·v0 + a·t·(t−1)/2`; velocity during advance `k` is `v0 + k·a`).
    Canonicality: `steps ≥ 1`; every signed literal `|·| ≤ 2^24`; an `Accel`
    with `(ax, ay) == (0, 0)` must be written `Linear`; two adjacent
    `Linear` segments with the same velocity must be merged; `count ≤
    max_trajectory_segments`.
  * `0x2c` — apply one advance of every active trajectory program (Phase I):
    each trajectory-carrying instance moves by its current velocity and its
    segment/velocity state updates per `0x2b` semantics; a program whose
    final segment is exhausted deactivates.
  * `0x2d id:u32 len:u32 entries(len)` — replace (or declare) the whole
    palette `id` (Phase J): mutable palette-table state; `len` in
    `1..=max_palette_entries`, `id` ≠ 0, table bounded by `max_palettes`.
  * `0x2e id:u32 count:u32 (idx:u8 v:u8)^count` — patch palette entries
    (Phase J): `idx` strictly ascending, `count ≤ 256`, every `idx` inside
    the palette's current length (`OutOfBounds` otherwise), palette must
    exist (`UnknownPalette`).
  * `0x2f iid:u32 pal:u32` — bind instance `iid` to palette `pal` (Phase J);
    `pal = 0` unbinds. The instance must exist; binding to an undeclared
    palette is `UnknownPalette` (palettes are set before they are bound).
    Palette-index objects painted by the instance resolve through the bound
    palette.
* `|x|,|y|,|vx|,|vy| ≤ 2^24`; for copy ops `w,h ≠ 0` and `w*h ≤ max_copy_area`
  ⇒ else a typed error.
* Cumulative translation-advance work (`moving_count` summed over every
  `0x27`) is capped by `Limits.max_transition_work`; cumulative trajectory
  steps (active programs summed over every `0x2c`, counted before the step is
  applied) are capped by `Limits.max_trajectory_work` — both enforced at parse
  and encode time.
* The whole file is bounded by `Limits.max_stream_bytes`.

State transitions
(create/set/velocity/trajectory/palette/advance/clear/patch/bind) apply in
listed order to procedural state; canvas ops (`0x24`, `0x25`, `0x2a`) do not
touch state — a replay step applies every state transition of the interval
first (in listed order), then materializes the canonical frame, then applies
the interval's canvas ops in listed order (COPY/MOVE read their source from
the immediately previous decoded frame; the residual op is self-contained).

### Integrity

The last 32 bytes equal `BLAKE3` over every preceding byte. The decoder
verifies the digest after structural checks so header-semantic errors surface
precisely; a flipped bit anywhere is caught as `IntegrityMismatch`.

## Canonical rules

* Non-canonical tag, reserved byte, length, or interval ordering is a typed
  error (`NonCanonicalEncoding`, `LengthMismatch`, etc). VOLE rejects the file
  rather than guessing.
* The one active `Limits` come from `limit_profile` (`src/limits.rs`).
* Unknown feature bits fail closed.

## Backwards compatibility

Once frozen, format v1 golden streams decode forever under v1 semantics
(`docs/conformance.md`, `CONFORMANCE.md`); old files are never reinterpreted.
New capability appears in new v-formats.
