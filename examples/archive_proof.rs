//! Phase T evidence proof: the archive profile (§67 / Phase-T block of §64).
//!
//! Builds `.volea` archive manifests over standalone `.vole` streams and
//! measures the operational archive properties: build and verify latency
//! (structural — no raster work — versus deep frame-hash verification), the
//! manifest's byte overhead, record-level corruption localization (a flipped
//! byte is reported with its exact record: kind, offset, interval time), and
//! the golden frame hashes that survive representation changes (`vole
//! optimize` rewrites decode to identical rasters with identical hashes).
//!
//! The FFV1 operational comparison is an external-harness step (FFmpeg runs
//! outside this crate by design — §57); `corpus/ffv1-compare.sh` runs it and
//! records a receipt when `ffmpeg` is available. VOLE numbers here are
//! compression-neutral archive operations: this phase measures integrity and
//! localization, never a size claim against a conventional codec.
//!
//! Run: `cargo run --release --example archive_proof`

use std::time::Instant;

use vole_video::{
    archive::{self, encode, ArchiveManifest, VerifyStatus},
    decoder, demo,
    ingest::Ingest,
    optimize,
    pixel::Canvas,
    rans::KIND_RAW,
    transition::Transition,
    VoleError,
};

fn raw_block(pts: &[(i32, i32, u8)]) -> Vec<u8> {
    let mut body = Vec::with_capacity(9 * pts.len());
    for (x, y, v) in pts {
        body.extend_from_slice(&x.to_le_bytes());
        body.extend_from_slice(&y.to_le_bytes());
        body.push(*v);
    }
    let mut block = vec![KIND_RAW];
    block.extend_from_slice(&(body.len() as u64).to_le_bytes());
    block.extend_from_slice(&body);
    block
}

/// 1920×1080 × 81 frames: the Phase-A proof pattern at full-HD depth (one
/// persistent 200×100 object, 80 transitions).
fn full_hd_track() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (1920u32, 1080u32);
    let mut a = Ingest::new(w, h);
    a.background(5);
    a.declare_fill(1, 200, 100, 180)?;
    a.instance(1, 1, 100, 500)?;
    for t in 1..=80u64 {
        a.at(t)?;
        a.set_position(1, 100 + 2 * t as i64, 500)?;
    }
    a.finish()
}

/// Palette + index + sparse + residual + copy content.
fn mixed_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (192u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(17);
    a.declare_palette(1, vec![10, 60, 120, 200, 250])?;
    let idx: Vec<u8> = (0..(96 * 64))
        .map(|i| (((i / 96) * 3 + (i % 96) * 5) % 5) as u8)
        .collect();
    a.declare_index(1, 96, 64, idx)?;
    a.instance_binding(1, 1, 10, 10, 1)?;
    a.declare_raster(2, 24, 24, vec![9u8; 576])?;
    a.instance(2, 2, 120, 40)?;
    for t in 1..=30u64 {
        a.at(t)?;
        a.set_position(2, 120 + t as i64, 40)?;
        if t % 5 == 0 {
            let changes: Vec<(u8, u8)> = (0..5).map(|i| (i, (10 + t * 7) as u8)).collect();
            a.patch_palette(1, changes)?;
        }
        if t % 7 == 0 {
            a.push(Transition::CopyRect {
                src_x: 10 + t as i64,
                src_y: 0,
                width: 20,
                height: 20,
                dst_x: 150,
                dst_y: 60,
            })?;
        }
        if t == 29 {
            a.push(Transition::Residual {
                block: raw_block(&[(2, 2, 200), (90, 60, 11)]),
            })?;
        }
    }
    a.finish()
}

/// A raster-origin stream (Phase-G inverse encode): two 480×270 frames of
/// deterministic texture, so the stream is raster-dominated.
fn raster_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (480u32, 270u32);
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let data: Vec<u8> = (0..w * h)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 256) as u8
        })
        .collect();
    let frame = Canvas::from_parts(w, h, data)?;
    let second = frame.clone();
    let report = vole_video::inverse::encode_frames(
        &[frame, second],
        &vole_video::inverse::EncodeOptions::default(),
    )?;
    Ok(report.vole)
}

