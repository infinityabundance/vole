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
| Phase T: record index tiles every stream shape exactly (header + decls + checkpoint + intervals + integrity; interval `t` == parsed timeline; deterministic digests); store-backed extern streams scan but refuse archiving typed | PASS |
| Phase T: manifest wire canonical + self-authenticating (any flip ⇒ IntegrityMismatch, truncation typed, hostile counts bounded); schema pinning (v2 ⇒ UnsupportedFeature, bad magic ⇒ BadMagic); golden phase-A file archives and deep-verifies 101/101 frames | PASS |
| Phase T: layered verification — pristine streams Complete (structural + objects + checkpoint + 101/101 deep frame hashes); one flipped byte localizes to its exact record (header / object decl / interval t / integrity trailer); grammar-breaking corruption is a typed error, never a panic | PASS |
| Phase T: representation equivalence — `vole optimize` rewrite reports StructuralMismatch while every frame hash is identical (reconstruction oracle); forged pinned universe ⇒ SelfDescriptionMismatch(Universe); archive verification orthogonal to partial views | PASS |
| Phase T (FFV1 external harness): FFmpeg FFV1 lossless roundtrip byte-verified on the phase-A court with a full receipt (sizes, times, environment); VOLE 2 692 B procedural vs FFV1 105 078 B raster on that synthetic court — no general claim | PASS (harness, external) |
| Phase U: quantizer lattice == documented formula over the full Gray8 domain × shifts 0..=7 × both roundings (half-up top half-bin saturates at 255; dead-zone never leaves the lattice); Box3 `[1 2 1] ≫ 2` pre-filter hand-computed; distortion metrics hand-computed | PASS |
| Phase U: exact profile is lossless and unmarked (byte-identical to the plain inverse encode); lossy `encode_lossy` sets the declaration bit and **decoder output == F̂ == Q(source)** byte-for-byte on every stream (normative proof); marker idempotent and deterministic | PASS |
| Phase U: RD ladder deterministic + monotone distortion, rows agree with direct encodes; `choose_rd` = least-distorted evaluated row within budget (recomputed expectation), honest unmet budget; no budget = smallest stream | PASS |
| Phase U: feature bit 0x2 is a declaration only — fake-set on an exact stream and fake-clear on a quantized stream both decode to identical frames; marker refuses store-backed and truncated input typed; corrupted marked stream still IntegrityMismatch | PASS |
| Phase U: quantized streams survive transport (byte-identical reassembly), archive (deep verify Complete), and `vole optimize` (declaration preserved, decode-identical F̂) | PASS |
| Phase U: measured flagships — flat panel + 2-bit temporal jitter 480×270 ×17: exact 1 806 807 B → q3 270 B (6 692×, MAE 1.5, 15.9 B/frame vs 106 282.8 B/frame); recorded non-monotone bytes (q1/q2 > exact), dominated q4 row never chosen; authored control exact == q2 bytes; noise control RAW at every shift with exact proof | PASS (measured) |
| Phase U: regression — Phase-A golden (101 full-HD frames) and every earlier-phase stream decode unchanged with bit 0x2 known; noise negative control stays RAW (procedural fraction < 0.15) | PASS |

## Court status (Phase V.1 — video programme)

