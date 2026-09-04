# EntropyFS integration (Phase P — implemented)

EntropyFS is an **optional persistence substrate** (ADR-0004). It is not
required to decode any standalone `.vole` stream, and the standalone build of
this crate does not link it. Phase P delivered the abstraction and both
implementations:

```text
ObjectStore                      the content-addressed store abstraction
    get(id, max) / put(bytes) / contains(id)
    unique_count / unique_payload_bytes / physical_bytes
    sync / close

EmbeddedStore                    the always-available in-crate store
    single append-only content-addressed log (src/store.rs):
      header "VSTO"+1 | blobs.log: [cid 32][len u64][payload]*
      roots/<name>: named snapshot roots (sorted 32-byte cids)
    hash-gated reads, bounded records, typed open/corruption errors,
    mark-compact GC over the root union (never collects a live blob)

EntropyFsStore                   feature `entropyfs-store` (default OFF)
    adapter over the published `entropyfs` embeddable engine
    (put/get/contains/sync/compact/metrics, typed error classes,
    exclusive store lock, durability barrier, own physical accounting)
```

## Sharing semantics

Sharing requires the **exact canonical content identity**, never appearance
(§46):

* object payloads are the canonical record bytes
  `identity::canonical_object_record` (also the bytes `content_id_of` hashes),
  so a store id for an object record **equals** the object's content id and
  the entropyfs engine's `BlobId` for the same bytes;
* palette-table snapshots publish under kind `0xE0` + entries, collision-free
  with object records by construction;
* rANS model tables and dictionary tables are not first-class v1 tables yet —
  sharing them remains recorded open surface, never silently claimed.

## Accounting (master brief §31)

Reported separately, never conflated:

* **declared** — the per-stream attribution: the sum of the record bytes each
  stream embeds (shared state is attributed to every stream that uses it,
  never zeroed);
* **unique payload** — the distinct payload volume without framing;
* **physical** — the actual store-level bytes (embedded log length incl.
  40 B/record framing; engine segment bytes for `EntropyFsStore`).

`archive_accounting` derives `dedup_saved = declared − unique_payload`.

## External object declarations (the materializer's provenance boundary)

Format v1 gained an additive, optional declaration form (Phase P): tag `0x09`
`[obj][cid 32]` + header feature bit `0x1`. Such a stream references
store-held canonical records instead of embedding payloads; parsing resolves
every reference through the `ObjectStore` (digest re-verified), after which
the objects are ordinary `Object`s — replay and materialization never touch
the store, and the materializer cannot tell file-resident from store-resident
objects. Streams with external declarations are deliberately **not
standalone**: store-less decode fails `StoreRequired`; unknown feature bits
fail closed; old files (`feature_bits == 0`) decode unchanged.

## Where the mechanics live

`src/store.rs` (trait, payload kinds, publish/accounting, `EmbeddedStore`,
`EntropyFsStore`), `src/format.rs` (tag `0x09`, feature bit), `src/decoder.rs`
(`decode_with_store`), `src/encoder.rs` (`encode_stream_external`),
`src/object.rs` (`Object::from_canonical_record`), `src/identity.rs`
(record bytes + content ids), `src/state.rs` (palette-table iterator). Courts:
`tests/phase_p.rs`; evidence: `evidence/campaigns/phase-p-store-…/`; receipt:
`docs/phase-p.md`.

Status: **ADOPTED** (Phase P, sealed). EntropyFS itself remains out-of-crate
and optional; the standalone `.vole` semantics are untouched.
