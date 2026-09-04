# Phase V.1.2 Receipt — multiplane core + frozen v2 core wire (V.1 video
# programme, contract `docs/phase-v1-video-architecture.md` §2.4–§2.6, §2.1;
# master brief §45–§48, §246–§247)
# (SEALED)

## Deliverable

V.1.2 generalizes the sealed v1 core (object table, instance painting,
background, persistent overlay, checkpoint/interval replay, COPY/RESIDUAL
canvas ops) from Gray8 to the canonical **component-plane model** of V.1.1,
and **freezes the v2 core wire grammar** in `docs/format-v2.md`.

* **Sample-domain pictures** (`media/picture.rs`): [`Picture`] — one epoch's
  ordered canonical planes with depth-validated `u32`-domain access
  (`get`/`put`/`fill_rect_clipped`/`blit`), validation against the epoch
  plane table, and canonical byte accounting.
* **Independent-plane procedural core** (`media/core.rs`): [`PlaneObject`]
  (fill/whole-raster), [`PlaneInstance`], [`PlaneOp`] (DeclareObject /
  CreateInstance / SetPosition / ClearInstances / ClearOverlay /
  PatchOverlay / CopyRect / Residual), [`PlaneProgram`], [`PlaneState`],
  `render_plane`, `materialize_plane`, [`MultiPlaneProgram`] (one epoch +
  one program per plane, aligned intervals), `materialize_all_observations`,
  `encode_plane_residual` (strict-sorted `(x, y, v)` → Phase-F block). Written
  as an **independent implementation** (no shared blit/paint with the v1
  materializer), so the v1 Gray8 specialization court is a real oracle, not a
  self-check. All normative arithmetic is integer; values live in the active
  depth; every limit is the frozen v1 [`Limits`] envelope applied per plane.
* **Replay semantics mirror v1 exactly**, generalized to the sample domain:
  an interval group applies its state transitions, renders the persistent
  state fresh (background → instances in paint order → authoritative overlay),
  then applies that interval's canvas ops (COPY_RECT reads the immediately
  previous materialized observation with snapshot semantics; RESIDUAL is a
  self-contained strict-sorted sparse overwrite in the plane's active domain).
  **Canvas ops are one-shot — they never persist into later frames.** Empty
  interval groups are legal and reproduce the state render (the unchanged
  lane). Object declarations may appear in interval groups; ids are never
  reused.
* **Exact raster-origin ingest floor** (`media/ingest.rs`,
  `encode_pictures_exact`): frame 0 → background (uniform) or one whole-plane
  RAW raster object; every later observation → one aligned interval group per
  plane: empty (unchanged against the committed state render), a
  strict-sorted residual of the samples differing from that committed render,
  or a full content replacement (fresh object id + clear + create). The
  residual basis is the **committed state render** (the base a canvas op is
  applied over), not the previous materialized observation — the v1-mirror
  one-shot semantics, discovered and fixed during V.1.2 (see Recorded). When
  following observations repeat the current one, the cheaper exact
  description over the observed run wins: one state sync + empty groups
  versus re-emitting the same residual per frame (complete-byte cost, no
  semantics change). The encoder **proves** its output: every observation is
  re-materialized and compared sample-for-sample per plane before the program
  is returned.
* **Frozen v2 core wire** (`media/wire.rs`, `docs/format-v2.md`):
  `Header (24 B, v1-prefix-shaped: magic/reserved/format_version=2/
  universe_id=2/limit_profile=1/feature_bits=0/width/height) +
  MediaDescriptor (0x11: layout, per-plane component+depth table, chroma
  location, primaries, transfer, matrix, range, SAR, orientation, field) +
  PlaneBlock (0x10, one per plane: background, objects, instances, overlay,
  intervals of ops) + BLAKE3 Integrity`. Registry codes, canonical rules,
  and the reserved-extension surface are spelled out in the format document;
  the v2 payload BLAKE3 of the authored specialization scenario is pinned as
  the frozen golden (`a5c1fb40…6a56a80f`) in the test suite. Structural
  errors surface typed before the digest; content flips that stay
  structurally parseable are `IntegrityMismatch`.
* Historical Gray8 v1 behavior is an **exact specialization**: v2 at depth 8,
  single Gray plane, mirrors the v1 decoder byte-for-byte. v1 parsing,
  decoding, materialization, goldens, and every A–U court are unchanged;
  v1 files decode forever under v1 semantics and never acquire v2 meaning.

## Courts

