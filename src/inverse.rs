//! Phase G: exhaustive inverse proceduralization (raster-origin encoder).
//!
//! This module builds the first true **inverse proceduralizer**: it accepts an
//! observed Gray8 raster sequence (`Vec<Canvas>`) and, per frame, evaluates an
//! exhaustive candidate space of bounded procedural explanations:
//!
//! ```text
//! RAW · FILL · UNCHANGED · EXACT_OBJECT_REF · SPARSE · COPY_RECT ·
//! TRANSLATION · RANS_RESIDUAL
//! ```
//!
//! plus the composite programs those families compose into (screen-scroll +
//! residual strip, prev-frame diff). Every candidate is a *declarative program*
//! over the normative state model — a list of state [`Transition`]s plus a
//! list of canvas ops — and its correctness is established by materializing
//! its expected frame through the same normative primitives the decoder runs
//! (`materialize_full`, `rect_copy`, the Phase-F residual block decode) and
//! comparing byte-for-byte with the target observation. The encoder never
//! trusts a hypothesis from appearance; a candidate that cannot reproduce the
//! target exactly is rejected and recorded, and the complete-cost winner
//! (persisted bytes, §31-style accounting) is the only program emitted. The
//! emitted stream is always verified end-to-end: it is decoded with the
//! normative decoder and every materialized frame must equal the input raster,
//! or the encoder returns a typed error instead of a stream.
//!
//! # Scope honesty
//!
//! Phase G is *whole-frame* granularity (variable regions arrive in Phase K),
//! so candidates reference full-canvas objects only. The enumerated candidate
//! space is finite and deterministic per frame; the exact members are
//! documented on [`FrameDecision`]. The full per-candidate materialization
//! court runs on canvases up to a declared size gate; above it, 1D scroll
//! candidates are row-hash-prefiltered (equality is always confirmed with a
//! byte comparison before a candidate is accepted, so exactness never depends
//! on a hash). Per-frame decisions are greedy and independent: temporal
//! re-optimization (velocity/trajectory collapse, checkpoint placement,
//! residual→persistent-content promotion) is Phase O, and this module reports
//! the temporal gaps it measures rather than hiding them.

use std::collections::HashMap;

use crate::{
    checked::ByteReader,
    decoder,
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
        Transition::PatchSparse { points } => 5 + 9 * points.len() as u64,
        Transition::CopyRect { .. } | Transition::MoveRect { .. } => 25,
        Transition::Residual { block } => 5 + block.len() as u64,
        Transition::DeclareObject(..) | Transition::DeclareFill { .. } => 0,
    }
}

/// Serialized declaration bytes of a never-before-seen object.
fn decl_bytes(w: u32, h: u32, content: &Content) -> u64 {
    let _ = w;
    let _ = h;
    match content {
        Content::Fill(_) => FILL_DECL,
        Content::Raster(data) => OBJECT_DECL_HEADER + data.len() as u64,
    }
}

/// Immutable object content (whole-frame granularity in Phase G).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    /// Uniform fill over the declared box.
    Fill(u8),
    /// Tight row-major Gray8 raster.
    Raster(Vec<u8>),
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
// Accounting (§31)
// ---------------------------------------------------------------------------

