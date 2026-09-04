//! Phase G/H + K: exhaustive inverse proceduralization (raster-origin encoder).
//!
//! This module builds the first true **inverse proceduralizer**: it accepts an
//! observed Gray8 raster sequence (`Vec<Canvas>`) and, per frame, evaluates an
//! exhaustive candidate space of bounded procedural explanations:
//!
//! ```text
//! RAW · FILL · UNCHANGED · EXACT_OBJECT_REF · SPARSE · COPY_RECT ·
//! TRANSLATION · REGIONS · RANS_RESIDUAL · TRANSFORM_RESIDUAL
//! ```
//!
//! plus the composite programs those families compose into (screen-scroll +
//! residual strip, prev-frame diff). **Phase K added the REGIONS family**
//! (variable granularity 64 → 32 → 16 → 8): the per-frame diff is partitioned
//! into tiles and each diff-bearing tile's *rectangular* bounding box is
//! declared as an immutable object holding the target's own sub-rectangle and
//! painted above the base by a fresh instance — so localized change never
//! needs a whole-canvas declaration, and repeated region content is reused by
//! exact identity with zero declaration bytes. **Phase M added the
//! TRANSFORM_RESIDUAL family**: when the diff is dense the residual field is
//! coded by the normative 4×4 integer lifting DCT (block skip mask + DC/AC
//! rANS streams; residual block kind 2) so smooth, structured deltas that no
//! procedural family explains can still be carried by a conventional
//! transform floor. Every candidate is a
//! *declarative program* over the normative state model — a list of state
//! [`Transition`]s plus a list of canvas ops — and its correctness is
//! established by materializing its expected frame through the same normative
//! primitives the decoder runs (`materialize_full`, `rect_copy`, the Phase-F
//! residual block decode) and comparing byte-for-byte with the target
//! observation. The encoder never trusts a hypothesis from appearance; a
//! candidate that cannot reproduce the target exactly is rejected and
//! recorded, and the complete-cost winner (persisted bytes, §31-style
//! accounting) is the only program emitted. The emitted stream is always
//! verified end-to-end: it is decoded with the normative decoder and every
//! materialized frame must equal the input raster, or the encoder returns a
//! typed error instead of a stream.
//!
//! # Scope honesty
//!
//! The *base* granularity is the whole frame: frame 0 (and any content-wide
//! rebase) declares a full-canvas object. The REGIONS family serves localized
//! change down to 8×8 rectangles; it is evaluated only when the per-frame
//! diff is non-empty and at most a quarter of the canvas (a larger diff is
//! served at least as cheaply by the whole-frame reset sentinel), capped at
//! 256 rectangles per candidate, and skipped when a changed sample is
//! shadowed by a persistent overlay point (overlay paints above every
//! instance). The enumerated candidate space is finite and deterministic per
//! frame; the exact members are documented on [`FrameDecision`]. The full
//! per-candidate materialization court runs on canvases up to a declared size
//! gate; above it, 1D scroll candidates are row-hash-prefiltered (equality is
//! always confirmed with a byte comparison before a candidate is accepted, so
//! exactness never depends on a hash). Per-frame decisions are greedy and
//! independent: temporal re-optimization (velocity/trajectory collapse,
//! checkpoint placement, residual→persistent-content promotion, region
//! instance retirement) is Phase O, and this module reports the temporal gaps
//! it measures rather than hiding them.

use std::collections::HashMap;

use crate::{
    checked::ByteReader,
    decoder,
    dsfb::{DsfbFrameDiag, DsfbModel, EncoderStrategy, FramePlan, Mode},
    encoder::encode_stream,
    error::VoleError,
    integr,
    limits::Limits,
    materialize,
    object::{Object, ObjectId},
    pixel::Canvas,
    rans,
    state::{Instance, InstanceId, State},
    transition::Transition,
};

/// Serialized size of one interval record envelope: `tag(1) + t:u64(8) +
/// count:u32(4)`.
const INTERVAL_ENVELOPE: u64 = 13;
/// Serialized object declaration header: `tag(1) + id(4) + w(4) + h(4)`.
const OBJECT_DECL_HEADER: u64 = 13;
/// Serialized fill object declaration: header + one sample byte.
const FILL_DECL: u64 = 14;

/// Checkpoint record bytes with `n` live instances:
/// `tag(1)+bg(1)+n(4)+n*(iid(4)+oid(4)+x(4)+y(4))`.
fn checkpoint_bytes(n: u64) -> u64 {
    6 + 16 * n
}

/// Serialized payload length (no interval envelope) of one transition.
fn tr_len(tr: &Transition) -> u64 {
    match tr {
        Transition::CreateInstance { .. } => 17,
        Transition::SetPosition { .. } | Transition::SetVelocity { .. } => 13,
        Transition::AdvanceTranslations | Transition::ClearInstances | Transition::ClearOverlay => {
            1
        }
        Transition::AdvanceTrajectories => 1,
        Transition::SetTrajectory { segments, .. } => {
            crate::trajectory::program_wire_bytes(segments)
        }
        Transition::SetPalette { entries, .. } => 9 + entries.len() as u64,
        Transition::PatchPalette { changes, .. } => 9 + 2 * changes.len() as u64,
        Transition::BindPalette { .. } => 9,
        Transition::SetAffine { params, .. } => params.wire_bytes(),
        Transition::PatchSparse { points } => 5 + 9 * points.len() as u64,
        Transition::CopyRect { .. } | Transition::MoveRect { .. } => 25,
        Transition::Residual { block } => 5 + block.len() as u64,
        Transition::DeclareObject(..) | Transition::DeclareFill { .. } => 0,
    }
}

/// Serialized declaration bytes of a never-before-seen object.
fn decl_bytes(_w: u32, _h: u32, content: &Content) -> u64 {
    match content {
        Content::Fill(_) => FILL_DECL,
        Content::Raster(data) => OBJECT_DECL_HEADER + data.len() as u64,
        Content::Generator(gen) => OBJECT_DECL_HEADER + gen.program_bytes().len() as u64,
    }
}

/// Immutable object content (whole-frame granularity in Phase G).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    /// Uniform fill over the declared box.
    Fill(u8),
    /// Tight row-major Gray8 raster.
    Raster(Vec<u8>),
    /// A bounded procedural content program (Phase N): samples are computed
    /// at materialization, never stored.
    Generator(crate::generator::Generator),
}

/// Canonical record bytes for content identity — byte-identical to the object
/// record form `identity::content_id_of` hashes, so the same content reaches
/// the same identity regardless of which path declared it.
fn content_record(w: u32, h: u32, content: &Content) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    match content {
        Content::Fill(v) => {
            out.push(0x02);
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&h.to_le_bytes());
            out.push(*v);
        }
        Content::Raster(data) => {
            out.push(0x01);
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&h.to_le_bytes());
            out.extend_from_slice(data);
        }
        Content::Generator(gen) => {
            out.push(0x07);
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&h.to_le_bytes());
            out.extend_from_slice(&gen.program_bytes());
        }
    }
    out
}

/// BLAKE3 content identity of a candidate object record.
fn content_id(w: u32, h: u32, content: &Content) -> [u8; 32] {
    integr::digest(&content_record(w, h, content))
}

/// Whether a canvas is a uniform fill, returning its value.
fn uniform_value(c: &Canvas) -> Option<u8> {
    let v = *c.as_slice().first()?;
    c.as_slice().iter().all(|&b| b == v).then_some(v)
}

/// The immutable content of a target sub-rectangle (Phase K region): `Fill`
/// when the rectangle is uniform, otherwise its tight row-major `Raster`.
fn rect_content(target: &Canvas, x0: i64, y0: i64, w: u32, h: u32) -> Content {
    let cw = usize::try_from(target.width()).expect("width fits usize");
    let mut first: Option<u8> = None;
    let mut uniform = true;
    let mut data = Vec::with_capacity(usize::try_from(w).unwrap() * usize::try_from(h).unwrap());
    for sy in 0..h as i64 {
        let row = (y0 + sy) as usize * cw + x0 as usize;
        for sx in 0..w as i64 {
            let v = target.as_slice()[row + sx as usize];
            match first {
                None => first = Some(v),
                Some(f) if f != v => uniform = false,
                _ => {}
            }
            data.push(v);
        }
    }
    match (uniform, first) {
        (true, Some(v)) => Content::Fill(v),
        _ => Content::Raster(data),
    }
}

/// Strict-lexicographic sparse diff of `target` over `base`: every coordinate
/// where the two differ, carrying the target value. Iteration is x-major, so
/// the output is already in the canonical wire order (x asc, then y asc).
fn diff_points(base: &Canvas, target: &Canvas) -> Vec<(i64, i64, u8)> {
    let (w, h) = (base.width() as usize, base.height() as usize);
    let (a, b) = (base.as_slice(), target.as_slice());
    let mut out = Vec::new();
    for x in 0..w {
        for y in 0..h {
            let i = y * w + x;
            if a[i] != b[i] {
                out.push((x as i64, y as i64, b[i]));
            }
        }
    }
    out
}

/// Encode a point list as a Phase-F self-describing block (RAW or rANS per the
/// declared accounting policy).
fn encode_point_block(pts: &[(i64, i64, u8)]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(9 * pts.len());
    for (x, y, v) in pts {
        bytes.extend_from_slice(&i32::try_from(*x).unwrap_or(i32::MAX).to_le_bytes());
        bytes.extend_from_slice(&i32::try_from(*y).unwrap_or(i32::MAX).to_le_bytes());
        bytes.push(*v);
    }
    rans::encode_block(&bytes)
}

fn block_is_rans(block: &[u8]) -> bool {
    block.first() == Some(&rans::KIND_RANS)
}

// ---------------------------------------------------------------------------
// Phase N: whole-frame procedural-generator discovery
// ---------------------------------------------------------------------------
// The encoder probes a small deterministic set of *content-derived* generator
// fits (never an unbounded parameter search): a gradient fit measured from
// the origin edges, a checker fit over a bounded cell lattice, and a
// periodic-sawtooth fit over a bounded period lattice. Every fit is first
// spot-checked on O(w+h) samples (cheap prefilter — mirrors the row-hash
// prefilter used for large-canvas copies) and then validated by rendering
// the *normative* generator field and comparing it with the target, so a
// candidate can never win on appearance. Noise is deliberately never fitted:
// discovering a seed by search is unbounded work, and a seed that merely
// relocates the target's bits must not masquerade as a procedural win (§21,
// §62-§63). A fit that passes the prefilter but is not exact is presented as
// a generator+residual candidate with its exact correction counted, but only
// when the fit explains at least 15/16 of the pixels (otherwise RAW is the
// honest floor).

/// Generator fits that pass their spot-check prefilter, in deterministic
/// evaluation order. `probe` restricts the search to the cheap gradient fit
/// (fixed heuristic / DSFB rotating sweep).
pub(crate) fn fit_generators(target: &Canvas, probe: bool) -> Vec<crate::generator::Generator> {
    use crate::generator::Generator;
    let w = target.width() as usize;
    let h = target.height() as usize;
    let s = target.as_slice();
    let at = |x: usize, y: usize| s[y * w + x];
    let wrapdiff = |a: u8, b: u8| (i64::from(a) - i64::from(b)).rem_euclid(256);
    if w < 2 || h < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    // -- gradient -------------------------------------------------------------
    let dx = wrapdiff(at(1, 0), at(0, 0));
    let dy = wrapdiff(at(0, 1), at(0, 0));
    if dx != 0 || dy != 0 {
        let row_ok = (0..w - 1).all(|x| wrapdiff(at(x + 1, 0), at(x, 0)) == dx);
        let col_ok = (0..h - 1).all(|y| wrapdiff(at(0, y + 1), at(0, y)) == dy);
        let diag_ok = wrapdiff(at(1, 1), at(0, 0)) == (dx + dy).rem_euclid(256);
        if row_ok && col_ok && diag_ok {
            out.push(Generator::Gradient {
                base: at(0, 0),
                sx: dx,
                sy: dy,
            });
        }
    }
    if probe {
        return out;
    }
    // -- checker (bounded cell lattice) ----------------------------------------
    for cs in [1u32, 2, 4, 8, 16, 32] {
        let c = cs as usize;
        // Need room for two cells along both axes (spot checks read (2c,0)
        // and (0,2c)).
        if w <= 2 * c || h <= 2 * c {
            continue;
        }
        let (a, b) = (at(0, 0), at(c, 0));
        if a == b {
            continue;
        }
        // Parity spots: (2c,0) and (0,2c) are again `a`; (0,c) is `b`.
        if at(2 * c, 0) == a && at(0, 2 * c) == a && at(0, c) == b && at(c, c) == a {
            out.push(Generator::Checker { a, b, cell: cs });
        }
    }
    // -- periodic sawtooth (bounded period lattice) ---------------------------
    for p in [2u32, 4, 8, 16, 32, 64, 128, 256] {
        let pn = p as usize;
        // Period-closure spot checks read column/row `pn`.
        if w <= pn || h <= pn {
            continue;
        }
        let sx = wrapdiff(at(1, 0), at(0, 0));
        let sy = wrapdiff(at(0, 1), at(0, 0));
        if sx == 0 && sy == 0 {
            continue;
        }
        // The row difference must be steady and the field must close its
        // period on both axes at the origin edges.
        let steady = wrapdiff(at(2, 0), at(1, 0)) == sx && wrapdiff(at(0, 2), at(0, 1)) == sy;
        if steady && at(pn, 0) == at(0, 0) && at(0, pn) == at(0, 0) && at(pn, pn) == at(0, 0) {
            out.push(Generator::Periodic {
                base: at(0, 0),
                sx,
                sy,
                period: p,
            });
        }
    }
    out
}

