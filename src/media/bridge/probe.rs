//! FFprobe manifest — Phase V.1.3 (V.1 video programme, brief §38).
//!
//! Before any decode, the bridge records the source's interpretation from
//! `ffprobe`: container, stream, coded/decoded geometry, pixel format, time
//! base, frame-rate fields, SAR, field order, primaries/transfer/matrix/
//! range/chroma location, orientation tags, and frame count where available.
//! The manifest is **evidence**: it is recorded in the import report and is
//! never a playback dependency (§38).
//!
//! Output is parsed from `ffprobe -of default=noprint_wrappers=1` key=value
//! text (no JSON dependency in the crate); stream records are delimited by
//! their `index=` lines. Every parse is bounded and typed; unknown values
//! stay `Unspecified`/`None` and the raw strings are preserved on the
//! manifest so nothing is silently guessed (§21).

use crate::error::VoleError;
use crate::media::bridge::run::{run_bounded, ChildLimits, RunOutcome, ToolPaths};
use crate::media::color::{
    ChromaLocation, ColorPrimaries, ColorRange, MatrixCoefficients, TransferCharacteristic,
};
use crate::media::meta::{FieldStructure, Orientation};

/// The container-level format facts.
#[derive(Debug, Clone, Default)]
pub struct FormatInfo {
    /// Raw `format_name` (may be a comma list).
    pub container: Option<String>,
    /// Raw `duration` seconds string.
    pub duration: Option<String>,
    /// Raw `start_time` seconds string.
    pub start_time: Option<String>,
    /// Raw `bit_rate` string.
    pub bit_rate: Option<String>,
    /// Declared stream count.
    pub nb_streams: Option<u64>,
}

/// One probed video stream (only fields the bridge interprets; raw strings
/// are preserved for evidence).
#[derive(Debug, Clone, Default)]
pub struct StreamInfo {
    /// Stream index.
    pub index: u64,
    /// `codec_name`.
    pub codec_name: Option<String>,
    /// `codec_type`.
    pub codec_type: Option<String>,
    /// `profile`.
    pub profile: Option<String>,
    /// Coded width.
    pub width: Option<u64>,
    /// Coded height.
    pub height: Option<u64>,
    /// `coded_width` when present.
    pub coded_width: Option<u64>,
    /// `coded_height` when present.
    pub coded_height: Option<u64>,
    /// Decoded pixel format name (`pix_fmt`).
    pub pix_fmt: Option<String>,
    /// Stream time base `num/den` string.
    pub time_base: Option<String>,
    /// `start_pts`.
    pub start_pts: Option<String>,
    /// `duration_ts`.
    pub duration_ts: Option<String>,
    /// `nb_frames` where the container declares it.
    pub nb_frames: Option<String>,
    /// `avg_frame_rate` rational string.
    pub avg_frame_rate: Option<String>,
    /// `r_frame_rate` rational string.
    pub r_frame_rate: Option<String>,
    /// `sample_aspect_ratio` `a:b` string.
    pub sample_aspect_ratio: Option<String>,
    /// `field_order` string.
    pub field_order: Option<String>,
    /// `color_range` raw string (`tv`/`pc`/`unknown`/…).
    pub color_range: Option<String>,
    /// `color_space` raw string.
    pub color_space: Option<String>,
    /// `color_transfer` raw string.
    pub color_transfer: Option<String>,
    /// `color_primaries` raw string.
    pub color_primaries: Option<String>,
    /// `chroma_location` raw string.
    pub chroma_location: Option<String>,
    /// `bits_per_raw_sample`.
    pub bits_per_raw_sample: Option<String>,
    /// Stream tags (`TAG:NAME=value`).
    pub tags: Vec<(String, String)>,
    /// `disposition.attached_pic` flag.
    pub attached_pic: bool,
    /// `disposition.default` flag.
    pub is_default: bool,
}

impl StreamInfo {
    /// The display orientation from the `rotate` tag (mkv/mov convention).
    pub fn orientation(&self) -> Orientation {
        for (k, v) in &self.tags {
            if k.eq_ignore_ascii_case("rotate") {
                match v.trim().parse::<i64>() {
                    Ok(90) | Ok(-270) => return Orientation::Rotate90,
                    Ok(180) | Ok(-180) => return Orientation::Rotate180,
                    Ok(270) | Ok(-90) => return Orientation::Rotate270,
                    _ => return Orientation::Normal,
                }
            }
        }
        Orientation::Normal
    }

