//! Foreign ingest bridge — Phase V.1.3 (V.1 video programme, brief §31–§40,
//! §217–§220).
//!
//! The bridge turns an ordinary media file into a **canonical video**:
//!
//! ```text
//! foreign media
//!   → ffprobe manifest (evidence; §38)
//!   → ffmpeg decode → raw video in a NUT PIPE (§36)
//!       → narrow NUT reader (§37) → per-frame payloads + exact PTS
//!   → framehash oracle (independent per-frame SHA-256 over the tight rows;
//!     §39)
//!   → canonicalizer: pixel format → canonical planes (§18)
//!   → oracle verification: VOLE's canonical bytes must reproduce the
//!     oracle's digest per frame, or IMPORT FAIL
//!   → a validated [`CanonicalVideo`] plus the recorded manifest, commands,
//!     tool versions, and sequence digests (§40).
//! ```
//!
//! Non-normative by design (§31): the bridge runs only at import; FFmpeg is a
//! subprocess (never a crate dependency), invoked with individual arguments
//! (§32), software-decoded (§33), without silent transforms (§34), and with
//! the decoded pixel format retained (no conversion — the manifest and the
//! NUT stream header must agree). Child processes are bounded and killed
//! cleanly on wall-clock or byte caps (§217). Network protocols are disabled
//! by default (§218). Frame ordering is presentation order, never the source
//! container's decode order (§11).

pub mod canonicalize;
pub mod crc;
pub mod framehash;
pub mod nut;
pub mod probe;
pub mod run;
pub mod sha256;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::VoleError;
use crate::media::color::ColorDescription;
use crate::media::epoch::{CanonicalVideo, CanonicalVideoObservation, EpochId, VideoEpoch};
use crate::media::layout::PixelLayout;
use crate::media::meta::SampleAspectRatio;
use crate::media::plane::BitDepth;
use crate::media::time::{Duration as VDuration, Pts, TimeBase};
use canonicalize::unpack_frame;
use framehash::{run_framehash, FramehashResult};
use nut::{parse_nut, NutStream};
use probe::{probe_media, ProbeManifest};
use run::{run_bounded, ChildLimits, ToolPaths};

/// Import budgets (the §212/§217 envelope applied to the foreign bridge).
#[derive(Debug, Clone)]
pub struct ImportLimits {
    /// Per-child wall-clock budget.
    pub wall: Duration,
    /// Maximum NUT pipe bytes accepted.
    pub nut_bytes: u64,
    /// Maximum accepted observations (bounded frame count).
    pub max_frames: u64,
}

impl Default for ImportLimits {
    fn default() -> Self {
        ImportLimits {
            wall: Duration::from_secs(300),
            nut_bytes: 1 << 30,
            max_frames: 4 << 20,
        }
    }
}

/// Import options.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// The local media file (network URLs are refused by the tool whitelist).
    pub source: PathBuf,
    /// Explicit video stream index; `None` = deterministic auto selection.
    pub stream: Option<u64>,
    /// Pre-resolved tools (discovered from the environment when `None`).
    pub tools: Option<ToolPaths>,
    /// Budgets.
    pub limits: ImportLimits,
}

/// The verified facts of one import.
#[derive(Debug, Clone)]
pub struct ImportChecks {
    /// Frame count reported by the oracle.
    pub oracle_frames: u64,
    /// Frames verified (canonical bytes == oracle digest per frame).
    pub verified_frames: u64,
    /// Total canonical sample bytes verified.
    pub sample_bytes: u64,
    /// Whether every oracle PTS matched the NUT-derived PTS exactly
    /// (rational equality on each frame's own time base).
    pub pts_matched: bool,
}

