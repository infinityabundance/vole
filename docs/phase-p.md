# Phase P Receipt — optional EntropyFS persistence substrate (SEALED)

## Deliverable

The optional content-addressed persistence substrate (§1 / §31 / §45 / §46,
ADR-0004), in three layers:

1. **`ObjectStore` abstraction** (`src/store.rs`) — `get(id, max)` /
   `put(bytes)` (returns the BLAKE3 content id + whether the payload was
   physically new) / `contains(id)` / `unique_count` /
   `unique_payload_bytes` / `physical_bytes` / `sync` / `close`. The
   materializer and format parser obtain immutable object bytes only through
   this trait: provenance (`.vole` file, `EmbeddedStore`, `EntropyFsStore`,
   memory cache) never leaks into normative semantics.
2. **Two implementations.** `EmbeddedStore` — the always-available in-crate
   store: header `"VSTO"+1` + one append-only log
   (`[cid 32][len u64][payload]*`, 40 B/record framing), hash-gated reads
   (digest mismatch ⇒ `IntegrityMismatch`), bounded records and log
   (hostile/truncated logs are typed errors, never allocation bombs), named
   snapshot roots (`roots/<name>`, sorted 32-byte cids), and mark-compact GC
   over the root union. `EntropyFsStore` — feature `entropyfs-store`
   (default OFF): an adapter over the published `entropyfs` embeddable
   engine (`put_blob`/`get_blob`/`contains`/`sync`/`compact`/`metrics`),
   whose `BlobId` for a payload is the same BLAKE3 as VOLE's content id.
3. **Wire extension (additive; old streams re-parse unchanged).**
   `TAG_OBJECT_EXTERN 0x09` (`[obj:u32][cid:32]`) + header feature bit
   `FEAT_EXTERNAL_OBJECTS 0x1`. `encoder::encode_stream_external` writes
   store-backed streams (payloads leave the stream); `decoder::decode_with_store`
   resolves every reference through the store at parse — each fetched record's
   digest must equal the declared cid (`IntegrityMismatch`) and is re-parsed
   by `Object::from_canonical_record` — after which the object is ordinary and
   replay/materialization never touches the store. A stream carrying `0x09` is
   deliberately **not standalone**: store-less decode is `StoreRequired`;
   referenced-but-absent records are `StoreObjectMissing`; unknown feature
   bits fail closed; bit-without-declaration and declaration-without-bit are
   `NonCanonicalEncoding`.

## Sharing semantics (§46)

Payloads are the exact canonical record bytes
(`identity::canonical_object_record`, which `content_id_of` hashes), so
store ids equal object content ids; palette-table *snapshots* publish as
`0xE0` + entries (collision-free with object records by construction);
byte-identical objects across videos share one physical record — sharing is
by exact identity, never appearance.

## Accounting (§31)

Per-stream **declared** attribution (the record bytes the stream embeds —
shared state never zeroed), **unique payload** volume, and **actual
store-level physical** bytes are reported separately
(`StreamPublish`/`ArchiveAccounting`); `dedup_saved = declared − unique`.

## Courts (`tests/phase_p.rs`, 13 tests; all pass, dev + release)

* canonical records round-trip every object kind (fill / raster /
  palette-index / generator) and hostile record forms are typed;
* EmbeddedStore round-trip, dedup (`fresh=false`), reopen durability, hash
  gate, physical == framing + payload, bounded reads;
* hostile store files: flipped byte ⇒ `IntegrityMismatch` at open, truncated /
  over-long record ⇒ `Truncated`, duplicate cid ⇒ `NonCanonicalEncoding`, bad
  magic / absent store ⇒ `StoreFailure`;
* roots + GC closure: unreferenced blobs reclaimed, live (≥ 1 root) never
  collected, last-root-drop ⇒ full closure, reclamation reported;
* cross-video sharing: four videos share a 32×32 logo (unique payloads 5,
  dedup = 3 logo records exactly), palette tables + index objects share
  across two videos (dedup exact at the payload level), shared objects
  attributed never zeroed;
* extern streams materialize **byte-identical** frames with the payload
  outside the stream (774 B → 428 B on the 11-frame court); store-less decode
  `StoreRequired`; missing object `StoreObjectMissing`; digest mismatch
  `IntegrityMismatch`; hostile wire forms typed (bit/tag/order/dup/truncated);
* publish and `vole optimize` reject non-standalone streams typed;
* EntropyFsStore adapter (feature-gated): identical content ids across stores,
  engine dedup to one blob, reopen durability, byte-exact get.

## Measured (evidence/campaigns/phase-p-store-1788541087, release)

| court | result |
|---|---|
| embedded dedup (32×32 logo record) | fresh=[true,false], unique=1, physical 1073 B = 40 B framing + 1033 B payload |
| cross-video archive (4 videos + 2 palette videos) | unique payloads 7, declared 6250 B, unique 2878 B, physical 3158 B, dedup saved 3372 B |
| GC closure | a,b,c roots; pass 1 reclaims d (50 B, retained 3); drop video-1 ⇒ b collected (retained 2); drop video-2 ⇒ full closure (retained 0) |
| extern identical materialization | standalone 774 B → external 428 B (346 B of payload moved out), 11 frames byte-identical |
| entropyfs engine (feature) | fresh=[true,false], unique=1, physical 4018 B engine segments, id == VOLE content id, reopen durable |

## Re-recorded (recorded, not hidden)

* `tests/malformed.rs feature_bits_must_be_zero`: header bit `0x1` became the
  *known* external-objects feature, so bit-set-without-declaration is now
  `NonCanonicalEncoding`; unknown bits remain `UnsupportedFeature`. Old
  streams (`feature_bits == 0`) are byte-identical in semantics.
* `EntropyFsStore.unique_payload_bytes` maps to the engine's
  `physical.live_bytes` (root-reachable canonical record bytes) and is
  advisory; byte-exact payload accounting is asserted on the `EmbeddedStore`,
  whose layout VOLE owns. Engine physical numbers are reported honestly.
* Publish covers object records and palette-table snapshots from the
  checkpoint state. Interval palette mutations remain per-stream timeline
  state; rANS model tables and dictionary tables are not first-class v1
  tables yet — their cross-video sharing stays recorded open surface.
* `vole optimize` operates on standalone streams only; store-backed streams
  are rejected typed (never silently rewritten).

## Gate

`cargo fmt --check` · `cargo check --all-targets` · `cargo clippy
--all-targets --all-features -- -D warnings` (0 warnings, both feature
configs) · `cargo test --all-features` (218 tests, 0 failures, dev + release)
· malformed-input courts · phase court (above) · evidence receipt · docs
updated (`format-v1.md`, `entropyfs.md`, `information-accounting.md`,
`empirical-status.md`, `CONFORMANCE.md`, `PROJECT_STATE.md`).

## Verdict

```
SEALED
```
