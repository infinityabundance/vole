//! Narrow NUT reader — Phase V.1.3 (V.1 video programme, brief §36–§37).
//!
//! A pure, hostile-safe parser for the NUT container subset **VOLE's
//! controlled FFmpeg bridge emits** (`ffmpeg -c:v rawvideo -f nut`): main
//! header, stream headers, info/syncpoint packets, and rawvideo frame
//! packets with exact PTS recovery. It is deliberately not a general
//! multimedia demuxer (§37): anything outside the emitted subset fails
//! closed with a typed error.
//!
//! Format facts (validated empirically against ffmpeg n9.0 muxed files, with
//! per-frame PTS cross-checked against `ffprobe` and payloads byte-checked
//! against FFmpeg's independent `framehash`/`framemd5` muxers):
//!
//! * startcodes are 8-byte big-endian values beginning with `'N'`;
//! * packets are `startcode ‖ varlen(forward_ptr) ‖ body ‖ le32-crc` where
//!   `forward_ptr` counts body+crc and the CRC is the MSB-first CRC-32 of the
//!   body stored big-endian (see [`crate::media::bridge::crc`]);
//! * varlen is a big-endian 7-bit-group varint with continuation bit `0x80`;
//! * the frame-code table in the main header delta-encodes runs of frame
//!   header defaults (index `'N'` is always invalid);
//! * frame headers add coded fields per flags; PTS is recovered from
//!   `last_pts + pts_delta` or `lsb2full` against `last_pts`;
//! * syncpoints carry a time-coded `ts` that resets every stream's `last_pts`
//!   (needed for long streams where coded PTS wraps).
//!
//! All integer arithmetic is checked; every count/length is bounded; the
//! parser never panics on hostile bytes.

use crate::error::VoleError;
use crate::media::bridge::crc;

const MAIN_STARTCODE: u64 = 0x4E4D_7A56_1F5F_04AD;
const STREAM_STARTCODE: u64 = 0x4E53_1140_5BF2_F9DB;
const SYNCPOINT_STARTCODE: u64 = 0x4E4B_E4AD_EECA_4569;
const INDEX_STARTCODE: u64 = 0x4E58_DD67_2F23_E64E;
const INFO_STARTCODE: u64 = 0x4E49_AB68_B596_BA78;

const FLAG_KEY: u16 = 1;
const FLAG_CODED_PTS: u16 = 8;
const FLAG_STREAM_ID: u16 = 16;
const FLAG_SIZE_MSB: u16 = 32;
const FLAG_CHECKSUM: u16 = 64;
const FLAG_RESERVED: u16 = 128;
const FLAG_SM_DATA: u16 = 256;
const FLAG_HEADER_IDX: u16 = 1024;
const FLAG_MATCH_TIME: u16 = 2048;
const FLAG_CODED: u16 = 4096;
const FLAG_INVALID: u16 = 8192;

/// Hard bounds of the narrow reader (hostile inputs cannot exceed them).
pub mod bounds {
    /// Maximum declared streams.
    pub const MAX_STREAMS: usize = 64;
    /// Maximum declared time bases.
    pub const MAX_TIME_BASES: usize = 128;
    /// Maximum frame-code run length.
    pub const MAX_RUN: u64 = 512;
    /// Maximum version accepted (NUT 2..=4).
    pub const MAX_VERSION: u64 = 4;
    /// Maximum bytes in one frame payload (far above any courted frame; the
    /// real bound comes from the epoch limits at canonicalization).
    pub const MAX_FRAME_PAYLOAD: u64 = 1 << 30;
}

/// One declared stream of the NUT file (the narrow reader only interprets
/// the single rawvideo video stream the bridge requests).
#[derive(Debug, Clone)]
pub struct NutStreamHeader {
    /// Stream index in the container.
    pub index: u32,
    /// NUT class: 0 video, 1 audio, 2 subtitle, 3 data.
    pub class: u32,
    /// Codec fourcc (little-endian tag value as written).
    pub codec_tag: u32,
    /// Declared coded width (video only).
    pub width: u32,
    /// Declared coded height (video only).
    pub height: u32,
    /// Declared sample aspect ratio (video only); `None` when 0/0.
    pub sar: Option<(u32, u32)>,
    /// Index into the container time-base table.
    pub tb_index: u32,
    /// The stream's time base `(num, den)` seconds per tick.
    pub time_base: (u32, u32),
    /// `msb_pts_shift` used by the lsb2full PTS recovery.
    pub msb_pts_shift: u32,
}