/// The full result of a verified import.
#[derive(Debug, Clone)]
pub struct VerifiedImport {
    /// Resolved tool paths + versions.
    pub tools: ToolPaths,
    /// Every bridge command recorded as argv (evidence; §35/§38).
    pub commands: Vec<Vec<String>>,
    /// The ffprobe manifest (evidence; never a playback dependency).
    pub manifest: ProbeManifest,
    /// The parsed NUT stream (frame payloads + PTS).
    pub nut: NutStream,
    /// The framehash oracle records.
    pub oracle: FramehashResult,
    /// The epoch describing the imported video.
    pub epoch: VideoEpoch,
    /// The validated canonical video (presentation order).
    pub video: CanonicalVideo,
    /// Verification facts.
    pub checks: ImportChecks,
    /// VOLE sequence digest (BLAKE3, domain-separated; §40).
    pub sequence_blake3: [u8; 32],
    /// VOLE sequence digest (SHA-256 over the same canonical record).
    pub sequence_sha256: [u8; 32],
}

/// Import a media file into a verified canonical video.
pub fn import_video(opts: &ImportOptions) -> Result<VerifiedImport, VoleError> {
    let tools = match &opts.tools {
        Some(t) => t.clone(),
        None => ToolPaths::discover()?,
    };
    let mut commands: Vec<Vec<String>> = Vec::new();

    // 1. Probe (evidence manifest; §38).
    let manifest = probe_media(&tools, &opts.source, opts.stream)?;
    commands.push(probe_command(&tools, &opts.source));
    let stream = manifest
        .streams
        .iter()
        .find(|s| s.index == manifest.chosen)
        .ok_or(VoleError::BridgeProbeFailed)?;
    let width = stream.width.ok_or(VoleError::BridgeProbeFailed)?;
    let height = stream.height.ok_or(VoleError::BridgeProbeFailed)?;
    let pix_fmt = stream
        .pix_fmt
        .as_deref()
        .ok_or(VoleError::BridgeProbeFailed)?;
    if width == 0 || height == 0 || width > u64::from(u32::MAX) || height > u64::from(u32::MAX) {
        return Err(VoleError::DimensionTooLarge);
    }

    // 2. The retained pixel format must be canonicalizable (never silently
    //    converted; §18/§35).
    let (layout, depth) = canonicalize::layout_and_depth(pix_fmt)?;
    let source_bytes = canonicalize::expected_canonical_bytes(pix_fmt, width, height)?;
    if source_bytes == 0 {
        return Err(VoleError::UnsupportedPixelLayout);
    }

    // 3. Framehash oracle (independent per-frame digests; §39).
    let oracle = run_framehash(&tools, &opts.source, manifest.chosen)?;
    commands.push(framehash_command(&tools, &opts.source, manifest.chosen));
    if oracle.records.len() as u64 > opts.limits.max_frames {
        return Err(VoleError::BridgeOutputLimit);
    }
    if oracle.records.is_empty() {
        return Err(VoleError::BridgeDecodeFailed);
    }

    // 4. NUT pipe decode + narrow parse.
    let nut_out = run_bounded(
        &tools.ffmpeg,
        &[
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file",
            "-i",
            &opts.source.display().to_string(),
            "-map",
            &format!("0:v:{}", manifest.chosen),
            "-an",
            "-sn",
            "-dn",
            "-fps_mode",
            "passthrough",
            "-c:v",
            "rawvideo",
            "-f",
            "nut",
            "-write_index",
            "0",
            "-",
        ],
        &ChildLimits {
            wall: opts.limits.wall,
            stdout_bytes: opts.limits.nut_bytes + (1 << 20),
            ..ChildLimits::default()
        },
    )?;
    commands.push(nut_command(&tools, &opts.source, manifest.chosen));
    if nut_out.code != Some(0) {
        return Err(VoleError::BridgeDecodeFailed);
    }
    let nut = parse_nut(&nut_out.stdout)?;
    if nut.streams.len() != 1 || nut.streams[0].class != 0 {
        return Err(VoleError::UnsupportedFeature);
    }
    let st = &nut.streams[0];
    if u64::from(st.width) != width || u64::from(st.height) != height {
        return Err(VoleError::GeometryMismatch);
    }
    let checks = verify_frames(pix_fmt, width, height, &oracle, &nut)?;
    if checks.oracle_frames as usize != nut.frames.len() {
        return Err(VoleError::CanonicalHashMismatch);
    }

    // 5. Epoch from the manifest interpretation.
    let epoch = build_epoch(
        &manifest,
        stream,
        layout,
        depth,
        width as u32,
        height as u32,
    )?;
    let (tbn, tbd) = st.time_base;
    let tb = TimeBase::new(tbn, tbd)?;

    // 6. Observations in presentation order with exact deltas.
    let mut obs_vec = Vec::with_capacity(nut.frames.len());
    for (i, frame) in nut.frames.iter().enumerate() {
        let planes = unpack_frame(pix_fmt, width, height, &frame.payload)?;
        let pts = Pts::new(frame.pts, tb);
        let dur = if i + 1 < nut.frames.len() {
            let delta = nut.frames[i + 1]
                .pts
                .checked_sub(frame.pts)
                .ok_or(VoleError::TimeNotRepresentable)?;
            Some(VDuration::new(delta, tb)?)
        } else {
            None
        };
        obs_vec.push(CanonicalVideoObservation::new(&epoch, pts, dur, planes)?);
    }
    let video = CanonicalVideo::new(vec![epoch.clone()], obs_vec)?;

    // 7. VOLE domain-separated sequence digests (§40).
    let record = canonical_record(&epoch, video.observations());
    let sequence_blake3 = crate::integr::digest(&record);
    let sequence_sha256 = sha256::sha256(&record);

    Ok(VerifiedImport {
        tools,
        commands,
        manifest,
        nut,
        oracle,
        epoch,
        video,
        checks,
        sequence_blake3,
        sequence_sha256,
    })
}

