//! Phase O courts: equivalence-preserving representation re-optimization
//! (`vole optimize`, §44).
//!
//! `optimize::optimize_stream` searches a bounded rewrite set over a decoded
//! stream — velocity collapse, trajectory collapse, residual promotion,
//! generator substitution, duplicate merge — and accepts a rewrite only when
//! the rebuilt stream is strictly smaller **and** decodes to byte-identical
//! frames (`M(D0) == M(D1)` plus `J(D1) < J(D0)`). Courts cover each family,
//! the fixpoint property, the never-grow invariant across earlier-phase
//! stream shapes (including palette streams, which are preserved verbatim),
//! and typed rejection of hostile input.

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

/// Assert the optimize invariant: never grows, frames byte-identical.
fn assert_optimize_invariant(bytes: &[u8]) -> Result<optimize::OptimizeReport, VoleError> {
    let report = optimize::optimize_stream(bytes)?;
    assert!(
        report.stream.len() <= bytes.len(),
        "optimize must never grow ({} -> {})",
        bytes.len(),
        report.stream.len()
    );
    assert!(report.exact, "optimized stream must decode identically");
    Ok(report)
}

#[test]
fn velocity_collapse_serves_linear_setposition_runs() -> Result<(), VoleError> {
    // Authored per-frame SetPosition with a constant delta: optimize must
    // pick the velocity rewrite (cheaper than the trajectory descriptor) and
    // stay decode-identical.
    let (w, h) = (192u32, 128u32);
    let bg = 60u8;
    let obj = Object::raster(24, 16, gradient_samples(24, 16, 90, 2, 1))?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 10,
        y: 10,
    };
    let mut timeline = Vec::new();
    for k in 1..=12u64 {
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
    let report = assert_optimize_invariant(&bytes)?;
    assert!(
        report.rewrites.contains(&"velocity_collapse"),
        "linear runs must collapse to velocity: {:?}",
        report.rewrites
    );
    assert!(
        report.stream.len() < bytes.len(),
        "must strictly shrink: {} -> {}",
        bytes.len(),
        report.stream.len()
    );
    // Velocity beats the trajectory descriptor for pure linear runs.
    let traj_only = collapse::collapse_fixpoint(bytes.clone())?;
    assert!(
        report.stream.len() < traj_only.len(),
        "velocity ({}) must beat trajectory-only ({})",
        report.stream.len(),
        traj_only.len()
    );
    // Frames byte-identical.
    let a = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
    let b = decoder::materialize_all(&decoder::decode_bytes(&report.stream)?)?;
    assert_eq!(a.len(), b.len());
    assert!(a.iter().zip(&b).all(|(x, y)| x.exactly_matches(y)));
    // Fixpoint: a second pass applies nothing more.
    let again = assert_optimize_invariant(&report.stream)?;
    assert!(again.rewrites.is_empty());
    assert_eq!(again.stream.len(), report.stream.len());
    Ok(())
}

#[test]
fn accelerating_runs_collapse_via_trajectory() -> Result<(), VoleError> {
    // Position sequence p0 + v0*k + a*k*(k-1)/2 with v0=(2,0), a=(1,0):
    // 10, 13, 17, 22, ... — not linear, so the accel trajectory fit applies.
    let (w, h) = (192u32, 128u32);
    let bg = 60u8;
    let obj = Object::raster(8, 8, gradient_samples(8, 8, 5, 3, 3))?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 10,
        y: 20,
    };
    let mut timeline = Vec::new();
    for k in 1..=10u64 {
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
    let report = assert_optimize_invariant(&bytes)?;
    assert!(
        report.rewrites.contains(&"trajectory_collapse"),
        "accel runs must collapse to a trajectory: {:?}",
        report.rewrites
    );
    assert!(report.stream.len() < bytes.len());
    Ok(())
}

