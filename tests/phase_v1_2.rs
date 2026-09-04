//! Phase V.1.2 courts: multiplane core (V.1 video programme — contract §2.4,
//! §2.6; `docs/format-v2.md`).
//!
//! Courts:
//!
//! 1. **v1 specialization oracle** — one authored scenario expressed twice,
//!    as a sealed v1 Gray8 stream and as a v2 single-plane depth-8 program
//!    (background, overlapping fill + raster instances, SetPosition motion,
//!    overlay patches, COPY_RECT, RAW residual): every materialized frame of
//!    the v2 core is byte-identical to the authoritative v1 decoder output.
//!    The v1 surface is the courted oracle; the v2 core is an independent
//!    implementation of the same semantics, so this is a real equality check.
//! 2. **authored 10-bit 4:2:0 with an independent compositor** — a moving
//!    fill sprite on a chroma-static 10-bit YUV420 picture, verified
//!    sample-for-sample against a naive per-plane compositor written in the
//!    court.
//! 3. **raster-origin floor** — observed 10-bit YUV420 sequences (static,
//!    sprite motion, noise) encode via the exact floor (RAW/unchanged/
//!    residual) and materialize sample-for-sample; static content measurably
//!    stops paying per-observation raster bytes.
//! 4. **v2 wire** — canonical serialization round-trips byte-exactly across
//!    layouts × depths; parse → write is a fixpoint; hostile containers are
//!    typed errors (never panics).
//! 5. **timeline/epoch integration** — programs bind to rational PTS into a
//!    canonical video; a two-epoch video with a mid-stream interpretation
//!    change validates and materializes exactly.

use vole_video::media::color::ColorDescription;
use vole_video::media::core::{
    MultiPlaneProgram, PlaneInstance, PlaneInstanceId, PlaneObject, PlaneObjectId, PlaneOp,
    PlaneProgram,
};
use vole_video::media::epoch::{CanonicalVideo, CanonicalVideoObservation, EpochId, VideoEpoch};
use vole_video::media::ingest::{encode_pictures_exact, ramp_picture, uniform_picture};
use vole_video::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
use vole_video::media::picture::Picture;
use vole_video::media::plane::{BitDepth, PlaneData};
use vole_video::media::time::{Duration, Pts, TimeBase};
use vole_video::media::wire::{parse_multiplane, write_multiplane};
use vole_video::media::PixelLayout;
use vole_video::VoleError;

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

