//! Phase V.1.3 courts — the foreign ingest bridge (V.1 video programme,
//! contract §2.1/§2.4; brief §31–§40, §217–§219).
//!
//! Every court that needs FFmpeg generates its fixtures deterministically
//! from authored canonical frames (via the reversible source-layout repacker)
//! and skips cleanly when the tools are not on `PATH`.

use std::path::{Path, PathBuf};

use vole_video::media::bridge::canonicalize::{self};
use vole_video::media::bridge::nut::{parse_nut, NutStream};
use vole_video::media::bridge::run::{run_bounded, ChildLimits, ToolPaths};
use vole_video::media::bridge::{import_video, verify_frames, ImportOptions, VerifiedImport};
use vole_video::media::color::ColorDescription;
use vole_video::media::epoch::{EpochId, VideoEpoch};
use vole_video::media::layout::PixelLayout;
use vole_video::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
use vole_video::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use vole_video::media::time::TimeBase;
use vole_video::VoleError;

fn epoch_for(layout: PixelLayout, depth: u8, w: u32, h: u32) -> VideoEpoch {
    VideoEpoch::new_uniform(
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
    .unwrap()
}

fn tmp_path(name: &str) -> PathBuf {
    let pid = std::process::id();
    let mut p = std::env::temp_dir();
    p.push(format!("vole-v13-{pid}-{name}"));
    p
}

/// Deterministic authored canonical frames: `frames` planes each with the
/// layout's canonical geometry, samples in the active depth.
fn authored_frames(
    layout: PixelLayout,
    depth: u8,
    w: u32,
    h: u32,
    frames: usize,
) -> Vec<vole_video::media::picture::Picture> {
    let epoch = epoch_for(layout, depth, w, h);
    let max = BitDepth::new(depth).unwrap().max_sample();
    (0..frames)
        .map(|f| {
            let mut planes = Vec::new();
            for p in 0..epoch.plane_count() {
                let (pw, ph) = epoch.plane_dimensions(p).unwrap();
                let n = (pw * ph) as usize;
                let values: Vec<u32> = (0..n)
                    .map(|i| {
                        let x = (i as u32) % pw;
                        let y = (i as u32) / pw;
                        (y * 7 + x * 3 + f as u32 * 11 + p as u32 * 5 + 1) % (max + 1)
                    })
                    .collect();
                let tmpl = &epoch.planes()[p];
                let data = match tmpl.bit_depth.storage() {
                    PlaneStorage::U8 => PlaneData::U8(values.iter().map(|v| *v as u8).collect()),
                    PlaneStorage::U16 => PlaneData::U16(values.iter().map(|v| *v as u16).collect()),
                };
                planes.push(
                    Plane::new(
                        tmpl.component,
                        pw,
                        ph,
                        tmpl.bit_depth,
                        tmpl.subsample_x,
                        tmpl.subsample_y,
                        data,
                    )
                    .unwrap(),
                );
            }
            vole_video::media::picture::Picture::from_planes(&epoch, planes).unwrap()
        })
        .collect()
}

/// Write `pix_fmt` source-layout bytes for the authored canonical frames and
/// wrap them in a NUT file (rawvideo carrier — the simplest foreign media).
#[allow(clippy::too_many_arguments)] // one fixture-builder helper for the courts
fn make_nut_fixture(
    tools: &ToolPaths,
    name: &str,
    pix_fmt: &str,
    layout: PixelLayout,
    depth: u8,
    w: u32,
    h: u32,
    frames: usize,
) -> PathBuf {
    let pics = authored_frames(layout, depth, w, h, frames);
    let mut source_bytes = Vec::new();
    for pic in &pics {
        let planes: Vec<Plane> = pic.planes().to_vec();
        let payload = canonicalize::repack_frame(pix_fmt, u64::from(w), u64::from(h), &planes)
            .expect("repack authored frames");
        source_bytes.extend_from_slice(&payload);
    }
    let yuv = tmp_path(name);
    let nut = tmp_path(&format!("{name}.nut"));
    std::fs::write(&yuv, &source_bytes).expect("write fixture");
    let out = run_bounded(
        &tools.ffmpeg,
        &[
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            pix_fmt,
            "-s",
            &format!("{w}x{h}"),
            "-r",
            "25",
            "-i",
            &yuv.display().to_string(),
            "-c:v",
            "rawvideo",
            "-f",
            "nut",
            "-y",
            &nut.display().to_string(),
        ],
        &ChildLimits::default(),
    )
    .unwrap();
    assert_eq!(out.code, Some(0), "fixture mux failed");
    let _ = std::fs::remove_file(&yuv);
    nut
}

fn import(opts_path: &Path) -> Result<VerifiedImport, VoleError> {
    let tools = ToolPaths::discover()?;
    import_video(&ImportOptions {
        source: opts_path.to_path_buf(),
        stream: None,
        tools: Some(tools),
        limits: Default::default(),
    })
}

/// The canonical sample bytes of one observation's picture.
fn pic_bytes(pic: &vole_video::media::picture::Picture) -> Vec<u8> {
    let mut out = Vec::new();
    for p in pic.planes() {
        out.extend_from_slice(&p.canonical_bytes());
    }
    out
}

fn ffmpeg_on_path() -> bool {
    vole_video::media::bridge::run::tools_available()
}

#[test]
fn planar_imports_are_exact_against_authored_frames() -> Result<(), VoleError> {
    if !ffmpeg_on_path() {
        eprintln!("skip: ffmpeg/ffprobe not on PATH");
        return Ok(());
    }
    let tools = ToolPaths::discover()?;
    // (pix_fmt, layout, depth, w, h) — planar, incl. odd 18x12 for the ceil
    // chroma rule and 10/16-bit depths.
    let cases: &[(&str, PixelLayout, u8, u32, u32)] = &[
        ("yuv420p", PixelLayout::Yuv420, 8, 18, 12),
        ("yuv420p10le", PixelLayout::Yuv420, 10, 32, 16),
        ("gray", PixelLayout::Gray, 8, 20, 14),
        ("gray16le", PixelLayout::Gray, 16, 20, 14),
        ("yuv444p", PixelLayout::Yuv444, 8, 16, 16),
        ("yuv422p", PixelLayout::Yuv422, 8, 16, 16),
        ("gbrp", PixelLayout::Gbr, 8, 16, 16),
    ];
    for (pix_fmt, layout, depth, w, h) in cases {
        let frames = 5usize;
        let nut = make_nut_fixture(&tools, pix_fmt, pix_fmt, *layout, *depth, *w, *h, frames);
        let imp = import(&nut).map_err(|e| {
            panic!("{pix_fmt}: import failed: {e:?}");
        })?;
        assert_eq!(
            imp.video.observation_count(),
            frames as u64,
            "{pix_fmt}: frame count"
        );
        assert_eq!(imp.epoch.layout(), *layout, "{pix_fmt}: layout");
        assert_eq!(imp.epoch.planes()[0].bit_depth.bits(), *depth);
        assert_eq!(imp.checks.verified_frames, frames as u64);
        assert_eq!(imp.checks.oracle_frames, frames as u64);
        assert!(imp.checks.pts_matched, "{pix_fmt}: oracle PTS all matched");
        // Exact ground truth: every observation equals the authored picture.
        let expected = authored_frames(*layout, *depth, *w, *h, frames);
        for (k, obs) in imp.video.observations().iter().enumerate() {
            let got = pic_bytes(&vole_video::media::picture::Picture::from_planes(
                &imp.epoch,
                obs.planes().to_vec(),
            )?);
            assert_eq!(got, pic_bytes(&expected[k]), "{pix_fmt}: frame {k} exact");
        }
        // Deterministic sequence digest: re-import reproduces it exactly.
        let again = import(&nut)?;
        assert_eq!(imp.sequence_blake3, again.sequence_blake3, "{pix_fmt}");
        assert_eq!(imp.sequence_sha256, again.sequence_sha256, "{pix_fmt}");
        let _ = std::fs::remove_file(&nut);
    }
    Ok(())
}

#[test]
fn packed_and_semiplanar_imports_repack_exactly() -> Result<(), VoleError> {
    if !ffmpeg_on_path() {
        eprintln!("skip: ffmpeg/ffprobe not on PATH");
        return Ok(());
    }
    let tools = ToolPaths::discover()?;
    // Packed/semi-planar source layouts. (p010le is not courted through a NUT
    // carrier: FFmpeg's NUT muxer has no p010 rawvideo tag and relabels it,
    // which VOLE's fail-closed canonicalizer correctly refuses — recorded.)
    let cases: &[(&str, PixelLayout, u8, u32, u32)] = &[
        ("rgb24", PixelLayout::Rgb, 8, 12, 10),
        ("bgra", PixelLayout::Bgra, 8, 12, 10),
        ("nv12", PixelLayout::Yuv420, 8, 16, 16),
        ("yuyv422", PixelLayout::Yuv422, 8, 16, 16),
    ];
    for (pix_fmt, layout, depth, w, h) in cases {
        let frames = 4usize;
        let nut = make_nut_fixture(&tools, pix_fmt, pix_fmt, *layout, *depth, *w, *h, frames);
        let imp = import(&nut).map_err(|e| {
            panic!("{pix_fmt}: import failed: {e:?}");
        })?;
        assert_eq!(imp.video.observation_count(), frames as u64, "{pix_fmt}");
        // Exact ground truth in the *source layout*: repacking the canonical
        // planes must reproduce the authored payload bytes.
        let expected = authored_frames(*layout, *depth, *w, *h, frames);
        for (k, obs) in imp.video.observations().iter().enumerate() {
            let planes: Vec<Plane> = obs.planes().to_vec();
            let repacked =
                canonicalize::repack_frame(pix_fmt, u64::from(*w), u64::from(*h), &planes)?;
            let exp_planes: Vec<Plane> = expected[k].planes().to_vec();
            let exp_payload =
                canonicalize::repack_frame(pix_fmt, u64::from(*w), u64::from(*h), &exp_planes)?;
            assert_eq!(repacked, exp_payload, "{pix_fmt}: frame {k} exact");
        }
        let _ = std::fs::remove_file(&nut);
    }
    Ok(())
}

#[test]
fn compressed_sources_import_oracle_exact() -> Result<(), VoleError> {
    if !ffmpeg_on_path() {
        eprintln!("skip: ffmpeg/ffprobe not on PATH");
        return Ok(());
    }
    let tools = ToolPaths::discover()?;
    // FFV1 (lossless) in Matroska, 10-bit: canonical ground truth must hold.
    let nut10 = make_nut_fixture(
        &tools,
        "ffv1src",
        "yuv420p10le",
        PixelLayout::Yuv420,
        10,
        32,
        16,
        6,
    );
    let mkv = tmp_path("ffv1.mkv");
    let out = run_bounded(
        &tools.ffmpeg,
        &[
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-i",
            &nut10.display().to_string(),
            "-c:v",
            "ffv1",
            "-y",
            &mkv.display().to_string(),
        ],
        &ChildLimits::default(),
    )?;
    assert_eq!(out.code, Some(0));
    let imp = import(&mkv)?;
    assert_eq!(imp.video.observation_count(), 6);
    let expected = authored_frames(PixelLayout::Yuv420, 10, 32, 16, 6);
    for (k, obs) in imp.video.observations().iter().enumerate() {
        assert_eq!(
            pic_bytes(&vole_video::media::picture::Picture::from_planes(
                &imp.epoch,
                obs.planes().to_vec(),
            )?),
            pic_bytes(&expected[k]),
            "ffv1 10-bit frame {k}"
        );
    }
    let _ = std::fs::remove_file(&nut10);
    let _ = std::fs::remove_file(&mkv);

    // H.264 (lossy) MP4: oracle-exact + reproducible, no authored ground
    // truth claim.
    let src = tmp_path("h264.mp4");
    let out = run_bounded(
        &tools.ffmpeg,
        &[
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=25:duration=1",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-y",
            &src.display().to_string(),
        ],
        &ChildLimits::default(),
    )?;
    assert_eq!(out.code, Some(0));
    let a = import(&src)?;
    let b = import(&src)?;
    assert_eq!(a.video.observation_count(), 25);
    assert_eq!(a.checks.verified_frames, 25);
    assert_eq!(a.sequence_blake3, b.sequence_blake3, "reproducible import");
    assert_eq!(a.epoch.layout(), PixelLayout::Yuv420);
    assert_eq!(a.epoch.planes()[0].bit_depth.bits(), 8);
    assert_eq!(a.epoch.width(), 160);
    let _ = std::fs::remove_file(&src);
    Ok(())
}

#[test]
fn vfr_timeline_preserves_exact_deltas() -> Result<(), VoleError> {
    if !ffmpeg_on_path() {
        eprintln!("skip: ffmpeg/ffprobe not on PATH");
        return Ok(());
    }
    let tools = ToolPaths::discover()?;
    // CFR authored source as rawvideo, then a time jump mid-sequence (true
    // VFR) re-muxed into NUT.
    let pics = authored_frames(PixelLayout::Yuv420, 8, 64, 48, 6);
    let mut raw = Vec::new();
    for pic in &pics {
        let planes: Vec<Plane> = pic.planes().to_vec();
        raw.extend_from_slice(&canonicalize::repack_frame("yuv420p", 64, 48, &planes)?);
    }
    let yuv = tmp_path("vfr.yuv");
    std::fs::write(&yuv, &raw).expect("write fixture");
    let vfr = tmp_path("vfr.nut");
    let out = run_bounded(
        &tools.ffmpeg,
        &[
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-s",
            "64x48",
            "-r",
            "25",
            "-i",
            &yuv.display().to_string(),
            "-vf",
            "setpts='if(eq(N,3),PTS+0.5/TB,PTS)'",
            "-c:v",
            "rawvideo",
            "-f",
            "nut",
            "-y",
            &vfr.display().to_string(),
        ],
        &ChildLimits::default(),
    )?;
    assert_eq!(out.code, Some(0));
    let _ = std::fs::remove_file(&yuv);
    let imp = import(&vfr)?;
    let obs = imp.video.observations();
    assert!(obs.len() >= 4, "jump dedup must still leave >= 4 frames");
    // Durations equal the exact PTS deltas; the jump appears as one large
    // delta (> 2x the modal delta).
    let mut deltas = Vec::new();
    for (i, o) in obs.iter().enumerate() {
        assert_eq!(
            o.pts().time_base(),
            imp.video.observations()[0].pts().time_base()
        );
        if i + 1 < obs.len() {
            let d = o.duration().expect("duration present before the last");
            deltas.push(d.value());
        } else {
            assert!(o.duration().is_none(), "last observation duration unknown");
        }
    }
    let base = *deltas.iter().min().unwrap();
    assert!(
        deltas.iter().any(|d| *d > 2 * base),
        "VFR jump present: {deltas:?}"
    );
    let _ = std::fs::remove_file(&vfr);
    Ok(())
}

#[test]
fn hostile_nut_bytes_are_typed_never_panic() -> Result<(), VoleError> {
    if !ffmpeg_on_path() {
        eprintln!("skip: ffmpeg/ffprobe not on PATH");
        return Ok(());
    }
    let tools = ToolPaths::discover()?;
    let nut = make_nut_fixture(
        &tools,
        "hostile",
        "yuv420p",
        PixelLayout::Yuv420,
        8,
        64,
        48,
        6,
    );
    let bytes = std::fs::read(&nut).expect("read fixture");
    assert!(parse_nut(&bytes).is_ok());
    // Wrong magic.
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert_eq!(parse_nut(&bad).unwrap_err(), VoleError::BadMagic);
    // Truncation across the whole file: always a typed error, never a panic.
    for cut in [
        0usize,
        1,
        24,
        60,
        200,
        bytes.len() / 2,
        bytes.len() - 10,
        bytes.len() - 1,
    ] {
        let _ = parse_nut(&bytes[..cut]);
    }
    // Structural flips in the header region are typed.
    let mut bad = bytes.clone();
    bad[25] ^= 0xff; // main-header version varlen byte
    assert!(parse_nut(&bad).is_err());
    let mut bad = bytes.clone();
    // Flip one byte inside the main packet body CRC region (end of main
    // header): must fail the CRC typed, not panic.
    let crc_off = find_main_header_end(&bytes);
    bad[crc_off] ^= 0xff;
    assert!(parse_nut(&bad).is_err());
    // Payload-byte flips stay structurally parseable (the oracle catches
    // them at verification — courted separately); they must never panic.
    for &off in &[bytes.len() / 2, bytes.len() - 40] {
        let mut bad = bytes.clone();
        bad[off] ^= 0x01;
        let _ = parse_nut(&bad);
    }
    let _ = std::fs::remove_file(&nut);
    Ok(())
}

/// Locate the CRC of the main header (startcode at 25, then the length
/// varlen, then the body, then 4 CRC bytes) — independent helper used by the
/// hostile court.
fn find_main_header_end(bytes: &[u8]) -> usize {
    let mut p = 25 + 8;
    let mut v: u64 = 0;
    loop {
        let b = bytes[p];
        p += 1;
        v = (v << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            break;
        }
    }
    // The length counts body + 4 CRC bytes; the CRC starts after body.
    p + (v as usize) - 4
}

#[test]
fn oracle_mismatch_is_typed() -> Result<(), VoleError> {
    if !ffmpeg_on_path() {
        eprintln!("skip: ffmpeg/ffprobe not on PATH");
        return Ok(());
    }
    let tools = ToolPaths::discover()?;
    let nut = make_nut_fixture(
        &tools,
        "mismatch",
        "yuv420p",
        PixelLayout::Yuv420,
        8,
        64,
        48,
        5,
    );
    let bytes = std::fs::read(&nut).expect("read fixture");
    let imp = import(&nut)?;
    let _ = std::fs::remove_file(&nut);
    // Tamper one byte in the middle of a frame payload: the structure still
    // parses, but the oracle verification must fail typed.
    let mid = bytes.len() / 2;
    let mut bad = bytes.clone();
    bad[mid] ^= 0x55;
    let parsed: NutStream = parse_nut(&bad)?;
    assert_eq!(parsed.frames.len(), imp.nut.frames.len());
    let w = u64::from(imp.epoch.width());
    let h = u64::from(imp.epoch.height());
    let r = verify_frames("yuv420p", w, h, &imp.oracle, &parsed);
    assert_eq!(r.unwrap_err(), VoleError::CanonicalHashMismatch);
    // A pristine re-parse verifies cleanly.
    let clean = parse_nut(&bytes)?;
    let r = verify_frames("yuv420p", w, h, &imp.oracle, &clean);
    assert!(r.is_ok());
    Ok(())
}

#[test]
fn missing_tools_are_typed_and_trash_inputs_fail_closed() {
    // Garbage inputs must produce typed bridge errors, never a hang/panic.
    let dir = std::env::temp_dir();
    let junk = dir.join(format!("vole-v13-junk-{}", std::process::id()));
    std::fs::write(&junk, vec![0x13u8; 4096]).unwrap();
    let r = import(&junk);
    let _ = std::fs::remove_file(&junk);
    if ffmpeg_on_path() {
        assert!(r.is_err(), "junk input must fail typed");
    }
    // Discovery on an empty PATH is a typed BridgeNotFound.
    let prev = std::env::var_os("PATH");
    std::env::remove_var("PATH");
    let d = ToolPaths::discover();
    if let Some(p) = prev {
        std::env::set_var("PATH", p);
    }
    assert_eq!(d.unwrap_err(), VoleError::BridgeNotFound);
}

#[test]
fn time_base_of_import_matches_the_nut_stream() -> Result<(), VoleError> {
    if !ffmpeg_on_path() {
        eprintln!("skip: ffmpeg/ffprobe not on PATH");
        return Ok(());
    }
    let tools = ToolPaths::discover()?;
    let nut = make_nut_fixture(&tools, "tb", "yuv420p", PixelLayout::Yuv420, 8, 32, 16, 4);
    let imp = import(&nut)?;
    let st = &imp.nut.streams[0];
    let tb = TimeBase::new(st.time_base.0, st.time_base.1)?;
    let obs_tb = imp.video.observations()[0].pts().time_base();
    assert_eq!(obs_tb.numerator(), tb.numerator());
    assert_eq!(obs_tb.denominator(), tb.denominator());
    let _ = std::fs::remove_file(&nut);
    Ok(())
}
