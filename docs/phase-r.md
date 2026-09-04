# Phase R Receipt — procedural transport (§34/§35/§36) and the §33 structural-innovation measurement (SEALED)

## Deliverable

1. **`vole_video::transport`** (`src/transport.rs`, §34/§35/§36): packetized
   streaming of a standalone `.vole` stream in the five transport classes —
   **OBJECT CHECKPOINT TRANSITION(INTERVAL) RESIDUAL INTEGRITY**. Frame grammar
   `[len:u32][kind:u8][seq:u64][body]` (13 B/packet overhead), canonical
   emission order `HEADER OBJECT* PALETTE* CHECKPOINT INTERVAL* INTEGRITY`,
   and six kind tags (`KIND_HEADER 0x00 … KIND_INTEGRITY 0x7E`). Every packet
   payload is produced by the shared `format::*_bytes` record helpers, so a
   body is **byte-identical to the standalone writer's record** — transport is
   framing/ordering only, and the receiver rebuilds the canonical prefix and
   plays it through the **normative parser** (`frames_so_far()` decodes the
   prefix via `decoder::decode_bytes` + `materialize_all`; never a duplicated
   transport-side state machine, so the materializer stays authoritative).
2. **Loss recovery**: out-of-sequence frames are the typed
   `VoleError::TransportGap`; `Transmitter::encode_from(seq)` retransmits from
   the gap. A receiver that fell behind rolls back with
   `Receiver::reset_to_checkpoint()` (declarations kept) and replays forward —
   replay bounded by the v1 envelope (`max_transition_replay` /
   `max_checkpoint_distance`), exactly like standalone decode.
3. **Integrity**: `INTEGRITY` carries the standalone stream's own BLAKE3
   trailer; `Receiver::verify()` / `reassemble()` return the exact standalone
   stream for canonical sources (byte-exactness asserted in court) and a
   decode-identical stream for every source.
4. **§33 measurement** (`profile_stream` / `TransportProfile` /
   `IntervalStat` / `event_label`): per-interval bytes-over-time vs
   structural-events-over-time, plus `residual_bytes`, over a standalone
   stream.

No format-v1 constants changed; `.vole` files are untouched; transport is an
additive framing layer over the standalone stream.

## Courts (`tests/phase_r.rs`, 9 tests)

| Court | Result |
|---|---|
| Packetize → reassemble: payload bytes == source length; emission order HEADER/OBJECT/PALETTE/CHECKPOINT/INTERVAL/INTEGRITY; deterministic re-packetize; fresh receiver reassembles the **exact source bytes**; 25 frames identical; digest verifies | PASS |
| Incremental playback through the normative parser: frame 0 at the checkpoint; frames available per interval advance 2…25 monotonically, each prefix decoding byte-identical to the offline full-stream prefix; playback before the checkpoint is `InvalidStatePhase` | PASS |
| Packet loss: dropping one interval packet is `TransportGap` and nothing applies; retransmission from the gap recovers the exact stream; declaration-packet loss is also a typed gap | PASS |
| Checkpoint recovery: 12 applied intervals roll back to the checkpoint, replay the full 24-interval timeline, stream identical; measured replay 24 ≤ the 1 000 000-interval envelope bound | PASS |
| Missing object: a checkpoint referencing an object whose OBJECT packet never arrived feeds cleanly but playback fails typed (`UnknownObject`) | PASS |
| Hostile framing/ordering (11 forms): truncated frame → `Truncated`; bad length word; unknown kind; header not first; interval before checkpoint; declaration after checkpoint; duplicate checkpoint; non-consecutive interval time; duplicate integrity; short integrity body; verify/reassemble before completion — all typed | PASS |
| Store-backed rejection: external-object (Phase P) streams are refused typed (`ApiConstraint`), recorded boundary | PASS |
| §33 structural-innovation series on a deterministic 25-frame authored scenario: static intervals 13 B payload/26 B framed with no events and 0 residual; one absolute `SetPosition` 26 B payload; one palette-entry patch 24 B; one `create_instance` 30 B; 24-interval lane total 756 B < half of one 15 360-sample raster frame; whole 25-frame transport 1 488 framed B < one raster frame; no residual anywhere | PASS |
| Long-run unchanged-lane amortization (§18): +200 static intervals → 225 frames at **29 B/frame framed** including one-time declarations and framing (measured, never zeroed) | PASS |

## Malformed-input court (transport domain)

Payload-byte flip inside an interval body → typed feed error (non-consecutive
interval); integrity-digest byte flip → `verify() == false` cleanly; truncated
frame → `Truncated`; corrupted length word → `TransportFormat`. Confirmed in
the release proof (see below) and hostile tests. No panic, OOM, or hang.

## Empirical court / evidence

`examples/transport_proof.rs` (release) exercises packetization, byte-exact
reassembly, incremental playback, gap loss + retransmission, checkpoint
recovery with bounded replay, three corruption courts, and the §33 series on
the 25-frame scenario plus the 225-frame amortization run:

```
packetize: source 1098B -> 30 packets (24 interval) -> framed 1488B (13 B/packet overhead)
reassemble: byte-exact 1098B; integrity verify passes; 25 frames identical
playback: frames available per interval [1:2 … 24:25] — monotone, matches offline decode
loss: dropped packet seq 10 -> TransportGap; retransmit 20 frames from seq 10; verify passes
checkpoint recovery: rollback to checkpoint, replay 24 intervals (envelope bound 1000000), stream identical
corruption: payload byte flip -> typed error during feed at frame 13
corruption: digest byte flip -> integrity verify = false (bounded, typed)
§33 per-interval lane: static 26 B framed, motion 39 B, palette patch 37 B, new instance 43 B
lane total: 756 B for 24 intervals (<= 7680 = half of one 15 360-sample raster frame)
whole 25-frame transport: 1488 framed B < one 15360B raster frame
amortized: 225 frames -> 29 B/frame framed vs 15360 raster samples/frame
transport proof: OK
```

Evidence: `evidence/campaigns/phase-r-transport-1788543916/`
(`transport-proof.log`, `environment.json`, `summary.json`).

## Recorded, not hidden

* Transport is a framing/ordering layer, not a new normative format: for
  canonical sources the reassembled bytes equal the source `.vole` byte-for-byte
  (asserted); for every source the reassembled stream decodes identically.
  No format-v1 constant changed; old streams and goldens untouched.
* Single-checkpoint v1 files packetize with exactly one CHECKPOINT packet;
  multi-checkpoint cadence, `OBJECT` re-sync, and long-running transport
  remain later-phase surface (§35 trade-offs measured when multi-checkpoint
  streams exist).
* Store-backed (external-object) streams are refused typed — transporting them
  needs the store substrate (Phase P) and is recorded open surface.
* Synthetic small-canvas courts only: the §33 numbers show that on authored
  content transported bytes respond to structural events and are minuscule
  fractions of the raster; no claim about natural video or a universal
  bandwidth law (§72).

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0 warnings) ·
`cargo test` (235 tests, 0 failures) · `cargo test --all-features` (237,
dev) · `cargo test --release --all-features` (237) · hostile-input courts ·
Phase-R court · evidence receipt (`evidence/campaigns/phase-r-transport-…/`) ·
docs updated (`transport.md`, `empirical-status.md`, `CONFORMANCE.md`,
`PROJECT_STATE.md`, README).

## Verdict

```
SEALED
```
