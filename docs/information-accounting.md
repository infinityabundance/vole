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
plus integrity (32 bytes) and header fixed cost. Finer typed category counters
(above) will be unlocked as those categories (residuals/models/dictionaries/
checkpoints at cadence) exist.

## Accounting honesty rules

* No shared object counts as zero: store-level physical cost and per-stream
  attribution are reported separately once EntropyFS integration lands.
* `procedural fraction` and any entropy phrasing are **engineering accounting
  metrics**, never Shannon-entropy claims.
* Negative results remain in the ledger and are never deleted.

Status: categories above PROPOSED as their phases land; the invariant "count
everything" is in force now.
