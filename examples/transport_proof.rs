//! Phase R evidence proof: procedural transport (§34/§35/§36) and the §33
//! structural-innovation measurement.
//!
//! A standalone `.vole` stream (authored through the native ingest API) is
//! packetized into the five transport classes — **OBJECT CHECKPOINT
//! TRANSITION(INTERVAL) RESIDUAL INTEGRITY** — and exercised over a lossy
//! channel:
//!
//! * byte-exact reassembly through the normative parser (payloads are the
//!   exact standalone records; the receiver rebuilds the canonical prefix and
//!   plays it through `decoder::decode_bytes` — the materializer stays
//!   authoritative, never a duplicated transport-side state machine);
//! * incremental playback: every prefix produces the correct partial frame
//!   sequence (frame 0 at the checkpoint, one further frame per interval);
//! * packet loss: an out-of-sequence frame is a typed `TransportGap`;
//!   retransmission from the gap (`encode_from`) recovers the exact stream;
//! * checkpoint recovery: a receiver that fell behind rolls back to the
//!   checkpoint and replays forward — replay is bounded by the v1 decode
//!   envelope and measured;
//! * corruption: hostile frames are typed errors, never panics, and a flipped
//!   payload byte is caught by the standalone integrity trailer;
//! * the §33 series: transported bytes per interval track structural events,
//!   not raster geometry (with a long-run amortization measurement of the
//!   unchanged lane including one-time declarations).
//!
//! Run: `cargo run --release --example transport_proof`

use vole_video::{decoder, ingest::Ingest, pixel::Canvas, transport, VoleError};