/// Render the normative field of a generator over `w × h` into a canvas.
fn render_field(w: u32, h: u32, gen: crate::generator::Generator) -> Result<Canvas, VoleError> {
    let mut d = Vec::with_capacity((w as usize) * (h as usize));
    for y in 0..i64::from(h) {
        for x in 0..i64::from(w) {
            d.push(gen.sample(x, y));
        }
    }
    Canvas::from_parts(w, h, d)
}

/// Number of coded blocks in a transform residual block (mask popcount), 0 on
/// structurally invalid input (diagnostic helper; never panics).
fn coded_blocks(block: &[u8], w: u32, h: u32) -> usize {
    if block.first() != Some(&rans::KIND_TSF) {
        return 0;
    }
    let mlen = crate::transform::mask_len(w, h);
    if block.len() < 2 + mlen {
        return 0;
    }
    let mask = &block[2..2 + mlen];
    mask.iter().map(|b| b.count_ones() as usize).sum()
}

/// Build a kind-2 transform residual block (Phase M) that closes the exact
/// difference `target − base`: aligned 4×4 blocks over the canvas, one mask
/// bit per block, and DC/AC zigzag coefficient streams (each self-describing
/// RAW/rANS container). `None` when the canvases are identical (no residual).
pub fn build_transform_block(base: &Canvas, target: &Canvas) -> Option<Vec<u8>> {
    let (w, h) = (base.width(), base.height());
    let cw = w as usize;
    let cn = cw * h as usize;
    let (b, t) = (base.as_slice(), target.as_slice());
    if b == t {
        return None;
    }
    let (bx, by) = crate::transform::blocks_per_axis(w, h);
    let nblocks = bx.checked_mul(by)?;
    let mlen = crate::transform::mask_len(w, h);
    let mut grid = vec![0i64; cn];
    let mut seen = vec![false; nblocks];
    let mut coded = 0usize;
    for i in 0..cn {
        let d = i64::from(t[i]) - i64::from(b[i]);
        if d != 0 {
            grid[i] = d;
            let k = ((i / cw) / crate::transform::BLOCK) * bx + (i % cw) / crate::transform::BLOCK;
            if !seen[k] {
                seen[k] = true;
                coded += 1;
            }
        }
    }
    debug_assert!(coded > 0);
    let mut mask = vec![0u8; mlen];
    for (k, s) in seen.iter().enumerate() {
        if *s {
            mask[k >> 3] |= 1 << (k & 7);
        }
    }
    let mut dc = Vec::with_capacity(4 * coded);
    let mut ac = Vec::with_capacity(60 * coded);
    let blk = crate::transform::BLOCK;
    for (k, s) in seen.iter().enumerate() {
        if !s {
            continue;
        }
        let (kxx, kyy) = (k % bx, k / bx);
        let mut samples = [0i64; 16];
        for vy in 0..blk {
            let gy = kyy * blk + vy;
            if gy >= h as usize {
                continue; // zero-padded edge row
            }
            for vx in 0..blk {
                let gx = kxx * blk + vx;
                if gx >= cw {
                    continue; // zero-padded edge column
                }
                samples[vy * blk + vx] = grid[gy * cw + gx];
            }
        }
        let coeffs = crate::transform::forward_block(&samples);
        dc.extend_from_slice(&crate::transform::zigzag(coeffs[0]).to_le_bytes());
        for c in &coeffs[1..] {
            ac.extend_from_slice(&crate::transform::zigzag(*c).to_le_bytes());
        }
    }
    let dc_c = rans::encode_block(&dc);
    let ac_c = rans::encode_block(&ac);
    let mut out = Vec::with_capacity(2 + mlen + 8 + dc_c.len() + ac_c.len());
    out.push(rans::KIND_TSF);
    out.push(crate::transform::TRANSFORM_ID_4X4);
    out.extend_from_slice(&mask);
    out.extend_from_slice(&(dc_c.len() as u32).to_le_bytes());
    out.extend_from_slice(&(ac_c.len() as u32).to_le_bytes());
    out.extend_from_slice(&dc_c);
    out.extend_from_slice(&ac_c);
    Some(out)
}

// ---------------------------------------------------------------------------
// Accounting (§31)
// ---------------------------------------------------------------------------

/// Complete physical accounting of a `.vole` byte stream with every byte
/// classified into a declared bucket. The ten primary buckets sum to
/// `total_bytes`; the `*_split` fields are informational sub-buckets of
/// `object_bytes` and are excluded from that invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RepresentationCost {
    /// Stream header (magic, binding, geometry).
    pub header_bytes: u64,
    /// Immutable object declarations (descriptor + samples), including
    /// palette-index objects.
    pub object_bytes: u64,
    /// The checkpoint record (background + interval-0 instances).
    pub checkpoint_bytes: u64,
    /// Interval envelopes + state transitions + COPY/MOVE canvas ops.
    pub transition_bytes: u64,
    /// Per-frame residual op payload bytes **excluding** inline entropy
    /// models (tag + length prefix + block minus any model bytes, which are
    /// reported in `model_bytes`; the buckets sum to the stream length
    /// exactly).
    pub residual_bytes: u64,
    /// Inline entropy models inside residual blocks (a sub-bucket of the
    /// residual op payload bytes).
    pub model_bytes: u64,
    /// Persistent procedural state declarations (Phase J: the pre-checkpoint
    /// palette-table records that initialize mutable palette state; 0 in
    /// streams without palettes).
    pub state_bytes: u64,
    /// Shared dictionary bytes (0 in v1).
    pub dictionary_bytes: u64,
    /// Index bytes (0 in v1; no index record yet).
    pub index_bytes: u64,
    /// Integrity trailer.
    pub integrity_bytes: u64,
    /// Total stream bytes.
    pub total_bytes: u64,
    /// Informational: Gray8 raster-object declarations (tag + id + geometry +
    /// samples); a sub-bucket of `object_bytes`.
    pub raster_object_bytes: u64,
    /// Informational: fill-object declarations; a sub-bucket of `object_bytes`.
    pub fill_object_bytes: u64,
    /// Informational: palette-index object declarations; a sub-bucket of
    /// `object_bytes` (index planes are structural, not RAW fallback).
    pub index_object_bytes: u64,
    /// Informational: procedural-generator object declarations (Phase N: the
    /// record stores the bounded program, never the samples); a sub-bucket of
    /// `object_bytes`.
    pub generator_object_bytes: u64,
    /// Informational: the literal Gray8 samples inside raster objects (the
    /// RAW fallback payload of the stream).
    pub raster_object_sample_bytes: u64,
}

impl RepresentationCost {
    /// Procedural-fraction engineering metric (§32) — clearly *not* Shannon
    /// entropy: `1 - (residual + raw-raster-fallback)/total`. Raster object
    /// samples are the RAW fallback payload; fill objects, transitions,
    /// checkpoints, models, and integrity count as procedural description.
    pub fn procedural_fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            1.0 - (self.residual_bytes + self.raster_object_sample_bytes) as f64
                / self.total_bytes as f64
        }
    }
}

