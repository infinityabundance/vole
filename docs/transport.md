# Procedural transport (Phase R — sealed)

VOLE transport organizes a stream around five packet classes:

```
OBJECT CHECKPOINT TRANSITION RESIDUAL INTEGRITY
```

A receiver keeps replicated state and materializes views through the
**normative parser**; it does not receive a fresh independent raster per
interval on structurally-static content.

Phase R (`src/transport.rs`, sealed) packetizes a **standalone** `.vole` stream
into ordered, length-prefixed, sequence-numbered frames and gives the receiver
incremental playback, typed packet-loss recovery, bounded checkpoint recovery,
and byte-exact reassembly. This is a *framing and ordering* layer — every
packet payload is the byte-exact v1 record the standalone writer emits (shared
record helpers in `format.rs`), so transport adds **no** second normative
semantics and the materializer stays authoritative.

## Frame grammar (additive to the `.vole` format, not part of it)

```text
frame := [len:u32][kind:u8][seq:u64][body]        len = 9 + body.len()

kind:  0x00 HEADER     0x01 OBJECT     0x02 PALETTE
       0x03 CHECKPOINT 0x04 INTERVAL   0x7E INTEGRITY
```

* `len` — little-endian u32, canonical (`9 + body.len()`; anything else is
  `TransportFormat("bad frame length")`).
* `kind` — one of the six tags above; any other byte is
  `TransportFormat("unknown frame kind")` (fail closed).
* `seq` — little-endian u64, dense from 0 in emission order. A receiver fed a
  frame whose `seq` is not the expected next one fails with the typed
  `VoleError::TransportGap` and applies nothing.
* `body` — the v1 record bytes (`format::header_bytes` / `object_decl_bytes` /
  `palette_decl_bytes` / `checkpoint_bytes` / `checkpoint_bindings_bytes` /
  `interval_bytes` / the 32-byte BLAKE3 digest for INTEGRITY).

Frame overhead is **13 bytes/packet** (`FRAME_OVERHEAD`).

## Emission order

```
HEADER OBJECT* PALETTE* CHECKPOINT INTERVAL* INTEGRITY
```

`HEADER` must be first; declarations precede the checkpoint; intervals are in
strictly increasing absolute `t`; exactly one checkpoint and one integrity
packet. Violations are `TransportFormat` with stable condition names:
`header not first`, `header required first`, `declaration after checkpoint`,
`interval before checkpoint`, `duplicate checkpoint`, `non-consecutive
interval`, `duplicate integrity packet`, `integrity body must be 32 bytes`.

`INTEGRITY` carries the standalone stream's own BLAKE3 trailer value (digest of
the concatenated prefix). `Receiver::verify` recomputes the digest over the
rebuilt prefix; `Receiver::reassemble` returns the prefix + trailer — for
canonical sources the exact transmitted stream bytes, for every source a byte
stream that decodes identically.

## Transmitter

`Transmitter::packetize(bytes)` accepts a standalone `.vole` stream
(`feature_bits == 0`). Store-backed streams (Phase P external-object
declarations) are deliberately rejected with a typed `ApiConstraint`: their
payloads live outside the stream, so transporting them requires the store
substrate — a recorded boundary, not a silent fallback.

* `encode()` — every frame; `encode_from(seq)` — retransmission from `seq`
  (used after a gap or from the first interval after a checkpoint reset).
* `payload_bytes()` equals the source stream length for canonical sources;
  `framed_bytes()` adds the 13 B/packet overhead.
* `packets()` / `packet_count()` / `interval_count()` / `digest()` /
  `first_interval_seq()` (the resume point: header + declarations +
  checkpoint).

## Receiver

`Receiver::feed(frame)` validates framing and ordering, then rebuilds the
canonical stream prefix. No transport-side state machine ever duplicates the
materializer:

* `frames_so_far()` decodes the received prefix **through the normative
  parser** (`decoder::decode_bytes` + `materialize_all`) — frame 0 as soon as
  the checkpoint arrives, one further frame per applied interval.
* `verify()` checks the integrity digest; `reassemble()` returns the complete
  standalone stream.
* `reset_to_checkpoint()` rolls applied intervals back to the checkpoint
  (declaration packets are kept) and resumes at `first_interval_seq`;
  replayed work is bounded by the v1 decode envelope
  (`Limits.max_transition_replay` / `max_checkpoint_distance`), exactly as the
  standalone decoder's replay is.

Accessors: `has_checkpoint`, `complete`, `expected_seq`, `applied_packets`,
`applied_bytes`, `interval_count_applied`, `object_count`, `prefix_len`,
`checkpoint_end_len`.

## Structural-innovation measurement (§33)

`profile_stream(bytes)` returns a `TransportProfile` with a per-interval
`IntervalStat` series: absolute `t`, payload and framed bytes, transition
count, structural `events` labels, and `residual_bytes`. This is the
"bytes over time vs structural events over time" measurement — on authored
content the interval lane tracks structural innovation (measured: static
26 B framed, one absolute position 39 B, one palette-entry patch 37 B, one
new instance 43 B), never raster geometry. The unchanged lane's amortized
cost is measured, never zeroed (§18): 225 frames including one-time
object/palette/checkpoint declarations + framing ≈ 29 B/frame on the sealed
court. See `docs/phase-r.md` and the evidence campaign.

## Hostile-input contract (transport domain)

Truncated frames (`Truncated`), corrupted length words, unknown kinds,
non-canonical ordering, duplicate declarations (`DuplicateId`), and digest
corruption (`verify() == false`) are all typed, deterministic outcomes —
never a panic, OOM, or hang.

## Status

Packetized procedural transport over standalone v1 streams: **ADOPTED**
(Phase R). Multi-checkpoint cadence, `OBJECT` re-sync for long-running
transport, and transport of store-backed (external-object) streams remain
later-phase surface (tracked in `PROJECT_STATE.md`).