`tests/phase_v1_2.rs` (10) + `src/media` unit tests (incl. `picture.rs`). |
Result
|---|---|
| v1 specialization oracle: the v2 Gray8 core at depth 8 reproduces the authoritative v1 decoder byte-for-byte over a 6-frame authored scenario (fill + raster sprites + SetPosition + sparse overlay + COPY_RECT + residual) | PASS |
| Authored 10-bit 4:2:0 (moving fill + overlay point on Y; static chroma) equals an independently written per-plane compositor frame by frame | PASS |
| Exact raster floor over a multiplexed 10-bit 4:2:0 sprite sequence + 3 static duplicates: sample-for-sample exact at the API and through the encoder's own proof; static duplicates stop repeating raster bytes (wire < raw/2) | PASS |
| Noise floor: cryptographic-style per-frame noise stays exact, terminates on the RAW lane, never pathological | PASS |
| v2 wire roundtrips byte-exactly across 11 layout×depth rows (Gray 8/10/16 incl. odd 9×7, YUV420 8/10, YUV444 8/12, GBR 8, RGB 10, RGBA 8 on odd 7×5, YUVA444 8) with canonical fixpoint `write∘parse == id` and materialization equality | PASS |
| Hostile typed corpus across 8 layout×depth rows: content flips → `IntegrityMismatch`; wrong magic/layout code → typed structural errors before the digest; truncations typed, never a panic | PASS |
| Unknown feature bits and a v1 version number on a v2 body fail closed typed | PASS |
| Frozen v2 golden digest pinned (grammar-change tripwire) | PASS |
| Programs bind to rational PTS and epoch transitions inside `CanonicalVideo` (Gray8 → YUV420 10-bit mid-timeline, exact span) | PASS |
| Picture unit courts: depth-exact u32 sample domain, put/out-of-bounds/depth refusal typed, clipped fill/blit semantics, epoch validation | PASS |
| Full A–U regression: v1 streams/goldens byte-identical, every earlier-phase court green | PASS |

## Measured (release, `examples/multiplex_proof.rs`)

Multiplexed 10-bit 4:2:0 sprite timeline (24×16, chroma 12×8, 8 observations:
6 moving frames + 2 static duplicates): the Y plane expresses drift from its
committed state render as 4 residual groups, syncs its state once at the
static-run start (1 replacement), and then rides 2 empty groups; the chroma
planes ride **7 empty groups** (never changed). Every one of **9 216**
canonical sample bytes re-materializes identically through the fresh program
*and* through its re-parse; wire **3 204 B** vs **9 216 B** raw (2.88×).
Layout×depth matrix (12 rows, 5 observations each = uniform + ramp + 3 static
duplicates): every row exact floor → wire → parse → materialize with
canonical fixpoint; wire/raw from 1.06× (RGBA 7×5, container-dominated) to
3.94× (YUV422 10-bit 24×16). RAW negative control (3-obs YUV420-8 noise):
floor terminates at the RAW lane, wire 1 434 B vs 864 B raw (1.66× bounded
overhead; no invented structure). Gray8 specialization pairing: identical
authored 48×32 content as a v1 `.vole` (287 B) and as a v2 Gray8 core
container (330 B, which carries the epoch media descriptor).

## Recorded, not hidden

* **One-shot canvas ops are the v1 mirror semantics.** A residual/COPY at
  interval `t` applies over the *fresh state render* of that interval and does
  not persist; the floor's first draft computed residuals against the previous
  materialized observation, which is exact only while state changes every
  interval. Fixed by tracking the committed state render per plane; the sprite
  court (a moving box over a static textured background) caught it at frame 2.
* **Raster payload length is canonical bytes, not sample count.** The v2
  writer originally emitted `u16` sample counts where the parser read byte
  lengths, silently misaligning every depth ≥ 9 object. Fixed to
  `byte_len == w·h·bps` with a strict equality check on both sides; the u16
  hostile corpus rows now pass.
* **Empty interval groups are the unchanged lane.** A plane whose observation
  equals its committed state render emits an empty group (12 B/plane/
  observation) — the v2 analogue of v1's zero-transition interval; aligned
  per-plane interval groups give `observation_count == observations` exactly.
* **Static-tail economy.** Because residuals are one-shot, a settled run
  after drift is served by one state sync + empty groups instead of re-emitting
  the same residual per frame; the floor chooses between those two exact
  descriptions by complete bytes over the observed identical run (no lookahead
  semantics, no output change — the final proof validates exactness).
* **The floor is a correctness-first floor.** Its economy is bounded
  (RAW-class worst case: per-observation cost ≤ full replacement + overhead,
  empty groups for unchanged lanes, sparse residuals for small drift); the
  deeper inverse families (translation, regions, generators, palette…) are
  V.1.4+ work per the brief's §247 ordering. The floor's matrix rows at
  container-dominated sizes (tiny pictures, 2-observation sequences) are
  reported as measured — the one-time container cost is not hidden.
* v2 core wire does not yet serialize side data or the rational PTS schedule;
  epochs carrying side data fail typed rather than silently dropping
  metadata (reserved extensions, recorded in `docs/format-v2.md`).
* Regression: dev 318 / all-features 320 / release 320 tests, 0 failures
  (was 306/308 at the V.1.1 seal); v1 `.vole` goldens unchanged.

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0) ·
`cargo test` (318, dev) · `cargo test --all-features` (320) ·
`cargo test --release --all-features` (320) · hostile typed corpus ·
Phase-V.1.2 courts · frozen v2 golden · evidence
(`evidence/campaigns/phase-v1-2-multiplane-core-…/`) · docs updated
(`format-v2.md` frozen, `empirical-status.md`, `PROJECT_STATE.md`,
`CONFORMANCE.md`, README).

## Next

V.1.3 — the foreign ingest bridge (ffprobe manifest, framehash oracle,
FFmpeg subprocess runner, narrow NUT reader, canonicalizer), over the frozen
v2 core container and the V.1.2 multiplane core.

## Verdict

```
SEALED
```
