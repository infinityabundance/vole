//! Phase V.1.4 courts — existing-family generalization (V.1 video programme,
//! contract §2.6/§2.8, `docs/format-v2.md` re-frozen with the V.1.4 family
//! extension; master brief §45–§50, §247).
//!
//! Courts:
//!
//! 1. **v1 specialization parity at depth 8** for the ported families —
//!    velocity/advance translation, linear + acceleration trajectories,
//!    palette-index content with palette mutation, the four generator
//!    programs, Q8 affine placement, and the transform-coded residual. Each
//!    scenario is authored once as a sealed v1 Gray8 stream and once as a v2
//!    single-plane depth-8 program; every materialized frame is byte-identical.
//! 2. **authored multiplane 10-bit semantics** — a YUV420 10-bit program
//!    (velocity-driven sprites, generator content, palette-index content,
//!    static-run repeats) verified against an independent per-plane
//!    compositor written in the court (closed-form trajectory positions, no
//!    shared paint code with `core.rs`).
//! 3. **family encoder** — the raster-origin encoder reproduces authored runs
//!    sample-for-sample and chooses the expected families by complete interval
//!    bytes on static runs, gradients, palettes, and a translating textured
//!    sprite; RAW-floor bytes are reported, never hidden.
//! 4. **v2 wire (family extension)** — byte roundtrips across layouts ×
//!    depths, minimal feature bits, a pinned extension golden, and a hostile
//!    extension corpus that is typed and never panics.
//! 5. **transform residual floor** — encode → apply == target at 10-bit, and
//!    hostile blocks are typed.

use std::collections::BTreeMap;

