# Format v2 core wire (`.vole` format_version 2)

This document is the normative, byte-level wire format implemented by
`src/media/wire.rs`, `src/checked.rs`, and `src/integr.rs`. The v2 grammar was
**frozen at the end of Phase V.1.2** (V.1 video programme) and **deliberately
extended and re-frozen at the end of Phase V.1.4** (feature bit `0x1`, the
*family extension*: palette-index and generator object content, per-instance
velocity/trajectory/affine state and palette bindings — both as interval ops
and as the initial-state tail — and the transform-coded residual op) and
**again at the end of Phase V.1.5** (feature bit `0x2`, the *global-motion
extension*: the `GlobalPredict` canvas op below). Both extensions are
additive: files without a bit keep their exact earlier byte meaning, and the
V.1.2 and V.1.4 goldens stay pinned.
The writer is the canonical emitter; the parser is the hostile-safe reader;
this document is the authoritative description both must satisfy. There is
**no generic serialization** (no serde/bincode/postcard/CBOR…). All multi-byte
integers are **little endian**. The whole crate is `#![forbid(unsafe_code)]`.

The v2 core container carries **one video epoch** and its per-plane procedural
programs. Timeline binding (rational PTS → observation), epoch sequences over
one stream, and side data remain reserved extensions of later V.1 subphases.

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
| 12 | 4 | feature_bits | bit `0x1` (family extension) declares the V.1.4 surface; bit `0x2` (global-motion extension) declares the V.1.5 surface; every other bit must be `0` |
| 16 | 4 | coded width | samples/row of the epoch's luma/canvas geometry |
| 20 | 4 | coded height | rows of the epoch's luma/canvas geometry |

Wrong magic ⇒ `BadMagic`. Wrong version/universe/profile ⇒ `Unsupported*`.
A nonzero feature bit outside `0x1 | 0x2` ⇒ `UnsupportedFeature`. A reserved
byte ≠ 0 ⇒ `NonCanonicalEncoding`. Width/height over the profile envelope ⇒
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
              [family-extension tail, only when feature bit 0x1 is set]
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
kind 0x03 (index):      byte_len:u64     // palette-index content (family
                        payload:byte[byte_len]  extension): tight row-major
                                              // **indices**, one byte per
                                              // sample; byte_len == w·h
kind 0x04 (generator):  program           // depth-aware procedural program
                                              // (family extension); params in
                                              // the active depth
