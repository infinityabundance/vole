//! Phase-K evidence producer: variable regions in the raster-origin encoder.
//!
//! `cargo run --release --example regions_proof` prints a deterministic
//! report: the localized-change flagship at 1920×1080 (zero whole-frame
//! rebases after frame 0), a region exact-ref reuse court across the three
//! search strategies (Exhaustive vs FixedHeuristic vs DsfbGuided over the
//! same candidate universe), rectangular-region content, and the noise
//! negative control. Every stream is decode-verified end-to-end by the
//! encoder before it is returned.

use std::time::Instant;

use vole_video::{decoder, dsfb::EncoderStrategy, error::VoleError, inverse, pixel::Canvas};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

/// Structured, non-uniform deterministic base (blocks), so no representation
/// can ride on whole-frame fill objects.
fn base_canvas(w: u32, h: u32) -> Canvas {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = ((x / 17 + y / 11) % 5) as u8 * 37 + 20;
            d.push(v);
        }
    }
    canvas_of(w, h, d)
}

/// Overwrite a rectangle of `base` with deterministic pseudo-random bytes.
#[allow(clippy::too_many_arguments)]
fn patch_rect(
    w: u32,
    h: u32,
    base: &Canvas,
    x: i64,
    y: i64,
    rw: u32,
    rh: u32,
    seed: u64,
) -> Canvas {
    let (_, _, mut data) = base.clone().into_parts();
    let mut s = seed.max(1);
    let mut byte = move || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (s >> 56) as u8
    };
    for dy in 0..rh as i64 {
        for dx in 0..rw as i64 {
            let cx = x + dx;
            let cy = y + dy;
            if cx >= 0 && cy >= 0 && cx < i64::from(w) && cy < i64::from(h) {
                data[cy as usize * w as usize + cx as usize] = byte();
            }
        }
    }
    canvas_of(w, h, data)
}

