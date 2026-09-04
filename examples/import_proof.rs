//! Phase V.1.3 evidence proof: the foreign ingest bridge (V.1 video
//! programme, brief §31–§40).
//!
//! Turns real (deterministically generated) media files into verified
//! canonical videos and measures the V.1.3 claims:
//!
//! 1. **Format matrix** — planar, semi-planar, and packed source pixel
//!    formats imported through the bridge; every observation verified
//!    against the independent framehash oracle and (for the planar cases)
//!    proven byte-exact against the authored frames.
//! 2. **Compressed sources** — an H.264 MP4 (lossy; oracle-exact +
//!    reproducible) and an FFV1 Matroska (lossless; authored ground truth).
//! 3. **The recorded manifest** — container/codec/geometry/pixel format/
//!    color facts captured as evidence, plus the exact bridge commands and
//!    tool versions.
//! 4. **Media → canonical → .vole** — the imported canonical video is
//!    proceduralized with the V.1.2 exact floor into a frozen-v2 `.vole`
//!    container, re-parsed, and re-materialized sample-for-sample: the full
//!    `ordinary file → canonical → procedural state` path of the mission,
//!    measured (raw canonical bytes vs `.vole` bytes).
//!
//! Run: `cargo run --release --example import_proof`

use std::time::Instant;

use vole_video::media::bridge::canonicalize;
use vole_video::media::bridge::run::{run_bounded, ChildLimits, ToolPaths};
use vole_video::media::bridge::{import_video, ImportOptions};
use vole_video::media::epoch::{EpochId, VideoEpoch};
use vole_video::media::ingest::encode_pictures_exact;
use vole_video::media::layout::PixelLayout;
use vole_video::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use vole_video::media::wire::{parse_multiplane, write_multiplane};
use vole_video::VoleError;

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("vole-v13-proof-{}-{name}", std::process::id()));
    p
}