```

Raster payload: `byte_len` must equal `w · h · bps` where `bps` = 1 for
depth ≤ 8 and 2 for depth ≥ 9 (`bps` = bytes per stored sample). Depth ≤ 8
payloads are raw `u8` samples; depth ≥ 9 payloads are raw `u16` little-endian
samples in the low bits with **padding high bits zero** (a nonzero padding bit
or a payload whose length disagrees with `w · h · bps` ⇒ `InvalidSamples` /
`NonCanonicalEncoding`). `w, h ≥ 1`; `w · h` within `max_object_bytes`.

Index payload: `byte_len` must equal `w · h` (one index byte per sample).
Indices are bounded by the frozen `max_palette_entries` (= 256) at render
time; the stored bytes are indices, not samples, so they are independent of
the plane depth.

Generator `program` (family extension): `kind:u8` + parameters:

```
kind 0x00 (gradient): base:u32 sx:i32 sy:i32          // v = (base + sx·x + sy·y) mod (max+1)
kind 0x01 (checker):  a:u32 b:u32 cell:u32            // cell 1..=4096; a,b in the depth
kind 0x02 (periodic): base:u32 sx:i32 sy:i32 period:u32
kind 0x03 (noise):    seed:u64
```

Generator value parameters (`base`, `a`, `b`) must lie inside the active
depth (`InvalidSamples` otherwise); slope coefficients in `±2^24`; cell and
period in `1..=4096`; an unknown program kind ⇒ `NonCanonicalEncoding`. At
depth 8 the programs reproduce the sealed v1 Phase-N generators byte-for-byte
(mod-256 arithmetic is the depth-8 sample domain).

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
| `0x29` | SetVelocity | `id:u32 vx:i32 vy:i32` (family extension) |
| `0x2A` | AdvanceTranslations | — (family extension) |
| `0x2B` | SetTrajectory | `id:u32 count:u32` + trajectory segments (family extension) |
| `0x2C` | AdvanceTrajectories | — (family extension) |
| `0x2D` | SetPalette | `id:u32 count:u32` + `entry:u32[count]` (family extension) |
| `0x2E` | PatchPalette | `id:u32 count:u32` + `count` × `(idx:u32 entry:u32)` (family extension; strictly ascending `idx`) |
| `0x2F` | BindPalette | `instance:u32 palette:u32` (family extension) |
| `0x30` | SetAffine | `id:u32 a:i32 b:i32 c:i32 d:i32 e:i32 f:i32` (family extension; Q8) |
| `0x31` | TransformResidual | `byte_len:u64` + transform block (family extension) |
| `0x32` | GlobalPredict | `shift:u8 a:i32 b:i32 c:i32 d:i32 e:i32 f:i32` (global-motion extension; see below) |

* `DeclareObject` inside an interval group is a state transition (immutable
  content; duplicate or reused id ⇒ `DuplicateId`; declaration order is
  irrelevant, ids must exist at use). Index and generator content kinds are
  allowed only under the family feature bit.
* `CreateInstance`/`SetPosition` reference live ids (`UnknownObject`,
  `UnknownInstance`, `DuplicateId` for a live duplicate instance id).
* `ClearInstances` frees every instance id for reuse **and kills the motion /
  binding state that dies with its instances** (velocity, trajectory, affine,
  palette bindings); `ClearOverlay` clears the persistent overlay. Palettes
  persist across instance clears (v1 Phase-J semantics).
* Family-extension ops (`0x29`–`0x31`) are state transitions except
  `TransformResidual`, which is a canvas op (see below). Motion kinds on one
  instance are mutually exclusive (velocity / trajectory / affine — attaching
  one removes the others, mirroring v1 Phase-E/I/L). `SetVelocity (0,0)`,
  `SetTrajectory` with an empty program, `BindPalette` to palette `0`, and
  `SetAffine` identity deactivate their state. Palette id `0` is reserved
  (`NONE`); `SetPalette`/`PatchPalette` entries lie inside the active depth;
  patch indices are strictly ascending and inside the palette's current
  length (`UnknownPalette` / `OutOfBounds` typed at apply).
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
* `TransformResidual` (family extension): the block is the Phase-M kind-2
  transform container (first byte `2` = `KIND_TSF`, second byte the transform
  id `0` = the 4×4 lifting DCT), masking coded aligned 4×4 blocks; the decoder
  inverse-transforms each coded block and **adds** the reconstructed samples
  to the current render (a result outside the active depth is typed
  `OutOfBounds`). The container framing, mask padding, and coefficient counts
  are canonical-checked like the sealed v1 phase-M block.
* `GlobalPredict` (global-motion extension): predicts the whole plane from the
  plane's **immediately previous materialized observation** through a
  canonical fixed-point map — destination `(x, y)` samples the previous plane
  at `((a·x + b·y + c) >> shift, (d·x + e·y + f) >> shift)` with signed floor
  division and `shift` from the map-shift registry (`8` = Q8, the sealed v1
  precision; `12` = Q12; `16` = Q16). The source sample is painted only when
  it lies inside the previous plane; otherwise the destination keeps the
  current interval's render (the `CopyRect` clip rule). Coefficients are in
  the canonical `±2^24` domain; a `shift` outside the registry or an
  out-of-domain coefficient ⇒ `NonCanonicalEncoding`. A per-materialization
  work budget (`Limits.max_motion_work`) caps how many whole-plane warps one
  materialization may execute (⇒ `MaterializationBudgetExceeded`). The
  identity map is a whole-plane hold of the previous observation (valid; the
  map arithmetic is the sealed v1 Phase-L integer rule at the declared
  precision — never floating point).
* Unknown op tags ⇒ `NonCanonicalEncoding`. Family-extension tags and content
  kinds without the feature bit ⇒ `NonCanonicalEncoding` (they may only be
  used when bit `0x1` is declared); `GlobalPredict` without bit `0x2` ⇒
  `NonCanonicalEncoding`.

### Family-extension tail (feature bit `0x1`)

When the feature bit is set, every `PlaneBlock` ends (after `intervals`) with:

```
palette_table:count:u32  PaletteRecord*   // strictly ascending id; id ≠ 0
PaletteRecord := id:u32 entry_count:u32 entry:u32[entry_count]
                                   // entry_count 1..=max_palette_entries;
                                   // entries inside the active depth