/// Walk the raw bytes of a stream and classify every byte into the accounting
/// buckets. Independent of `parse_stream`'s transition construction (shares
/// only the tag grammar) so accounting cannot silently inherit a parse bug.
pub fn account_stream(bytes: &[u8]) -> Result<RepresentationCost, VoleError> {
    if bytes.len() < 32 {
        return Err(VoleError::Truncated);
    }
    let limits = Limits::default();
    limits.check_stream_len(bytes.len() as u64)?;
    let content_len = bytes.len() - 32;
    let mut r = ByteReader::new(&bytes[..content_len]);
    // Header geometry (fixed offsets: magic(4)+reserved(1)+fver(2)+univ(4)+
    // profile(1)+feature(4)+w(4)+h(4)) — needed to walk kind-2 mask lengths.
    let header_w = u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| VoleError::Truncated)?);
    let header_h = u32::from_le_bytes(bytes[20..24].try_into().map_err(|_| VoleError::Truncated)?);
    limits.check_canvas(header_w, header_h)?;
    let mut cost = RepresentationCost {
        total_bytes: bytes.len() as u64,
        ..RepresentationCost::default()
    };
    let mut raster_objects = 0u64;
    let mut fill_objects = 0u64;
    let mut index_objects = 0u64;
    let mut generator_bytes = 0u64;
    let mut raster_samples = 0u64;
    let mut index_samples = 0u64;
    // Header is fixed width (24 bytes): magic(4)+reserved(1)+fver(2)+univ(4)+
    // profile(1)+feature(4)+w(4)+h(4).
    cost.header_bytes = 24;
    r.skip(24)?;
    while r.remaining() > 0 {
        let tag = r.u8()?;
        match tag {
            0x01 => {
                let _id = r.pull::<u32>()?;
                let w = r.pull::<u32>()?;
                let h = r.pull::<u32>()?;
                let n = u64::from(w) * u64::from(h);
                r.skip(usize::try_from(n).map_err(|_| VoleError::ArithmeticOverflow)?)?;
                raster_objects += 1;
                raster_samples += n;
            }
            0x02 => {
                let _id = r.pull::<u32>()?;
                let _w = r.pull::<u32>()?;
                let _h = r.pull::<u32>()?;
                let _v = r.u8()?;
                fill_objects += 1;
            }
            0x05 => {
                // Palette-index object declaration (Phase J).
                let _id = r.pull::<u32>()?;
                let w = r.pull::<u32>()?;
                let h = r.pull::<u32>()?;
                let n = u64::from(w) * u64::from(h);
                r.skip(usize::try_from(n).map_err(|_| VoleError::ArithmeticOverflow)?)?;
                index_objects += 1;
                index_samples += n;
            }
            0x07 => {
                // Procedural-generator object declaration (Phase N): the
                // record carries the bounded program, never the samples.
                let _id = r.pull::<u32>()?;
                let _w = r.pull::<u32>()?;
                let _h = r.pull::<u32>()?;
                let kind = r.u8()?;
                let n: usize = match kind {
                    crate::generator::GEN_GRADIENT => 9,  // base u8 + 2 x i32
                    crate::generator::GEN_CHECKER => 6,   // a u8 + b u8 + cell u32
                    crate::generator::GEN_PERIODIC => 13, // base u8 + 2 x i32 + period u32
                    crate::generator::GEN_NOISE => 8,     // seed u64
                    _ => return Err(VoleError::NonCanonicalEncoding),
                };
                r.skip(n)?;
                generator_bytes += OBJECT_DECL_HEADER + 1 + n as u64;
            }
            0x06 => {
                // Pre-checkpoint palette-table record (Phase J): mutable
                // procedural state initialization.
                let _id = r.pull::<u32>()?;
                let len = r.pull::<u32>()?;
                if u64::from(len) > u64::from(limits.max_palette_entries) {
                    return Err(VoleError::DimensionTooLarge);
                }
                if len == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                r.skip(len as usize)?;
                cost.state_bytes += 9 + u64::from(len);
            }
            0x03 | 0x08 => {
                let with_bindings = tag == 0x08;
                let _bg = r.u8()?;
                let n = r.pull::<u32>()?;
                if with_bindings {
                    r.skip(20 * n as usize)?;
                    cost.checkpoint_bytes += 6 + 20 * u64::from(n);
                } else {
                    r.skip(16 * n as usize)?;
                    cost.checkpoint_bytes += checkpoint_bytes(u64::from(n));
                }
            }
            0x04 => {
                let _t = r.pull::<u64>()?;
                let n = r.pull::<u32>()?;
                cost.transition_bytes += INTERVAL_ENVELOPE;
                for _ in 0..n {
                    let tag2 = r.u8()?;
                    match tag2 {
                        0x21 => {
                            r.skip(16)?;
                            cost.transition_bytes += 17;
                        }
                        0x22 | 0x26 => {
                            r.skip(12)?;
                            cost.transition_bytes += 13;
                        }
                        0x23 => {
                            let m = r.pull::<u32>()?;
                            r.skip(9 * m as usize)?;
                            cost.transition_bytes += 5 + 9 * u64::from(m);
                        }
                        0x24 | 0x25 => {
                            r.skip(24)?;
                            cost.transition_bytes += 25;
                        }
                        0x27..=0x29 => {
                            cost.transition_bytes += 1;
                        }
                        0x2c => {
                            cost.transition_bytes += 1;
                        }
                        0x2d => {
                            let _id = r.pull::<u32>()?;
                            let len = r.pull::<u32>()?;
                            if u64::from(len) > u64::from(limits.max_palette_entries) {
                                return Err(VoleError::DimensionTooLarge);
                            }
                            if len == 0 {
                                return Err(VoleError::NonCanonicalEncoding);
                            }
                            r.skip(len as usize)?;
                            cost.transition_bytes += 9 + u64::from(len);
                        }
                        0x2e => {
                            let _id = r.pull::<u32>()?;
                            let m = r.pull::<u32>()?;
                            if u64::from(m) > 256 {
                                return Err(VoleError::NonCanonicalEncoding);
                            }
                            r.skip(2 * m as usize)?;
                            cost.transition_bytes += 9 + 2 * u64::from(m);
                        }
                        0x2f => {
                            r.skip(8)?;
                            cost.transition_bytes += 9;
                        }
                        0x30 => {
                            r.skip(28)?;
                            cost.transition_bytes += 29;
                        }
                        0x2b => {
                            let _id = r.pull::<u32>()?;
                            let m = r.pull::<u32>()?;
                            if u64::from(m) > u64::from(limits.max_trajectory_segments) {
                                return Err(VoleError::MaterializationBudgetExceeded);
                            }
                            let mut seg_bytes = 9u64;
                            for _ in 0..m {
                                match r.u8()? {
                                    crate::trajectory::SEG_LINEAR => {
                                        r.skip(16)?;
                                        seg_bytes += 17;
                                    }
                                    crate::trajectory::SEG_ACCEL => {
                                        r.skip(24)?;
                                        seg_bytes += 25;
                                    }
                                    _ => return Err(VoleError::NonCanonicalEncoding),
                                }
                            }
                            cost.transition_bytes += seg_bytes;
                        }
                        0x2a => {
                            let len = r.pull::<u32>()?;
                            let block = r.take(len as usize)?;
                            cost.transition_bytes += 5;
                            // Inline entropy models live inside the block bytes;
                            // they are reported in `model_bytes`, so they are
                            // excluded from `residual_bytes` (the buckets must
                            // sum to the stream length exactly).
                            let mut models = 0u64;
                            match block.first() {
                                Some(&rans::KIND_TSF) => {
                                    // Phase M transform residual: mask then two
                                    // self-describing sub-containers.
                                    let (bx, by) =
                                        crate::transform::blocks_per_axis(header_w, header_h);
                                    let nblocks = bx.saturating_mul(by);
                                    let mlen = nblocks.div_ceil(8);
                                    let o = 2usize.saturating_add(mlen);
                                    if block.len() >= o + 8 {
                                        let dc_len = u64::from(u32::from_le_bytes([
                                            block[o],
                                            block[o + 1],
                                            block[o + 2],
                                            block[o + 3],
                                        ]));
                                        let ac_len = u64::from(u32::from_le_bytes([
                                            block[o + 4],
                                            block[o + 5],
                                            block[o + 6],
                                            block[o + 7],
                                        ]));
                                        let dc_off = o + 8;
                                        if block.get(dc_off) == Some(&rans::KIND_RANS) {
                                            models += 1;
                                        }
                                        let ac_off = dc_off.saturating_add(dc_len as usize);
                                        if block.get(ac_off) == Some(&rans::KIND_RANS) {
                                            models += 1;
                                        }
                                        let _ = ac_len;
                                    }
                                }
                                Some(&rans::KIND_RANS)
                                    if block.len() >= 9 + rans::MODEL_SERIALIZED =>
                                {
                                    // A single-container rANS payload always
                                    // carries its inline model.
                                    models = 1;
                                }
                                _ => {}
                            }
                            let model_bytes = models * rans::MODEL_SERIALIZED as u64;
                            cost.model_bytes += model_bytes;
                            cost.residual_bytes += u64::from(len) - model_bytes;
                        }
                        _ => return Err(VoleError::NonCanonicalEncoding),
                    }
                }
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        }
    }
    cost.integrity_bytes = 32;
    cost.raster_object_bytes = raster_objects * OBJECT_DECL_HEADER + raster_samples;
    cost.fill_object_bytes = fill_objects * FILL_DECL;
    cost.index_object_bytes = index_objects * OBJECT_DECL_HEADER + index_samples;
    cost.generator_object_bytes = generator_bytes;
    cost.object_bytes = cost.raster_object_bytes
        + cost.fill_object_bytes
        + cost.index_object_bytes
        + cost.generator_object_bytes;
    cost.raster_object_sample_bytes = raster_samples;
    Ok(cost)
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// Aggregate per-family statistics for one frame decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FamilyStat {
    /// Machine-stable family label.
    pub family: &'static str,
    /// Candidates of this family generated and evaluated.
    pub evaluated: u64,
    /// Candidates that reproduced the target exactly.
    pub valid: u64,
    /// Cheapest valid candidate incremental payload of this family (0 when
    /// none valid).
    pub best_payload: u64,
}

/// The exhaustive decision record for one frame (§28).
#[derive(Debug, Clone)]
pub struct FrameDecision {
    /// Frame index (0 is the checkpoint frame).
    pub frame: u64,
    /// Winning family label.
    pub winner_family: &'static str,
    /// Candidates generated and evaluated for this frame.
    pub candidates_evaluated: u64,
    /// Candidates that reproduced the target exactly.
    pub candidates_valid: u64,
    /// Incremental persisted bytes of the winner: interval envelope + state
    /// transitions + canvas ops + any first-use object declaration.
    pub winner_payload_bytes: u64,
    /// Winning interval bytes excluding first-use object declarations.
    pub winner_interval_bytes: u64,
    /// Object-declaration bytes first introduced by this decision (0 unless
    /// the winner declared a new object).
    pub object_decl_bytes: u64,
    /// Residual points carried by the winning program.
    pub residual_points: u64,
    /// Winning program: the interval's full transition list (state transitions
    /// first, then canvas ops — the writer's canonical emission order).
    pub emitted: Vec<Transition>,
    /// Per-family aggregate of the exhaustive evaluation.
    pub families: Vec<FamilyStat>,
    /// True: the winner was committed to the real state and re-materialized
    /// through the normative path, byte-equal to the target.
    pub materialized_exact: bool,
    /// Search work performed for this frame: candidate count + weighted pixel
    /// scans (deterministic; encoder-side metric, never part of the stream).
    pub search_work: u64,
    /// DSFB diagnostics for this frame (None under non-guided strategies).
    pub dsfb_diag: Option<DsfbFrameDiag>,
}

/// Search-space mode used for a run (canvas-size dependent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSpace {
    /// Every enumerated candidate is materialized and byte-verified.
    Exhaustive,
    /// Large-canvas run: 1D scroll candidates are row-hash-prefiltered
    /// (accepted candidates are still byte-verified); every other family is
    /// fully evaluated.
    HashPrefiltered,
}

/// Background sweep report.
#[derive(Debug, Clone)]
pub struct BgSweep {
    /// Whether the sweep was enabled.
    pub enabled: bool,
    /// Every tried background value and its total stream bytes.
    pub tried: Vec<(u8, u64)>,
    /// The chosen background.
    pub chosen: u8,
}

/// Options controlling the Phase-G/H encoder.
#[derive(Debug, Clone)]
pub struct EncodeOptions {
    /// Sweep the deterministic background candidate set (`{0,255}` ∪ frame-0
    /// corners ∪ global mode) and keep the cheapest complete run.
    pub bg_sweep: bool,
    /// Fix the checkpoint background to this value (no sweep). Ignored when
    /// `bg_sweep` is true.
    pub background: Option<u8>,
    /// Restrict every decision to the RAW family: a VOLE raster-only baseline
    /// (each frame stored as an immutable raster object + minimal instance;
    /// identical content is still content-deduped, which is identity, not
    /// procedural reuse).
    pub raster_only: bool,
    /// Translation window radius: candidates test `|dx|, |dy| <= r` per frame.
    pub translation_window: i64,
    /// Search strategy (Phase H): Exhaustive (oracle) by default;
    /// FixedHeuristic; DsfbGuided.
    pub strategy: EncoderStrategy,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions {
            bg_sweep: true,
            background: None,
            raster_only: false,
            translation_window: 2,
            strategy: EncoderStrategy::Exhaustive,
        }
    }
}

/// The complete result of a raster encode.
#[derive(Debug, Clone)]
pub struct EncodeReport {
    /// The standalone `.vole` stream bytes.
    pub vole: Vec<u8>,
    /// Canvas geometry.
    pub width: u32,
    pub height: u32,
    /// Number of frames encoded.
    pub frame_count: u64,
    /// Chosen checkpoint background.
    pub background: u8,
    /// End-to-end verification: the stream was decoded with the normative
    /// decoder and every materialized frame is byte-identical to the input
    /// raster. The encoder refuses to return a stream for which this is false.
    pub exact: bool,
    /// Per-frame exhaustive decision records.
    pub decisions: Vec<FrameDecision>,
    /// Complete physical accounting of the produced stream.
    pub cost: RepresentationCost,
    /// Total raw raster bytes of the input sequence.
    pub raw_raster_bytes: u64,
    /// Background sweep report.
    pub bg_sweep: BgSweep,
    /// Search-space mode actually used.
    pub search_space: SearchSpace,
}

// ---------------------------------------------------------------------------
// The exhaustive encoder
// ---------------------------------------------------------------------------

/// Candidate-space gates. Whole-canvas 2D toroidal scrolls are enumerated in
/// full only when the candidate count is small; 1D wrap/screen scrolls are
/// fully enumerated below a sample gate and row-hash-prefiltered above it.
const TOROIDAL_CANDIDATE_GATE: u64 = 4096;
const FULL_SCROLL_GATE: u64 = 16 * 1024;

// --- Phase K: variable-region gates ----------------------------------------
// The region family partitions the canvas into tiles of a granularity and
// declares one rectangle (the diff bounding box within each diff-bearing
// tile) as an immutable object painted above the base. The candidate space is
// bounded deterministically:
// * regions are only evaluated when the per-frame diff is non-empty and at
//   most a quarter of the canvas (a larger diff is served at least as cheaply
//   by the RAW/whole-frame reset sentinel — a whole-canvas region equals a
//   reset plus a redundant create op);
// * at most REGION_MAX_RECTS rectangles per candidate (a partition needing
//   more could not beat the reset sentinel's single declaration);
// * a candidate whose rectangle would be painted *under* a persistent overlay
//   point that disagrees with the target is invalid (overlay paints above all
//   instances; the residual/sparse families serve those frames).
const REGION_MAX_DIFF: u64 = 1 << 20;
const REGION_MAX_RECTS: usize = 256;
/// Granularity ladder evaluated by the full (exhaustive) plan.
const REGION_GRANULARITIES: [u32; 4] = [64, 32, 16, 8];
/// Granularity probed by the fixed-heuristic / rotating-sweep probe.
const REGION_PROBE_GRANULARITY: u32 = 16;

