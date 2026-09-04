//! Archive profile — Phase T (master brief §67, the Phase-T block of §64).
//!
//! An **archive** of a standalone `.vole` stream is the stream plus a
//! self-describing, self-authenticating **archive manifest** (`.volea`, a
//! deterministic manual-wire sidecar — never part of the `.vole` grammar). The
//! manifest provides:
//!
//! * **self-description** — format version, universe binding, limits profile,
//!   feature bits, pixel format, canvas geometry, frame count, stream length,
//!   and the whole-stream BLAKE3 digest;
//! * **record index** — every top-level record (header, object/palette
//!   declaration, checkpoint, each interval group, integrity trailer) with its
//!   offset, length, kind, interval time / object id, and BLAKE3 digest;
//!   structural verification compares these digests so a corrupted byte
//!   localizes to its record **without any raster decode**;
//! * **object hashes** — the immutable content identity (BLAKE3 over the
//!   canonical object record) of every declared object, by id;
//! * **checkpoint hash** — the BLAKE3 digest of the stream prefix through the
//!   checkpoint record (the interval-0 state's canonical representation);
//! * **frame hashes** — the canonical reconstruction hashes: BLAKE3 of every
//!   materialized full-frame raster, in timeline order (§67 "expected
//!   reconstruction hashes"). A *different representation* that decodes to the
//!   identical raster sequence has identical frame hashes, so frame hashes are
//!   the golden equivalence oracle across encodings and future decoders.
//!
//! Verification runs in three independent layers, so failures are typed and
//! localized:
//!
//! * stream validity (typed `VoleError`);
//! * structural: record digests + self-description + object/checkpoint hashes
//!   (no raster decode);
//! * deep (optional): frame-hash verification (one bounded decode pass).
//!
//! The manifest is a **hostile input like any other**: canonical manual wire,
//! bounded counts, checked arithmetic, and its own trailing BLAKE3 digest
//! (decode refuses a tampered manifest).
//!
//! Long-term universe versioning: the manifest pins the stream's format
//! version / universe / limits profile; a future decoder with a different
//! universe refuses (`UnsupportedUniverse` at stream decode) or reports the
//! pinned binding through the manifest, so an archive never silently changes
//! meaning.
//!
//! Boundary: archiving is defined for **standalone** streams (no external
//! object declarations, Phase P); frame hashes require the objects, which a
//! standalone archive must carry.

use crate::{
    checked::{ByteReader, ByteSink},
    decoder,
    error::VoleError,
    format::ParsedStream,
    identity, integr,
    limits::Limits,
    pixel::Canvas,
};

/// Self-identifying magic of an archive manifest sidecar.
pub const MANIFEST_MAGIC: &[u8; 8] = b"VOLEARC1";
/// Manifest schema version (frozen at 1; unknown versions fail closed).
pub const MANIFEST_VERSION: u32 = 1;
/// Canonical pixel-format code carried by the self-description (Gray8).
pub const PIXEL_GRAY8: u8 = 0;

// Top-level record kinds of a `.vole` stream (the archive index granularity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    /// The 24-byte canonical stream header.
    Header,
    /// An immutable object declaration (fill / raster / index / generator /
    /// external).
    Object,
    /// A palette-table declaration.
    Palette,
    /// The checkpoint (interval-0 state), plain or palette-binding variant.
    Checkpoint,
    /// One interval group (`t` and its transitions).
    Interval,
    /// The 32-byte BLAKE3 integrity trailer.
    Integrity,
}

impl RecordKind {
    fn wire(self) -> u8 {
        match self {
            RecordKind::Header => 0,
            RecordKind::Object => 1,
            RecordKind::Palette => 2,
            RecordKind::Checkpoint => 3,
            RecordKind::Interval => 4,
            RecordKind::Integrity => 5,
        }
    }
    fn from_wire(b: u8) -> Option<RecordKind> {
        match b {
            0 => Some(RecordKind::Header),
            1 => Some(RecordKind::Object),
            2 => Some(RecordKind::Palette),
            3 => Some(RecordKind::Checkpoint),
            4 => Some(RecordKind::Interval),
            5 => Some(RecordKind::Integrity),
            _ => None,
        }
    }
    /// Stable label for reports.
    pub fn label(self) -> &'static str {
        match self {
            RecordKind::Header => "header",
            RecordKind::Object => "object",
            RecordKind::Palette => "palette",
            RecordKind::Checkpoint => "checkpoint",
            RecordKind::Interval => "interval",
            RecordKind::Integrity => "integrity",
        }
    }
}