motion:count:u32  MotionRecord*   // strictly ascending instance id
MotionRecord := instance:u32 kind:u8 payload
kind 0x01 (velocity):   vx:i32 vy:i32          // never (0,0)
kind 0x02 (trajectory): seg_count:u32 segments  // seg_count ≥ 1; segments as
                                                // in SetTrajectory
kind 0x03 (affine):     a:i32 b:i32 c:i32 d:i32 e:i32 f:i32  // never identity
kind 0x04 (binding):    palette:u32             // palette id ≠ 0, must exist
```

Trajectory segment wire (used by `SetTrajectory` and the tail):

```
Segment := kind:u8
kind 0x00 (linear): vx:i32 vy:i32 steps:u64
kind 0x01 (accel):  vx0:i32 vy0:i32 ax:i32 ay:i32 steps:u64
```

Canonical rules (mirroring the sealed v1 forms): `steps ≥ 1`, velocities /
accelerations in `±2^24`, an `Accel` with `(ax, ay) == (0,0)` is rejected
(that is a constant velocity and must be `Linear`), segment count within
`max_trajectory_segments`, and two adjacent `Linear` segments with the same
velocity are non-canonical. Tail records reference initial instances (`Unknown
Instance` if absent, `DuplicateId` if repeated); binding records reference
declared palette-table ids (`UnknownPalette`). Violations are typed.

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
              + every instance in paint order, clipped:
                  fill ............ paint the value over the box
                  raster .......... blit the stored samples
                  index ........... resolve each index through the palette
                                   bound to the instance (binding/palette
                                   missing ⇒ UnknownPalette; index at or
                                   beyond the palette length ⇒ OutOfBounds)
                  generator ....... compute each sample of the box from the
                                   depth-aware integer program
                an instance with an affine placement instead scans the plane
                through the canonical Q8 source map `(su,sv) = (a·x+b·y+c,
                d·x+e·y+f) >> 8` and paints the sampled object content when
                the source lies inside the object rectangle
              + the persistent overlay (authoritative, above all instances)
```

An interval group at `t` first applies its state transitions in listed order
(`DeclareObject`, `CreateInstance`, `SetPosition`, `ClearInstances`,
`ClearOverlay`, `PatchOverlay`, and the family-extension transitions
`SetVelocity` / `AdvanceTranslations` / `SetTrajectory` /
`AdvanceTrajectories` / `SetPalette` / `PatchPalette` / `BindPalette` /
`SetAffine`), then renders the state **fresh**, then applies that interval's
canvas ops in listed order (`CopyRect` and `GlobalPredict` from the previous
materialized observation — snapshot semantics, dependency depth 1, out-of-
bounds sources keep the fresh render — and the `Residual` overwrite /
`TransformResidual` additive blocks). **Canvas ops are one-shot: they never
persist into later frames** — a later interval starts again from the fresh
state render. An empty interval group therefore reproduces the state render
exactly (unchanged frame).

Velocity semantics (v1 Phase-E mirror): a `SetVelocity (vx, vy)` instance gains
`(vx, vy)` once per `AdvanceTranslations`; `(0,0)` deactivates. Trajectory
semantics (v1 Phase-I mirror): a `SetTrajectory` program steps once per
`AdvanceTrajectories` — position advances by the current velocity, an `Accel`
segment adds `(ax, ay)` to its velocity after each advance, an exhausted
segment moves to the next, and a finished program deactivates. Motion kinds
are mutually exclusive per instance and die with their instance on
`ClearInstances`.

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

