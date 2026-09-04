//! Framehash oracle — Phase V.1.3 (V.1 video programme, brief §39).
//!
//! An **independent** per-observation digest path: FFmpeg's `framehash`
//! muxer decodes the source with the same settings as the NUT pipe and prints
//! one record per frame — `#frame#, dts, pts, duration, size, sha256` — where
//! the SHA-256 covers the decoded frame's **tight rows** in its pixel format
//! (empirically established: the muxers hash exactly `width` bytes per row,
//! never the aligned linesize, so odd widths are tight too). VOLE's
//! NUT-derived canonical bytes must reproduce the same digest per frame;
//! any disagreement is [`VoleError::CanonicalHashMismatch`] (IMPORT FAIL).
//!
//! The record's `size` column independently confirms the expected canonical
//! payload byte count of each frame.

use crate::error::VoleError;
use crate::media::bridge::run::{run_bounded, ChildLimits, ToolPaths};

/// One per-frame oracle record.
#[derive(Debug, Clone)]
pub struct FramehashRecord {
    /// Frame ordinal (decode order, 0-based).
    pub frame: u64,
    /// Decode timestamp in the oracle's time base (`None` when `N/A`).
    pub dts: Option<i64>,
    /// Presentation timestamp in the oracle's time base.
    pub pts: Option<i64>,
    /// Frame duration in the oracle's time base.
    pub duration: Option<i64>,
    /// Canonical sample byte count of the frame.
    pub size: u64,
    /// SHA-256 over the tight-row decoded frame.
    pub sha256: [u8; 32],
}

/// The parsed oracle run.
#[derive(Debug, Clone)]
pub struct FramehashResult {
    /// The oracle output time base `(num, den)` seconds per tick (from the
    /// `#tb 0:` header line).
    pub time_base: (u32, u32),
    /// Per-frame records in decode order.
    pub records: Vec<FramehashRecord>,
}

/// Run the framehash oracle over `source`'s chosen video stream.
///
/// The decode settings mirror the NUT pipe exactly (software decode, no
/// conversion, no frame dropping/duplication) so the digest path is the
/// canonical-sample oracle of §39.
pub fn run_framehash(
    tools: &ToolPaths,
    source: &std::path::Path,
    stream_index: u64,
) -> Result<FramehashResult, VoleError> {
    let out = run_bounded(
        &tools.ffmpeg,
        &[
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file",
            "-i",
            &source.display().to_string(),
            "-map",
            &format!("0:v:{}", stream_index),
            "-an",
            "-sn",
            "-dn",
            "-fps_mode",
            "passthrough",
            "-f",
            "framehash",
            "-",
        ],
        &ChildLimits {
            stdout_bytes: 1 << 28,
            ..ChildLimits::default()
        },
    )?;
    parse_framehash(&out.stdout)
}

/// Parse the framehash text output. Bounded: at most `max_records` records
/// are accepted; anything malformed is a typed error.
pub fn parse_framehash(bytes: &[u8]) -> Result<FramehashResult, VoleError> {
    let text = String::from_utf8_lossy(bytes);
    let mut time_base: Option<(u32, u32)> = None;
    let mut records = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            if let Some(tb) = line.strip_prefix("#tb ") {
                // Format: "#tb 0: 1/25" (stream id, then the rational).
                if let Some((_, rat)) = tb.split_once(':') {
                    if let Some((num, den)) = parse_rational(rat.trim()) {
                        time_base = Some((num, den));
                    }
                }
            }
            continue;
        }
        // Data lines: frame, dts, pts, duration, size, sha256.
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if cols.len() < 6 {
            return Err(VoleError::BridgeProbeFailed);
        }
        let frame = cols[0]
            .parse::<u64>()
            .map_err(|_| VoleError::BridgeProbeFailed)?;
        let dts = parse_i64_opt(cols[1]);
        let pts = parse_i64_opt(cols[2]);
        let duration = parse_i64_opt(cols[3]);
        let size = cols[4]
            .parse::<u64>()
            .map_err(|_| VoleError::BridgeProbeFailed)?;
        let hash_hex = cols[5].trim();
        if hash_hex.len() != 64 {
            return Err(VoleError::BridgeProbeFailed);
        }
        let mut sha256 = [0u8; 32];
        for (i, byte) in sha256.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hash_hex[i * 2..i * 2 + 2], 16)
                .map_err(|_| VoleError::BridgeProbeFailed)?;
        }
        records.push(FramehashRecord {
            frame,
            dts,
            pts,
            duration,
            size,
            sha256,
        });
        if records.len() > 4 << 20 {
            return Err(VoleError::BridgeOutputLimit);
        }
    }
    let tb = time_base.ok_or(VoleError::BridgeProbeFailed)?;
    Ok(FramehashResult {
        time_base: tb,
        records,
    })
}

fn parse_i64_opt(s: &str) -> Option<i64> {
    if s == "N/A" || s.is_empty() {
        return None;
    }
    s.parse().ok()
}

fn parse_rational(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.split_once('/')?;
    let (a, b) = (a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?);
    if a == 0 || b == 0 || a >= (1 << 31) || b >= (1 << 31) {
        return None;
    }
    Some((a as u32, b as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_and_header() {
        let text = "\
#format: frame checksums
#version: 2
#hash: SHA256
#tb 0: 1/25
#media_type 0: video
0,          0,          0,        1,    86400, 78422a4f0ef0ac0789fef8205b4346d51a8193516e09809d6ee0320d1139e79a
1,          1,          1,        1,    86400, adbf495c8245f798d9fa14eb2047cb4f0503d2261955626defd34b6c42edd1aa
";
        let r = parse_framehash(text.as_bytes()).expect("parse");
        assert_eq!(r.time_base, (1, 25));
        assert_eq!(r.records.len(), 2);
        assert_eq!(r.records[0].pts, Some(0));
        assert_eq!(r.records[1].size, 86400);
        assert_eq!(r.records[0].sha256[0], 0x78);
    }

    #[test]
    fn malformed_records_are_typed() {
        assert!(parse_framehash(b"0, 1, x, 1, 2, deadbeef\n").is_err());
        assert!(parse_framehash(b"#tb 0: 1/25\n0,0,0,1,4,abcd\n").is_err());
    }
}
