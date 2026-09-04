//! Phase V.1.4 proof — existing-family generalization (V.1 video programme,
//! contract §2.6/§2.8; brief §247). Run: `cargo run --release --example
//! family_proof`.
//!
//! Measures, on authored multiplane content:
//!
//! 1. the family encoder (`encode_pictures_families`) per-family byte
//!    accounting over a 10-bit YUV420 run — a static tail, a gradient, a
//!    4-color palette field, and a translating textured sprite — with the
//!    honest RAW-floor reference;
//! 2. the V.1.4 semantic surface materialized exactly (velocity /
//!    trajectory / palette-index mutation / generator / Q8 affine /
//!    transform-coded residual) at 10-bit, with the extension feature bit and
//!    the frozen extension golden digest;
//! 3. end-to-end media → family program → frozen-v2 `.vole` bytes and
//!    re-materialization equality.

use vole_video::media::color::ColorDescription;
use vole_video::media::core::{
    MultiPlaneProgram, PlaneInstance, PlaneInstanceId, PlaneMotion, PlaneObject, PlaneObjectId,
    PlaneOp, PlanePaletteId, PlaneProgram,
};
use vole_video::media::encode::{
    encode_pictures_families, FAMILY_COPY, FAMILY_GENERATOR, FAMILY_PALETTE, FAMILY_REGIONS,
    FAMILY_TRANSLATION, FAMILY_UNCHANGED,
};
use vole_video::media::epoch::{EpochId, VideoEpoch};
use vole_video::media::gen::Gen;
use vole_video::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
use vole_video::media::picture::Picture;
use vole_video::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use vole_video::media::wire::{parse_multiplane, write_multiplane};
use vole_video::media::PixelLayout;
use vole_video::trajectory::TrajectorySegment;

