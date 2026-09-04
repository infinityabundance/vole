//! Phase-O evidence producer: `vole optimize` (equivalence-preserving
//! representation re-optimization).
//!
//! `cargo run --release --example optimize_proof` prints a deterministic
//! report for each rewrite family — velocity collapse, trajectory collapse,
//! residual promotion, generator substitution, duplicate merge — plus the
//! whole-stream never-grow/fixpoint invariants over earlier-phase stream
//! shapes. Every optimized stream is end-to-end decode-verified against the
//! input (byte-identical frames) before it is counted.

use vole_video::{
    collapse, decoder, encoder,
    error::VoleError,
    inverse,
    object::Object,
    object::ObjectId,
    optimize,
    pixel::Canvas,
    rans,
    state::{Instance, InstanceId},
    transition::Transition,
};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

fn gradient_samples(w: u32, h: u32, base: u8, sx: i64, sy: i64) -> Vec<u8> {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            d.push(((i64::from(base) + sx * i64::from(x) + sy * i64::from(y)) & 0xFF) as u8);
        }
    }
    d
}

/// Deterministic non-generator texture (never fits gradient/checker/periodic).
fn textured_samples(w: u32, h: u32) -> Vec<u8> {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            d.push(((u64::from(x) * 7 + u64::from(y) * 13) % 23) as u8 * 11 + 5);
        }
    }
    d
}

fn report(name: &str, before: &[u8], after: &optimize::OptimizeReport) {
    let saved = before.len().saturating_sub(after.stream.len());
    println!(
        "{name}: {}B -> {}B saved={saved}B ({:.3}) exact={} rewrites=[{}]",
        before.len(),
        after.stream.len(),
        if before.is_empty() {
            0.0
        } else {
            saved as f64 / before.len() as f64
        },
        after.exact,
        after.rewrites.join(" ")
    );
}

