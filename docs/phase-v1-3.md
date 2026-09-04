# Phase V.1.3 Receipt — foreign ingest bridge (V.1 video programme, contract
# `docs/phase-v1-video-architecture.md` §2.1/§2.4; master brief §31–§40, §217–§219)
# (SEALED)

## Deliverable

V.1.3 builds the **foreign ingest bridge** over the frozen v2 core wire and
the V.1.2 multiplane core (`src/media/bridge/`): ordinary media files become
verified canonical videos through a bounded, recorded, non-normative FFmpeg
pipeline. NUT/FFmpeg never appear inside `.vole`.

* **Bounded subprocess runner** (`bridge/run.rs`): every foreign invocation is
  a [`std::process::Command`] with individual arguments — never a shell
  command string (§32). Wall-clock and stdout/stderr byte caps are enforced
  and the child is killed cleanly (typed `BridgeTimeout` /
  `BridgeOutputLimit`). `ToolPaths::discover()` resolves ffmpeg/ffprobe
  (`VOLE_FFMPEG`/`VOLE_FFPROBE` overrides, then `PATH`) and records both
  version strings.
* **FFprobe manifest** (`bridge/probe.rs`, §38): deterministic video-stream
  selection (explicit index, else the default-disposition usable video
  stream, attached pictures excluded, ties to the lowest index); typed
  parsing of `-of default=noprint_wrappers=1` output into container facts +
  per-stream geometry/pix-fmt/time-base/field-order/color/orientation
  (`rotate` tags)/SAR; unknown values stay `Unspecified`/`None` with the raw
  strings preserved. The manifest is **evidence**, never a playback
  dependency.
* **Framehash oracle** (`bridge/framehash.rs`, §39): an independent FFmpeg
  decode (`-f framehash`, SHA-256 per frame over the **tight rows** —
  empirically established) producing `frame, dts, pts, duration, size,
  sha256` records in the oracle's own time base.
* **Narrow NUT reader** (`bridge/nut.rs`, `bridge/crc.rs`, §36–§37): a pure,
  hostile-safe parser for exactly the NUT subset VOLE's controlled FFmpeg
  emits — main header (+ delta-encoded frame-code table + elision headers),
  stream headers, info/syncpoint/index packets (skipped with CRC
  verification), and rawvideo frame headers with exact PTS recovery
  (`last_pts + pts_delta` / `lsb2full`, syncpoint `last_pts` resets for long
  streams). Packet CRCs are verified with the MSB-first CRC-32 (poly
  `0x04C11DB7`, init 0) whose stored form is the big-endian bytes of the
  MSB-first value — pinned against captured ffmpeg fixtures. No general
  demuxer; anything outside the emitted subset fails closed typed.
* **Reversible canonicalizer** (`bridge/canonicalize.rs`, §17–§18):
  supported pixel formats → canonical layout/depth + planes. Planar formats
  map plane-for-plane (with declared reorder for `gbrp`/`gbrap`); NV12/NV21/
  P010/P016 semi-planar chroma is de-interleaved; packed RGB families
  (rgb24/bgr24/rgb0/bgr0/0rgb/0bgr/rgba/bgra/argb/abgr) and packed 4:2:2
  (yuyv422/uyvy422, even widths) are de-interleaved. Every format is
  **reversible** (`repack_frame` restores the exact source payload), which is
  how the oracle proof runs for packed formats. Unsupported formats (BE
  planes, pal8, anything else) fail closed typed — never a silent conversion.
  Stride padding is never preserved.