    /// `field_order` → [`FieldStructure`].
    pub fn field_structure(&self) -> FieldStructure {
        match self.field_order.as_deref() {
            Some("progressive") => FieldStructure::Progressive,
            Some("tt") => FieldStructure::InterlacedTopFieldFirst,
            Some("bb") => FieldStructure::InterlacedBottomFieldFirst,
            Some("tb") => FieldStructure::InterlacedTopFieldFirst,
            Some("bt") => FieldStructure::InterlacedBottomFieldFirst,
            _ => FieldStructure::Unknown,
        }
    }

    /// `color_range` → [`ColorRange`] (`tv`=limited, `pc`=full).
    pub fn color_range(&self) -> ColorRange {
        match self.color_range.as_deref() {
            Some("tv") | Some("limited") => ColorRange::Limited,
            Some("pc") | Some("full") => ColorRange::Full,
            _ => ColorRange::Unspecified,
        }
    }

    /// `color_space` (matrix) → [`MatrixCoefficients`].
    pub fn matrix(&self) -> MatrixCoefficients {
        match self.color_space.as_deref() {
            Some("bt709") => MatrixCoefficients::Bt709,
            Some("bt470bg") | Some("smpte170m") => MatrixCoefficients::Smpte170M,
            Some("smpte240m") => MatrixCoefficients::Smpte240M,
            Some("ycgco") => MatrixCoefficients::YcgCo,
            Some("bt2020nc") | Some("bt2020ncl") => MatrixCoefficients::Bt2020Ncl,
            Some("bt2020c") | Some("bt2020cl") => MatrixCoefficients::Bt2020Cl,
            Some("gbr") | Some("identity") | Some("rgb") => MatrixCoefficients::Identity,
            Some("fcc") => MatrixCoefficients::Unspecified, // no FCC entry in the canonical set
            _ => MatrixCoefficients::Unspecified,
        }
    }

    /// `color_transfer` → [`TransferCharacteristic`].
    pub fn transfer(&self) -> TransferCharacteristic {
        match self.color_transfer.as_deref() {
            Some("bt709") => TransferCharacteristic::Bt709,
            Some("bt470m") | Some("gamma22") => TransferCharacteristic::Gamma22,
            Some("bt470bg") | Some("gamma28") => TransferCharacteristic::Gamma28,
            Some("smpte170m") => TransferCharacteristic::Smpte170M,
            Some("smpte240m") => TransferCharacteristic::Smpte240M,
            Some("linear") => TransferCharacteristic::Linear,
            Some("iec61966-2-1") | Some("srgb") => TransferCharacteristic::Srgb,
            Some("bt2020-10") => TransferCharacteristic::Bt2020_10,
            Some("bt2020-12") => TransferCharacteristic::Bt2020_12,
            Some("smpte2084") => TransferCharacteristic::Smpte2084,
            Some("arib-std-b67") => TransferCharacteristic::AribStdB67,
            _ => TransferCharacteristic::Unspecified,
        }
    }

    /// `color_primaries` → [`ColorPrimaries`].
    pub fn primaries(&self) -> ColorPrimaries {
        match self.color_primaries.as_deref() {
            Some("bt709") => ColorPrimaries::Bt709,
            Some("bt470m") => ColorPrimaries::Bt470M,
            Some("bt470bg") => ColorPrimaries::Bt470Bg,
            Some("smpte170m") => ColorPrimaries::Smpte170M,
            Some("smpte240m") => ColorPrimaries::Smpte240M,
            Some("film") => ColorPrimaries::Film,
            Some("bt2020") => ColorPrimaries::Bt2020,
            _ => ColorPrimaries::Unspecified,
        }
    }

    /// `chroma_location` → [`ChromaLocation`].
    pub fn chroma_location(&self) -> ChromaLocation {
        match self.chroma_location.as_deref() {
            Some("left") => ChromaLocation::Left,
            Some("center") => ChromaLocation::Center,
            Some("topleft") => ChromaLocation::TopLeft,
            Some("top") => ChromaLocation::Top,
            Some("bottomleft") => ChromaLocation::BottomLeft,
            Some("bottom") => ChromaLocation::Bottom,
            _ => ChromaLocation::Unspecified,
        }
    }

    /// Parse `sample_aspect_ratio` (`a:b`), `None` for `0:0`/`N/A`.
    pub fn sar(&self) -> Option<(u32, u32)> {
        let s = self.sample_aspect_ratio.as_deref()?;
        let (a, b) = s.split_once(':')?;
        let (a, b) = (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?);
        if a == 0 || b == 0 || a >= (1 << 31) || b >= (1 << 31) {
            return None;
        }
        Some((a as u32, b as u32))
    }
}

