//! Phase-H evidence producer: Exhaustive vs FixedHeuristic vs DsfbGuided.
//!
//! `cargo run --release --example dsfb_proof` prints a deterministic report
//! for the evidence campaign: per court and strategy — final bytes, candidate
//! counts, search work, winner-family histogram, local-rebase events, byte
//! exactness — plus the regime-change court's measured adaptation latencies
//! and the DSFB diagnostics (`φ`/`ω`/`α`) sampled across a regime switch.
//! Every stream is decode-verified end-to-end by the encoder before it is
//! returned; the encoder refuses to return an unverified stream.

use std::time::Instant;

use vole_video::{
    dsfb::{DsfbFrameDiag, EncoderStrategy},
    error::VoleError,
    inverse::{self, EncodeOptions},
    pixel::Canvas,
};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

fn paint_boxes(w: u32, h: u32, bg: u8, boxes: &[(i64, i64, u32, u32, u8)]) -> Canvas {
    let mut data = vec![bg; (w * h) as usize];
    for (bx, by, bw, bh, v) in boxes {
        for dy in 0..*bh as i64 {
            for dx in 0..*bw as i64 {
                let x = bx + dx;
                let y = by + dy;
                if x >= 0 && y >= 0 && x < i64::from(w) && y < i64::from(h) {
                    data[y as usize * w as usize + x as usize] = *v;
                }
            }
        }
    }
    canvas_of(w, h, data)
}

fn textured(w: u32, h: u32, bg: u8, seed: u64) -> Canvas {
    let mut rng = Det(seed);
    let mut data = vec![bg; (w * h) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            if rng.byte().is_multiple_of(3) {
                data[y * w as usize + x] = rng.byte();
            }
        }
    }
    canvas_of(w, h, data)
}

struct Det(u64);
impl Det {
    fn next(&mut self) -> u64 {
        let mut x = self.0.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        self.0 = x;
        x
    }
    fn byte(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }
}

fn families(r: &inverse::EncodeReport) -> String {
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

fn rebases(r: &inverse::EncodeReport) -> u64 {
    r.decisions
        .iter()
        .filter(|d| d.frame > 0 && d.winner_family == "raw" && d.object_decl_bytes > 0)
        .count() as u64
}

/// Run the exhaustive oracle and print its own row (the reference totals
/// that the other strategies are compared against).
fn run_oracle(label: &str, frames: &[Canvas], bg: Option<u8>) -> inverse::EncodeReport {
    let r = run_strategy(frames, EncoderStrategy::Exhaustive, bg);
    print_strategy_row(label, EncoderStrategy::Exhaustive, frames, None, bg);
    r
}

fn run_strategy(
    frames: &[Canvas],
    strategy: EncoderStrategy,
    bg: Option<u8>,
) -> inverse::EncodeReport {
    let opts = EncodeOptions {
        bg_sweep: false,
        background: bg,
        strategy,
        ..EncodeOptions::default()
    };
    let t = Instant::now();
    let r = inverse::encode_frames(frames, &opts).expect("encode");
    assert!(r.exact);
    eprintln!(
        "  [{:?}] {}ms",
        strategy,
        t.elapsed().as_secs_f64() * 1000.0
    );
    r
}

fn print_strategy_row(
    label: &str,
    strategy: EncoderStrategy,
    frames: &[Canvas],
    oracle: Option<&inverse::EncodeReport>,
    bg: Option<u8>,
) {
    let r = run_strategy(frames, strategy, bg);
    let n: u64 = r.decisions.iter().map(|d| d.candidates_evaluated).sum();
    let work: u64 = r.decisions.iter().map(|d| d.search_work).sum();
    let oracle_line = match oracle {
        Some(o) => {
            let on: u64 = o.decisions.iter().map(|d| d.candidates_evaluated).sum();
            let owork: u64 = o.decisions.iter().map(|d| d.search_work).sum();
            format!(
                " oracle_N={on} oracle_work={owork} N_ratio={:.3} work_ratio={:.3} J_ratio={:.3}",
                n as f64 / on as f64,
                work as f64 / owork as f64,
                r.vole.len() as f64 / o.vole.len() as f64
            )
        }
        None => String::new(),
    };
    println!(
        "court={label} strategy={} frames={} vole={}B raw={}B exact=true rebases={} \
         candidates={} work={} winners=[{}]{}",
        strategy.label(),
        r.frame_count,
        r.vole.len(),
        r.raw_raster_bytes,
        rebases(&r),
        n,
        work,
        families(&r),
        oracle_line
    );
}

/// Steady pan content (box gliding inside a 64x64 canvas over uniform 90).
fn pan_frames() -> Vec<Canvas> {
    let (w, h) = (64u32, 64u32);
    (0..40i64)
        .map(|k| paint_boxes(w, h, 90, &[(8 + k, 12 + k, 10, 6, 180)]))
        .collect()
}

/// Wrap-by-7 content with distinct non-zero rows (40x32).
fn wrap_frames() -> Vec<Canvas> {
    let (w, h) = (40u32, 32u32);
    let f0 = canvas_of(w, h, {
        let mut d = Vec::with_capacity((w * h) as usize);
        for r in 0..h {
            for _ in 0..w {
                d.push(((r + 1) % 251) as u8);
            }
        }
        d
    });
    let s = 7i64;
    let mut frames = vec![f0.clone()];
    for t in 1..=24u32 {
        let mut d = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            let src = (y + t as usize * s as usize) % h as usize;
            d.extend_from_slice(&f0.as_slice()[src * w as usize..(src + 1) * w as usize]);
        }
        frames.push(canvas_of(w, h, d));
    }
    frames
}