/// One declared rectangle of a region program (Phase K): the immutable
/// content of the target's sub-rectangle, placed at `(x, y)` on later
/// materializations by a fresh instance.
#[derive(Debug, Clone)]
struct RegionSpec {
    x: i64,
    y: i64,
    w: u32,
    h: u32,
    content: Content,
}

/// A winning (or candidate) program plan.
#[derive(Debug, Clone)]
enum Plan {
    /// No transitions, no ops; frame == materialized state.
    Unchanged,
    /// Clear instances + overlay; frame == uniform background.
    ClearToBg,
    /// Clear instances only (overlay stays).
    ClearInstancesOnly,
    /// Clear overlay only (instances stay).
    ClearOverlayOnly,
    /// Reset to one full-canvas instance of the frame's content. The content
    /// is derived from the target at commit time; the `new_content` flag
    /// records whether this frame declares a new object.
    Reset { new_content: bool },
    /// Persistent sparse overlay commit over the materialized base.
    Patch { pts: Vec<(i64, i64, u8)> },
    /// One-shot residual block over the materialized base.
    Residual { block: Vec<u8> },
    /// Canvas ops (copies) only — valid iff they reproduce the target.
    Copies { ops: Vec<Transition> },
    /// Copy ops plus a one-shot residual closing the delta.
    CopyResidual {
        ops: Vec<Transition>,
        block: Vec<u8>,
    },
    /// Whole-pixel translation of the live full-canvas instance.
    SetPosition { dx: i64, dy: i64 },
    /// Advance the live instance's persistent translation.
    Advance,
    /// Variable-region repair (Phase K): declare the changed rectangles as
    /// immutable objects and paint them above the base with fresh instances.
    /// Valid iff every changed sample lies inside a rectangle whose content
    /// is the target's own sub-rectangle and no rectangle is shadowed by a
    /// disagreeing overlay point.
    Regions { rects: Vec<RegionSpec> },
    /// Reset to one full-canvas instance of a bounded procedural generator
    /// (Phase N): the frame content is the generator's computed field, so the
    /// declaration stores a program, never the samples. Valid iff the
    /// normative render equals the target exactly.
    Generator {
        new_content: bool,
        gen: crate::generator::Generator,
    },
    /// Generator reset plus a one-shot point residual closing the exactness
    /// gap (`F = M(generator) ⊕_ρ R`, Phase N): every generator candidate
    /// carries its exact residual correction — an approximation that cannot
    /// reproduce the target is admissible only with the residual counted.
    GenResidual {
        new_content: bool,
        gen: crate::generator::Generator,
        block: Vec<u8>,
    },
}

/// Candidate under evaluation.
struct Cand {
    label: &'static str,
    plan: Plan,
    valid: bool,
    invalid_reason: &'static str,
    /// Incremental payload (envelope + transitions + ops + intro decl).
    payload: u64,
}

impl Cand {
    fn new(label: &'static str, plan: Plan, payload: u64) -> Cand {
        Cand {
            label,
            plan,
            valid: false,
            invalid_reason: "",
            payload,
        }
    }
}

/// Per-frame evaluation context.
struct Eval {
    /// Family label → (evaluated, valid, best_payload).
    fam: Vec<(&'static str, u64, u64, u64)>,
    order: u64,
    /// Deterministic search-work estimate: candidate count + weighted pixel
    /// scans performed by the family evaluations.
    work: u64,
    best: Option<Cand>,
}

impl Eval {
    fn new() -> Eval {
        Eval {
            fam: Vec::new(),
            order: 0,
            work: 0,
            best: None,
        }
    }

    /// Account for a deterministic amount of pixel-scan work (in samples).
    fn add_work(&mut self, samples: u64) {
        self.work = self.work.saturating_add(samples);
    }

    fn consider(&mut self, c: Cand) {
        self.order += 1;
        self.work = self.work.saturating_add(1);
        let family = c.label;
        let valid = c.valid;
        let payload = c.payload;
        if valid {
            // Tie-break deterministically by enumeration order.
            let better = match &self.best {
                None => true,
                Some(b) => payload < b.payload,
            };
            if better {
                self.best = Some(c);
            }
        }
        let slot = self.fam.iter_mut().find(|(f, ..)| *f == family);
        match slot {
            Some(slot) => {
                slot.1 += 1;
                if valid {
                    slot.2 += 1;
                    if payload < slot.3 {
                        slot.3 = payload;
                    }
                }
            }
            None => self.fam.push((
                family,
                1,
                u64::from(valid),
                if valid { payload } else { u64::MAX },
            )),
        }
    }

    fn family_stats(&self) -> Vec<FamilyStat> {
        self.fam
            .iter()
            .map(|(family, evaluated, valid, best)| FamilyStat {
                family,
                evaluated: *evaluated,
                valid: *valid,
                best_payload: if *valid == 0 { 0 } else { *best },
            })
            .collect()
    }
}

/// The complete per-frame cost of a reset-to-full-canvas-object program.
const RESET_INTERVAL_COST: u64 = INTERVAL_ENVELOPE + 1 + 1 + 17;

struct Encoder<'a> {
    w: u32,
    h: u32,
    frames: &'a [Canvas],
    opts: EncodeOptions,
    bg: u8,
    /// Content identity → object id (exact reuse registry).
    index: HashMap<[u8; 32], u32>,
    st: State,
    prev: Option<Canvas>,
    /// Live instances at the checkpoint (frame 0), for stream assembly.
    checkpoint_instances: Vec<Instance>,
    decisions: Vec<FrameDecision>,
    next_object_id: u32,
    next_instance_id: u32,
    search_space: SearchSpace,
    /// Search strategy (Phase H).
    strategy: EncoderStrategy,
    /// DSFB model (present only under DsfbGuided).
    model: Option<DsfbModel>,
    /// The active per-frame evaluation plan (strategy-derived).
    plan: FramePlan,
    /// Copy ops emitted by the previous winning frame (probe replay).
    last_copy_ops: Vec<Transition>,
    /// Translation delta emitted by the previous winning frame (probe replay).
    last_translation: Option<(i64, i64)>,
}

