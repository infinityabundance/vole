# Procedural transport (production note)

VOLE transport organizes a stream around five packet classes:

```
OBJECT CHECKPOINT TRANSITION RESIDUAL INTEGRITY
```

A receiver keeps replicated state and materializes views; it does not receive a
fresh independent raster per interval on structurally-static content. This repo
is currently at v1 single-checkpoint files: a `.vole` file already *is* a
bounded object+checkpoint+transition+integrity stream. Network / loss /
restart profile (packetization over a transport, multi-checkpoint cadence,
`OBJECT` re-sync, packet-loss and recovery courts) is a later-phase activity
tracked in `PROJECT_STATE.md`.

Status: base (file grammar) ADOPTED; networked transport PROPOSED.