/// Verify the NUT-derived frames against the independent framehash oracle:
/// per frame the source-layout payload must be byte-exact to the oracle's
/// digest and size, and every present PTS must agree exactly (rational
/// equality across the two independent decode time bases). Any disagreement
/// is [`VoleError::CanonicalHashMismatch`] (IMPORT FAIL; §39).
///
/// Also public so recorded evidence can be re-verified offline (V.1.19+
/// verification tooling) without re-running the bridge.
pub fn verify_frames(
    pix_fmt: &str,
    width: u64,
    height: u64,
    oracle: &FramehashResult,
    nut: &NutStream,
) -> Result<ImportChecks, VoleError> {
    let source_bytes = canonicalize::expected_canonical_bytes(pix_fmt, width, height)?;
    if nut.frames.len() as u64 != oracle.records.len() as u64 {
        return Err(VoleError::CanonicalHashMismatch);
    }
    if nut.frames.is_empty() {
        return Err(VoleError::BridgeDecodeFailed);
    }
    let (oracle_tb_num, oracle_tb_den) = oracle.time_base;
    let mut samples = 0u64;
    let mut pts_matched = true;
    for (i, frame) in nut.frames.iter().enumerate() {
        if frame.payload.len() as u64 != source_bytes {
            return Err(VoleError::CanonicalHashMismatch);
        }
        let rec = &oracle.records[i];
        if rec.size != source_bytes {
            return Err(VoleError::CanonicalHashMismatch);
        }
        let planes = unpack_frame(pix_fmt, width, height, &frame.payload)?;
        // Re-pack to the source layout and compare with the oracle digest:
        // byte-exact round trip proves the canonical bytes carry exactly the
        // decoded samples FFmpeg hashed independently.
        let repacked = canonicalize::repack_frame(pix_fmt, width, height, &planes)?;
        let digest = sha256::sha256(&repacked);
        if digest != rec.sha256 {
            return Err(VoleError::CanonicalHashMismatch);
        }
        // PTS rational equality between the two independent decode paths.
        if let Some(opts_pts) = rec.pts {
            let st = &nut.streams[frame.stream as usize];
            let (tbn, tbd) = st.time_base;
            let a = i128::from(frame.pts) * i128::from(tbn) * i128::from(oracle_tb_den);
            let b = i128::from(opts_pts) * i128::from(oracle_tb_num) * i128::from(tbd);
            if a != b {
                pts_matched = false;
            }
        }
        for p in &planes {
            samples += p.sample_count() * p.storage().bytes_per_sample();
        }
    }
    if !pts_matched {
        return Err(VoleError::CanonicalHashMismatch);
    }
    Ok(ImportChecks {
        oracle_frames: nut.frames.len() as u64,
        verified_frames: nut.frames.len() as u64,
        sample_bytes: samples,
        pts_matched: true,
    })
}

