//! Phase M courts: the deterministic integer transform residual floor.
//!
//! Phase G's residual algebra (`F = M ⊕_ρ R`) coded the per-frame delta as a
//! sparse point list (RAW or order-0 rANS over the serialized points). When
//! the delta is **dense and structured** — smooth gradients, lighting or
//! brightness drift, imperfectly predicted natural texture — that floor
//! wastes bytes: a smooth field needs one point per pixel in point form.
//! Phase M adds the **TRANSFORM_RESIDUAL family**: the delta is partitioned
//! into aligned 4×4 blocks, each block is decorrelated by the normative
//! integer lifting DCT (reversible, integer only, no floating point, no
//! quantization), and the DC/AC coefficient streams plus a per-block skip
//! mask are entropy-coded (residual block kind 2). The decoder
//! inverse-transforms and **adds** the reconstruction back. This is the
//! conventional-coder floor: it lets ordinary transform + entropy coding win
//! exactly where procedural state cannot explain dense smooth deltas
//! economically — and it must lose (RAW stays) on noise.
//!
//! Courts: the brightness-drift flagship (dense smooth deltas, transform
//! winners, byte-exact end-to-end, far from raster-proportional); the
//! drifting-ramp court (full-range smooth content stays exact); the textured
//! court; noise/random negative controls; the sparse-gate (tiny diffs never
//! pay the transform overhead); oracle invariants; hostile payload courts at
//! parse (unknown transform id, padding bits, length disagreement,
//! truncation) and at materialization (out-of-Gray8 reconstruction) built
//! through the public writer so integrity stays valid.

use vole_video::{
    decoder, error::VoleError, format::StreamWriter, integr, inverse, pixel::Canvas, rans,
    time::Interval, transition::Transition,
};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

/// Non-uniform deterministic base ramp with **curvature in both axes**
/// (`x²` and `y²` terms), so a whole-canvas brightness offset is *not*
/// equivalent to any scroll/copy of the base (a linear ramp would be — the
/// exhaustive encoder would rightly find the scroll instead). Values stay
/// well inside the Gray8 domain for small offsets.
fn ramp_base(w: u32, h: u32) -> Canvas {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = 70 + u64::from(x) * u64::from(x) / 512 + u64::from(y) * u64::from(y) / 512;
            d.push(v as u8);
        }
    }
    canvas_of(w, h, d)
}

/// Brightness drift: `base` plus `t` everywhere (a dense, smooth, small
/// field no fill/sparse/object family can express).
fn drifted(base: &Canvas, t: u8) -> Canvas {
    let mut data = base.as_slice().to_vec();
    for v in &mut data {
        *v = v.saturating_add(t);
    }
    canvas_of(base.width(), base.height(), data)
}

/// Full-range drifting ramp with a wrap (dense smooth content that spans the
/// whole Gray8 scale each frame).
fn ramp_wrap_frame(w: u32, h: u32, phase: u64) -> Canvas {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = (3 * u64::from(x) + 5 * u64::from(y) + 11 * phase) % 256;
            d.push(v as u8);
        }
    }
    canvas_of(w, h, d)
}

/// Structured natural-texture-like field: smooth ramp plus small deterministic
/// periodic texture.
fn textured_frame(w: u32, h: u32, phase: u64) -> Canvas {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let ramp = (2 * u64::from(x) + 4 * u64::from(y) + 7 * phase) % 256;
            let tex = ((u64::from(x) * 3 + u64::from(y) * 7) % 9) as u8;
            d.push(((ramp + u64::from(tex)) % 256) as u8);
        }
    }
    canvas_of(w, h, d)
}

fn noise_frame(w: u32, h: u32, seed: u64) -> Canvas {
    let mut s = seed.max(1);
    let mut d = Vec::with_capacity((w * h) as usize);
    for _ in 0..(w * h) {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        d.push((s >> 56) as u8);
    }
    canvas_of(w, h, d)
}

fn encode_default(frames: &[Canvas]) -> Result<inverse::EncodeReport, VoleError> {
    inverse::encode_frames(
        frames,
        &inverse::EncodeOptions {
            bg_sweep: true,
            ..inverse::EncodeOptions::default()
        },
    )
}

/// Reseal a mutated payload with a fresh BLAKE3 trailer.
fn seal(payload: &[u8]) -> Vec<u8> {
    let mut out = payload.to_vec();
    out.extend_from_slice(&integr::digest(payload));
    out
}