/// The probed manifest: container facts plus every probed video stream and
/// the deterministic stream-selection choice (§41): explicit index wins;
/// otherwise the usable default-disposition video stream (attached pictures
/// excluded); ties go to the lowest stream index.
#[derive(Debug, Clone)]
pub struct ProbeManifest {
    /// Container format facts.
    pub format: FormatInfo,
    /// Every video stream.
    pub streams: Vec<StreamInfo>,
    /// The chosen stream's index.
    pub chosen: u64,
}

/// Deterministic stream selection over the probed video streams.
pub fn choose_video_stream(
    streams: &[StreamInfo],
    explicit: Option<u64>,
) -> Result<u64, VoleError> {
    if let Some(idx) = explicit {
        if streams
            .iter()
            .any(|s| s.index == idx && s.is_usable_video())
        {
            return Ok(idx);
        }
        return Err(VoleError::BridgeProbeFailed);
    }
    let usable: Vec<&StreamInfo> = streams.iter().filter(|s| s.is_usable_video()).collect();
    let chosen = usable
        .iter()
        .filter(|s| s.is_default || s.attached_pic)
        .min_by_key(|s| s.index)
        .or_else(|| usable.iter().min_by_key(|s| s.index))
        .ok_or(VoleError::BridgeProbeFailed)?;
    Ok(chosen.index)
}

impl StreamInfo {
    /// Whether this probed stream is a usable video track (codec type video,
    /// not an attached picture).
    pub fn is_usable_video(&self) -> bool {
        self.codec_type.as_deref() == Some("video") && !self.attached_pic
    }
}

/// Parse one key=value block (a stream record or the format record).
fn parse_block(lines: &[&str]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn get<'a>(kv: &'a [(String, String)], key: &str) -> Option<&'a str> {
    kv.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Probe `source` with the resolved tools and choose the video stream.
pub fn probe_media(
    tools: &ToolPaths,
    source: &std::path::Path,
    explicit_stream: Option<u64>,
) -> Result<ProbeManifest, VoleError> {
    let limits = ChildLimits {
        stdout_bytes: 4 << 20,
        ..ChildLimits::default()
    };
    // All video streams (records delimited by their index= line).
    let streams_out = run_bounded(
        &tools.ffprobe,
        &[
            "-v",
            "error",
            "-show_streams",
            "-select_streams",
            "v",
            "-of",
            "default=noprint_wrappers=1",
            &source.display().to_string(),
        ],
        &limits,
    )?;
    check_probe_exit(&streams_out, "ffprobe streams")?;
    let streams = parse_stream_records(&streams_out.stdout);

    let format_out = run_bounded(
        &tools.ffprobe,
        &[
            "-v",
            "error",
            "-show_format",
            "-of",
            "default=noprint_wrappers=1",
            &source.display().to_string(),
        ],
        &limits,
    )?;
    check_probe_exit(&format_out, "ffprobe format")?;
    let format = parse_format(&format_out.stdout);

    let chosen = choose_video_stream(&streams, explicit_stream)?;
    Ok(ProbeManifest {
        format,
        streams,
        chosen,
    })
}

fn check_probe_exit(out: &RunOutcome, what: &str) -> Result<(), VoleError> {
    if out.code == Some(0) {
        return Ok(());
    }
    let _ = what;
    Err(VoleError::BridgeProbeFailed)
}

/// Split the merged default-output stream records on their `index=` lines.
fn parse_stream_records(bytes: &[u8]) -> Vec<StreamInfo> {
    let text = String::from_utf8_lossy(bytes);
    let mut records: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_record = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("index=") {
            if in_record {
                records.push(std::mem::take(&mut current));
            }
            in_record = true;
        }
        if in_record {
            current.push(trimmed);
        }
    }
    if in_record {
        records.push(current);
    }
    records
        .iter()
        .map(|lines| stream_from_kv(&parse_block(lines)))
        .collect()
}

fn parse_format(bytes: &[u8]) -> FormatInfo {
    let text = String::from_utf8_lossy(bytes);
    let kv = parse_block(&text.lines().collect::<Vec<_>>());
    FormatInfo {
        container: get(&kv, "format_name").map(str::to_string),
        duration: get(&kv, "duration").map(str::to_string),
        start_time: get(&kv, "start_time").map(str::to_string),
        bit_rate: get(&kv, "bit_rate").map(str::to_string),
        nb_streams: get(&kv, "nb_streams").and_then(|v| v.parse().ok()),
    }
}

