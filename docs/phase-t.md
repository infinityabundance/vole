# Phase T Receipt — archive profile: `.volea` manifests, strong integrity, corruption localization, universe pinning (§67 / Phase-T block of §64) (SEALED)

## Deliverable

`vole_video::archive` (`src/archive.rs`) + the `vole archive` / `vole verify
--archive` CLI. An archive of a standalone `.vole` stream is the stream plus a
**self-describing, self-authenticating manifest** (`.volea` sidecar; manual
canonical wire, magic `VOLEARC1`, schema version 1, trailing BLAKE3 self-seal
— never part of the `.vole` grammar):

* **self-description** — format version, universe binding, limits profile,
  feature bits, pixel format (Gray8), canvas geometry, frame count, stream
  length, whole-stream BLAKE3 digest;
* **record index** — every top-level record (header, object/palette
  declarations, checkpoint incl. the 0x08 binding variant, each interval
  group, integrity trailer) with kind, ordinal, byte offset/length, BLAKE3
  digest, declaration id, and interval time;
* **object hashes** — the immutable BLAKE3 content identity of every declared
  object, by id;
* **checkpoint hash** — BLAKE3 of the stream prefix through the checkpoint
  record (the interval-0 state's canonical bytes);
* **frame hashes** — canonical reconstruction hashes (BLAKE3 of every
  materialized full-frame raster, timeline order): the §67 "expected
  reconstruction hashes" golden oracle — a different representation
  (`vole optimize`) decodes to identical rasters and therefore identical
  frame hashes.

**Layered verification** (`archive::verify`), decode-independent until needed:

1. self-description (raw header fields compared without parsing a corrupt
   file);
2. record digests — byte-level corruption localization with **no raster
   work** (a flipped byte is reported with its exact record: kind, offset,
   interval time);
3. decode + object content identities (a structurally pristine stream always
   parses);
4. deep frame-hash verification — one bounded decode pass, early exit at the
   first divergence.

A corrupted stream is **reported** (status, first bad record) rather than
aborting; grammar-breaking corruption is a typed error. The manifest itself is
hostile input like any other: bounded counts (limits envelope + its own
length), checked arithmetic, self-seal digest; unknown schema versions fail
closed (`UnsupportedFeature`), bad magic is `BadMagic`.

**Long-term universe versioning**: the manifest pins the stream's format
version / universe / limits profile, so a future decoder either reproduces the
same meaning or refuses; a forged manifest is reported with the exact pinned
field. Store-backed (external-object) streams are refused typed at build
(standalone archive = payloads must be in the file); their record structure
still scans.

## Courts (`tests/phase_t.rs`, 14 tests)

| Court | Result |
|---|---|
| Record scan tiles every stream shape (phase-A golden v1, palette/binding, kitchen-sink incl. velocity/trajectory/affine/palette ops/clears/sparse/copy/move/residual/generator, store-backed extern): header(24) + decls + checkpoint + intervals + integrity(32), interval `t` == parsed timeline, counts match, digests deterministic | PASS |
| Manifest wire: canonical roundtrip + encode fixpoint; any byte flip ⇒ `IntegrityMismatch`; truncation typed; schema pinning (v2 ⇒ `UnsupportedFeature`, bad magic ⇒ `BadMagic`); hostile record counts ⇒ `DimensionTooLarge` (bounded) | PASS |
| Pristine build+verify(deep) ⇒ Complete everywhere incl. the golden (101/101 frames, 104/104 records, objects, checkpoint, no divergence) | PASS |
| Corruption localization: header width ⇒ header record + `Width` self-description mismatch; object sample ⇒ that object record; interval `t` byte ⇒ that interval record (offset+t); trailer byte ⇒ integrity record — all with decode failing cleanly at the trailer; grammar-breaking flip (transition-count byte) ⇒ typed error | PASS |
| Cross-stream and representation: same-shape different-content stream ⇒ `StructuralMismatch` at the differing object record; `vole optimize` rewrite ⇒ `StructuralMismatch` while **all frame hashes are identical** (reconstruction oracle) | PASS |
| Self-description pinned (format/universe/profile/features/canvas/pixel/frames/stream digest); forged pinned-universe manifest ⇒ `SelfDescriptionMismatch(Universe)`; untouched manifest ⇒ Complete | PASS |
| Hostile manifest counts bounded; archive orthogonal to partial views (Rect view on an archived stream == whole-frame crop) | PASS |

## Measured (release)

| stream | .vole | .volea | overhead | records | frames | build | structural verify | deep verify |
|---|---|---|---|---|---|---|---|---|
| phase-a moving-rect (1080p ×101) | 2 692 B | 9 768 B | 362.9% | 104 | 101 | 19.4 ms | 0.017 ms | 19.3 ms |
| full-hd-81f | 2 172 B | 7 908 B | 364.1% | 84 | 81 | 15.3 ms | 0.014 ms | 15.4 ms |
| mixed-31f | 7 888 B | 3 416 B | 43.3% | 36 | 31 | 0.23 ms | 0.023 ms | 0.22 ms |
| raster-origin 480×270 ×2 | 129 704 B | 561 B | **0.4%** | 5 | 2 | 0.12 ms | 0.071 ms | 0.10 ms |

Manifest overhead scales with records and frame hashes, not raster bytes —
measured on both ends (363% on tiny procedural streams, 0.4% on raster-
dominated streams), never zeroed. Structural verification is microseconds;
deep verification is exactly one decode pass.

### FFV1 operational comparison (external harness, §57)

`corpus/ffv1-compare.sh` runs FFmpeg's lossless FFV1 **outside the crate**
(normative VOLE never calls external codecs), with a byte-verified lossless
roundtrip and a full receipt (tool version, command, sizes, times). On the
synthetic phase-A moving-rect court (1920×1080 ×101; raw 209 433 600 B):

```
vole stream: 2692 B (procedural state); raw Gray8: 209433600 B
ffmpeg: ffmpeg version n9.0.1
ffv1 stream: 105078 B; decode byte-identical to source raw: yes
ffv1 encode ms: 109; ffv1 decode ms: 74
```

VOLE stores the *state* here (2 692 B), FFV1 stores a lossless *raster*
(105 078 B): on this one synthetic authored court VOLE's `.vole` is 39×
smaller. **No general compression claim against FFV1 is made** (§72): this is
a single authored-procedural court; natural-raster content courts remain the
external baseline harness's domain. The phase measures archive integrity and
localization operationally.

## Recorded, not hidden

* No `.vole` grammar change: the manifest is a sidecar; v1 goldens decode and
  archive unchanged (frozen Phase-A stream: 101 frames, deep-verified).
* Store-backed streams are refused typed at build; scanning them (pure record
  boundaries) still works.
* Grammar-breaking corruption (a flip inside a length/count field) is a typed
  error; content corruption localizes to its exact record — both bounded,
  never a panic.
* The checkpoint digest and whole-stream digest are implied by the record
  digests (records tile the prefix) and are reported as independent signals
  without overriding record-level localization.
* Structural verification is representation-level (record digests); frame
  hashes are the reconstruction oracle used to prove cross-representation
  equivalence and to pin future-decoder conformance.

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0 warnings) ·
`cargo test` (263, dev) · `cargo test --all-features` (265) ·
`cargo test --release --all-features` (265) · hostile courts · Phase-T court ·
FFV1 external harness receipt · evidence
(`evidence/campaigns/phase-t-archive-…/`) · docs updated
(`empirical-status.md`, `CONFORMANCE.md`, `PROJECT_STATE.md`, README).

## Verdict

```
SEALED
```