/// One top-level record of the indexed stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordRef {
    /// Record kind.
    pub kind: RecordKind,
    /// Ordinal of the record in the stream (0-based).
    pub index: u32,
    /// Byte offset of the record's first byte in the stream.
    pub offset: u64,
    /// Record length in bytes.
    pub length: u64,
    /// BLAKE3 digest of the record's bytes.
    pub digest: [u8; 32],
    /// Object / palette id for declarations, else `None`.
    pub id: Option<u32>,
    /// Absolute interval time for interval records, else `None`.
    pub t: Option<u64>,
}

/// The self-description half of a manifest: what a decoder must agree with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfDescription {
    /// Format version of the archived stream.
    pub format_version: u16,
    /// Universe binding of the archived stream.
    pub universe_id: u32,
    /// Limits profile of the archived stream.
    pub limit_profile: u8,
    /// Feature bits of the archived stream.
    pub feature_bits: u32,
    /// Canvas width (samples per row).
    pub width: u32,
    /// Canvas height (rows).
    pub height: u32,
    /// Canonical pixel format code ([`PIXEL_GRAY8`]).
    pub pixel_code: u8,
    /// Number of materializable frames (checkpoint + intervals).
    pub frame_count: u64,
    /// Total `.vole` stream bytes (including the integrity trailer).
    pub stream_len: u64,
    /// BLAKE3 digest of the entire `.vole` stream (including the trailer).
    pub stream_digest: [u8; 32],
}

/// Field of the self-description, for precise mismatch reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfField {
    /// `stream_len`.
    StreamLength,
    /// `stream_digest`.
    StreamDigest,
    /// `format_version`.
    FormatVersion,
    /// `universe_id`.
    Universe,
    /// `limit_profile`.
    LimitProfile,
    /// `feature_bits`.
    FeatureBits,
    /// `width`.
    Width,
    /// `height`.
    Height,
    /// `pixel_code`.
    PixelFormat,
    /// `frame_count`.
    FrameCount,
}

impl SelfField {
    /// Stable label for reports.
    pub fn label(self) -> &'static str {
        match self {
            SelfField::StreamLength => "stream_len",
            SelfField::StreamDigest => "stream_digest",
            SelfField::FormatVersion => "format_version",
            SelfField::Universe => "universe_id",
            SelfField::LimitProfile => "limit_profile",
            SelfField::FeatureBits => "feature_bits",
            SelfField::Width => "width",
            SelfField::Height => "height",
            SelfField::PixelFormat => "pixel_format",
            SelfField::FrameCount => "frame_count",
        }
    }
}

/// One declared object's immutable content identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectHash {
    /// Declared object id.
    pub id: u32,
    /// Content identity (BLAKE3 over the canonical object record).
    pub content_id: [u8; 32],
}

/// An archive manifest over one standalone `.vole` stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveManifest {
    /// Self-description of the archived stream.
    pub stream: SelfDescription,
    /// BLAKE3 of the stream prefix through the checkpoint record (the
    /// interval-0 state's canonical bytes).
    pub checkpoint_digest: [u8; 32],
    /// Record index (top-level records in stream order).
    pub records: Vec<RecordRef>,
    /// Object content identities by declared id.
    pub objects: Vec<ObjectHash>,
    /// Canonical reconstruction hashes: BLAKE3 of each materialized full frame.
    pub frame_hashes: Vec<[u8; 32]>,
}

impl ArchiveManifest {
    /// Build the archive manifest of a standalone stream: validates the
    /// stream, indexes its records, decodes every frame once to compute the
    /// canonical reconstruction hashes, and records object / checkpoint
    /// hashes. Store-backed (external-object) streams are refused typed
    /// (their payloads are not in the file, so a standalone archive cannot
    /// hold them).
    pub fn build(bytes: &[u8]) -> Result<ArchiveManifest, VoleError> {
        // External-object streams are refused before decode (decoding them
        // requires the store). Quantized-content declarations (Phase U bit
        // 0x2) are standalone and archive normally.
        let (_, _, _, feat, _, _) = raw_header_fields(bytes)?;
        if feat & crate::format::FEAT_EXTERNAL_OBJECTS != 0 {
            return Err(VoleError::ApiConstraint(
                "archiving requires a standalone stream (no external objects)",
            ));
        }
        let parsed = decoder::decode_bytes(bytes)?;
        let header = parsed.header();
        let records = scan_stream(bytes)?;
        let frame_hashes = compute_frame_hashes(&parsed)?;
        let checkpoint_end = records
            .iter()
            .find(|r| r.kind == RecordKind::Checkpoint)
            .map(|r| r.offset + r.length)
            .ok_or(VoleError::NonCanonicalEncoding)?;
        let checkpoint_digest = integr::digest(&bytes[..checkpoint_end as usize]);
        let objects = initial_objects(&parsed);
        let stream = SelfDescription {
            format_version: header.format_version(),
            universe_id: header.universe_id(),
            limit_profile: header.limit_profile(),
            feature_bits: header.feature_bits(),
            width: parsed.width(),
            height: parsed.height(),
            pixel_code: PIXEL_GRAY8,
            frame_count: parsed.frame_count(),
            stream_len: bytes.len() as u64,
            stream_digest: integr::digest(bytes),
        };
        Ok(ArchiveManifest {
            stream,
            checkpoint_digest,
            records,
            objects,
            frame_hashes,
        })
    }