impl<'a> Encoder<'a> {
    fn new(frames: &'a [Canvas], bg: u8, opts: &EncodeOptions) -> Result<Encoder<'a>, VoleError> {
        if frames.is_empty() {
            return Err(VoleError::ApiConstraint("encode needs at least one frame"));
        }
        let limits = Limits::default();
        let w = frames[0].width();
        let h = frames[0].height();
        limits.check_canvas(w, h)?;
        if frames.len() as u64 > limits.max_checkpoint_distance + 1 {
            return Err(VoleError::CheckpointOutOfEnvelope);
        }
        for f in frames {
            if f.width() != w || f.height() != h {
                return Err(VoleError::ObjectGeometryMismatch);
            }
        }
        let search_space = if u64::from(w) * u64::from(h) <= FULL_SCROLL_GATE {
            SearchSpace::Exhaustive
        } else {
            SearchSpace::HashPrefiltered
        };
        let mut st = State::new(crate::time::Interval::ZERO);
        st.set_background(bg);
        let strategy = opts.strategy;
        let model = (strategy == EncoderStrategy::DsfbGuided).then(DsfbModel::new);
        Ok(Encoder {
            w,
            h,
            frames,
            opts: opts.clone(),
            bg,
            index: HashMap::new(),
            st,
            prev: None,
            checkpoint_instances: Vec::new(),
            decisions: Vec::new(),
            next_object_id: 1,
            next_instance_id: 1,
            search_space,
            strategy,
            model,
            plan: FramePlan::full(),
            last_copy_ops: Vec::new(),
            last_translation: None,
        })
    }

    /// Resolve the per-frame evaluation plan from the strategy.
    fn plan_for_frame(&mut self) -> FramePlan {
        let plan = match self.strategy {
            EncoderStrategy::Exhaustive => FramePlan::full(),
            EncoderStrategy::FixedHeuristic => FramePlan::fixed_heuristic(),
            EncoderStrategy::DsfbGuided => match &self.model {
                Some(m) => m.plan(),
                None => FramePlan::full(),
            },
        };
        self.plan = plan.clone();
        plan
    }

    fn content_of_target(&self, target: &Canvas) -> Content {
        match uniform_value(target) {
            Some(v) => Content::Fill(v),
            None => Content::Raster(target.as_slice().to_vec()),
        }
    }

    fn is_new_content(&self, content: &Content) -> bool {
        !self
            .index
            .contains_key(&content_id(self.w, self.h, content))
    }

    /// Whether `(w, h, content)` is new to the registry (geometry-generic).
    fn is_new_region(&self, w: u32, h: u32, content: &Content) -> bool {
        !self.index.contains_key(&content_id(w, h, content))
    }

    /// Register `content` at the given geometry (assigning an id if new),
    /// declare it into the working state, and return its object id.
    fn ensure_object_wh(&mut self, w: u32, h: u32, content: Content) -> Result<u32, VoleError> {
        let cid = content_id(w, h, &content);
        if let Some(&id) = self.index.get(&cid) {
            return Ok(id);
        }
        let id = self.next_object_id;
        self.next_object_id += 1;
        let obj = match &content {
            Content::Fill(v) => Object::fill(w, h, *v)?,
            Content::Raster(data) => Object::raster(w, h, data.clone())?,
            Content::Generator(gen) => Object::procedural(w, h, *gen)?,
        };
        self.st.declare_object(ObjectId(id), obj)?;
        self.index.insert(cid, id);
        Ok(id)
    }

    /// Register `content` at the canvas geometry.
    fn ensure_object(&mut self, content: Content) -> Result<u32, VoleError> {
        self.ensure_object_wh(self.w, self.h, content)
    }

    /// Transitions for a full reset to one full-canvas instance of `object_id`.
    fn reset_trs(&mut self, object_id: u32) -> Vec<Transition> {
        let iid = self.next_instance_id;
        self.next_instance_id += 1;
        vec![
            Transition::ClearInstances,
            Transition::ClearOverlay,
            Transition::CreateInstance {
                id: InstanceId(iid),
                object: ObjectId(object_id),
                x: 0,
                y: 0,
            },
        ]
    }

    /// Declare (if new) a generator object and reset state to one full-canvas
    /// instance of it; returns the applied reset transitions and the
    /// declaration bytes they introduce (Phase N).
    fn generator_reset(
        &mut self,
        gen: crate::generator::Generator,
        new_content: bool,
    ) -> Result<(Vec<Transition>, u64), VoleError> {
        let content = Content::Generator(gen);
        let decl = if new_content {
            decl_bytes(self.w, self.h, &content)
        } else {
            0
        };
        let oid = self.ensure_object(content)?;
        let trs = self.reset_trs(oid);
        for tr in &trs {
            tr.apply(&mut self.st)?;
        }
        Ok((trs, decl))
    }

    /// Materialize a candidate program over a *clone* of the working state.
    /// Used only for state-mutating candidates whose expected frame is not a
    /// pure content compare (the clear-only resets).
    fn sim_clear(&self, trs: &[Transition]) -> Result<Canvas, VoleError> {
        let mut st = self.st.clone();
        for tr in trs {
            tr.apply(&mut st)?;
        }
        materialize::materialize_full(&st, self.w, self.h, &Limits::default())
    }

    /// Run the whole greedy encode for the configured background.
    fn run(&mut self) -> Result<EncodeReport, VoleError> {
        self.encode_frame0()?;
        for k in 1..self.frames.len() {
            self.encode_frame(k)?;
        }
        let bytes = self.assemble()?;
        let exact = self.verify(&bytes)?;
        if !exact {
            return Err(VoleError::ApiConstraint(
                "end-to-end verification failed (encoder invariant)",
            ));
        }
        let cost = account_stream(&bytes)?;
        Ok(EncodeReport {
            vole: bytes,
            width: self.w,
            height: self.h,
            frame_count: self.frames.len() as u64,
            background: self.bg,
            exact,
            decisions: std::mem::take(&mut self.decisions),
            cost,
            raw_raster_bytes: self.frames.iter().map(|f| f.sample_count()).sum(),
            bg_sweep: BgSweep {
                enabled: false,
                tried: Vec::new(),
                chosen: self.bg,
            },
            search_space: self.search_space,
        })
    }

    // -- frame 0 -------------------------------------------------------------

    fn encode_frame0(&mut self) -> Result<(), VoleError> {
        let target = &self.frames[0];
        let mut ev = Eval::new();
        if !self.opts.raster_only && uniform_value(target) == Some(self.bg) {
            let mut c = Cand::new("fill", Plan::ClearToBg, checkpoint_bytes(0));
            c.valid = true;
            ev.consider(c);
        }
        let content = self.content_of_target(target);
        let new = self.is_new_content(&content);
        let label = if self.opts.raster_only || new {
            "raw"
        } else {
            "fill"
        };
        let payload = decl_bytes(self.w, self.h, &content) + checkpoint_bytes(1);
        let mut c = Cand::new(label, Plan::Reset { new_content: true }, payload);
        c.valid = true;
        ev.consider(c);

        // Phase N: whole-frame procedural-generator discovery on the frame-0
        // content (full fit set — frame 0 is a cold start for every strategy).
        if !self.opts.raster_only {
            self.consider_generators(target, &mut ev, Mode::Full, true);
        }

        let stats = ev.family_stats();
        let winner = ev
            .best
            .ok_or(VoleError::ApiConstraint("frame0 has no candidates"))?;
        match &winner.plan {
            Plan::ClearToBg => {}
            Plan::Reset { .. } => {
                let oid = self.ensure_object(content)?;
                let trs = self.reset_trs(oid);
                for tr in &trs {
                    tr.apply(&mut self.st)?;
                }
                self.checkpoint_instances = self.st.instances().cloned().collect();
            }
            Plan::Generator { gen, .. } => {
                let oid = self.ensure_object(Content::Generator(*gen))?;
                let trs = self.reset_trs(oid);
                for tr in &trs {
                    tr.apply(&mut self.st)?;
                }
                self.checkpoint_instances = self.st.instances().cloned().collect();
            }
            _ => {
                return Err(VoleError::ApiConstraint("unexpected frame0 winner plan"));
            }
        }
        let canvas = materialize::materialize_full(&self.st, self.w, self.h, &Limits::default())?;
        if canvas != *target {
            return Err(VoleError::ApiConstraint("frame0 materialization mismatch"));
        }
        self.prev = Some(canvas);
        self.decisions.push(FrameDecision {
            frame: 0,
            winner_family: winner.label,
            candidates_evaluated: ev.order,
            candidates_valid: stats.iter().map(|s| s.valid).sum(),
            winner_payload_bytes: winner.payload,
            winner_interval_bytes: 0,
            object_decl_bytes: match &winner.plan {
                Plan::Reset { .. } => decl_bytes(self.w, self.h, &self.content_of_target(target)),
                Plan::Generator { gen, .. } => {
                    decl_bytes(self.w, self.h, &Content::Generator(*gen))
                }
                _ => 0,
            },
            residual_points: 0,
            emitted: Vec::new(),
            families: stats,
            materialized_exact: true,
            search_work: ev.work,
            dsfb_diag: None,
        });
        Ok(())
    }

    // -- frame k >= 1 ---------------------------------------------------------

    fn encode_frame(&mut self, k: usize) -> Result<(), VoleError> {
        let target = &self.frames[k];
        let limits = Limits::default();
        // The per-frame plan depends only on history (deterministic).
        let plan = self.plan_for_frame();
        let mut ev = Eval::new();
        let base = materialize::materialize_full(&self.st, self.w, self.h, &limits)?;
        ev.add_work(u64::from(self.w) * u64::from(self.h));
        let prev = self.prev.clone().expect("previous frame");
        let mut emitted_cost = 0u64;

        if self.opts.raster_only {
            self.consider_reset(target, &mut ev, true);
        } else {
            if plan.unchanged {
                self.consider_unchanged(target, &base, &mut ev);
            }
            if plan.clears == Mode::Full {
                self.consider_clears(target, &mut ev);
            }
            // The reset candidate is the RAW sentinel: resetting to a fresh
            // full-canvas object of the target's own content always reproduces
            // the target exactly (RAW guarantee), so every plan evaluates it.
            self.consider_reset(target, &mut ev, false);
            if plan.sparse || plan.transform != Mode::Off {
                self.consider_sparse_and_residual(target, &base, &mut ev, plan.transform);
            }
            if plan.prev_diff {
                self.consider_prev_diff(target, &base, &prev, &mut ev);
            }
            match plan.copies {
                Mode::Full => self.consider_copies_full(target, &base, &prev, &mut ev),
                Mode::Probe => self.consider_copies_probe(target, &base, &prev, &mut ev),
                Mode::Off => {}
            }
            match plan.translation {
                Mode::Full => self.consider_translation(target, &mut ev, false),
                Mode::Probe => self.consider_translation(target, &mut ev, true),
                Mode::Off => {}
            }
            // Phase K: variable-region repair family.
            match plan.regions {
                Mode::Full => self.consider_regions(target, &base, &mut ev, &REGION_GRANULARITIES),
                Mode::Probe => {
                    self.consider_regions(target, &base, &mut ev, &[REGION_PROBE_GRANULARITY])
                }
                Mode::Off => {}
            }
            // Phase N: whole-frame procedural-generator discovery.
            match plan.generators {
                Mode::Full => self.consider_generators(target, &mut ev, Mode::Full, false),
                Mode::Probe => self.consider_generators(target, &mut ev, Mode::Probe, false),
                Mode::Off => {}
            }
        }

        let stats = ev.family_stats();
        let candidates_valid = stats.iter().map(|f| f.valid).sum();
        let winner = ev
            .best
            .ok_or(VoleError::ApiConstraint("frame has no candidates"))?;

        // ---- commit the winner to the real state and materialize ----
        let mut emitted: Vec<Transition> = Vec::new();
        let mut residual_points = 0u64;
        let mut decl = 0u64;
        match &winner.plan {
            Plan::Unchanged => {
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::ClearToBg => {
                for tr in [Transition::ClearInstances, Transition::ClearOverlay] {
                    tr.apply(&mut self.st)?;
                    emitted_cost += tr_len(&tr);
                    emitted.push(tr);
                }
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::ClearInstancesOnly => {
                let tr = Transition::ClearInstances;
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::ClearOverlayOnly => {
                let tr = Transition::ClearOverlay;
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::Reset { new_content } => {
                let content = self.content_of_target(target);
                if *new_content {
                    decl = decl_bytes(self.w, self.h, &content);
                }
                let oid = self.ensure_object(content)?;
                let trs = self.reset_trs(oid);
                for tr in &trs {
                    tr.apply(&mut self.st)?;
                    emitted_cost += tr_len(tr);
                    emitted.push(tr.clone());
                }
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::Generator { new_content, gen } => {
                let (trs, d) = self.generator_reset(*gen, *new_content)?;
                decl = d;
                for tr in &trs {
                    emitted_cost += tr_len(tr);
                    emitted.push(tr.clone());
                }
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::GenResidual {
                new_content,
                gen,
                block,
            } => {
                // Phase N: reset to the generator field, then carry the exact
                // point residual that closes the approximation gap.
                let (trs, d) = self.generator_reset(*gen, *new_content)?;
                decl = d;
                for tr in &trs {
                    emitted_cost += tr_len(tr);
                    emitted.push(tr.clone());
                }
                residual_points = decode_point_count(block, self.w, self.h, &limits);
                emitted_cost += 5 + block.len() as u64;
                emitted.push(Transition::Residual {
                    block: block.clone(),
                });
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::Patch { pts } => {
                residual_points = pts.len() as u64;
                let tr = Transition::PatchSparse {
                    points: pts.clone(),
                };
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::Residual { block } => {
                residual_points = decode_point_count(block, self.w, self.h, &limits);
                emitted_cost += 5 + block.len() as u64;
                emitted.push(Transition::Residual {
                    block: block.clone(),
                });
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::Copies { ops } => {
                self.last_copy_ops = copy_ops_only(ops);
                self.last_translation = None;
                for op in ops {
                    emitted_cost += tr_len(op);
                    emitted.push(op.clone());
                }
            }
            Plan::CopyResidual { ops, block } => {
                residual_points = decode_point_count(block, self.w, self.h, &limits);
                self.last_copy_ops = copy_ops_only(ops);
                self.last_translation = None;
                for op in ops {
                    emitted_cost += tr_len(op);
                    emitted.push(op.clone());
                }
                emitted_cost += 5 + block.len() as u64;
                emitted.push(Transition::Residual {
                    block: block.clone(),
                });
            }
            Plan::SetPosition { dx, dy } => {
                let inst = self
                    .st
                    .instances()
                    .next()
                    .ok_or(VoleError::UnknownInstance)?
                    .clone();
                let tr = Transition::SetPosition {
                    id: inst.id,
                    x: inst.x + dx,
                    y: inst.y + dy,
                };
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
                self.last_copy_ops.clear();
                self.last_translation = Some((*dx, *dy));
            }
            Plan::Advance => {
                let tr = Transition::AdvanceTranslations;
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
            Plan::Regions { rects } => {
                // Declare each region content (reusing any exact content
                // already in the object library) and paint it above the base
                // with a fresh instance. Created instances persist, so a
                // region stays repaired until something over-paints or clears
                // it — the same persistence semantics as every state commit.
                for spec in rects {
                    if self.is_new_region(spec.w, spec.h, &spec.content) {
                        decl = decl
                            .checked_add(decl_bytes(spec.w, spec.h, &spec.content))
                            .ok_or(VoleError::ArithmeticOverflow)?;
                    }
                    let oid = self.ensure_object_wh(spec.w, spec.h, spec.content.clone())?;
                    let iid = self.next_instance_id;
                    self.next_instance_id += 1;
                    let tr = Transition::CreateInstance {
                        id: InstanceId(iid),
                        object: ObjectId(oid),
                        x: spec.x,
                        y: spec.y,
                    };
                    tr.apply(&mut self.st)?;
                    emitted_cost += tr_len(&tr);
                    emitted.push(tr);
                }
                self.last_copy_ops.clear();
                self.last_translation = None;
            }
        }

        let mut canvas = materialize::materialize_full(&self.st, self.w, self.h, &limits)?;
        for tr in &emitted {
            if matches!(
                tr,
                Transition::CopyRect { .. }
                    | Transition::MoveRect { .. }
                    | Transition::Residual { .. }
            ) {
                materialize::apply_canvas_op(&mut canvas, &prev, tr, &limits)?;
            }
        }
        if canvas != *target {
            return Err(VoleError::ApiConstraint(
                "winner materialization mismatch (encoder invariant)",
            ));
        }
        self.prev = Some(canvas);

        let payload = INTERVAL_ENVELOPE + emitted_cost + decl;
        let diag = match &mut self.model {
            Some(m) => {
                m.observe(winner.label, payload, ev.order);
                Some(m.diagnostics(winner.label, payload))
            }
            None => None,
        };
        self.decisions.push(FrameDecision {
            frame: k as u64,
            winner_family: winner.label,
            candidates_evaluated: ev.order,
            candidates_valid,
            winner_payload_bytes: payload,
            winner_interval_bytes: INTERVAL_ENVELOPE + emitted_cost,
            object_decl_bytes: decl,
            residual_points,
            emitted,
            families: stats,
            materialized_exact: true,
            search_work: ev.work,
            dsfb_diag: diag,
        });
        Ok(())
    }

    // ---- candidate families -------------------------------------------------

    fn consider_unchanged(&self, target: &Canvas, base: &Canvas, ev: &mut Eval) {
        let valid = target.as_slice() == base.as_slice();
        let mut c = Cand::new("unchanged", Plan::Unchanged, INTERVAL_ENVELOPE);
        c.valid = valid;
        if !valid {
            c.invalid_reason = "state base differs from target";
        }
        ev.consider(c);
    }

    fn consider_clears(&mut self, target: &Canvas, ev: &mut Eval) {
        {
            let valid = uniform_value(target) == Some(self.bg);
            let mut c = Cand::new("fill", Plan::ClearToBg, INTERVAL_ENVELOPE + 2);
            c.valid = valid;
            if !valid {
                c.invalid_reason = "target is not the uniform background";
            }
            ev.consider(c);
        }
        if self.st.instance_count() > 0 {
            let sim = self.sim_clear(&[Transition::ClearInstances]);
            let valid = matches!(&sim, Ok(c) if c == target);
            let mut c = Cand::new(
                "clear_instances",
                Plan::ClearInstancesOnly,
                INTERVAL_ENVELOPE + 1,
            );
            c.valid = valid;
            if !valid {
                c.invalid_reason = "background+overlay differs from target";
            }
            ev.consider(c);
        }
        if self.st.overlay_len() > 0 {
            let sim = self.sim_clear(&[Transition::ClearOverlay]);
            let valid = matches!(&sim, Ok(c) if c == target);
            let mut c = Cand::new(
                "clear_overlay",
                Plan::ClearOverlayOnly,
                INTERVAL_ENVELOPE + 1,
            );
            c.valid = valid;
            if !valid {
                c.invalid_reason = "background+instances differs from target";
            }
            ev.consider(c);
        }
    }

    fn consider_sparse_and_residual(
        &self,
        target: &Canvas,
        base: &Canvas,
        ev: &mut Eval,
        transform_mode: Mode,
    ) {
        if target.as_slice() == base.as_slice() {
            return; // unchanged covers the degenerate case
        }
        ev.add_work(u64::from(self.w) * u64::from(self.h));
        let pts = diff_points(base, target);
        let k = pts.len() as u64;
        // Persistent overlay commit (state sparse). The commit reproduces the
        // target by construction (points are exactly the base/target
        // difference); validity is bounded by the overlay point cap.
        let within_cap = self.st.overlay_len() as u64 + k <= Limits::default().max_overlay_points;
        if within_cap {
            let mut c = Cand::new(
                "sparse",
                Plan::Patch { pts: pts.clone() },
                INTERVAL_ENVELOPE + 5 + 9 * k,
            );
            c.valid = true;
            ev.consider(c);
        }
        // One-shot residual over the base (RAW or rANS block, whatever the
        // Phase-F accounting policy chose).
        let block = encode_point_block(&pts);
        let label: &'static str = if block_is_rans(&block) {
            "rans_residual"
        } else {
            "residual"
        };
        let payload = INTERVAL_ENVELOPE + 5 + block.len() as u64;
        let mut c = Cand::new(label, Plan::Residual { block }, payload);
        c.valid = true;
        ev.consider(c);
        // Phase M: transform-coded residual floor (kind-2 block). Evaluated
        // only when it could beat the point-list baselines (Full) or when the
        // diff is dense (Probe), gated deterministically; exactness is proven
        // by construction (normative inverse) and by the end-to-end verify.
        if transform_mode != Mode::Off {
            let (bx, by) = crate::transform::blocks_per_axis(self.w, self.h);
            let nblocks = bx.checked_mul(by).unwrap_or(0);
            let mlen = nblocks.div_ceil(8) as u64;
            let canvas = u64::from(self.w) * u64::from(self.h);
            // A raw point list costs 9 B/point; the transform payload costs at
            // least the mask + two container envelopes. Below that it cannot
            // win, so it is not evaluated.
            let possible = 9 * k >= mlen + 64;
            let dense = k >= canvas / 16;
            let go = match transform_mode {
                Mode::Full => possible,
                Mode::Probe => possible && dense,
                Mode::Off => false,
            };
            if go {
                ev.add_work(canvas.saturating_mul(2));
                if let Some(block) = build_transform_block(base, target) {
                    ev.add_work(16 * coded_blocks(&block, self.w, self.h) as u64);
                    let payload = INTERVAL_ENVELOPE + 5 + block.len() as u64;
                    let mut c = Cand::new("transform_residual", Plan::Residual { block }, payload);
                    c.valid = true;
                    ev.consider(c);
                }
            }
        }
    }

    /// The unified reset candidate — the RAW sentinel. Resetting the state to
    /// one full-canvas instance of the target's own content always reproduces
    /// the target exactly. When the content is new this stores a full raster
    /// object (family `raw`); when the content already exists in the object
    /// library it is exact reuse (family `exact_ref`, zero declaration bytes).
    /// `force_raw` labels the reuse as `raw` (VOLE raster-only baseline mode).
    /// Evaluate one reset-to-full-canvas-object candidate (RAW sentinel).
    fn consider_reset(&mut self, target: &Canvas, ev: &mut Eval, force_raw: bool) {
        let content = self.content_of_target(target);
        ev.add_work(u64::from(self.w) * u64::from(self.h));
        let new = self.is_new_content(&content);
        let label = if force_raw || new { "raw" } else { "exact_ref" };
        let decl = if new {
            decl_bytes(self.w, self.h, &content)
        } else {
            0
        };
        let mut c = Cand::new(
            label,
            Plan::Reset { new_content: new },
            RESET_INTERVAL_COST + decl,
        );
        c.valid = true;
        ev.consider(c);
    }

    /// Phase N: whole-frame procedural-generator candidates. Every candidate
    /// is validated by rendering the *normative* generator field and comparing
    /// it with the target byte-for-byte; a fit that is not exact is presented
    /// only as a generator+residual candidate with its exact correction
    /// counted, and only when the fit explains at least 15/16 of the pixels
    /// (otherwise RAW is the honest floor). `frame0` disables the
    /// residual-closure candidate (frame 0 cannot carry canvas ops).
    fn consider_generators(&self, target: &Canvas, ev: &mut Eval, mode: Mode, frame0: bool) {
        if self.opts.raster_only || mode == Mode::Off {
            return;
        }
        let fits = fit_generators(target, mode == Mode::Probe);
        if fits.is_empty() {
            return;
        }
        let wh = u64::from(self.w) * u64::from(self.h);
        for gen in fits {
            if gen.check().is_err() {
                continue;
            }
            // A zero-slope program is a uniform fill (cheaper elsewhere).
            let uniform = matches!(
                gen,
                crate::generator::Generator::Gradient { sx: 0, sy: 0, .. }
                    | crate::generator::Generator::Periodic { sx: 0, sy: 0, .. }
            );
            if uniform {
                continue;
            }
            ev.add_work(wh);
            let content = Content::Generator(gen);
            let new = self.is_new_content(&content);
            let decl = if new {
                decl_bytes(self.w, self.h, &content)
            } else {
                0
            };
            let render = match render_field(self.w, self.h, gen) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let label = if new { "generator" } else { "exact_ref" };
            if render.as_slice() == target.as_slice() {
                let payload = if frame0 {
                    decl + checkpoint_bytes(1)
                } else {
                    RESET_INTERVAL_COST + decl
                };
                let mut c = Cand::new(
                    label,
                    Plan::Generator {
                        new_content: new,
                        gen,
                    },
                    payload,
                );
                c.valid = true;
                ev.consider(c);
            } else if !frame0 {
                // Exact residual correction, counted in full (§21: a
                // hypothesis that cannot reproduce the target must carry its
                // residual or lose the court).
                let pts = diff_points(&render, target);
                let k = pts.len() as u64;
                if k * 16 <= wh {
                    ev.add_work(k);
                    let block = encode_point_block(&pts);
                    let payload = RESET_INTERVAL_COST + decl + 5 + block.len() as u64;
                    let mut c = Cand::new(
                        "generator_residual",
                        Plan::GenResidual {
                            new_content: new,
                            gen,
                            block,
                        },
                        payload,
                    );
                    c.valid = true;
                    ev.consider(c);
                }
            }
        }
    }

    /// Evaluate one 1-rect "screen scroll" candidate (rect plus residual) and,
    /// when the rect alone reproduces the target, the pure copy candidate.
    fn screen_scroll_candidate(
        &self,
        target: &Canvas,
        base: &Canvas,
        prev: &Canvas,
        ev: &mut Eval,
        op: Transition,
    ) {
        let limits = Limits::default();
        ev.add_work(u64::from(self.w) * u64::from(self.h));
        let mut scratch = base.clone();
        if materialize::apply_canvas_op(&mut scratch, prev, &op, &limits).is_err() {
            return;
        }
        if scratch.as_slice() == target.as_slice() {
            let mut c = Cand::new(
                "copy_rect",
                Plan::Copies { ops: vec![op] },
                INTERVAL_ENVELOPE + 25,
            );
            c.valid = true;
            ev.consider(c);
            return;
        }
        let pts = diff_points(&scratch, target);
        if pts.is_empty() {
            return;
        }
        let block = encode_point_block(&pts);
        let payload = INTERVAL_ENVELOPE + 25 + 5 + block.len() as u64;
        let mut c = Cand::new(
            "copy_residual",
            Plan::CopyResidual {
                ops: vec![op],
                block,
            },
            payload,
        );
        c.valid = true;
        ev.consider(c);
    }

    fn consider_copies_full(
        &mut self,
        target: &Canvas,
        base: &Canvas,
        prev: &Canvas,
        ev: &mut Eval,
    ) {
        if self.search_space == SearchSpace::Exhaustive {
            // 2D toroidal whole-canvas wrap scrolls when the candidate count
            // is small.
            let cands = (u64::from(self.w) + 1) * (u64::from(self.h) + 1);
            if cands <= TOROIDAL_CANDIDATE_GATE {
                for dy in 0..self.h as i64 {
                    for dx in 0..self.w as i64 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        self.wrap_candidate(target, base, prev, ev, dx, dy);
                    }
                }
            }
            // 1D vertical wrap by s.
            for s in 1..self.h as i64 {
                self.wrap_candidate(target, base, prev, ev, 0, s);
            }
            // 1D horizontal wrap by s.
            for s in 1..self.w as i64 {
                self.wrap_candidate(target, base, prev, ev, s, 0);
            }
            // Screen scrolls (new content enters at the exposed edge): one
            // rect plus a residual over the delta.
            for s in 1..self.h as i64 {
                let op = rect_op(0, s, self.w, (self.h as i64 - s) as u32, 0, 0);
                self.screen_scroll_candidate(target, base, prev, ev, op);
            }
            for s in 1..self.h as i64 {
                let op = rect_op(0, 0, self.w, (self.h as i64 - s) as u32, 0, s);
                self.screen_scroll_candidate(target, base, prev, ev, op);
            }
        } else {
            // Hash-prefiltered 1D scrolls (large canvases).
            let th = row_hashes(target);
            let ph = row_hashes(prev);
            for s in 1..self.h as i64 {
                if wrap_rows_exact(target, prev, s, &th, &ph) {
                    self.wrap_candidate(target, base, prev, ev, 0, s);
                }
            }
            for s in 1..self.h as i64 {
                let op = rect_op(0, s, self.w, (self.h as i64 - s) as u32, 0, 0);
                if self.screen_scroll_top_matches(target, prev, s, &th, &ph) {
                    self.screen_scroll_candidate(target, base, prev, ev, op);
                }
            }
            for s in 1..self.h as i64 {
                let op = rect_op(0, 0, self.w, (self.h as i64 - s) as u32, 0, s);
                if self.screen_scroll_bottom_matches(target, prev, s, &th, &ph) {
                    self.screen_scroll_candidate(target, base, prev, ev, op);
                }
            }
        }
    }

    /// Probe evaluation of the COPY_RECT family: replay the previous winner's
    /// rect ops when it was a copy program (steady scrolls reproduce exactly),
    /// otherwise evaluate the small deterministic default probe (wraps and
    /// vertical screen scrolls by 1..=DEFAULT_PROBE_SHIFTS).
    fn consider_copies_probe(
        &mut self,
        target: &Canvas,
        base: &Canvas,
        prev: &Canvas,
        ev: &mut Eval,
    ) {
        if !self.last_copy_ops.is_empty() {
            self.consider_ops_program(target, base, prev, ev, self.last_copy_ops.clone());
            return;
        }
        let s = crate::dsfb::DEFAULT_PROBE_SHIFTS;
        for d in 1..=s {
            self.wrap_candidate(target, base, prev, ev, 0, d);
            self.wrap_candidate(target, base, prev, ev, d, 0);
        }
        for s2 in 1..=s {
            let op = rect_op(0, s2, self.w, (self.h as i64 - s2) as u32, 0, 0);
            self.screen_scroll_candidate(target, base, prev, ev, op);
            let op = rect_op(0, 0, self.w, (self.h as i64 - s2) as u32, 0, s2);
            self.screen_scroll_candidate(target, base, prev, ev, op);
        }
    }

    /// Validate and consider one copy-ops program: when the ops reproduce the
    /// target exactly it is a pure copy candidate; otherwise the exact
    /// residual is appended (copy+residual candidate, always valid).
    fn consider_ops_program(
        &self,
        target: &Canvas,
        base: &Canvas,
        prev: &Canvas,
        ev: &mut Eval,
        ops: Vec<Transition>,
    ) {
        let limits = Limits::default();
        let wh = u64::from(self.w) * u64::from(self.h);
        ev.add_work(wh.saturating_mul(ops.len() as u64 + 1));
        let mut scratch = base.clone();
        let ok = ops
            .iter()
            .all(|op| materialize::apply_canvas_op(&mut scratch, prev, op, &limits).is_ok());
        if !ok {
            return;
        }
        if scratch.as_slice() == target.as_slice() {
            let payload = INTERVAL_ENVELOPE + 25 * ops.len() as u64;
            let mut c = Cand::new("copy_rect", Plan::Copies { ops }, payload);
            c.valid = true;
            ev.consider(c);
            return;
        }
        let pts = diff_points(&scratch, target);
        if pts.is_empty() {
            return;
        }
        let block = encode_point_block(&pts);
        let payload = INTERVAL_ENVELOPE + 25 * ops.len() as u64 + 5 + block.len() as u64;
        let mut c = Cand::new("copy_residual", Plan::CopyResidual { ops, block }, payload);
        c.valid = true;
        ev.consider(c);
    }

    /// Prev-frame diff: whole-canvas copy + one-shot residual (or a plain
    /// repeat when the frame is unchanged relative to the previous frame).
    fn consider_prev_diff(&self, target: &Canvas, base: &Canvas, prev: &Canvas, ev: &mut Eval) {
        let limits = Limits::default();
        let wh = u64::from(self.w) * u64::from(self.h);
        let op = rect_op(0, 0, self.w, self.h, 0, 0);
        let mut scratch = base.clone();
        ev.add_work(wh.saturating_mul(2));
        if materialize::apply_canvas_op(&mut scratch, prev, &op, &limits).is_ok() {
            if scratch.as_slice() == target.as_slice() {
                let mut c = Cand::new(
                    "prev_diff",
                    Plan::Copies { ops: vec![op] },
                    INTERVAL_ENVELOPE + 25,
                );
                c.valid = true;
                ev.consider(c);
            } else {
                let pts = diff_points(&scratch, target);
                let block = encode_point_block(&pts);
                let payload = INTERVAL_ENVELOPE + 25 + 5 + block.len() as u64;
                let mut c = Cand::new(
                    "prev_diff",
                    Plan::CopyResidual {
                        ops: vec![op],
                        block,
                    },
                    payload,
                );
                c.valid = true;
                ev.consider(c);
            }
        }
    }

    /// Evaluate one exact toroidal-wrap copy candidate (valid iff the rect
    /// program reproduces the target).
    fn wrap_candidate(
        &self,
        target: &Canvas,
        base: &Canvas,
        prev: &Canvas,
        ev: &mut Eval,
        dx: i64,
        dy: i64,
    ) {
        let ops = wrap_rects(self.w, self.h, dx, dy);
        if ops.is_empty() {
            return;
        }
        let limits = Limits::default();
        ev.add_work(u64::from(self.w) * u64::from(self.h));
        let mut scratch = base.clone();
        let ok = ops
            .iter()
            .all(|op| materialize::apply_canvas_op(&mut scratch, prev, op, &limits).is_ok());
        let valid = ok && scratch.as_slice() == target.as_slice();
        let payload = INTERVAL_ENVELOPE + 25 * ops.len() as u64;
        let mut c = Cand::new("copy_rect", Plan::Copies { ops }, payload);
        c.valid = valid;
        if !valid {
            c.invalid_reason = "wrap scroll does not reproduce target";
        }
        ev.consider(c);
    }

    /// Prefilter for the up-scroll candidate: target rows `[0, H-s)` must equal
    /// prev rows `[s, H)` (hash then bytes).
    fn screen_scroll_top_matches(
        &self,
        target: &Canvas,
        prev: &Canvas,
        s: i64,
        th: &[u64],
        ph: &[u64],
    ) -> bool {
        let hs = (self.h as i64 - s) as usize;
        if hs == 0 {
            return false;
        }
        if !(0..hs).all(|y| th[y] == ph[y + s as usize]) {
            return false;
        }
        rows_equal(target, prev, 0..hs, s as usize)
    }

    fn screen_scroll_bottom_matches(
        &self,
        target: &Canvas,
        prev: &Canvas,
        s: i64,
        th: &[u64],
        ph: &[u64],
    ) -> bool {
        let hs = (self.h as i64 - s) as usize;
        if hs == 0 {
            return false;
        }
        if !(0..hs).all(|y| th[y + s as usize] == ph[y]) {
            return false;
        }
        rows_equal(target, prev, s as usize..self.h as usize, 0)
    }

    /// Phase K — variable-region family. Partition the target/base diff into
    /// tiles of each requested granularity; for every tile that holds diff
    /// samples, declare the tile's diff bounding box as an immutable object
    /// (the target's own sub-rectangle) and paint it above the base with a
    /// fresh instance. Rectangles are disjoint (one per tile), so paint order
    /// is irrelevant and every changed sample is covered by construction;
    /// unchanged samples under a rectangle are re-painted with their own
    /// target value (exact). The candidate is evaluated only when a diff
    /// exists, the diff is at most a quarter of the canvas (otherwise the
    /// whole-frame reset sentinel is at least as cheap and this family is
    /// documented as skipped), no changed sample is shadowed by a persistent
    /// overlay point (overlay paints above every instance), and the partition
    /// fits the rectangle cap. `probe` evaluates only the fixed probe
    /// granularity; Full walks the 64→32→16→8 ladder.
    fn consider_regions(
        &self,
        target: &Canvas,
        base: &Canvas,
        ev: &mut Eval,
        granularities: &[u32],
    ) {
        let wh = u64::from(self.w) * u64::from(self.h);
        ev.add_work(wh);
        let pts = diff_points(base, target);
        let k = pts.len() as u64;
        if k == 0 || k > REGION_MAX_DIFF || k > wh / 4 {
            return;
        }
        if self.st.overlay_len() > 0
            && pts
                .iter()
                .any(|(x, y, _)| self.st.overlay_pixel(*x, *y).is_some())
        {
            return;
        }
        for &g in granularities {
            if let Some(c) = self.region_candidate(&pts, target, g) {
                ev.consider(c);
            }
        }
    }

    /// Build the region candidate for one granularity (None when the
    /// partition exceeds the rectangle cap).
    fn region_candidate(&self, pts: &[(i64, i64, u8)], target: &Canvas, g: u32) -> Option<Cand> {
        let tile_w = usize::try_from(self.w.div_ceil(g)).ok()?;
        let tile_h = usize::try_from(self.h.div_ceil(g)).ok()?;
        let cell = |x: i64, y: i64| -> usize {
            (y / i64::from(g)) as usize * tile_w + (x / i64::from(g)) as usize
        };
        let mut minx = vec![i64::MAX; tile_w * tile_h];
        let mut maxx = vec![i64::MIN; tile_w * tile_h];
        let mut miny = vec![i64::MAX; tile_w * tile_h];
        let mut maxy = vec![i64::MIN; tile_w * tile_h];
        for (x, y, _) in pts {
            let c = cell(*x, *y);
            minx[c] = minx[c].min(*x);
            maxx[c] = maxx[c].max(*x);
            miny[c] = miny[c].min(*y);
            maxy[c] = maxy[c].max(*y);
        }
        let mut rects: Vec<RegionSpec> = Vec::new();
        for ty in 0..tile_h {
            for tx in 0..tile_w {
                let c = ty * tile_w + tx;
                if minx[c] == i64::MAX {
                    continue;
                }
                if rects.len() >= REGION_MAX_RECTS {
                    return None;
                }
                let (x0, y0) = (minx[c], miny[c]);
                let (x1, y1) = (maxx[c], maxy[c]);
                let w = u32::try_from(x1 - x0 + 1).ok()?;
                let h = u32::try_from(y1 - y0 + 1).ok()?;
                let content = rect_content(target, x0, y0, w, h);
                rects.push(RegionSpec {
                    x: x0,
                    y: y0,
                    w,
                    h,
                    content,
                });
            }
        }
        if rects.is_empty() {
            return None;
        }
        let mut decl = 0u64;
        for r in &rects {
            if self.is_new_region(r.w, r.h, &r.content) {
                decl += decl_bytes(r.w, r.h, &r.content);
            }
        }
        let payload = INTERVAL_ENVELOPE + 17 * rects.len() as u64 + decl;
        let mut c = Cand::new("regions", Plan::Regions { rects }, payload);
        c.valid = true;
        Some(c)
    }

    /// Whole-pixel instance translation. `probe` restricts evaluation to the
    /// previous winning delta (steady motion), Full evaluates the window.
    fn consider_translation(&self, target: &Canvas, ev: &mut Eval, probe: bool) {
        if self.st.instance_count() != 1 || self.st.overlay_len() > 0 {
            return;
        }
        let Some(inst) = self.st.instances().next() else {
            return;
        };
        let Some(obj) = self.st.object(inst.object_id) else {
            return;
        };
        if obj.width() != self.w || obj.height() != self.h {
            return;
        }
        let content = match obj.generator() {
            Some(gen) => Content::Generator(gen),
            None => match obj.fill_value() {
                Some(v) => Content::Fill(v),
                None => Content::Raster(obj.samples().expect("stored object").to_vec()),
            },
        };
        let test = |dx: i64, dy: i64, ev: &mut Eval| {
            ev.add_work(u64::from(self.w) * u64::from(self.h));
            let valid = blit_matches(
                &content,
                self.w,
                self.h,
                target,
                inst.x + dx,
                inst.y + dy,
                self.bg,
            );
            let mut c = Cand::new(
                "translation",
                Plan::SetPosition { dx, dy },
                INTERVAL_ENVELOPE + 13,
            );
            c.valid = valid;
            if !valid {
                c.invalid_reason = "translated content differs from target";
            }
            ev.consider(c);
        };
        if probe {
            if let Some((dx, dy)) = self.last_translation {
                test(dx, dy, ev);
            }
        } else {
            let r = self.opts.translation_window;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    test(dx, dy, ev);
                }
            }
        }
        // Advance-only: the live instance carries a persistent translation and
        // a single advance reproduces the target.
        let (vx, vy) = self.st.velocity(inst.id);
        if vx != 0 || vy != 0 {
            ev.add_work(u64::from(self.w) * u64::from(self.h));
            let valid = blit_matches(
                &content,
                self.w,
                self.h,
                target,
                inst.x + vx,
                inst.y + vy,
                self.bg,
            );
            let mut c = Cand::new("translation", Plan::Advance, INTERVAL_ENVELOPE + 1);
            c.valid = valid;
            if !valid {
                c.invalid_reason = "translation advance differs from target";
            }
            ev.consider(c);
        }
    }

    // -- assembly / verification ----------------------------------------------

    fn assemble(&mut self) -> Result<Vec<u8>, VoleError> {
        let mut objects = Vec::new();
        for (id, obj) in self.st.objects() {
            objects.push((id.0, obj.clone()));
        }
        let checkpoint = self.checkpoint_instances.clone();
        let mut timeline: Vec<(u64, Vec<Transition>)> = Vec::new();
        for d in &self.decisions {
            if d.frame > 0 {
                timeline.push((d.frame, d.emitted.clone()));
            }
        }
        let bytes = encode_stream(self.w, self.h, self.bg, &objects, &checkpoint, &timeline)?;
        // Accounting invariant: the per-decision incremental payloads plus the
        // fixed header/trailer must equal the real stream size.
        let mut sum = 24u64 + 32u64;
        for d in &self.decisions {
            sum = sum
                .checked_add(d.winner_payload_bytes)
                .ok_or(VoleError::ArithmeticOverflow)?;
        }
        if sum != bytes.len() as u64 {
            return Err(VoleError::ApiConstraint(
                "decision accounting does not match stream bytes (encoder invariant)",
            ));
        }
        Ok(bytes)
    }

    fn verify(&self, bytes: &[u8]) -> Result<bool, VoleError> {
        let parsed = decoder::decode_bytes(bytes)?;
        let frames = decoder::materialize_all(&parsed)?;
        if frames.len() != self.frames.len() {
            return Ok(false);
        }
        for (a, b) in frames.iter().zip(self.frames.iter()) {
            if a != b {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Whether `target` rows `ty` equal `prev` rows `py` (byte-exact).
fn rows_equal(target: &Canvas, prev: &Canvas, ty: std::ops::Range<usize>, py: usize) -> bool {
    let w = target.width() as usize;
    let t = target.as_slice();
    let p = prev.as_slice();
    for (i, y) in ty.enumerate() {
        if t[y * w..(y + 1) * w] != p[(py + i) * w..(py + i + 1) * w] {
            return false;
        }
    }
    true
}

/// Rectangles copying `prev` onto the canvas shifted toroidally by `(dx, dy)`
/// (`target(x,y) = prev((x+dx)%W, (y+dy)%H)`). Two rects when one axis is
/// zero, four when both are nonzero.
fn wrap_rects(w: u32, h: u32, dx: i64, dy: i64) -> Vec<Transition> {
    let ww = w as i64;
    let hh = h as i64;
    let dx = dx.rem_euclid(ww);
    let dy = dy.rem_euclid(hh);
    debug_assert!(dx != 0 || dy != 0);
    let mut ops = Vec::with_capacity(4);
    if ww - dx > 0 && hh - dy > 0 {
        ops.push(rect_op(dx, dy, (ww - dx) as u32, (hh - dy) as u32, 0, 0));
    }
    if dx > 0 && hh - dy > 0 {
        ops.push(rect_op(0, dy, dx as u32, (hh - dy) as u32, ww - dx, 0));
    }
    if ww - dx > 0 && dy > 0 {
        ops.push(rect_op(dx, 0, (ww - dx) as u32, dy as u32, 0, hh - dy));
    }
    if dx > 0 && dy > 0 {
        ops.push(rect_op(0, 0, dx as u32, dy as u32, ww - dx, hh - dy));
    }
    ops
}

fn rect_op(sx: i64, sy: i64, width: u32, height: u32, dx: i64, dy: i64) -> Transition {
    Transition::CopyRect {
        src_x: sx,
        src_y: sy,
        width,
        height,
        dst_x: dx,
        dst_y: dy,
    }
}

/// The COPY/MOVE ops of a winning program (the probe-replay geometry).
fn copy_ops_only(trs: &[Transition]) -> Vec<Transition> {
    trs.iter()
        .filter(|t| matches!(t, Transition::CopyRect { .. } | Transition::MoveRect { .. }))
        .cloned()
        .collect()
}

/// Number of residual points a block decodes to (0 on any structural issue).
fn decode_point_count(block: &[u8], w: u32, h: u32, limits: &Limits) -> u64 {
    if block.len() < 9 {
        return 0;
    }
    if block.first() == Some(&rans::KIND_TSF) {
        // Transform residual: the residual closes every masked cell; report
        // the masked cells (popcount × 16 block cells) as the point count.
        let (bx, by) = crate::transform::blocks_per_axis(w, h);
        let nblocks = bx.saturating_mul(by);
        let mlen = nblocks.div_ceil(8);
        if block.len() < 2 + mlen + 8 {
            return 0;
        }
        let mask = &block[2..2 + mlen];
        let mut coded = 0u64;
        for k in 0..nblocks {
            if mask[k >> 3] & (1 << (k & 7)) != 0 {
                coded += 1;
            }
        }
        coded * 16
    } else {
        let len = u64::from_le_bytes(block[1..9].try_into().expect("8-byte window"));
        if len > limits.max_residual_bytes {
            return 0;
        }
        len / 9
    }
}

/// Compare `target` against `content` blitted at `(dx, dy)` over uniform `bg`
/// (the exact materializer semantics for one full-canvas instance).
fn blit_matches(
    content: &Content,
    w: u32,
    h: u32,
    target: &Canvas,
    dx: i64,
    dy: i64,
    bg: u8,
) -> bool {
    let cw = target.width() as i64;
    let ch = target.height() as i64;
    match content {
        Content::Fill(v) => {
            let x0 = dx.max(0);
            let x1 = (dx + w as i64).min(cw);
            let y0 = dy.max(0);
            let y1 = (dy + h as i64).min(ch);
            for y in y0..y1 {
                for x in x0..x1 {
                    if target.get(x as u32, y as u32) != *v {
                        return false;
                    }
                }
            }
            true
        }
        Content::Raster(data) => {
            for y in 0..ch {
                for x in 0..cw {
                    let inside = x >= dx && y >= dy && x < dx + w as i64 && y < dy + h as i64;
                    let expected = if inside {
                        let sx = (x - dx) as usize;
                        let sy = (y - dy) as usize;
                        data[sy * w as usize + sx]
                    } else {
                        bg
                    };
                    if target.get(x as u32, y as u32) != expected {
                        return false;
                    }
                }
            }
            true
        }
        Content::Generator(gen) => {
            // A generator content blit samples the program at the translated
            // content-local coordinate (the same rule the materializer runs).
            for y in 0..ch {
                for x in 0..cw {
                    let inside = x >= dx && y >= dy && x < dx + w as i64 && y < dy + h as i64;
                    let expected = if inside {
                        gen.sample(x - dx, y - dy)
                    } else {
                        bg
                    };
                    if target.get(x as u32, y as u32) != expected {
                        return false;
                    }
                }
            }
            true
        }
    }
}

/// Row hashes (FNV-1a) of a canvas — an equality *prefilter* only; acceptance
/// is always confirmed byte-for-byte.
fn row_hashes(c: &Canvas) -> Vec<u64> {
    let w = c.width() as usize;
    c.as_slice().chunks_exact(w).map(fnv1a).collect()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Whether target rows equal prev rows shifted down by `dy` (toroidal),
/// verified byte-exact (hashes are only a prefilter).
fn wrap_rows_exact(target: &Canvas, prev: &Canvas, dy: i64, th: &[u64], ph: &[u64]) -> bool {
    let h = target.height() as usize;
    let w = target.width() as usize;
    let dy = dy.rem_euclid(target.height() as i64) as usize;
    for y in 0..h {
        if th[y] != ph[(y + dy) % h] {
            return false;
        }
    }
    let t = target.as_slice();
    let p = prev.as_slice();
    for y in 0..h {
        let sy = (y + dy) % h;
        if t[y * w..(y + 1) * w] != p[sy * w..(sy + 1) * w] {
            return false;
        }
    }
    true
}

/// Background candidate values for the sweep: `{0, 255}`, the frame-0 corners,
/// and the global mode over all frames.
fn bg_candidates(frames: &[Canvas]) -> Vec<u8> {
    let mut set: Vec<u8> = vec![0, 255];
    if let Some(f0) = frames.first() {
        let w = f0.width() as usize;
        let h = f0.height() as usize;
        if w > 0 && h > 0 {
            let s = f0.as_slice();
            set.push(s[0]);
            set.push(s[w - 1]);
            set.push(s[(h - 1) * w]);
            set.push(s[h * w - 1]);
        }
    }
    let mut counts = [0u64; 256];
    for f in frames {
        for &b in f.as_slice() {
            counts[b as usize] += 1;
        }
    }
    let mode = counts
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| **c)
        .map(|(v, _)| v as u8)
        .unwrap_or(0);
    set.push(mode);
    set.sort_unstable();
    set.dedup();
    set
}

/// Encode an observed Gray8 raster sequence into a standalone `.vole` stream
/// (Phase G exhaustive inverse proceduralization). The stream is verified
/// end-to-end before it is returned.
pub fn encode_frames(frames: &[Canvas], opts: &EncodeOptions) -> Result<EncodeReport, VoleError> {
    if frames.is_empty() {
        return Err(VoleError::ApiConstraint("encode needs at least one frame"));
    }
    let mut best: Option<EncodeReport> = None;
    let mut tried: Vec<(u8, u64)> = Vec::new();
    let bg_list: Vec<u8> = if let Some(bg) = opts.background {
        vec![bg]
    } else if opts.bg_sweep {
        bg_candidates(frames)
    } else {
        vec![0]
    };
    for bg in bg_list {
        let mut enc = Encoder::new(frames, bg, opts)?;
        let report = enc.run()?;
        let total = report.vole.len() as u64;
        tried.push((bg, total));
        let replace = match &best {
            None => true,
            Some(b) => total < b.vole.len() as u64,
        };
        if replace {
            best = Some(report);
        }
    }
    let mut report = best.ok_or(VoleError::ApiConstraint("encode produced no report"))?;
    report.bg_sweep = BgSweep {
        enabled: opts.bg_sweep,
        tried,
        chosen: report.background,
    };
    Ok(report)
}

/// Convenience: interpret a concatenated Gray8 `.raw` buffer as frames.
pub fn frames_from_raw(data: &[u8], width: u32, height: u32) -> Result<Vec<Canvas>, VoleError> {
    let per = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| VoleError::ArithmeticOverflow)?;
    if per == 0 || !data.len().is_multiple_of(per) {
        return Err(VoleError::LengthMismatch);
    }
    let mut out = Vec::with_capacity(data.len() / per);
    for chunk in data.chunks_exact(per) {
        out.push(Canvas::from_parts(width, height, chunk.to_vec())?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::content_id_of;
    use crate::object::Object;

    fn sample_raster() -> Vec<u8> {
        (0..16u8).collect()
    }

    #[test]
    fn content_identity_matches_object_identity() {
        let data = sample_raster();
        let obj = Object::raster(4, 4, data.clone()).unwrap();
        let content = Content::Raster(data);
        let via_object = content_id_of(&obj);
        let via_record: [u8; 32] = integr::digest(&content_record(4, 4, &content));
        assert_eq!(via_object.as_bytes().as_slice(), &via_record[..]);
        let fill = Object::fill(8, 4, 77).unwrap();
        let cfill = Content::Fill(77);
        assert_eq!(
            content_id_of(&fill).as_bytes().as_slice(),
            &integr::digest(&content_record(8, 4, &cfill))[..]
        );
    }

    #[test]
    fn decl_bytes_are_the_wire_lengths() {
        assert_eq!(decl_bytes(4, 4, &Content::Fill(9)), FILL_DECL);
        assert_eq!(
            decl_bytes(4, 4, &Content::Raster(sample_raster())),
            OBJECT_DECL_HEADER + 16
        );
    }

    #[test]
    fn diff_points_are_x_major_strict() {
        let a = Canvas::from_parts(3, 2, vec![0, 1, 2, 3, 4, 5]).unwrap();
        let b = Canvas::from_parts(3, 2, vec![0, 9, 2, 3, 4, 8]).unwrap();
        let pts = diff_points(&a, &b);
        assert_eq!(pts, vec![(1, 0, 9), (2, 1, 8)]);
        assert!(pts
            .windows(2)
            .all(|w| { (w[0].0, w[0].1) < (w[1].0, w[1].1) }));
    }

    #[test]
    fn wrap_rects_reproduce_toroidal_shift() {
        let w = 4u32;
        let h = 3u32;
        let src = Canvas::from_parts(w, h, (0..12).map(|i| i as u8).collect()).unwrap();
        for (dx, dy) in [(1i64, 2i64), (3, 0), (0, 1), (2, 2)] {
            let ops = wrap_rects(w, h, dx, dy);
            assert!(!ops.is_empty());
            let mut dst = Canvas::zeroed(w, h, &Limits::default()).unwrap();
            for op in &ops {
                materialize::apply_canvas_op(&mut dst, &src, op, &Limits::default()).unwrap();
            }
            for y in 0..h {
                for x in 0..w {
                    let expect = src.get((x + dx as u32) % w, (y + dy as u32) % h);
                    assert_eq!(dst.get(x, y), expect, "at ({x},{y}) for ({dx},{dy})");
                }
            }
        }
    }

    #[test]
    fn account_stream_buckets_sum_to_total() {
        let court = crate::demo::StaticSceneCourt::default();
        let bytes = court.vole().unwrap();
        let cost = account_stream(&bytes).unwrap();
        let sum = cost.header_bytes
            + cost.object_bytes
            + cost.checkpoint_bytes
            + cost.transition_bytes
            + cost.residual_bytes
            + cost.model_bytes
            + cost.state_bytes
            + cost.dictionary_bytes
            + cost.index_bytes
            + cost.integrity_bytes;
        assert_eq!(sum, cost.total_bytes);
        assert_eq!(cost.total_bytes, bytes.len() as u64);
    }
}
