# Phase Q Receipt — native procedural ingest API and the §55 preservation court (SEALED)

## Deliverable

1. **`vole_video::ingest::Ingest`** (`src/ingest.rs`, §39 / §3.1): a typed
   session for applications that already possess procedural state — immutable
   objects (fill / raster / palette-index / generator), pre-checkpoint palette
   tables, checkpoint instances (optionally palette-bound), and a timeline of
   interval groups at explicit absolute times (`at(t)`). Transition helpers
   mirror the full v1 op surface. `finish()` re-validates everything through
   the normative encoder (`encode_stream` / `encode_palette_stream`, plus a
   `Limits.check_canvas` geometry guard), so an `Ingest` stream is
   **byte-canonical by construction**; misuse is typed, never a malformed
   file. **No wire-format change.**
2. **`vole_video::script`** (`src/script.rs`, §53): the deterministic
   research-harness text format (canvas / background / object
   fill|gradient|checker|periodic|noise|raster|index / palette / instance
   [palette] / at + ops). Parses to an `Ingest`; malformed scripts are typed
   `VoleError::ScriptParse`. Not normative; never part of the `.vole` wire.
3. **The §55 native-procedural preservation court** (`tests/phase_q.rs`,
   `examples/ingest_proof.rs`): the same authored state is (A) ingested
   directly and (B) rasterized (normative materializer) then re-proceduralized
   by the exhaustive inverse encoder; both legs must decode to the **same
   canonical raster sequence byte-for-byte**. The flattening tax `B/A`
   (total and per-interval marginal) is measured, never assumed.

## Courts

* API: `Ingest` output == descriptor-encoder output byte-for-byte (plain and
  palette paths); velocity/advance/copy/move/sparse/clear/residual ops
  round-trip with exact pixel expectations (move clears its source rect);
  misuse typed — duplicate object/instance ids (`DuplicateId`), `at(0)` and
  decreasing times (`NonConsecutiveInterval`), push without an open interval
  (`InvalidStatePhase`), out-of-domain coordinates (`NonCanonicalEncoding`),
  empty/oversized palettes, unsorted patches, unknown references at finish
  (`UnknownObject` / `UnknownInstance`), zero canvas (`DimensionTooLarge`).
* Script: parses to the byte-identical hand-built stream; deterministic
  (parse twice → same bytes); hostile scripts typed (`ScriptParse` +
  semantic errors) across 13 cases.
* §55 flattening courts (pinned byte-exact; the encoder is deterministic):

| content (synthetic) | A total | B total | total tax | A interval | B interval | interval tax |
|---|---|---|---|---|---|---|
| palette rotation, every pixel changes (96×96, 13 f) | 9 688 B | 74 294 B | 7.7× | 360 B | 64 987 B | **180×** |
| palette accent strip (uniform color change) | 1 165 B | 10 013 B | 8.6× | 288 B | 706 B | 2.5× |
| constant-acceleration object (trajectory) | 649 B | 24 115 B | 37.2× | 174 B | 4 824 B | 27.7× |
| affine rotation of a noise tile (Q8) | 310 B | 15 246 B | 49.2× | 210 B | 11 059 B | 52.7× |
| authored seeded-noise region, static | 126 B | 4 213 B | 33.4× | 26 B | 26 B | 1.0× (unchanged lane) |

Every court: leg B decodes byte-identically to leg A; leg C = raw raster bytes
(external conventional codecs belong to the §57 harness, outside this repo).

## Measured structural information loss (not just bytes)

Leg B of the palette courts carries **zero palette state**
(`state_bytes == 0` vs `> 0` for A, which also keeps the index plane); B of
the motion courts cannot express trajectory/affine/velocity state (it emits
per-frame translation / region / transform-residual repairs); B of the noise
court can never recover an authored seed (§21) — the 33× tax is structural
and permanent, and after the flattened base both legs ride the unchanged lane
at 13 B/frame.

## Recorded, not hidden

* Synthetic small-canvas courts only — no claim about natural video. Frame-0
  bases are comparable in both legs (each pays its base object once); the
  **interval marginal tax** (up to 180×) is what compounds with stream length.
* The accent-strip court measures B recovering the *visual* change as reusable
  region content (2.5× interval tax): palette semantics are still lost
  (`state_bytes == 0`), but the flattening cost of that particular visual is
  modest — an honest negative for that content class.
* §57 conventional-codec leg C and raster-domain courts (desktop/terminal/
  natural video) remain external-harness / later-court territory.
* The §53 script format is explicitly non-normative; a future native-ingest
  transport (Phase R) will packetize the same state model.

## Gate

`cargo fmt --check` · `cargo check --all-targets` · `cargo clippy
--all-targets --all-features -- -D warnings` (0 warnings) · `cargo test
--all-features` (228 tests, 0 failures, dev + release) · hostile-input courts ·
§55 phase court · evidence receipt (`evidence/campaigns/phase-q-ingest-…/`) ·
docs updated (`ingest.md`, `empirical-status.md`, `CONFORMANCE.md`,
`PROJECT_STATE.md`, README).

## Verdict

```
SEALED
```
