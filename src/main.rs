//! VOLE command-line interface.
//!
//! Subcommands (format v1, profile 1):
//!
//! ```text
//! vole demo moving-rect [out.vole]
//! vole decode <in.vole> [outdir]
//! vole verify <in.vole> [--archive m.volea]
//! vole archive <in.vole> [out.volea]
//! vole optimize <in.vole> <out.vole>
//! vole bench
//! ```

use std::io::Write as _;

use vole_video::{decoder, demo, error::VoleError, format::ParsedStream, inverse, pixel::Canvas};

fn main() -> Result<(), VoleError> {
    let mut a = std::env::args().skip(1);
    let cmd = a.next();
    match cmd.as_deref() {
        Some("demo") => cmd_demo(a),
        Some("encode") => cmd_encode(a),
        Some("decode") => cmd_decode(a),
        Some("verify") => cmd_verify(a),
        Some("archive") => cmd_archive(a),
        Some("optimize") => cmd_optimize(a),
        Some("bench") => cmd_bench(),
        Some("statics") => cmd_statics(),
        other => {
            eprintln!("vole: unknown or missing subcommand: {:?}", other);
            eprintln!("usage: vole <demo|encode|decode|verify|archive|optimize|bench|statics> ...");
            if other.is_some() {
                Err(VoleError::ApiConstraint("unknown subcommand"))
            } else {
                std::process::exit(2);
            }
        }
    }
}

fn cmd_demo(mut a: impl Iterator<Item = String>) -> Result<(), VoleError> {
    let kind = a.next().unwrap_or_else(|| "moving-rect".into());
    match kind.as_str() {
        "moving-rect" => {
            let outfile = a.next().unwrap_or_else(|| "court-moving-rect.vole".into());
            let court = demo::MovingRectCourt::default();
            let bytes = court.vole()?;
            std::fs::write(&outfile, &bytes)
                .map_err(|_| VoleError::ApiConstraint("write failed"))?;
            let frames = court.materialize_and_verify()?;
            println!(
                "wrote {} ({} frames, {} bytes)",
                outfile,
                frames.len(),
                bytes.len()
            );
            let raw_size = court.raw_bytes_all();
            println!(
                "frames={} byte_stream_size={} raw_first_frame={} raw_all_frames={}",
                frames.len(),
                bytes.len(),
                u64::from(court.width) * u64::from(court.height),
                raw_size
            );
            let _ = court.reference_raw();
            Ok(())
        }
        _ => Err(VoleError::ApiConstraint("unknown demo")),
    }
}

fn cmd_encode(mut a: impl Iterator<Item = String>) -> Result<(), VoleError> {
    // vole encode --width W --height H [--frames N] in.raw out.vole
    // The input is a concatenated Gray8 sequence of full frames (raster-origin
    // input; Phase-G exhaustive inverse proceduralization).
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut frames_n: Option<u64> = None;
    let mut args: Vec<String> = Vec::new();
    while let Some(s) = a.next() {
        match s.as_str() {
            "--width" => width = Some(parse_u32(&mut a)?),
            "--height" => height = Some(parse_u32(&mut a)?),
            "--frames" => frames_n = Some(parse_u64(&mut a)?),
            _ => args.push(s),
        }
    }
    let (w, h) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        _ => {
            return Err(VoleError::ApiConstraint(
                "encode needs --width and --height",
            ))
        }
    };
    let infile = args
        .first()
        .ok_or(VoleError::ApiConstraint("encode needs an input .raw"))?;
    let outfile = args
        .get(1)
        .ok_or(VoleError::ApiConstraint("encode needs an output .vole"))?;
    let data = std::fs::read(infile).map_err(|_| VoleError::ApiConstraint("read failed"))?;
    let per = u64::from(w) * u64::from(h);
    let want = frames_n
        .map(|n| n.saturating_mul(per))
        .unwrap_or(data.len() as u64);
    if per == 0 || want != data.len() as u64 || data.is_empty() {
        return Err(VoleError::ApiConstraint(
            "input length must be frames x w x h",
        ));
    }
    let frames = inverse::frames_from_raw(&data, w, h)?;
    let report = inverse::encode_frames(&frames, &inverse::EncodeOptions::default())?;
    std::fs::write(outfile, &report.vole).map_err(|_| VoleError::ApiConstraint("write failed"))?;
    println!(
        "vole encode: {}x{} frames={} -> {} ({} bytes)",
        w,
        h,
        frames.len(),
        outfile,
        report.vole.len()
    );
    println!(
        "  raw_raster={}B procedural_fraction={:.3} background={} exact={}",
        report.raw_raster_bytes,
        report.cost.procedural_fraction(),
        report.background,
        report.exact
    );
    Ok(())
}

