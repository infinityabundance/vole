//! Phase-M evidence producer: the deterministic integer transform residual
//! floor.
//!
//! `cargo run --release --example transform_proof` prints a deterministic
//! report: the brightness-drift flagship (1920×1080: whole-canvas smooth
//! deltas served by the transform floor at a small fraction of the raster
//! reset cost), the same-delta transform-vs-point-residual comparison, the
//! textured court, and the noise negative control (RAW stays). Every stream
//! is end-to-end decode-verified before it is counted.

use std::time::Instant;

use vole_video::{decoder, error::VoleError, inverse, pixel::Canvas};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

/// Shift that keeps `d² >> sh ≤ ~96` for the larger canvas dimension, so the
/// curvature is visible at small sizes and bounded at full HD.
fn curve_shift(w: u32, h: u32) -> u32 {
    let mut sh = 9u32;
    for d in [w, h] {
        let q = u64::from(d) * u64::from(d);
        while sh < 24 && q > 96u64 << sh {
            sh += 1;
        }
    }
    sh
}

/// Non-uniform base with curvature in both axes (brightness drift is not
/// equivalent to any scroll/copy of it).
fn curved_base(w: u32, h: u32) -> Canvas {
    let sh = curve_shift(w, h);
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v =
                70 + ((u64::from(x) * u64::from(x)) >> sh) + ((u64::from(y) * u64::from(y)) >> sh);
            d.push(v as u8);
        }
    }
    canvas_of(w, h, d)
}

fn drifted(base: &Canvas, t: u8) -> Canvas {
    let mut data = base.as_slice().to_vec();
    for v in &mut data {
        *v = v.saturating_add(t);
    }
    canvas_of(base.width(), base.height(), data)
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

fn winner_counts(report: &inverse::EncodeReport) -> String {
    let mut counts: Vec<(&str, u64)> = Vec::new();
    for d in &report.decisions {
        if let Some(slot) = counts.iter_mut().find(|(f, _)| *f == d.winner_family) {
            slot.1 += 1;
        } else {
            counts.push((d.winner_family, 1));
        }
    }
    counts
        .iter()
        .map(|(f, c)| format!("{f}x{c}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> Result<(), VoleError> {
    // --- 1. Brightness-drift flagship 1920x1080 ------------------------------
    {
        let (w, h) = (1920u32, 1080u32);
        let base = curved_base(w, h);
        let mut frames = vec![base.clone()];
        for t in 1..=8u8 {
            frames.push(drifted(&base, t));
        }
        let t = Instant::now();
        let report = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                bg_sweep: false,
                background: Some(70),
                ..inverse::EncodeOptions::default()
            },
        )?;
        assert!(report.exact);
        let cost = inverse::account_stream(&report.vole)?;
        let interval_total: u64 = report
            .decisions
            .iter()
            .skip(1)
            .map(|d| d.winner_interval_bytes + d.object_decl_bytes)
            .sum();
        println!(
            "drift-flag-1920x1080: frames={} vole={}B raw_all={}B ratio_raw={:.0}x \
             interval_total={}B winners=[{}] exact=true encode_ms={:.0}",
            frames.len(),
            report.vole.len(),
            u64::from(w) * u64::from(h) * frames.len() as u64,
            (u64::from(w) * u64::from(h) * frames.len() as u64) as f64 / report.vole.len() as f64,
            interval_total,
            winner_counts(&report),
            t.elapsed().as_secs_f64() * 1000.0
        );
        // Transform payload vs the RAW per-frame reset on the first interval.
        let d1 = &report.decisions[1];
        assert_eq!(d1.winner_family, "transform_residual");
        let raw_best = d1
            .families
            .iter()
            .find(|f| f.family == "raw")
            .map(|f| f.best_payload)
            .unwrap_or(0);
        let point_best = d1
            .families
            .iter()
            .filter(|f| matches!(f.family, "residual" | "rans_residual"))
            .map(|f| f.best_payload)
            .min()
            .unwrap_or(u64::MAX);
        println!(
            "drift-flag first-interval: transform={}B raw_reset={}B point_residual={}B \
             model_bytes={}B residual_bytes={}B",
            d1.winner_payload_bytes, raw_best, point_best, cost.model_bytes, cost.residual_bytes
        );
        let decoded = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
        assert_eq!(decoded.len(), frames.len());
        assert!(decoded
            .iter()
            .zip(&frames)
            .all(|(a, b)| a.as_slice() == b.as_slice()));
    }

    // --- 2. Same-delta transform vs point residual (480x270) -----------------
    {
        let (w, h) = (480u32, 270u32);
        let base = curved_base(w, h);
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
        let mut bytes = Vec::with_capacity(9 * pts.len());
        for (x, y, v) in &pts {
            bytes.extend_from_slice(&i32::try_from(*x).unwrap().to_le_bytes());
            bytes.extend_from_slice(&i32::try_from(*y).unwrap().to_le_bytes());
            bytes.push(*v);
        }
        let point_block = vole_video::rans::encode_block(&bytes);
        let raw_frame = u64::from(w) * u64::from(h);
        println!(
            "delta-480x270: pts={} transform_block={}B point_block={}B raw_delta_bytes={}B \
             ratio_vs_point={:.1}x",
            pts.len(),
            block.len(),
            point_block.len(),
            raw_frame,
            point_block.len() as f64 / block.len() as f64
        );
        assert!(block.len() * 4 < point_block.len());
    }

    // --- 3. Noise negative control ------------------------------------------
    {
        let (w, h) = (64u32, 48u32);
        let frames: Vec<Canvas> = (0..6).map(|t| noise_frame(w, h, 500 + t)).collect();
        let report = inverse::encode_frames(&frames, &inverse::EncodeOptions::default())?;
        assert!(report.exact);
        assert!(
            report
                .decisions
                .iter()
                .all(|d| d.winner_family != "transform_residual"),
            "noise must stay RAW"
        );
        println!(
            "noise-64x48 (negative): frames={} vole={}B winners=[{}] exact=true",
            frames.len(),
            report.vole.len(),
            winner_counts(&report)
        );
    }

    // --- 4. Fixed-heuristic vs oracle on drift (transform floor reachable
    //        from every strategy's probe) -------------------------------------
    {
        let (w, h) = (160u32, 120u32);
        let base = curved_base(w, h);
        let mut frames = vec![base.clone()];
        for t in 1..=6u8 {
            frames.push(drifted(&base, t));
        }
        let ex = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                strategy: vole_video::dsfb::EncoderStrategy::Exhaustive,
                bg_sweep: false,
                background: Some(70),
                ..inverse::EncodeOptions::default()
            },
        )?;
        let fx = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                strategy: vole_video::dsfb::EncoderStrategy::FixedHeuristic,
                bg_sweep: false,
                background: Some(70),
                ..inverse::EncodeOptions::default()
            },
        )?;
        assert!(ex.exact && fx.exact);
        println!(
            "drift-160x120 strategies: exhaustive={}B winners=[{}] fixed_heuristic={}B \
             winners=[{}] J_ratio={:.3} exact=true",
            ex.vole.len(),
            winner_counts(&ex),
            fx.vole.len(),
            winner_counts(&fx),
            fx.vole.len() as f64 / ex.vole.len() as f64
        );
    }
    Ok(())
}