#[test]
fn brightness_drift_is_transform_coded_and_stays_exact() -> Result<(), VoleError> {
    // A non-uniform base plus a whole-canvas brightness offset per frame:
    // every pixel changes by a small smooth amount. No procedural family and
    // no sparse point list can serve it cheaply; the transform floor should
    // win every drift interval and the stream must decode byte-identically.
    let (w, h) = (192u32, 128u32);
    let base = ramp_base(w, h);
    let mut frames = vec![base.clone()];
    for t in 1..=8u8 {
        frames.push(drifted(&base, t));
    }
    let report = encode_default(&frames)?;
    assert!(report.exact, "encoder end-to-end verify");
    let mut transform_wins = 0u64;
    for d in &report.decisions {
        if d.winner_family == "transform_residual" {
            transform_wins += 1;
            let t = d
                .families
                .iter()
                .find(|f| f.family == "transform_residual")
                .expect("family evaluated");
            assert!(t.valid > 0);
            let point_floor = d
                .families
                .iter()
                .filter(|f| matches!(f.family, "residual" | "rans_residual"))
                .map(|f| f.best_payload)
                .min()
                .unwrap_or(u64::MAX);
            let raw = d
                .families
                .iter()
                .find(|f| f.family == "raw")
                .map(|f| f.best_payload)
                .unwrap_or(u64::MAX);
            assert!(
                t.best_payload < point_floor && t.best_payload < raw,
                "transform must win the cost court on smooth deltas \
                 (transform {} < point {point_floor} / raw {raw})",
                t.best_payload
            );
        }
    }
    assert_eq!(transform_wins, 8, "every drift interval is transform-coded");
    // Far from raster-proportional: the whole 9-frame sequence is a fraction
    // of one raw frame's bytes.
    let one_raw = u64::from(w) * u64::from(h);
    assert!(
        (report.vole.len() as u64) * 4 < one_raw * 9,
        "{} B vs {one_raw} B/frame raw",
        report.vole.len()
    );
    // Decode and compare every frame to the reference.
    let decoded = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
    assert_eq!(decoded.len(), frames.len());
    for (i, f) in decoded.iter().enumerate() {
        assert_eq!(f.as_slice(), frames[i].as_slice(), "frame {i} exact");
    }
    Ok(())
}

#[test]
fn drifting_full_range_ramp_is_exact_and_uses_the_transform_floor() -> Result<(), VoleError> {
    // A wrap-around ramp across the full Gray8 scale is dense and smooth
    // within every block, so the Phase-M transform floor serves it — **until
    // Phase N**: the ramp is also an *exact integer gradient*, so the encoder
    // now discovers the procedural explanation instead and the transform
    // floor is never needed (measured post-N reality: every frame wins as
    // `generator`). Everything stays byte-exact; the pure-ramp court thus
    // demonstrates the ordering of explanations, not the floor.
    let (w, h) = (160u32, 120u32);
    let frames: Vec<Canvas> = (0..7)
        .map(|t| ramp_wrap_frame(w, h, (t as u64) * 7))
        .collect();
    let report = encode_default(&frames)?;
    assert!(report.exact);
    let wins: Vec<&str> = report.decisions.iter().map(|d| d.winner_family).collect();
    assert!(
        wins.iter().all(|f| *f == "generator"),
        "pure ramps are procedurally explained after Phase N: {wins:?}"
    );
    let decoded = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
    for (i, f) in decoded.iter().enumerate() {
        assert_eq!(f.as_slice(), frames[i].as_slice(), "frame {i} exact");
    }
    Ok(())
}

#[test]
fn textured_drift_is_exact_with_a_transform_winner() -> Result<(), VoleError> {
    let (w, h) = (128u32, 96u32);
    let frames: Vec<Canvas> = (0..8)
        .map(|t| textured_frame(w, h, (t as u64) * 3 + 1))
        .collect();
    let report = encode_default(&frames)?;
    assert!(report.exact);
    let wins: Vec<&str> = report.decisions.iter().map(|d| d.winner_family).collect();
    assert!(
        wins.contains(&"transform_residual"),
        "textured drift must use the transform floor at least once: {wins:?}"
    );
    let parsed = decoder::decode_bytes(&report.vole)?;
    let decoded = decoder::materialize_all(&parsed)?;
    for (i, f) in decoded.iter().enumerate() {
        assert_eq!(f.as_slice(), frames[i].as_slice(), "frame {i} exact");
    }
    // The transform payload's accounting buckets sum to the stream length.
    let cost = inverse::account_stream(&report.vole)?;
    let sum = cost.header_bytes
        + cost.object_bytes
        + cost.checkpoint_bytes
        + cost.transition_bytes
        + cost.residual_bytes
        + cost.model_bytes
        + cost.state_bytes
        + cost.dictionary_bytes
        + cost.index_bytes
        + cost.integrity_bytes;
    assert_eq!(sum, cost.total_bytes);
    assert_eq!(cost.total_bytes, report.vole.len() as u64);
    Ok(())
}