fn parse_u32(a: &mut impl Iterator<Item = String>) -> Result<u32, VoleError> {
    a.next()
        .ok_or(VoleError::ApiConstraint("missing value"))?
        .parse()
        .map_err(|_| VoleError::ApiConstraint("expected an integer"))
}

fn parse_u64(a: &mut impl Iterator<Item = String>) -> Result<u64, VoleError> {
    a.next()
        .ok_or(VoleError::ApiConstraint("missing value"))?
        .parse()
        .map_err(|_| VoleError::ApiConstraint("expected an integer"))
}

fn cmd_decode(mut a: impl Iterator<Item = String>) -> Result<(), VoleError> {
    let infile = a
        .next()
        .ok_or(VoleError::ApiConstraint("decode needs input"))?;
    let outdir = a.next().unwrap_or_else(|| "vole-frames".into());
    let bytes = std::fs::read(&infile).map_err(|_| VoleError::ApiConstraint("read failed"))?;
    let frames = frames_of(&bytes)?;
    std::fs::create_dir_all(&outdir).map_err(|_| VoleError::ApiConstraint("mkdir failed"))?;
    for (i, f) in frames.iter().enumerate() {
        let name = format!("{}/frame-{:04}.gray", outdir, i);
        let mut fh =
            std::fs::File::create(&name).map_err(|_| VoleError::ApiConstraint("create failed"))?;
        fh.write_all(f.as_slice())
            .map_err(|_| VoleError::ApiConstraint("write failed"))?;
    }
    println!("decoded {} frames into {}", frames.len(), outdir);
    Ok(())
}

/// Decode and validate a stream, returning its materialized frames.
fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed: ParsedStream = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

fn cmd_optimize(mut a: impl Iterator<Item = String>) -> Result<(), VoleError> {
    // vole optimize <in.vole> <out.vole>
    let infile = a
        .next()
        .ok_or(VoleError::ApiConstraint("optimize needs an input .vole"))?;
    let outfile = a
        .next()
        .ok_or(VoleError::ApiConstraint("optimize needs an output .vole"))?;
    let bytes = std::fs::read(&infile).map_err(|_| VoleError::ApiConstraint("read failed"))?;
    let report = vole_video::optimize::optimize_stream(&bytes)?;
    if report.stream.len() < bytes.len() {
        std::fs::write(&outfile, &report.stream)
            .map_err(|_| VoleError::ApiConstraint("write failed"))?;
    }
    println!(
        "vole optimize: {} -> {} ({} B -> {} B, saved {} B) exact={} rewrites=[{}]",
        infile,
        outfile,
        bytes.len(),
        report.stream.len(),
        bytes.len().saturating_sub(report.stream.len()),
        report.exact,
        report.rewrites.join(" ")
    );
    if report.stream.len() >= bytes.len() && report.rewrites.is_empty() {
        println!("  (fixpoint: no improving rewrite exists; input preserved)");
    }
    Ok(())
}

fn cmd_verify(mut a: impl Iterator<Item = String>) -> Result<(), VoleError> {
    let mut infile: Option<String> = None;
    let mut archive_path: Option<String> = None;
    while let Some(s) = a.next() {
        match s.as_str() {
            "--archive" => archive_path = a.next(),
            other => {
                if infile.is_none() {
                    infile = Some(other.to_string());
                } else {
                    return Err(VoleError::ApiConstraint("verify takes one input"));
                }
            }
        }
    }
    let infile = infile.ok_or(VoleError::ApiConstraint("verify needs input"))?;
    let bytes = std::fs::read(&infile).map_err(|_| VoleError::ApiConstraint("read failed"))?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    println!(
        "ok {}x{} frames={} bytes={}",
        parsed.width(),
        parsed.height(),
        frames.len(),
        bytes.len()
    );
    if let Some(mp) = archive_path {
        let mbytes =
            std::fs::read(&mp).map_err(|_| VoleError::ApiConstraint("read manifest failed"))?;
        let manifest = vole_video::archive::decode(&mbytes)?;
        let report = vole_video::archive::verify(&bytes, &manifest, true)?;
        println!(
            "archive verify {}: self_desc={} structural={} records_checked={} objects={} deep_frames={} first_frame_div={:?}",
            archive_status(&report),
            report.self_description_ok,
            report.structural_ok,
            report.records_checked,
            report.objects_ok,
            report.frames_checked,
            report.first_frame_divergence,
        );
        if let Some(i) = report.first_bad_record {
            let rec = &manifest.records[i as usize];
            println!(
                "  first bad record #{i}: kind={} offset={} length={} t={:?} id={:?}",
                rec.kind.label(),
                rec.offset,
                rec.length,
                rec.t,
                rec.id
            );
        }
        if let Some(field) = report.mismatch_field {
            println!("  self-description mismatch: {}", field.label());
        }
        if report.status != vole_video::archive::VerifyStatus::Complete {
            return Err(VoleError::ApiConstraint(
                "archive verification failed (see report)",
            ));
        }
    }
    Ok(())
}