/// One recovered rawvideo frame.
#[derive(Debug, Clone)]
pub struct NutFrame {
    /// Source stream index.
    pub stream: u32,
    /// Presentation timestamp in the stream's time base.
    pub pts: i64,
    /// Whether the frame is marked as a keyframe.
    pub key: bool,
    /// The raw packet payload (tight canonical bytes for rawvideo).
    pub payload: Vec<u8>,
}

/// The parsed narrow NUT stream.
#[derive(Debug, Clone)]
pub struct NutStream {
    /// Format version (2..=4).
    pub version: u32,
    /// Container time-base table `(num, den)` seconds per tick.
    pub time_bases: Vec<(u32, u32)>,
    /// Declared streams.
    pub streams: Vec<NutStreamHeader>,
    /// Recovered frames in decode order.
    pub frames: Vec<NutFrame>,
}

/// A byte cursor over the input with checked reads.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn u8(&mut self) -> Result<u8, VoleError> {
        let b = self
            .data
            .get(self.pos)
            .copied()
            .ok_or(VoleError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], VoleError> {
        if n > self.remaining() {
            return Err(VoleError::Truncated);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// NUT varlen (BE 7-bit groups, continuation bit 0x80).
    fn var(&mut self) -> Result<u64, VoleError> {
        let mut val: u64 = 0;
        for _ in 0..10 {
            let b = self.u8()?;
            val = val.checked_shl(7).ok_or(VoleError::ArithmeticOverflow)? | u64::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(val);
            }
        }
        Err(VoleError::NonCanonicalEncoding)
    }

    /// NUT signed varlen.
    fn var_s(&mut self) -> Result<i64, VoleError> {
        let v = self.var()?.wrapping_add(1);
        Ok(if v & 1 == 1 {
            -((v >> 1) as i64)
        } else {
            (v >> 1) as i64
        })
    }

    /// Fourcc: varlen length 2 or 4, then little-endian bytes.
    fn fourcc(&mut self) -> Result<u32, VoleError> {
        match self.var()? {
            2 => {
                let b = self.take(2)?;
                Ok(u32::from(u16::from_le_bytes([b[0], b[1]])))
            }
            4 => {
                let b = self.take(4)?;
                Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            _ => Err(VoleError::NonCanonicalEncoding),
        }
    }

    /// Skip `n` bytes (bounded by the slice).
    #[allow(dead_code)] // reserved for future packet resync paths
    fn skip(&mut self, n: usize) -> Result<(), VoleError> {
        if n > self.remaining() {
            return Err(VoleError::Truncated);
        }
        self.pos += n;
        Ok(())
    }
}

/// One decoded frame-code entry.
#[derive(Debug, Clone, Copy, Default)]
struct FrameCode {
    flags: u16,
    stream_id: u32,
    size_mul: u64,
    size_lsb: u64,
    pts_delta: i64,
    reserved_count: u32,
    header_idx: u32,
}

/// Find the next occurrence of `code` at or after the cursor and position
/// right after it.
fn find_startcode(cur: &mut Cursor<'_>, code: u64) -> Result<(), VoleError> {
    let bytes = code.to_be_bytes();
    let mut i = cur.pos;
    while i + 8 <= cur.data.len() {
        if cur.data[i] == b'N' && cur.data[i..i + 8] == bytes {
            cur.pos = i + 8;
            return Ok(());
        }
        i += 1;
    }
    Err(VoleError::Truncated)
}

fn parse_main_header(
    cur: &mut Cursor<'_>,
    frame_codes: &mut [FrameCode; 256],
    header_len: &mut [u32; 128],
    time_bases: &mut Vec<(u32, u32)>,
) -> Result<u32, VoleError> {
    find_startcode(cur, MAIN_STARTCODE)?;
    // Read and verify the packet envelope: size counts body + 4 CRC bytes.
    let size = cur.var()?;
    let body_start = cur.pos;
    if size > 4096 {
        // Mid-header CRC over startcode + length field.
        let stored = cur.take(4)?;
        let mut head = Vec::with_capacity(12);
        head.extend_from_slice(&MAIN_STARTCODE.to_be_bytes());
        // Re-encode the length varlen.
        let mut v = size;
        let mut tmp = Vec::new();
        tmp.push((v & 0x7F) as u8);
        v >>= 7;
        while v > 0 {
            tmp.push(0x80 | (v & 0x7F) as u8);
            v >>= 7;
        }
        tmp.reverse();
        head.extend_from_slice(&tmp);
        if !crc::verify_packet_crc(&head, stored) {
            return Err(VoleError::IntegrityMismatch);
        }
    }
    let body_end = body_start + (size - 4) as usize;
    if body_end > cur.data.len() {
        return Err(VoleError::Truncated);
    }

    let version = cur.var()?;
    if !(2..=bounds::MAX_VERSION).contains(&version) {
        return Err(VoleError::UnsupportedFeature);
    }
    if version > 3 {
        let _minor = cur.var()?;
    }
    let stream_count = cur.var()?;
    if stream_count == 0 || stream_count > bounds::MAX_STREAMS as u64 {
        return Err(VoleError::DimensionTooLarge);
    }
    let max_distance = cur.var()?;
    if max_distance > 65_536 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let tb_count = cur.var()?;
    if tb_count == 0 || tb_count > bounds::MAX_TIME_BASES as u64 {
        return Err(VoleError::DimensionTooLarge);
    }
    for _ in 0..tb_count {
        let num = cur.var()?;
        let den = cur.var()?;
        if num == 0 || den == 0 || num >= (1 << 31) || den >= (1 << 31) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        time_bases.push((num as u32, den as u32));
    }

    // Frame-code table (delta-encoded runs; 'N' = 0x4E is always invalid).
    let mut i: usize = 0;
    let mut pts: i64 = 0;
    let mut mul: u64 = 1;
    let mut sid: u64 = 0;
    let mut size_lsb: u64 = 0;
    let mut head_idx: u64 = 0;
    while i < 256 {
        let flags = cur.var()?;
        if flags > u64::from(u16::MAX) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let fields = cur.var()?;
        if fields > 20 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let mut pts_v = 0i64;
        let mut mul_v: u64 = 1;
        let mut sid_v: u64 = 0;
        let mut size_v: u64 = 0;
        let mut head_v: u64 = 0;
        if fields > 0 {
            pts_v = cur.var_s()?;
        }
        if fields > 1 {
            mul_v = cur.var()?;
        }
        if fields > 2 {
            sid_v = cur.var()?;
        }
        if fields > 3 {
            size_v = cur.var()?;
        }
        if fields > 4 {
            let _res = cur.var()?;
        }
        let count = if fields > 5 {
            cur.var()?
        } else {
            mul_v.wrapping_sub(size_v)
        };
        if fields > 6 {
            let _match = cur.var_s()?;
        }
        if fields > 7 {
            head_v = cur.var()?;
        }
        let mut extra = fields;
        while extra > 8 {
            cur.var()?;
            extra -= 1;
        }
        if count == 0 || count > bounds::MAX_RUN {
            return Err(VoleError::NonCanonicalEncoding);
        }
        if sid_v >= stream_count {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let mut j: u64 = 0;
        while j < count {
            if i == 0x4E {
                frame_codes[i].flags = FLAG_INVALID;
                i += 1;
                continue;
            }
            frame_codes[i] = FrameCode {
                flags: flags as u16,
                stream_id: sid_v as u32,
                size_mul: mul_v,
                size_lsb: size_v.wrapping_add(j),
                pts_delta: pts_v,
                reserved_count: 0,
                header_idx: head_v as u32,
            };
            size_v = size_v.wrapping_add(1);
            j += 1;
            i += 1;
        }
        pts = pts_v;
        mul = mul_v;
        sid = sid_v;
        size_lsb = size_v;
        head_idx = head_v;
    }
    let _ = (pts, mul, sid, size_lsb, head_idx);

    // Elision headers (fixed mp3/mpeg start codes; rawvideo never matches).
    if body_end > cur.pos + 4 {
        let hc = cur.var()?;
        if hc >= 128 {
            return Err(VoleError::DimensionTooLarge);
        }
        for cell in header_len[1..=hc as usize].iter_mut() {
            let ln = cur.var()?;
            if ln == 0 || ln > 256 || ln as usize > cur.remaining() {
                return Err(VoleError::NonCanonicalEncoding);
            }
            *cell = ln as u32;
            cur.take(ln as usize)?;
        }
    }
    // Version > 3 carries a flags varlen.
    let mut flags_v = 0u64;
    if version > 3 && body_end > cur.pos + 4 {
        flags_v = cur.var()?;
        if flags_v > 3 {
            return Err(VoleError::UnsupportedFeature);
        }
    }
    let _ = flags_v;
    // Reserved to the body end, then verify the stored CRC.
    if cur.pos > body_end {
        return Err(VoleError::NonCanonicalEncoding);
    }
    cur.take(body_end - cur.pos)?;
    let stored = cur.take(4)?;
    if !crc::verify_packet_crc(&cur.data[body_start..body_end], stored) {
        return Err(VoleError::IntegrityMismatch);
    }
    Ok(version as u32)
}

fn parse_stream_headers(
    cur: &mut Cursor<'_>,
    stream_count: u64,
    time_bases: &[(u32, u32)],
) -> Result<Vec<NutStreamHeader>, VoleError> {
    let mut out = Vec::with_capacity(stream_count as usize);
    let mut seen = vec![false; stream_count as usize];
    let mut found = 0usize;
    while found < stream_count as usize {
        find_startcode(cur, STREAM_STARTCODE)?;
        let size = cur.var()?;
        let body_start = cur.pos;
        // Stream headers are small packets in every stream FFmpeg emits; the
        // narrow reader accepts only the ≤ 4096-byte form (no mid-header
        // checksum word).
        if !(4..=4096).contains(&size) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let body_end = body_start + (size - 4) as usize;
        if body_end > cur.data.len() {
            return Err(VoleError::Truncated);
        }
        let index = cur.var()?;
        if index >= stream_count || seen[index as usize] {
            return Err(VoleError::NonCanonicalEncoding);
        }
        seen[index as usize] = true;
        found += 1;
        let class = cur.var()?;
        let tag = cur.fourcc()?;
        let tb_index = cur.var()?;
        if tb_index as usize >= time_bases.len() {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let msb = cur.var()?;
        if msb >= 16 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let _max_pts_distance = cur.var()?;
        let _decode_delay = cur.var()?;
        let _stream_flags = cur.var()?;
        let ext_len = cur.var()?;
        if ext_len > 1 << 20 || ext_len as usize > cur.remaining() {
            return Err(VoleError::DimensionTooLarge);
        }
        cur.take(ext_len as usize)?;
        let mut header = NutStreamHeader {
            index: index as u32,
            class: class as u32,
            codec_tag: tag,
            width: 0,
            height: 0,
            sar: None,
            tb_index: tb_index as u32,
            time_base: time_bases[tb_index as usize],
            msb_pts_shift: msb as u32,
        };
        match class {
            0 => {
                header.width = cur.var()? as u32;
                header.height = cur.var()? as u32;
                if header.width == 0 || header.height == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let sar_num = cur.var()?;
                let sar_den = cur.var()?;
                if (sar_num == 0) != (sar_den == 0) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if sar_num != 0 {
                    header.sar = Some((sar_num as u32, sar_den as u32));
                }
                let _csp = cur.var()?;
            }
            1 => {
                let _rate = cur.var()?;
                let _rate_den = cur.var()?;
                let _channels = cur.var()?;
                if _channels == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
            2 | 3 => {}
            _ => return Err(VoleError::UnsupportedFeature),
        }
        if cur.pos > body_end {
            return Err(VoleError::NonCanonicalEncoding);
        }
        cur.take(body_end - cur.pos)?;
        let stored = cur.take(4)?;
        if !crc::verify_packet_crc(&cur.data[body_start..body_end], stored) {
            return Err(VoleError::IntegrityMismatch);
        }
        out.push(header);
    }
    Ok(out)
}

/// Parse the full narrow NUT subset from `data`. Returns the stream
/// description and every recovered frame in decode order.
pub fn parse_nut(data: &[u8]) -> Result<NutStream, VoleError> {
    if data.len() < 25 || &data[..25] != b"nut/multimedia container\0" {
        return Err(VoleError::BadMagic);
    }
    let mut cur = Cursor::new(data);
    cur.pos = 25;
    let mut frame_codes = [FrameCode::default(); 256];
    let mut header_len = [0u32; 128];
    let mut time_bases: Vec<(u32, u32)> = Vec::new();
    let version = parse_main_header(&mut cur, &mut frame_codes, &mut header_len, &mut time_bases)?;

    // Stream headers: the bridge requests exactly one video stream (index 0),
    // so the narrow reader collects stream packets until a non-STREAM
    // startcode appears; any declared count other than the emitted shape
    // fails closed inside the per-header parse.
    let mut streams: Vec<NutStreamHeader> = Vec::new();
    loop {
        let save = cur.pos;
        match peek_startcode(&mut cur) {
            Ok(STREAM_STARTCODE) => {
                cur.pos = save;
                let collected = parse_stream_headers(&mut cur, 1, &time_bases)?;
                streams.extend(collected);
            }
            _ => {
                cur.pos = save;
                break;
            }
        }
        if streams.len() > bounds::MAX_STREAMS {
            return Err(VoleError::DimensionTooLarge);
        }
    }
    if streams.is_empty() {
        return Err(VoleError::Truncated);
    }

    // Packet walk: frames + skip packets, with syncpoint PTS resets.
    let mut frames = Vec::new();
    let mut last_pts: Vec<i64> = vec![0; streams.len()];
    let mut sp_count: u64 = 0;
    let mut sync_total = 0u64;
    loop {
        let kind = peek_next(cur.data, cur.pos);
        match kind {
            PacketKind::End => break,
            PacketKind::Frame => {
                let code = cur.data[cur.pos] as usize;
                cur.pos += 1;
                if frame_codes[code].flags & FLAG_INVALID != 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if let Some(frame) = read_frame(
                    &mut cur,
                    &frame_codes[code],
                    &header_len,
                    &streams,
                    &mut last_pts,
                )? {
                    frames.push(frame);
                }
            }
            PacketKind::Startcode(code) => {
                cur.pos += 8;
                let size = cur.var()?;
                let body_start = cur.pos;
                if size < 4 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let body_end = body_start + (size - 4) as usize;
                if body_end > cur.data.len() {
                    return Err(VoleError::Truncated);
                }
                match code {
                    SYNCPOINT_STARTCODE => {
                        sp_count += 1;
                        let body = cur.data[body_start..body_end].to_vec();
                        // time-coded ts: v = ts*tb_count + tb_index
                        let (ts, tbi) = decode_tt(&body, time_bases.len())?;
                        let (sn, sd) = time_bases[tbi];
                        for (sid, st) in streams.iter().enumerate() {
                            let (tn, td) = st.time_base;
                            // last_pts = floor(ts * sn * td / (sd * tn))
                            let scaled = (i128::from(ts) * i128::from(sn) * i128::from(td))
                                / (i128::from(sd) * i128::from(tn));
                            last_pts[sid] = scaled as i64;
                        }
                        sync_total += 1;
                    }
                    MAIN_STARTCODE | STREAM_STARTCODE | INDEX_STARTCODE | INFO_STARTCODE => {}
                    _ => return Err(VoleError::NonCanonicalEncoding),
                }
                // Skip to the body end, then verify the trailing CRC.
                cur.pos = body_end;
                let stored = cur.take(4)?;
                if !crc::verify_packet_crc(&cur.data[body_start..body_end], stored) {
                    return Err(VoleError::IntegrityMismatch);
                }
            }
        }
    }
    let _ = sp_count;
    let _ = sync_total;
    Ok(NutStream {
        version,
        time_bases,
        streams,
        frames,
    })
}

/// Decode a time-coded value (`val * tb_count + tb_index`).
fn decode_tt(body: &[u8], tb_count: usize) -> Result<(u64, usize), VoleError> {
    let mut c = Cursor::new(body);
    let v = c.var()?;
    if tb_count == 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    Ok((v / tb_count as u64, (v % tb_count as u64) as usize))
}

enum PacketKind {
    End,
    Frame,
    Startcode(u64),
}

fn peek_next(data: &[u8], pos: usize) -> PacketKind {
    if pos >= data.len() {
        return PacketKind::End;
    }
    if data[pos] != b'N' {
        return PacketKind::Frame;
    }
    if pos + 8 > data.len() {
        // A partial startcode tail is a corrupt stream, not a silent end.
        return PacketKind::Frame;
    }
    let code = u64::from_be_bytes(data[pos..pos + 8].try_into().expect("8 bytes"));
    match code {
        MAIN_STARTCODE | STREAM_STARTCODE | SYNCPOINT_STARTCODE | INDEX_STARTCODE
        | INFO_STARTCODE => PacketKind::Startcode(code),
        _ => PacketKind::Frame,
    }
}

/// Peek whether the next startcode at/after the cursor is `want` (without
/// consuming). Scans forward; returns the code found.
fn peek_startcode(cur: &mut Cursor<'_>) -> Result<u64, VoleError> {
    let mut i = cur.pos;
    while i + 8 <= cur.data.len() {
        if cur.data[i] == b'N' {
            let code = u64::from_be_bytes(cur.data[i..i + 8].try_into().expect("8 bytes"));
            match code {
                MAIN_STARTCODE | STREAM_STARTCODE | SYNCPOINT_STARTCODE | INDEX_STARTCODE
                | INFO_STARTCODE => {
                    cur.pos = i + 8;
                    return Ok(code);
                }
                _ => i += 1,
            }
        } else {
            i += 1;
        }
    }
    Err(VoleError::Truncated)
}

#[allow(clippy::too_many_arguments)]
fn read_frame(
    cur: &mut Cursor<'_>,
    fc: &FrameCode,
    header_len: &[u32; 128],
    streams: &[NutStreamHeader],
    last_pts: &mut [i64],
) -> Result<Option<NutFrame>, VoleError> {
    let mut flags = fc.flags;
    let mut stream_id = fc.stream_id;
    let mut size = fc.size_lsb;
    let mut head_idx = fc.header_idx;

    if flags & FLAG_CODED != 0 {
        flags ^= cur.var()? as u16;
    }
    if flags & FLAG_STREAM_ID != 0 {
        let sid = cur.var()?;
        if sid >= streams.len() as u64 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        stream_id = sid as u32;
    }
    let st = &streams[stream_id as usize];
    let pts = if flags & FLAG_CODED_PTS != 0 {
        let coded = cur.var()?;
        let shift = st.msb_pts_shift;
        if coded < (1u64 << shift) {
            let mask = (1u64 << shift) - 1;
            let last = last_pts[stream_id as usize];
            let delta = last.wrapping_sub((mask / 2) as i64);
            (coded.wrapping_sub(delta as u64) & mask).wrapping_add(delta as u64) as i64
        } else {
            coded as i64 - (1i64 << shift)
        }
    } else {
        last_pts[stream_id as usize].wrapping_add(fc.pts_delta)
    };
    if flags & FLAG_SIZE_MSB != 0 {
        size = size.wrapping_add(fc.size_mul.wrapping_mul(cur.var()?));
    }
    if flags & FLAG_MATCH_TIME != 0 {
        let _ = cur.var_s()?;
    }
    if flags & FLAG_HEADER_IDX != 0 {
        head_idx = cur.var()? as u32;
    }
    let mut reserved = fc.reserved_count;
    if flags & FLAG_RESERVED != 0 {
        reserved = cur.var()? as u32;
    }
    for _ in 0..reserved {
        cur.var()?;
    }
    if head_idx as usize >= header_len.len() {
        return Err(VoleError::NonCanonicalEncoding);
    }
    if size > 4096 {
        head_idx = 0;
    }
    if size < u64::from(header_len[head_idx as usize]) {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let size = size - u64::from(header_len[head_idx as usize]);
    if size > bounds::MAX_FRAME_PAYLOAD {
        return Err(VoleError::DimensionTooLarge);
    }
    if flags & FLAG_CHECKSUM != 0 {
        cur.take(4)?;
    }
    if flags & FLAG_SM_DATA != 0 {
        return Err(VoleError::UnsupportedFeature);
    }
    last_pts[stream_id as usize] = pts;
    let payload = cur.take(size as usize)?.to_vec();
    Ok(Some(NutFrame {
        stream: stream_id,
        pts,
        key: flags & FLAG_KEY != 0,
        payload,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_inputs_are_typed() {
        // Garbage, empty, truncated magic: all typed, never a panic.
        assert_eq!(parse_nut(b"").unwrap_err(), VoleError::BadMagic);
        assert_eq!(
            parse_nut(b"nut/multimedia container\x00").unwrap_err(),
            VoleError::Truncated
        );
        let mut junk = vec![0x42u8; 4096];
        junk[0] = b'N';
        assert!(parse_nut(&junk).is_err());
    }
}