fn stream_from_kv(kv: &[(String, String)]) -> StreamInfo {
    let mut tags = Vec::new();
    for (k, v) in kv {
        if let Some(name) = k.strip_prefix("TAG:") {
            tags.push((name.to_string(), v.clone()));
        }
    }
    let num = |key: &str| get(kv, key).and_then(|v| v.parse::<u64>().ok());
    StreamInfo {
        index: num("index").unwrap_or(0),
        codec_name: get(kv, "codec_name").map(str::to_string),
        codec_type: get(kv, "codec_type").map(str::to_string),
        profile: get(kv, "profile").map(str::to_string),
        width: num("width"),
        height: num("height"),
        coded_width: num("coded_width"),
        coded_height: num("coded_height"),
        pix_fmt: get(kv, "pix_fmt").map(str::to_string),
        time_base: get(kv, "time_base").map(str::to_string),
        start_pts: get(kv, "start_pts").map(str::to_string),
        duration_ts: get(kv, "duration_ts").map(str::to_string),
        nb_frames: get(kv, "nb_frames").map(str::to_string),
        avg_frame_rate: get(kv, "avg_frame_rate").map(str::to_string),
        r_frame_rate: get(kv, "r_frame_rate").map(str::to_string),
        sample_aspect_ratio: get(kv, "sample_aspect_ratio").map(str::to_string),
        field_order: get(kv, "field_order").map(str::to_string),
        color_range: get(kv, "color_range").map(str::to_string),
        color_space: get(kv, "color_space").map(str::to_string),
        color_transfer: get(kv, "color_transfer").map(str::to_string),
        color_primaries: get(kv, "color_primaries").map(str::to_string),
        chroma_location: get(kv, "chroma_location").map(str::to_string),
        bits_per_raw_sample: get(kv, "bits_per_raw_sample").map(str::to_string),
        tags,
        attached_pic: get(kv, "disposition.attached_pic") == Some("1"),
        is_default: get(kv, "disposition.default") == Some("1"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_records_split_on_index() {
        let text = "\nindex=0\ncodec_name=h264\nwidth=320\nTAG:rotate=90\n\nindex=1\ncodec_name=png\nwidth=10\nTAG:rotate=90\n";
        let streams = parse_stream_records(text.as_bytes());
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].index, 0);
        assert_eq!(streams[0].width, Some(320));
        assert_eq!(streams[0].orientation(), Orientation::Rotate90);
        assert_eq!(streams[1].index, 1);
    }

    #[test]
    fn field_and_color_maps() {
        let s = StreamInfo {
            color_range: Some("tv".into()),
            color_space: Some("bt709".into()),
            color_transfer: Some("smpte2084".into()),
            color_primaries: Some("bt2020".into()),
            chroma_location: Some("topleft".into()),
            field_order: Some("tt".into()),
            sample_aspect_ratio: Some("4:3".into()),
            ..Default::default()
        };
        assert_eq!(s.color_range(), ColorRange::Limited);
        assert_eq!(s.matrix(), MatrixCoefficients::Bt709);
        assert_eq!(s.transfer(), TransferCharacteristic::Smpte2084);
        assert_eq!(s.primaries(), ColorPrimaries::Bt2020);
        assert_eq!(s.chroma_location(), ChromaLocation::TopLeft);
        assert_eq!(s.field_structure(), FieldStructure::InterlacedTopFieldFirst);
        assert_eq!(s.sar(), Some((4, 3)));
    }

    #[test]
    fn stream_selection_prefers_default_video() {
        let a = StreamInfo {
            index: 0,
            codec_type: Some("video".into()),
            attached_pic: false,
            is_default: false,
            ..Default::default()
        };
        let b = StreamInfo {
            index: 1,
            codec_type: Some("video".into()),
            attached_pic: false,
            is_default: true,
            ..Default::default()
        };
        assert_eq!(choose_video_stream(&[a.clone(), b.clone()], None), Ok(1));
        assert_eq!(choose_video_stream(&[a.clone(), b.clone()], Some(0)), Ok(0));
        // Attached pictures are excluded from auto selection.
        let c = StreamInfo {
            index: 2,
            codec_type: Some("video".into()),
            attached_pic: true,
            ..Default::default()
        };
        assert_eq!(
            choose_video_stream(&[c], None),
            Err(VoleError::BridgeProbeFailed)
        );
    }
}