fn winners(r: &inverse::EncodeReport) -> String {
    let mut counts: Vec<(&str, u64)> = Vec::new();
    for d in &r.decisions {
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

fn rebases_after_frame0(r: &inverse::EncodeReport) -> u64 {
    r.decisions
        .iter()
        .filter(|d| d.frame > 0 && d.winner_family == "raw" && d.object_decl_bytes > 0)
        .count() as u64
}

fn run_strategy(
    frames: &[Canvas],
    strategy: EncoderStrategy,
    bg: u8,
) -> Result<inverse::EncodeReport, VoleError> {
    let opts = inverse::EncodeOptions {
        bg_sweep: false,
        background: Some(bg),
        strategy,
        ..inverse::EncodeOptions::default()
    };
    inverse::encode_frames(frames, &opts)
}

fn main() -> Result<(), VoleError> {
    // --- 1. Flagship: localized changes at 1920x1080 ------------------------
    {
        let (w, h) = (1920u32, 1080u32);
        let base = base_canvas(w, h);
        let mut frames = vec![base.clone()];
        for k in 1..=40u64 {
            // A "clock panel" whose content changes every frame.
            let mut f = patch_rect(w, h, &base, 60, 200, 200, 120, 0x5EED + k);
            // A full-width status bar that changes every 8 frames (rectangular
            // region: w x 16).
            if k % 8 == 0 {
                f = patch_rect(w, h, &f, 0, 900, w, 16, 0xC0DE + k / 8);
            }
            frames.push(f);
        }
        let t = Instant::now();
        let report = run_strategy(&frames, EncoderStrategy::Exhaustive, base.as_slice()[0])?;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        assert!(report.exact);
        let raw_all = report.raw_raster_bytes;
        let interval = report.vole.len() as u64 - (24 + 32 + 13 + u64::from(w) * u64::from(h));
        println!(
            "flag-local-1920x1080: frames={} vole={}B raw_all={}B ratio_raw={:.0}x \
             interval_bytes={}B winners=[{}] rebases_after_frame0={} exact=true \
             encode_ms={:.0}",
            report.frame_count,
            report.vole.len(),
            raw_all,
            raw_all as f64 / report.vole.len() as f64,
            interval,
            winners(&report),
            rebases_after_frame0(&report),
            ms
        );
        assert_eq!(rebases_after_frame0(&report), 0);
    }

    // --- 2. Region exact-ref reuse court, three strategies -------------------
    {
        let (w, h) = (160u32, 120u32);
        let base = base_canvas(w, h);
        let still = patch_rect(w, h, &base, 40, 24, 40, 24, 0xAB00 + 5);
        let mut frames = vec![base.clone()];
        for k in 1..=5u64 {
            let mut f = patch_rect(w, h, &base, 40, 24, 40, 24, 0xAB00 + k);
            f = patch_rect(
                w,
                h,
                &f,
                96,
                40,
                16,
                16,
                if k % 2 == 0 { 0x1111 } else { 0x2222 },
            );
            frames.push(f);
        }
        for k in 6..=36u64 {
            let f = patch_rect(
                w,
                h,
                &still,
                96,
                40,
                16,
                16,
                if k % 2 == 0 { 0x1111 } else { 0x2222 },
            );
            frames.push(f);
        }
        let oracle = run_strategy(&frames, EncoderStrategy::Exhaustive, base.as_slice()[0])?;
        println!(
            "reuse-court-160x120: strategy=exhaustive vole={}B winners=[{}] exact=true",
            oracle.vole.len(),
            winners(&oracle)
        );
        for strategy in [EncoderStrategy::FixedHeuristic, EncoderStrategy::DsfbGuided] {
            let r = run_strategy(&frames, strategy, base.as_slice()[0])?;
            assert!(r.exact);
            let n: u64 = r.decisions.iter().map(|d| d.candidates_evaluated).sum();
            let on: u64 = oracle
                .decisions
                .iter()
                .map(|d| d.candidates_evaluated)
                .sum();
            println!(
                "reuse-court-160x120: strategy={} vole={}B J_ratio={:.4} N_ratio={:.4} \
                 winners=[{}] exact=true",
                strategy.label(),
                r.vole.len(),
                r.vole.len() as f64 / oracle.vole.len() as f64,
                n as f64 / on as f64,
                winners(&r)
            );
        }
    }

    // --- 3. Rectangular + granularity content --------------------------------
    {
        let (w, h) = (128u32, 96u32);
        let base = base_canvas(w, h);
        let mut frames = vec![base.clone()];
        for k in 1..=16u64 {
            // Full-width banner (w x 8, rectangular) plus a 24x24 block plus a
            // 8x8 cell: three different region shapes per frame.
            let mut f = patch_rect(w, h, &base, 0, 40, w, 8, 0x31 + k);
            f = patch_rect(w, h, &f, 100, 10, 24, 24, 0x77 + k);
            f = patch_rect(w, h, &f, 8, 70, 8, 8, 0x99 + k);
            frames.push(f);
        }
        let report = run_strategy(&frames, EncoderStrategy::Exhaustive, base.as_slice()[0])?;
        assert!(report.exact);
        assert_eq!(rebases_after_frame0(&report), 0);
        // Region rectangles created by the winners (CreateInstance ops).
        let creates: u64 = report
            .decisions
            .iter()
            .filter(|d| d.winner_family == "regions")
            .map(|d| {
                d.emitted
                    .iter()
                    .filter(|t| {
                        matches!(t, vole_video::transition::Transition::CreateInstance { .. })
                    })
                    .count() as u64
            })
            .sum();
        println!(
            "rect-shapes-128x96: frames={} vole={}B winners=[{}] region_creates={} \
             exact=true",
            report.frame_count,
            report.vole.len(),
            winners(&report),
            creates
        );
    }

    // --- 4. Noise negative control ------------------------------------------
    {
        let (w, h) = (48u32, 32u32);
        let mut seed = 0x0BAD_F00Du64;
        let mut frames = Vec::new();
        for _ in 0..12 {
            let mut d = Vec::with_capacity((w * h) as usize);
            for _ in 0..(w * h) {
                seed ^= seed >> 12;
                seed ^= seed << 25;
                seed ^= seed >> 27;
                d.push((seed >> 56) as u8);
            }
            frames.push(canvas_of(w, h, d));
        }
        let report = run_strategy(
            &frames,
            EncoderStrategy::Exhaustive,
            frames[0].as_slice()[0],
        )?;
        assert!(report.exact);
        println!(
            "noise-48x32: frames={} vole={}B raw_all={}B winners=[{}] exact=true",
            report.frame_count,
            report.vole.len(),
            report.raw_raster_bytes,
            winners(&report)
        );
        let _ = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
    }
    Ok(())
}