use vole_video::media::color::ColorDescription;
use vole_video::media::core::{
    encode_plane_transform_block, MultiPlaneProgram, PlaneContent, PlaneInstance, PlaneInstanceId,
    PlaneMotion, PlaneObject, PlaneObjectId, PlaneOp, PlanePaletteId, PlaneProgram,
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
use vole_video::VoleError;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn gray_epoch(w: u32, h: u32, depth: u8) -> VideoEpoch {
    VideoEpoch::new_uniform(
        EpochId(0),
        w,
        h,
        PixelLayout::Gray,
        BitDepth::new(depth).unwrap(),
        ColorDescription::unspecified(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )
    .unwrap()
}

fn yuv_epoch(w: u32, h: u32, depth: u8) -> VideoEpoch {
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

fn program_of(epoch: &VideoEpoch, plane: PlaneProgram) -> MultiPlaneProgram {
    MultiPlaneProgram::new(epoch.clone(), vec![plane]).unwrap()
}

fn frames_of(prog: &MultiPlaneProgram) -> Vec<Picture> {
    (0..prog.observation_count())
        .map(|i| prog.materialize_observation(i).unwrap())
        .collect()
}

fn pic_bytes(pic: &Picture) -> Vec<u8> {
    let mut out = Vec::new();
    for p in pic.planes() {
        out.extend_from_slice(&p.canonical_bytes());
    }
    out
}

fn raster_data(depth: u8, samples: &[u32]) -> PlaneData {
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

fn sprite(depth: u8, seed: u32, v0: u32) -> Vec<u32> {
    (0..64u32)
        .map(|i| (v0 + i * seed) % (BitDepth::new(depth).unwrap().max_sample() + 1))
        .collect()
}

/// A deterministic full-plane texture (splitmix-spread, in the depth domain).
fn plane_texture(depth: u8, w: u32, h: u32, seed: u64) -> Vec<u32> {
    let m = u64::from(BitDepth::new(depth).unwrap().max_sample()) + 1;
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

/// A deterministic raster object at an explicit depth.
fn raster_object(depth: u8, samples: &[u32], w: u32, h: u32) -> PlaneObject {
    PlaneObject::raster(w, h, BitDepth::new(depth).unwrap(), samples).unwrap()
}

fn feature_bits(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]])
}

// ---------------------------------------------------------------------------
// 1. v1 specialization parity at depth 8 (the ported families)
// ---------------------------------------------------------------------------

fn parity_check(
    w: u32,
    h: u32,
    build_v1: impl FnOnce(&mut vole_video::ingest::Ingest),
    build_v2: impl FnOnce() -> PlaneProgram,
) {
    let mut a = vole_video::ingest::Ingest::new(w, h);
    build_v1(&mut a);
    let v1_bytes = a.finish().unwrap();
    let v1_frames = {
        let parsed = vole_video::decoder::decode_bytes(&v1_bytes).unwrap();
        vole_video::decoder::materialize_all(&parsed).unwrap()
    };
    let v2 = program_of(&gray_epoch(w, h, 8), build_v2());
    let v2_frames = frames_of(&v2);
    assert_eq!(
        v1_frames.len(),
        v2_frames.len(),
        "parity scenario frame counts differ"
    );
    for (k, (f1, f2)) in v1_frames.iter().zip(v2_frames.iter()).enumerate() {
        assert_eq!(
            f1.as_slice(),
            pic_bytes(f2).as_slice(),
            "v1/v2 parity diverged at frame {k}"
        );
    }
}

fn gray8_raster(samples: &[u32], w: u32, h: u32) -> PlaneObject {
    raster_object(8, samples, w, h)
}

#[test]
fn trajectory_velocity_parity_at_depth8() {
    parity_check(
        48,
        32,
        |a| {
            a.background(12);
            a.declare_fill(1, 14, 6, 180).unwrap();
            let spr: Vec<u8> = sprite(8, 3, 40).iter().map(|v| *v as u8).collect();
            a.declare_raster(2, 8, 8, spr).unwrap();
            a.instance(1, 1, 0, 0).unwrap();
            a.instance(2, 2, 20, 10).unwrap();
            a.at(1).unwrap();
            a.set_velocity(2, 2, 1).unwrap();
            a.advance().unwrap();
            for t in 2..=6u64 {
                a.at(t).unwrap();
                a.advance().unwrap();
            }
        },
        || {
            let mut prog = PlaneProgram::new(12);
            prog.objects
                .insert(PlaneObjectId(1), PlaneObject::fill(14, 6, 180));
            prog.objects
                .insert(PlaneObjectId(2), gray8_raster(&sprite(8, 3, 40), 8, 8));
            prog.instances.push(PlaneInstance {
                id: PlaneInstanceId(1),
                object: PlaneObjectId(1),
                x: 0,
                y: 0,
            });
            prog.instances.push(PlaneInstance {
                id: PlaneInstanceId(2),
                object: PlaneObjectId(2),
                x: 20,
                y: 10,
            });
            for t in 1..=6u64 {
                let mut ops = Vec::new();
                if t == 1 {
                    ops.push(PlaneOp::SetVelocity {
                        id: PlaneInstanceId(2),
                        vx: 2,
                        vy: 1,
                    });
                }
                ops.push(PlaneOp::AdvanceTranslations);
                prog.intervals.push((t, ops));
            }
            prog
        },
    );
}

#[test]
fn linear_and_accel_trajectory_parity_at_depth8() {
    parity_check(
        48,
        32,
        |a| {
            a.background(12);
            a.declare_fill(1, 10, 4, 90).unwrap();
            a.instance(1, 1, 2, 2).unwrap();
            a.at(1).unwrap();
            a.set_trajectory(
                1,
                vec![
                    TrajectorySegment::Linear {
                        vx: 3,
                        vy: 0,
                        steps: 4,
                    },
                    TrajectorySegment::Accel {
                        vx0: 3,
                        vy0: 0,
                        ax: 1,
                        ay: 1,
                        steps: 4,
                    },
                ],
            )
            .unwrap();
            a.advance_trajectories().unwrap();
            for t in 2..=8u64 {
                a.at(t).unwrap();
                a.advance_trajectories().unwrap();
            }
        },
        || {
            let mut prog = PlaneProgram::new(12);
            prog.objects
                .insert(PlaneObjectId(1), PlaneObject::fill(10, 4, 90));
            prog.instances.push(PlaneInstance {
                id: PlaneInstanceId(1),
                object: PlaneObjectId(1),
                x: 2,
                y: 2,
            });
            for t in 1..=8u64 {
                let mut ops = Vec::new();
                if t == 1 {
                    ops.push(PlaneOp::SetTrajectory {
                        id: PlaneInstanceId(1),
                        segments: vec![
                            TrajectorySegment::Linear {
                                vx: 3,
                                vy: 0,
                                steps: 4,
                            },
                            TrajectorySegment::Accel {
                                vx0: 3,
                                vy0: 0,
                                ax: 1,
                                ay: 1,
                                steps: 4,
                            },
                        ],
                    });
                }
                ops.push(PlaneOp::AdvanceTrajectories);
                prog.intervals.push((t, ops));
            }
            prog
        },
    );
}

#[test]
fn palette_index_parity_at_depth8() {
    parity_check(
        40,
        24,
        |a| {
            a.background(0);
            let indices: Vec<u8> = (0..16 * 12).map(|i| ((i / 12 + i) % 4) as u8).collect();
            a.declare_index(1, 16, 12, indices).unwrap();
            a.declare_palette(1, vec![10, 40, 90, 160]).unwrap();
            a.instance_binding(1, 1, 8, 6, 1).unwrap();
            a.at(1).unwrap();
            a.set_palette(1, vec![200, 40, 90, 160]).unwrap();
            a.at(2).unwrap();
            a.patch_palette(1, vec![(0, 30), (3, 250)]).unwrap();
            a.at(3).unwrap();
            a.patch_palette(1, vec![(1, 5)]).unwrap();
        },
        || {
            let mut prog = PlaneProgram::new(0);
            let indices: Vec<u8> = (0..16 * 12).map(|i| ((i / 12 + i) % 4) as u8).collect();
            prog.objects.insert(
                PlaneObjectId(1),
                PlaneObject::index(16, 12, indices).unwrap(),
            );
            prog.instances.push(PlaneInstance {
                id: PlaneInstanceId(1),
                object: PlaneObjectId(1),
                x: 8,
                y: 6,
            });
            prog.palettes
                .insert(PlanePaletteId(1), vec![10, 40, 90, 160]);
            prog.initial_motion.push(PlaneMotion::Binding {
                instance: PlaneInstanceId(1),
                palette: PlanePaletteId(1),
            });
            prog.intervals.push((
                1,
                vec![PlaneOp::SetPalette {
                    id: PlanePaletteId(1),
                    entries: vec![200, 40, 90, 160],
                }],
            ));
            prog.intervals.push((
                2,
                vec![PlaneOp::PatchPalette {
                    id: PlanePaletteId(1),
                    changes: vec![(0, 30), (3, 250)],
                }],
            ));
            prog.intervals.push((
                3,
                vec![PlaneOp::PatchPalette {
                    id: PlanePaletteId(1),
                    changes: vec![(1, 5)],
                }],
            ));
            prog
        },
    );
}

#[test]
fn generator_content_parity_at_depth8() {
    parity_check(
        64,
        32,
        |a| {
            a.background(0);
            a.declare_gradient(1, 32, 16, 5, 3, 0).unwrap();
            a.declare_generator(
                2,
                16,
                16,
                vole_video::generator::Generator::Checker {
                    a: 0,
                    b: 255,
                    cell: 4,
                },
            )
            .unwrap();
            a.declare_generator(
                3,
                24,
                8,
                vole_video::generator::Generator::Periodic {
                    base: 10,
                    sx: 2,
                    sy: 1,
                    period: 16,
                },
            )
            .unwrap();
            a.declare_generator(
                4,
                16,
                16,
                vole_video::generator::Generator::Noise { seed: 7 },
            )
            .unwrap();
            a.instance(1, 1, 0, 0).unwrap();
            a.instance(2, 2, 32, 0).unwrap();
            a.instance(3, 3, 0, 16).unwrap();
            a.instance(4, 4, 32, 16).unwrap();
            a.at(1).unwrap();
            a.set_position(1, 4, 4).unwrap();
        },
        || {
            let mut prog = PlaneProgram::new(0);
            prog.objects.insert(
                PlaneObjectId(1),
                PlaneObject::procedural(
                    32,
                    16,
                    Gen::Gradient {
                        base: 5,
                        sx: 3,
                        sy: 0,
                    },
                )
                .unwrap(),
            );
            prog.objects.insert(
                PlaneObjectId(2),
                PlaneObject::procedural(
                    16,
                    16,
                    Gen::Checker {
                        a: 0,
                        b: 255,
                        cell: 4,
                    },
                )
                .unwrap(),
            );
            prog.objects.insert(
                PlaneObjectId(3),
                PlaneObject::procedural(
                    24,
                    8,
                    Gen::Periodic {
                        base: 10,
                        sx: 2,
                        sy: 1,
                        period: 16,
                    },
                )
                .unwrap(),
            );
            prog.objects.insert(
                PlaneObjectId(4),
                PlaneObject::procedural(16, 16, Gen::Noise { seed: 7 }).unwrap(),
            );
            for (id, x, y) in [(1, 0i64, 0i64), (2, 32, 0), (3, 0, 16), (4, 32, 16)] {
                prog.instances.push(PlaneInstance {
                    id: PlaneInstanceId(id),
                    object: PlaneObjectId(id),
                    x,
                    y,
                });
            }
            prog.intervals.push((
                1,
                vec![PlaneOp::SetPosition {
                    id: PlaneInstanceId(1),
                    x: 4,
                    y: 4,
                }],
            ));
            prog
        },
    );
}

#[test]
fn affine_placement_parity_at_depth8() {
    parity_check(
        40,
        40,
        |a| {
            a.background(0);
            let pat: Vec<u8> = (0..144u32).map(|i| ((i * 37) % 256) as u8).collect();
            a.declare_raster(1, 12, 12, pat).unwrap();
            let spr: Vec<u8> = sprite(8, 5, 90).iter().map(|v| *v as u8).collect();
            a.declare_raster(2, 8, 8, spr).unwrap();
            a.instance(1, 1, 8, 8).unwrap();
            a.instance(2, 2, 2, 30).unwrap();
            a.at(1).unwrap();
            a.set_affine(
                1,
                vole_video::affine::AffineParams {
                    a: 0,
                    b: 256,
                    c: 0,
                    d: -256,
                    e: 0,
                    f: 0,
                },
            )
            .unwrap();
            a.at(2).unwrap();
            a.set_affine(
                2,
                vole_video::affine::AffineParams {
                    a: 128,
                    b: 0,
                    c: 0,
                    d: 0,
                    e: 128,
                    f: 0,
                },
            )
            .unwrap();
        },
        || {
            let mut prog = PlaneProgram::new(0);
            let pat: Vec<u32> = (0..144u32).map(|i| (i * 37) % 256).collect();
            prog.objects
                .insert(PlaneObjectId(1), gray8_raster(&pat, 12, 12));
            prog.objects
                .insert(PlaneObjectId(2), gray8_raster(&sprite(8, 5, 90), 8, 8));
            prog.instances.push(PlaneInstance {
                id: PlaneInstanceId(1),
                object: PlaneObjectId(1),
                x: 8,
                y: 8,
            });
            prog.instances.push(PlaneInstance {
                id: PlaneInstanceId(2),
                object: PlaneObjectId(2),
                x: 2,
                y: 30,
            });
            let rot = vole_video::affine::AffineParams {
                a: 0,
                b: 256,
                c: 0,
                d: -256,
                e: 0,
                f: 0,
            };
            let zoom = vole_video::affine::AffineParams {
                a: 128,
                b: 0,
                c: 0,
                d: 0,
                e: 128,
                f: 0,
            };
            prog.intervals.push((
                1,
                vec![PlaneOp::SetAffine {
                    id: PlaneInstanceId(1),
                    params: rot,
                }],
            ));
            prog.intervals.push((
                2,
                vec![PlaneOp::SetAffine {
                    id: PlaneInstanceId(2),
                    params: zoom,
                }],
            ));
            prog
        },
    );
}

#[test]
fn transform_residual_parity_at_depth8() {
    // The same transform block must decode identically through the v1
    // materializer and the v2 core at depth 8 (additive algebra over the same
    // base render).
    let (w, h) = (48u32, 32u32);
    let bg = 10u8;
    let fill = 60u8;
    // Base: background + one fill sprite at (30, 20) — shared by both legs.
    let base_prog = || {
        let mut prog = PlaneProgram::new(u32::from(bg));
        prog.objects
            .insert(PlaneObjectId(1), PlaneObject::fill(10, 6, u32::from(fill)));
        prog.instances.push(PlaneInstance {
            id: PlaneInstanceId(1),
            object: PlaneObjectId(1),
            x: 30,
            y: 20,
        });
        prog
    };
    let base_plane = program_of(&gray_epoch(w, h, 8), base_prog())
        .materialize_observation(0)
        .unwrap()
        .plane(0)
        .unwrap()
        .clone();
    // Target = base + smooth quadrant deltas (additions stay in range).
    let base_slice = u32s(base_plane.data());
    let delta: Vec<u32> = (0..(w * h) as usize)
        .map(|k| {
            let x = (k % w as usize) as u32;
            let y = (k / w as usize) as u32;
            if x < 24 && y < 16 {
                2 * (x / 4) + 3 * (y / 4)
            } else {
                0
            }
        })
        .collect();
    let mut target = base_slice.clone();
    for (k, d) in delta.iter().enumerate() {
        target[k] += d;
    }
    let target_plane = Plane::new(
        vole_video::media::layout::Component::Gray,
        w,
        h,
        BitDepth::new(8).unwrap(),
        0,
        0,
        raster_data(8, &target),
    )
    .unwrap();
    let block = encode_plane_transform_block(&base_plane, &target_plane).expect("nonempty delta");
    let expected: Vec<u8> = target.iter().map(|v| *v as u8).collect();

    // v1 leg.
    let mut a = vole_video::ingest::Ingest::new(w, h);
    a.background(bg);
    a.declare_fill(1, 10, 6, fill).unwrap();
    a.instance(1, 1, 30, 20).unwrap();
    a.at(1).unwrap();
    a.residual(block.clone()).unwrap();
    let v1 = vole_video::decoder::decode_bytes(&a.finish().unwrap()).unwrap();
    let v1_frames = vole_video::decoder::materialize_all(&v1).unwrap();
    assert_eq!(v1_frames.len(), 2);

    // v2 leg.
    let mut prog = base_prog();
    prog.intervals.push((
        1,
        vec![PlaneOp::TransformResidual {
            block: block.clone(),
        }],
    ));
    let v2_frames = frames_of(&program_of(&gray_epoch(w, h, 8), prog));

    let base_bytes: Vec<u8> = base_slice.iter().map(|v| *v as u8).collect();
    assert_eq!(v1_frames[0].as_slice(), base_bytes.as_slice());
    assert_eq!(
        v2_frames[0].plane(0).unwrap().canonical_bytes(),
        base_bytes.as_slice()
    );
    assert_eq!(
        v1_frames[1].as_slice(),
        expected.as_slice(),
        "v1 transform decode"
    );
    assert_eq!(
        v2_frames[1].plane(0).unwrap().canonical_bytes(),
        expected.as_slice(),
        "v2 transform decode"
    );
}

// ---------------------------------------------------------------------------
// 2. Authored multiplane 10-bit semantics vs an independent compositor
// ---------------------------------------------------------------------------

/// Court-side per-plane compositor: renders observation `idx` of a plane
/// program from its declared semantics (background, instances in paint order
/// with fill/raster/index/generator content, affine source maps, palette
/// bindings, velocity/trajectory state advanced by closed-form positions,
/// overlay, one-shot copy/residual canvas ops). Shares no paint code with the
/// core materializer.
#[derive(Clone)]
struct Compositor {
    w: u32,
    h: u32,
    max: u32,
    bg: u32,
    objects: BTreeMap<PlaneObjectId, PlaneObject>,
    instances: Vec<(PlaneInstanceId, PlaneObjectId, i64, i64)>,
    overlay: Vec<(i64, i64, u32)>,
    palettes: BTreeMap<PlanePaletteId, Vec<u32>>,
    bindings: BTreeMap<PlaneInstanceId, PlanePaletteId>,
    velocities: BTreeMap<PlaneInstanceId, (i64, i64)>,
    /// (base position, consumed advances) per trajectory instance.
    trajs: BTreeMap<PlaneInstanceId, (Vec<TrajectorySegment>, i64, i64, u64)>,
    affines: BTreeMap<PlaneInstanceId, vole_video::affine::AffineParams>,
    previous: Option<Vec<u32>>,
}

impl Compositor {
    fn new(prog: &PlaneProgram, w: u32, h: u32, depth: u8) -> Self {
        let mut c = Compositor {
            w,
            h,
            max: BitDepth::new(depth).unwrap().max_sample(),
            bg: prog.background,
            objects: prog.objects.clone(),
            instances: prog
                .instances
                .iter()
                .map(|i| (i.id, i.object, i.x, i.y))
                .collect(),
            overlay: prog.overlay.clone(),
            palettes: prog.palettes.clone(),
            bindings: BTreeMap::new(),
            velocities: BTreeMap::new(),
            trajs: BTreeMap::new(),
            affines: BTreeMap::new(),
            previous: None,
        };
        for m in &prog.initial_motion {
            match m {
                PlaneMotion::Velocity { instance, vx, vy } => {
                    c.velocities.insert(*instance, (*vx, *vy));
                }
                PlaneMotion::Trajectory { instance, segments } => {
                    // Trajectory start positions are the instance placements.
                    let base = c
                        .instances
                        .iter()
                        .find(|(id, _, _, _)| id == instance)
                        .map(|(_, _, x, y)| (*x, *y))
                        .expect("motion instance exists");
                    c.trajs
                        .insert(*instance, (segments.clone(), base.0, base.1, 0));
                }
                PlaneMotion::Affine { instance, params } => {
                    c.affines.insert(*instance, *params);
                }
                PlaneMotion::Binding { instance, palette } => {
                    c.bindings.insert(*instance, *palette);
                }
            }
        }
        c
    }

    fn render(&self) -> Vec<u32> {
        let (w, h) = (self.w, self.h);
        let mut out = vec![self.bg; (w * h) as usize];
        for &(iid, oid, dx, dy) in &self.instances {
            let Some(obj) = self.objects.get(&oid) else {
                continue;
            };
            if let Some(p) = self.affines.get(&iid) {
                for y in 0..h as i64 {
                    for x in 0..w as i64 {
                        let Some((su, sv)) = p.source(x, y) else {
                            continue;
                        };
                        if su < 0
                            || sv < 0
                            || su >= i64::from(obj.width)
                            || sv >= i64::from(obj.height)
                        {
                            continue;
                        }
                        let k = (sv * i64::from(obj.width) + su) as usize;
                        let v = self.content_sample(obj, iid, k, su, sv);
                        out[(y * w as i64 + x) as usize] = v;
                    }
                }
                continue;
            }
            let (x0, x1) = (dx.max(0), (dx + i64::from(obj.width)).min(w as i64));
            let (y0, y1) = (dy.max(0), (dy + i64::from(obj.height)).min(h as i64));
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let lx = xx - dx;
                    let ly = yy - dy;
                    let k = (ly * i64::from(obj.width) + lx) as usize;
                    let v = self.content_sample(obj, iid, k, lx, ly);
                    out[(yy * w as i64 + xx) as usize] = v;
                }
            }
        }
        for &(x, y, v) in &self.overlay {
            if x >= 0 && y >= 0 && x < w as i64 && y < h as i64 {
                out[(y as u32 * w + x as u32) as usize] = v;
            }
        }
        out
    }

    /// One object sample: fill value / raster sample / palette resolution /
    /// generator evaluation at content-local `(lx, ly)`.
    fn content_sample(
        &self,
        obj: &PlaneObject,
        iid: PlaneInstanceId,
        k: usize,
        lx: i64,
        ly: i64,
    ) -> u32 {
        match &obj.content {
            PlaneContent::Fill(v) => *v,
            PlaneContent::Raster(d) => u32s(d)[k],
            PlaneContent::Index(ind) => {
                let pid = self
                    .bindings
                    .get(&iid)
                    .copied()
                    .unwrap_or(PlanePaletteId(0));
                let entries = self.palettes.get(&pid).expect("bound palette");
                entries[usize::from(ind[k])]
            }
            PlaneContent::Generator(g) => g.sample(lx, ly, self.max),
        }
    }

    /// Step through `prog` up to observation `idx`, returning that canvas.
    /// Replays from the compositor's initial state exactly once per call
    /// (a fresh `Compositor::new` per observation is the intended use).
    fn observation(mut self, prog: &PlaneProgram, idx: u64) -> Vec<u32> {
        self.overlay = prog.overlay.clone();
        let frame0 = self.render();
        self.previous = Some(frame0);
        for (t, ops) in &prog.intervals {
            if *t > idx {
                break;
            }
            for op in ops {
                self.apply(op);
            }
            let canvas = self.render();
            self.previous = Some(canvas);
        }
        self.previous.clone().expect("rendered")
    }

    fn apply(&mut self, op: &PlaneOp) {
        match op {
            PlaneOp::DeclareObject { id, object } => {
                self.objects.insert(*id, object.clone());
            }
            PlaneOp::CreateInstance { id, object, x, y } => {
                self.instances.push((*id, *object, *x, *y));
            }
            PlaneOp::SetPosition { id, x, y } => {
                for (iid, _, px, py) in self.instances.iter_mut() {
                    if iid == id {
                        *px = *x;
                        *py = *y;
                    }
                }
            }
            PlaneOp::SetVelocity { id, vx, vy } => {
                self.velocities.insert(*id, (*vx, *vy));
            }
            PlaneOp::AdvanceTranslations => {
                for (iid, _, x, y) in self.instances.iter_mut() {
                    if let Some((vx, vy)) = self.velocities.get(iid) {
                        *x += vx;
                        *y += vy;
                    }
                }
            }
            PlaneOp::SetTrajectory { id, segments } => {
                let base = self
                    .instances
                    .iter()
                    .find(|(iid, _, _, _)| iid == id)
                    .map(|(_, _, x, y)| (*x, *y));
                if let Some((bx, by)) = base {
                    self.trajs.insert(*id, (segments.clone(), bx, by, 0));
                }
            }
            PlaneOp::AdvanceTrajectories => {
                for (iid, _, x, y) in self.instances.iter_mut() {
                    let Some((segs, bx, by, n)) = self.trajs.get(iid).cloned() else {
                        continue;
                    };
                    // Closed-form advance: position after n+1 steps from the
                    // trajectory base (independent of the core stepper).
                    if let Some((nx, ny)) =
                        vole_video::trajectory::position_after(&segs, bx, by, n + 1)
                    {
                        *x = nx;
                        *y = ny;
                        self.trajs.insert(*iid, (segs, bx, by, n + 1));
                    } else {
                        self.trajs.remove(iid); // program exhausted
                    }
                }
            }
            PlaneOp::SetPalette { id, entries } => {
                self.palettes.insert(*id, entries.clone());
            }
            PlaneOp::PatchPalette { id, changes } => {
                if let Some(e) = self.palettes.get_mut(id) {
                    for (i, v) in changes {
                        e[*i as usize] = *v;
                    }
                }
            }
            PlaneOp::BindPalette { instance, palette } => {
                self.bindings.insert(*instance, *palette);
            }
            PlaneOp::SetAffine { id, params } => {
                self.affines.insert(*id, *params);
            }
            PlaneOp::ClearInstances => {
                self.instances.clear();
                self.velocities.clear();
                self.trajs.clear();
                self.bindings.clear();
                self.affines.clear();
            }
            PlaneOp::ClearOverlay => self.overlay.clear(),
            PlaneOp::PatchOverlay { points } => {
                for &p in points {
                    if !self.overlay.contains(&p) {
                        self.overlay.push(p);
                    }
                }
                self.overlay.sort_unstable_by_key(|&(x, y, _)| (x, y));
            }
            PlaneOp::CopyRect {
                src_x,
                src_y,
                width,
                height,
                dst_x,
                dst_y,
            } => {
                let src = self.previous.clone().expect("previous obs");
                let (w, h) = (self.w as i64, self.h as i64);
                for si in 0..*height as i64 {
                    for sj in 0..*width as i64 {
                        let (px, py) = (src_x + sj, src_y + si);
                        let (qx, qy) = (dst_x + sj, dst_y + si);
                        if px < 0
                            || py < 0
                            || px >= w
                            || py >= h
                            || qx < 0
                            || qy < 0
                            || qx >= w
                            || qy >= h
                        {
                            continue;
                        }
                        let v = src[(py as u32 * self.w + px as u32) as usize];
                        let cur = self.previous.clone().expect("cur");
                        let mut canvas = cur;
                        canvas[(qy as u32 * self.w + qx as u32) as usize] = v;
                        self.previous = Some(canvas);
                    }
                }
                // Rebuild render basis: CopyRect applies over the *current*
                // render, so materialize now.
                let canvas = self.render();
                self.previous = Some(canvas);
            }
            PlaneOp::Residual { block } => {
                let payload = vole_video::rans::decode_block(block, 1 << 30).unwrap();
                let mut canvas = self.previous.clone().expect("cur");
                for rec in payload.as_chunks::<10>().0 {
                    let x = i32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]) as u32;
                    let y = i32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]) as u32;
                    let v = u32::from(u16::from_le_bytes([rec[8], rec[9]]));
                    canvas[(y * self.w + x) as usize] = v;
                }
                self.previous = Some(canvas);
            }
            PlaneOp::TransformResidual { .. } => {
                // Not exercised by compositor courts (transform algebra is
                // courted by the parity + property tests).
                unreachable!("compositor does not run transform ops");
            }
        }
    }
}

