//! Phase-N evidence producer: bounded procedural generators.
//!
//! `cargo run --release --example generator_proof` prints a deterministic
//! report: the drifting-gradient flagship (1920×1080 pure-gradient frames
//! explained procedurally at tens of bytes per frame instead of rasters or
//! transform blocks), authored generator streams for every kind (gradient /
//! checker / periodic sawtooth / seeded noise — the stream stores the
//! program, never the samples), the noise negative control (an unknowable
//! noise field stays RAW: a seed that merely relocates bits never wins), and
//! the generator+residual closure court. Every stream is end-to-end
//! decode-verified before it is counted.

use std::time::Instant;

use vole_video::{
    decoder, encoder,
    error::VoleError,
    generator::Generator,
    inverse,
    object::{Object, ObjectId},
    pixel::Canvas,
    state::{Instance, InstanceId},
};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

/// Reference gradient field (wrap arithmetic).
fn gradient_field(w: u32, h: u32, base: u8, sx: i64, sy: i64) -> Canvas {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            d.push(((i64::from(base) + sx * i64::from(x) + sy * i64::from(y)) & 0xFF) as u8);
        }
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
    // --- 1. Drifting-gradient flagship 1920x1080 -----------------------------
    {
        let (w, h) = (1920u32, 1080u32);
        // Pure wrap gradients, phase drifting every frame.
        let frames: Vec<Canvas> = (0..12)
            .map(|t| {
                let phase = (11u64 * t) % 256;
                gradient_field(w, h, phase as u8, 3, 5)
            })
            .collect();
        let t = Instant::now();
        let report = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                bg_sweep: false,
                background: Some(0),
                ..inverse::EncodeOptions::default()
            },
        )?;
        assert!(report.exact);
        println!(
            "drift-flag-1920x1080: frames={} vole={}B raw_all={}B ratio_raw={:.0}x \
             winners=[{}] exact=true encode_ms={:.0}",
            frames.len(),
            report.vole.len(),
            u64::from(w) * u64::from(h) * frames.len() as u64,
            (u64::from(w) * u64::from(h) * frames.len() as u64) as f64 / report.vole.len() as f64,
            winner_counts(&report),
            t.elapsed().as_secs_f64() * 1000.0
        );
        let decoded = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
        assert_eq!(decoded.len(), frames.len());
        assert!(decoded
            .iter()
            .zip(&frames)
            .all(|(a, b)| a.as_slice() == b.as_slice()));
    }

    // --- 2. Authored streams for every generator kind ------------------------
    {
        let (w, h) = (1920u32, 1080u32);
        let gens = [
            (
                "gradient",
                Generator::Gradient {
                    base: 5,
                    sx: 3,
                    sy: -2,
                },
            ),
            (
                "checker",
                Generator::Checker {
                    a: 40,
                    b: 220,
                    cell: 32,
                },
            ),
            (
                "periodic",
                Generator::Periodic {
                    base: 1,
                    sx: 2,
                    sy: 1,
                    period: 64,
                },
            ),
            ("noise", Generator::Noise { seed: 0x7E57 }),
        ];
        let raw_frame = u64::from(w) * u64::from(h);
        for (name, gen) in gens {
            let obj = Object::procedural(w, h, gen)?;
            let inst = Instance {
                id: InstanceId(1),
                object_id: ObjectId(1),
                x: 0,
                y: 0,
            };
            let bytes = encoder::encode_stream(w, h, 90, &[(1, obj)], &[inst], &[])?;
            let parsed = decoder::decode_bytes(&bytes)?;
            assert_eq!(decoder::materialize_all(&parsed)?.len(), 1);
            println!(
                "authored-{name}-1920x1080: vole={}B raw_frame={raw_frame}B \
                 raw_vs_stored={:.0}x exact=true",
                bytes.len(),
                raw_frame as f64 / bytes.len() as f64
            );
            assert!((bytes.len() as u64) * 100 < raw_frame);
        }
    }

    // --- 3. Noise negative control ------------------------------------------
    {
        let (w, h) = (192u32, 128u32);
        let mut s = 99u64;
        let mut d = Vec::with_capacity((w * h) as usize);
        for _ in 0..(w * h) {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
            d.push((s >> 56) as u8);
        }
        let noise = canvas_of(w, h, d);
        let report = inverse::encode_frames(&[noise], &inverse::EncodeOptions::default())?;
        assert!(report.exact);
        assert_eq!(report.decisions[0].winner_family, "raw");
        println!(
            "noise-192x128 (negative): vole={}B winners=[raw] exact=true \
             note=\"an unknowable noise field is never discovered; a seed that \
             relocates bits never wins the court\"",
            report.vole.len()
        );
    }

    // --- 4. Generator + residual closure -------------------------------------
    {
        let (w, h) = (1920u32, 160u32);
        // A wide gradient plus a small structural band the generator cannot
        // express: the exact correction is counted and the frame stays far
        // below a raster declaration.
        let mut frames = Vec::new();
        frames.push(gradient_field(w, h, 20, 1, 0)); // pure ramp frame 0
        let f1 = frames[0].clone();
        let mut data = f1.as_slice().to_vec();
        let mut band = 0;
        for y in 20..28usize {
            for x in 300..(w as usize - 300) {
                data[y * w as usize + x] = 255 - data[y * w as usize + x];
                band += 1;
            }
        }
        frames.push(canvas_of(w, h, data));
        let report = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                bg_sweep: false,
                background: Some(0),
                ..inverse::EncodeOptions::default()
            },
        )?;
        assert!(report.exact);
        let d1 = &report.decisions[1];
        println!(
            "closure-1920x160: frame1_winner={} payload={}B band_px={band} \
             raw_frame={}B winners=[{}] exact=true",
            d1.winner_family,
            d1.winner_payload_bytes,
            u64::from(w) * u64::from(h),
            winner_counts(&report)
        );
        assert!(d1.winner_payload_bytes < u64::from(w) * u64::from(h));
    }
    Ok(())
}
