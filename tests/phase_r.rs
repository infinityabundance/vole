//! Phase R courts: procedural transport (§34/§35/§36) — packetized
//! OBJECT/CHECKPOINT/TRANSITION(INTERVAL)/RESIDUAL/INTEGRITY streaming over a
//! standalone `.vole` stream, incremental playback through the normative
//! parser, packet-loss gap detection + retransmission, checkpoint recovery
//! with bounded replay, byte-exact reassembly, hostile framing, and the §33
//! structural-innovation (bytes-over-time vs events-over-time) measurement.

use vole_video::{
    decoder, ingest::Ingest, object::Object, pixel::Canvas, store::ObjectStore, transport,
    VoleError,
};

fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

fn frames_equal(a: &[Canvas], b: &[Canvas]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.exactly_matches(y))
}

/// A deterministic "structural-innovation scenario" (§33): static run →
/// per-frame motion → static → palette change → a new instance appears →
/// static.
///
/// Objects: 1 = 8x8 fill, 2 = 64x8 palette-index plane (palette 1).
/// Instances at checkpoint: 1 = fill sprite, 2 = index plane bound to palette 1.
///
/// Intervals 1..=24 → 25 frames:
///   t 1..8    static (empty groups, the unchanged lane)
///   t 9..16   sprite moves by absolute SetPosition (+2 px per frame)
///   t 17..20  static
///   t 21      patch one palette entry
///   t 22      create a third instance (new object instance appears)
///   t 23..24  static
///
/// `tail` appends that many further static intervals (used by the §33
/// long-run amortization measurement).
fn scenario_with_tail(tail: u64) -> Result<Vec<u8>, VoleError> {
    let (w, h) = (160u32, 96u32);
    let bg = 25u8;
    let mut a = Ingest::new(w, h);
    a.background(bg);
    a.declare_palette(1, vec![200, 60, 30, 90])?;
    a.declare_fill(1, 8, 8, 200)?;
    let idx: Vec<u8> = (0..(64 * 8)).map(|_| 1u8).collect();
    a.declare_index(2, 64, 8, idx)?;
    a.instance(1, 1, 30, 40)?;
    a.instance_binding(2, 2, 0, 80, 1)?;
    for t in 1..=8u64 {
        a.at(t)?;
    }
    for k in 0..8u64 {
        a.at(9 + k)?;
        a.set_position(1, 30 + 2 * k as i64, 40)?;
    }
    for t in 17..=20u64 {
        a.at(t)?;
    }
    a.at(21)?;
    a.patch_palette(1, vec![(1, 210)])?;
    a.at(22)?;
    a.create_instance(3, 1, 100, 40)?;
    for t in 23..=24u64 {
        a.at(t)?;
    }
    for t in 25..=24u64 + tail {
        a.at(t)?;
    }
    a.finish()
}

fn scenario() -> Result<Vec<u8>, VoleError> {
    scenario_with_tail(0)
}

#[test]
fn packetize_reassembles_byte_exact_and_frames_are_identical() -> Result<(), VoleError> {
    let bytes = scenario()?;
    let full = frames_of(&bytes)?;
    let tx = transport::Transmitter::packetize(&bytes)?;

    // Payload bytes equal the source stream bytes (payloads are the exact
    // stream records + the trailer digest); framing overhead is 13 B/packet.
    assert_eq!(tx.payload_bytes()?, bytes.len() as u64);
    assert_eq!(
        tx.framed_bytes()?,
        bytes.len() as u64 + transport::FRAME_OVERHEAD * tx.packet_count() as u64
    );
    // Emission order: HEADER OBJECT* PALETTE* CHECKPOINT INTERVAL* INTEGRITY.
    let kinds: Vec<u8> = tx.packets().iter().map(|p| p.kind()).collect();
    assert_eq!(kinds[0], transport::KIND_HEADER);
    assert_eq!(kinds[1], transport::KIND_OBJECT);
    assert!(kinds.contains(&transport::KIND_PALETTE));
    assert!(kinds.contains(&transport::KIND_CHECKPOINT));
    assert!(kinds.contains(&transport::KIND_INTERVAL));
    assert_eq!(kinds.last().copied(), Some(transport::KIND_INTEGRITY));

    // Determinism: packetizing twice produces identical frames.
    let frames = tx.encode()?;
    let tx2 = transport::Transmitter::packetize(&bytes)?;
    assert_eq!(frames, tx2.encode()?);

    // A fresh receiver consuming every frame reassembles the exact source
    // bytes and decodes the identical raster sequence; integrity verifies.
    let mut rx = transport::Receiver::new();
    for frame in split_frames(&frames) {
        rx.feed(frame)?;
    }
    assert!(rx.complete());
    assert!(rx.verify()?);
    let reassembled = rx.reassemble()?;
    assert_eq!(
        reassembled, bytes,
        "transport reassembles the standalone bytes"
    );
    let got = frames_of(&reassembled)?;
    assert!(frames_equal(&full, &got));
    Ok(())
}