* **In-crate SHA-256** (`bridge/sha256.rs`): integer-only, no dependencies,
  pinned to the FIPS 180-4 vectors, used to verify the oracle digests from
  the native side (VOLE's own digests stay BLAKE3).
* **Import orchestration** (`bridge/mod.rs`, `import_video`): probe → oracle →
  NUT pipe decode → `verify_frames` (per-frame payload size + repacked-byte
  SHA-256 vs the oracle, exact rational PTS equality across the two
  independent decode time bases, frame count) → epoch construction (layout/
  depth from `pix_fmt`, color description, SAR, orientation, field order,
  manifest dims cross-checked against the NUT stream header) → canonical
  observations in presentation order with exact PTS-delta durations (last
  frame duration unknown) → a validated [`CanonicalVideo`] plus the recorded
  manifest, every bridge command as argv, tool versions, and the
  **domain-separated sequence digests** (BLAKE3 + SHA-256 over epoch +
  layout + color + per-observation PTS/duration/plane bytes, §40).
  `verify_frames` is public so recorded evidence can be re-verified offline.

New typed errors (§219): `BridgeNotFound`, `BridgeProbeFailed`,
`BridgeDecodeFailed`, `BridgeTimeout`, `BridgeOutputLimit`,
`CanonicalHashMismatch`. Child processes are bounded (§217); the tool
whitelist is `file` only, so network protocols are refused by default (§218).

## Courts

`tests/phase_v1_3.rs` (8, ffmpeg-gated) + `src/media/bridge` unit tests (15). |
Result
|---|---|
| Planar imports (yuv420p 18×12 odd, yuv420p10le, gray, gray16le, yuv444p, yuv422p, gbrp): every observation byte-exact against authored canonical frames; oracle frames == observations; deterministic sequence digests reproducible across re-imports | PASS |
| Packed/semi-planar imports (rgb24, bgra, nv12, yuyv422): canonical observations repack to the authored source payload byte-for-byte | PASS |
| FFV1 (lossless) Matroska 10-bit: canonical ground truth == authored frames, depth 10 preserved | PASS |
| H.264 MP4 (lossy): oracle-exact, 25/25 verified, reproducible sequence digests, layout/depth/dims from the manifest | PASS |
| VFR timeline: exact PTS deltas preserved; one delta > 2× the modal (the time jump); last-observation duration unknown (never guessed) | PASS |
| Hostile NUT corpus: truncations across the whole file, wrong magic, version/CRC-region flips, payload flips — typed errors, never a panic | PASS |
| Oracle mismatch: a tampered payload byte stays structurally parseable but `verify_frames` fails typed `CanonicalHashMismatch`; a pristine re-parse verifies | PASS |
| Missing tools / junk inputs: `BridgeNotFound` typed on an empty PATH; garbage files fail closed (typed bridge errors), never hang | PASS |
| Import time base == the NUT stream time base; unit courts: bounded runner (timeout/output-cap/missing-tool typed), NUT CRC vs the captured ffmpeg fixture, framehash record parsing, ffprobe splitting + color/field/orientation maps + stream selection, SHA-256 FIPS vectors, unpack/repack round trips across the whole supported table (odd sizes incl.) + padding-bit refusal + unknown-format fail-closed | PASS |
| Full A–U / V.1.1 / V.1.2 regression: dev 341 / all-features 343 / release 343, 0 failures, v1 goldens unchanged | PASS |

## Measured (release, `examples/import_proof.rs`, ffmpeg n9.0.1)

10-row format matrix imported file → canonical → oracle-verified: planar
carriers byte-exact against authored frames, packed/semi-planar byte-exact
through reversible repacking, per-frame oracle SHA-256 verification on every
row; `yuyv422` and `yuv422p` carriers of identical canonical content produce
**identical** sequence digests (cross-carrier consistency). H.264 MP4
160×90, 25 obs, 540 000 canonical sample bytes, reproducible digests across
two imports. FFV1 10-bit lossless round trip **exact** (depth 10 preserved).
Recorded evidence prints the manifest fields, both time bases (NUT 1/51200,
oracle 1/25), and every bridge command as argv. Media → canonical → frozen-v2
`.vole` library path: 6 obs × 10-bit YUV420 = 9 216 B canonical raw → exact
floor + v2 container 10 254 B (0.90× — RAW-class floor on fully-changing
authored content; overhead measured, not hidden), re-parsed and
re-materialized plane-sample-exact.

## Recorded, not hidden

* **NUT rawvideo payloads are tight.** Empirically established (row-copy
  verification against captured fixtures) and relied on by the canonicalizer;
  stride padding is never preserved (§18) and would be rejected as a length
  mismatch, never silently stripped.
* **Framehash muxers hash tight rows** (not aligned linesizes) — verified at
  odd widths, which makes FFmpeg's per-frame SHA-256 a genuine oracle over
  the canonical sample layout (§39). The oracle hashes FFmpeg's own layout,
  so packed formats are proven through byte-exact repacking instead.
* **P010 in a NUT carrier is mislabelled by FFmpeg** (no NUT rawvideo tag;
  ffprobe reports `rgb555le`): VOLE's canonicalizer refuses the wrong label
  typed — recorded; P010 arrives through other carriers in the V.1.19 real-
  media courts.
* **The NUT stream time base is adopted as the canonical tick grid**
  (1/51200 for 25 fps content); the oracle time base (1/25) is recorded and
  every PTS is cross-checked by exact rational equality per frame.
* **The last observation carries no duration** (VFR end is not derivable from
  the frame stream alone) — never guessed; `CanonicalVideo` span queries
  report `None` accordingly.
* **Import is subprocess-based and non-normative** (§31): no ffmpeg-sys/
  libav dependency; NUT appears only on the import/export pipe. Byte caps,
  wall-clock bounds, and the `file`-only protocol whitelist bound hostile
  inputs (§217–§218). The whole-NUT-in-memory bound for V.1.3 courts is
  recorded; streaming ingest arrives with the V.1.17 streaming decoder and
  the V.1.19 real-media import.
* Regression: dev 341 / all-features 343 / release 343 tests, 0 failures
  (was 318/320/320 at the V.1.2 seal); v1 goldens unchanged; the frozen v2
  grammar is untouched (V.1.3 adds no wire surface).

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0) ·
`cargo test` (341, dev) · `cargo test --all-features` (343) ·
`cargo test --release --all-features` (343) · hostile NUT/oracle/junk courts ·
Phase-V.1.3 court · evidence
(`evidence/campaigns/phase-v1-3-import-bridge-…/`) · docs updated
(`empirical-status.md`, `PROJECT_STATE.md`, `CONFORMANCE.md`, README).

## Next

V.1.4 — existing-family generalization: port the sealed v1 representation
families (FILL/RAW/SPARSE/COPY/REGIONS/TRANSLATION/TRAJECTORY/PALETTE/AFFINE/
TRANSFORM/GENERATOR) onto the canonical multiplane domain with the V.1.2
floor as the exact raster-origin basis (brief §247).

## Verdict

```
SEALED
```