/// Complete physical accounting of a `.vole` byte stream with every byte
/// classified into a declared bucket. The ten primary buckets sum to
/// `total_bytes`; the two `*_split` fields are informational sub-buckets of
/// `object_bytes` and are excluded from that invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RepresentationCost {
    /// Stream header (magic, binding, geometry).
    pub header_bytes: u64,
    /// Immutable object declarations (descriptor + samples).
    pub object_bytes: u64,
    /// The checkpoint record (background + interval-0 instances).
    pub checkpoint_bytes: u64,
    /// Interval envelopes + state transitions + COPY/MOVE canvas ops.
    pub transition_bytes: u64,
    /// Per-frame residual op wire bytes (tag + length prefix + block).
    pub residual_bytes: u64,
    /// Inline entropy models inside residual blocks.
    pub model_bytes: u64,
    /// Persistent procedural state snapshot bytes (0 in v1: state is laid down
    /// by the checkpoint and transitions, already counted above).
    pub state_bytes: u64,
    /// Shared dictionary bytes (0 in v1).
    pub dictionary_bytes: u64,
    /// Index bytes (0 in v1; no index record yet).
    pub index_bytes: u64,
    /// Integrity trailer.
    pub integrity_bytes: u64,
    /// Total stream bytes.
    pub total_bytes: u64,
    /// Informational: raster-object declarations (tag + id + geometry +
    /// samples); a sub-bucket of `object_bytes`.
    pub raster_object_bytes: u64,
    /// Informational: fill-object declarations; a sub-bucket of `object_bytes`.
    pub fill_object_bytes: u64,
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
    let mut cost = RepresentationCost {
        total_bytes: bytes.len() as u64,
        ..RepresentationCost::default()
    };
    let mut raster_objects = 0u64;
    let mut fill_objects = 0u64;
    let mut raster_samples = 0u64;
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
            0x03 => {
                let _bg = r.u8()?;
                let n = r.pull::<u32>()?;
                r.skip(16 * n as usize)?;
                cost.checkpoint_bytes += checkpoint_bytes(u64::from(n));
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
                        0x2a => {
                            let len = r.pull::<u32>()?;
                            let block = r.take(len as usize)?;
                            cost.transition_bytes += 5;
                            cost.residual_bytes += u64::from(len);
                            if block.first() == Some(&rans::KIND_RANS) {
                                cost.model_bytes += rans::MODEL_SERIALIZED as u64;
                            }
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
    cost.object_bytes = cost.raster_object_bytes + cost.fill_object_bytes;
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

/// Options controlling the Phase-G encoder.
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
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions {
            bg_sweep: true,
            background: None,
            raster_only: false,
            translation_window: 2,
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

/// One declared-object library entry.
#[derive(Debug, Clone)]
struct LibObject {
    id: u32,
    w: u32,
    h: u32,
    content: Content,
}

impl LibObject {
    fn content_matches(&self, target: &Canvas) -> bool {
        match &self.content {
            Content::Fill(v) => uniform_value(target) == Some(*v),
            Content::Raster(data) => {
                data.len() == target.as_slice().len() && data.as_slice() == target.as_slice()
            }
        }
    }
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
    best: Option<Cand>,
}

impl Eval {
    fn new() -> Eval {
        Eval {
            fam: Vec::new(),
            order: 0,
            best: None,
        }
    }

    fn consider(&mut self, c: Cand) {
        self.order += 1;
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
    library: Vec<LibObject>,
    index: HashMap<[u8; 32], usize>,
    st: State,
    prev: Option<Canvas>,
    /// Live instances at the checkpoint (frame 0), for stream assembly.
    checkpoint_instances: Vec<Instance>,
    decisions: Vec<FrameDecision>,
    next_object_id: u32,
    next_instance_id: u32,
    search_space: SearchSpace,
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
        Ok(Encoder {
            w,
            h,
            frames,
            opts: opts.clone(),
            bg,
            library: Vec::new(),
            index: HashMap::new(),
            st,
            prev: None,
            checkpoint_instances: Vec::new(),
            decisions: Vec::new(),
            next_object_id: 1,
            next_instance_id: 1,
            search_space,
        })
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

    /// Register `content` (assigning an id if new), declare it into the
    /// working state, and return its object id.
    fn ensure_object(&mut self, content: Content) -> Result<u32, VoleError> {
        let cid = content_id(self.w, self.h, &content);
        if let Some(&i) = self.index.get(&cid) {
            return Ok(self.library[i].id);
        }
        let id = self.next_object_id;
        self.next_object_id += 1;
        let obj = match &content {
            Content::Fill(v) => Object::fill(self.w, self.h, *v)?,
            Content::Raster(data) => Object::raster(self.w, self.h, data.clone())?,
        };
        self.st.declare_object(ObjectId(id), obj)?;
        self.index.insert(cid, self.library.len());
        self.library.push(LibObject {
            id,
            w: self.w,
            h: self.h,
            content,
        });
        Ok(id)
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
                _ => 0,
            },
            residual_points: 0,
            emitted: Vec::new(),
            families: stats,
            materialized_exact: true,
        });
        Ok(())
    }

    // -- frame k >= 1 ---------------------------------------------------------

    fn encode_frame(&mut self, k: usize) -> Result<(), VoleError> {
        let target = &self.frames[k];
        let limits = Limits::default();
        let base = materialize::materialize_full(&self.st, self.w, self.h, &limits)?;
        let mut ev = Eval::new();
        let mut emitted_cost = 0u64;

        if self.opts.raster_only {
            self.consider_raw(target, &mut ev);
        } else {
            self.consider_unchanged(target, &base, &mut ev);
            self.consider_clears(target, &mut ev);
            self.consider_exact_refs(target, &mut ev);
            self.consider_sparse_and_residual(target, &base, &mut ev);
            let prev = self.prev.clone().expect("previous frame");
            self.consider_copies(target, &base, &prev, &mut ev);
            self.consider_translation(target, &mut ev);
            self.consider_raw(target, &mut ev);
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
            Plan::Unchanged => {}
            Plan::ClearToBg => {
                for tr in [Transition::ClearInstances, Transition::ClearOverlay] {
                    tr.apply(&mut self.st)?;
                    emitted_cost += tr_len(&tr);
                    emitted.push(tr);
                }
            }
            Plan::ClearInstancesOnly => {
                let tr = Transition::ClearInstances;
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
            }
            Plan::ClearOverlayOnly => {
                let tr = Transition::ClearOverlay;
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
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
            }
            Plan::Patch { pts } => {
                residual_points = pts.len() as u64;
                let tr = Transition::PatchSparse {
                    points: pts.clone(),
                };
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
            }
            Plan::Residual { block } => {
                residual_points = decode_point_count(block, &limits);
                emitted_cost += 5 + block.len() as u64;
                emitted.push(Transition::Residual {
                    block: block.clone(),
                });
            }
            Plan::Copies { ops } => {
                for op in ops {
                    emitted_cost += tr_len(op);
                    emitted.push(op.clone());
                }
            }
            Plan::CopyResidual { ops, block } => {
                residual_points = decode_point_count(block, &limits);
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
            }
            Plan::Advance => {
                let tr = Transition::AdvanceTranslations;
                tr.apply(&mut self.st)?;
                emitted_cost += tr_len(&tr);
                emitted.push(tr);
            }
        }

        let prev = self.prev.clone().expect("previous frame");
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

        self.decisions.push(FrameDecision {
            frame: k as u64,
            winner_family: winner.label,
            candidates_evaluated: ev.order,
            candidates_valid,
            winner_payload_bytes: INTERVAL_ENVELOPE + emitted_cost + decl,
            winner_interval_bytes: INTERVAL_ENVELOPE + emitted_cost,
            object_decl_bytes: decl,
            residual_points,
            emitted,
            families: stats,
            materialized_exact: true,
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

    fn consider_exact_refs(&self, target: &Canvas, ev: &mut Eval) {
        for obj in &self.library {
            if obj.w != self.w || obj.h != self.h {
                continue;
            }
            let valid = obj.content_matches(target);
            let mut c = Cand::new(
                "exact_ref",
                Plan::Reset { new_content: false },
                RESET_INTERVAL_COST,
            );
            c.valid = valid;
            if !valid {
                c.invalid_reason = "object content differs from target";
            }
            ev.consider(c);
        }
    }

    fn consider_sparse_and_residual(&self, target: &Canvas, base: &Canvas, ev: &mut Eval) {
        if target.as_slice() == base.as_slice() {
            return; // unchanged covers the degenerate case
        }
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
    }

    fn consider_raw(&mut self, target: &Canvas, ev: &mut Eval) {
        let content = self.content_of_target(target);
        let new = self.is_new_content(&content);
        if !new && !self.opts.raster_only {
            // Reuse of existing content is the exact_ref candidate; the RAW
            // sentinel only needs to exist when it would store new bytes.
            return;
        }
        let payload = RESET_INTERVAL_COST
            + if new {
                decl_bytes(self.w, self.h, &content)
            } else {
                0
            };
        let mut c = Cand::new("raw", Plan::Reset { new_content: true }, payload);
        c.valid = true;
        ev.consider(c);
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

    fn consider_copies(&mut self, target: &Canvas, base: &Canvas, prev: &Canvas, ev: &mut Eval) {
        let limits = Limits::default();
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

        // Prev-frame diff: whole-canvas copy + one-shot residual (or a plain
        // repeat when the frame is unchanged relative to the previous frame).
        let op = rect_op(0, 0, self.w, self.h, 0, 0);
        let mut scratch = base.clone();
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

    fn consider_translation(&self, target: &Canvas, ev: &mut Eval) {
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
        let content = match obj.fill_value() {
            Some(v) => Content::Fill(v),
            None => Content::Raster(obj.samples().expect("raster object").to_vec()),
        };
        let r = self.opts.translation_window;
        let mut any = false;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx == 0 && dy == 0 {
                    continue;
                }
                any = true;
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
            }
        }
        let _ = any;
        // Advance-only: the live instance carries a persistent translation and
        // a single advance reproduces the target.
        let (vx, vy) = self.st.velocity(inst.id);
        if vx != 0 || vy != 0 {
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

/// Number of residual points a block decodes to (0 on any structural issue).
fn decode_point_count(block: &[u8], limits: &Limits) -> u64 {
    if block.len() < 9 {
        return 0;
    }
    let len = u64::from_le_bytes(block[1..9].try_into().expect("8-byte window"));
    if len > limits.max_residual_bytes {
        return 0;
    }
    len / 9
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
