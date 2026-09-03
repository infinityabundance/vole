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
```

* `w*h` must not exceed the active `max_object_bytes`.
* Duplicate `obj` ids ⇒ `DuplicateId`.
* After the checkpoint tag, object-declaration bytes are non-canonical
  (`NonCanonicalEncoding`).

### Checkpoint

```
0x03 bg:u8 n:u32  ( iid:u32 oid:u32 x:i32 y:i32 )^n
```

* Instances are in paint order; `oid` must already be declared
  (`UnknownObject`), `iid` unique (`DuplicateId`).
* `n` ≤ `max_instances`.
* Exactly one checkpoint per v1 stream.

### Interval

```
0x04 t:u64 m:u32  Transition^m
```

* `t` must be > the previous interval and > 0 ⇒ else `NonConsecutiveInterval`.
* `m` ≤ `max_transitions_per_interval`.
* Transitions:
  * `0x21 iid:u32 oid:u32 x:i32 y:i32` — create instance.
  * `0x22 iid:u32 x:i32 y:i32` — set instance position (absolute).
  * `0x23 n:u32 (x:i32 y:i32 v:u8)^n` — sparse overlay patch (sorted).
  * `0x24 sx:i32 sy:i32 w:u32 h:u32 dx:i32 dy:i32` — COPY_RECT from the prior
    decoded frame (Phase D machinery).
  * `0x25 sx:i32 sy:i32 w:u32 h:u32 dx:i32 dy:i32` — MOVE_RECT (Phase D).
  * `0x26 iid:u32 vx:i32 vy:i32` — set a persistent integer translation on an
    instance (Phase E).
  * `0x27` — advance every active translation once: `position += (vx, vy)`
    (Phase E).
* `|x|,|y|,|vx|,|vy| ≤ 2^24`; for copy ops `w,h ≠ 0` and `w*h ≤ max_copy_area`
  ⇒ else a typed error.
* Cumulative translation-advance work (`moving_count` summed over every
  `0x27`) is capped by `Limits.max_transition_work` at parse and encode time.

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
