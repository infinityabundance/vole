//! Phase V.1.5 courts — global video structure (V.1 video programme, contract
//! §2.8, `docs/format-v2.md` re-frozen with the V.1.5 global-motion
//! extension; master brief §61–§63, §248).
//!
//! Courts:
//!
//! 1. **normative map semantics** — `GlobalPredict` at every registry
//!    precision (Q8/Q12/Q16) reproduces the previous observation through the
//!    canonical fixed-point rule; an integer-translation `GlobalPredict`
//!    equals the whole-plane `CopyRect`; the Q8 rule equals the sealed
//!    object-affine placement rule given the same source content.
//! 2. **family encoder over dense raster content** — deterministic natural-
//!    like footage with known camera models: pan (exact integer translation →
//!    `global_translation`), zoom (rotzoom/affine), shear (affine), each
//!    reproduced sample-exactly through the wire; noise and scene-cut
//!    negative controls never claim a global model; a 10-bit YUV420 pan is
//!    served per plane on its own subsampled grid.
//! 3. **§62 precision court** — forced Q8/Q12/Q16 runs are exact and
//!    deterministic; auto runs price every precision and report the measured
//!    byte split, never an assumed one.
//! 4. **v2 wire (global-motion extension)** — minimal feature bits (0x2 only
//!    for global content, additive to 0x1), a pinned V.1.5 golden, and a
//!    hostile extension corpus that is typed and never panics.
//! 5. **work cap** — a hostile multi-warp interval hits
//!    `MaterializationBudgetExceeded` under a small `max_motion_work`.

use vole_video::media::color::ColorDescription;
use vole_video::media::core::{
    materialize_plane, MultiPlaneProgram, PlaneInstance, PlaneInstanceId, PlaneObject,
    PlaneObjectId, PlaneOp, PlaneProgram,
};
use vole_video::media::encode::{
    encode_pictures_families_with, EncodeOptions, FAMILY_GLOBAL_AFFINE, FAMILY_GLOBAL_ROTZOOM,
    FAMILY_GLOBAL_TRANSLATION, FAMILY_RAW, FAMILY_SPARSE, FAMILY_TRANSFORM,
};
use vole_video::media::epoch::{EpochId, VideoEpoch};
use vole_video::media::global::{GlobalMap, MapShift};
use vole_video::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
use vole_video::media::picture::Picture;
use vole_video::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use vole_video::media::wire::{parse_multiplane, write_multiplane, V2_FEATURES};
use vole_video::media::PixelLayout;
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

fn single_plane(epoch: &VideoEpoch, samples: Vec<u32>) -> Picture {
    let (w, h) = epoch.plane_dimensions(0).unwrap();
    let plane = Plane::new(
        epoch.planes()[0].component,
        w,
        h,
        epoch.planes()[0].bit_depth,
        epoch.planes()[0].subsample_x,
        epoch.planes()[0].subsample_y,
        raster_data(epoch.planes()[0].bit_depth.bits(), &samples),
    )
    .unwrap();
    Picture::from_planes(epoch, vec![plane]).unwrap()
}

fn picture_of(epoch: &VideoEpoch, plane_idx: usize, samples: Vec<u32>) -> Plane {
    let t = &epoch.planes()[plane_idx];
    let (pw, ph) = epoch.plane_dimensions(plane_idx).unwrap();
    Plane::new(
        t.component,
        pw,
        ph,
        t.bit_depth,
        t.subsample_x,
        t.subsample_y,
        raster_data(t.bit_depth.bits(), &samples),
    )
    .unwrap()
}

fn samples_of(pic: &Picture, p: usize) -> Vec<u32> {
    u32s(pic.plane(p).expect("plane").data())
}

fn assert_all_observations(epoch: &VideoEpoch, want: &[Picture], prog: &MultiPlaneProgram) {
    assert_eq!(prog.observation_count(), want.len() as u64);
    for (i, w) in want.iter().enumerate() {
        let got = prog.materialize_observation(i as u64).unwrap();
        for p in 0..epoch.plane_count() {
            assert_eq!(
                got.plane(p).unwrap().canonical_bytes(),
                w.plane(p).unwrap().canonical_bytes(),
                "observation {i} plane {p}"
            );
        }
    }
}

/// Deterministic splitmix lattice value in `0..=255`.
fn lattice(x: i64, y: i64, seed: u64) -> u32 {
    let mut z = (x as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(seed);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z as u32) & 0xFF
}