    /// The manifest's pinned universe binding.
    pub fn universe_id(&self) -> u32 {
        self.stream.universe_id
    }

    /// The manifest's pinned format version.
    pub fn format_version(&self) -> u16 {
        self.stream.format_version
    }
}

/// Content identities of every declared object, in id order.
fn initial_objects(parsed: &ParsedStream) -> Vec<ObjectHash> {
    parsed
        .clone_initial()
        .objects()
        .map(|(id, obj)| ObjectHash {
            id: id.0,
            content_id: *identity::content_id_of(obj).as_bytes(),
        })
        .collect()
}

/// Canonical reconstruction hashes: replay the timeline and BLAKE3 every
/// materialized full frame (frame 0 = checkpoint). One bounded decode pass.
pub fn compute_frame_hashes(parsed: &ParsedStream) -> Result<Vec<[u8; 32]>, VoleError> {
    let limits = parsed.limits();
    let w = parsed.width();
    let h = parsed.height();
    let mut state = parsed.clone_initial();
    let mut out = Vec::with_capacity(parsed.frame_count() as usize);
    let first = crate::materialize::materialize_full(&state, w, h, limits)?;
    out.push(hash_canvas(&first));
    let mut prev = first;
    for (_, trs) in parsed.intervals() {
        let canvas = decoder::step_frame(&mut state, &prev, trs, w, h, limits)?;
        out.push(hash_canvas(&canvas));
        prev = canvas;
    }
    Ok(out)
}

/// BLAKE3 of a canonical Gray8 raster (frame reconstruction hash).
pub fn hash_canvas(canvas: &Canvas) -> [u8; 32] {
    integr::digest(canvas.as_slice())
}

// ---------------------------------------------------------------------------
// Record scanning (offsets, kinds, digests)
// ---------------------------------------------------------------------------