#[test]
fn repeated_residual_is_promoted_to_the_unchanged_lane() -> Result<(), VoleError> {
    // A static scene plus the *same* one-shot residual on 6 consecutive
    // frames: optimize promotes it to one persistent sparse overlay and the
    // later intervals ride the unchanged lane (the recorded Phase-G/K gap).
    let (w, h) = (192u32, 128u32);
    let bg = 70u8;
    let obj = Object::raster(64, 48, gradient_samples(64, 48, 30, 1, 2))?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 40,
        y: 30,
    };
    // One changed pixel carried as a canonical point residual block.
    let mut point_bytes = Vec::with_capacity(9);
    point_bytes.extend_from_slice(&10i32.to_le_bytes());
    point_bytes.extend_from_slice(&10i32.to_le_bytes());
    point_bytes.push(200u8);
    let block = rans::encode_block(&point_bytes);
    let mut timeline = Vec::new();
    for k in 1..=6u64 {
        timeline.push((
            k,
            vec![Transition::Residual {
                block: block.clone(),
            }],
        ));
    }
    let bytes = encoder::encode_stream(w, h, bg, &[(1, obj)], &[inst], &timeline)?;
    let report = assert_optimize_invariant(&bytes)?;
    assert!(
        report.rewrites.contains(&"residual_promotion"),
        "repeated identical residuals must be promoted: {:?}",
        report.rewrites
    );
    assert!(
        report.stream.len() < bytes.len(),
        "{} -> {}",
        bytes.len(),
        report.stream.len()
    );
    // After promotion the repeated frames are the unchanged lane: the
    // per-frame cost falls to the envelope.
    let before = inverse::account_stream(&bytes)?;
    let after = inverse::account_stream(&report.stream)?;
    assert!(
        after.residual_bytes < before.residual_bytes,
        "residual bytes must fall"
    );
    Ok(())
}

#[test]
fn raster_objects_are_substituted_by_generators() -> Result<(), VoleError> {
    // A declared *raster* object whose samples are exactly a gradient: the
    // optimizer re-declares it as a generator (samples never stored) with
    // byte-identical decoding.
    let (w, h) = (192u32, 128u32);
    let bg = 9u8;
    let samples = gradient_samples(192, 128, 20, 3, -2);
    let obj = Object::raster(w, h, samples.clone())?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let bytes = encoder::encode_stream(w, h, bg, &[(1, obj)], &[inst], &[])?;
    let report = assert_optimize_invariant(&bytes)?;
    assert!(
        report.rewrites.contains(&"generator_substitution"),
        "{:?}",
        report.rewrites
    );
    assert!(report.stream.len() < bytes.len());
    // The object is now a generator declaration: generator bytes == object
    // bytes, no stored raster samples.
    let cost = inverse::account_stream(&report.stream)?;
    assert_eq!(cost.generator_object_bytes, cost.object_bytes);
    assert_eq!(cost.raster_object_sample_bytes, 0);
    // A *checker* raster substitutes too.
    let mut check = Vec::with_capacity((64 * 48) as usize);
    for y in 0..48i64 {
        for x in 0..64i64 {
            check.push(if (x / 8 + y / 8) % 2 == 0 { 30 } else { 210 });
        }
    }
    let obj2 = Object::raster(64, 48, check)?;
    let inst2 = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let bytes2 = encoder::encode_stream(w, h, bg, &[(1, obj2)], &[inst2], &[])?;
    let report2 = assert_optimize_invariant(&bytes2)?;
    assert!(report2.rewrites.contains(&"generator_substitution"));
    // Non-generator raster (noise) is a fixpoint: never substituted.
    let mut s = 42u64;
    let mut noise = Vec::with_capacity((64 * 48) as usize);
    for _ in 0..(64 * 48) {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        noise.push((s >> 56) as u8);
    }
    let obj3 = Object::raster(64, 48, noise)?;
    let inst3 = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let bytes3 = encoder::encode_stream(w, h, bg, &[(1, obj3)], &[inst3], &[])?;
    let report3 = assert_optimize_invariant(&bytes3)?;
    assert!(
        !report3.rewrites.contains(&"generator_substitution"),
        "noise is never substituted: {:?}",
        report3.rewrites
    );
    Ok(())
}

#[test]
fn duplicate_objects_share_one_declaration() -> Result<(), VoleError> {
    // Two declarations of byte-identical content (authored twice) merge into
    // one object; references are remapped and decoding is identical.
    let (w, h) = (192u32, 128u32);
    let bg = 3u8;
    let tile = gradient_samples(32, 32, 100, 4, -1);
    let a = Object::raster(32, 32, tile.clone())?;
    let b = Object::raster(32, 32, tile)?;
    let instances = vec![
        Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 10,
            y: 10,
        },
        Instance {
            id: InstanceId(2),
            object_id: ObjectId(2),
            x: 90,
            y: 50,
        },
    ];
    let bytes = encoder::encode_stream(w, h, bg, &[(1, a), (2, b)], &instances, &[])?;
    let report = assert_optimize_invariant(&bytes)?;
    assert!(
        report.rewrites.contains(&"duplicate_merge"),
        "{:?}",
        report.rewrites
    );
    assert!(report.stream.len() < bytes.len());
    // One object declaration remains for the shared content.
    let cost = inverse::account_stream(&report.stream)?;
    assert!(
        cost.object_bytes < inverse::account_stream(&bytes)?.object_bytes,
        "object declaration bytes must fall"
    );
    Ok(())
}