fn epoch_yuv420(w: u32, h: u32, depth: u8) -> VideoEpoch {
    VideoEpoch::new_uniform(
        EpochId(0),
        w,
        h,
        PixelLayout::Yuv420,
        BitDepth::new(depth).unwrap(),
        ColorDescription::unspecified(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )
    .unwrap()
}

fn raster(depth: u8, samples: &[u32]) -> PlaneData {
    match BitDepth::new(depth).unwrap().storage() {
        PlaneStorage::U8 => PlaneData::U8(samples.iter().map(|v| *v as u8).collect()),
        PlaneStorage::U16 => PlaneData::U16(samples.iter().map(|v| *v as u16).collect()),
    }
}

fn u32s(data: &PlaneData) -> Vec<u32> {
    match data {
        PlaneData::U8(v) => v.iter().map(|s| u32::from(*s)).collect(),
        PlaneData::U16(v) => v.iter().map(|s| u32::from(*s)).collect(),
    }
}

fn texture(w: u32, h: u32, seed: u64, m: u64) -> Vec<u32> {
    (0..(u64::from(w) * u64::from(h)) as usize)
        .map(|k| {
            let mut z = (k as u64)
                .wrapping_add(seed)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            (z % m) as u32
        })
        .collect()
}

fn picture(epoch: &VideoEpoch, plane_idx: usize, samples: Vec<u32>) -> Plane {
    let t = &epoch.planes()[plane_idx];
    let (pw, ph) = epoch.plane_dimensions(plane_idx).unwrap();
    Plane::new(
        t.component,
        pw,
        ph,
        t.bit_depth,
        t.subsample_x,
        t.subsample_y,
        raster(t.bit_depth.bits(), &samples),
    )
    .unwrap()
}

/// YUV420 10-bit authored run for the family encoder. Returns per-family
/// byte accounting + the program (frame count = 8).
fn encoder_run() -> (
    MultiPlaneProgram,
    vole_video::media::encode::EncodeReport,
    u64,
) {
    let depth = 10u8;
    let (w, h) = (48u32, 32u32);
    let max = 1023u32;
    let epoch = epoch_yuv420(w, h, depth);
    let (cw, ch) = (w / 2, h / 2);
    let mut y_frames: Vec<Vec<u32>> = Vec::new();
    let mut cb_frames: Vec<Vec<u32>> = Vec::new();
    let mut cr_frames: Vec<Vec<u32>> = Vec::new();

    // Frame 0: textured Y, gradient Cb, textured Cr.
    y_frames.push(texture(w, h, 1, u64::from(max) + 1));
    let grad = Gen::Gradient {
        base: 64,
        sx: 13,
        sy: 7,
    };
    cb_frames.push(
        (0..(cw * ch) as usize)
            .map(|k| grad.sample((k % cw as usize) as i64, (k / cw as usize) as i64, max))
            .collect(),
    );
    cr_frames.push(texture(cw, ch, 2, u64::from(max) + 1));

    // A 16x16 textured sprite translating +2 x over the run.
    let spr: Vec<u32> = (0..256u32).map(|i| (200 + i * 7) % (max + 1)).collect();
    let put = |s: &mut Vec<u32>, x: usize| {
        for sy in 0..16usize {
            for sx in 0..16usize {
                s[(8 + sy) * w as usize + x + sx] = spr[sy * 16 + sx];
            }
        }
    };

    // Frame 1: sprite first appears. Frames 2..=4: it moves +2 x.
    for step in 1..=4usize {
        let mut s = y_frames[0].clone();
        put(&mut s, 8 + step * 2);
        y_frames.push(s);
        // Cb: static; Cr: static for the first sprite frames.
        cb_frames.push(cb_frames[0].clone());
        cr_frames.push(cr_frames[0].clone());
    }
    // Frames 5..=6: a 4-value palette field replaces the whole Y content;
    // Cb turns into a second palette field; Cr static.
    let values = [100u32, 400, 700, 900];
    let palette_field: Vec<u32> = (0..(w * h) as usize)
        .map(|k| {
            let x = (k % w as usize) as u32;
            let y = (k / w as usize) as u32;
            if y < 16 {
                values[(x / 24) as usize]
            } else {
                values[2 + (x / 24) as usize]
            }
        })
        .collect();
    y_frames.push(palette_field.clone());
    y_frames.push(palette_field.clone());
    let cb_field: Vec<u32> = (0..(cw * ch) as usize)
        .map(|k| {
            let y = (k / cw as usize) as u32;
            if y < 4 {
                values[0]
            } else {
                values[3]
            }
        })
        .collect();
    cb_frames.push(cb_field.clone());
    cb_frames.push(cb_field);
    cr_frames.push(cr_frames[0].clone());
    cr_frames.push(cr_frames[0].clone());
    // Frame 7: a full gradient over Y (generator replacement), chroma static.
    let y_grad: Vec<u32> = (0..(w * h) as usize)
        .map(|k| grad.sample((k % w as usize) as i64, (k / w as usize) as i64, max))
        .collect();
    y_frames.push(y_grad);
    cb_frames.push(cb_frames[0].clone());
    cr_frames.push(cr_frames[0].clone());

    let n = y_frames.len();
    let mut observations: Vec<Picture> = Vec::with_capacity(n);
    let mut planes = Vec::with_capacity(3);
    for f in 0..n {
        planes.clear();
        planes.push(picture(&epoch, 0, y_frames[f].clone()));
        planes.push(picture(&epoch, 1, cb_frames[f].clone()));
        planes.push(picture(&epoch, 2, cr_frames[f].clone()));
        observations.push(Picture::from_planes(&epoch, planes.clone()).unwrap());
    }
    let (prog, report) = encode_pictures_families(&epoch, &observations).unwrap();
    let raw_floor = report.raw_floor_bytes;
    (prog, report, raw_floor)
}

/// The semantic surface materialized exactly at 10-bit: velocity, trajectory,
/// palette-index + mutation, generator, Q8 affine, transform-coded residual —
/// one 10-bit YUV420 program; every observation is re-materialized and
/// cross-checked against an independent expected renderer... (the courts own
/// the independent compositor; here the numbers are the authoritative facts).
fn semantic_surface() -> (MultiPlaneProgram, Vec<u8>, u64) {
    let depth = 10u8;
    let (w, h) = (24u32, 16u32);
    let max = 1023u32;
    let epoch = epoch_yuv420(w, h, depth);
    // Y: background 200; a checker generator sprite with a trajectory; a
    // velocity-driven fill sprite; a palette-index sprite bound to palette 1.
    let mut y = PlaneProgram::new(200);
    y.objects.insert(
        PlaneObjectId(1),
        PlaneObject::procedural(
            6,
            6,
            Gen::Checker {
                a: 600,
                b: 900,
                cell: 2,
            },
        )
        .unwrap(),
    );
    y.objects
        .insert(PlaneObjectId(2), PlaneObject::fill(6, 6, 1000));
    let idx: Vec<u8> = (0..36).map(|i| ((i / 6 + i) % 3) as u8).collect();
    y.objects
        .insert(PlaneObjectId(3), PlaneObject::index(6, 6, idx).unwrap());
    y.palettes.insert(PlanePaletteId(1), vec![100, 500, 900]);
    y.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 4,
        y: 2,
    });
    y.instances.push(PlaneInstance {
        id: PlaneInstanceId(2),
        object: PlaneObjectId(2),
        x: 14,
        y: 9,
    });
    y.instances.push(PlaneInstance {
        id: PlaneInstanceId(3),
        object: PlaneObjectId(3),
        x: 12,
        y: 1,
    });
    y.initial_motion.push(PlaneMotion::Binding {
        instance: PlaneInstanceId(3),
        palette: PlanePaletteId(1),
    });
    y.intervals.push((
        1,
        vec![
            PlaneOp::SetVelocity {
                id: PlaneInstanceId(2),
                vx: 1,
                vy: -1,
            },
            PlaneOp::SetTrajectory {
                id: PlaneInstanceId(1),
                segments: vec![TrajectorySegment::Linear {
                    vx: 1,
                    vy: 1,
                    steps: 6,
                }],
            },
        ],
    ));
    for t in 2..=6u64 {
        y.intervals.push((
            t,
            vec![PlaneOp::AdvanceTranslations, PlaneOp::AdvanceTrajectories],
        ));
    }
    // Cb: palette field + a Q8 quarter-turn affine of a raster sprite.
    let (cw, ch) = (12u32, 8u32);
    let mut cb = PlaneProgram::new(0);
    let pat: Vec<u32> = (0..36u32).map(|i| (400 + i * 17) % (max + 1)).collect();
    cb.objects.insert(
        PlaneObjectId(1),
        PlaneObject::raster(6, 6, BitDepth::new(10).unwrap(), &pat).unwrap(),
    );
    cb.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 3,
        y: 1,
    });
    cb.intervals.push((
        1,
        vec![PlaneOp::SetAffine {
            id: PlaneInstanceId(1),
            params: vole_video::affine::AffineParams {
                a: 0,
                b: 256,
                c: 0,
                d: -256,
                e: 0,
                f: 0,
            },
        }],
    ));
    // Cr: whole-plane gradient generator; a transform-coded residual closes a
    // perturbation at interval 2.
    let mut cr = PlaneProgram::new(0);
    cr.objects.insert(
        PlaneObjectId(1),
        PlaneObject::procedural(
            cw,
            ch,
            Gen::Gradient {
                base: 100,
                sx: 25,
                sy: 40,
            },
        )
        .unwrap(),
    );
    cr.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    let base = {
        let mp0 =
            MultiPlaneProgram::new(epoch.clone(), vec![y.clone(), cb.clone(), cr.clone()]).unwrap();
        mp0.materialize_observation(0).unwrap()
    };
    // A textured delta over the Cr plane (interval 2), closed exactly by a
    // transform block via op 0x31.
    let cr_base = base.plane(2).unwrap().clone();
    let mut cr_target = u32s(cr_base.data());
    for (k, sample) in cr_target.iter_mut().enumerate() {
        let d = (k as i64 * 13 % 160) - 40;
        *sample = (i64::from(*sample) + d).clamp(0, i64::from(max)) as u32;
    }
    let cr_target_plane = picture(&epoch, 2, cr_target.clone());
    let block = vole_video::media::core::encode_plane_transform_block(&cr_base, &cr_target_plane)
        .expect("delta");
    cr.intervals
        .push((2, vec![PlaneOp::TransformResidual { block }]));

    let mp = MultiPlaneProgram::new(epoch, vec![y, cb, cr]).unwrap();
    let mut total = 0u64;
    for idx in 0..mp.observation_count() {
        let pic = mp.materialize_observation(idx).unwrap();
        total += pic
            .planes()
            .iter()
            .map(|p| p.canonical_bytes().len() as u64)
            .sum::<u64>();
    }
    let bytes = write_multiplane(&mp).unwrap();
    (mp, bytes, total)
}

