# EntropyFS integration (scoping note)

EntropyFS is an **optional persistence substrate** (ADR-0004). It is not
required to decode any standalone `.vole` stream, and it is not part of this
crate. When the standalone representation stabilizes through the phase ladder,
an `ObjectStore`-shaped adapter (EmbeddedStore/EntropyFsStore) will let exact
immutable objects be shared across videos with physical accounting and GC.

Status: PROPOSED — disabled until Phase P / EntropyFS-integration phase. See
`docs/adr/0004-entropyfs-optional.md`.