/// Split a framed byte stream into its frames (hostile-free helper for
/// courts; the receiver itself parses frames from any slice in `feed`).
fn split_frames(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().expect("len")) as usize;
        out.push(&bytes[pos..pos + 4 + len]);
        pos += 4 + len;
    }
    out
}

#[test]
fn receiver_plays_back_partial_prefix_through_the_normative_parser() -> Result<(), VoleError> {
    let bytes = scenario()?;
    let full = frames_of(&bytes)?;
    let tx = transport::Transmitter::packetize(&bytes)?;
    let frames = tx.encode()?;
    let parts = split_frames(&frames);

    let mut rx = transport::Receiver::new();
    // No checkpoint yet: playback is not possible.
    assert_eq!(
        rx.frames_so_far().unwrap_err(),
        VoleError::InvalidStatePhase
    );
    // Header + declarations + checkpoint.
    let first_interval = tx.first_interval_seq();
    for f in &parts[..first_interval as usize] {
        rx.feed(f)?;
    }
    assert!(rx.has_checkpoint());
    assert_eq!(rx.object_count(), 2);
    let f0 = rx.frames_so_far()?;
    assert_eq!(f0.len(), 1);
    assert!(f0[0].exactly_matches(&full[0]));

    // Apply intervals 1..=8 (static run) → 9 frames available.
    for f in &parts[first_interval as usize..first_interval as usize + 8] {
        rx.feed(f)?;
    }
    let partial = rx.frames_so_far()?;
    assert_eq!(partial.len(), 9);
    assert!(frames_equal(&full[..9], &partial));

    // Every frame is available incrementally, matching the offline decode
    // (the loop covers intervals 9..=24; the trailing integrity packet adds no
    // frame).
    let mut prev = partial.len();
    for f in &parts[first_interval as usize + 8..parts.len() - 1] {
        rx.feed(f)?;
        let now = rx.frames_so_far()?;
        assert_eq!(now.len(), prev + 1);
        prev = now.len();
    }
    rx.feed(parts.last().expect("integrity packet"))?;
    assert!(rx.complete());
    assert!(rx.verify()?);
    let final_frames = rx.frames_so_far()?;
    assert!(frames_equal(&full, &final_frames));
    assert_eq!(rx.interval_count_applied(), 24);
    Ok(())
}

#[test]
fn packet_loss_is_a_typed_gap_and_retransmission_recovers() -> Result<(), VoleError> {
    let bytes = scenario()?;
    let full = frames_of(&bytes)?;
    let tx = transport::Transmitter::packetize(&bytes)?;
    let encoded = tx.encode()?;
    let parts = split_frames(&encoded);
    let n = parts.len();
    assert!(n > 24);

    // Lose one interval packet in the middle of the motion run: the packet at
    // index `lost` is never transmitted, so the next frame the receiver sees
    // is `parts[lost + 1]` — a typed gap; nothing applies.
    let lost = tx.first_interval_seq() as usize + 5;
    let mut rx = transport::Receiver::new();
    for f in &parts[..lost] {
        rx.feed(f)?;
    }
    assert_eq!(rx.expected_seq(), lost as u64);
    assert_eq!(
        rx.feed(parts[lost + 1]).unwrap_err(),
        VoleError::TransportGap
    );
    assert_eq!(rx.expected_seq(), lost as u64);
    assert_eq!(rx.interval_count_applied(), 5);

    // Retransmission from the gap recovers the exact stream.
    let resume = tx.encode_from(lost as u64)?;
    let retransmitted = split_frames(&resume);
    for f in &retransmitted {
        rx.feed(f)?;
    }
    assert!(rx.complete());
    assert!(rx.verify()?);
    let got = frames_of(&rx.reassemble()?)?;
    assert!(frames_equal(&full, &got));

    // Object-packet loss is also a gap (declaration packets are ordered).
    let mut rx2 = transport::Receiver::new();
    rx2.feed(parts[0])?; // header
    assert_eq!(rx2.feed(parts[2]).unwrap_err(), VoleError::TransportGap);
    Ok(())
}

