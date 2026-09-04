# Format v2 core wire (`.vole` format_version 2)

This document is the normative, byte-level wire format implemented by
`src/media/wire.rs`, `src/checked.rs`, and `src/integr.rs`. It was **frozen at
the end of Phase V.1.2** (V.1 video programme). The writer is the canonical
emitter; the parser is the hostile-safe reader; this document is the
authoritative description both must satisfy. There is **no generic
serialization** (no serde/bincode/postcard/CBOR…). All multi-byte integers
are **little endian**. The whole crate is `#![forbid(unsafe_code)]`.

The v2 core container carries **one video epoch** and its per-plane procedural
programs. Timeline binding (rational PTS → observation), epoch sequences over
one stream, side data, and the richer inverse families are reserved
extensions of later V.1 subphases; the container grammar here is closed and
frozen as of V.1.2.

## Relationship to format v1

v1 and v2 are separate universes that dispatch on the same prefix:

```
v1: magic "VOLE"  reserved 0  format_version 1  universe_id 1 …
v2: magic "VOLE"  reserved 0  format_version 2  universe_id 2 …
```

Both are permanently supported by the same crate and binary. **No v1 stream
ever acquires v2 interpretation and no v2 stream is ever read as v1.** All A–U
v1 semantics, goldens, and courts are untouched by v2.

## File (top level)

```
File := Header MediaDescriptor PlaneBlock* Integrity
```

Numbering: all fields fixed-width unless noted.

### Header (24 bytes)

| offset | size | field | notes |
|---|---|---|---|
| 0 | 4 | magic `"VOLE"` | ASCII |
| 4 | 1 | reserved | must be `0x00` |
| 5 | 2 | format_version | must equal `2` |
| 7 | 4 | universe_id | must equal `2` (video universe v2) |
| 11 | 1 | limit_profile | must equal `1` (the frozen v1 envelope, applied per plane) |
| 12 | 4 | feature_bits | v2 core: no features defined; every bit must be `0` |
| 16 | 4 | coded width | samples/row of the epoch's luma/canvas geometry |
| 20 | 4 | coded height | rows of the epoch's luma/canvas geometry |

Wrong magic ⇒ `BadMagic`. Wrong version/universe/profile ⇒ `Unsupported*`.
A nonzero feature bit ⇒ `UnsupportedFeature`. A reserved byte ≠ 0 ⇒
`NonCanonicalEncoding`. Width/height over the profile envelope ⇒
`DimensionTooLarge`.

### MediaDescriptor (tag `0x11`)

Declares the epoch's full media interpretation. Length = `19 + 2·p` bytes
where `p` is the layout's plane count.