| Court | Result |
|---|---|
| Phase V.1.1: frame-rate grid exact (23.976 … 120); cross-base ordering/rescale; VFR per-observation durations; layout registry with independent ceil-geometry oracle on odd dimensions; bit depths 8/9/10/12/14/16 with padding-bit discipline and LE canonical round-trips; color/HDR semantics; orientation/SAR/interlace preserved as interpretation; epoch model + canonical-video validation; hostile constructions typed | PASS |
| Phase V.1.1: flagship synthetic HDR vector — 24 obs 10-bit BT.2020/PQ YUV420 1919×1079 at 23.976 + epoch transition to 12-bit 4:4:4 1921×1081; geometry/storage/timeline/color exact, never rounded | PASS |
| Phase V.1.2: v1 Gray8 specialization oracle — the v2 core at depth 8 reproduces the authoritative v1 decoder byte-for-byte over a 6-frame authored scenario (fill + raster + SetPosition + sparse + COPY_RECT + residual) | PASS |
| Phase V.1.2: authored 10-bit 4:2:0 equals an independent per-plane compositor; exact raster-origin floor proven sample-for-sample (fresh program AND re-parse) with static duplicates on the unchanged lane; noise falls to RAW with bounded overhead | PASS |
| Phase V.1.2: v2 core wire roundtrips byte-exactly across 11 layout×depth rows (Gray 8/10/16 odd 9×7, YUV420 8/10, YUV444 8/12, GBR 8, RGB 10, RGBA 8 odd 7×5, YUVA444 8) with canonical fixpoint `write∘parse == id` | PASS |
| Phase V.1.2: hostile typed corpus across 8 layout×depth rows (content flips ⇒ IntegrityMismatch, wrong magic/layout ⇒ typed structural errors before the digest, truncations typed, never a panic); unknown feature bits and v1-on-v2 bodies fail closed | PASS |
| Phase V.1.2: frozen v2 grammar golden digest pinned (`a5c1fb40…6a56a80f`); programs bind to rational PTS and epoch transitions; full A–U regression clean (v1 goldens unchanged) | PASS |
| Phase V.1.3: bounded foreign-tool runner — argv-only commands (never a shell string), wall-clock + stdout/stderr byte caps with clean kills, typed `BridgeNotFound`/`BridgeTimeout`/`BridgeOutputLimit`; unit courts pass | PASS |
| Phase V.1.3: import matrix over 10 source layouts/depths — planar carriers (yuv420p at odd 18×12, yuv420p10le, gray, gray16le, yuv444p, yuv422p, gbrp) byte-exact against authored canonical frames, packed/semi-planar (rgb24, bgra, nv12, yuyv422) byte-exact through reversible repacking; every observation oracle-verified per frame (independent framehash SHA-256 over tight rows, exact rational PTS on each frame's own time base) | PASS |
| Phase V.1.3: compressed sources — FFV1 Matroska 10-bit lossless round trip EXACT (depth 10 preserved); H.264 MP4 oracle-exact, 25/25 verified, deterministic BLAKE3 + SHA-256 sequence digests reproducible across re-imports; NUT stream time base (1/51200) == imported canonical time base | PASS |
| Phase V.1.3: VFR timeline preserves exact PTS deltas (one delta > 2× modal); last-observation duration unknown, never guessed | PASS |
| Phase V.1.3: hostile corpus typed and panic-free — NUT truncations across the whole file, wrong magic, version/CRC-region flips, payload flips; a tampered payload byte stays structurally parseable but oracle verification fails typed `CanonicalHashMismatch`, and a pristine re-parse verifies | PASS |
| Phase V.1.3: missing tools typed (`BridgeNotFound` on empty PATH); garbage inputs fail closed (typed bridge errors, never a hang); import time base == the NUT stream time base | PASS |
| Phase V.1.4: v1 specialization parity at depth 8 for the ported families — velocity/advance translation, linear + accel trajectories, palette-index content + palette mutation, all four generator programs, Q8 quarter-turn + 2× zoom affine, and the transform-coded residual; every materialized frame byte-identical to the authoritative v1 decoder | PASS |
| Phase V.1.4: authored 10-bit YUV420 semantic surface matches an independent per-plane compositor (closed-form trajectory positions, no shared paint code) on every observation | PASS |
| Phase V.1.4: depth-aware generators — depth-8 identity to the sealed v1 Phase-N generators coordinate-by-coordinate; mod-(max+1) wrap and noise-scaling courts at 10/12/16-bit; wire record round-trips; hostile parameters typed | PASS |
| Phase V.1.4: family encoder — static runs ride unchanged groups; 10-bit gradients and 4-value fields declared once as generator/palette content; a translating textured sprite served by CopyRect region reuse from its second frame (first appearance on a residual class); every run sample-exact with honest RAW-floor accounting (authored 10-bit YUV420: 3 664 B vs 33 327 B floor, 9.10×) | PASS |
| Phase V.1.4: v2 family-extension wire — byte fixpoint roundtrips across 8 layout×depth rows incl. odd geometry and 16-bit; minimal feature bits (0 old surface / 0x1 extension); V.1.2 golden bytes unchanged; extension golden pinned; hostile corpus (cleared bit, op-without-bit, unknown motion kind, semantic refs, truncations/flips) typed, never a panic | PASS |
| Phase V.1.4: transform floor — encode → op-0x31 decode == target at 10-bit through the wire, randomized 16-bit delta fields exact, hostile blocks typed | PASS |
| Phase V.1.5: `GlobalPredict` (op 0x32, feature bit 0x2) equals whole-plane `CopyRect` for integer translations and matches a naive per-sample expectation oracle over arbitrary Q8 maps (quarter-turn / 2× zoom / general); the map-shift registry (Q8/Q12/Q16) is exact at 8/10/16-bit depths | PASS |
| Phase V.1.5: family encoder over dense natural-like raster content — camera pan (96×64 Gray8, 6 obs): 5/5 intervals global_translation, 6 643 B vs 30 975 B RAW (4.66×); ablation without the global family == the RAW floor exactly; continuous zoom-in: 18 673 B < 20 735 B RAW with the precision court returning identical Q8/Q12/Q16 bytes (tie → Q8, reported per-shift); 10-bit YUV420 multiplane pan: all 15 plane-intervals global_translation on each plane's own grid, 11 619 B vs 23 805 B RAW (2.05×); every run sample-exact through the wire | PASS |
| Phase V.1.5: negative controls — iid noise yields 0 global observations and exactly the RAW floor; a hard scene cut stops the global run; static tail after a pan is held from the previous observation (greedy economics recorded) | PASS |
| Phase V.1.5: v2 global-motion wire — minimal/additive feature bits (0x2 alone, 0x3 with the family surface); V.1.5 extension golden pinned (`2791d622…ecbbae9`); hostile corpus typed never a panic (op without the bit, unknown shift byte, out-of-domain coefficients, unknown feature bits, truncations/flips); whole-plane warp bomb under a small `max_motion_work` ⇒ `MaterializationBudgetExceeded`, default envelope fine | PASS |
| Phase V.1.5: estimator boundedness + determinism (pyramid ≤ 24 coarse candidates, fits over ≤ 4096 grid points; identical hypotheses/work on re-run; unit recovery of exact pan and 1.05× bilinear zoom near ground truth) | PASS |

## Goldens

Sealed format v1 golden streams must decode forever under v1 semantics:
(old) stream hash + expected reconstruction hashes are recorded in evidence.
Any future re-encode or version bump must not reinterpret v1 files.

See `docs/architecture.md`, `PROJECT_STATE.md`, `evidence/campaigns/`.