#[test]
fn checkpoint_recovery_replays_with_bounded_work() -> Result<(), VoleError> {
    let bytes = scenario()?;
    let full = frames_of(&bytes)?;
    let tx = transport::Transmitter::packetize(&bytes)?;
    let encoded = tx.encode()?;
    let parts = split_frames(&encoded);
    let first_interval = tx.first_interval_seq() as usize;

    // Receiver falls behind: it has applied a few intervals, then rolls back
    // to the checkpoint and replays the whole timeline from the transmitter.
    let mut rx = transport::Receiver::new();
    for f in &parts[..first_interval + 12] {
        rx.feed(f)?;
    }
    assert_eq!(rx.interval_count_applied(), 12);
    let partial = rx.frames_so_far()?;
    assert_eq!(partial.len(), 13);
    assert!(frames_equal(&full[..13], &partial));

    // Roll back and replay from the checkpoint (bounded: the v1 replay
    // envelope caps transition replay; the measured replay here is the whole
    // 24-interval timeline).
    rx.reset_to_checkpoint();
    assert_eq!(rx.interval_count_applied(), 0);
    assert_eq!(rx.expected_seq(), first_interval as u64);
    let resume = tx.encode_from(first_interval as u64)?;
    for f in split_frames(&resume) {
        rx.feed(f)?;
    }
    assert!(rx.complete());
    let got = frames_of(&rx.reassemble()?)?;
    assert!(frames_equal(&full, &got));
    let replayed = rx.interval_count_applied();
    assert_eq!(replayed, 24);
    assert!(
        replayed <= vole_video::Limits::default().max_transition_replay,
        "replay stays inside the bounded envelope"
    );
    Ok(())
}

#[test]
fn missing_object_referenced_by_checkpoint_is_typed() -> Result<(), VoleError> {
    // A crafted sequence whose checkpoint references an object whose OBJECT
    // packet never arrived: feed accepts the ordering, playback fails typed.
    let mut rx = transport::Receiver::new();
    rx.feed(&raw_frame(transport::KIND_HEADER, 0, &raw_header(160, 96)))?;
    // Checkpoint with one instance referencing object id 9 (never declared).
    let mut cp = vec![0x03u8, 25u8];
    cp.extend_from_slice(&1u32.to_le_bytes());
    cp.extend_from_slice(&1u32.to_le_bytes()); // instance id
    cp.extend_from_slice(&9u32.to_le_bytes()); // object id — missing
    cp.extend_from_slice(&0i32.to_le_bytes());
    cp.extend_from_slice(&0i32.to_le_bytes());
    rx.feed(&raw_frame(transport::KIND_CHECKPOINT, 1, &cp))?;
    assert!(rx.has_checkpoint());
    assert_eq!(
        rx.frames_so_far().unwrap_err(),
        VoleError::UnknownObject,
        "referencing a never-received object is a typed error at playback"
    );
    Ok(())
}