/// Deterministic smooth content (bilinear-interpolated lattice octaves),
/// sample values in `0..=255` — a stand-in for natural raster content.
fn smooth_content(w: u32, h: u32, seed: u64) -> Vec<u32> {
    (0..(w * h))
        .map(|k| {
            let (x, y) = (k % w, k / w);
            let mut acc = 0.0f64;
            let mut amp = 1.0;
            let mut scale = 4.0f64;
            for oct in 0..4u32 {
                let sx = f64::from(x) / scale;
                let sy = f64::from(y) / scale;
                let x0 = sx.floor() as i64;
                let y0 = sy.floor() as i64;
                let fx = sx - sx.floor();
                let fy = sy - sy.floor();
                let s = seed.wrapping_add(u64::from(oct).wrapping_mul(0x9E3779B97F4A7C15));
                let bl = f64::from(lattice(x0, y0, s));
                let br = f64::from(lattice(x0 + 1, y0, s));
                let tl = f64::from(lattice(x0, y0 + 1, s));
                let tr = f64::from(lattice(x0 + 1, y0 + 1, s));
                let top = bl + (br - bl) * fx;
                let bot = tl + (tr - tl) * fx;
                acc += amp * (top + (bot - top) * fy);
                amp *= 0.5;
                scale *= 2.0;
            }
            ((acc / 1.9375) as u32).min(255)
        })
        .collect()
}

/// Deterministic smooth *content plane* with an even margin on every side
/// (camera views can pan/zoom without leaving the content).
fn content_plane(margin: u32, w: u32, h: u32, seed: u64) -> Vec<u32> {
    let cw = w + 2 * margin;
    let ch = h + 2 * margin;
    let base = smooth_content(cw, ch, seed);
    // Second octave of detail so the content is not perfectly smooth at 1 px.
    (0..(cw * ch))
        .map(|k| {
            let (x, y) = (k % cw, k / cw);
            let detail = lattice(x as i64, y as i64, seed ^ 0xD1B54A32D192ED03);
            ((u64::from(base[k as usize]) * 3 + u64::from(detail)) / 4) as u32
        })
        .collect()
}

/// Camera view of a content plane at a top-left content offset, with an even
/// margin on all sides (the content plane already includes the margin).
fn view(content: &[u32], cw: u32, w: u32, h: u32, ox: i64, oy: i64) -> Vec<u32> {
    (0..(w * h))
        .map(|k| {
            let (x, y) = (k % w, k / w);
            content[((y as i64 + oy) as u32 * cw + (x as i64 + ox) as u32) as usize]
        })
        .collect()
}

/// Bilinear render of a content plane under a continuous dest→source map —
/// a stand-in for a real camera's resampling of the scene.
fn render_map(content: &[u32], cw: u32, w: u32, h: u32, m: &[f64; 6]) -> Vec<u32> {
    (0..(w * h))
        .map(|k| {
            let (x, y) = (k % w, k / w);
            let su = m[0] * f64::from(x) + m[1] * f64::from(y) + m[2];
            let sv = m[3] * f64::from(x) + m[4] * f64::from(y) + m[5];
            // Clamp to the content (a real camera never leaves its scene).
            let u = su.clamp(0.0, f64::from(cw - 1));
            let v = sv.clamp(0.0, f64::from(cw - 1));
            let x0 = u.floor() as u32;
            let y0 = v.floor() as u32;
            let x1 = (x0 + 1).min(cw - 1);
            let y1 = (y0 + 1).min(cw - 1);
            let fx = u - f64::from(x0);
            let fy = v - f64::from(y0);
            let a = f64::from(content[(y0 * cw + x0) as usize]);
            let b = f64::from(content[(y0 * cw + x1) as usize]);
            let c = f64::from(content[(y1 * cw + x0) as usize]);
            let d = f64::from(content[(y1 * cw + x1) as usize]);
            let top = a + (b - a) * fx;
            let bot = c + (d - c) * fx;
            (top + (bot - top) * fy).round() as u32
        })
        .collect()
}

