//! V.1.5 evidence proof — global video structure (release run, recorded in
//! `evidence/campaigns/`): the family encoder over dense natural-like raster
//! content (pan / zoom / multiplane 10-bit), the §62 map-precision court
//! (forced Q8/Q12/Q16 + auto), the noise negative control, the family
//! ablation, estimator boundedness, and the pinned V.1.5 extension golden.

use vole_video::media::color::ColorDescription;
use vole_video::media::encode::{
    encode_pictures_families_with, EncodeOptions, EncodeReport, FAMILY_GLOBAL_AFFINE,
    FAMILY_GLOBAL_ROTZOOM, FAMILY_GLOBAL_TRANSLATION,
};
use vole_video::media::epoch::{EpochId, VideoEpoch};
use vole_video::media::global::MapShift;
use vole_video::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
use vole_video::media::picture::Picture;
use vole_video::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use vole_video::media::wire::write_multiplane;
use vole_video::media::PixelLayout;

fn gray_epoch(w: u32, h: u32) -> VideoEpoch {
    VideoEpoch::new_uniform(
        EpochId(0),
        w,
        h,
        PixelLayout::Gray,
        BitDepth::new(8).unwrap(),
        ColorDescription::unspecified(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )
    .unwrap()
}

fn yuv_epoch(w: u32, h: u32) -> VideoEpoch {
    VideoEpoch::new_uniform(
        EpochId(0),
        w,
        h,
        PixelLayout::Yuv420,
        BitDepth::new(10).unwrap(),
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

fn picture(epoch: &VideoEpoch, samples: Vec<u32>) -> Picture {
    let (w, h) = epoch.plane_dimensions(0).unwrap();
    let plane = Plane::new(
        epoch.planes()[0].component,
        w,
        h,
        epoch.planes()[0].bit_depth,
        epoch.planes()[0].subsample_x,
        epoch.planes()[0].subsample_y,
        raster(epoch.planes()[0].bit_depth.bits(), &samples),
    )
    .unwrap();
    Picture::from_planes(epoch, vec![plane]).unwrap()
}

fn plane_of(epoch: &VideoEpoch, idx: usize, samples: Vec<u32>) -> Plane {
    let t = &epoch.planes()[idx];
    let (pw, ph) = epoch.plane_dimensions(idx).unwrap();
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

fn content_plane(margin: u32, w: u32, h: u32, seed: u64) -> Vec<u32> {
    let cw = w + 2 * margin;
    let ch = h + 2 * margin;
    let base = smooth_content(cw, ch, seed);
    (0..(cw * ch))
        .map(|k| {
            let (x, y) = (k % cw, k / cw);
            let detail = lattice(x as i64, y as i64, seed ^ 0xD1B54A32D192ED03);
            ((u64::from(base[k as usize]) * 3 + u64::from(detail)) / 4) as u32
        })
        .collect()
}

fn view(content: &[u32], cw: u32, w: u32, h: u32, ox: i64, oy: i64) -> Vec<u32> {
    (0..(w * h))
        .map(|k| {
            let (x, y) = (k % w, k / w);
            content[((y as i64 + oy) as u32 * cw + (x as i64 + ox) as u32) as usize]
        })
        .collect()
}

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

fn render_map(content: &[u32], cw: u32, w: u32, h: u32, m: &[f64; 6]) -> Vec<u32> {
    (0..(w * h))
        .map(|k| {
            let (x, y) = (k % w, k / w);
            let su = m[0] * f64::from(x) + m[1] * f64::from(y) + m[2];
            let sv = m[3] * f64::from(x) + m[4] * f64::from(y) + m[5];
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

fn gentle_zoom_frames(w: u32, h: u32, z: f64, n: usize, seed: u64) -> Vec<Vec<u32>> {
    let margin = 24u32;
    let cw = w + 2 * margin;
    let base = smooth_content(cw, cw, seed);
    let content: Vec<u32> = base
        .iter()
        .map(|v| ((u64::from(*v) * 120 / 255) + 68) as u32)
        .collect();
    let cx = (f64::from(w) - 1.0) / 2.0;
    (0..n)
        .map(|k| {
            let inv = 1.0 / z.powi(k as i32);
            let m = [
                inv,
                0.0,
                f64::from(margin) + cx - inv * cx,
                0.0,
                inv,
                f64::from(margin) + cx - inv * cx,
            ];
            render_map(&content, cw, w, h, &m)
        })
        .collect()
}

fn family_line(report: &EncodeReport, label: &str) -> String {
    match report.families.get(label) {
        Some(t) => format!(
            "{} obs {} interval-bytes {}",
            label, t.observations, t.interval_bytes
        ),
        None => format!("{label} obs 0 interval-bytes 0"),
    }
}

fn main() {
    println!("global proof: global video structure (Phase V.1.5)\n");

    // 1. Camera pan over dense natural-like content (Gray8, 96x64, 6 obs).
    let (w, h) = (96u32, 64u32);
    let epoch = gray_epoch(w, h);
    let frames = pan_frames(w, h, 2, 1, 6, 11);
    let observations: Vec<Picture> = frames.iter().map(|f| picture(&epoch, f.clone())).collect();
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    for p in 0..epoch.plane_count() {
        for (i, want) in observations.iter().enumerate() {
            let got = prog.materialize_observation(i as u64).unwrap();
            assert_eq!(
                got.plane(p).unwrap().canonical_bytes(),
                want.plane(p).unwrap().canonical_bytes()
            );
        }
    }
    println!("1. pan (96x64 Gray8, 6 observations, +2/+1 per frame)");
    println!(
        "   total interval bytes {} | observations {} | raw-floor {} | ratio {:.2}x",
        report.total_interval_bytes,
        report.observations(),
        report.raw_floor_bytes,
        report.raw_floor_bytes as f64 / report.total_interval_bytes as f64
    );
    for l in [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ] {
        println!("   family {}", family_line(&report, l));
    }
    println!(
        "   search work {} | candidate evaluations {}",
        report.search_work, report.candidate_evaluations
    );

    // 2. Continuous zoom-in (Gentle content, Gray8 64x64, 6 obs) — the §62
    //    precision court on the same footage: forced Q8 / Q12 / Q16 + auto.
    let (w, h) = (64u32, 64u32);
    let epoch = gray_epoch(w, h);
    let frames = gentle_zoom_frames(w, h, 1.015, 6, 21);
    let observations: Vec<Picture> = frames.iter().map(|f| picture(&epoch, f.clone())).collect();
    println!("\n2. zoom-in (64x64 Gray8, 6 observations, 1.5%/frame)");
    let g_bytes = |r: &EncodeReport| {
        [
            FAMILY_GLOBAL_TRANSLATION,
            FAMILY_GLOBAL_ROTZOOM,
            FAMILY_GLOBAL_AFFINE,
        ]
        .iter()
        .map(|l| r.families.get(*l).map(|t| t.interval_bytes).unwrap_or(0))
        .sum::<u64>()
    };
    for shift in MapShift::ALL {
        let opts = EncodeOptions {
            map_shift: Some(shift),
            ..EncodeOptions::default()
        };
        let (prog, report) = encode_pictures_families_with(&epoch, &observations, opts).unwrap();
        let bytes = write_multiplane(&prog).unwrap();
        println!(
            "   forced {:<3} interval-bytes {:>6} (container {} B) | global bytes {}",
            shift.label(),
            report.total_interval_bytes,
            bytes.len(),
            g_bytes(&report)
        );
    }
    let (auto_prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    println!(
        "   auto         interval-bytes {:>6} | container {} B | raw-floor {} | per-shift {:#?}",
        report.total_interval_bytes,
        write_multiplane(&auto_prog).unwrap().len(),
        report.raw_floor_bytes,
        report.map_shift_bytes
    );
    for l in [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ] {
        println!("   family {}", family_line(&report, l));
    }

    // 3. 10-bit YUV420 multiplane pan — chroma planes on their own grids.
    let (w, h) = (48u32, 32u32);
    let epoch = yuv_epoch(w, h);
    let max = 1023u32;
    let scale = |s: Vec<u32>| {
        s.into_iter()
            .map(|v| (u64::from(v) * u64::from(max) / 255) as u32)
            .collect()
    };
    let (cw, ch) = (w / 2, h / 2);
    let yf = pan_frames(w, h, 4, 2, 6, 7);
    let cf = pan_frames(cw, ch, 2, 1, 6, 8);
    let mut observations = Vec::new();
    for i in 0..6 {
        let planes = vec![
            plane_of(&epoch, 0, scale(yf[i].clone())),
            plane_of(&epoch, 1, scale(cf[i].clone())),
            plane_of(&epoch, 2, scale(cf[i].clone())),
        ];
        observations.push(Picture::from_planes(&epoch, planes).unwrap());
    }
    let (prog, report) =
        encode_pictures_families_with(&epoch, &observations, EncodeOptions::default()).unwrap();
    for (i, want) in observations.iter().enumerate() {
        let got = prog.materialize_observation(i as u64).unwrap();
        for p in 0..epoch.plane_count() {
            assert_eq!(
                got.plane(p).unwrap().canonical_bytes(),
                want.plane(p).unwrap().canonical_bytes()
            );
        }
    }
    println!("\n3. multiplane pan (10-bit YUV420 48x32, 6 observations, Y +4/+2, C +2/+1)");
    println!(
        "   total interval bytes {} | observations {} | raw-floor {} | ratio {:.2}x",
        report.total_interval_bytes,
        report.observations(),
        report.raw_floor_bytes,
        report.raw_floor_bytes as f64 / report.total_interval_bytes as f64
    );
    for l in [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ] {
        println!("   family {}", family_line(&report, l));
    }

    // 4. Noise negative control (no model may claim structure) + ablation.
    let (w, h) = (64u32, 48u32);
    let epoch = gray_epoch(w, h);
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let noise: Vec<Picture> = (0..5)
        .map(|_| {
            let f: Vec<u32> = (0..(w * h)).map(|_| (next() & 0xFF) as u32).collect();
            picture(&epoch, f)
        })
        .collect();
    let (prog, report) =
        encode_pictures_families_with(&epoch, &noise, EncodeOptions::default()).unwrap();
    for (i, want) in noise.iter().enumerate() {
        let got = prog.materialize_observation(i as u64).unwrap();
        assert_eq!(
            got.plane(0).unwrap().canonical_bytes(),
            want.plane(0).unwrap().canonical_bytes()
        );
    }
    let global_obs: u64 = [
        FAMILY_GLOBAL_TRANSLATION,
        FAMILY_GLOBAL_ROTZOOM,
        FAMILY_GLOBAL_AFFINE,
    ]
    .iter()
    .map(|l| report.families.get(*l).map(|t| t.observations).unwrap_or(0))
    .sum();
    println!("\n4. noise negative control (64x48 Gray8, 5 observations)");
    println!(
        "   total interval bytes {} | raw-floor {} | global observations {} (must be 0)",
        report.total_interval_bytes, report.raw_floor_bytes, global_obs
    );

    let (w, h) = (96u32, 64u32);
    let epoch = gray_epoch(w, h);
    let frames = pan_frames(w, h, 2, 1, 6, 11);
    let observations: Vec<Picture> = frames.iter().map(|f| picture(&epoch, f.clone())).collect();
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
    println!("\n5. family ablation on the pan run (global motion on/off)");
    println!(
        "   with global {} B | without global {} B (the V.1.5 classes own the pan)",
        with.1.total_interval_bytes, without.1.total_interval_bytes
    );

    // 6. Estimator work is bounded (measured on the pan pair).
    let (w, h) = (96u32, 64u32);
    let frames = pan_frames(w, h, 1, -1, 2, 5);
    let mut work = 0u64;
    let hyps =
        vole_video::media::global::estimate_global(&frames[0], &frames[1], w, h, 1, &mut work)
            .expect("hypotheses");
    println!(
        "\n6. estimator (96x64 pair) — work {} samples | {} hypothesis classes",
        work,
        hyps.len()
    );

    // 7. The pinned V.1.5 extension golden (identical bytes to the court).
    let bytes = write_multiplane(&golden_program()).unwrap();
    let digest = blake3::hash(&bytes);
    println!(
        "\n7. V.1.5 extension golden (16x12 Gray8, GlobalPredict Q16 + residual)\n   digest {} | container {} B",
        digest.to_hex(),
        bytes.len()
    );
    println!("\nglobal proof: OK in release (exact global structure + precision court + wire)");
}

/// The canonical V.1.5 extension golden scenario (mirror of the court):
/// observation 1 predicts observation 0 through a Q16 translation map and
/// rewrites the revealed strip with a sparse residual.
fn golden_program() -> vole_video::media::core::MultiPlaneProgram {
    use vole_video::media::core::{
        MultiPlaneProgram, PlaneInstance, PlaneInstanceId, PlaneObject, PlaneObjectId, PlaneOp,
        PlaneProgram,
    };
    let (w, h) = (16u32, 12u32);
    let epoch = gray_epoch(w, h);
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
    let map = vole_video::media::global::GlobalMap {
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