/// One authored scenario used by the v1-specialization oracle.
/// Returns (v1 stream bytes, v2 program).
fn authored_scenario() -> (Vec<u8>, MultiPlaneProgram) {
    let (w, h) = (48u32, 32u32);
    // --- v1 leg (authoritative, sealed Phase-A/G semantics) ---
    let mut a = vole_video::ingest::Ingest::new(w, h);
    a.background(12);
    a.declare_fill(1, 14, 6, 180).unwrap();
    // Raster sprite with a deterministic pattern.
    let pattern: Vec<u8> = (0..36u32).map(|i| (30 + (i * 37) % 200) as u8).collect();
    a.declare_raster(2, 6, 6, pattern.clone()).unwrap();
    a.instance(1, 1, 0, 0).unwrap();
    a.instance(2, 2, 30, 2).unwrap();
    for t in 1..=5u64 {
        a.at(t).unwrap();
        a.set_position(1, 2 * t as i64, 0).unwrap();
        match t {
            3 => {
                a.patch_sparse(vec![(20, 20, 99), (21, 20, 100)]).unwrap();
            }
            4 => {
                a.copy_rect(10, 0, 8, 8, 40, 24).unwrap();
            }
            5 => {
                let mut body = Vec::new();
                for (x, y, v) in [(1i32, 1i32, 7u8), (40, 20, 5)] {
                    body.extend_from_slice(&x.to_le_bytes());
                    body.extend_from_slice(&y.to_le_bytes());
                    body.push(v);
                }
                let mut block = vec![vole_video::rans::KIND_RAW];
                block.extend_from_slice(&(body.len() as u64).to_le_bytes());
                block.extend_from_slice(&body);
                a.residual(block).unwrap();
            }
            _ => {}
        }
    }
    let v1_bytes = a.finish().unwrap();
    let v1_frames = {
        let parsed = vole_video::decoder::decode_bytes(&v1_bytes).unwrap();
        vole_video::decoder::materialize_all(&parsed).unwrap()
    };
    assert_eq!(v1_frames.len(), 6);

    // --- v2 leg (single Gray plane, depth 8, same scenario) ---
    let epoch = gray_epoch(w, h, 8);
    let mut prog = PlaneProgram::new(12);
    prog.objects
        .insert(PlaneObjectId(1), PlaneObject::fill(14, 6, 180));
    prog.objects.insert(
        PlaneObjectId(2),
        PlaneObject::raster(
            6,
            6,
            BitDepth::new(8).unwrap(),
            &pattern.iter().map(|v| u32::from(*v)).collect::<Vec<_>>(),
        )
        .unwrap(),
    );
    prog.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    prog.instances.push(PlaneInstance {
        id: PlaneInstanceId(2),
        object: PlaneObjectId(2),
        x: 30,
        y: 2,
    });
    for t in 1..=5u64 {
        let mut ops = vec![PlaneOp::SetPosition {
            id: PlaneInstanceId(1),
            x: 2 * t as i64,
            y: 0,
        }];
        match t {
            3 => ops.push(PlaneOp::PatchOverlay {
                points: vec![(20, 20, 99), (21, 20, 100)],
            }),
            4 => ops.push(PlaneOp::CopyRect {
                src_x: 10,
                src_y: 0,
                width: 8,
                height: 8,
                dst_x: 40,
                dst_y: 24,
            }),
            5 => ops.push(PlaneOp::Residual {
                block: vole_video::media::core::encode_plane_residual(&[(1, 1, 7), (40, 20, 5)])
                    .unwrap(),
            }),
            _ => {}
        }
        prog.intervals.push((t, ops));
    }
    let program = MultiPlaneProgram::new(epoch, vec![prog]).unwrap();
    // Frame count parity: 6 observations on both sides.
    assert_eq!(program.observation_count(), 6);
    (v1_bytes, program)
}

// ---------------------------------------------------------------------------
// 1. v1 specialization oracle: v2 depth-8 single-plane == v1 decoder output
// ---------------------------------------------------------------------------