/// Walk the top-level records of a `.vole` stream and digest each one. The
/// walker is length-driven and needs no object store; it returns typed errors
/// on structurally unparseable input (never panics, never allocates from
/// untrusted lengths). This is the corruption-localization primitive: for a
/// grammatically intact file, every byte flip changes exactly one record
/// digest.
pub fn scan_stream(bytes: &[u8]) -> Result<Vec<RecordRef>, VoleError> {
    if bytes.len() < integr::DIGEST_LEN + 24 {
        return Err(VoleError::Truncated);
    }
    let limits = Limits::default();
    let mut records: Vec<RecordRef> = Vec::new();
    let mut off = 0usize;
    let end = bytes.len() - integr::DIGEST_LEN;
    // Header record (fixed 24 bytes).
    push_record(&mut records, RecordKind::Header, bytes, off, 24, None, None);
    off += 24;
    // Declarations, checkpoint, and interval groups until the trailer.
    while off < end {
        let rec_start = off;
        let tag = *bytes.get(off).ok_or(VoleError::Truncated)?;
        // The reader starts after the tag byte; record length includes it.
        let mut r = ByteReader::new(&bytes[off + 1..]);
        let remaining_before = r.remaining();
        let mut id: Option<u32> = None;
        let mut t: Option<u64> = None;
        let kind = match tag {
            0x01 | 0x05 => {
                // raster / palette-index object: id w h payload(w*h)
                let oid = r.pull::<u32>()?;
                let w = u64::from(r.pull::<u32>()?);
                let h = u64::from(r.pull::<u32>()?);
                let n = w.checked_mul(h).ok_or(VoleError::ArithmeticOverflow)?;
                if n > limits.max_object_bytes {
                    return Err(VoleError::DimensionTooLarge);
                }
                let n = usize::try_from(n).map_err(|_| VoleError::ArithmeticOverflow)?;
                r.take(n)?;
                id = Some(oid);
                RecordKind::Object
            }
            0x02 => {
                // fill object: id w h value
                let oid = r.pull::<u32>()?;
                let w = u64::from(r.pull::<u32>()?);
                let h = u64::from(r.pull::<u32>()?);
                if w.checked_mul(h).ok_or(VoleError::ArithmeticOverflow)? > limits.max_object_bytes
                {
                    return Err(VoleError::DimensionTooLarge);
                }
                r.u8()?;
                id = Some(oid);
                RecordKind::Object
            }
            0x07 => {
                // generator object: id w h program (parsed to bound the skip)
                let oid = r.pull::<u32>()?;
                let w = u64::from(r.pull::<u32>()?);
                let h = u64::from(r.pull::<u32>()?);
                if w.checked_mul(h).ok_or(VoleError::ArithmeticOverflow)? > limits.max_object_bytes
                {
                    return Err(VoleError::DimensionTooLarge);
                }
                let _gen = crate::generator::Generator::parse_program(&mut r)?;
                id = Some(oid);
                RecordKind::Object
            }
            0x09 => {
                // external object declaration: id content_id(32)
                let oid = r.pull::<u32>()?;
                r.take(32)?;
                id = Some(oid);
                RecordKind::Object
            }
            0x06 => {
                // palette declaration: id len entries
                let pid = r.pull::<u32>()?;
                let len = u64::from(r.pull::<u32>()?);
                if len == 0 || len > u64::from(limits.max_palette_entries) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                r.take(len as usize)?;
                id = Some(pid);
                RecordKind::Palette
            }
            0x03 | 0x08 => {
                // checkpoint (plain / palette-binding): bg count instances
                let with_bindings = tag == 0x08;
                r.u8()?;
                let n = u64::from(r.pull::<u32>()?);
                if n > u64::from(limits.max_instances) {
                    return Err(VoleError::DimensionTooLarge);
                }
                let per = if with_bindings { 20u64 } else { 16u64 };
                let skip = n.checked_mul(per).ok_or(VoleError::ArithmeticOverflow)?;
                r.take(skip as usize)?;
                RecordKind::Checkpoint
            }
            0x04 => {
                // interval group: t count transitions
                let ti = r.pull::<u64>()?;
                t = Some(ti);
                let n = u64::from(r.pull::<u32>()?);
                if n > u64::from(limits.max_transitions_per_interval) {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
                for k in 0..n {
                    skip_transition(&mut r, &limits)?;
                    let _ = k;
                }
                RecordKind::Interval
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        };
        let consumed = remaining_before - r.remaining();
        if consumed == 0 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        // `consumed` covers the payload after the tag byte; the record length
        // includes the tag byte itself.
        let rec_len = consumed + 1;
        off = rec_start + rec_len;
        if off > end {
            return Err(VoleError::Truncated);
        }
        push_record(&mut records, kind, bytes, rec_start, rec_len, id, t);
    }
    // The final 32 bytes are the integrity trailer.
    if bytes.len() - off != integr::DIGEST_LEN {
        return Err(VoleError::NonCanonicalEncoding);
    }
    push_record(
        &mut records,
        RecordKind::Integrity,
        bytes,
        off,
        integr::DIGEST_LEN,
        None,
        None,
    );
    Ok(records)
}

fn push_record(
    records: &mut Vec<RecordRef>,
    kind: RecordKind,
    bytes: &[u8],
    offset: usize,
    length: usize,
    id: Option<u32>,
    t: Option<u64>,
) {
    let digest = integr::digest(&bytes[offset..offset + length]);
    records.push(RecordRef {
        kind,
        index: records.len() as u32,
        offset: offset as u64,
        length: length as u64,
        digest,
        id,
        t,
    });
}

/// Consume one transition record from the reader (length mirror of the v1
/// transition grammar; bounds match the parser so work stays in the decode
/// envelope). No semantic construction — the archive walker only needs record
/// boundaries.
fn skip_transition(r: &mut ByteReader<'_>, limits: &Limits) -> Result<(), VoleError> {
    let tag = r.u8()?;
    match tag {
        0x21 => {
            // create instance: id object x y
            r.take(16)?;
        }
        0x22 | 0x26 => {
            // set position / set velocity: id x y (id vx vy)
            r.take(12)?;
        }
        0x27 | 0x2c | 0x28 | 0x29 => {
            // advance translations / advance trajectories / clear instances /
            // clear overlay: no payload
        }
        0x23 => {
            // patch sparse: count + 9 bytes per point
            let m = u64::from(r.pull::<u32>()?);
            if m > limits.max_canvas_bytes {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let n = m.checked_mul(9).ok_or(VoleError::ArithmeticOverflow)?;
            r.take(n as usize)?;
        }
        0x2a => {
            // residual: len + block
            let len = u64::from(r.pull::<u32>()?);
            if len > limits.max_residual_bytes {
                return Err(VoleError::DimensionTooLarge);
            }
            r.take(len as usize)?;
        }
        0x2b => {
            // set trajectory: id count segments
            r.take(4)?;
            let n = u64::from(r.pull::<u32>()?);
            if n > u64::from(limits.max_trajectory_segments) {
                return Err(VoleError::MaterializationBudgetExceeded);
            }
            for _ in 0..n {
                match r.u8()? {
                    crate::trajectory::SEG_LINEAR => {
                        r.take(16)?;
                    }
                    crate::trajectory::SEG_ACCEL => {
                        r.take(24)?;
                    }
                    _ => return Err(VoleError::NonCanonicalEncoding),
                }
            }
        }
        0x2d => {
            // set palette: id len entries
            r.take(4)?;
            let len = u64::from(r.pull::<u32>()?);
            if len == 0 || len > u64::from(limits.max_palette_entries) {
                return Err(VoleError::NonCanonicalEncoding);
            }
            r.take(len as usize)?;
        }
        0x2e => {
            // patch palette: id count (idx, value) pairs
            r.take(4)?;
            let m = u64::from(r.pull::<u32>()?);
            if m > 256 {
                return Err(VoleError::NonCanonicalEncoding);
            }
            r.take((m * 2) as usize)?;
        }
        0x2f => {
            // bind palette: instance palette
            r.take(8)?;
        }
        0x30 => {
            // set affine: id + six Q8 coefficients (i32 each)
            r.take(28)?;
        }
        0x24 | 0x25 => {
            // copy / move rect: src_x src_y width height dst_x dst_y (all
            // i32/u32 four-byte fields); validate the real width/height.
            r.take(8)?; // src_x src_y
            let width = r.pull::<u32>()?;
            let height = r.pull::<u32>()?;
            if width == 0 || height == 0 {
                return Err(VoleError::NonCanonicalEncoding);
            }
            if u64::from(width) * u64::from(height) > limits.max_copy_area {
                return Err(VoleError::MaterializationBudgetExceeded);
            }
            r.take(8)?; // dst_x dst_y
        }
        _ => return Err(VoleError::NonCanonicalEncoding),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// Outcome status of a verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyStatus {
    /// Stream and manifest agree on every layer (deep or structural).
    Complete,
    /// A self-description field disagrees with the stream.
    SelfDescriptionMismatch,
    /// A record digest / count disagrees with the manifest.
    StructuralMismatch,
    /// The whole-stream digest disagrees while every record digest matches
    /// (trailer corruption, or a manifest built from a byte-identical-prefix
    /// stream).
    StreamDigestMismatch,
    /// An object content identity disagrees.
    ObjectMismatch,
    /// Deep verification found a frame whose reconstruction hash disagrees.
    FrameDivergence,
}

/// Result of [`verify`]. Structural layers run on the bytes alone, so a
/// corrupted stream is *reported* (with its first bad record) instead of
/// failing the whole verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// Overall status.
    pub status: VerifyStatus,
    /// Whether the stream parsed cleanly (its trailer digest held).
    pub decode_ok: bool,
    /// Whether every self-description field agreed.
    pub self_description_ok: bool,
    /// First self-description field that disagreed, if any.
    pub mismatch_field: Option<SelfField>,
    /// Whether every record digest (and count) agreed.
    pub structural_ok: bool,
    /// Number of records compared.
    pub records_checked: u64,
    /// Index (into the manifest's record list) of the first disagreeing
    /// record, if any. The record's kind/offset/t are available through
    /// `manifest.records[i]` — this is the byte-level corruption
    /// localization.
    pub first_bad_record: Option<u32>,
    /// Whether record counts agreed.
    pub record_count_ok: bool,
    /// Whether the whole-stream digest agreed.
    pub stream_digest_ok: bool,
    /// Whether the checkpoint-prefix digest agreed (implied by the record
    /// digests; kept as an independent signal).
    pub checkpoint_ok: bool,
    /// Whether every object content identity agreed (decode-ok only).
    pub objects_ok: bool,
    /// Whether deep frame-hash verification ran.
    pub deep: bool,
    /// Number of frames whose reconstruction hash matched before the first
    /// divergence (deep only).
    pub frames_checked: u64,
    /// First frame whose reconstruction hash disagreed (deep only).
    pub first_frame_divergence: Option<u64>,
}

impl VerifyReport {
    /// The offending record (kind + stream position) when a record mismatch
    /// was found.
    pub fn first_bad_record_ref<'a>(
        &'a self,
        manifest: &'a ArchiveManifest,
    ) -> Option<&'a RecordRef> {
        self.first_bad_record
            .and_then(|i| manifest.records.get(i as usize))
    }
}

/// Raw header fields at their canonical offsets (for decode-independent
/// self-description comparison on corrupted streams). No validation: the
/// values are only compared against the manifest.
fn raw_header_fields(bytes: &[u8]) -> Result<(u16, u32, u8, u32, u32, u32), VoleError> {
    if bytes.len() < 24 {
        return Err(VoleError::Truncated);
    }
    let f = |off: usize, n: usize| -> Result<u64, VoleError> {
        let mut b = [0u8; 8];
        b[..n].copy_from_slice(&bytes[off..off + n]);
        Ok(u64::from_le_bytes(b))
    };
    let format_version = f(5, 2)? as u16;
    let universe_id = f(7, 4)? as u32;
    let limit_profile = bytes[11];
    let feature_bits = f(12, 4)? as u32;
    let width = f(16, 4)? as u32;
    let height = f(20, 4)? as u32;
    Ok((
        format_version,
        universe_id,
        limit_profile,
        feature_bits,
        width,
        height,
    ))
}

/// Verify a standalone `.vole` stream against its archive manifest.
///
/// Layers, in order (each runs on the bytes alone until decode is required):
///
/// 1. **self-description** — raw header fields, stream length, whole-stream
///    digest, pixel format;
/// 2. **structural** — per-record digests (byte-level corruption
///    localization, no raster work) and the checkpoint digest;
/// 3. **decode** — the stream is parsed (a structurally pristine stream
///    always parses); object content identities are compared;
/// 4. **deep** (optional) — materialize the timeline once and compare every
///    frame's reconstruction hash, stopping at the first divergence.
///
/// A corrupted stream never aborts verification: it is reported with the
/// first disagreeing record (kind, offset) or field.
pub fn verify(
    bytes: &[u8],
    manifest: &ArchiveManifest,
    deep: bool,
) -> Result<VerifyReport, VoleError> {
    let stream = &manifest.stream;

    // Layer 1: self-description (decode-independent). Only the header-carried
    // semantic fields are reported as self-description mismatches; derived
    // totals (stream length, whole-stream digest, frame count) are kept as
    // separate report signals so record-level localization always surfaces.
    let (fv, univ, prof, feat, w, h) = raw_header_fields(bytes)?;
    let mut mismatch_field = None;
    let mut sd_ok = true;
    for (field, ok) in [
        (SelfField::FormatVersion, fv == stream.format_version),
        (SelfField::Universe, univ == stream.universe_id),
        (SelfField::LimitProfile, prof == stream.limit_profile),
        (SelfField::FeatureBits, feat == stream.feature_bits),
        (SelfField::Width, w == stream.width),
        (SelfField::Height, h == stream.height),
        (SelfField::PixelFormat, PIXEL_GRAY8 == stream.pixel_code),
    ] {
        if !ok {
            sd_ok = false;
            mismatch_field = Some(field);
            break;
        }
    }

    // Layer 2: record digests (byte-level localization).
    let scanned = scan_stream(bytes)?;
    let mut structural_ok = true;
    let mut first_bad = None;
    let count_ok = scanned.len() == manifest.records.len();
    let mut checked = 0u64;
    for (a, b) in scanned.iter().zip(manifest.records.iter()) {
        if a.kind != b.kind
            || a.offset != b.offset
            || a.length != b.length
            || a.digest != b.digest
            || a.id != b.id
            || a.t != b.t
        {
            structural_ok = false;
            first_bad = Some(checked as u32);
            break;
        }
        checked += 1;
    }
    if !count_ok && first_bad.is_none() {
        structural_ok = false;
    }

    // Layer 2b: checkpoint digest (the interval-0 state's canonical bytes).
    // Kept as a report-level signal; it is implied by the record digests
    // (records tile the prefix), so it never overrides record localization.
    let checkpoint_end = scanned
        .iter()
        .find(|r| r.kind == RecordKind::Checkpoint)
        .map(|r| r.offset + r.length)
        .ok_or(VoleError::NonCanonicalEncoding)?;
    let checkpoint_ok =
        integr::digest(&bytes[..checkpoint_end as usize]) == manifest.checkpoint_digest;

    // Layer 3: decode (objects cross-check). A structurally pristine stream
    // always parses; a corrupted one is reported above.
    let parsed = decoder::decode_bytes(bytes);
    let mut decode_ok = false;
    let mut objects_ok = false;
    if let Ok(parsed) = &parsed {
        decode_ok = true;
        objects_ok = initial_objects(parsed) == manifest.objects;
    }

    // Layer 4: deep frame-hash verification (one decode pass, early exit).
    let mut frames_checked = 0u64;
    let mut first_frame_divergence = None;
    if deep && decode_ok && structural_ok && sd_ok && objects_ok {
        let parsed = parsed.as_ref().expect("decode ok");
        let limits = parsed.limits();
        let w = parsed.width();
        let h = parsed.height();
        let mut state = parsed.clone_initial();
        let first = crate::materialize::materialize_full(&state, w, h, limits)?;
        let mut prev = first;
        let mut expect = manifest.frame_hashes.iter();
        if let Some(expected) = expect.next() {
            if *expected != hash_canvas(&prev) {
                first_frame_divergence = Some(0);
            } else {
                frames_checked = 1;
            }
        }
        for (_, trs) in parsed.intervals() {
            if first_frame_divergence.is_some() {
                break;
            }
            let canvas = decoder::step_frame(&mut state, &prev, trs, w, h, limits)?;
            match expect.next() {
                Some(expected) if *expected == hash_canvas(&canvas) => {
                    frames_checked += 1;
                }
                _ => {
                    first_frame_divergence = Some(frames_checked.saturating_add(1));
                    break;
                }
            }
            prev = canvas;
        }
    }

    let stream_digest_ok = integr::digest(bytes) == stream.stream_digest;
    let status = if !sd_ok {
        VerifyStatus::SelfDescriptionMismatch
    } else if !structural_ok {
        VerifyStatus::StructuralMismatch
    } else if !decode_ok {
        VerifyStatus::StreamDigestMismatch
    } else if !objects_ok {
        VerifyStatus::ObjectMismatch
    } else if !stream_digest_ok {
        VerifyStatus::StreamDigestMismatch
    } else if deep && first_frame_divergence.is_some() {
        VerifyStatus::FrameDivergence
    } else {
        VerifyStatus::Complete
    };
    Ok(VerifyReport {
        status,
        decode_ok,
        self_description_ok: sd_ok,
        mismatch_field,
        structural_ok,
        records_checked: checked,
        first_bad_record: first_bad,
        record_count_ok: count_ok,
        stream_digest_ok,
        checkpoint_ok,
        objects_ok,
        deep,
        frames_checked,
        first_frame_divergence,
    })
}

// ---------------------------------------------------------------------------
// Manifest wire (manual, canonical, self-authenticating)
// ---------------------------------------------------------------------------

/// Encode the manifest canonically (`.volea` bytes). The trailing BLAKE3
/// digest commits to every preceding byte; decode refuses a tampered
/// manifest.
pub fn encode(manifest: &ArchiveManifest) -> Result<Vec<u8>, VoleError> {
    let mut s = ByteSink::new();
    s.extend(MANIFEST_MAGIC)?;
    s.push(MANIFEST_VERSION)?;
    // Self-description.
    let sd = &manifest.stream;
    s.push(sd.format_version)?;
    s.push(sd.universe_id)?;
    s.byte(sd.limit_profile)?;
    s.push(sd.feature_bits)?;
    s.push(sd.width)?;
    s.push(sd.height)?;
    s.byte(sd.pixel_code)?;
    s.push(sd.frame_count)?;
    s.push(sd.stream_len)?;
    s.extend(&sd.stream_digest)?;
    s.extend(&manifest.checkpoint_digest)?;
    // Records.
    let rc = u32::try_from(manifest.records.len()).map_err(|_| VoleError::ArithmeticOverflow)?;
    s.push(rc)?;
    for r in &manifest.records {
        s.byte(r.kind.wire())?;
        s.push(r.id.unwrap_or(0))?;
        s.push(r.t.unwrap_or(0))?;
        s.push(r.offset)?;
        s.push(r.length)?;
        s.extend(&r.digest)?;
    }
    // Objects.
    let oc = u32::try_from(manifest.objects.len()).map_err(|_| VoleError::ArithmeticOverflow)?;
    s.push(oc)?;
    for o in &manifest.objects {
        s.push(o.id)?;
        s.extend(&o.content_id)?;
    }
    // Frame reconstruction hashes.
    let fc =
        u32::try_from(manifest.frame_hashes.len()).map_err(|_| VoleError::ArithmeticOverflow)?;
    s.push(fc)?;
    for h in &manifest.frame_hashes {
        s.extend(h)?;
    }
    // Self-authenticate.
    let d = integr::digest(s.as_slice());
    s.extend(&d)?;
    Ok(s.into_vec())
}

/// Decode a canonical `.volea` manifest (hostile input: bounded counts,
/// checked arithmetic, self-authenticating trailer).
pub fn decode(bytes: &[u8]) -> Result<ArchiveManifest, VoleError> {
    if bytes.len() < integr::DIGEST_LEN {
        return Err(VoleError::Truncated);
    }
    integr::verify_trailer(bytes)?;
    let mut r = ByteReader::new(&bytes[..bytes.len() - integr::DIGEST_LEN]);
    if r.take(MANIFEST_MAGIC.len())? != MANIFEST_MAGIC {
        return Err(VoleError::BadMagic);
    }
    let version = r.pull::<u32>()?;
    if version != MANIFEST_VERSION {
        // Unknown manifest schema: fail closed (a future schema may not be
        // interpretable by this decoder).
        return Err(VoleError::UnsupportedFeature);
    }
    let limits = Limits::default();
    let stream = SelfDescription {
        format_version: r.pull::<u16>()?,
        universe_id: r.pull::<u32>()?,
        limit_profile: r.u8()?,
        feature_bits: r.pull::<u32>()?,
        width: r.pull::<u32>()?,
        height: r.pull::<u32>()?,
        pixel_code: r.u8()?,
        frame_count: r.pull::<u64>()?,
        stream_len: r.pull::<u64>()?,
        stream_digest: read_32(&mut r)?,
    };
    if stream.width == 0
        || stream.height == 0
        || stream.pixel_code != PIXEL_GRAY8
        || stream.frame_count > limits.max_checkpoint_distance + 1
    {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let checkpoint_digest = read_32(&mut r)?;
    let rc = r.pull::<u32>()?;
    // Records are bounded by the manifest's own remaining bytes and by the
    // stream envelope (declarations + one checkpoint + intervals + trailer).
    let max_records = limits.max_checkpoint_distance + 4;
    if u64::from(rc) > max_records || u64::from(rc) > r.remaining() as u64 / 61 {
        return Err(VoleError::DimensionTooLarge);
    }
    let mut records: Vec<RecordRef> = Vec::with_capacity(rc as usize);
    for _ in 0..rc {
        let kind = RecordKind::from_wire(r.u8()?).ok_or(VoleError::NonCanonicalEncoding)?;
        let id_raw = r.pull::<u32>()?;
        let t_raw = r.pull::<u64>()?;
        let offset = r.pull::<u64>()?;
        let length = r.pull::<u64>()?;
        let digest = read_32(&mut r)?;
        if length == 0 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        records.push(RecordRef {
            kind,
            index: records.len() as u32,
            offset,
            length,
            digest,
            id: (id_raw != 0).then_some(id_raw),
            t: (t_raw != 0).then_some(t_raw),
        });
    }
    let oc = r.pull::<u32>()?;
    if u64::from(oc) > u64::from(limits.max_objects) || u64::from(oc) > r.remaining() as u64 / 36 {
        return Err(VoleError::DimensionTooLarge);
    }
    let mut objects = Vec::with_capacity(oc as usize);
    for _ in 0..oc {
        let id = r.pull::<u32>()?;
        let content_id = read_32(&mut r)?;
        objects.push(ObjectHash { id, content_id });
    }
    let fc = r.pull::<u32>()?;
    if u64::from(fc) != stream.frame_count || u64::from(fc) > r.remaining() as u64 / 32 {
        return Err(VoleError::DimensionTooLarge);
    }
    let mut frame_hashes = Vec::with_capacity(fc as usize);
    for _ in 0..fc {
        frame_hashes.push(read_32(&mut r)?);
    }
    if r.remaining() != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    Ok(ArchiveManifest {
        stream,
        checkpoint_digest,
        records,
        objects,
        frame_hashes,
    })
}

fn read_32(r: &mut ByteReader<'_>) -> Result<[u8; 32], VoleError> {
    let raw = r.take(32)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(raw);
    Ok(out)
}