#[test]
fn hostile_framing_and_ordering_are_typed() -> Result<(), VoleError> {
    // (a) Truncated frame.
    assert_eq!(
        transport::Receiver::new()
            .feed(&[0, 0, 0, 0, 0])
            .unwrap_err(),
        VoleError::Truncated
    );
    // (b) Declared length disagrees with the frame.
    let mut rx = transport::Receiver::new();
    let mut bad = raw_frame(transport::KIND_HEADER, 0, &raw_header(8, 8));
    let n = bad.len();
    bad[0] = 0xFF; // length no longer matches
    assert_eq!(
        rx.feed(&bad).unwrap_err(),
        VoleError::TransportFormat("bad frame length")
    );
    let _ = n;
    // (c) Unknown kind.
    let mut rx = transport::Receiver::new();
    assert_eq!(
        rx.feed(&raw_frame(0xEE, 0, &[])).unwrap_err(),
        VoleError::TransportFormat("unknown frame kind")
    );
    // (d) Header not first.
    let mut rx = transport::Receiver::new();
    let header = raw_frame(transport::KIND_HEADER, 0, &raw_header(8, 8));
    rx.feed(&header)?;
    assert_eq!(
        rx.feed(&raw_frame(transport::KIND_HEADER, 1, &raw_header(8, 8)))
            .unwrap_err(),
        VoleError::TransportFormat("header not first")
    );
    // (e) Interval before checkpoint.
    let mut rx = transport::Receiver::new();
    rx.feed(&header)?;
    let int = raw_frame(transport::KIND_INTERVAL, 1, &raw_interval(1));
    assert_eq!(
        rx.feed(&int).unwrap_err(),
        VoleError::TransportFormat("interval before checkpoint")
    );
    // (f) Declaration after checkpoint.
    let mut rx = transport::Receiver::new();
    rx.feed(&header)?;
    let cp = raw_frame(transport::KIND_CHECKPOINT, 1, &raw_checkpoint(0));
    rx.feed(&cp)?;
    assert_eq!(
        rx.feed(&raw_frame(
            transport::KIND_OBJECT,
            2,
            &[0x02, 1, 0, 0, 0, 0, 0, 0, 0, 5]
        ))
        .unwrap_err(),
        VoleError::TransportFormat("declaration after checkpoint")
    );
    // (g) Duplicate checkpoint.
    let mut rx = transport::Receiver::new();
    rx.feed(&header)?;
    let cp = raw_frame(transport::KIND_CHECKPOINT, 1, &raw_checkpoint(0));
    rx.feed(&cp)?;
    assert_eq!(
        rx.feed(&raw_frame(
            transport::KIND_CHECKPOINT,
            2,
            &raw_checkpoint(0)
        ))
        .unwrap_err(),
        VoleError::TransportFormat("duplicate checkpoint")
    );
    // (h) Non-consecutive interval times.
    let mut rx = transport::Receiver::new();
    rx.feed(&header)?;
    let cp = raw_frame(transport::KIND_CHECKPOINT, 1, &raw_checkpoint(0));
    rx.feed(&cp)?;
    rx.feed(&raw_frame(transport::KIND_INTERVAL, 2, &raw_interval(5)))?;
    assert_eq!(
        rx.feed(&raw_frame(transport::KIND_INTERVAL, 3, &raw_interval(5)))
            .unwrap_err(),
        VoleError::TransportFormat("non-consecutive interval")
    );
    // (i) Duplicate integrity packet.
    let mut rx = transport::Receiver::new();
    rx.feed(&header)?;
    rx.feed(&raw_frame(
        transport::KIND_CHECKPOINT,
        1,
        &raw_checkpoint(0),
    ))?;
    rx.feed(&raw_frame(transport::KIND_INTEGRITY, 2, &[0u8; 32]))?;
    assert_eq!(
        rx.feed(&raw_frame(transport::KIND_INTEGRITY, 3, &[0u8; 32]))
            .unwrap_err(),
        VoleError::TransportFormat("duplicate integrity packet")
    );
    // (j) Integrity with a short body.
    let mut rx = transport::Receiver::new();
    rx.feed(&header)?;
    rx.feed(&raw_frame(
        transport::KIND_CHECKPOINT,
        1,
        &raw_checkpoint(0),
    ))?;
    assert_eq!(
        rx.feed(&raw_frame(transport::KIND_INTEGRITY, 2, &[0u8; 8]))
            .unwrap_err(),
        VoleError::TransportFormat("integrity body must be 32 bytes")
    );
    // (k) Receiver state queries before completion.
    let mut rx = transport::Receiver::new();
    rx.feed(&header)?;
    assert_eq!(
        rx.verify().unwrap_err(),
        VoleError::TransportFormat("integrity packet not received")
    );
    assert_eq!(
        rx.reassemble().unwrap_err(),
        VoleError::TransportFormat("stream incomplete")
    );
    Ok(())
}

#[test]
fn transport_rejects_store_backed_streams_typed() -> Result<(), VoleError> {
    // A stream with external object declarations (Phase P) is not standalone;
    // transport refuses it (recorded Phase-R boundary).
    let tile = Object::raster(8, 8, vec![7u8; 64])?;
    let mut st = vole_video::store::EmbeddedStore::create(
        &std::env::temp_dir().join(format!("vole-r-hostile-{}", std::process::id())),
    )?;
    let cid = st.put(&vole_video::store::object_record(&tile))?.id;
    let ext = vole_video::encoder::encode_stream_external(
        16,
        16,
        0,
        &[(1, cid)],
        &[vole_video::state::Instance {
            id: vole_video::state::InstanceId(1),
            object_id: vole_video::object::ObjectId(1),
            x: 0,
            y: 0,
        }],
        &[],
    )?;
    assert!(transport::Transmitter::packetize(&ext).is_err());
    Ok(())
}