#[test]
fn v2_core_is_an_exact_gray8_specialization_of_v1() -> Result<(), VoleError> {
    let (v1_bytes, program) = authored_scenario();
    let v1_frames = {
        let parsed = vole_video::decoder::decode_bytes(&v1_bytes)?;
        vole_video::decoder::materialize_all(&parsed)?
    };
    for (idx, v1) in v1_frames.iter().enumerate() {
        let v2_plane = program.materialize_observation(idx as u64)?;
        let got = v2_plane.plane(0).expect("gray plane").canonical_bytes();
        assert_eq!(
            got,
            v1.as_slice(),
            "v2 core frame {idx} must equal the authoritative v1 decoder"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 2. Authored 10-bit 4:2:0 vs an independent per-plane compositor
// ---------------------------------------------------------------------------

/// Naive per-plane compositor written independently in the court.
fn naive_composite(epoch: &VideoEpoch, frames: usize) -> Vec<Picture> {
    let depth = epoch.planes()[0].bit_depth;
    let max = depth.max_sample();
    let mut out = Vec::new();
    for f in 0..frames {
        let mut planes = Vec::new();
        for p in 0..epoch.plane_count() {
            let (pw, ph) = epoch.plane_dimensions(p).unwrap();
            let bg = match p {
                0 => 64u32,
                _ => 512,
            };
            let mut samples = vec![bg; (pw * ph) as usize];
            if p == 0 {
                // Y plane: a 6x4 box of value 900 moving right by 2/frame.
                let x0 = 2 + 2 * f as u32;
                for yy in 2..6u32 {
                    for xx in x0..(x0 + 6).min(pw) {
                        samples[(yy * pw + xx) as usize] = 900;
                    }
                }
                // Overlay point on the final frame.
                if f == frames - 1 && f >= 2 {
                    samples[(8 * pw + 3) as usize] = 1000;
                }
            }
            let _ = max;
            planes.push(
                vole_video::media::plane::Plane::new(
                    epoch.planes()[p].component,
                    pw,
                    ph,
                    depth,
                    epoch.planes()[p].subsample_x,
                    epoch.planes()[p].subsample_y,
                    if depth.is_byte_depth() {
                        PlaneData::U8(samples.iter().map(|v| *v as u8).collect())
                    } else {
                        PlaneData::U16(samples.iter().map(|v| *v as u16).collect())
                    },
                )
                .unwrap(),
            );
        }
        out.push(Picture::from_planes(epoch, planes).unwrap());
    }
    out
}

#[test]
fn authored_10bit_420_matches_independent_compositor() -> Result<(), VoleError> {
    let epoch = yuv_epoch(20, 12, 10);
    let expected = naive_composite(&epoch, 4);
    // Author the same content procedurally: only the Y plane carries state.
    let mut y_prog = PlaneProgram::new(64);
    y_prog
        .objects
        .insert(PlaneObjectId(1), PlaneObject::fill(6, 4, 900));
    y_prog.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 2,
        y: 2,
    });
    // Observation 1..3: move; observation 3 adds an overlay point.
    for t in 1..=3u64 {
        let mut ops = vec![PlaneOp::SetPosition {
            id: PlaneInstanceId(1),
            x: 2 + 2 * t as i64,
            y: 2,
        }];
        if t == 3 {
            ops.push(PlaneOp::PatchOverlay {
                points: vec![(3, 8, 1000)],
            });
        }
        y_prog.intervals.push((t, ops));
    }
    let program = MultiPlaneProgram::new(
        epoch.clone(),
        vec![y_prog, PlaneProgram::new(512), PlaneProgram::new(512)],
    )?;
    assert_eq!(program.observation_count(), 4);
    for idx in 0..4u64 {
        let got = program.materialize_observation(idx)?;
        let want = &expected[idx as usize];
        for p in 0..epoch.plane_count() {
            assert_eq!(
                got.plane(p).unwrap().canonical_bytes(),
                want.plane(p).unwrap().canonical_bytes(),
                "authored 10-bit 420 frame {idx} plane {p}"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 3. Raster-origin exact floor (RAW / unchanged / residual) at 10-bit 4:2:0
// ---------------------------------------------------------------------------

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

/// A deterministic "sprite" sequence: static textured background on Y with a
/// moving bright box; chroma planes static at mid-gray.
fn sprite_observations(epoch: &VideoEpoch, frames: usize) -> Result<Vec<Picture>, VoleError> {
    let _pw0 = epoch.plane_dimensions(0)?;
    let max = epoch.planes()[0].bit_depth.max_sample();
    let mut out = Vec::new();
    for f in 0..frames {
        let mut planes = Vec::new();
        for p in 0..epoch.plane_count() {
            let (pw, ph) = epoch.plane_dimensions(p)?;
            let mut samples = Vec::with_capacity((pw * ph) as usize);
            for y in 0..ph {
                for x in 0..pw {
                    let v = if p == 0 {
                        let bg = 100 + (hash2(x, y, 0) % 12) as u32;
                        // moving 4x4 box of 950
                        let x0 = 2 + 2 * f as u32;
                        if (4..8).contains(&y) && (x >= x0 && x < x0 + 4) {
                            950
                        } else {
                            bg
                        }
                    } else {
                        512
                    };
                    samples.push(v.min(max));
                }
            }
            planes.push(vole_video::media::plane::Plane::new(
                epoch.planes()[p].component,
                pw,
                ph,
                epoch.planes()[p].bit_depth,
                epoch.planes()[p].subsample_x,
                epoch.planes()[p].subsample_y,
                PlaneData::U16(samples.iter().map(|v| *v as u16).collect()),
            )?);
        }
        out.push(Picture::from_planes(epoch, planes)?);
    }
    Ok(out)
}

#[test]
fn raster_floor_is_exact_and_stops_repeating_static_bytes() -> Result<(), VoleError> {
    let epoch = yuv_epoch(24, 16, 10);
    let mut obs = sprite_observations(&epoch, 4)?;
    // Append static duplicates: frames 4..6 identical to frame 3.
    for _ in 0..3 {
        obs.push(obs[3].clone());
    }
    let program = encode_pictures_exact(&epoch, &obs)?;
    assert_eq!(program.observation_count(), 7);
    // Sample-for-sample proof (the encoder proves it; assert at the API too).
    for (idx, want) in obs.iter().enumerate() {
        let got = program.materialize_observation(idx as u64)?;
        for p in 0..epoch.plane_count() {
            assert_eq!(
                got.plane(p).unwrap().canonical_bytes(),
                want.plane(p).unwrap().canonical_bytes()
            );
        }
    }
    // Static duplicates ride the unchanged lane: frame 6's plane has no
    // interval ops after frame 3's content was established. Measured claim:
    // the wire form is far smaller than the raw canonical bytes of 7 obs.
    let wire = write_multiplane(&program)?;
    let raw = obs.iter().map(|p| p.total_bytes()).sum::<u64>();
    assert!(
        (wire.len() as u64) < raw / 2,
        "unchanged/residual floor must beat full raster: {} < {}",
        wire.len(),
        raw
    );
    Ok(())
}

#[test]
fn raster_floor_handles_noise_as_raw_fallback() -> Result<(), VoleError> {
    // Random per-sample content changes every frame: the floor stays exact
    // and terminates (RAW replacement per changed plane), never pathological.
    let epoch = yuv_epoch(16, 12, 8);
    let mut obs = Vec::new();
    for f in 0..3u32 {
        let mut planes = Vec::new();
        for p in 0..epoch.plane_count() {
            let (pw, ph) = epoch.plane_dimensions(p)?;
            let n = (pw * ph) as usize;
            let data: Vec<u8> = (0..n)
                .map(|k| (hash2(k as u32, f, p as u32) % 256) as u8)
                .collect();
            planes.push(vole_video::media::plane::Plane::new(
                epoch.planes()[p].component,
                pw,
                ph,
                epoch.planes()[p].bit_depth,
                epoch.planes()[p].subsample_x,
                epoch.planes()[p].subsample_y,
                PlaneData::U8(data),
            )?);
        }
        obs.push(Picture::from_planes(&epoch, planes)?);
    }
    let program = encode_pictures_exact(&epoch, &obs)?;
    for (idx, want) in obs.iter().enumerate() {
        let got = program.materialize_observation(idx as u64)?;
        for p in 0..epoch.plane_count() {
            assert_eq!(
                got.plane(p).unwrap().canonical_bytes(),
                want.plane(p).unwrap().canonical_bytes()
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 4. v2 wire: canonical round trips and hostile containers
// ---------------------------------------------------------------------------

#[test]
fn wire_roundtrips_byte_exactly_across_layouts_and_depths() -> Result<(), VoleError> {
    let cases: Vec<(PixelLayout, u8, u32, u32)> = vec![
        (PixelLayout::Gray, 8, 16, 16),
        (PixelLayout::Gray, 10, 16, 16),
        (PixelLayout::Gray, 16, 9, 7),
        (PixelLayout::Yuv420, 8, 24, 16),
        (PixelLayout::Yuv420, 10, 24, 16),
        (PixelLayout::Yuv444, 8, 12, 10),
        (PixelLayout::Yuv444, 12, 8, 8),
        (PixelLayout::Gbr, 8, 10, 10),
        (PixelLayout::Rgb, 10, 9, 9),
        (PixelLayout::Rgba, 8, 7, 5),
        (PixelLayout::Yuva444, 8, 6, 6),
    ];
    for (layout, depth, w, h) in cases {
        let epoch = VideoEpoch::new_uniform(
            EpochId(0),
            w,
            h,
            layout,
            BitDepth::new(depth)?,
            ColorDescription::unspecified(),
            SampleAspectRatio::square(),
            Orientation::Normal,
            FieldStructure::Progressive,
        )?;
        let bg: Vec<u32> = epoch
            .planes()
            .iter()
            .map(|t| t.bit_depth.max_sample() / 2)
            .collect();
        let mut obs = vec![uniform_picture(&epoch, &bg)?];
        // Two ramp observations for the raster floor path.
        obs.push(ramp_picture(&epoch, 1)?);
        let program = encode_pictures_exact(&epoch, &obs)?;
        let bytes = write_multiplane(&program)?;
        // Header dispatch prefix: magic/version/universe.
        assert_eq!(&bytes[0..4], b"VOLE");
        assert_eq!(u16::from_le_bytes([bytes[5], bytes[6]]), 2);
        let parsed = parse_multiplane(&bytes)?;
        assert_eq!(parsed.epoch.layout(), layout);
        // Semantic identity: materialized observations match the original.
        for idx in 0..2u64 {
            let a = program.materialize_observation(idx)?;
            let b = parsed.materialize_observation(idx)?;
            for p in 0..epoch.plane_count() {
                assert_eq!(
                    a.plane(p).unwrap().canonical_bytes(),
                    b.plane(p).unwrap().canonical_bytes()
                );
            }
        }
        // Canonical fixpoint: writing the parse reproduces the bytes.
        let again = write_multiplane(&parsed)?;
        assert_eq!(again, bytes, "v2 wire is canonical (write∘parse == id)");
    }
    Ok(())
}

#[test]
fn wire_hostile_containers_are_typed() -> Result<(), VoleError> {
    let epoch = yuv_epoch(8, 8, 10);
    let bg = vec![0, 512, 512];
    let obs = vec![uniform_picture(&epoch, &bg)?];
    let program = encode_pictures_exact(&epoch, &obs)?;
    let bytes = write_multiplane(&program)?;
    // Content-level corruption (structure parses; the trailing digest catches
    // it). Offset 47 is the background sample LSB of the single-plane block
    // (see the grammar: header 24 + descriptor 21 + tag/idx 2); flipping it
    // to 1 stays inside the active depth, so only the digest can fire.
    for i in [47usize, bytes.len() - 1] {
        let mut bad = bytes.clone();
        bad[i] ^= 0x01;
        assert_eq!(
            parse_multiplane(&bad).unwrap_err(),
            VoleError::IntegrityMismatch,
            "flip at {i}"
        );
    }
    // Wrong magic surfaces structurally before the digest (v1 mirror).
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert_eq!(parse_multiplane(&bad).unwrap_err(), VoleError::BadMagic);
    // Truncations are typed (never a panic).
    for cut in [0usize, 1, 20, bytes.len() - 2] {
        let _ = parse_multiplane(&bytes[..cut]).unwrap_err();
    }
    // Unknown layout code surfaces structurally.
    let mut bad = bytes.clone();
    let dpos = 24; // descriptor tag at 24; layout code at 25..27
    assert_eq!(bytes[dpos], 0x11);
    bad[dpos + 1..dpos + 3].copy_from_slice(&999u16.to_le_bytes());
    assert_eq!(
        parse_multiplane(&bad).unwrap_err(),
        VoleError::UnsupportedPixelLayout
    );
    Ok(())
}

#[test]
fn wire_hostile_corpus_across_layouts_and_depths() -> Result<(), VoleError> {
    // The hostile contract must hold for every supported layout/depth family,
    // not only the single Gray/YUV420 fixtures above.
    let cases: Vec<(PixelLayout, u8, u32, u32)> = vec![
        (PixelLayout::Gray, 8, 12, 10),
        (PixelLayout::Gray, 16, 9, 7),
        (PixelLayout::Yuv420, 8, 24, 16),
        (PixelLayout::Yuv420, 10, 16, 12),
        (PixelLayout::Yuv444, 12, 8, 8),
        (PixelLayout::Rgb, 10, 9, 9),
        (PixelLayout::Rgba, 8, 7, 5),
        (PixelLayout::Yuva444, 8, 6, 6),
    ];
    for (layout, depth, w, h) in cases {
        let epoch = VideoEpoch::new_uniform(
            EpochId(0),
            w,
            h,
            layout,
            BitDepth::new(depth)?,
            ColorDescription::unspecified(),
            SampleAspectRatio::square(),
            Orientation::Normal,
            FieldStructure::Progressive,
        )?;
        let bg: Vec<u32> = epoch
            .planes()
            .iter()
            .map(|t| t.bit_depth.max_sample() / 2)
            .collect();
        let mut obs = vec![uniform_picture(&epoch, &bg)?];
        obs.push(ramp_picture(&epoch, 7)?);
        let program = encode_pictures_exact(&epoch, &obs)?;
        let bytes = write_multiplane(&program)?;
        let p = epoch.plane_count();
        // First plane block background LSB: header 24 + descriptor
        // (19 + 2p: tag 1 + layout 2 + count 1 + 2/plane + chroma/primaries/
        // transfer/matrix/range 5 + SAR 8 + orientation/field 2) + tag/idx 2.
        let bg_off = 45usize + 2 * p;
        // Content-level flips (structure still parses; the trailing digest
        // fires) for every layout/depth in the corpus.
        for i in [bg_off, bytes.len() - 1] {
            let mut bad = bytes.clone();
            bad[i] ^= 0x01;
            assert_eq!(
                parse_multiplane(&bad).unwrap_err(),
                VoleError::IntegrityMismatch,
                "{layout:?} d{depth}: content flip at {i}"
            );
        }
        // Wrong magic is structural, before the digest.
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert_eq!(
            parse_multiplane(&bad).unwrap_err(),
            VoleError::BadMagic,
            "{layout:?} d{depth}: magic"
        );
        // Unknown layout code surfaces structurally (descriptor layout field
        // at bytes 25..27).
        let mut bad = bytes.clone();
        bad[25..27].copy_from_slice(&999u16.to_le_bytes());
        assert_eq!(
            parse_multiplane(&bad).unwrap_err(),
            VoleError::UnsupportedPixelLayout,
            "{layout:?} d{depth}: layout code"
        );
        // Truncations are typed, never a panic.
        assert_eq!(
            parse_multiplane(&bytes[..0]).unwrap_err(),
            VoleError::Truncated,
            "{layout:?} d{depth}: empty"
        );
        for cut in [1usize, 31, 40, bytes.len() - 3] {
            let r = parse_multiplane(&bytes[..cut.min(bytes.len())]);
            assert!(r.is_err(), "{layout:?} d{depth}: cut at {cut} must fail");
        }
    }
    Ok(())
}

#[test]
fn wire_declares_unknown_features_and_versions() -> Result<(), VoleError> {
    let epoch = gray_epoch(4, 4, 8);
    let prog = MultiPlaneProgram::new(epoch, vec![PlaneProgram::new(0)])?;
    let mut bytes = write_multiplane(&prog)?;
    // Unknown feature bit.
    bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
    let n = bytes.len();
    let d = vole_video::integr::digest(&bytes[..n - 32]);
    bytes[n - 32..].copy_from_slice(&d);
    assert_eq!(
        parse_multiplane(&bytes).unwrap_err(),
        VoleError::UnsupportedFeature
    );
    // v1 version number on a v2 body fails closed.
    let mut bytes = write_multiplane(&prog)?;
    bytes[5..7].copy_from_slice(&1u16.to_le_bytes());
    let n = bytes.len();
    let d = vole_video::integr::digest(&bytes[..n - 32]);
    bytes[n - 32..].copy_from_slice(&d);
    assert_eq!(
        parse_multiplane(&bytes).unwrap_err(),
        VoleError::UnsupportedFeature
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 4b. Frozen v2 core grammar golden (pinned at the V.1.2 seal; deliberate
//     regression tripwire: any grammar change re-seals the format doc + tests)
// ---------------------------------------------------------------------------

#[test]
fn v2_core_wire_golden_is_stable() -> Result<(), VoleError> {
    let (_, program) = authored_scenario();
    let bytes = write_multiplane(&program)?;
    // Dispatch prefix: magic/version/universe are pure prefix fields (v1
    // mirror) and the stream is a standalone canonical v2 core container.
    assert_eq!(&bytes[0..4], b"VOLE");
    assert_eq!(bytes[4], 0);
    assert_eq!(u16::from_le_bytes([bytes[5], bytes[6]]), 2);
    assert_eq!(
        u32::from_le_bytes([bytes[7], bytes[8], bytes[9], bytes[10]]),
        2
    );
    // BLAKE3 over the payload == the pinned V.1.2 golden (frozen grammar).
    let hex = hex_digest(&bytes);
    assert_eq!(
        hex, "a5c1fb407c8b86604cb7f40227f1956a628061c63e5145d6c39b7d9b0a56a80f",
        "v2 core wire digest changed; re-freeze docs/format-v2.md deliberately"
    );
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let n = bytes.len();
    vole_video::integr::digest(&bytes[..n - 32])
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// 5. Timeline/epoch integration into a canonical video
// ---------------------------------------------------------------------------

#[test]
fn programs_bind_to_rational_pts_and_epoch_transitions() -> Result<(), VoleError> {
    // Epoch A: Gray8 8-bit; program with one moving fill (3 observations).
    let ea = gray_epoch(16, 12, 8);
    let mut pa = PlaneProgram::new(0);
    pa.objects
        .insert(PlaneObjectId(1), PlaneObject::fill(4, 4, 200));
    pa.instances.push(PlaneInstance {
        id: PlaneInstanceId(1),
        object: PlaneObjectId(1),
        x: 0,
        y: 0,
    });
    for t in 1..=2u64 {
        pa.intervals.push((
            t,
            vec![PlaneOp::SetPosition {
                id: PlaneInstanceId(1),
                x: 2 * t as i64,
                y: 0,
            }],
        ));
    }
    let prog_a = MultiPlaneProgram::new(ea.clone(), vec![pa])?;

    // Epoch B: YUV420 10-bit (interpretation change mid-stream; dense id 1).
    let eb = VideoEpoch::new_uniform(
        EpochId(1),
        16,
        12,
        PixelLayout::Yuv420,
        BitDepth::new(10)?,
        ColorDescription::unspecified(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )?;
    let prog_b = encode_pictures_exact(&eb, &[uniform_picture(&eb, &[512, 256, 256])?])?;

    let tb = TimeBase::for_frame_rate(24000, 1001)?;
    let mut observations = Vec::new();
    let mut pts = Pts::new(0, tb);
    for (program, epoch) in [(&prog_a, &ea), (&prog_b, &eb)] {
        for idx in 0..program.observation_count() {
            let picture = program.materialize_observation(idx)?;
            let planes = picture.into_planes();
            observations.push(CanonicalVideoObservation::new(
                epoch,
                pts,
                Some(Duration::new(1, tb)?),
                planes,
            )?);
            pts = pts.checked_add(Duration::new(1, tb)?)?;
        }
    }
    let video = CanonicalVideo::new(vec![ea, eb], observations)?;
    assert_eq!(video.observation_count(), 4);
    assert_eq!(video.epoch_of(0).unwrap().layout(), PixelLayout::Gray);
    assert_eq!(video.epoch_of(3).unwrap().layout(), PixelLayout::Yuv420);
    assert_eq!(video.total_span(tb)?.unwrap().value(), 4);
    Ok(())
}

// ---------------------------------------------------------------------------
// 6. Deterministic matrix sweep (used by the evidence example)
// ---------------------------------------------------------------------------

/// A curated matrix of (layout, depth, frames) exercised end-to-end by the
/// phase example: exact floor -> program -> wire -> parse -> materialize.
#[allow(clippy::type_complexity)] // one curated matrix row type for the evidence example
pub fn matrix_programs(
) -> Result<Vec<(String, VideoEpoch, MultiPlaneProgram, Vec<Picture>)>, VoleError> {
    let mut out = Vec::new();
    let specs: Vec<(PixelLayout, u8, u32, u32, usize)> = vec![
        (PixelLayout::Gray, 8, 32, 24, 3),
        (PixelLayout::Yuv420, 10, 32, 24, 3),
        (PixelLayout::Yuv444, 8, 16, 16, 2),
        (PixelLayout::Yuv422, 10, 24, 16, 2),
        (PixelLayout::Gbr, 8, 16, 12, 2),
    ];
    for (layout, depth, w, h, frames) in specs {
        let epoch = VideoEpoch::new_uniform(
            EpochId(0),
            w,
            h,
            layout,
            BitDepth::new(depth)?,
            ColorDescription::unspecified(),
            SampleAspectRatio::square(),
            Orientation::Normal,
            FieldStructure::Progressive,
        )?;
        let mut obs = Vec::with_capacity(frames);
        for f in 0..frames {
            let pic = if f == 0 {
                let bg: Vec<u32> = epoch
                    .planes()
                    .iter()
                    .map(|t| t.bit_depth.max_sample() / 3)
                    .collect();
                uniform_picture(&epoch, &bg)?
            } else {
                ramp_picture(&epoch, f as u64)?
            };
            obs.push(pic);
        }
        let program = encode_pictures_exact(&epoch, &obs)?;
        out.push((format!("{}-d{depth}", layout.label()), epoch, program, obs));
    }
    Ok(out)
}