fn main() -> Result<(), VoleError> {
    // ------------------------------------------------------------------
    // Streams under test.
    // ------------------------------------------------------------------
    let golden = demo::MovingRectCourt::default().vole()?; // 1920×1080, 101 frames
    let hd = full_hd_track()?;
    let mixed = mixed_stream()?;
    let raster = raster_stream()?;
    let streams: Vec<(&str, Vec<u8>)> = vec![
        ("phase-a-moving-rect", golden),
        ("full-hd-81f", hd),
        ("mixed-31f", mixed),
        ("raster-origin-2f", raster),
    ];
    assert!(
        streams[3].1.len() > 100_000,
        "raster stream is raster-dominated ({} B)",
        streams[3].1.len()
    );

    println!("| stream | .vole B | .volea B | overhead | records | frames | build ms | structural verify ms | deep verify ms |");
    println!("|---|---|---|---|---|---|---|---|---|");
    let mut table: Vec<(String, u64, u64, u64, u64)> = Vec::new();
    for (name, bytes) in &streams {
        let t = Instant::now();
        let m = ArchiveManifest::build(bytes)?;
        let build_ms = t.elapsed().as_secs_f64() * 1e3;
        let wire = encode(&m)?;
        let t = Instant::now();
        let r = archive::verify(bytes, &m, false)?;
        let fast_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let r2 = archive::verify(bytes, &m, true)?;
        let deep_ms = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(r.status, VerifyStatus::Complete);
        assert_eq!(r2.status, VerifyStatus::Complete);
        assert_eq!(r2.frames_checked, m.stream.frame_count);
        let overhead = wire.len() as f64 * 100.0 / bytes.len().max(1) as f64;
        println!(
            "| {name} | {} | {} | {overhead:.1}% | {} | {} | {build_ms:.2} | {fast_ms:.3} | {deep_ms:.3} |",
            bytes.len(),
            wire.len(),
            m.records.len(),
            m.stream.frame_count
        );
        table.push((
            name.to_string(),
            bytes.len() as u64,
            wire.len() as u64,
            m.records.len() as u64,
            m.stream.frame_count,
        ));
    }

    // ------------------------------------------------------------------
    // Corruption localization on the mixed stream.
    // ------------------------------------------------------------------
    let bytes = &streams[2].1;
    let m = ArchiveManifest::build(bytes)?;
    println!();
    println!("corruption localization (mixed-31f, one flipped byte per case):");
    let mut cases: Vec<(&str, usize, usize)> = Vec::new();
    // Header width field (byte 16 of the stream).
    cases.push(("header width", 0, 16));
    // The palette record's last entry byte.
    let pal = m
        .records
        .iter()
        .position(|r| r.kind == archive::RecordKind::Palette)
        .expect("palette");
    cases.push(("palette entry", pal, m.records[pal].length as usize - 1));
    // Interval t=13's time byte.
    let iv = m
        .records
        .iter()
        .position(|r| r.kind == archive::RecordKind::Interval && r.t == Some(13))
        .expect("interval 13");
    cases.push(("interval t=13", iv, 1));
    // The integrity trailer.
    cases.push(("integrity trailer", m.records.len() - 1, 5));
    for (label, rec_idx, at) in cases {
        let r = m.records[rec_idx];
        let mut bad = bytes.clone();
        bad[(r.offset as usize + at).min(bytes.len() - 1)] ^= 0x01;
        match archive::verify(&bad, &m, false) {
            Ok(rep) => match rep.first_bad_record {
                Some(i) => {
                    let rec = &m.records[i as usize];
                    println!(
                        "  flipped {label}: -> record #{i} (kind={}, offset={}, t={:?})  [{}]",
                        rec.kind.label(),
                        rec.offset,
                        rec.t,
                        archive_status(&rep)
                    );
                    assert_eq!(i as usize, rec_idx, "{label} localizes to its own record");
                }
                None => {
                    println!(
                        "  flipped {label}: -> {} (no record mismatch)",
                        archive_status(&rep)
                    );
                }
            },
            Err(e) => {
                println!(
                    "  flipped {label}: -> typed error {e:?} (grammar-breaking flip; no panic)"
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // Golden frame hashes across a representation change (optimize).
    // ------------------------------------------------------------------
    let track = full_hd_track()?;
    let m = ArchiveManifest::build(&track)?;
    let opt = optimize::optimize_stream(&track)?;
    assert!(opt.stream.len() < track.len());
    let parsed_opt = decoder::decode_bytes(&opt.stream)?;
    let opt_hashes = archive::compute_frame_hashes(&parsed_opt)?;
    assert_eq!(
        opt_hashes, m.frame_hashes,
        "optimize rewrite: identical rasters => identical hashes"
    );
    let rep = archive::verify(&opt.stream, &m, false)?;
    assert_eq!(rep.status, VerifyStatus::StructuralMismatch);
    println!();
    println!(
        "representation equivalence: 2692 B SetPosition stream -> optimize rewrite ({} B): all {} frame hashes identical (golden oracle); verify reports STRUCTURAL_MISMATCH as expected (representation differs, reconstruction does not)",
        opt.stream.len(),
        m.frame_hashes.len()
    );

    // ------------------------------------------------------------------
    // Self-description + universe pinning of the phase-A golden.
    // ------------------------------------------------------------------
    let g = &streams[0];
    let m = ArchiveManifest::build(&g.1)?;
    println!();
    println!(
        "self-description (phase-a golden): format_v{} universe={} profile={} features={:#x} {}x{} gray8 frames={} stream={} B digest={}",
        m.stream.format_version,
        m.stream.universe_id,
        m.stream.limit_profile,
        m.stream.feature_bits,
        m.stream.width,
        m.stream.height,
        m.stream.frame_count,
        m.stream.stream_len,
        hex8(&m.stream.stream_digest)
    );
    println!(
        "  records={} intervals={} objects={} checkpoint_digest={}",
        m.records.len(),
        m.records
            .iter()
            .filter(|r| r.kind == archive::RecordKind::Interval)
            .count(),
        m.objects.len(),
        hex8(&m.checkpoint_digest)
    );

    println!();
    println!("archive proof: OK ({} streams)", table.len());
    Ok(())
}

fn hex8(d: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in d {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn archive_status(rep: &archive::VerifyReport) -> &'static str {
    match rep.status {
        VerifyStatus::Complete => "COMPLETE",
        VerifyStatus::SelfDescriptionMismatch => "SELF_DESCRIPTION_MISMATCH",
        VerifyStatus::StructuralMismatch => "STRUCTURAL_MISMATCH",
        VerifyStatus::StreamDigestMismatch => "STREAM_DIGEST_MISMATCH",
        VerifyStatus::ObjectMismatch => "OBJECT_MISMATCH",
        VerifyStatus::FrameDivergence => "FRAME_DIVERGENCE",
    }
}