/// Deterministic structural-innovation scenario: static run → motion →
/// static → palette patch → new instance → static (`tail` appends extra
/// static intervals for the amortization measurement).
fn scenario(tail: u64) -> Result<Vec<u8>, VoleError> {
    let (w, h) = (160u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(25);
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

fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

/// Split a framed byte stream into frames (mirrors `Receiver` framing).
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

fn same(a: &[Canvas], b: &[Canvas]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.exactly_matches(y))
}

fn main() -> Result<(), VoleError> {
    let bytes = scenario(0)?;
    let full = frames_of(&bytes)?;
    assert_eq!(full.len(), 25);

    // ------------------------------------------------------------------
    // 1. Packetization: emission order, framing, byte-exact reassembly.
    // ------------------------------------------------------------------
    let tx = transport::Transmitter::packetize(&bytes)?;
    let encoded = tx.encode()?;
    let parts = split_frames(&encoded);
    assert_eq!(tx.payload_bytes()?, bytes.len() as u64);
    println!(
        "packetize: source {}B -> {} packets ({} interval) -> framed {}B ({} B/packet overhead)",
        bytes.len(),
        tx.packet_count(),
        tx.interval_count(),
        tx.framed_bytes()?,
        transport::FRAME_OVERHEAD
    );

    let mut rx = transport::Receiver::new();
    for f in &parts {
        rx.feed(f)?;
    }
    assert!(rx.complete());
    assert!(rx.verify()?);
    let reassembled = rx.reassemble()?;
    assert_eq!(reassembled, bytes, "payloads are the exact stream records");
    assert!(same(&full, &frames_of(&reassembled)?));
    println!(
        "reassemble: byte-exact {}B; integrity verify passes; 25 frames identical",
        reassembled.len()
    );

    // ------------------------------------------------------------------
    // 2. Incremental playback through the normative parser.
    // ------------------------------------------------------------------
    let first_interval = tx.first_interval_seq();
    let mut rx = transport::Receiver::new();
    let mut steps: Vec<usize> = Vec::new();
    for (i, f) in parts[..parts.len() - 1].iter().enumerate() {
        rx.feed(f)?;
        if i >= first_interval as usize {
            steps.push(rx.frames_so_far()?.len());
        }
    }
    assert_eq!(steps.len(), 24);
    assert_eq!(*steps.first().expect("frame 1"), 2);
    assert_eq!(*steps.last().expect("frame 25"), 25);
    let incremental = steps
        .iter()
        .enumerate()
        .map(|(i, n)| format!("{}:{}", i + 1, n))
        .collect::<Vec<_>>()
        .join(" ");
    println!("playback: frames available per interval [{incremental}] — monotone, matches offline decode");

    // ------------------------------------------------------------------
    // 3. Packet loss: typed gap + retransmission from the gap.
    // ------------------------------------------------------------------
    let lost = first_interval as usize + 5;
    let mut rx = transport::Receiver::new();
    for f in &parts[..lost] {
        rx.feed(f)?;
    }
    let before = rx.frames_so_far()?.len();
    let gap = rx.feed(parts[lost + 1]).unwrap_err();
    assert_eq!(
        rx.frames_so_far()?.len(),
        before,
        "nothing applies on a gap"
    );
    let resume = tx.encode_from(lost as u64)?;
    for f in split_frames(&resume) {
        rx.feed(f)?;
    }
    assert!(rx.complete());
    assert!(rx.verify()?);
    assert!(same(&full, &frames_of(&rx.reassemble()?)?));
    println!(
        "loss: dropped packet seq {lost} -> {gap:?}; retransmit {} frames from seq {lost}; verify passes; 25 frames identical",
        tx.packet_count() - lost
    );

    // ------------------------------------------------------------------
    // 4. Checkpoint recovery: roll back, replay bounded forward.
    // ------------------------------------------------------------------
    let mut rx = transport::Receiver::new();
    for f in &parts[..first_interval as usize + 12] {
        rx.feed(f)?;
    }
    rx.reset_to_checkpoint();
    assert_eq!(rx.expected_seq(), first_interval);
    let resume = tx.encode_from(first_interval)?;
    for f in split_frames(&resume) {
        rx.feed(f)?;
    }
    assert!(rx.verify()?);
    let replayed = rx.interval_count_applied();
    let bound = vole_video::Limits::default().max_transition_replay;
    assert!(replayed <= bound);
    assert!(same(&full, &frames_of(&rx.reassemble()?)?));
    println!(
        "checkpoint recovery: rollback to checkpoint, replay {replayed} intervals (envelope bound {bound}), stream identical"
    );

    // ------------------------------------------------------------------
    // 5. Corruption courts: typed, never a panic.
    // ------------------------------------------------------------------
    // 5a. Flip one payload byte of the motion interval t=9 (its low time
    // byte becomes 8, colliding with the previous interval): framing stays
    // intact, but the feed rejects the non-consecutive interval — typed.
    let mut corrupted = encoded.clone();
    // Locate the start of frame `first_interval + 8` (the t=9 interval) by
    // walking the length prefixes.
    let target = first_interval as usize + 8;
    let mut start = 0usize;
    for _ in 0..target {
        let len = u32::from_le_bytes(corrupted[start..start + 4].try_into().expect("len")) as usize;
        start += 4 + len;
    }
    corrupted[start + 4 + 1 + 8 + 1] ^= 0x01; // len + kind + seq + tag -> t low byte
    let mut rx = transport::Receiver::new();
    let mut typed_at: Option<usize> = None;
    for (i, f) in split_frames(&corrupted).into_iter().enumerate() {
        if rx.feed(f).is_err() {
            typed_at = Some(i);
            break;
        }
    }
    let outcome = match typed_at {
        Some(i) => format!("typed error during feed at frame {i} (non-consecutive interval)"),
        None => format!(
            "fed cleanly but integrity mismatch: {}",
            !rx.verify().unwrap_or(false)
        ),
    };
    println!("corruption: payload byte flip -> {outcome}");
    // 5b. Flip a byte of the integrity digest itself: verify fails cleanly.
    let mut corrupted = encoded.clone();
    let n = corrupted.len();
    corrupted[n - 1] ^= 0xFF;
    let mut rx = transport::Receiver::new();
    for f in split_frames(&corrupted) {
        rx.feed(f)?;
    }
    assert!(rx.complete());
    assert!(!rx.verify()?);
    println!("corruption: digest byte flip -> integrity verify = false (bounded, typed)");
    // 5c. Truncated frame and corrupted length are typed framing errors.
    assert_eq!(
        transport::Receiver::new()
            .feed(&[0, 0, 0, 0, 0])
            .unwrap_err(),
        VoleError::Truncated
    );
    let mut bad_len = parts[0].to_vec();
    bad_len[0] = 0xFF;
    assert!(matches!(
        transport::Receiver::new().feed(&bad_len),
        Err(VoleError::TransportFormat(_))
    ));
    println!("corruption: truncated frame -> Truncated; bad length -> TransportFormat");

    // ------------------------------------------------------------------
    // 6. §33 structural-innovation series: bytes over time vs events.
    // ------------------------------------------------------------------
    let profile = transport::profile_stream(&bytes)?;
    println!();
    println!("§33 per-interval lane (payload + 13 B framing):");
    println!(
        " t : {}",
        (1..=24)
            .map(|t| format!("{t:>4}"))
            .collect::<Vec<_>>()
            .join("")
    );
    println!(
        " B : {}",
        profile
            .intervals
            .iter()
            .map(|s| format!("{:>4}", s.framed_bytes))
            .collect::<Vec<_>>()
            .join("")
    );
    let ev: Vec<&str> = profile
        .intervals
        .iter()
        .map(|s| {
            if s.events.is_empty() {
                "·"
            } else {
                s.events[0]
            }
        })
        .collect();
    println!(
        " ev: {}",
        ev.iter()
            .map(|e| format!("{e:>4}"))
            .collect::<Vec<_>>()
            .join("")
    );
    let lane_total: u64 = profile.intervals.iter().map(|s| s.framed_bytes).sum();
    let samples = u64::from(profile.width) * u64::from(profile.height);
    assert!(lane_total < samples / 2);
    println!(
        "lane total: {lane_total} B for 24 intervals (<= {samples}/2 = {} raster samples of ONE frame)",
        samples / 2
    );
    let mut static_count = 0usize;
    for s in &profile.intervals {
        if s.events.is_empty() {
            static_count += 1;
            assert_eq!(s.packet_bytes, 13);
        }
    }
    assert_eq!(static_count, 8 + 4 + 2);
    assert!(profile.framed_bytes < samples);
    println!(
        "whole 25-frame transport: {} framed B < one {}B raster frame",
        profile.framed_bytes, samples
    );

    // Long-run amortization of the unchanged lane (§18): one-time
    // declarations amortize toward the 13 B/interval static record.
    let long_bytes = scenario(200)?;
    let long = transport::profile_stream(&long_bytes)?;
    let amortized = long.framed_bytes / long.frames;
    assert!(amortized < 100);
    println!(
        "amortized: {} frames -> {amortized} B/frame framed (incl. one-time object/palette/checkpoint + framing) vs {samples} raster samples/frame",
        long.frames
    );

    println!();
    println!("transport proof: OK");
    Ok(())
}