#[test]
fn structural_innovation_bytes_track_events_not_raster_geometry() -> Result<(), VoleError> {
    // The §33 hypothesis on authored content: transported bytes per interval
    // respond to structural events, and are tiny fractions of the raster
    // samples those frames represent.
    let bytes = scenario()?;
    let profile = transport::profile_stream(&bytes)?;
    assert_eq!(profile.width, 160);
    assert_eq!(profile.height, 96);
    assert_eq!(profile.frames, 25);
    assert_eq!(profile.objects, 2);
    assert_eq!(profile.intervals.len(), 24);

    // Static intervals are the unchanged lane: an empty 13-byte record.
    for i in 0..8 {
        let s = &profile.intervals[i];
        assert_eq!(s.packet_bytes, 13, "static interval t={}", s.t);
        assert!(s.events.is_empty(), "no structural event on static frames");
        assert_eq!(s.residual_bytes, 0);
    }
    // Motion intervals carry one absolute position update: 13 + 13 bytes.
    for i in 8..16 {
        let s = &profile.intervals[i];
        assert_eq!(s.events, vec!["position"]);
        assert_eq!(s.packet_bytes, 26);
    }
    // A single palette-entry patch: 24 bytes; the new instance: 30 bytes.
    assert_eq!(profile.intervals[20].events, vec!["patch_palette"]);
    assert_eq!(profile.intervals[20].packet_bytes, 24);
    assert_eq!(profile.intervals[21].events, vec!["create_instance"]);
    assert_eq!(profile.intervals[21].packet_bytes, 30);
    // No residual anywhere: procedural state explains every frame exactly.
    assert!(profile.intervals.iter().all(|s| s.residual_bytes == 0));

    // The interval lane (§33 series): bytes sent per interval track the
    // structural events the interval carries. Every interval — static,
    // motion, palette patch, or new instance — ships in at most 43 framed
    // bytes, versus 15,360 raster samples per frame.
    let samples = u64::from(profile.width) * u64::from(profile.height);
    for s in &profile.intervals {
        assert!(
            s.framed_bytes <= 43,
            "interval t={} ships {} framed bytes",
            s.t,
            s.framed_bytes
        );
    }
    let lane_total: u64 = profile.intervals.iter().map(|s| s.framed_bytes).sum();
    assert!(
        lane_total < samples / 2,
        "whole 24-interval lane ({lane_total} B) < half of one raster frame"
    );
    // Whole stream (one-time object + palette declarations, checkpoint, every
    // interval, framing, and the integrity trailer): the entire 25-frame
    // transport is smaller than one raster frame's samples.
    assert!(
        profile.framed_bytes < samples,
        "25 frames in {} framed bytes < one 15,360-sample raster frame",
        profile.framed_bytes
    );

    // Long-run amortization (§18): extend the same timeline with 200 static
    // intervals; the measured per-frame cost (including the one-time
    // declarations and per-packet framing) collapses toward the unchanged
    // lane and stays far below the raster per-frame sample count.
    let long_bytes = scenario_with_tail(200)?;
    let long = transport::profile_stream(&long_bytes)?;
    assert_eq!(long.frames, 225);
    assert_eq!(long.intervals.len(), 224);
    let amortized = long.framed_bytes / long.frames;
    assert!(
        amortized < 100,
        "measured amortized per-frame transport cost: {amortized} B"
    );
    assert!(
        amortized * 100 < samples,
        "amortized per-frame cost is <1% of one raster frame"
    );
    Ok(())
}

#[test]
fn receiver_recovers_from_loss_after_checkpoint_reset() -> Result<(), VoleError> {
    // Combined recovery: heavy loss forces a checkpoint reset, then replay.
    let bytes = scenario()?;
    let full = frames_of(&bytes)?;
    let tx = transport::Transmitter::packetize(&bytes)?;
    let encoded = tx.encode()?;
    let parts = split_frames(&encoded);
    let first_interval = tx.first_interval_seq() as usize;

    let mut rx = transport::Receiver::new();
    for f in &parts[..first_interval + 3] {
        rx.feed(f)?;
    }
    // Simulate corruption of the applied tail: reset to the checkpoint and
    // retransmit from the first interval.
    rx.reset_to_checkpoint();
    let resume = tx.encode_from(first_interval as u64)?;
    for f in split_frames(&resume) {
        rx.feed(f)?;
    }
    assert!(rx.complete());
    assert!(rx.verify()?);
    let got = frames_of(&rx.reassemble()?)?;
    assert!(frames_equal(&full, &got));
    Ok(())
}

// ---------------------------------------------------------------------------
// Raw frame builders (hostile courts; layouts mirror the framing grammar)
// ---------------------------------------------------------------------------

fn raw_frame(kind: u8, seq: u64, body: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&((9 + body.len()) as u32).to_le_bytes());
    f.push(kind);
    f.extend_from_slice(&seq.to_le_bytes());
    f.extend_from_slice(body);
    f
}

fn raw_header(w: u32, h: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"VOLE");
    b.push(0);
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.push(1);
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    b
}

fn raw_checkpoint(bg: u8) -> Vec<u8> {
    let mut b = vec![0x03u8, bg];
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}

fn raw_interval(t: u64) -> Vec<u8> {
    let mut b = vec![0x04u8];
    b.extend_from_slice(&t.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}