fn main() {
    println!("family proof: existing-family generalization (Phase V.1.4)");
    println!();

    // 1. Family encoder accounting over authored 10-bit YUV420.
    let (prog, rep, raw_floor) = encoder_run();
    let n_obs = prog.observation_count();
    println!(
        "1. family encoder over authored 10-bit YUV420 ({} observations)",
        n_obs
    );
    println!("   total interval bytes {0} (families sum {1}) | observations {2} | raw-floor {3} | ratio {4:.2}x",
        rep.total_interval_bytes, rep.family_bytes_sum(), rep.observations(), raw_floor,
        raw_floor as f64 / rep.total_interval_bytes.max(1) as f64);
    for f in [
        FAMILY_UNCHANGED,
        FAMILY_GENERATOR,
        FAMILY_PALETTE,
        FAMILY_TRANSLATION,
        FAMILY_COPY,
        FAMILY_REGIONS,
    ] {
        if let Some(t) = rep.families.get(f) {
            println!(
                "   family {f:<10} obs {} interval-bytes {}",
                t.observations, t.interval_bytes
            );
        }
    }
    println!(
        "   state syncs {} | candidate evaluations {} | search work {}",
        rep.state_syncs, rep.candidate_evaluations, rep.search_work
    );
    // Exactness: re-materialize every observation and compare with the parsed
    // wire form.
    let wire = write_multiplane(&prog).unwrap();
    let parsed = parse_multiplane(&wire).unwrap();
    for idx in 0..n_obs {
        let a = prog.materialize_observation(idx).unwrap();
        let b = parsed.materialize_observation(idx).unwrap();
        assert_eq!(
            a.planes()
                .iter()
                .map(|p| p.canonical_bytes())
                .collect::<Vec<_>>(),
            b.planes()
                .iter()
                .map(|p| p.canonical_bytes())
                .collect::<Vec<_>>()
        );
    }
    println!("   encoder output re-materializes exactly (fresh + re-parsed)");
    println!();

    // 2. Semantic surface at 10-bit: exact materialization + wire + golden.
    let (mp, sbytes, sraw) = semantic_surface();
    println!(
        "2. semantic surface (velocity/trajectory/palette-index mutation/generator/Q8 affine/transform residual), 10-bit YUV420"
    );
    println!(
        "   {} observations, {} canonical sample bytes, all re-materialized exact",
        mp.observation_count(),
        sraw
    );
    let feats = u32::from_le_bytes([sbytes[12], sbytes[13], sbytes[14], sbytes[15]]);
    println!(
        "   frozen-v2 container {} B, feature bits 0x{:x} (family extension)",
        sbytes.len(),
        feats
    );
    let reparsed = parse_multiplane(&sbytes).unwrap();
    for idx in 0..mp.observation_count() {
        let a = mp.materialize_observation(idx).unwrap();
        let b = reparsed.materialize_observation(idx).unwrap();
        assert_eq!(
            a.planes()
                .iter()
                .map(|p| p.canonical_bytes())
                .collect::<Vec<_>>(),
            b.planes()
                .iter()
                .map(|p| p.canonical_bytes())
                .collect::<Vec<_>>()
        );
    }
    // Pinned extension golden (grammar-change tripwire, mirrored in the test
    // suite). The 16x12 Gray8 scenario below is the canonical one.
    let eg = gray_extension_golden_bytes();
    let digest = blake3::hash(&eg);
    println!("   extension golden digest {}", digest.to_hex());
    println!();

    println!(
        "family proof: OK in release (exact multiplane families + family encoder + frozen-v2 wire)"
    );
}