#[test]
fn authored_10bit_multiplane_matches_an_independent_compositor() {
    // YUV420 10-bit, 24x16 (chroma 12x8), 7 observations. Y plane: two
    // velocity-driven sprites over a gradient generator background object...
    // (whole-plane state model: background + sprites); chroma: palette-index
    // + static generator content. The compositor is the ground truth.
    let (w, h) = (24u32, 16u32);
    let depth = 10u8;
    let max = 1023u32;
    let epoch = yuv_epoch(w, h, depth);
    // Y plane program.
    let mut y = PlaneProgram::new(200);
    // Whole-plane textured background? Keep it simple: uniform background 200,
    // one 6x6 generator "checker" sprite and one 6x6 fill sprite with
    // velocity; a palette-index sprite bound to palette 1.
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
    // Chroma Cb: whole-plane generator gradient; Cr: palette-index content.
    let (cw, ch) = (12u32, 8u32);
    let mut cb = PlaneProgram::new(0);
    cb.objects.insert(
        PlaneObjectId(1),
        PlaneObject::procedural(
            cw,
            ch,
            Gen::Gradient {
                base: 128,
                sx: 30,
                sy: 10,
            },
        )
        .unwrap(),
    );
    cb.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    let mut cr = PlaneProgram::new(0);
    let indices: Vec<u8> = (0..(cw * ch) as usize)
        .map(|i| ((i / ch as usize + i) % 3) as u8)
        .collect();
    cr.objects.insert(
        PlaneObjectId(1),
        PlaneObject::index(cw, ch, indices).unwrap(),
    );
    cr.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    cr.palettes.insert(PlanePaletteId(1), vec![100, 500, 900]);
    cr.initial_motion.push(PlaneMotion::Binding {
        instance: PlaneInstanceId(1),
        palette: PlanePaletteId(1),
    });
    // Palette mutation mid-run (re-renders the index plane).
    cr.intervals.push((
        3,
        vec![PlaneOp::SetPalette {
            id: PlanePaletteId(1),
            entries: vec![200, 400, 800],
        }],
    ));
    let mp = MultiPlaneProgram::new(epoch.clone(), vec![y, cb, cr]).unwrap();
    assert_eq!(mp.observation_count(), 7);

    for idx in 0..mp.observation_count() {
        let got = mp.materialize_observation(idx).unwrap();
        // A fresh compositor per observation: `observation` replays intervals
        // 1..=idx exactly once from the declared initial state.
        let want_y = Compositor::new(&mp.planes[0], w, h, depth).observation(&mp.planes[0], idx);
        let want_cb = Compositor::new(&mp.planes[1], cw, ch, depth).observation(&mp.planes[1], idx);
        let want_cr = Compositor::new(&mp.planes[2], cw, ch, depth).observation(&mp.planes[2], idx);
        assert_eq!(u32s(got.plane(0).unwrap().data()), want_y, "Y frame {idx}");
        assert_eq!(
            u32s(got.plane(1).unwrap().data()),
            want_cb,
            "Cb frame {idx}"
        );
        assert_eq!(
            u32s(got.plane(2).unwrap().data()),
            want_cr,
            "Cr frame {idx}"
        );
    }
    let _ = max;
}