#[test]
fn noise_negative_control_stays_raw() -> Result<(), VoleError> {
    // Full-canvas noise has no smooth structure: the transform cannot compact
    // it, so RAW must win every frame and transform_residual never wins.
    let (w, h) = (64u32, 48u32);
    let frames: Vec<Canvas> = (0..6)
        .map(|t| noise_frame(w, h, 1000 + (t as u64)))
        .collect();
    let report = encode_default(&frames)?;
    assert!(report.exact);
    for d in &report.decisions {
        assert_ne!(
            d.winner_family, "transform_residual",
            "noise must not be transform-coded"
        );
    }
    Ok(())
}

#[test]
fn tiny_localized_change_never_pays_the_transform_overhead() -> Result<(), VoleError> {
    // One blinking pixel per frame: the diff gate (9k < mask + envelope)
    // means the transform family is not even evaluated; sparse stays the
    // winner after the frame-0 base.
    let (w, h) = (160u32, 120u32);
    let base = ramp_base(w, h);
    let mut frames = vec![base.clone()];
    for t in 1..=5u64 {
        let mut data = base.as_slice().to_vec();
        let (px, py) = (10usize + 5 * t as usize, 12usize);
        data[py * w as usize + px] ^= 0xFF;
        frames.push(canvas_of(w, h, data));
    }
    let report = encode_default(&frames)?;
    assert!(report.exact);
    for d in &report.decisions {
        if d.frame == 0 {
            continue; // frame 0 is the raw/fill base
        }
        assert_eq!(d.winner_family, "sparse", "blink stays sparse");
        assert!(
            !d.families.iter().any(|f| f.family == "transform_residual"),
            "tiny diffs must not evaluate the transform family"
        );
    }
    Ok(())
}

#[test]
fn oracle_invariant_holds_with_the_transform_family() -> Result<(), VoleError> {
    // Winner payload == minimum over every *evaluated* family.
    let (w, h) = (96u32, 64u32);
    let base = ramp_base(w, h);
    let mut frames = vec![base.clone()];
    for t in 1..=5u8 {
        frames.push(drifted(&base, t));
    }
    let report = encode_default(&frames)?;
    assert!(report.exact);
    for d in &report.decisions {
        let best = d
            .families
            .iter()
            .filter(|f| f.valid > 0)
            .map(|f| f.best_payload)
            .min()
            .expect("a valid family");
        assert_eq!(d.winner_payload_bytes, best, "oracle min over evaluated");
    }
    Ok(())
}

#[test]
fn transform_block_beats_the_point_residual_on_the_same_delta() -> Result<(), VoleError> {
    // Same delta both ways: the kind-2 block must be far smaller than the
    // Phase-G point-list container for a dense smooth field.
    let (w, h) = (64u32, 48u32);
    let base = ramp_base(w, h);
    let target = drifted(&base, 3);
    let block = inverse::build_transform_block(&base, &target).expect("block");
    let mut pts = Vec::new();
    for x in 0..w as i64 {
        for y in 0..h as i64 {
            let bv = base.get(x as u32, y as u32);
            let tv = target.get(x as u32, y as u32);
            if bv != tv {
                pts.push((x, y, tv));
            }
        }
    }
    assert_eq!(pts.len() as u64, u64::from(w) * u64::from(h), "dense delta");
    let mut bytes = Vec::with_capacity(9 * pts.len());
    for (x, y, v) in &pts {
        bytes.extend_from_slice(&i32::try_from(*x).unwrap().to_le_bytes());
        bytes.extend_from_slice(&i32::try_from(*y).unwrap().to_le_bytes());
        bytes.push(*v);
    }
    let point_block = rans::encode_block(&bytes);
    assert!(
        block.len() * 4 < point_block.len(),
        "transform floor must beat the point residual on smooth deltas: \
         {} vs {}",
        block.len(),
        point_block.len()
    );
    Ok(())
}

/// Build a minimal two-frame stream whose interval carries `block` verbatim
/// (frame 0 is a uniform background canvas). Byte layout of the result is
/// fully deterministic: header 24 | checkpoint 6 | interval envelope 13
/// (0x04 t:u64 count:u32) | residual op 5 (0x2a len:u32) | block, so the
/// block starts at offset 48.
fn stream_with_block(w: u32, h: u32, bg: u8, block: &[u8]) -> Result<Vec<u8>, VoleError> {
    StreamWriter::begin(w, h)
        .background(bg)
        .checkpoint_with(&[])?
        .interval(
            Interval(1),
            &[Transition::Residual {
                block: block.to_vec(),
            }],
        )?
        .finish()
}