fn epoch_for(layout: PixelLayout, depth: u8, w: u32, h: u32) -> VideoEpoch {
    use vole_video::media::color::ColorDescription;
    use vole_video::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
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

/// Deterministic authored canonical planes (used as ground truth and to
/// build the source-layout payloads).
fn authored_planes(layout: PixelLayout, depth: u8, w: u32, h: u32, f: usize) -> Vec<Plane> {
    let epoch = epoch_for(layout, depth, w, h);
    let mut planes = Vec::new();
    for p in 0..epoch.plane_count() {
        let (pw, ph) = epoch.plane_dimensions(p).unwrap();
        let tmpl = &epoch.planes()[p];
        let max = tmpl.bit_depth.max_sample();
        let n = (pw * ph) as usize;
        let values: Vec<u32> = (0..n)
            .map(|i| {
                let x = (i as u32) % pw;
                let y = (i as u32) / pw;
                (y * 7 + x * 3 + f as u32 * 11 + p as u32 * 5 + 1) % (max + 1)
            })
            .collect();
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
    planes
}

fn authored_source_payload(
    pix_fmt: &str,
    layout: PixelLayout,
    depth: u8,
    w: u32,
    h: u32,
    frames: usize,
) -> Vec<u8> {
    let mut out = Vec::new();
    for f in 0..frames {
        let planes = authored_planes(layout, depth, w, h, f);
        out.extend_from_slice(
            &canonicalize::repack_frame(pix_fmt, u64::from(w), u64::from(h), &planes).unwrap(),
        );
    }
    out
}

/// Wrap authored frames in a NUT file (rawvideo carrier).
#[allow(clippy::too_many_arguments)] // one fixture builder for the proof
fn make_nut(
    tools: &ToolPaths,
    name: &str,
    pix_fmt: &str,
    layout: PixelLayout,
    depth: u8,
    w: u32,
    h: u32,
    frames: usize,
) -> std::path::PathBuf {
    let raw = authored_source_payload(pix_fmt, layout, depth, w, h, frames);
    let yuv = tmp_path(&format!("{name}.yuv"));
    let nut = tmp_path(&format!("{name}.nut"));
    std::fs::write(&yuv, &raw).expect("write yuv");
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

fn import(
    file: &std::path::Path,
    tools: &ToolPaths,
) -> Result<vole_video::media::bridge::VerifiedImport, VoleError> {
    import_video(&ImportOptions {
        source: file.to_path_buf(),
        stream: None,
        tools: Some(tools.clone()),
        limits: Default::default(),
    })
}

fn hex32(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() -> Result<(), VoleError> {
    let t0 = Instant::now();
    let tools = ToolPaths::discover()?;
    println!("import proof: foreign ingest bridge (Phase V.1.3)");
    println!(
        "tools: {} | {}",
        tools.ffmpeg_version, tools.ffprobe_version
    );
    println!();

    // 1. Format matrix (planar + packed + semi-planar carriers).
    {
        let cases: &[(&str, PixelLayout, u8, u32, u32, usize)] = &[
            ("yuv420p", PixelLayout::Yuv420, 8, 18, 12, 6),
            ("yuv420p10le", PixelLayout::Yuv420, 10, 32, 16, 6),
            ("gray", PixelLayout::Gray, 8, 20, 14, 6),
            ("gray16le", PixelLayout::Gray, 16, 20, 14, 6),
            ("yuv444p", PixelLayout::Yuv444, 8, 16, 16, 6),
            ("yuv422p", PixelLayout::Yuv422, 8, 16, 16, 6),
            ("gbrp", PixelLayout::Gbr, 8, 16, 16, 6),
            ("nv12", PixelLayout::Yuv420, 8, 16, 16, 6),
            ("rgb24", PixelLayout::Rgb, 8, 12, 10, 6),
            ("yuyv422", PixelLayout::Yuv422, 8, 16, 16, 6),
        ];
        println!("1. format matrix: file -> canonical -> oracle-verified");
        println!(
            "   {:<12} {:>3} {:>3} {:>5} {:>9} {:>9} {:>12} {:>10}",
            "pix_fmt", "pl", "dp", "obs", "samples", "oracle", "ground truth", "seq blake3"
        );
        for (pix, layout, depth, w, h, frames) in cases {
            let nut = make_nut(&tools, pix, pix, *layout, *depth, *w, *h, *frames);
            let imp = import(&nut, &tools)?;
            let _ = std::fs::remove_file(&nut);
            // Ground truth: canonical observations equal the authored frames
            // (directly for planar carriers; via source-layout repack for
            // packed/semi-planar carriers).
            let mut label = "FAIL";
            let mut all_exact = true;
            for (k, obs) in imp.video.observations().iter().enumerate() {
                let want = authored_planes(*layout, *depth, *w, *h, k);
                match canonicalize::layout_kind(pix).expect("courted format") {
                    canonicalize::CarrierKind::Canonical => {
                        for (p, pl) in obs.planes().iter().enumerate() {
                            if pl.canonical_bytes() != want[p].canonical_bytes() {
                                all_exact = false;
                            }
                        }
                    }
                    canonicalize::CarrierKind::SourceRepacked => {
                        let got = canonicalize::repack_frame(
                            pix,
                            u64::from(*w),
                            u64::from(*h),
                            obs.planes(),
                        )
                        .unwrap();
                        let exp =
                            canonicalize::repack_frame(pix, u64::from(*w), u64::from(*h), &want)
                                .unwrap();
                        if got != exp {
                            all_exact = false;
                        }
                    }
                }
            }
            if all_exact {
                label = match canonicalize::layout_kind(pix).expect("courted format") {
                    canonicalize::CarrierKind::Canonical => "authored",
                    canonicalize::CarrierKind::SourceRepacked => "repack-exact",
                };
            }
            println!(
                "   {:<12} {:>3} {:>3} {:>5} {:>9} {:>9} {:>12} {}",
                pix,
                imp.epoch.plane_count(),
                depth,
                imp.video.observation_count(),
                imp.checks.sample_bytes,
                imp.checks.oracle_frames,
                label,
                &hex32(&imp.sequence_blake3)[..10]
            );
        }
        println!();
    }

    // 2. Compressed sources.
    {
        println!("2. compressed sources (oracle-exact + reproducible)");
        // H.264 MP4.
        let mp4 = tmp_path("h264.mp4");
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
                &mp4.display().to_string(),
            ],
            &ChildLimits::default(),
        )?;
        assert_eq!(out.code, Some(0));
        let a = import(&mp4, &tools)?;
        let b = import(&mp4, &tools)?;
        assert_eq!(a.sequence_blake3, b.sequence_blake3);
        println!(
            "   h264 mp4: container {}, codec {} ({}), {}x{} {}, {} obs, {} frames oracle-verified, {} sample bytes",
            a.manifest.format.container.as_deref().unwrap_or("?"),
            a.manifest.streams[0].codec_name.as_deref().unwrap_or("?"),
            a.manifest.streams[0].profile.as_deref().unwrap_or("?"),
            a.epoch.width(),
            a.epoch.height(),
            a.manifest.streams[0].pix_fmt.as_deref().unwrap_or("?"),
            a.video.observation_count(),
            a.checks.verified_frames,
            a.checks.sample_bytes
        );
        println!(
            "   reproducible: import twice == identical sequence digests (blake3 {} sha256 {})",
            &hex32(&a.sequence_blake3)[..16],
            &hex32(&a.sequence_sha256)[..16]
        );
        let _ = std::fs::remove_file(&mp4);

        // FFV1 Matroska (lossless): authored ground truth exact.
        let nut10 = make_nut(
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
        let imp = import(&mkv, &tools)?;
        let mut exact = true;
        for (k, obs) in imp.video.observations().iter().enumerate() {
            let want = authored_planes(PixelLayout::Yuv420, 10, 32, 16, k);
            for (p, pl) in obs.planes().iter().enumerate() {
                if pl.canonical_bytes() != want[p].canonical_bytes() {
                    exact = false;
                }
            }
        }
        println!(
            "   ffv1 mkv: 10-bit lossless round trip {} ({} obs, {} sample bytes, depth {} preserved)",
            if exact { "EXACT == authored frames" } else { "FAILED" },
            imp.video.observation_count(),
            imp.checks.sample_bytes,
            imp.epoch.planes()[0].bit_depth.bits()
        );
        assert!(exact);
        let _ = std::fs::remove_file(&nut10);
        let _ = std::fs::remove_file(&mkv);
        println!();
    }

    // 3. Recorded manifest + bridge commands.
    {
        let nut = make_nut(
            &tools,
            "manifest",
            "yuv420p",
            PixelLayout::Yuv420,
            8,
            32,
            16,
            3,
        );
        let imp = import(&nut, &tools)?;
        println!("3. recorded evidence (one import)");
        let st = &imp.manifest.streams[0];
        println!(
            "   ffprobe: container {:?}, codec {:?}, {}x{} pix_fmt {:?}, color {:?}/{:?}/{:?}/{:?}, tb {:?}",
            imp.manifest.format.container,
            st.codec_name,
            st.width.unwrap_or(0),
            st.height.unwrap_or(0),
            st.pix_fmt,
            st.color_primaries,
            st.color_transfer,
            st.color_space,
            st.color_range,
            st.time_base
        );
        println!(
            "   nut stream: tb {:?} (container ticks), {} frames",
            imp.nut.time_bases,
            imp.nut.frames.len()
        );
        println!(
            "   oracle: tb {:?}, {} records",
            imp.oracle.time_base,
            imp.oracle.records.len()
        );
        println!("   bridge commands recorded (argv, never shell strings):");
        for cmd in &imp.commands {
            println!("     {}", cmd.join(" "));
        }
        let _ = std::fs::remove_file(&nut);
        println!();
    }

    // 4. Media -> canonical -> frozen-v2 .vole (library path, exact floor).
    {
        let nut = make_nut(
            &tools,
            "v2path",
            "yuv420p10le",
            PixelLayout::Yuv420,
            10,
            32,
            16,
            6,
        );
        let imp = import(&nut, &tools)?;
        let _ = std::fs::remove_file(&nut);
        let epoch = &imp.epoch;
        // Rebuild pictures from the canonical observations.
        let pictures: Vec<_> = imp
            .video
            .observations()
            .iter()
            .map(|o| {
                vole_video::media::picture::Picture::from_planes(epoch, o.planes().to_vec())
                    .expect("obs planes match epoch")
            })
            .collect();
        let raw_bytes: u64 = pictures.iter().map(|p| p.total_bytes()).sum();
        let program = encode_pictures_exact(epoch, &pictures)?;
        let v2 = write_multiplane(&program)?;
        // Re-parse + re-materialize: sample-exact through the frozen grammar.
        let parsed = parse_multiplane(&v2)?;
        for (idx, want) in pictures.iter().enumerate() {
            let got = parsed.materialize_observation(idx as u64)?;
            for p in 0..epoch.plane_count() {
                assert_eq!(
                    got.plane(p).unwrap().canonical_bytes(),
                    want.plane(p).unwrap().canonical_bytes(),
                    "v2 re-materialization frame {idx} plane {p}"
                );
            }
        }
        println!("4. media -> canonical -> .vole (library path, V.1.2 floor + frozen v2 wire)");
        println!(
            "   canonical raw: {raw_bytes} B over {} obs | exact floor + v2 container: {} B ({:.2}x)",
            pictures.len(),
            v2.len(),
            raw_bytes as f64 / v2.len() as f64
        );
        println!(
            "   v2 parse -> materialize: every plane sample-exact against the imported canonical observations"
        );
        println!();
    }

    println!(
        "import proof: OK (format matrix + compressed sources + recorded evidence + media->canonical->.vole) in {:.1} s",
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}