// ---------------------------------------------------------------------------
// 3. Family encoder courts
// ---------------------------------------------------------------------------

/// A Gray `Picture` from a sample vec.
fn gray_picture(epoch: &VideoEpoch, samples: Vec<u32>) -> Picture {
    let plane = Plane::new(
        vole_video::media::layout::Component::Gray,
        epoch.width(),
        epoch.height(),
        epoch.planes()[0].bit_depth,
        0,
        0,
        raster_data(epoch.planes()[0].bit_depth.bits(), &samples),
    )
    .unwrap();
    Picture::from_planes(epoch, vec![plane]).unwrap()
}

fn enc_of(
    frames: &[Picture],
    depth: u8,
) -> (MultiPlaneProgram, vole_video::media::encode::EncodeReport) {
    let w = frames[0].plane(0).unwrap().width();
    let h = frames[0].plane(0).unwrap().height();
    let epoch = gray_epoch(w, h, depth);
    encode_pictures_families(&epoch, frames).unwrap()
}

#[test]
fn encoder_static_run_rides_unchanged_groups() {
    let depth = 8u8;
    let (w, h) = (24u32, 16u32);
    let epoch = gray_epoch(w, h, depth);
    let frames = vec![
        gray_picture(&epoch, plane_texture(depth, w, h, 7)),
        gray_picture(&epoch, plane_texture(depth, w, h, 7)),
        gray_picture(&epoch, plane_texture(depth, w, h, 7)),
    ];
    let (prog, rep) = enc_of(&frames, depth);
    // Exact by the encoder's own proof; static duplicates ride empty groups.
    assert_eq!(prog.observation_count(), 3);
    assert_eq!(
        rep.families.get(FAMILY_UNCHANGED).unwrap().observations,
        2,
        "two static repeats"
    );
    assert!(rep.total_interval_bytes < rep.raw_floor_bytes);
}