#[test]
fn hostile_transform_streams_fail_typed_at_parse_and_materialize() -> Result<(), VoleError> {
    let (w, h) = (64u32, 48u32);
    let bg = 70u8;
    let base = canvas_of(w, h, vec![bg; (w * h) as usize]);
    let target = canvas_of(w, h, vec![bg + 1; (w * h) as usize]);
    let canonical = inverse::build_transform_block(&base, &target).expect("block");
    // The canonical stream decodes and materializes exactly.
    let good = stream_with_block(w, h, bg, &canonical)?;
    let parsed = decoder::decode_bytes(&good)?;
    let frames = decoder::materialize_all(&parsed)?;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].as_slice(), target.as_slice());
    let (payload, _trailer) = good.split_at(good.len() - 32);
    // Block starts at offset 48 (see stream_with_block); mask at 50..
    // 50+mlen; length prefixes at 50+mlen; dc container right after.
    const BLOCK_OFF: usize = 48;
    let mlen = vole_video::transform::mask_len(w, h);
    assert_eq!(payload[BLOCK_OFF], rans::KIND_TSF);
    assert_eq!(payload[BLOCK_OFF + 1], 0);
    let o = BLOCK_OFF + 2 + mlen;

    // Unknown transform id -> typed at parse (before the trailer runs).
    let mut b1 = payload.to_vec();
    b1[BLOCK_OFF + 1] = 9;
    assert_eq!(
        decoder::decode_bytes(&seal(&b1)).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );

    // dc_len disagreement -> typed at parse.
    let mut b2 = payload.to_vec();
    b2[o..o + 4].copy_from_slice(&999_999u32.to_le_bytes());
    assert_eq!(
        decoder::decode_bytes(&seal(&b2)).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );

    // Padding bit set (a mask bit past the last block) -> typed at parse.
    // A single-block canvas makes the padding bit trivially reachable.
    let (w1, h1) = (4u32, 4u32);
    let base1 = canvas_of(w1, h1, vec![bg; 16]);
    let target1 = canvas_of(w1, h1, vec![bg + 1; 16]);
    let bsmall = inverse::build_transform_block(&base1, &target1).expect("block");
    let good_small = stream_with_block(w1, h1, bg, &bsmall)?;
    let (payload_small, _t) = good_small.split_at(good_small.len() - 32);
    let mut b3 = payload_small.to_vec();
    const BLOCK_OFF_SMALL: usize = 48;
    b3[BLOCK_OFF_SMALL + 2] |= 0x02; // bit 1 is padding for a single block
    assert_eq!(
        decoder::decode_bytes(&seal(&b3)).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );

    // Out-of-Gray8 reconstruction is unreachable from a canonical encoder
    // Truncation of the payload is a typed parse error.
    let t = &payload[..payload.len() - 5];
    assert!(decoder::decode_bytes(&seal(t)).is_err());
    Ok(())
}

#[test]
fn transform_payload_out_of_range_reconstruction_is_typed() -> Result<(), VoleError> {
    // Hand-built hostile payload with a RAW dc container: non-uniform
    // high-entropy deltas make the DC byte stream incompressible, so the
    // container stays RAW and a huge zigzag coefficient lands in the payload
    // verbatim. Parse succeeds (structure is canonical); materialization must
    // fail typed `OutOfBounds` (the reconstructed sample cannot fit Gray8).
    let (w, h) = (64u32, 48u32);
    let bg = 70u8;
    let base = canvas_of(w, h, vec![bg; (w * h) as usize]);
    let mut s = 77u64;
    let mut data = Vec::with_capacity((w * h) as usize);
    for _ in 0..(w * h) {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        data.push(((s >> 56) as u16 + 1) as u8); // never equal to bg
    }
    let target = canvas_of(w, h, data);
    let block = inverse::build_transform_block(&base, &target).expect("block");
    // The canonical block carries the exact (in-range) delta: it decodes.
    let good = stream_with_block(w, h, bg, &block)?;
    assert_eq!(
        decoder::materialize_all(&decoder::decode_bytes(&good)?)?.len(),
        2
    );
    let (payload, _t) = good.split_at(good.len() - 32);
    const BLOCK_OFF: usize = 48;
    let mlen = vole_video::transform::mask_len(w, h);
    let dc_container = BLOCK_OFF + 2 + mlen + 8;
    assert_eq!(payload[dc_container], rans::KIND_RAW);
    let mut b = payload.to_vec();
    let huge = vole_video::transform::zigzag(1 << 29).to_le_bytes();
    let first_dc = dc_container + 9; // RAW payload starts after kind + len
    b[first_dc..first_dc + 4].copy_from_slice(&huge);
    let sealed = seal(&b);
    let p = decoder::decode_bytes(&sealed)?;
    assert_eq!(
        decoder::materialize_all(&p).unwrap_err(),
        VoleError::OutOfBounds
    );
    Ok(())
}