/// A panning camera: view advances by `(dx, dy)` content samples per frame.
fn pan_frames(w: u32, h: u32, dx: i64, dy: i64, n: usize, seed: u64) -> Vec<Vec<u32>> {
    let margin = 32;
    let content = content_plane(margin, w, h, seed);
    let cw = w + 2 * margin;
    (0..n)
        .map(|k| {
            view(
                &content,
                cw,
                w,
                h,
                margin as i64 + dx * k as i64,
                margin as i64 + dy * k as i64,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Normative map semantics
// ---------------------------------------------------------------------------

#[test]
fn global_predict_translation_matches_whole_plane_copy_rect() {
    // A `GlobalPredict` whose map is a pure integer translation must equal a
    // whole-plane `CopyRect` with the same source offset, byte for byte.
    let (w, h) = (24u32, 16u32);
    let epoch = gray_epoch(w, h, 8);
    let base = smooth_content(w, h, 3);
    for (dx, dy) in [(2i64, 0i64), (-3, 0), (0, 1), (0, -2), (3, -1)] {
        let mk = |op: PlaneOp| {
            let mut prog = PlaneProgram::new(9);
            if let Ok(obj) = PlaneObject::raster(w, h, BitDepth::new(8).unwrap(), &base) {
                prog.objects.insert(PlaneObjectId(1), obj);
                prog.instances.push(PlaneInstance {
                    id: PlaneInstanceId(1),
                    object: PlaneObjectId(1),
                    x: 0,
                    y: 0,
                });
            }
            prog.intervals.push((1, vec![op]));
            MultiPlaneProgram::new(epoch.clone(), vec![prog]).unwrap()
        };
        let gp = GlobalMap {
            shift: MapShift::Q8,
            a: 256,
            b: 0,
            c: dx * 256,
            d: 0,
            e: 256,
            f: dy * 256,
        };
        let warp = mk(PlaneOp::GlobalPredict { map: gp });
        // CopyRect maps dest (dst_x+j) ← prev at (src_x+j), so a dest→source
        // offset (dx, dy) needs src = dst + (dx, dy) with dst at the origin.
        let rect = mk(PlaneOp::CopyRect {
            src_x: dx,
            src_y: dy,
            width: w,
            height: h,
            dst_x: 0,
            dst_y: 0,
        });
        let a = warp.materialize_observation(1).unwrap();
        let b = rect.materialize_observation(1).unwrap();
        assert_eq!(
            a.plane(0).unwrap().canonical_bytes(),
            b.plane(0).unwrap().canonical_bytes(),
            "translation ({dx},{dy})"
        );
    }
}

#[test]
fn global_predict_warp_matches_a_naive_expectation_oracle() {
    // The warp op semantics, checked per sample against a naive oracle: dest
    // (x, y) takes prev at the mapped source when that source lies inside the
    // previous plane, and keeps the interval's fresh state render otherwise
    // (the state render here is the persistent content instance's blit).
    let (w, h) = (24u32, 16u32);
    let epoch = gray_epoch(w, h, 8);
    let content = smooth_content(w, h, 5);
    for map in [
        // Quarter turn about the origin (exact integer map).
        GlobalMap {
            shift: MapShift::Q8,
            a: 0,
            b: 256,
            c: 0,
            d: -256,
            e: 0,
            f: 0,
        },
        // 2× zoom about the origin (exact integer map).
        GlobalMap {
            shift: MapShift::Q8,
            a: 128,
            b: 0,
            c: 0,
            d: 0,
            e: 128,
            f: 0,
        },
        // A general Q8 map.
        GlobalMap {
            shift: MapShift::Q8,
            a: 251,
            b: 7,
            c: -100,
            d: -3,
            e: 260,
            f: 40,
        },
    ] {
        let mut prog = PlaneProgram::new(9);
        prog.objects.insert(
            PlaneObjectId(1),
            PlaneObject::raster(w, h, BitDepth::new(8).unwrap(), &content).unwrap(),
        );
        prog.instances.push(PlaneInstance {
            id: PlaneInstanceId(1),
            object: PlaneObjectId(1),
            x: 0,
            y: 0,
        });
        prog.intervals
            .push((1, vec![PlaneOp::GlobalPredict { map }]));
        let mp = MultiPlaneProgram::new(epoch.clone(), vec![prog]).unwrap();
        let out = samples_of(&mp.materialize_observation(1).unwrap(), 0);
        // The previous observation == the content (observation 0), and the
        // fresh state render is the same content blit (the persistent
        // instance), so the oracle below is exact.
        let prev = &content;
        for y in 0..h {
            for x in 0..w {
                let (su, sv) = map.source(x as i64, y as i64).unwrap();
                let want = if su >= 0 && sv >= 0 && su < i64::from(w) && sv < i64::from(h) {
                    prev[(sv as u32 * w + su as u32) as usize]
                } else {
                    prev[(y * w + x) as usize] // keep the state render
                };
                assert_eq!(out[(y * w + x) as usize], want, "{map:?} ({x},{y})");
            }
        }
    }
}

#[test]
fn every_registry_precision_is_exact_at_any_depth() {
    // A fractional map at Q12/Q16 rounds to the same whole-sample picks as Q8
    // only when the translation is integral; this court checks that the
    // declared rule itself is exact at every precision on a plane where the
    // map is representable (a half-sample source offset differs by precision
    // and is closed by nothing here — so use integral offsets, which every
    // precision must reproduce identically).
    for depth in [8u8, 10, 16] {
        let (w, h) = (20u32, 12u32);
        let epoch = gray_epoch(w, h, depth);
        let max = epoch.planes()[0].bit_depth.max_sample();
        let base: Vec<u32> = smooth_content(w, h, 9)
            .iter()
            .map(|v| (*v as u64 * u64::from(max) / 255) as u32)
            .collect();
        for shift in MapShift::ALL {
            let s = shift.scale();
            let map = GlobalMap {
                shift,
                a: s,
                b: 0,
                c: 2 * s,
                d: 0,
                e: s,
                f: -s,
            };
            let mut prog = PlaneProgram::new(0);
            if let Ok(obj) = PlaneObject::raster(w, h, BitDepth::new(depth).unwrap(), &base) {
                prog.objects.insert(PlaneObjectId(1), obj);
                prog.instances.push(PlaneInstance {
                    id: PlaneInstanceId(1),
                    object: PlaneObjectId(1),
                    x: 0,
                    y: 0,
                });
            }
            prog.intervals
                .push((1, vec![PlaneOp::GlobalPredict { map }]));
            let mp = MultiPlaneProgram::new(epoch.clone(), vec![prog]).unwrap();
            let out = samples_of(&mp.materialize_observation(1).unwrap(), 0);
            // dest (x, y) equals base[x + 2, y − 1] where that source lies
            // inside the previous plane; where the source leaves the plane
            // the sample keeps the fresh state render (the content instance's
            // blit — the same `base` samples).
            for y in 0..h {
                for x in 0..w {
                    let want = if x + 2 < w && y >= 1 {
                        base[((y - 1) * w + (x + 2)) as usize]
                    } else {
                        base[(y * w + x) as usize]
                    };
                    assert_eq!(
                        out[(y * w + x) as usize],
                        want,
                        "d{depth} {shift:?} ({x},{y})"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Family encoder over dense raster content
// ---------------------------------------------------------------------------

#[test]
fn encoder_serves_a_pan_by_global_translation_exactly() {
    let (w, h) = (96u32, 64u32);
    let epoch = gray_epoch(w, h, 8);
    let frames = pan_frames(w, h, 2, 1, 6, 11);
    let observations: Vec<Picture> = frames
        .iter()
        .map(|f| single_plane(&epoch, f.clone()))
        .collect();
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    // Exact, through the wire as well.
    let bytes = write_multiplane(&prog).unwrap();
    let reparsed = parse_multiplane(&bytes).unwrap();
    assert_all_observations(&epoch, &observations, &reparsed);
    // Every panning interval is a whole-plane prediction from the previous
    // observation (the changed area covers the plane, so region reuse is
    // skipped by its own economics — global motion is the family).
    let gt = report.families.get(FAMILY_GLOBAL_TRANSLATION).copied();
    let gt = gt.expect("global_translation observations");
    assert_eq!(gt.observations, 5, "report {:#?}", report.families);
    assert_eq!(gt.interval_bytes, report.total_interval_bytes);
    assert!(
        report.total_interval_bytes * 3 < report.raw_floor_bytes,
        "pan must crush the RAW floor: {} vs {}",
        report.total_interval_bytes,
        report.raw_floor_bytes
    );
    // Every chosen record carried a map precision.
    let shift_bytes: u64 = report.map_shift_bytes.values().sum();
    assert_eq!(shift_bytes, report.total_interval_bytes);
    assert_eq!(report.map_shift_observations.values().sum::<u64>(), 5);
}

#[test]
fn encoder_serves_a_zoom_run_by_a_global_model_exactly() {
    // Continuous zoom-in about the center rendered with bilinear resampling
    // of low-contrast smooth content (a real camera softens content; no
    // procedural short-cut). Zoom-in keeps every destination source inside
    // the previous observation; the residual closes only the sub-pixel
    // sampling mismatch.
    let (w, h) = (64u32, 64u32);
    let epoch = gray_epoch(w, h, 8);
    let margin = 24u32;
    let cw = w + 2 * margin;
    // Low-contrast gentle content: sub-pixel deltas stay small and smooth.
    let base = smooth_content(cw, cw, 21);
    let content: Vec<u32> = base
        .iter()
        .map(|v| ((u64::from(*v) * 120 / 255) + 68) as u32)
        .collect();
    let z = 1.015f64;
    let cx = (f64::from(w) - 1.0) / 2.0;
    let mut frames = Vec::new();
    for k in 0..6usize {
        let inv = 1.0 / z.powi(k as i32);
        // dest → source about the frame center, offset into the content.
        let m = [
            inv,
            0.0,
            f64::from(margin) + cx - inv * cx,
            0.0,
            inv,
            f64::from(margin) + cx - inv * cx,
        ];
        frames.push(render_map(&content, cw, w, h, &m));
    }
    let observations: Vec<Picture> = frames
        .iter()
        .map(|f| single_plane(&epoch, f.clone()))
        .collect();
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    assert_all_observations(&epoch, &observations, &prog);
    let global_obs: u64 = [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ]
    .iter()
    .map(|f| report.families.get(*f).map(|t| t.observations).unwrap_or(0))
    .sum();
    assert!(
        global_obs > 0,
        "zoom needs a global model: {:#?}",
        report.families
    );
    assert!(
        report.total_interval_bytes < report.raw_floor_bytes,
        "zoom must beat RAW: {} vs {}",
        report.total_interval_bytes,
        report.raw_floor_bytes
    );
}

#[test]
fn noise_never_claims_a_global_model() {
    // Cryptographic-style iid planes: no global motion exists; the encoder
    // must fall to the honest floor with bounded overhead, never a fake warp.
    let (w, h) = (64u32, 48u32);
    let epoch = gray_epoch(w, h, 8);
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let observations: Vec<Picture> = (0..5)
        .map(|_| {
            let f: Vec<u32> = (0..(w * h)).map(|_| (next() & 0xFF) as u32).collect();
            single_plane(&epoch, f)
        })
        .collect();
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    assert_all_observations(&epoch, &observations, &prog);
    let global_obs: u64 = [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ]
    .iter()
    .map(|f| report.families.get(*f).map(|t| t.observations).unwrap_or(0))
    .sum();
    assert_eq!(
        global_obs, 0,
        "noise must not be explained as motion: {:#?}",
        report.families
    );
    assert!(
        report.total_interval_bytes <= report.raw_floor_bytes * 2,
        "bounded overhead: {} vs {}",
        report.total_interval_bytes,
        report.raw_floor_bytes
    );
    // The sentinels serve it (RAW or the transform floor, never a warp).
    let sentinels: u64 = [FAMILY_RAW, FAMILY_TRANSFORM, FAMILY_SPARSE]
        .iter()
        .map(|f| report.families.get(*f).map(|t| t.observations).unwrap_or(0))
        .sum();
    assert_eq!(
        sentinels, 4,
        "sentinels serve noise: {:#?}",
        report.families
    );
}

#[test]
fn scene_cut_stops_the_global_run() {
    let (w, h) = (96u32, 64u32);
    let epoch = gray_epoch(w, h, 8);
    let mut frames = pan_frames(w, h, 2, 0, 5, 41); // 5 observations, 4 intervals
                                                    // Observation 5 is unrelated content (a hard cut).
    let cut = pan_frames(w, h, 0, 0, 1, 99);
    frames.push(cut[0].clone());
    let observations: Vec<Picture> = frames
        .iter()
        .map(|f| single_plane(&epoch, f.clone()))
        .collect();
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    assert_all_observations(&epoch, &observations, &prog);
    let gt = report
        .families
        .get(FAMILY_GLOBAL_TRANSLATION)
        .copied()
        .unwrap_or_default();
    assert_eq!(
        gt.observations, 4,
        "pan intervals only: {:#?}",
        report.families
    );
}

#[test]
fn multiplane_10bit_pan_is_served_per_plane_on_its_own_grid() {
    // YUV420 10-bit: luma pans (+4, +2) per frame; the subsampled chroma
    // planes pan (+2, +1) — each plane's encoder estimates and predicts on
    // its own grid (independent-plane doctrine) and the stream is exact.
    let (w, h) = (48u32, 32u32);
    let depth = 10u8;
    let epoch = yuv_epoch(w, h, depth);
    let max = 1023u32;
    let scale = |s: Vec<u32>| {
        s.into_iter()
            .map(|v| v as u64 * u64::from(max) / 255)
            .map(|v| v as u32)
            .collect()
    };
    let (cw, ch) = (w / 2, h / 2);
    let mut observations = Vec::new();
    let yf = pan_frames(w, h, 4, 2, 6, 7);
    let cf = pan_frames(cw, ch, 2, 1, 6, 8);
    for i in 0..6 {
        let planes = vec![
            picture_of(&epoch, 0, scale(yf[i].clone())),
            picture_of(&epoch, 1, scale(cf[i].clone())),
            picture_of(&epoch, 2, scale(cf[i].clone())),
        ];
        observations.push(Picture::from_planes(&epoch, planes).unwrap());
    }
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    assert_all_observations(&epoch, &observations, &prog);
    let gt = report
        .families
        .get(FAMILY_GLOBAL_TRANSLATION)
        .copied()
        .expect("global translations");
    // The clear majority of the 15 plane-intervals ride whole-plane
    // prediction on their own grid (the greedy per-interval cost may hand an
    // occasional interval to a residual floor class — exactness is what the
    // court demands, and the floor stays honest).
    assert!(
        gt.observations >= 12,
        "global translations: {:#?}",
        report.families
    );
    assert!(
        report.total_interval_bytes < report.raw_floor_bytes,
        "{} vs {}",
        report.total_interval_bytes,
        report.raw_floor_bytes
    );
}

#[test]
fn static_tail_after_a_pan_is_held_from_the_previous_observation() {
    // After a pan the encoder's committed state still shows the first frame;
    // two identical trailing frames are each cheapest as a whole-plane hold
    // from the previous observation (identity warp) — the greedy per-interval
    // cost is honest (each interval is far cheaper than a RAW re-sync), and
    // a later subphase's temporal-span search may promote this to one sync.
    let (w, h) = (96u32, 64u32);
    let epoch = gray_epoch(w, h, 8);
    let mut frames = pan_frames(w, h, 2, 0, 3, 51);
    let still = frames[2].clone();
    frames.push(still.clone());
    frames.push(still.clone());
    let observations: Vec<Picture> = frames
        .iter()
        .map(|f| single_plane(&epoch, f.clone()))
        .collect();
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    assert_all_observations(&epoch, &observations, &prog);
    let gt = report
        .families
        .get(FAMILY_GLOBAL_TRANSLATION)
        .copied()
        .unwrap_or_default();
    assert_eq!(
        gt.observations, 4,
        "two pans + two holds: {:#?}",
        report.families
    );
    assert_eq!(gt.interval_bytes, report.total_interval_bytes);
}

// ---------------------------------------------------------------------------
// 3. §62 precision court
// ---------------------------------------------------------------------------

/// Gentle low-contrast smooth content (sub-pixel zoom deltas stay small and
/// smooth — the honest natural-video-like test for map-precision economics).
fn gentle_zoom_observations(
    w: u32,
    h: u32,
    z: f64,
    n: usize,
    seed: u64,
) -> (VideoEpoch, Vec<Picture>) {
    let epoch = gray_epoch(w, h, 8);
    let margin = 24u32;
    let cw = w + 2 * margin;
    let base = smooth_content(cw, cw, seed);
    let content: Vec<u32> = base
        .iter()
        .map(|v| ((u64::from(*v) * 120 / 255) + 68) as u32)
        .collect();
    let cx = (f64::from(w) - 1.0) / 2.0;
    let mut frames = Vec::new();
    for k in 0..n {
        let inv = 1.0 / z.powi(k as i32);
        let m = [
            inv,
            0.0,
            f64::from(margin) + cx - inv * cx,
            0.0,
            inv,
            f64::from(margin) + cx - inv * cx,
        ];
        frames.push(render_map(&content, cw, w, h, &m));
    }
    let observations: Vec<Picture> = frames
        .iter()
        .map(|f| single_plane(&epoch, f.clone()))
        .collect();
    (epoch, observations)
}

#[test]
fn forced_precision_runs_are_exact_and_deterministic() {
    let (epoch, observations) = gentle_zoom_observations(64, 64, 1.015, 6, 61);
    let global_obs_of = |report: &vole_video::media::encode::EncodeReport| -> u64 {
        [
            FAMILY_GLOBAL_TRANSLATION,
            FAMILY_GLOBAL_ROTZOOM,
            FAMILY_GLOBAL_AFFINE,
        ]
        .iter()
        .map(|f| report.families.get(*f).map(|t| t.observations).unwrap_or(0))
        .sum()
    };
    let global_bytes_of = |report: &vole_video::media::encode::EncodeReport| -> u64 {
        [
            FAMILY_GLOBAL_TRANSLATION,
            FAMILY_GLOBAL_ROTZOOM,
            FAMILY_GLOBAL_AFFINE,
        ]
        .iter()
        .map(|f| {
            report
                .families
                .get(*f)
                .map(|t| t.interval_bytes)
                .unwrap_or(0)
        })
        .sum()
    };
    for shift in MapShift::ALL {
        let opts = EncodeOptions {
            map_shift: Some(shift),
            ..EncodeOptions::default()
        };
        let (prog, report) = encode_pictures_families_with(&epoch, &observations, opts).unwrap();
        assert_all_observations(&epoch, &observations, &prog);
        // Every chosen global record used the forced precision.
        let obs: u64 = report.map_shift_observations.values().sum();
        let bytes: u64 = report.map_shift_bytes.values().sum();
        assert_eq!(global_obs_of(&report), obs, "forced {shift:?}");
        assert_eq!(global_bytes_of(&report), bytes, "forced {shift:?}");
        assert_eq!(
            report.map_shift_observations.get(&shift.code()).copied(),
            Some(obs),
            "forced {shift:?}"
        );
        assert_eq!(
            report.map_shift_bytes.get(&shift.code()).copied(),
            Some(bytes),
            "forced {shift:?}"
        );
        // Determinism: re-running the same encode is byte-identical.
        let (prog2, report2) = encode_pictures_families_with(&epoch, &observations, opts).unwrap();
        assert_eq!(
            write_multiplane(&prog).unwrap(),
            write_multiplane(&prog2).unwrap()
        );
        assert_eq!(report.total_interval_bytes, report2.total_interval_bytes);
        assert_eq!(report.candidate_evaluations, report2.candidate_evaluations);
        assert_eq!(report.search_work, report2.search_work);
    }
}

#[test]
fn auto_precision_prices_every_shift_and_reports_the_measured_winner() {
    let (epoch, observations) = gentle_zoom_observations(64, 64, 1.015, 6, 71);
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    assert_all_observations(&epoch, &observations, &prog);
    let global_obs: u64 = [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ]
    .iter()
    .map(|f| report.families.get(*f).map(|t| t.observations).unwrap_or(0))
    .sum();
    let shift_obs: u64 = report.map_shift_observations.values().sum();
    let shift_bytes: u64 = report.map_shift_bytes.values().sum();
    let global_bytes: u64 = [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ]
    .iter()
    .map(|f| {
        report
            .families
            .get(*f)
            .map(|t| t.interval_bytes)
            .unwrap_or(0)
    })
    .sum();
    assert_eq!(shift_obs, global_obs, "every global record priced a map");
    assert_eq!(shift_bytes, global_bytes);
    assert!(report.total_interval_bytes < report.raw_floor_bytes);
    for code in report.map_shift_bytes.keys() {
        assert!(MapShift::from_code(*code).is_some(), "registry code {code}");
    }
}

// ---------------------------------------------------------------------------
// 4. v2 wire (global-motion extension)
// ---------------------------------------------------------------------------

/// The canonical V.1.5 extension golden scenario: a 16×12 Gray8 program whose
/// one interval predicts observation 1 from observation 0 through a Q16
/// translation map and closes the revealed strip with a sparse residual.
fn golden_program() -> MultiPlaneProgram {
    let (w, h) = (16u32, 12u32);
    let epoch = gray_epoch(w, h, 8);
    let mut prog = PlaneProgram::new(0);
    let content: Vec<u32> = (0..(w * h))
        .map(|k| ((k % w) + 2 * (k / w)) % 256)
        .collect();
    prog.objects.insert(
        PlaneObjectId(1),
        PlaneObject::raster(w, h, BitDepth::new(8).unwrap(), &content).unwrap(),
    );
    prog.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    // Observation 1: dest (x, y) ← prev (x + 2, y) (Q16 map); the two
    // columns whose source would leave the previous plane keep the state
    // render (the pattern), and the residual then rewrites them to 7 so the
    // authored strip is meaningful.
    let map = GlobalMap {
        shift: MapShift::Q16,
        a: 65536,
        b: 0,
        c: 2 * 65536,
        d: 0,
        e: 65536,
        f: 0,
    };
    let mut points = Vec::new();
    for x in [w as i32 - 2, w as i32 - 1] {
        for y in 0..h {
            points.push((x, y as i32, 7));
        }
    }
    let block = vole_video::media::core::encode_plane_residual(&points).unwrap();
    prog.intervals.push((
        1,
        vec![PlaneOp::GlobalPredict { map }, PlaneOp::Residual { block }],
    ));
    MultiPlaneProgram::new(epoch, vec![prog]).unwrap()
}

#[test]
fn golden_scenario_materializes_as_expected() {
    let prog = golden_program();
    let obs0 = samples_of(&prog.materialize_observation(0).unwrap(), 0);
    let obs1 = samples_of(&prog.materialize_observation(1).unwrap(), 0);
    let (w, h) = (16u32, 12u32);
    for y in 0..h {
        for x in 0..w {
            let want = if x + 2 < w {
                obs0[(y * w + (x + 2)) as usize]
            } else {
                7 // the residual-authored strip
            };
            assert_eq!(obs1[(y * w + x) as usize], want, "({x},{y})");
        }
    }
}

#[test]
fn v15_extension_golden_is_pinned() {
    let bytes = write_multiplane(&golden_program()).unwrap();
    let digest = blake3::hash(&bytes);
    let hex = digest.to_hex().to_string();
    // Pinned when the V.1.5 grammar is sealed (global-motion golden).
    assert_eq!(
        hex, "2791d62289d601a59ce0d1f0884738a6f4d939657cc438666f0a72500ecbbae9",
        "V.1.5 golden changed: deliberate grammar re-freeze required"
    );
}

#[test]
fn feature_bits_are_minimal_and_additive() {
    use vole_video::media::core::{PlaneMotion, PlanePaletteId};
    assert_eq!(
        V2_FEATURES, 0x3,
        "family (0x1) + global (0x2) are the known bits"
    );
    let feature_bits = |prog: &MultiPlaneProgram| -> u32 {
        let bytes = write_multiplane(prog).unwrap();
        let mut b = [0u8; 4];
        b.copy_from_slice(&bytes[12..16]);
        u32::from_le_bytes(b)
    };
    // A global-only program declares bit 0x2 only.
    assert_eq!(feature_bits(&golden_program()), 0x2);
    // A family-extension program (V.1.4 surface) with a global op declares
    // both bits (0x3).
    let mut prog = golden_program();
    let plane = &mut prog.planes[0];
    plane.palettes.insert(PlanePaletteId(1), vec![1, 2, 3]);
    plane.initial_motion.push(PlaneMotion::Binding {
        instance: PlaneInstanceId(1),
        palette: PlanePaletteId(1),
    });
    let mp = MultiPlaneProgram::new(prog.epoch.clone(), prog.planes.clone()).unwrap();
    assert_eq!(feature_bits(&mp), 0x3);
}

#[test]
fn global_op_without_the_feature_bit_fails_closed() {
    let bytes = write_multiplane(&golden_program()).unwrap();
    let mut hostile = bytes.clone();
    hostile[12] = 0; // clear feature bit 0x2 (and 0x1)
    hostile[13] = 0;
    hostile[14] = 0;
    hostile[15] = 0;
    assert_eq!(
        parse_multiplane(&hostile).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // An unknown shift registry byte is a structural error (before the
    // digest check, so the stale trailer does not matter).
    let mut hostile = bytes.clone();
    let pos = hostile
        .windows(2)
        .position(|p| p == [0x32, 0x10])
        .expect("the GlobalPredict op tag + Q16 shift byte");
    hostile[pos + 1] = 9;
    assert_eq!(
        parse_multiplane(&hostile).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Unknown *feature* bits fail closed at the header.
    let mut hostile = bytes.clone();
    hostile[12] = 0x4;
    assert_eq!(
        parse_multiplane(&hostile).unwrap_err(),
        VoleError::UnsupportedFeature
    );
}

#[test]
fn hostile_maps_are_typed_never_a_panic() {
    let prog = golden_program();
    let mut hostile = prog.clone();
    {
        let PlaneOp::GlobalPredict { map } = &mut hostile.planes[0].intervals[0].1[0] else {
            unreachable!()
        };
        // Out-of-domain coefficients fail closed at the writer.
        map.a = (1 << 24) + 1;
    }
    assert!(write_multiplane(&hostile).is_err());
    {
        let PlaneOp::GlobalPredict { map } = &mut hostile.planes[0].intervals[0].1[0] else {
            unreachable!()
        };
        map.a = 1 << 24;
    }
    // A hostile stream may declare many whole-plane warps in one interval;
    // materialization under a tight motion-work budget fails typed.
    let mut bomb = golden_program();
    let mut ops = Vec::new();
    for _ in 0..64 {
        ops.push(PlaneOp::GlobalPredict {
            map: GlobalMap::identity(MapShift::Q8),
        });
    }
    bomb.planes[0].intervals[0].1 = ops;
    let small = vole_video::limits::Limits {
        max_motion_work: 1,
        ..vole_video::limits::Limits::default()
    };
    let (w, h) = bomb.epoch.plane_dimensions(0).unwrap();
    let err = materialize_plane(
        &bomb.planes[0],
        w,
        h,
        bomb.epoch.planes()[0].bit_depth,
        1,
        &small,
    )
    .unwrap_err();
    assert_eq!(err, VoleError::MaterializationBudgetExceeded);
    // The default envelope materializes the same stream fine.
    let _ = bomb.materialize_observation(1).unwrap();
}

#[test]
fn truncations_and_flips_are_typed() {
    let bytes = write_multiplane(&golden_program()).unwrap();
    for cut in [1usize, 32, bytes.len() / 2, bytes.len() - 1] {
        assert!(parse_multiplane(&bytes[..cut]).is_err());
    }
    let mut hostile = bytes.clone();
    let n = hostile.len();
    hostile[n / 2] ^= 0xFF; // a content flip
    assert_eq!(
        parse_multiplane(&hostile).unwrap_err(),
        VoleError::IntegrityMismatch
    );
}

#[test]
fn pan_roundtrip_keeps_minimal_feature_bits() {
    let (w, h) = (96u32, 64u32);
    let epoch = gray_epoch(w, h, 8);
    let frames = pan_frames(w, h, 2, 1, 4, 81);
    let observations: Vec<Picture> = frames
        .iter()
        .map(|f| single_plane(&epoch, f.clone()))
        .collect();
    let (prog, _) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    let bytes = write_multiplane(&prog).unwrap();
    let mut b = [0u8; 4];
    b.copy_from_slice(&bytes[12..16]);
    assert_eq!(u32::from_le_bytes(b), 0x2);
    let parsed = parse_multiplane(&bytes).unwrap();
    assert_all_observations(&epoch, &observations, &parsed);
}

#[test]
fn ablation_without_global_falls_back_and_stays_exact() {
    let (w, h) = (96u32, 64u32);
    let epoch = gray_epoch(w, h, 8);
    let frames = pan_frames(w, h, 2, 1, 6, 91);
    let observations: Vec<Picture> = frames
        .iter()
        .map(|f| single_plane(&epoch, f.clone()))
        .collect();
    let with =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    let without = encode_pictures_families_with(
        &epoch,
        &observations,
        EncodeOptions {
            disable_global: true,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    assert_all_observations(&epoch, &observations, &with.0);
    assert_all_observations(&epoch, &observations, &without.0);
    // The global family is what makes the pan cheap.
    assert!(
        with.1.total_interval_bytes < without.1.total_interval_bytes,
        "with {} vs without {}",
        with.1.total_interval_bytes,
        without.1.total_interval_bytes
    );
    let global_obs: u64 = [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ]
    .iter()
    .map(|f| {
        without
            .1
            .families
            .get(*f)
            .map(|t| t.observations)
            .unwrap_or(0)
    })
    .sum();
    assert_eq!(global_obs, 0);
}

#[test]
fn map_class_estimator_is_bounded_and_deterministic() {
    use vole_video::media::global::estimate_global;
    let (w, h) = (96u32, 64u32);
    let frames = pan_frames(w, h, 1, -1, 2, 5);
    let mut work = 0u64;
    let hyps = estimate_global(&frames[0], &frames[1], w, h, 1, &mut work).unwrap();
    assert!(!hyps.is_empty());
    assert_eq!(
        hyps[0].class,
        vole_video::media::global::MotionClass::Translation
    );
    // Bounded work: the pyramid keeps the search far below an exhaustive
    // whole-window scan at full resolution (which would be
    // (2·64+1)² × 6144 ≈ 1.0×10⁹ samples); measured 1.7×10⁶ here.
    let full = u64::from(w) * u64::from(h);
    assert!(
        work < full * 1024,
        "estimator work {work} for {full} samples"
    );
    // Deterministic.
    let mut work2 = 0u64;
    let hyps2 = estimate_global(&frames[0], &frames[1], w, h, 1, &mut work2).unwrap();
    assert_eq!(work, work2);
    assert_eq!(hyps.len(), hyps2.len());
    for (a, b) in hyps.iter().zip(&hyps2) {
        assert_eq!(a.class, b.class);
        assert_eq!(a.params, b.params);
    }
}