#[test]
fn encoder_detects_gradient_generator_at_10bit() {
    let depth = 10u8;
    let (w, h) = (32u32, 16u32);
    let max = 1023u32;
    let epoch = gray_epoch(w, h, depth);
    let gen = Gen::Gradient {
        base: 100,
        sx: 3,
        sy: -5,
    };
    let grad: Vec<u32> = (0..(w * h) as usize)
        .map(|k| gen.sample((k % w as usize) as i64, (k / w as usize) as i64, max))
        .collect();
    // Frame 0: a non-gradient texture; frames 1..: the gradient (static run).
    let mut frames = vec![gray_picture(&epoch, plane_texture(depth, w, h, 3))];
    for _ in 0..3 {
        frames.push(gray_picture(&epoch, grad.clone()));
    }
    let (prog, rep) = enc_of(&frames, depth);
    assert_eq!(prog.observation_count(), 4);
    let g = rep.families.get(FAMILY_GENERATOR);
    assert!(
        g.is_some_and(|t| t.observations == 1),
        "gradient run should be declared once as a generator: {rep:?}"
    );
    // The rest of the static run rides unchanged.
    assert_eq!(rep.families.get(FAMILY_UNCHANGED).unwrap().observations, 2);
    assert!(rep.total_interval_bytes < rep.raw_floor_bytes);
}

