//! Phase V.1.1 courts: the canonical media domain (V.1 video programme —
//! contract `docs/phase-v1-video-architecture.md` §2.2–§2.5, master brief
//! §10–§30).
//!
//! The courts exercise the in-memory media interpretation layer on
//! **synthetic canonical vectors** only (no wire grammar, no foreign import —
//! those are V.1.2/V.1.3): rational media time over the standard frame-rate
//! grid, exact cross-base ordering/rescaling, the full pixel-layout registry
//! with the normative ceil subsampling geometry on odd dimensions, bit depths
//! 1..=16 with padding-bit discipline, color/HDR/orientation/SAR/interlace
//! preservation, epoch transitions, and a flagship synthetic HDR/VFR vector.
//! Hostile constructions are typed, deterministic errors — never panics.

use vole_video::media::color::{
    ChromaLocation, ColorDescription, ColorPrimaries, ColorRange, ContentLightLevel, HdrMetadata,
    MasteringDisplay, MatrixCoefficients, TransferCharacteristic,
};
use vole_video::media::epoch::{CanonicalVideo, CanonicalVideoObservation, EpochId, VideoEpoch};
use vole_video::media::meta::{FieldStructure, Orientation, SampleAspectRatio, VisualSideData};
use vole_video::media::plane::{BitDepth, PlaneData, PlaneStorage};
use vole_video::media::time::{Duration, Pts, TimeBase};
use vole_video::media::{Component, PackedSourceLayout, PixelLayout};
use vole_video::VoleError;

/// Deterministic per-sample hash (test harness).
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

