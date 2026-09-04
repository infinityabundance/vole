# Complete physical information accounting

VOLE storage decisions compare **all persisted bytes**, never only residual or
only descriptor bytes. The accounting model is strongly typed (see brief §31):

```rust
struct RepresentationCost {
    state_bytes, transition_bytes, object_bytes, residual_bytes,
    checkpoint_bytes, model_bytes, dictionary_bytes, index_bytes,
    integrity_bytes, other_bytes: u64,
}
```

Phase-A reporting currently counts the user-visible byte split at granularity
the courts already publish: `vole` total vs the raw full-frame equivalent,
plus integrity (32 bytes) and header fixed cost. Since Phase G the typed
category counters exist (`inverse::RepresentationCost`: header/object/
checkpoint/transition/residual/model/state/dictionary/index/integrity) and
the ten buckets sum to the stream length exactly. Inline entropy models are
reported as a **sub-bucket**: model bytes are counted in `model_bytes` and
excluded from `residual_bytes` (a Phase-M transform residual block carries up
to two inline models, one per DC/AC container).

## Accounting honesty rules

* No shared object counts as zero: store-level physical cost and per-stream
  attribution are reported separately (Phase P, `src/store.rs`
  `ArchiveAccounting` — see `docs/entropyfs.md` and `docs/phase-p.md`).
* `procedural fraction` and any entropy phrasing are **engineering accounting
  metrics**, never Shannon-entropy claims.
* Negative results remain in the ledger and are never deleted.

Status: the stream-bucket ledger is in force now; the store-level ledger
(declared / unique-payload / physical) landed in Phase P.