| field | size | notes |
|---|---|---|
| tag | 1 | `0x11` |
| layout | 2 | canonical layout code (registry below) |
| plane_count | 1 | must equal the layout's plane count |
| per plane `i` | 2 each | `component:u8` (must equal the layout template's component), `depth_bits:u8` (`1..=16`) |
| chroma_location | 1 | code below (`0` = unspecified) |
| primaries | 1 | code below |
| transfer | 1 | code below |
| matrix | 1 | code below |
| range | 1 | code below |
| sar width | 4 | `> 0` |
| sar height | 4 | `> 0` |
| orientation | 1 | code below |
| field_structure | 1 | code below |

`plane_count` mismatch, a component that disagrees with the layout template,
an unknown code, or a zero SAR axis ⇒ typed error (`NonCanonicalEncoding`,
`UnsupportedPixelLayout`, `InvalidSamples`…). Color properties are preserved
exactly; `unspecified` codes mean *unspecified*, never a guessed default.

### PlaneBlock (tag `0x10`)

One plane block per epoch plane, each exactly once (duplicates ⇒
`DuplicateId`, missing ⇒ `NonCanonicalEncoding`). The canonical writer emits
them in ascending plane index order.

```
PlaneBlock := tag:0x10 idx:u8
              background:u32
              objects:count:u32  Object*
              instances:count:u32  Instance*
              overlay:count:u32  OverlayPoint*
              intervals:count:u32  Interval*
```

* `background` must be inside the plane's active depth (`≤ 2^depth_bits − 1`);
  above ⇒ `InvalidSamples`.
* `objects` — the plane's immutable object table. Duplicate object id ⇒
  `DuplicateId`; count over the profile ⇒ `DimensionTooLarge`.

```
Object := id:u32  w:u32  h:u32  kind:u8
kind 0x01 (fill):       v:u32            // uniform fill; v inside active depth
kind 0x02 (raster):     byte_len:u64     // canonical payload bytes, LE, tight
                        payload:byte[byte_len]
```

Raster payload: `byte_len` must equal `w · h · bps` where `bps` = 1 for
depth ≤ 8 and 2 for depth ≥ 9 (`bps` = bytes per stored sample). Depth ≤ 8
payloads are raw `u8` samples; depth ≥ 9 payloads are raw `u16` little-endian
samples in the low bits with **padding high bits zero** (a nonzero padding bit
or a payload whose length disagrees with `w · h · bps` ⇒ `InvalidSamples` /
`NonCanonicalEncoding`). `w, h ≥ 1`; `w · h` within `max_object_bytes`.

```
Instance := id:u32  object:u32  x:i32  y:i32
```

`object` must name a declared object (`UnknownObject`); instance ids must be
unique among the plane's initial instances (`DuplicateId`).

```
OverlayPoint := x:i32  y:i32  v:u32
```

Overlay points are **strictly ascending by `(x, y)`** (⇒ `NonCanonicalEncoding`
otherwise); `v` inside the active depth (`InvalidSamples`); bounds
`|x|, |y| ≤ 2^24`.

```
Interval := t:u64  op_count:u32  Op*
```

* `t ≥ 1` and **strictly ascending** within the plane (`NonConsecutiveInterval`).
  An interval group may have `op_count = 0` (an empty group): the interval's
  fresh state render already is the observation.
* `intervals:count` over `max_checkpoint_distance` ⇒
  `CheckpointOutOfEnvelope`.

### Ops (inside interval groups)

| tag | op | fields |
|---|---|---|
| `0x21` | DeclareObject | `id:u32 w:u32 h:u32 kind:u8` + object payload (as above) |
| `0x22` | CreateInstance | `id:u32 object:u32 x:i32 y:i32` |
| `0x23` | SetPosition | `id:u32 x:i32 y:i32` |
| `0x24` | ClearInstances | — |
| `0x25` | ClearOverlay | — |
| `0x26` | PatchOverlay | `count:u32`, then `count` × `x:i32 y:i32 v:u32` (strictly ascending `(x, y)`) |
| `0x27` | CopyRect | `src_x:i32 src_y:i32 width:u32 height:u32 dst_x:i32 dst_y:i32` |
| `0x28` | Residual | `byte_len:u64` + Phase-F byte container of that length |

* `DeclareObject` inside an interval group is a state transition (immutable
  content; duplicate or reused id ⇒ `DuplicateId`; declaration order is
  irrelevant, ids must exist at use).
* `CreateInstance`/`SetPosition` reference live ids (`UnknownObject`,
  `UnknownInstance`, `DuplicateId` for a live duplicate instance id).
* `ClearInstances` frees every instance id for reuse; `ClearOverlay` clears
  the persistent overlay.
* `CopyRect`: `width, height ≥ 1`, area within `max_copy_area`, coordinates in
  the canonical coordinate domain. The copy reads the plane's **immediately
  previous materialized observation** (snapshot semantics; overlap-safe),
  clipped to both source and destination planes, and writes into the current
  interval's render.
* `Residual`: the block is a Phase-F container (RAW or rANS, byte-oriented,
  depth-agnostic — the sealed v1 coder reused unchanged). Its decoded payload
  is a record list of `(x:i32, y:i32, v:u16)` triples, 10 bytes each: the
  payload length must be a multiple of 10, triples strictly ascending by
  `(x, y)`, every `(x, y)` inside the plane's geometry, and `v` inside the
  active depth — violations are typed (`NonCanonicalEncoding`,
  `InvalidSamples`). Each triple overwrites that sample of the current
  interval's render.
* Unknown op tags ⇒ `NonCanonicalEncoding`.

### Integrity

The last 32 bytes are BLAKE3 over **every preceding byte**. Structural errors
(magic/version/universe/profile/feature/tag/layout/code/ordering/length
violations) surface as their specific typed error during parsing; the digest
is verified after the structure parses, so a content flip that stays
structurally parseable is `IntegrityMismatch`.

## Replay semantics (normative, mirrors v1)

One plane program is a state machine:

```
render(state) = background fill
              + every instance in paint order (fill/raster overwrite, clipped)
              + the persistent overlay (authoritative, above all instances)
```

An interval group at `t` first applies its state transitions in listed order
(`DeclareObject`, `CreateInstance`, `SetPosition`, `ClearInstances`,
`ClearOverlay`, `PatchOverlay`), then renders the state **fresh**, then
applies that interval's canvas ops in listed order (`CopyRect` from the
previous materialized observation, `Residual` overwrite). **Canvas ops are
one-shot: they never persist into later frames** — a later interval starts
again from the fresh state render. An empty interval group therefore
reproduces the state render exactly (unchanged frame).

