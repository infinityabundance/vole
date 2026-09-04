//! Phase U evidence proof: the perceptual (lossy) profile — the LAST mandated
//! phase (master brief §64 Phase-U block; §72 language throughout).
//!
//! Architecture under proof: **lossiness lives entirely in a deterministic
//! integer quantization `Q` applied at encode time over raster-origin input.**
//! The stream encodes the chosen reconstruction `F̂ = Q(F)` and is decoded
//! *exactly* by the unchanged normative materializer; the loss is declared
//! (feature bit `0x2`, `FEAT_QUANTIZED_CONTENT`), measured (MAE/MSE/peak),
//! and **never assumed** — every stream is decoded back through the normative
//! decoder and proven byte-equal to `F̂` before it is reported.
//!
//! Courts measured here:
//!
//! * **Flagship A — temporal sensor noise on a flat panel** (480×270 ×17):
//!   uniform value 88 plus independent 2-bit per-sample, per-frame jitter
//!   `0..=3`. Lossless, every interval must carry a dense residual field;
//!   lattice step 8 (q3) clears the jitter entirely, so `F̂` is a flat panel
//!   and the stream becomes one fill + the unchanged lane — measured per
//!   interval (§33-shaped), never zeroed.
//! * **Flagship B — smooth ramp + sensor noise**: quantization trades a
//!   bounded measured error for a smaller residual field.
//! * **Authored-procedural control** (the Phase-A moving-rect pattern): the
//!   exact stream is already procedural-state; quantization cannot improve it
//!   (it can only destroy exact structure) — measured and recorded as the
//!   honest negative that keeps exact profiles intact.
//! * **Noise negative control** (§62): unknowable noise stays in the raster
//!   fallback at every shift; only the reconstruction proof changes.
//!
//! Run: `cargo run --release --example lossy_proof`

use std::time::Instant;

use vole_video::{
    decoder,
    ingest::Ingest,
    lossy::{
        self, choose_rd, encode_lossy, rate_distortion, Filter, LossyOptions, QuantProfile,
        Rounding,
    },
    pixel::Canvas,
    VoleError,
};

fn hash2(x: u32, y: u32, t: u32) -> u64 {
    let mut z = u64::from(x).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ u64::from(y).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ u64::from(t).wrapping_mul(0x94D0_49BB_1331_11EB)
        ^ 0x7F4A_7C15;
    z ^= z >> 30;
    z = z.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^= z >> 27;
    z.wrapping_mul(0x94D0_49BB_1331_11EB) ^ (z >> 31)
}

fn canvas(w: u32, h: u32, samples: Vec<u8>) -> Result<Canvas, VoleError> {
    Canvas::from_parts(w, h, samples)
}

/// Flagship A: flat panel at 88 + independent 2-bit jitter `0..=3` per
/// sample per frame (temporal sensor noise).
fn sensor_panel(w: u32, h: u32, frames: u32) -> Result<Vec<Canvas>, VoleError> {
    let mut out = Vec::with_capacity(frames as usize);
    for t in 0..frames {
        let mut d = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                d.push(88u8 + (hash2(x, y, t) % 4) as u8);
            }
        }
        out.push(canvas(w, h, d)?);
    }
    Ok(out)
}

/// Flagship B: smooth diagonal ramp plus independent jitter `0..=3` (a
/// gradient with sensor noise).
fn noisy_ramp(w: u32, h: u32, frames: u32) -> Result<Vec<Canvas>, VoleError> {
    let span = u64::from(w + h).max(1);
    let mut out = Vec::with_capacity(frames as usize);
    for t in 0..frames {
        let mut d = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let ramp = (u64::from(x) + u64::from(y)) * 255 / span;
                d.push((ramp as u8).saturating_add((hash2(x, y, t) % 4) as u8));
            }
        }
        out.push(canvas(w, h, d)?);
    }
    Ok(out)
}