/// Build the epoch for the imported video from the manifest's interpretation.
fn build_epoch(
    manifest: &ProbeManifest,
    stream: &probe::StreamInfo,
    layout: PixelLayout,
    depth: u8,
    width: u32,
    height: u32,
) -> Result<VideoEpoch, VoleError> {
    let color = ColorDescription::new(
        stream.primaries(),
        stream.transfer(),
        stream.matrix(),
        stream.color_range(),
        stream.chroma_location(),
    );
    let sar = match stream.sar() {
        Some((n, d)) => SampleAspectRatio::new(n, d)?,
        None => SampleAspectRatio::square(),
    };
    let orientation = stream.orientation();
    let field = stream.field_structure();
    let epoch = VideoEpoch::new_uniform(
        EpochId(0),
        width,
        height,
        layout,
        BitDepth::new(depth)?,
        color,
        sar,
        orientation,
        field,
    )?;
    let _ = manifest;
    Ok(epoch)
}

/// The domain-separated canonical record (§40): epoch description + layout +
/// color + per-observation (PTS, duration, plane geometry, canonical sample
/// bytes).
fn canonical_record(epoch: &VideoEpoch, obs: &[CanonicalVideoObservation]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"VOLE-CANONICAL-v1\0");
    out.extend_from_slice(&epoch.width().to_le_bytes());
    out.extend_from_slice(&epoch.height().to_le_bytes());
    let mut layout = vec![0u8];
    layout_code(epoch.layout(), &mut layout);
    out.extend_from_slice(&layout);
    out.push(epoch.planes().len() as u8);
    for p in epoch.planes() {
        out.push(component_byte(p.component));
        out.push(p.bit_depth.bits());
        out.push(p.subsample_x);
        out.push(p.subsample_y);
    }
    let c = epoch.color();
    out.push(color_byte(c.primaries()));
    out.push(color_byte(c.transfer()));
    out.push(color_byte(c.matrix()));
    out.push(color_byte(c.range()));
    out.push(color_byte(c.chroma_location()));
    for o in obs {
        let pts = o.pts();
        out.extend_from_slice(&pts.value().to_le_bytes());
        out.extend_from_slice(&pts.time_base().numerator().to_le_bytes());
        out.extend_from_slice(&pts.time_base().denominator().to_le_bytes());
        match o.duration() {
            Some(d) => {
                out.push(1);
                out.extend_from_slice(&d.value().to_le_bytes());
            }
            None => out.push(0),
        }
        for plane in o.planes() {
            out.extend_from_slice(&plane.canonical_bytes());
        }
    }
    out
}

fn layout_code(l: PixelLayout, out: &mut Vec<u8>) {
    use crate::media::layout::PixelLayout::*;
    let code = match l {
        Gray => 1u8,
        Yuv400 => 2,
        Yuv420 => 3,
        Yuv422 => 4,
        Yuv444 => 5,
        Yuva420 => 6,
        Yuva444 => 7,
        Gbr => 8,
        Gbra => 9,
        Rgb => 10,
        Bgr => 11,
        Rgba => 12,
        Bgra => 13,
        Argb => 14,
        Abgr => 15,
        Indexed => 16,
    };
    out.push(code);
}

