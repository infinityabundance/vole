# Phase V.1.1 Receipt — canonical media domain (V.1 video programme, contract
# `docs/phase-v1-video-architecture.md` §2.2–§2.5; master brief §10–§30)
# (SEALED)

## Deliverable

`vole_video::media` (`src/media/`) — the **in-memory media interpretation
layer** of the V.1 video programme. V.1.1 deliberately ships **no wire
grammar** (the v2 byte format is frozen at the end of V.1.2 in
`docs/format-v2.md`) and **no foreign import** (V.1.3): its contract is the
exact, validated, integer-only description of the canonical video domain,
exercised on synthetic canonical vectors.

* **Rational media time** (`media/time.rs`): [`TimeBase`] (`num/den` seconds
  per tick; degenerate bases refused), [`Pts`] (signed, nonzero/negative
  origins), [`Duration`] (positive ticks). Exact checked rescaling with
  pairwise cancellation (i128-bounded), exact cross-base ordering, checked
  addition — no floating point anywhere. CFR helpers cover the standard grid
  (23.976 = 24000/1001 … 120); VFR is a per-observation duration.
* **Component-plane model** (`media/layout.rs`, `media/plane.rs`):
  [`Component`] (Y/Cb/Cr/R/G/B/A/Gray/Index/Other) and the canonical
  [`PixelLayout`] registry (Gray, YUV 4:0:0, 4:2:0, 4:2:2, 4:4:4, YUVA,
  GBR/GBRA, RGB/BGR/RGBA/BGRA/ARGB/ABGR, Indexed) with the **normative ceil
  subsampling rule** `ceil(n / 2^s)` per axis — courted on 1×1, 3×3,
  1919×1079, 1921×1081. Packed **source** layouts (NV12/NV21/P010/P016/
  YUYV422/UYVY422/PAL8) map to canonical planar targets for import (V.1.3).
  [`BitDepth`] 1..=16: ≤ 8 bits → `u8` planes; 9..=16 → `u16` planes,
  little-endian canonical bytes, **active-bit discipline** (padding bits above
  the declared depth are refused). Float sample sources are out of scope until
  integer video is sealed (never silently quantized).
* **Color semantics** (`media/color.rs`): primaries, transfer characteristic
  (incl. PQ/HLG), matrix, range, chroma sample location; `Unspecified` means
  unspecified — never guessed. Standard signaling sets (BT.601/709/2020-PQ/
  2020-HLG/sRGB-full) plus validated HDR static metadata (ST 2086 mastering
  display with declared unit bounds, CEA-861.3 content light level).
* **Picture interpretation** (`media/meta.rs`): orientation, SAR (exact
  display-aspect derivation), field structure (never deinterlaced), and the
  bounded side-data registry (typed mastering/CLL/timecode; bounded opaque;
  unsupported fails closed).
* **Epochs and observations** (`media/epoch.rs`): every
  [`CanonicalVideoObservation`] binds to a [`VideoEpoch`] declaring the full
  interpretation; a change of any declared property is a new epoch, never a
  silent rescale. [`CanonicalVideo`] validates dense epoch ids, per-observation
  plane tables against the epoch, and strict presentation-order PTS.
  Observation storage is planar/tight/LE; canonical byte forms round-trip
  exactly.

New typed errors (`error.rs`): `InvalidTimeBase`, `TimeNotRepresentable`,
`GeometryMismatch`, `InvalidSamples`, `UnsupportedPixelLayout`,
`EpochViolation`. The two-clock separation is normative: the procedural state
machine keeps v1 `Interval` semantics; this module only maps presentation
time to observation.

## Courts