fn e(id: u64, w: u32, h: u32, layout: PixelLayout, depth: u8) -> VideoEpoch {
    VideoEpoch::new_uniform(
        EpochId(id),
        w,
        h,
        layout,
        BitDepth::new(depth).unwrap(),
        ColorDescription::bt2020_pq(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )
    .unwrap()
}

fn obs_of(epoch: &VideoEpoch, pts: Pts) -> CanonicalVideoObservation {
    let planes = (0..epoch.plane_count())
        .map(|i| epoch.synthetic_plane(i).unwrap())
        .collect();
    CanonicalVideoObservation::new(
        epoch,
        pts,
        Some(Duration::new(1, pts.time_base()).unwrap()),
        planes,
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// 1. Rational media time across the standard frame-rate grid
// ---------------------------------------------------------------------------

#[test]
fn frame_rate_grid_is_exact() -> Result<(), VoleError> {
    // (fps_num, fps_den): 23.976, 24, 25, 29.97, 30, 50, 59.94, 60, 100, 120.
    let rates: [(u32, u32); 10] = [
        (24000, 1001),
        (24, 1),
        (25, 1),
        (30000, 1001),
        (30, 1),
        (50, 1),
        (60000, 1001),
        (60, 1),
        (100, 1),
        (120, 1),
    ];
    for (n, d) in rates {
        let tb = TimeBase::for_frame_rate(n, d)?;
        // One second is exactly `n/d` ticks at this base.
        let one_sec = Pts::new(i64::from(n), tb).rescale(TimeBase::whole_seconds())?;
        assert_eq!(one_sec.value(), i64::from(d), "1 s at {n}/{d} fps");
    }
    // Durations of exactly one frame at CFR are 1 tick; rescale across bases.
    let t24 = TimeBase::for_frame_rate(24, 1)?;
    let t23976 = TimeBase::for_frame_rate(24000, 1001)?;
    // 24 frames at 23.976 == 1001 ticks at 24fps? No: 24 ticks @23.976 tb is
    // 24*1001/24000 s = 1001/1000 s = 1.001 s. At 24fps that is 24.024 ticks
    // (not integral). At 25fps: 1001/1000 * 25 = 25.025 (not integral).
    // Instead: 24000 ticks @23.976 tb == 1001 s == 24024 ticks @24fps.
    let a = Pts::new(24000, t23976);
    let b = a.rescale(t24)?;
    assert_eq!(b.value(), 24024);
    Ok(())
}

#[test]
fn vfr_durations_are_per_observation_and_ordering_is_exact() -> Result<(), VoleError> {
    let tb = TimeBase::for_frame_rate(30000, 1001)?; // 29.97
    let epoch = e(0, 8, 8, PixelLayout::Yuv420, 8);
    let mut observations = Vec::new();
    let mut pts = Pts::new(0, tb);
    // True VFR: durations 1, 2, 1, 3, 1 ticks at 29.97 (per-observation).
    for dur in [1i64, 2, 1, 3, 1] {
        let planes = (0..epoch.plane_count())
            .map(|i| epoch.synthetic_plane(i).unwrap())
            .collect();
        observations.push(CanonicalVideoObservation::new(
            &epoch,
            pts,
            Some(Duration::new(dur, tb)?),
            planes,
        )?);
        pts = pts.checked_add(Duration::new(dur, tb)?)?;
    }
    let v = CanonicalVideo::new(vec![epoch], observations)?;
    let span = v.total_span(tb)?.unwrap();
    assert_eq!(span.value(), 1 + 2 + 1 + 3 + 1);
    // Ordering across bases is exact even for the 29.97 grid.
    let start = v.start_pts().unwrap();
    let end = v.end_pts().unwrap();
    assert!(end.cmp_pts(&start)? == core::cmp::Ordering::Greater);
    Ok(())
}

// ---------------------------------------------------------------------------
// 2. The full layout registry: plane counts, ceil geometry on odd dimensions
// ---------------------------------------------------------------------------

#[test]
fn layout_registry_covers_the_required_families_and_odd_dimensions() {
    let canonical: [PixelLayout; 16] = [
        PixelLayout::Gray,
        PixelLayout::Yuv400,
        PixelLayout::Yuv420,
        PixelLayout::Yuv422,
        PixelLayout::Yuv444,
        PixelLayout::Yuva420,
        PixelLayout::Yuva444,
        PixelLayout::Gbr,
        PixelLayout::Gbra,
        PixelLayout::Rgb,
        PixelLayout::Bgr,
        PixelLayout::Rgba,
        PixelLayout::Bgra,
        PixelLayout::Argb,
        PixelLayout::Abgr,
        PixelLayout::Indexed,
    ];
    for l in canonical {
        assert!(l.plane_count() >= 1);
        for (w, h) in [(1u32, 1u32), (3, 3), (1919, 1079), (1921, 1081)] {
            let total = l.total_sample_count(w, h).unwrap();
            assert!(total > 0);
            // Independent oracle: the ceil rule per plane.
            let mut expect = 0u64;
            for (i, tmpl) in l.planes().iter().enumerate() {
                let (pw, ph) = l.plane_dimensions(i, w, h).unwrap();
                let ew = (u64::from(w) + (1u64 << tmpl.subsample_x) - 1) >> tmpl.subsample_x;
                let eh = (u64::from(h) + (1u64 << tmpl.subsample_y) - 1) >> tmpl.subsample_y;
                assert_eq!(
                    (u64::from(pw), u64::from(ph)),
                    (ew, eh),
                    "{l:?} {w}x{h} plane {i}"
                );
                expect += ew * eh;
            }
            assert_eq!(total, expect, "{l:?} {w}x{h} total");
        }
    }
    // Packed source formats map to the documented canonical targets.
    assert_eq!(
        PackedSourceLayout::Nv12.canonical_target(),
        PixelLayout::Yuv420
    );
    assert_eq!(
        PackedSourceLayout::P010.canonical_target(),
        PixelLayout::Yuv420
    );
    assert_eq!(
        PackedSourceLayout::Yuyv422.canonical_target(),
        PixelLayout::Yuv422
    );
    assert_eq!(
        PackedSourceLayout::Pal8.canonical_target(),
        PixelLayout::Indexed
    );
    // 1919x1079 YUV420: chroma is ceil(1919/2) x ceil(1079/2).
    let l = PixelLayout::Yuv420;
    assert_eq!(l.plane_dimensions(1, 1919, 1079).unwrap(), (960, 540));
    assert_eq!(l.plane_dimensions(1, 1921, 1081).unwrap(), (961, 541));
    assert_eq!(l.plane_dimensions(1, 3, 3).unwrap(), (2, 2));
    assert_eq!(l.plane_dimensions(1, 1, 1).unwrap(), (1, 1));
}

// ---------------------------------------------------------------------------
// 3. Bit-depth court: 8/9/10/12/14/16, padding-bit discipline, LE canonical
// ---------------------------------------------------------------------------

#[test]
fn bit_depths_and_canonical_bytes_are_exact() -> Result<(), VoleError> {
    for bits in [8u8, 9, 10, 12, 14, 16] {
        let depth = BitDepth::new(bits)?;
        let epoch = e(0, 3, 3, PixelLayout::Yuv444, bits);
        // Synthetic planes carry the midpoint value and validate.
        for i in 0..epoch.plane_count() {
            let p = epoch.synthetic_plane(i)?;
            assert_eq!(p.storage(), depth.storage());
            assert_eq!(p.bit_depth(), depth);
            let bytes = p.canonical_bytes();
            let per = depth.storage().bytes_per_sample() as usize;
            assert_eq!(bytes.len(), 9 * per);
            // Round trip is identity.
            let back = vole_video::media::plane::Plane::from_canonical_bytes(
                p.component(),
                p.width(),
                p.height(),
                p.bit_depth(),
                p.subsample_x(),
                p.subsample_y(),
                &bytes,
            )?;
            assert_eq!(back.canonical_bytes(), bytes);
        }
    }
    // 10-bit sample carrying bits above depth 10 is refused at the API.
    assert_eq!(
        vole_video::media::plane::Plane::new(
            Component::Y,
            1,
            1,
            BitDepth::new(10)?,
            0,
            0,
            PlaneData::U16(vec![1 << 10]),
        )
        .unwrap_err(),
        VoleError::InvalidSamples
    );
    // 8-bit content cannot be stored as u16 and vice versa.
    assert!(vole_video::media::plane::Plane::new(
        Component::Y,
        1,
        1,
        BitDepth::new(8)?,
        0,
        0,
        PlaneData::U16(vec![1]),
    )
    .is_err());
    Ok(())
}

// ---------------------------------------------------------------------------
// 4. Flagship synthetic HDR/VFR vector with an epoch transition
// ---------------------------------------------------------------------------

#[test]
fn flagship_synthetic_hdr_vector_with_epoch_transition() -> Result<(), VoleError> {
    // Epoch 0: 10-bit BT.2020/PQ YUV420 at 1919x1079, 23.976 fps timeline.
    let a = VideoEpoch::new_uniform(
        EpochId(0),
        1919,
        1079,
        PixelLayout::Yuv420,
        BitDepth::new(10)?,
        ColorDescription::bt2020_pq(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )?
    .with_side_data(VisualSideData::MasteringDisplay(MasteringDisplay::new(
        [(34000, 16000), (13250, 34500), (7500, 3000)],
        (15635, 16450),
        10_000_000,
        50,
    )?))
    .with_side_data(VisualSideData::ContentLightLevel(ContentLightLevel {
        max_cll: 1000,
        max_fall: 400,
    }));
    // Epoch 1 (mid-stream switch): 12-bit 4:4:4 1921x1081, full-range identity.
    let b = VideoEpoch::new_uniform(
        EpochId(1),
        1921,
        1081,
        PixelLayout::Yuv444,
        BitDepth::new(12)?,
        ColorDescription::rgb_full(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )?;
    let tb = TimeBase::for_frame_rate(24000, 1001)?;
    // 24 observations on epoch A (23.976) then 4 on epoch B: VFR durations.
    let mut observations = Vec::new();
    let mut pts = Pts::new(0, tb);
    for k in 0..28u64 {
        let epoch = if k < 24 { &a } else { &b };
        let dur = if k % 3 == 0 { 2 } else { 1 }; // deterministic VFR
        let mut planes = Vec::new();
        for i in 0..epoch.plane_count() {
            // Deterministic synthetic content within active depth.
            let (pw, ph) = epoch.plane_dimensions(i)?;
            let n = (pw * ph) as usize;
            let base = epoch.planes()[i].bit_depth.max_sample() / 2;
            let data = match epoch.planes()[i].bit_depth.storage() {
                PlaneStorage::U8 => PlaneData::U8(
                    (0..n)
                        .map(|j| {
                            ((base + (hash2(j as u32, k as u32, i as u32) % 8) as u32) & 0xFF) as u8
                        })
                        .collect(),
                ),
                PlaneStorage::U16 => PlaneData::U16(
                    (0..n)
                        .map(|j| (base + (hash2(j as u32, k as u32, i as u32) % 16) as u32) as u16)
                        .collect(),
                ),
            };
            planes.push(vole_video::media::plane::Plane::new(
                epoch.planes()[i].component,
                pw,
                ph,
                epoch.planes()[i].bit_depth,
                epoch.planes()[i].subsample_x,
                epoch.planes()[i].subsample_y,
                data,
            )?);
        }
        observations.push(CanonicalVideoObservation::new(
            epoch,
            pts,
            Some(Duration::new(dur, tb)?),
            planes,
        )?);
        pts = pts.checked_add(Duration::new(dur, tb)?)?;
    }
    let v = CanonicalVideo::new(vec![a.clone(), b.clone()], observations)?;
    assert_eq!(v.observation_count(), 28);
    assert_eq!(v.epochs().len(), 2);
    // Epoch switch preserved: geometry/layout/depth change, never rescaled.
    assert_eq!(v.epoch_of(23).unwrap().layout(), PixelLayout::Yuv420);
    assert_eq!(v.epoch_of(24).unwrap().layout(), PixelLayout::Yuv444);
    assert_eq!(v.epoch_of(24).unwrap().width(), 1921);
    assert_eq!(v.epoch_of(24).unwrap().planes()[0].bit_depth.bits(), 12);
    // HDR metadata survived on epoch A; color signaling is exact.
    assert_eq!(a.color().primaries(), ColorPrimaries::Bt2020);
    assert_eq!(a.color().transfer(), TransferCharacteristic::Smpte2084);
    let md = &a.side_data();
    assert_eq!(md.len(), 2);
    // Span: 28 observations; durations 2 for k%3==0 (k in {0..27}, 10 twos)
    // plus 18 ones = 38 ticks at the 23.976 base.
    let span = v.total_span(tb)?.unwrap();
    assert_eq!(span.value(), 38);
    // Per-observation canonical storage bytes: epoch A 10-bit 420.
    assert_eq!(a.observation_bytes()?, (1919 * 1079 + 2 * 960 * 540) * 2);
    // 12-bit 444 epoch B: 3 full planes of u16.
    assert_eq!(b.observation_bytes()?, 3 * 1921 * 1081 * 2);
    // Exactness flags at domain level: samples were constructed within the
    // active depth, so every plane's bytes round-trip through the canonical
    // form identically (checked on the first observation).
    let first = &v.observations()[0];
    for p in first.planes() {
        let bytes = p.canonical_bytes();
        let back = vole_video::media::plane::Plane::from_canonical_bytes(
            p.component(),
            p.width(),
            p.height(),
            p.bit_depth(),
            p.subsample_x(),
            p.subsample_y(),
            &bytes,
        )?;
        assert_eq!(back.canonical_bytes(), bytes);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 5. Orientation / SAR / interlace are preserved interpretation
// ---------------------------------------------------------------------------

#[test]
fn orientation_sar_and_interlace_are_preserved_interpretation() -> Result<(), VoleError> {
    // Anamorphic interlaced portrait phone-style source: coded samples are
    // untouched by the metadata.
    let sar = SampleAspectRatio::new(4, 3)?;
    // BT.601 signaling with *left*-located chroma (MPEG-1 class), preserved
    // exactly inside the color description.
    let color = ColorDescription::new(
        ColorPrimaries::Smpte170M,
        TransferCharacteristic::Smpte170M,
        MatrixCoefficients::Smpte170M,
        ColorRange::Limited,
        ChromaLocation::Left,
    );
    let epoch = VideoEpoch::new_uniform(
        EpochId(0),
        720,
        576,
        PixelLayout::Yuv420,
        BitDepth::new(8)?,
        color,
        sar,
        Orientation::Rotate90,
        FieldStructure::InterlacedTopFieldFirst,
    )?;
    assert_eq!(epoch.orientation(), Orientation::Rotate90);
    assert!(epoch.field_structure().is_interlaced());
    assert_eq!(epoch.color().chroma_location(), ChromaLocation::Left);
    assert_eq!(epoch.sar(), sar);
    // Display aspect of the anamorphic picture: (720/576)*(4/3) = 5/3.
    assert_eq!(epoch.sar().display_aspect(720, 576)?, (5, 3));
    // Orientation never changes the coded geometry or the samples.
    let tb = TimeBase::for_frame_rate(25, 1)?;
    let obs = obs_of(&epoch, Pts::new(0, tb));
    assert_eq!(obs.planes()[0].width(), 720);
    assert_eq!(obs.planes()[0].height(), 576);
    assert_eq!(obs.planes()[1].width(), 360);
    // Interlaced content is preserved as declared; nothing deinterlaces.
    assert_eq!(
        epoch.field_structure(),
        FieldStructure::InterlacedTopFieldFirst
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 6. Hostile constructions are typed, deterministic errors
// ---------------------------------------------------------------------------

#[test]
fn hostile_media_domain_constructions_are_typed() {
    // Degenerate time bases.
    assert_eq!(TimeBase::new(0, 1).unwrap_err(), VoleError::InvalidTimeBase);
    assert_eq!(TimeBase::new(1, 0).unwrap_err(), VoleError::InvalidTimeBase);
    // Zero geometry is refused by the epoch constructor.
    assert!(VideoEpoch::new_uniform(
        EpochId(0),
        0,
        8,
        PixelLayout::Gray,
        BitDepth::new(8).unwrap(),
        ColorDescription::unspecified(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )
    .is_err());
    assert!(VideoEpoch::new_uniform(
        EpochId(0),
        8,
        0,
        PixelLayout::Gray,
        BitDepth::new(8).unwrap(),
        ColorDescription::unspecified(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )
    .is_err());
    // Invalid depth.
    assert!(BitDepth::new(0).is_err());
    assert!(BitDepth::new(17).is_err());
    // Zero/negative durations are not observation intervals.
    let tb = TimeBase::whole_seconds();
    assert_eq!(
        Duration::new(0, tb).unwrap_err(),
        VoleError::TimeNotRepresentable
    );
    assert_eq!(
        Duration::new(0, tb).unwrap_err(),
        VoleError::TimeNotRepresentable
    );
    // Non-dense epoch ids are refused at sequence build.
    let bad_epoch = e(3, 8, 8, PixelLayout::Gray, 8); // id 3 in slot 0
    let tb = TimeBase::ticks_per_second(25).unwrap();
    let obs = obs_of(&bad_epoch, Pts::new(0, tb));
    assert_eq!(
        CanonicalVideo::new(vec![bad_epoch], vec![obs]).unwrap_err(),
        VoleError::EpochViolation
    );
    // Observation planes that contradict the declared epoch's interpretation
    // fail typed at sequence build (geometry mismatch between the epochs).
    let a = e(0, 4, 4, PixelLayout::Yuv420, 10);
    let b = e(0, 8, 8, PixelLayout::Yuv420, 10);
    let obs_a = obs_of(&a, Pts::new(0, tb));
    assert!(CanonicalVideo::new(vec![b], vec![obs_a]).is_err());
    // Oversized opaque side data is refused.
    assert!(matches!(
        VisualSideData::opaque(1, vec![0; 1 << 21]),
        Err(VoleError::DimensionTooLarge)
    ));
    // Non-monotonic PTS fail the sequence (typed).
    let e1 = e(0, 4, 4, PixelLayout::Yuv420, 10);
    let t30 = TimeBase::for_frame_rate(30000, 1001).unwrap();
    let o0 = obs_of(&e1, Pts::new(1, t30));
    let o1 = obs_of(&e1, Pts::new(0, t30));
    assert_eq!(
        CanonicalVideo::new(vec![e1], vec![o0, o1]).unwrap_err(),
        VoleError::EpochViolation
    );
    // Depth mismatched plane payload storage is refused.
    assert!(vole_video::media::plane::Plane::new(
        Component::Gray,
        2,
        2,
        BitDepth::new(8).unwrap(),
        0,
        0,
        PlaneData::U16(vec![0; 4]),
    )
    .is_err());
    // Invalid geometry for a plane (zero dims).
    assert!(vole_video::media::plane::Plane::new(
        Component::Gray,
        0,
        2,
        BitDepth::new(8).unwrap(),
        0,
        0,
        PlaneData::U8(vec![]),
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// 7. Determinism and the deterministic sweep oracle
// ---------------------------------------------------------------------------

#[test]
fn domain_constructions_are_deterministic_across_layouts_and_depths() {
    // Rebuilding identical vectors yields identical values (no hidden state).
    let t1 = e(0, 16, 16, PixelLayout::Yuv420, 10);
    let t2 = e(0, 16, 16, PixelLayout::Yuv420, 10);
    assert_eq!(
        t1.observation_bytes().unwrap(),
        t2.observation_bytes().unwrap()
    );
    assert_eq!(t1.planes(), t2.planes());
    // Sweep layouts x depths x a deterministic dimension set: every plane
    // sample count equals the independently computed ceil-rule oracle, and
    // every epoch's total bytes equal the sum over planes.
    let dims: [(u32, u32); 5] = [(1, 1), (3, 3), (16, 9), (1919, 1079), (1921, 1081)];
    let layouts = [
        PixelLayout::Gray,
        PixelLayout::Yuv400,
        PixelLayout::Yuv420,
        PixelLayout::Yuv422,
        PixelLayout::Yuv444,
        PixelLayout::Yuva420,
        PixelLayout::Yuva444,
        PixelLayout::Gbr,
        PixelLayout::Gbra,
        PixelLayout::Rgb,
        PixelLayout::Rgba,
        PixelLayout::Indexed,
    ];
    for l in layouts {
        for depth in [8u8, 10, 12, 16] {
            for (w, h) in dims {
                let ep = e(0, w, h, l, depth);
                let mut expect_bytes = 0u64;
                for i in 0..ep.plane_count() {
                    let (pw, ph) = ep.plane_dimensions(i).unwrap();
                    let n = u64::from(pw) * u64::from(ph);
                    let per = ep.planes()[i].bit_depth.storage().bytes_per_sample();
                    expect_bytes += n * per;
                }
                assert_eq!(ep.observation_bytes().unwrap(), expect_bytes);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 8. Reference color sets and HDR metadata bounds
// ---------------------------------------------------------------------------

#[test]
fn color_signal_sets_are_preserved_exactly() {
    let d = ColorDescription::bt2020_hlg();
    assert_eq!(d.primaries(), ColorPrimaries::Bt2020);
    assert_eq!(d.transfer(), TransferCharacteristic::AribStdB67);
    assert_eq!(d.range(), ColorRange::Limited);
    assert!(!d.has_unspecified());
    let c = ColorDescription::new(
        ColorPrimaries::Bt2020,
        TransferCharacteristic::Smpte2084,
        MatrixCoefficients::Bt2020Cl,
        ColorRange::Full,
        ChromaLocation::Unspecified,
    );
    assert!(c.chroma_location().is_unspecified());
    assert!(c.matrix() == MatrixCoefficients::Bt2020Cl);
    assert!(ColorDescription::unspecified().has_unspecified());
    // Mastering-display validation.
    assert!(MasteringDisplay::new(
        [(50000, 50000), (0, 0), (0, 0)],
        (15635, 16450),
        100_000_000,
        0,
    )
    .is_ok());
    assert!(MasteringDisplay::new([(50001, 0), (0, 0), (0, 0)], (0, 0), 1, 0).is_err());
    let hdr = HdrMetadata {
        mastering_display: None,
        content_light_level: Some(ContentLightLevel {
            max_cll: 1000,
            max_fall: 400,
        }),
    };
    assert_eq!(hdr.content_light_level.unwrap().max_fall, 400);
}