Observation `idx` of a plane program is the result of replaying every interval
`t ≤ idx`. A multi-plane program materializes observation `idx` as the picture
whose plane `p` is plane `p`'s replay of the same `idx` — intervals are
aligned across planes (each plane steps one group per observation). The
program's observation count is `1 + max over planes of the last interval t`
(a program with no intervals has one observation).

## Registries (codes)

Layouts (`u16`): `1` Gray · `2` YUV400 · `3` YUV420 · `4` YUV422 · `5` YUV444
· `6` YUVA420 · `7` YUVA444 · `8` GBR · `9` GBRA · `10` RGB · `11` BGR · `12`
RGBA · `13` BGRA · `14` ARGB · `15` ABGR · `16` Indexed.

Components (`u8`): `1` Y · `2` Cb · `3` Cr · `4` R · `5` G · `6` B · `7` A ·
`8` Gray · `9` Index.

Chroma location (`u8`): `0` unspecified · `1` center · `2` left · `3` top-left
· `4` top · `5` bottom-left · `6` bottom.

Primaries (`u8`): `0` unspecified · `1` BT.709 · `2` BT.470M · `3` BT.470BG ·
`4` SMPTE 170M · `5` SMPTE 240M · `6` film · `7` BT.2020.

Transfer (`u8`): `0` unspecified · `1` BT.709 · `2` gamma 2.2 · `3` gamma 2.8
· `4` SMPTE 170M · `5` SMPTE 240M · `6` linear · `7` sRGB · `8` BT.2020 10-bit
· `9` BT.2020 12-bit · `10` SMPTE 2084 (PQ) · `11` ARIB STD-B67 (HLG).

Matrix (`u8`): `0` unspecified · `1` identity · `2` BT.709 · `3` SMPTE 170M ·
`4` SMPTE 240M · `5` YCgCo · `6` BT.2020 NCL · `7` BT.2020 CL.

Range (`u8`): `0` unspecified · `1` limited · `2` full.

Orientation (`u8`): `0` normal · `1` rotate 90° · `2` rotate 180° · `3`
rotate 270° · `4` flip horizontal · `5` flip vertical.

Field structure (`u8`): `0` unknown · `1` progressive · `2` interlaced
top-field-first · `3` interlaced bottom-field-first.

Object kinds (`u8`): `1` fill · `2` raster.

Plane/descriptor tags: `0x10` plane block · `0x11` media descriptor.

Op tags: `0x21` … `0x28` (table above).

## Canonical rules (summary)

* Planar, tight, little-endian, **no stride padding**; a plane's canonical
  byte form is `sample_count × bps` bytes.
* Values are in the active depth: `0 ≤ v ≤ 2^depth_bits − 1`; `u16` storage
  keeps the sample in the low bits with zero padding.
* Coordinates and sizes: `|x|, |y| ≤ 2^24`; `w, h ≥ 1` for objects and
  copy rects.
* Strict-ascending orders where required: overlay points, patch-overlay
  points, residual triples, interval times. The residual point list is
  strictly ascending by `(x, y)` (row-major scans are not, and must be
  sorted before encoding).
* Object ids are never reused in a plane program. Instance ids are unique
  among live instances.
* Every count and length is bounded by the frozen limit profile (v1
  envelope applied per plane: `max_width`/`max_height`/`max_canvas_bytes` per
  plane, `max_objects`, `max_object_bytes`, `max_instances`,
  `max_transitions_per_interval`, `max_checkpoint_distance`, `max_copy_area`,
  `max_overlay_points`, `max_residual_bytes`, `max_stream_bytes`). Violations
  are `DimensionTooLarge` or `CheckpointOutOfEnvelope`, never panics.

## Reserved extensions (not in the frozen v2 core container)

* Rational PTS schedule and duration per observation (V.1.1 domain types
  exist in memory; the container layer lands with the epoch-sequence
  container in a later subphase).
* Side data (HDR mastering/CLL, orientation override, timecode): epochs with
  side data cannot be serialized by the v2 core writer and fail typed rather
  than dropping metadata.
* Cross-plane hypotheses, richer families (V.1.4+), and the transport/store/
  archive layers over v2.

## Conformance

* v1 files decode forever under v1 semantics (unchanged, permanently
  supported); v2 never reinterprets v1.
* The frozen golden: the canonical v2 core container of the V.1.2 authored
  specialization scenario (48×32 Gray8, two objects, five `SetPosition`
  intervals) has a fixed payload BLAKE3 of
  `a5c1fb407c8b86604cb7f40227f1956a628061c63e5145d6c39b7d9b0a56a80f`,
  pinned in `tests/phase_v1_2.rs`. Any deliberate grammar change re-freezes
  this document, the golden, and the hostile corpus together.