`src/media` unit tests (20) + `tests/phase_v1_1.rs` (9). | Result
|---|---|
| Frame-rate grid exact (23.976/24/25/29.97/30/50/59.94/60/100/120); 1 s == `n/d` ticks per rate; cross-base rescale (23.976 grid → 24 fps grid = 24000 → 24024 ticks) | PASS |
| Ordering exact across bases; 1 s equality across 25/50/23.976 grids; negative timestamps symmetric | PASS |
| Durations positive + checked; same-base addition overflow typed; degenerate time bases typed | PASS |
| Layout registry: every canonical layout's planes and the independent ceil-rule oracle agree on 1×1, 3×3, 1919×1079, 1921×1081; packed source → canonical target mapping total; component labels stable | PASS |
| Bit depths 8/9/10/12/14/16: storage width, active-bit refusal (10-bit sample with bit 10 set), u8/u16 mismatch refusal, LE canonical byte round-trip | PASS |
| VFR per-observation durations at 29.97 with exact ordering and span | PASS |
| Flagship synthetic HDR vector: 24 obs of 10-bit BT.2020/PQ YUV420 1919×1079 at 23.976 with VFR + HDR side data, then an epoch transition to 12-bit 4:4:4 1921×1081 — geometry, storage bytes, span (38 ticks), color signaling, and canonical round-trips all exact | PASS |
| Orientation/SAR/interlace preserved as interpretation (anamorphic interlaced portrait source: coded geometry untouched, display aspect 5/3 exact) | PASS |
| Hostile constructions typed: degenerate time base, zero geometry, invalid depth, zero/negative duration, non-dense epoch ids, geometry/depth-mismatched observations, oversized opaque side data, non-monotonic PTS | PASS |
| Deterministic sweep across layouts × depths × dimensions: every plane count equals the independent oracle; no hidden state | PASS |

## Measured (release, `examples/media_proof.rs`)

Flagship synthetic canonical vector: epoch A `1919×1079 yuv420 10-bit
bt2020/pq/bt2020ncl/limited/center` at `1001/24000` s/tick — plane geometry
`Y 1919×1079, Cb/Cr 960×540` (ceil of the odd picture), observation storage
6 214 802 B (u16 LE); 24 VFR observations then an epoch transition to
`1921×1081 yuv444 12-bit` (12 459 606 B/obs); total canonical sample bytes
**198 993 672** across 28 observations. Timeline accounting exact: 38 ticks
= `19019/12000` s — deliberately not rounded to milliseconds. HDR mastering +
content-light side data preserved typed. Every plane's canonical byte form
round-trips exactly.

## Recorded, not hidden

* V.1.1 is the domain layer only: no `.vole`/`.volea`/transport/store change,
  no CLI, no decoder/encoder change — the entire A–U surface and every golden
  decode unchanged (regression: 306 dev / 308 all-features tests, 0
  failures).
* Chroma geometry is a declared **ceil rule** (every coded sample covered by a
  sample of each plane); odd 4:2:0 dimensions yield chroma larger than half —
  exact and courted, chosen deliberately over floor (a floor rule would leave
  edge luma samples without chroma coverage at presentation).
* `Unspecified` color properties are preserved as unspecified; no inference
  ("HD means BT.709" is forbidden by the contract).
* Float sample sources (F16/F32) and packed-layout *unpacking* are V.1.2/V.1.3
  scope; the registry declares their canonical targets now.
* HDR static-metadata unit conventions (0.00002 chromaticity, 0.0001 cd/m²
  luminance) are declared and validated here and become normative with the v2
  wire grammar (V.1.2).

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0) ·
`cargo test` (306, dev) · `cargo test --all-features` (308) ·
`cargo test --release --all-features` (308) · hostile constructions typed ·
Phase-V.1.1 court · evidence
(`evidence/campaigns/phase-v1-1-media-domain-…/`) · docs updated
(`empirical-status.md`, `PROJECT_STATE.md`).

## Next

V.1.2 — multiplane core: generalize the sealed v1 materializer/encoder
families from Gray8 to the canonical plane model (YUV444/420/RGB 8- and
10-bit first), keeping v1 behavior as an exact specialization, then freeze
`docs/format-v2.md`.

## Verdict

```
SEALED
```