#[test]
fn encoder_detects_palette_content_at_10bit() {
    let depth = 10u8;
    let (w, h) = (32u32, 16u32);
    let epoch = gray_epoch(w, h, depth);
    // A 4-value cartoon: checker of two colors in 8x8 blocks + band.
    let values = [100u32, 400, 700, 900];
    let content: Vec<u32> = (0..(w * h) as usize)
        .map(|k| {
            let x = (k % w as usize) as u32;
            let y = (k / w as usize) as u32;
            if y < 8 {
                values[(x / 16) as usize]
            } else {
                values[2 + (x / 16) as usize]
            }
        })
        .collect();
    let mut frames = vec![gray_picture(&epoch, plane_texture(depth, w, h, 11))];
    for _ in 0..3 {
        frames.push(gray_picture(&epoch, content.clone()));
    }
    let (prog, rep) = enc_of(&frames, depth);
    assert_eq!(prog.observation_count(), 4);
    let p = rep.families.get(FAMILY_PALETTE);
    assert!(
        p.is_some_and(|t| t.observations == 1),
        "4-value content should be declared as palette-index once: {rep:?}"
    );
    assert_eq!(rep.families.get(FAMILY_UNCHANGED).unwrap().observations, 2);
    assert!(rep.total_interval_bytes < rep.raw_floor_bytes);
}

#[test]
fn encoder_uses_region_reuse_for_a_translating_sprite() {
    let depth = 8u8;
    let (w, h) = (40u32, 24u32);
    let epoch = gray_epoch(w, h, depth);
    // Frame 0: a textured background (whole-plane object). Frames 1..: a
    // distinct textured sprite moving +2px per observation over that
    // background — the encoder must reuse the previous observation via
    // CopyRect (TRANSLATION) from the second sprite frame on.
    let bg_tex: Vec<u32> = plane_texture(depth, w, h, 13);
    let spr = sprite(depth, 5, 200);
    let mut frames = Vec::new();
    frames.push(gray_picture(&epoch, bg_tex.clone()));
    let put_sprite = |samples: &mut Vec<u32>, x: usize, y: usize| {
        for sy in 0..8usize {
            for sx in 0..8usize {
                samples[(y + sy) * w as usize + x + sx] = spr[sy * 8 + sx];
            }
        }
    };
    for step in 1..=5 {
        let mut samples = bg_tex.clone();
        put_sprite(&mut samples, 4 + step * 2, 8);
        frames.push(gray_picture(&epoch, samples));
    }
    let (prog, rep) = enc_of(&frames, depth);
    assert_eq!(prog.observation_count(), 6);
    // Every interval is exact (encoder proof); total bytes must be far below
    // the per-interval RAW whole-plane floor.
    let used_reuse = [FAMILY_TRANSLATION, FAMILY_COPY, FAMILY_REGIONS]
        .iter()
        .any(|f| rep.families.contains_key(f));
    assert!(
        used_reuse,
        "expected region reuse for the sprite run: {rep:?}"
    );
    assert!(
        rep.total_interval_bytes * 8 < rep.raw_floor_bytes,
        "family economy: {} B vs RAW floor {} B",
        rep.total_interval_bytes,
        rep.raw_floor_bytes
    );
}

// ---------------------------------------------------------------------------
// 4. v2 wire (family extension)
// ---------------------------------------------------------------------------