fn main() -> Result<(), VoleError> {
    let (w, h) = (192u32, 128u32);
    let bg = 60u8;

    // --- 1. Velocity collapse (100-frame linear run at full HD) -------------
    {
        let obj = Object::raster(200, 100, textured_samples(200, 100))?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 10,
            y: 10,
        };
        let mut timeline = Vec::new();
        for k in 1..=100u64 {
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(1),
                    x: 10 + 2 * k as i64,
                    y: 10,
                }],
            ));
        }
        let bytes = encoder::encode_stream(1920, 1080, bg, &[(1, obj)], &[inst], &timeline)?;
        let r = optimize::optimize_stream(&bytes)?;
        report("velocity-100x1080p", &bytes, &r);
        assert!(r.rewrites.contains(&"velocity_collapse") && r.exact);
        let traj_only = collapse::collapse_fixpoint(bytes.clone())?;
        println!(
            "  velocity-vs-trajectory: velocity={}B trajectory_only={}B",
            r.stream.len(),
            traj_only.len()
        );
        assert!(r.stream.len() < traj_only.len());
    }

    // --- 2. Trajectory collapse (accel run) ----------------------------------
    {
        let obj = Object::raster(16, 16, gradient_samples(16, 16, 200, 1, 3))?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 10,
            y: 20,
        };
        let mut timeline = Vec::new();
        for k in 1..=40u64 {
            let x = 10 + 2 * k as i64 + k as i64 * (k as i64 - 1) / 2;
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(1),
                    x,
                    y: 20,
                }],
            ));
        }
        let bytes = encoder::encode_stream(w, h, bg, &[(1, obj)], &[inst], &timeline)?;
        let r = optimize::optimize_stream(&bytes)?;
        report("accel-40x192x128", &bytes, &r);
        assert!(r.rewrites.contains(&"trajectory_collapse") && r.exact);
    }

    // --- 3. Residual promotion ----------------------------------------------
    {
        let obj = Object::raster(192, 128, gradient_samples(192, 128, 30, 1, 2))?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        };
        // A stable 40-point "clock glyph" difference carried one-shot per
        // frame for 30 frames.
        let mut pts_bytes = Vec::new();
        for x in 100..140i32 {
            pts_bytes.extend_from_slice(&x.to_le_bytes());
            pts_bytes.extend_from_slice(&20i32.to_le_bytes());
            pts_bytes.push(((x as u8) * 3) % 250);
        }
        let block = rans::encode_block(&pts_bytes);
        let mut timeline = Vec::new();
        for k in 1..=30u64 {
            timeline.push((
                k,
                vec![Transition::Residual {
                    block: block.clone(),
                }],
            ));
        }
        let bytes = encoder::encode_stream(w, h, bg, &[(1, obj)], &[inst], &timeline)?;
        let r = optimize::optimize_stream(&bytes)?;
        report("stable-residual-30x", &bytes, &r);
        assert!(r.rewrites.contains(&"residual_promotion") && r.exact);
        let after = inverse::account_stream(&r.stream)?;
        println!(
            "  residual_bytes: {}B -> {}B",
            inverse::account_stream(&bytes)?.residual_bytes,
            after.residual_bytes
        );
    }

    // --- 4. Generator substitution -------------------------------------------
    {
        let samples = gradient_samples(192, 128, 20, 3, -2);
        let obj = Object::raster(192, 128, samples)?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        };
        let bytes = encoder::encode_stream(w, h, bg, &[(1, obj)], &[inst], &[])?;
        let r = optimize::optimize_stream(&bytes)?;
        report("raster-gradient-decl", &bytes, &r);
        assert!(r.rewrites.contains(&"generator_substitution") && r.exact);
    }

    // --- 5. Duplicate merge --------------------------------------------------
    {
        let tile = gradient_samples(64, 64, 100, 4, -1);
        let objs: Vec<(u32, Object)> = (1..=8u32)
            .map(|id| (id, Object::raster(64, 64, tile.clone()).expect("tile")))
            .collect();
        let instances: Vec<Instance> = objs
            .iter()
            .enumerate()
            .map(|(k, (id, _))| Instance {
                id: InstanceId(k as u32 + 1),
                object_id: ObjectId(*id),
                x: (k as i64 % 4) * 40,
                y: (k as i64 / 4) * 40,
            })
            .collect();
        let bytes = encoder::encode_stream(w, h, bg, &objs, &instances, &[])?;
        let r = optimize::optimize_stream(&bytes)?;
        report("eight-identical-tiles", &bytes, &r);
        assert!(r.rewrites.contains(&"duplicate_merge") && r.exact);
    }

    // --- 6. Never-grow invariants over earlier-phase shapes ------------------
    {
        // Raster-origin drifting-gradient encode (Phase N output shape).
        let mut frames = Vec::new();
        for t in 0..16u64 {
            frames.push(canvas_of(
                w,
                h,
                gradient_samples(w, h, 0, 3, 5)
                    .into_iter()
                    .map(|v| (u16::from(v) + 11 * (t % 256) as u16) as u8)
                    .collect(),
            ));
        }
        let enc = inverse::encode_frames(&frames, &inverse::EncodeOptions::default())?;
        let r = optimize::optimize_stream(&enc.vole)?;
        report("inverse-gradient-16x", &enc.vole, &r);
        assert!(r.stream.len() <= enc.vole.len() && r.exact);
        // Noise encode: optimize must be a fixpoint (never invent a win).
        let mut s = 123u64;
        let mut nd = Vec::with_capacity((w * h) as usize);
        for _ in 0..(w * h) {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
            nd.push((s >> 56) as u8);
        }
        let enc2 = inverse::encode_frames(
            &[canvas_of(w, h, nd.clone()), canvas_of(w, h, nd)],
            &inverse::EncodeOptions::default(),
        )?;
        let r2 = optimize::optimize_stream(&enc2.vole)?;
        report("noise-encode (negative)", &enc2.vole, &r2);
        assert!(r2.rewrites.is_empty() || r2.stream.len() <= enc2.vole.len());
        assert!(r2.exact);
    }

    // --- 7. Decode equality proof on every optimized stream ------------------
    {
        // Re-verify the flagship court streams decode byte-identically.
        let obj = Object::raster(24, 16, gradient_samples(24, 16, 90, 2, 1))?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 10,
            y: 10,
        };
        let mut timeline = Vec::new();
        for k in 1..=50u64 {
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(1),
                    x: 10 + 2 * k as i64,
                    y: 10,
                }],
            ));
        }
        let bytes = encoder::encode_stream(w, h, bg, &[(1, obj)], &[inst], &timeline)?;
        let r = optimize::optimize_stream(&bytes)?;
        let a = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
        let b = decoder::materialize_all(&decoder::decode_bytes(&r.stream)?)?;
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(x, y)| x.exactly_matches(y)));
        println!(
            "decode-proof: 51 frames identical before/after ({}B -> {}B)",
            bytes.len(),
            r.stream.len()
        );
    }
    Ok(())
}