fn archive_status(report: &vole_video::archive::VerifyReport) -> &'static str {
    match report.status {
        vole_video::archive::VerifyStatus::Complete => "COMPLETE",
        vole_video::archive::VerifyStatus::SelfDescriptionMismatch => "SELF_DESCRIPTION_MISMATCH",
        vole_video::archive::VerifyStatus::StructuralMismatch => "STRUCTURAL_MISMATCH",
        vole_video::archive::VerifyStatus::StreamDigestMismatch => "STREAM_DIGEST_MISMATCH",
        vole_video::archive::VerifyStatus::ObjectMismatch => "OBJECT_MISMATCH",
        vole_video::archive::VerifyStatus::FrameDivergence => "FRAME_DIVERGENCE",
    }
}

fn cmd_archive(mut a: impl Iterator<Item = String>) -> Result<(), VoleError> {
    // vole archive <in.vole> [out.volea]
    let infile = a
        .next()
        .ok_or(VoleError::ApiConstraint("archive needs an input .vole"))?;
    let outfile = a.next().unwrap_or_else(|| format!("{infile}.volea"));
    let bytes = std::fs::read(&infile).map_err(|_| VoleError::ApiConstraint("read failed"))?;
    let manifest = vole_video::archive::ArchiveManifest::build(&bytes)?;
    let wire = vole_video::archive::encode(&manifest)?;
    std::fs::write(&outfile, &wire).map_err(|_| VoleError::ApiConstraint("write failed"))?;
    let sd = &manifest.stream;
    let overhead = if bytes.is_empty() {
        0.0
    } else {
        wire.len() as f64 * 100.0 / bytes.len() as f64
    };
    println!(
        "vole archive: {} -> {} ({} bytes + {} B manifest, {overhead:.1}% overhead)",
        infile,
        outfile,
        bytes.len(),
        wire.len()
    );
    println!(
        "  self-description: format_v{} universe={} profile={} features={:#x} {}x{} gray8 frames={} stream_digest={}",
        sd.format_version,
        sd.universe_id,
        sd.limit_profile,
        sd.feature_bits,
        sd.width,
        sd.height,
        sd.frame_count,
        hex_prefix(&sd.stream_digest)
    );
    println!(
        "  records={} (intervals={}) objects={} frame_hashes={} checkpoint_digest={}",
        manifest.records.len(),
        manifest
            .records
            .iter()
            .filter(|r| r.kind == vole_video::archive::RecordKind::Interval)
            .count(),
        manifest.objects.len(),
        manifest.frame_hashes.len(),
        hex_prefix(&manifest.checkpoint_digest)
    );
    Ok(())
}

fn hex_prefix(d: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(16);
    for b in &d[..8] {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn cmd_statics() -> Result<(), VoleError> {
    // Persistent static object across N intervals: an UNCHANGED lane estimate.
    let court = demo::StaticSceneCourt::default();
    let (stream, frames, raw) = court.account()?;
    let per = stream as f64 / frames as f64;
    println!(
        "static: frames={} stream={}B raw_all={}B per_frame_amortized={:.3}B",
        frames, stream, raw, per
    );
    Ok(())
}

fn cmd_bench() -> Result<(), VoleError> {
    let court = demo::MovingRectCourt::default();
    let start = std::time::Instant::now();
    let bytes = court.vole()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "translate court: frames={} stream={}B raw_all={}B decode_ms={:.3}",
        frames.len(),
        bytes.len(),
        court.raw_bytes_all(),
        ms
    );
    Ok(())
}