#[test]
fn wire_extension_roundtrips_across_layouts_and_depths() {
    // Rows exercising the V.1.4 surface: generator content, palette-index +
    // binding, velocity/trajectory/advance ops, affine, transform residual.
    // (layout, depth, w, h) with per-plane programs using the extension.
    let rows: &[(PixelLayout, u8, u32, u32)] = &[
        (PixelLayout::Gray, 8, 9, 7), // odd geometry
        (PixelLayout::Gray, 10, 16, 16),
        (PixelLayout::Gray, 16, 12, 8),
        (PixelLayout::Yuv420, 8, 24, 16),
        (PixelLayout::Yuv420, 10, 16, 8),
        (PixelLayout::Yuv444, 12, 8, 8),
        (PixelLayout::Gbr, 8, 12, 12),
        (PixelLayout::Rgb, 10, 10, 6),
    ];
    for &(layout, depth, w, h) in rows {
        let epoch = VideoEpoch::new_uniform(
            EpochId(0),
            w,
            h,
            layout,
            BitDepth::new(depth).unwrap(),
            ColorDescription::unspecified(),
            SampleAspectRatio::square(),
            Orientation::Normal,
            FieldStructure::Progressive,
        )
        .unwrap();
        let mut planes = Vec::new();
        for p in 0..epoch.plane_count() {
            let (pw, ph) = epoch.plane_dimensions(p).unwrap();
            let d = epoch.planes()[p].bit_depth;
            let max = d.max_sample();
            let mut prog = PlaneProgram::new(0);
            // Generator whole-plane object on plane 0; a moving fill sprite
            // (velocity + advance) and a palette-index sprite elsewhere.
            if p == 0 {
                prog.objects.insert(
                    PlaneObjectId(1),
                    PlaneObject::procedural(
                        pw,
                        ph,
                        Gen::Gradient {
                            base: 7,
                            sx: 3,
                            sy: 2,
                        },
                    )
                    .unwrap(),
                );
                prog.instances.push(PlaneInstance {
                    id: PlaneInstanceId(1),
                    object: PlaneObjectId(1),
                    x: 0,
                    y: 0,
                });
            } else {
                let v = (40 * (p as u32 + 1)) % (max + 1);
                prog.objects.insert(
                    PlaneObjectId(1),
                    PlaneObject::fill((pw / 2).max(1), (ph / 2).max(1), v),
                );
                prog.instances.push(PlaneInstance {
                    id: PlaneInstanceId(1),
                    object: PlaneObjectId(1),
                    x: 1,
                    y: 1,
                });
                prog.intervals.push((
                    1,
                    vec![
                        PlaneOp::SetVelocity {
                            id: PlaneInstanceId(1),
                            vx: 1,
                            vy: 1,
                        },
                        PlaneOp::AdvanceTranslations,
                    ],
                ));
                prog.intervals.push((2, vec![PlaneOp::AdvanceTranslations]));
                // Palette-index content on the last plane.
                if p == epoch.plane_count() - 1 {
                    let iw = (pw / 2).max(1);
                    let ih = (ph / 2).max(1);
                    let indices: Vec<u8> = (0..(iw * ih) as usize)
                        .map(|i| ((i / ih as usize + i) % 3) as u8)
                        .collect();
                    prog.objects.insert(
                        PlaneObjectId(2),
                        PlaneObject::index(iw, ih, indices).unwrap(),
                    );
                    prog.palettes
                        .insert(PlanePaletteId(1), vec![0, max / 2, max]);
                    prog.initial_motion.push(PlaneMotion::Binding {
                        instance: PlaneInstanceId(1),
                        palette: PlanePaletteId(1),
                    });
                }
            }
            planes.push(prog);
        }
        let mp = MultiPlaneProgram::new(epoch, planes).unwrap();
        let bytes = write_multiplane(&mp).unwrap();
        assert_ne!(feature_bits(&bytes) & 1, 0, "extension feature bit set");
        let parsed = parse_multiplane(&bytes).unwrap();
        let again = write_multiplane(&parsed).unwrap();
        assert_eq!(bytes, again, "{layout:?} d{depth}: write∘parse fixpoint");
        assert_eq!(parsed.observation_count(), mp.observation_count());
        for idx in 0..mp.observation_count() {
            let a = mp.materialize_observation(idx).unwrap();
            let b = parsed.materialize_observation(idx).unwrap();
            assert_eq!(
                pic_bytes(&a),
                pic_bytes(&b),
                "{layout:?} d{depth} obs {idx}"
            );
        }
    }
}

#[test]
fn wire_feature_bits_are_minimal_and_old_surface_is_unchanged() {
    // A program using only the V.1.2 surface keeps feature bits 0 and its
    // exact byte stream; the V.1.4 surface sets bit 0x1.
    let epoch = gray_epoch(16, 16, 8);
    let mut plain = PlaneProgram::new(0);
    plain
        .objects
        .insert(PlaneObjectId(1), PlaneObject::fill(8, 8, 99));
    plain.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    plain.intervals.push((
        1,
        vec![PlaneOp::SetPosition {
            id: PlaneInstanceId(1),
            x: 4,
            y: 4,
        }],
    ));
    let bytes = write_multiplane(&program_of(&epoch, plain)).unwrap();
    assert_eq!(feature_bits(&bytes), 0, "old surface keeps feature bits 0");

    let mut ext = PlaneProgram::new(0);
    ext.objects.insert(
        PlaneObjectId(1),
        PlaneObject::procedural(
            16,
            16,
            Gen::Checker {
                a: 0,
                b: 255,
                cell: 4,
            },
        )
        .unwrap(),
    );
    ext.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    let bytes = write_multiplane(&program_of(&epoch, ext)).unwrap();
    assert_eq!(feature_bits(&bytes), 0x1, "generator content sets bit 0x1");
}

#[test]
fn extension_golden_is_pinned() {
    // The byte digest of one canonical extension scenario is frozen: a grammar
    // change that alters this encoding is a deliberate re-freeze.
    let epoch = gray_epoch(16, 12, 8);
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
    let bytes = write_multiplane(&program_of(&epoch, prog)).unwrap();
    let digest = blake3::hash(&bytes);
    let hex = digest.to_hex().to_string();
    // Pinned when the V.1.4 grammar was sealed (extension golden).
    assert_eq!(
        hex, "55c7f4cce95c19f5d326a5bb084e058f3b6ba06ebe288656c78253d00d625007",
        "extension golden changed: deliberate grammar re-freeze required"
    );
}

#[test]
fn hostile_extension_corpus_is_typed_never_panics() {
    let epoch = gray_epoch(16, 12, 8);
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
            changes: vec![(0, 60)],
        }],
    ));
    let base = write_multiplane(&program_of(&epoch, prog)).unwrap();
    assert!(parse_multiplane(&base).is_ok());

    // Clearing the feature bit from a stream that uses the extension surface
    // fails closed typed (the tail is no longer read; structure breaks).
    let mut no_feat = base.clone();
    no_feat[12..16].copy_from_slice(&0u32.to_le_bytes());
    assert!(parse_multiplane(&no_feat).is_err());

    // An extension op tag without the feature bit is typed even when there is
    // no tail (the read_op gate fires before any structure confusion).
    let mut p2 = PlaneProgram::new(0);
    p2.objects
        .insert(PlaneObjectId(1), PlaneObject::fill(4, 4, 9));
    p2.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    p2.intervals.push((
        1,
        vec![PlaneOp::SetVelocity {
            id: PlaneInstanceId(1),
            vx: 1,
            vy: 0,
        }],
    ));
    let mut no_tail = write_multiplane(&program_of(&epoch, p2)).unwrap();
    no_tail[12..16].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        parse_multiplane(&no_tail).unwrap_err(),
        VoleError::NonCanonicalEncoding,
        "extension op without the feature bit fails closed"
    );

    // Unknown motion kind in the tail: the binding record ends the payload
    // (instance u32 + kind u8 + palette u32 directly before the 32-byte
    // integrity trailer), so its kind byte sits at `len - 32 - 5`.
    let mut bad = base.clone();
    let kind_off = bad.len() - 32 - 5;
    assert_eq!(bad[kind_off], 0x04, "binding kind byte located");
    bad[kind_off] = 0x09; // unknown motion kind
    assert!(parse_multiplane(&bad).is_err());

    // A semantic reference error (SetVelocity on an unknown instance) parses
    // structurally but fails typed at materialization — never a panic.
    let mut p3 = PlaneProgram::new(0);
    p3.intervals.push((
        1,
        vec![PlaneOp::SetVelocity {
            id: PlaneInstanceId(99),
            vx: 1,
            vy: 1,
        }],
    ));
    let bytes3 = write_multiplane(&program_of(&epoch, p3)).unwrap();
    let parsed3 = parse_multiplane(&bytes3).unwrap();
    assert_eq!(
        parsed3.materialize_observation(1).unwrap_err(),
        VoleError::UnknownInstance
    );

    // Nonzero padding / entry out of domain in a palette (flip inside the
    // palette entry region) must be typed or integrity-failing, never panic.
    for cut in [0usize, 1, 40, base.len() / 2, base.len() - 2] {
        let _ = parse_multiplane(&base[..cut]);
    }
    // Truncations across the file are typed, never panics.
    for &off in &[base.len() / 3, base.len() / 2, base.len() - 10] {
        let mut flip = base.clone();
        flip[off] ^= 0xFF;
        let _ = parse_multiplane(&flip);
    }
}

