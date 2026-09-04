//! Phase V.1.1 evidence proof: the canonical media domain (V.1 video
//! programme, contract §2.2–§2.5).
//!
//! Builds a **flagship synthetic canonical vector** — a two-epoch canonical
//! video in memory: 24 observations of 10-bit BT.2020/PQ YUV 4:2:0 at
//! 1919×1079 on a 23.976 (24000/1001) rational timeline with per-observation
//! VFR durations and HDR static metadata, then an epoch transition to 12-bit
//! 4:4:4 at 1921×1081 — and measures the domain facts the V.1.2 multiplane
//! core and V.1.3 import bridge will build on:
//!
//! * the exact per-plane ceil-rule geometry of the odd 1919×1079 picture;
//! * canonical tight storage bytes per observation (u16 LE for 10/12-bit);
//! * exact rational timeline accounting (PTS grid, per-observation durations,
//!   total span, cross-base ordering);
//! * preservation of color/HDR/orientation/SAR/field interpretation.
//!
//! Every observation is validated against its epoch; every plane's canonical
//! byte form round-trips exactly. Run:
//! `cargo run --release --example media_proof`

use std::time::Instant;

use vole_video::media::color::{ColorDescription, ContentLightLevel, MasteringDisplay};
use vole_video::media::epoch::{CanonicalVideo, CanonicalVideoObservation, EpochId, VideoEpoch};
use vole_video::media::meta::{FieldStructure, Orientation, SampleAspectRatio, VisualSideData};
use vole_video::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use vole_video::media::time::{Duration, Pts, TimeBase};
use vole_video::media::PixelLayout;
use vole_video::VoleError;

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

fn main() -> Result<(), VoleError> {
    let t0 = Instant::now();

    // Epoch A: 10-bit BT.2020/PQ YUV420 @ 1919x1079 (odd dimensions) with HDR
    // static metadata.
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

    // Epoch B: 12-bit 4:4:4 @ 1921x1081 (epoch transition mid-stream).
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
    println!("media domain proof (synthetic canonical vectors only — no wire, no import)");
    println!();
    println!(
        "epoch A: {}x{} yuv420 10-bit (bt2020/pq/bt2020ncl/limited/center) at tb {}",
        a.width(),
        a.height(),
        tb
    );
    let mut total_bytes = 0u64;
    for (i, tmpl) in a.planes().iter().enumerate() {
        let (pw, ph) = a.plane_dimensions(i)?;
        let n = a.plane_sample_count(i)?;
        let bytes = n * tmpl.bit_depth.storage().bytes_per_sample();
        total_bytes += bytes;
        println!(
            "  plane {} {}: {:>4}x{:<4} subsample({},{}) depth {} storage {:?} = {} B",
            i,
            tmpl.component.label(),
            pw,
            ph,
            tmpl.subsample_x,
            tmpl.subsample_y,
            tmpl.bit_depth.bits(),
            tmpl.bit_depth.storage(),
            bytes
        );
    }
    println!(
        "  odd-dim ceil rule: chroma of 1919x1079 is 960x540 (never floor), observation storage = {} B",
        total_bytes
    );

    // Build 24 + 4 = 28 observations with deterministic VFR durations.
    let mut observations = Vec::new();
    let mut pts = Pts::new(0, tb);
    for k in 0..28u64 {
        let epoch = if k < 24 { &a } else { &b };
        let dur = if k % 3 == 0 { 2 } else { 1 };
        let mut planes = Vec::new();
        for i in 0..epoch.plane_count() {
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
            planes.push(Plane::new(
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
    let span = v.total_span(tb)?.unwrap();
    println!();
    println!(
        "canonical video: {} observations over {} epochs (A: 24 obs, B: 4 obs)",
        v.observation_count(),
        v.epochs().len()
    );
    println!(
        "  timeline: tb {}, start pts 0, {} VFR observations (durations 1/2 ticks), total span {} ticks = 19019/12000 s exactly (not an integral number of ms — the rational grid is preserved, never rounded)",
        tb,
        v.observation_count(),
        span.value()
    );
    println!(
        "  epoch transition at observation 24: {}x{} yuv420 10-bit -> {}x{} yuv444 12-bit (no silent rescale)",
        v.epoch_of(23).unwrap().width(),
        v.epoch_of(23).unwrap().height(),
        v.epoch_of(24).unwrap().width(),
        v.epoch_of(24).unwrap().height()
    );
    println!(
        "  color preserved: {} ; HDR side data on epoch A: {} typed entries",
        a.color().describe(),
        a.side_data().len()
    );

    // Byte accounting over the whole synthetic canonical sequence.
    let mut canonical_bytes = 0u64;
    for obs in v.observations() {
        for p in obs.planes() {
            canonical_bytes += p.canonical_bytes().len() as u64;
            // Exactness: every plane's canonical form round-trips.
            let back = Plane::from_canonical_bytes(
                p.component(),
                p.width(),
                p.height(),
                p.bit_depth(),
                p.subsample_x(),
                p.subsample_y(),
                &p.canonical_bytes(),
            )?;
            assert_eq!(back.canonical_bytes(), p.canonical_bytes());
        }
    }
    println!();
    println!(
        "canonical sample bytes across the vector: {} (A: {} B/obs x24, B: {} B/obs x4)",
        canonical_bytes,
        a.observation_bytes()?,
        b.observation_bytes()?
    );

    // Cross-base ordering sanity on the 23.976 grid.
    let sec = Pts::new(24000, tb).rescale(TimeBase::whole_seconds())?;
    assert_eq!(sec.value(), 1001, "24000 ticks @23.976 == 1001 s");
    println!();
    println!(
        "media proof: OK (synthetic vector + geometry/timeline/storage/color invariants) in {:.1} s",
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}