/// Static then blink content.
fn blink_frames() -> Vec<Canvas> {
    let (w, h) = (32u32, 24u32);
    let base = paint_boxes(w, h, 70, &[(4, 4, 20, 10, 150)]);
    let mut frames: Vec<Canvas> = std::iter::repeat_n(base.clone(), 16).collect();
    for k in 0..16u8 {
        let idx = (10 * w + 20) as usize;
        let (_, _, mut data) = base.clone().into_parts();
        data[idx] = if k.is_multiple_of(2) { 255 } else { 70 };
        frames.push(canvas_of(w, h, data));
    }
    frames
}

/// Regime sequence: static textured scene -> wrap-by-7 -> noise -> pan.
fn regime_frames() -> Vec<Canvas> {
    let (w, h) = (32u32, 32u32);
    let scene = {
        let (_, _, mut d) = textured(w, h, 90, 7).into_parts();
        for y in 6..14usize {
            for x in 6..18usize {
                d[y * w as usize + x] = 180;
            }
        }
        canvas_of(w, h, d)
    };
    let mut frames: Vec<Canvas> = std::iter::repeat_n(scene.clone(), 24).collect();
    let s = 7i64;
    for _ in 0..22u32 {
        let prev = frames.last().unwrap().clone();
        let mut d = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            let src = (y + s as usize) % h as usize;
            d.extend_from_slice(&prev.as_slice()[src * w as usize..(src + 1) * w as usize]);
        }
        frames.push(canvas_of(w, h, d));
    }
    let mut rng = Det(0xC0FFEE);
    for _ in 0..14 {
        let mut d = Vec::with_capacity((w * h) as usize);
        for _ in 0..(w * h) {
            d.push(rng.byte());
        }
        frames.push(canvas_of(w, h, d));
    }
    for k in 0..18u32 {
        let mut d = vec![90u8; (w * h) as usize];
        let kk = k as usize;
        for y in 0..h as usize {
            if y + kk < h as usize {
                d[(y + kk) * w as usize..(y + kk + 1) * w as usize]
                    .copy_from_slice(&scene.as_slice()[y * w as usize..(y + 1) * w as usize]);
            }
        }
        frames.push(canvas_of(w, h, d));
    }
    frames
}