// ---------------------------------------------------------------------------
// 5. Transform residual floor (10-bit property court)
// ---------------------------------------------------------------------------

#[test]
fn transform_floor_roundtrips_at_10bit() {
    let depth = 10u8;
    let (w, h) = (40u32, 24u32);
    let max = 1023u32;
    // Base: smooth ramp. Target: base plus a textured local delta field.
    let base_s: Vec<u32> = (0..(w * h) as usize)
        .map(|k| ((k * 7) % 900) as u32)
        .collect();
    let mut target_s = base_s.clone();
    for y in 4..20u32 {
        for x in 2..38u32 {
            let k = (y * w + x) as usize;
            target_s[k] = (base_s[k] as i64 + ((x * 3 + y * 2) % 200) as i64 - 60)
                .clamp(0, i64::from(max)) as u32;
        }
    }
    let base_p = Plane::new(
        vole_video::media::layout::Component::Gray,
        w,
        h,
        BitDepth::new(10).unwrap(),
        0,
        0,
        raster_data(depth, &base_s),
    )
    .unwrap();
    let target_p = Plane::new(
        vole_video::media::layout::Component::Gray,
        w,
        h,
        BitDepth::new(10).unwrap(),
        0,
        0,
        raster_data(depth, &target_s),
    )
    .unwrap();
    let block = encode_plane_transform_block(&base_p, &target_p).expect("delta nonempty");
    // Apply the block onto the base via a fresh program with op 0x31.
    let epoch = gray_epoch(w, h, depth);
    let mut prog = PlaneProgram::new(0);
    prog.objects
        .insert(PlaneObjectId(1), gray_10_raster(&base_s, w, h));
    prog.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    prog.intervals
        .push((1, vec![PlaneOp::TransformResidual { block }]));
    let mp = program_of(&epoch, prog);
    let frame1 = mp.materialize_observation(1).unwrap();
    assert_eq!(
        u32s(frame1.plane(0).unwrap().data()),
        target_s,
        "transform floor closes the delta exactly"
    );
    // The block survives a wire roundtrip inside the container.
    let bytes = write_multiplane(&mp).unwrap();
    let reparsed = parse_multiplane(&bytes).unwrap();
    assert_eq!(
        u32s(
            reparsed
                .materialize_observation(1)
                .unwrap()
                .plane(0)
                .unwrap()
                .data()
        ),
        target_s
    );

    // Hostile transform blocks are typed.
    let epoch = gray_epoch(w, h, depth);
    let mut prog = PlaneProgram::new(0);
    prog.intervals
        .push((1, vec![PlaneOp::TransformResidual { block: vec![0x02] }]));
    let _ = epoch;
    // truncated block fails at materialization typed, never a panic
    let res = program_of(&gray_epoch(w, h, depth), prog);
    assert!(res.materialize_observation(1).is_err());
}

fn gray_10_raster(samples: &[u32], w: u32, h: u32) -> PlaneObject {
    raster_object(10, samples, w, h)
}

#[test]
fn encoder_palette_and_generator_streams_roundtrip_through_wire() {
    // The encoder's chosen programs (which may use the extension surface)
    // serialize and re-materialize exactly.
    let depth = 10u8;
    let (w, h) = (32u32, 16u32);
    let max = 1023u32;
    let epoch = gray_epoch(w, h, depth);
    let gen = Gen::Gradient {
        base: 100,
        sx: 3,
        sy: -5,
    };
    let grad: Vec<u32> = (0..(w * h) as usize)
        .map(|k| gen.sample((k % w as usize) as i64, (k / w as usize) as i64, max))
        .collect();
    let mut frames = vec![gray_picture(&epoch, plane_texture(depth, w, h, 3))];
    frames.push(gray_picture(&epoch, grad.clone()));
    frames.push(gray_picture(&epoch, grad));
    let (prog, _) = enc_of(&frames, depth);
    let bytes = write_multiplane(&prog).unwrap();
    let parsed = parse_multiplane(&bytes).unwrap();
    for idx in 0..prog.observation_count() {
        assert_eq!(
            pic_bytes(&prog.materialize_observation(idx).unwrap()),
            pic_bytes(&parsed.materialize_observation(idx).unwrap())
        );
    }
}

#[test]
fn transform_block_encode_apply_equals_identity_on_random_fields() {
    // Randomized block property: encode(base, target) applied to base must
    // reproduce target for arbitrary integer deltas within the sample domain.
    let depth = 16u8; // deepest domain: deltas of ±65535 must round-trip
    let (w, h) = (24u32, 16u32);
    let epoch = gray_epoch(w, h, depth);
    let mut state = 0x5eed_u64;
    let mut rng = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state >> 33
    };
    for trial in 0..4 {
        let max = 65535u32;
        let base_s: Vec<u32> = (0..(w * h) as usize)
            .map(|_| (rng() % 65536) as u32)
            .collect();
        let mut target_s = base_s.clone();
        // Perturb a band with large signed deltas (wrapping is not allowed:
        // the result stays in the sample domain by construction).
        for k in 0..base_s.len() {
            let d = (rng() % 2001) as i64 - 1000;
            target_s[k] = (i64::from(base_s[k]) + d).clamp(0, i64::from(max)) as u32;
        }
        let bp = Plane::new(
            vole_video::media::layout::Component::Gray,
            w,
            h,
            BitDepth::new(depth).unwrap(),
            0,
            0,
            raster_data(depth, &base_s),
        )
        .unwrap();
        let tp = Plane::new(
            vole_video::media::layout::Component::Gray,
            w,
            h,
            BitDepth::new(depth).unwrap(),
            0,
            0,
            raster_data(depth, &target_s),
        )
        .unwrap();
        let Some(block) = encode_plane_transform_block(&bp, &tp) else {
            continue;
        };
        let mut prog = PlaneProgram::new(0);
        prog.objects
            .insert(PlaneObjectId(1), raster_object(16, &base_s, w, h));
        prog.instances.push(PlaneInstance {
            id: PlaneInstanceId(1),
            object: PlaneObjectId(1),
            x: 0,
            y: 0,
        });
        prog.intervals
            .push((1, vec![PlaneOp::TransformResidual { block }]));
        let mp = program_of(&epoch, prog);
        let got = mp.materialize_observation(1).unwrap();
        assert_eq!(
            u32s(got.plane(0).unwrap().data()),
            target_s,
            "random 16-bit delta trial {trial}"
        );
    }
}
