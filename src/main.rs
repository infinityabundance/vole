//! VOLE command-line interface.
//!
//! Subcommands (format v1, profile 1):
//!
//! ```text
//! vole demo moving-rect [out.vole]
//! vole decode <in.vole> [outdir]
//! vole verify <in.vole>
//! vole bench
//! ```

use std::io::Write as _;

use vole_video::{decoder, demo, error::VoleError, format::ParsedStream, pixel::Canvas};

fn main() -> Result<(), VoleError> {
    let mut a = std::env::args().skip(1);
    let cmd = a.next();
    match cmd.as_deref() {
        Some("demo") => cmd_demo(a),
        Some("decode") => cmd_decode(a),
        Some("verify") => cmd_verify(a),
        Some("bench") => cmd_bench(),
        Some("statics") => cmd_statics(),
        other => {
            eprintln!("vole: unknown or missing subcommand: {:?}", other);
            eprintln!("usage: vole <demo|decode|verify|bench|statics> ...");
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

fn cmd_verify(mut a: impl Iterator<Item = String>) -> Result<(), VoleError> {
    let infile = a
        .next()
        .ok_or(VoleError::ApiConstraint("verify needs input"))?;
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
    Ok(())
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