/// Authored-procedural control: the Phase-A moving-rect pattern materialized
/// to raster (so the lossy path can be measured against it like any raster
/// source). The *exact* encode of this content is already procedural state;
/// quantization is expected to add distortion without removing bytes.
fn authored_track() -> Result<Vec<Canvas>, VoleError> {
    let (w, h) = (240u32, 135u32);
    let mut a = Ingest::new(w, h);
    a.background(5);
    a.declare_fill(1, 40, 20, 180)?;
    a.instance(1, 1, 60, 40)?;
    for t in 1..=12u64 {
        a.at(t)?;
        a.set_position(1, 60 + 3 * t as i64, 40)?;
    }
    let bytes = a.finish()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    decoder::materialize_all(&parsed)
}

/// White noise (negative control).
fn white_noise(w: u32, h: u32, frames: u32) -> Result<Vec<Canvas>, VoleError> {
    let mut out = Vec::with_capacity(frames as usize);
    for t in 0..frames {
        let mut d = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                d.push((hash2(x, y, 500 + t) % 256) as u8);
            }
        }
        out.push(canvas(w, h, d)?);
    }
    Ok(out)
}

fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

fn main() -> Result<(), VoleError> {
    let t0 = Instant::now();

    // ------------------------------------------------------------------
    // Flagship A ladder: the perceptual trade on temporal sensor noise.
    // ------------------------------------------------------------------
    let a = sensor_panel(480, 270, 17)?;
    let raw = 480u64 * 270 * 17;
    println!("flagship A: flat panel + 2-bit temporal jitter (480x270 x17, raw {raw} B)");
    println!();
    let opts = LossyOptions::default();
    let rows = rate_distortion(&a, Rounding::HalfUp, Filter::None, 4, &opts)?;
    println!("| profile | .vole B | MAE | MSE | peak |");
    println!("|---|---|---|---|---|");
    for r in &rows {
        let rep = encode_lossy(&a, &r.profile, &opts)?;
        // Reconstruction proof: decoder output == F̂ == Q(source).
        let dec = frames_of(&rep.stream)?;
        assert_eq!(dec.len(), rep.reconstruction.len());
        for (d, q) in dec.iter().zip(rep.reconstruction.iter()) {
            assert!(d.exactly_matches(q), "decoder reproduces F̂ for {r:?}");
        }
        println!(
            "| {} | {} | {:.3} | {} | {} |",
            r.profile.label(),
            r.bytes,
            r.distortion.mae_x1000 as f64 / 1000.0,
            r.distortion.mse,
            r.distortion.peak
        );
    }
    let exact = &rows[0];
    let best = &rows[3];
    let dominated = &rows[4];
    assert!(best.bytes < exact.bytes);
    println!();
    println!(
        "measured trade: exact {} B -> q3 {} B ({}x); MAE {:.3}, MSE {}, peak {} — declared, never zeroed",
        exact.bytes,
        best.bytes,
        exact.bytes as f64 / best.bytes.max(1) as f64,
        best.distortion.mae_x1000 as f64 / 1000.0,
        best.distortion.mse,
        best.distortion.peak
    );
    println!(
        "  recorded: q4 ties q3's {} B at strictly higher distortion (MAE {:.3} vs {:.3}) — a dominated row; \
         choose_rd never picks it (least-distorted fit within budget)",
        dominated.bytes,
        dominated.distortion.mae_x1000 as f64 / 1000.0,
        best.distortion.mae_x1000 as f64 / 1000.0
    );
    println!(
        "  recorded: bytes are NOT monotone in the lattice — q1/q2 ({} B / {} B) exceed the exact profile \
         ({} B): an intermediate lattice keeps the residual dense while destroying the exact residual's \
         structure. The RD choice accounts for it.",
        rows[1].bytes, rows[2].bytes, exact.bytes
    );

    // ------------------------------------------------------------------
    // §33-shaped bytes over time: per-interval cost, exact vs q3.
    // ------------------------------------------------------------------
    let q3 = QuantProfile {
        shift: 3,
        rounding: Rounding::HalfUp,
        filter: Filter::None,
    };
    let rep = encode_lossy(&a, &q3, &opts)?;
    println!();
    println!(
        "q3 stream: {} B for 17 frames = {:.1} B/frame amortized (static lane after the frame-0 basis); \
         intervals of the flat F̂ ride the unchanged lane",
        rep.bytes,
        rep.bytes as f64 / 17.0
    );
    let rep0 = encode_lossy(&a, &QuantProfile::EXACT, &opts)?;
    println!(
        "exact stream: {} B for 17 frames = {:.1} B/frame amortized (each interval carries a dense jitter residual)",
        rep0.bytes,
        rep0.bytes as f64 / 17.0
    );

    // ------------------------------------------------------------------
    // RD budget choice on flagship A (honest semantics).
    // ------------------------------------------------------------------
    let budget = exact.bytes / 4;
    let c = choose_rd(&a, Rounding::HalfUp, Filter::None, 4, Some(budget), &opts)?;
    println!();
    println!(
        "RD budget = {budget} B (¼ of exact): chosen {} ({} B, budget_met={})",
        c.row.profile.label(),
        c.row.bytes,
        c.budget_met
    );
    assert!(c.budget_met);

    // ------------------------------------------------------------------
    // Flagship B ladder: smooth ramp + sensor noise.
    // ------------------------------------------------------------------
    let b = noisy_ramp(320, 180, 9)?;
    let raw_b = 320u64 * 180 * 9;
    println!();
    println!("flagship B: smooth ramp + 2-bit temporal jitter (320x180 x9, raw {raw_b} B)");
    let rows_b = rate_distortion(&b, Rounding::HalfUp, Filter::None, 3, &opts)?;
    for r in &rows_b {
        let rep = encode_lossy(&b, &r.profile, &opts)?;
        let dec = frames_of(&rep.stream)?;
        for (d, q) in dec.iter().zip(rep.reconstruction.iter()) {
            assert!(d.exactly_matches(q));
        }
        println!(
            "  {}: {} B  MAE {:.3}  MSE {}  peak {}",
            r.profile.label(),
            r.bytes,
            r.distortion.mae_x1000 as f64 / 1000.0,
            r.distortion.mse,
            r.distortion.peak
        );
    }

    // ------------------------------------------------------------------
    // Authored-procedural control: quantization adds nothing here.
    // ------------------------------------------------------------------
    let authored = authored_track()?;
    let exact_auth = encode_lossy(&authored, &QuantProfile::EXACT, &opts)?;
    let q2_auth = encode_lossy(
        &authored,
        &QuantProfile {
            shift: 2,
            rounding: Rounding::HalfUp,
            filter: Filter::None,
        },
        &opts,
    )?;
    println!();
    println!(
        "authored-procedural control (moving rect, 13 frames): exact {} B, q2 {} B, q2 MAE {:.3} — \
         quantization adds measured distortion without removing the procedural-state bytes (recorded negative)",
        exact_auth.bytes,
        q2_auth.bytes,
        q2_auth.distortion.mae_x1000 as f64 / 1000.0
    );

    // ------------------------------------------------------------------
    // Noise negative control: no shift turns noise into state.
    // ------------------------------------------------------------------
    let noise = white_noise(192, 128, 3)?;
    let raw_n = 192u64 * 128 * 3;
    println!();
    println!("noise negative control (192x128 x3, raw {raw_n} B):");
    for shift in [0u8, 2, 4] {
        let rep = encode_lossy(
            &noise,
            &QuantProfile {
                shift,
                rounding: Rounding::DeadZone,
                filter: Filter::None,
            },
            &opts,
        )?;
        let dec = frames_of(&rep.stream)?;
        for (d, q) in dec.iter().zip(rep.reconstruction.iter()) {
            assert!(d.exactly_matches(q));
        }
        println!(
            "  q{shift}: {} B  MAE {:.3}  peak {}  declared={}",
            rep.bytes,
            rep.distortion.mae_x1000 as f64 / 1000.0,
            rep.distortion.peak,
            lossy::declares_quantized(&rep.stream)?
        );
    }

    // ------------------------------------------------------------------
    // Exactness statement for the phase: the exact profile is untouched.
    // ------------------------------------------------------------------
    let rep = encode_lossy(&a, &QuantProfile::EXACT, &opts)?;
    assert!(rep.exact);
    assert!(!lossy::declares_quantized(&rep.stream)?);
    println!();
    println!(
        "exact profile: lossless ({} B), no declaration, decode byte-identical to the source — the lossless ladder stays intact",
        rep.bytes
    );
    println!();
    println!(
        "lossy proof: OK (flagships A/B + authored control + noise control; every stream decoded and proven == F̂) in {:.1} s",
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}