Object kinds (`u8`): `1` fill · `2` raster · `3` index (family extension) ·
`4` generator (family extension).

Plane/descriptor tags: `0x10` plane block · `0x11` media descriptor.

Op tags: `0x21` … `0x28` (core) · `0x29` … `0x31` (family extension, table
above) · `0x32` (global-motion extension).

Trajectory segment kinds: `0x00` linear · `0x01` accel.

Tail motion kinds: `0x01` velocity · `0x02` trajectory · `0x03` affine ·
`0x04` binding.

Feature bits: `0x1` family extension (all V.1.4 surface) · `0x2`
global-motion extension (the V.1.5 `GlobalPredict` op).

Map-shift registry (`u8`, `GlobalPredict`): `8` Q8 · `12` Q12 · `16` Q16.

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
* `GlobalPredict` maps are canonical fixed-point records: `shift` in the
  registry and every coefficient in `±2^24` (the sealed v1 domain, shared
  with the Q8 affine coefficients). The map arithmetic is checked
  (overflow is a typed error, never a wrap).
* Object ids are never reused in a plane program. Instance ids are unique
  among live instances.
* Every count and length is bounded by the frozen limit profile (v1
  envelope applied per plane: `max_width`/`max_height`/`max_canvas_bytes` per
  plane, `max_objects`, `max_object_bytes`, `max_instances`,
  `max_transitions_per_interval`, `max_checkpoint_distance`, `max_copy_area`,
  `max_overlay_points`, `max_residual_bytes`, `max_palettes`,
  `max_palette_entries`, `max_trajectory_segments`, `max_affine_work`,
  `max_stream_bytes`). Violations are `DimensionTooLarge` or
  `CheckpointOutOfEnvelope`, never panics.

## Reserved extensions (not in the v2 core container)

* Rational PTS schedule and duration per observation (V.1.1 domain types
  exist in memory; the container layer lands with the epoch-sequence
  container in a later subphase).
* Side data (HDR mastering/CLL, orientation override, timecode): epochs with
  side data cannot be serialized by the v2 core writer and fail typed rather
  than dropping metadata.
* The transport/store/archive layers over v2, subpixel / local-motion
  descriptor families and their shared cross-plane geometry (V.1.6–V.1.10
  entry-gated), and the hierarchical inverse DAG over the family surface
  (V.1.11+).

## Conformance

* v1 files decode forever under v1 semantics (unchanged, permanently
  supported); v2 never reinterprets v1.
* v2 files without feature bits keep their exact earlier byte meaning and
  parse identically (the V.1.2 authored-specialization golden below is
  unchanged; the V.1.4 extension golden is unchanged). Any deliberate grammar
  change re-freezes this document, the goldens, and the hostile corpus
  together.
* Frozen goldens: the canonical v2 core container of the V.1.2 authored
  specialization scenario (48×32 Gray8, two objects, five `SetPosition`
  intervals) has a fixed payload BLAKE3 of
  `a5c1fb407c8b86604cb7f40227f1956a628061c63e5145d6c39b7d9b0a56a80f`,
  pinned in `tests/phase_v1_2.rs`; the canonical V.1.4 extension scenario
  (16×12 Gray8 palette-index content with an initial binding and a palette
  patch interval) is pinned in `tests/phase_v1_4.rs` at
  `55c7f4cce95c19f5d326a5bb084e058f3b6ba06ebe288656c78253d00d625007`; the
  canonical V.1.5 extension scenario (16×12 Gray8, a Q16 `GlobalPredict`
  translation plus a sparse residual strip) is pinned in
  `tests/phase_v1_5.rs` at
  `2791d62289d601a59ce0d1f0884738a6f4d939657cc438666f0a72500ecbbae9`.
  None of the files is ever reinterpreted.