#[test]
fn palette_streams_are_preserved_verbatim() -> Result<(), VoleError> {
    // The Phase-J palette path: rebuilds would need pre-checkpoint palette
    // records and bindings, so optimize preserves these streams exactly
    // (documented limitation — never a silent change).
    let court = vole_video::demo::PaletteCourt {
        width: 96,
        height: 64,
        background: 10,
        box_w: 32,
        box_h: 32,
        box_x: 20,
        box_y: 12,
        indices: {
            let mut d = Vec::with_capacity(32 * 32);
            for y in 0..32 {
                for x in 0..32 {
                    d.push(((x + 3 * y) % 5) as u8);
                }
            }
            d
        },
        mode: vole_video::demo::PaletteMode::AccentCycle,
        base_entries: vec![10, 40, 90, 150, 220],
        accent_index: 1,
        cycle: vec![200, 60],
        intervals: 8,
        palette_id: 1,
        object_id: 1,
        instance_id: 1,
    };
    let bytes = court.vole()?;
    let report = assert_optimize_invariant(&bytes)?;
    assert!(
        report.rewrites.is_empty() && report.stream.len() == bytes.len(),
        "palette streams are preserved verbatim"
    );
    Ok(())
}

#[test]
fn optimize_never_grows_on_prior_phase_stream_shapes() -> Result<(), VoleError> {
    // A spread of earlier-phase stream shapes must never grow and must always
    // decode identically under optimize.
    let (w, h) = (96u32, 64u32);
    // (a) Raster-origin inverse encode: drifting gradient frames (generators).
    let mut frames: Vec<Canvas> = Vec::new();
    for t in 0..8u64 {
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
    assert!(enc.exact);
    let r1 = assert_optimize_invariant(&enc.vole)?;
    assert!(r1.stream.len() <= enc.vole.len());
    // (b) Noise negative encode (RAW resets).
    let mut s = 5u64;
    let mut nd = Vec::with_capacity((w * h) as usize);
    for _ in 0..(w * h) {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        nd.push((s >> 56) as u8);
    }
    let frames2 = vec![canvas_of(w, h, nd.clone()), canvas_of(w, h, nd)];
    let enc2 = inverse::encode_frames(&frames2, &inverse::EncodeOptions::default())?;
    let r2 = assert_optimize_invariant(&enc2.vole)?;
    // (c) The Phase-L affine rotating-tile court stream.
    let court = vole_video::demo::AffineCourt {
        width: 96,
        height: 96,
        background: 90,
        tile_w: 32,
        tile_h: 32,
        content: {
            let mut d = Vec::with_capacity(32 * 32);
            for y in 0..32i64 {
                for x in 0..32i64 {
                    d.push(((x / 3 + y / 5) % 9) as u8 * 23 + 10);
                }
            }
            d
        },
        plain_x: 24,
        plain_y: 24,
        object_id: 1,
        instance_id: 1,
        params: (1..=4u64)
            .map(|k| vole_video::demo::quarter_turn_params(k as i64, 16, 16, 48, 48))
            .collect(),
        intervals: 4,
    };
    let r3 = assert_optimize_invariant(&court.vole()?)?;
    let _ = r2;
    let _ = r1;
    let _ = r3;
    Ok(())
}

#[test]
fn hostile_input_is_typed() {
    // Garbage and truncated streams must fail typed, never panic.
    assert!(optimize::optimize_stream(&[0u8; 4]).is_err());
    let good = encoder::encode_stream(
        8,
        8,
        0,
        &[(1, Object::fill(8, 8, 3).expect("fill"))],
        &[Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        }],
        &[(1u64, vec![Transition::AdvanceTranslations])],
    )
    .expect("encode");
    // A valid stream is accepted and never grows on this no-op content.
    let report = optimize::optimize_stream(&good).expect("optimize ok");
    assert!(report.stream.len() <= good.len());
    assert!(report.exact);
    // Truncating the stream is a typed error path.
    assert!(optimize::optimize_stream(&good[..good.len() - 4]).is_err());
}