fn main() -> Result<(), VoleError> {
    // --- steady pan ---------------------------------------------------------
    let pan = pan_frames();
    let oracle = run_oracle("steady-pan-64x64", &pan, Some(90));
    print_strategy_row(
        "steady-pan-64x64",
        EncoderStrategy::FixedHeuristic,
        &pan,
        Some(&oracle),
        Some(90),
    );
    print_strategy_row(
        "steady-pan-64x64",
        EncoderStrategy::DsfbGuided,
        &pan,
        Some(&oracle),
        Some(90),
    );
    let oracle_pan = &oracle;

    // --- steady wrap by 7 ----------------------------------------------------
    let wrap = wrap_frames();
    let oracle_w = run_oracle("wrap-by-7-40x32", &wrap, Some(0));
    print_strategy_row(
        "wrap-by-7-40x32",
        EncoderStrategy::FixedHeuristic,
        &wrap,
        Some(&oracle_w),
        Some(0),
    );
    print_strategy_row(
        "wrap-by-7-40x32",
        EncoderStrategy::DsfbGuided,
        &wrap,
        Some(&oracle_w),
        Some(0),
    );

    // --- static + blink ------------------------------------------------------
    let blink = blink_frames();
    let oracle_b = run_oracle("static-blink-32x24", &blink, None);
    print_strategy_row(
        "static-blink-32x24",
        EncoderStrategy::DsfbGuided,
        &blink,
        Some(&oracle_b),
        None,
    );

    // --- regime change -------------------------------------------------------
    let regime = regime_frames();
    let oracle_r = run_oracle("regime-32x32", &regime, Some(90));
    print_strategy_row(
        "regime-32x32",
        EncoderStrategy::FixedHeuristic,
        &regime,
        Some(&oracle_r),
        Some(90),
    );
    print_strategy_row(
        "regime-32x32",
        EncoderStrategy::DsfbGuided,
        &regime,
        Some(&oracle_r),
        Some(90),
    );

    // Adaptation latency per oracle regime switch (frames until the guided
    // winner equals the oracle winner for every remaining frame of the
    // segment).
    let dsfb = run_strategy(&regime, EncoderStrategy::DsfbGuided, Some(90));
    let ex_fams: Vec<&str> = oracle_r.decisions.iter().map(|d| d.winner_family).collect();
    let ds_fams: Vec<&str> = dsfb.decisions.iter().map(|d| d.winner_family).collect();
    let mut switches = Vec::new();
    for i in 1..ex_fams.len() {
        if ex_fams[i] != ex_fams[i - 1] {
            switches.push(i);
        }
    }
    for &sw in &switches {
        let mut latency: Option<usize> = None;
        for i in sw..ds_fams.len() {
            let next_switch = switches.iter().find(|s| **s > i);
            let seg_end = next_switch.copied().unwrap_or(ds_fams.len());
            if (i..seg_end).all(|j| ds_fams[j] == ex_fams[j]) {
                latency = Some(i - sw);
                break;
            }
        }
        println!(
            "regime-switch: at_frame={} oracle_winner={} guided_recovery_latency_frames={:?}",
            sw,
            ex_fams[sw],
            latency.map(|l| l as u64)
        );
    }

    // DSFB diagnostics sampled across the first regime switch.
    let d = &dsfb;
    let switch = switches[0];
    for idx in [switch - 2, switch, switch + 1, switch + 3] {
        if let Some(fd) = d.decisions.get(idx) {
            if let Some(diag) = &fd.dsfb_diag {
                print_diag_sample(idx, diag);
            }
        }
    }
    // Tail diagnostics: steady pan after the last switch.
    let last = d.decisions.last().unwrap();
    if let Some(diag) = &last.dsfb_diag {
        print_diag_sample(d.decisions.len() - 1, diag);
    }
    let _ = oracle_pan;
    Ok(())
}

fn print_diag_sample(frame: usize, d: &DsfbFrameDiag) {
    let phi = d
        .phi
        .iter()
        .map(|(f, v)| format!("{f}:{v:.2}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "dsfb-diag frame={frame} winner={} payload={}B alpha={} broadened={} since_regime={} \
         omega={:.4} active=[{}] phi={{ {} }} total_evaluated={}",
        d.winner,
        d.winner_payload,
        d.alpha,
        d.broadened,
        d.since_regime,
        d.omega,
        d.active.join(" "),
        phi,
        d.total_evaluated
    );
}