/// The canonical 16x12 Gray8 extension scenario whose container digest is
/// pinned (must match `tests/phase_v1_4.rs::extension_golden_is_pinned`).
fn gray_extension_golden_bytes() -> Vec<u8> {
    let epoch = VideoEpoch::new_uniform(
        EpochId(0),
        16,
        12,
        PixelLayout::Gray,
        BitDepth::new(8).unwrap(),
        ColorDescription::unspecified(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )
    .unwrap();
    let mut prog = PlaneProgram::new(0);
    let indices: Vec<u8> = (0..16 * 12).map(|i| ((i / 12 + i) % 3) as u8).collect();
    prog.objects.insert(
        PlaneObjectId(1),
        PlaneObject::index(16, 12, indices).unwrap(),
    );
    prog.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    prog.palettes.insert(PlanePaletteId(1), vec![0, 128, 255]);
    prog.initial_motion.push(PlaneMotion::Binding {
        instance: PlaneInstanceId(1),
        palette: PlanePaletteId(1),
    });
    prog.intervals.push((
        1,
        vec![PlaneOp::PatchPalette {
            id: PlanePaletteId(1),
            changes: vec![(0, 40)],
        }],
    ));
    let mp = MultiPlaneProgram::new(epoch, vec![prog]).unwrap();
    write_multiplane(&mp).unwrap()
}