fn component_byte(c: crate::media::layout::Component) -> u8 {
    use crate::media::layout::Component::*;
    match c {
        Y => 1,
        Cb => 2,
        Cr => 3,
        R => 4,
        G => 5,
        B => 6,
        A => 7,
        Gray => 8,
        Index => 9,
        Other(_) => 0,
    }
}

fn color_byte<T: ColorCode>(c: T) -> u8 {
    c.code()
}

trait ColorCode {
    fn code(&self) -> u8;
}

impl ColorCode for crate::media::color::ColorPrimaries {
    fn code(&self) -> u8 {
        use crate::media::color::ColorPrimaries::*;
        match self {
            Unspecified => 0,
            Bt709 => 1,
            Bt470M => 2,
            Bt470Bg => 3,
            Smpte170M => 4,
            Smpte240M => 5,
            Film => 6,
            Bt2020 => 7,
        }
    }
}

impl ColorCode for crate::media::color::TransferCharacteristic {
    fn code(&self) -> u8 {
        use crate::media::color::TransferCharacteristic::*;
        match self {
            Unspecified => 0,
            Bt709 => 1,
            Gamma22 => 2,
            Gamma28 => 3,
            Smpte170M => 4,
            Smpte240M => 5,
            Linear => 6,
            Srgb => 7,
            Bt2020_10 => 8,
            Bt2020_12 => 9,
            Smpte2084 => 10,
            AribStdB67 => 11,
        }
    }
}

impl ColorCode for crate::media::color::MatrixCoefficients {
    fn code(&self) -> u8 {
        use crate::media::color::MatrixCoefficients::*;
        match self {
            Unspecified => 0,
            Identity => 1,
            Bt709 => 2,
            Smpte170M => 3,
            Smpte240M => 4,
            YcgCo => 5,
            Bt2020Ncl => 6,
            Bt2020Cl => 7,
        }
    }
}

impl ColorCode for crate::media::color::ColorRange {
    fn code(&self) -> u8 {
        use crate::media::color::ColorRange::*;
        match self {
            Unspecified => 0,
            Limited => 1,
            Full => 2,
        }
    }
}

impl ColorCode for crate::media::color::ChromaLocation {
    fn code(&self) -> u8 {
        use crate::media::color::ChromaLocation::*;
        match self {
            Unspecified => 0,
            Center => 1,
            Left => 2,
            TopLeft => 3,
            Top => 4,
            BottomLeft => 5,
            Bottom => 6,
        }
    }
}

fn probe_command(tools: &ToolPaths, source: &Path) -> Vec<String> {
    vec![
        tools.ffprobe.display().to_string(),
        "-v".into(),
        "error".into(),
        "-show_streams".into(),
        "-select_streams".into(),
        "v".into(),
        "-of".into(),
        "default=noprint_wrappers=1".into(),
        source.display().to_string(),
    ]
}

fn framehash_command(tools: &ToolPaths, source: &Path, stream: u64) -> Vec<String> {
    vec![
        tools.ffmpeg.display().to_string(),
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "error".into(),
        "-protocol_whitelist".into(),
        "file".into(),
        "-i".into(),
        source.display().to_string(),
        "-map".into(),
        format!("0:v:{stream}"),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-fps_mode".into(),
        "passthrough".into(),
        "-f".into(),
        "framehash".into(),
        "-".into(),
    ]
}

fn nut_command(tools: &ToolPaths, source: &Path, stream: u64) -> Vec<String> {
    vec![
        tools.ffmpeg.display().to_string(),
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "error".into(),
        "-protocol_whitelist".into(),
        "file".into(),
        "-i".into(),
        source.display().to_string(),
        "-map".into(),
        format!("0:v:{stream}"),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-fps_mode".into(),
        "passthrough".into(),
        "-c:v".into(),
        "rawvideo".into(),
        "-f".into(),
        "nut".into(),
        "-write_index".into(),
        "0".into(),
        "-".into(),
    ]
}
