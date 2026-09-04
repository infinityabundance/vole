# Conformance

## Statement

A conforming `.vole` decoder, given a canonical v1 stream and default
limit-profile 1, produces the exact sequence of full Gray8 frames specified by
`SPEC.md`, `docs/format-v1.md`, `docs/materialization.md`, and
`docs/transitions.md`. On hostile input it returns a typed `VoleError` and
never panics/hangs/OOMs.

## Independent oracle

The Phase-A conformance court does **not** merely decode against the encoder's
own expectations. `tests/court.rs` compares the materialized frames against an
**independent reference painter** (`src/demo.rs::reference_painter`) that
shares no blit code, so a bug in the shared `Canvas::blit` cannot pass the
court. Boundary frames are additionally hashed (SHA-256) against an
independently re-derived reference in `proof/`.

## Court status (Phase A)

| Check | Result |
|---|---|
| materialize == independent painter, all 101 frames | PASS |
| first/last frame sample checkpoints under motion model | PASS |
| stored stream is not raster-proportional | PASS |
| hostile-input cases all terminate typed | PASS |
| `cargo fmt`, `check`, `clippy -D warnings`, `test` | PASS |
| Phase I: trajectory materialization == independent closed-form painter, all frames | PASS |
| Phase I: closed-form simulator == normative state stepper (200 random programs) | PASS |
| Phase I: trajectory collapse decode-identical and strictly smaller | PASS |
| Phase J: palette-index materialization == independent palette painter, all frames | PASS |
| Phase J: palette ops (0x06/0x08/0x2d–0x2f) round-trip and hostile forms fail typed | PASS |
| Phase J: palette accounting buckets (state_bytes, index_object_bytes) sum to total | PASS |
| Phase K: variable-region encoder streams decode byte-identical (zero whole-frame rebases on localized change) | PASS |
| Phase K: region exact-ref reuse, DSFB byte-equality, noise RAW negative control | PASS |
| Phase L: affine materialization == independent incremental sampling painter (rotation / zoom / sub-pixel pan / random parameters) | PASS |
| Phase L: Q8 30°-rotation approximation + residual == float-rendered target byte-for-byte | PASS |
| Phase L: affine over palette-index and fill objects exact; work budget + hostile wire forms typed | PASS |
| Phase M: transform residual roundtrip exact (random + gradient blocks; unit courts) | PASS |
| Phase M: transform materialization == target byte-for-byte end-to-end (drift / wrap-ramp / textured) | PASS |
| Phase M: noise stays RAW; tiny diffs never evaluate the family; oracle min-payload invariant holds | PASS |
| Phase M: hostile kind-2 streams typed at parse (id/padding/length/truncation) and materialization (EntropyCorrupt / OutOfBounds) | PASS |
| Phase N: generator objects materialize byte-exact vs independent references (all four kinds, plain + affine + motion) | PASS |
| Phase N: pure-gradient sequences are discovered procedurally (35 245× flagship); noise and wrong-seed controls stay RAW | PASS |
| Phase N: generator+residual closure exact; hostile generator wire forms typed; identity == wire record; accounting sums | PASS |
| Phase O: every accepted rewrite is strictly smaller and decode-identical (velocity / trajectory collapse, residual promotion, generator substitution, duplicate merge) | PASS |
| Phase O: never grows on earlier-phase stream shapes; palette streams preserved verbatim; hostile input typed | PASS |
| Phase P: EmbeddedStore round-trip / dedup / reopen / hash gate exact; hostile store files typed at open (flip ⇒ IntegrityMismatch, truncate ⇒ Truncated, dup-cid ⇒ NonCanonical, bad magic ⇒ StoreFailure) | PASS |
| Phase P: cross-video exact-object + palette sharing dedups to one physical record per distinct payload; declared / unique / physical reported separately (never zeroed); GC closure never collects a live root, last drop ⇒ full closure | PASS |
| Phase P: external-declaration streams (`encode_stream_external`/`decode_with_store`) materialize byte-identical frames with payloads outside the stream; store-less decode / missing record / digest mismatch / hostile wire forms typed; old streams (feature_bits 0) re-parse unchanged | PASS |
| Phase P: EntropyFsStore adapter (feature) — engine BlobId == VOLE content id, dedup to one blob, reopen durable, byte-exact get | PASS |
| Phase P: standalone invariance — the full pre-Phase-P suite passes untouched (no store required for embedded streams) | PASS |
| Phase Q: `Ingest` output byte-identical to the descriptor encoder (plain + palette paths); velocity/advance/copy/move/sparse/clear/residual round-trips exact | PASS |
| Phase Q: §53 script format parses to the byte-identical hand-built stream and is deterministic; hostile scripts typed (`ScriptParse`) | PASS |
| Phase Q (§55): direct-ingest and rasterize→inverse legs reproduce the same canonical raster sequence byte-for-byte on every court; flattening taxes pinned (palette rotation 180× interval, accel 37× total, affine noise rotation 49×, seeded noise 33×) | PASS |
| Phase Q: palette state survives only in the ingest leg (`state_bytes > 0` vs 0); zero canvas geometry and unknown references fail typed at finish | PASS |
| Phase R: transport packet payloads are byte-identical to the standalone v1 records; a fresh receiver reassembles the exact source bytes and decodes identical frames; integrity verifies | PASS |
| Phase R: incremental playback through the normative parser — frame 0 at the checkpoint, one further frame per applied interval, every prefix byte-equal to the offline full-stream decode | PASS |
| Phase R: packet loss is a typed `TransportGap` with nothing applied; retransmission from the gap recovers the exact stream; checkpoint rollback replays within the v1 envelope (measured 24 intervals ≤ 1 000 000 bound) | PASS |
| Phase R: hostile transport forms typed — truncated frame, bad length, unknown kind, header-not-first, interval-before-checkpoint, declaration-after-checkpoint, duplicate checkpoint, non-consecutive interval, duplicate/short integrity, store-backed stream refused; corruption courts bounded (payload flip → typed feed error, digest flip → verify false) | PASS |
| Phase R (§33): per-interval bytes track structural events on the deterministic 25-frame court (static 26 B framed → one palette patch 37 B → one new instance 43 B; 24-interval lane 756 B < half of one raster frame; whole transport 1 488 framed B < one raster frame); unchanged-lane amortization 29 B/frame over 225 frames — measured, never zeroed | PASS |
| Phase S: partial `Rect`/`Tile` views equal the whole-frame crop sample-for-sample on every frame of every stream shape (COPY_RECT chains, RAW/rANS residuals, palette content, affine rotation, generator drift, 12 deterministic random movies × 4 random views each); `FullFrame` view == canonical whole-frame decode | PASS |
| Phase S: tile grids partition frames exactly; decoder/state-level view APIs agree with `Decoder::materialize` crops; stats internally consistent (painted == base+copy+residual; residual writes only in-region) | PASS |
| Phase S (audit-scope): index poison inside the view → `OutOfBounds` identical to whole-frame decode; outside the view → clean samples (documented sampling contract); unsorted residual errors identical for any view of its frame; out-of-range frames/views and zero-size geometry typed | PASS |
| Phase S (work): 1920×1080 41-frame viewport court — one level painted per decode (56 400 samples vs ≥ 2 073 600 whole-frame), objects touched 1, 2.72% of the whole-frame lane, peak raster 36 400 samples; release random access frame 40 = 0.068 ms vs 13.5 ms whole (198×); copy-chain demand exact and bounded | PASS |

## Goldens

Sealed format v1 golden streams must decode forever under v1 semantics:
(old) stream hash + expected reconstruction hashes are recorded in evidence.
Any future re-encode or version bump must not reinterpret v1 files.

See `docs/architecture.md`, `PROJECT_STATE.md`, `evidence/campaigns/`.
