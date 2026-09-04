//! Multi-plane procedural core — Phase V.1.2/V.1.4 (V.1 video programme,
//! contract §2.4, §2.6; V.1.1 receipt "next action").
//!
//! This module generalizes the sealed v1 core (object table, instance
//! painting, background, overlay, checkpoint/interval replay, COPY/RESIDUAL
//! canvas ops) from the Gray8 domain to the canonical **u32 sample domain**
//! of a plane: any [`BitDepth`] 1..=16, any plane geometry. It is written as
//! an **independent implementation** (no shared blit/paint code with the v1
//! materializer) so the V.1.2 specialization court — v1 Gray8 output vs this
//! core at depth 8 — is a meaningful oracle comparison, not a self-check.
//!
//! V.1.2 modeled **independent planes** (§46): a video is an epoch plus one
//! [`PlaneProgram`] per plane; each plane is proceduralized and materialized
//! separately at its own subsampling-correct geometry. V.1.4 (brief §247)
//! ports the remaining sealed v1 families onto this plane domain as additive
//! semantics over the same program model: **palette-index content and palette
//! state** (Phase J), **depth-aware procedural generator content** (Phase N),
//! **persistent translation / trajectory state** (Phase E/I), **Q8 affine
//! placement** (Phase L), and the **transform-coded residual floor** (Phase M,
//! 4×4 lifting DCT) — each with its v1 meaning mirrored exactly in the plane's
//! sample domain (depth-8 Gray identity is courted). The family extension is
//! wire-additive under v2 feature bit 0x1 (`docs/format-v2.md`); historical
//! v1 Gray8 behavior remains an exact specialization.
//!
//! Replay semantics mirror v1 exactly, generalized to the sample domain:
//!
//! * render = background fill, then every instance in paint order
//!   (fill/raster/index/generator overwrite, clipped; palette-index content
//!   resolves through the instance's bound palette; an affine placement scans
//!   the plane through the canonical Q8 source map), then the persistent
//!   overlay (authoritative, above all instances; out-of-canvas points
//!   dropped);
//! * an interval group separates state transitions from canvas ops, applies
//!   state transitions in listed order, materializes the base, then applies
//!   canvas ops in listed order — COPY_RECT reads the plane's *previous*
//!   materialized observation, and the residual ops (sparse and
//!   transform-coded) are self-contained overwrite/additions over the fresh
//!   render, values in the plane's active domain;
//! * motion state (velocity / trajectory / affine / palette binding) dies
//!   with its instance; palettes persist across instance clears, mirroring v1.
//!
//! All limits are the frozen v1 [`Limits`] envelope, applied per plane.

use std::collections::BTreeMap;

use crate::affine::AffineParams;
use crate::error::VoleError;
use crate::limits::Limits;
use crate::media::epoch::VideoEpoch;
use crate::media::gen::Gen;
use crate::media::picture::Picture;
use crate::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use crate::rans;
use crate::trajectory::TrajectorySegment;

/// Object id in one plane program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaneObjectId(pub u32);

/// Instance id in one plane program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaneInstanceId(pub u32);

/// Palette id in one plane program (`NONE` = unbound / reserved).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanePaletteId(pub u32);

impl PlanePaletteId {
    /// The reserved unbound id (never a stored palette).
    pub const NONE: PlanePaletteId = PlanePaletteId(0);
}

/// Immutable object content of one plane (v2 core: fill, raster, index, or
/// generator).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaneContent {
    /// Uniform fill: paints its value over the object's clipped box.
    Fill(u32),
    /// Raster samples (canonical tight storage matching the object depth).
    Raster(PlaneData),
    /// Palette-index samples (V.1.4, Phase-J semantics): tight row-major
    /// **indices** (one byte each) into the palette bound to the painting
    /// instance; the materializer maps every index through the active palette
    /// to produce the plane sample. The same index plane re-renders with
    /// different samples as the palette mutates.
    Index(Vec<u8>),
    /// Depth-aware procedural content program (V.1.4, Phase-N semantics):
    /// samples are computed at materialization by [`Gen`] in the plane's
    /// sample domain, never stored.
    Generator(Gen),
}

/// An immutable plane object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneObject {
    /// Object width in plane samples.
    pub width: u32,
    /// Object height in plane samples.
    pub height: u32,
    /// Content.
    pub content: PlaneContent,
}

impl PlaneObject {
    /// A uniform fill object (validated against the plane depth at program
    /// build / state-apply time).
    pub fn fill(width: u32, height: u32, value: u32) -> Self {
        PlaneObject {
            width,
            height,
            content: PlaneContent::Fill(value),
        }
    }

    /// A palette-index object (V.1.4): `count == w·h` one-byte indices, each
    /// `0..=255` (indices are bounded by the frozen `max_palette_entries`).
    pub fn index(width: u32, height: u32, indices: Vec<u8>) -> Result<Self, VoleError> {
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(VoleError::ArithmeticOverflow)?;
        if indices.len() as u64 != count {
            return Err(VoleError::InvalidSamples);
        }
        Ok(PlaneObject {
            width,
            height,
            content: PlaneContent::Index(indices),
        })
    }

    /// A procedural content object (V.1.4): every sample of the declared box
    /// is computed at materialization by the canonical depth-aware program
    /// (validated against the plane depth by the caller via `check(max)`).
    pub fn procedural(width: u32, height: u32, gen: Gen) -> Result<Self, VoleError> {
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(VoleError::ArithmeticOverflow)?;
        if count == 0 || width == 0 || height == 0 {
            return Err(VoleError::InvalidSamples);
        }
        Ok(PlaneObject {
            width,
            height,
            content: PlaneContent::Generator(gen),
        })
    }

    /// A raster object from canonical u32-domain samples (tight row-major).
    pub fn raster(
        width: u32,
        height: u32,
        depth: BitDepth,
        samples: &[u32],
    ) -> Result<Self, VoleError> {
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(VoleError::ArithmeticOverflow)?;
        if samples.len() as u64 != count {
            return Err(VoleError::InvalidSamples);
        }
        let max = depth.max_sample();
        let data = match depth.storage() {
            PlaneStorage::U8 => {
                if samples.iter().any(|v| *v > max) {
                    return Err(VoleError::InvalidSamples);
                }
                PlaneData::U8(samples.iter().map(|v| *v as u8).collect())
            }
            PlaneStorage::U16 => {
                if samples.iter().any(|v| *v > max) {
                    return Err(VoleError::InvalidSamples);
                }
                PlaneData::U16(samples.iter().map(|v| *v as u16).collect())
            }
        };
        Ok(PlaneObject {
            width,
            height,
            content: PlaneContent::Raster(data),
        })
    }
}

/// A live instance in a plane program (paint order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneInstance {
    /// Instance id.
    pub id: PlaneInstanceId,
    /// Object id (must be declared).
    pub object: PlaneObjectId,
    /// Top-left placement in plane coordinates.
    pub x: i64,
    /// Top-left placement in plane coordinates.
    pub y: i64,
}

/// One per-plane transition or canvas op of an interval group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaneOp {
    /// Declare an immutable object (allowed before intervals and inside
    /// interval groups). Duplicate ids are `DuplicateId`.
    DeclareObject {
        /// Object id.
        id: PlaneObjectId,
        /// Object.
        object: PlaneObject,
    },
    /// Create an instance (`UnknownObject` when the object is not declared).
    CreateInstance {
        /// Instance id.
        id: PlaneInstanceId,
        /// Object id.
        object: PlaneObjectId,
        /// Placement x.
        x: i64,
        /// Placement y.
        y: i64,
    },
    /// Set an instance's absolute position (`UnknownInstance`).
    SetPosition {
        /// Instance id.
        id: PlaneInstanceId,
        /// New x.
        x: i64,
        /// New y.
        y: i64,
    },
    /// Clear every live instance (ids become free for reuse).
    ClearInstances,
    /// Clear every persistent overlay point.
    ClearOverlay,
    /// Add overlay points (strictly ascending by `(x, y)`; out-of-canvas
    /// points are dropped at render, mirroring v1).
    PatchOverlay {
        /// Points.
        points: Vec<(i64, i64, u32)>,
    },
    /// COPY_RECT from the plane's immediately previous materialized
    /// observation (snapshot semantics, dependency depth 1).
    CopyRect {
        /// Source left.
        src_x: i64,
        /// Source top.
        src_y: i64,
        /// Copy width.
        width: u32,
        /// Copy height.
        height: u32,
        /// Destination left.
        dst_x: i64,
        /// Destination top.
        dst_y: i64,
    },
    /// Per-observation sparse residual (self-contained overwrite points).
    Residual {
        /// The residual block (Phase-F byte container; decoded payload is a
        /// strict-sorted list of `(x i32, y i32, v u16)` records, `v` inside
        /// the plane's active depth).
        block: Vec<u8>,
    },
    // --- V.1.4 family extension (feature bit 0x1; semantics mirror v1) ---
    /// Attach a persistent integer translation to an instance (Phase E): the
    /// position gains `(vx, vy)` once per [`PlaneOp::AdvanceTranslations`].
    /// A `(0, 0)` translation deactivates. Translation, trajectory, and
    /// affine state on one instance are mutually exclusive.
    SetVelocity {
        /// Instance id.
        id: PlaneInstanceId,
        /// Per-advance x displacement.
        vx: i64,
        /// Per-advance y displacement.
        vy: i64,
    },
    /// Apply every active persistent translation exactly once.
    AdvanceTranslations,
    /// Attach a bounded parametric trajectory program to an instance (Phase I).
    /// An empty program deactivates; trajectory and translation state are
    /// mutually exclusive.
    SetTrajectory {
        /// Instance id.
        id: PlaneInstanceId,
        /// Finite segment program (canonical forms enforced).
        segments: Vec<TrajectorySegment>,
    },
    /// Apply one advance of every active trajectory program.
    AdvanceTrajectories,
    /// Replace (or declare) the whole palette `id` with sample entries
    /// (Phase J). Mutation is first-class state: a palette-index object
    /// re-renders with the new values from the next materialization.
    SetPalette {
        /// Palette id (`NONE` is reserved).
        id: PlanePaletteId,
        /// Entries (each inside the plane's active depth).
        entries: Vec<u32>,
    },
    /// Patch palette entries: `(index, value)` pairs in canonical strictly
    /// ascending index order; every index must be inside the palette's
    /// current length.
    PatchPalette {
        /// Palette id.
        id: PlanePaletteId,
        /// `(index, sample)` pairs, strictly ascending by index.
        changes: Vec<(u32, u32)>,
    },
    /// Bind an instance to a palette (Phase J): palette-index objects painted
    /// by the instance resolve through that palette's entries. `NONE`
    /// unbinds; binding to an undeclared palette is a typed error.
    BindPalette {
        /// Instance id.
        instance: PlaneInstanceId,
        /// Palette id (`NONE` unbinds).
        palette: PlanePaletteId,
    },
    /// Attach a canonical Q8 fixed-point affine placement to an instance
    /// (Phase L). The identity affine deactivates; affine, translation, and
    /// trajectory state on one instance are mutually exclusive.
    SetAffine {
        /// Instance id.
        id: PlaneInstanceId,
        /// Canonical Q8 parameters.
        params: AffineParams,
    },
    /// Per-observation transform-coded residual (Phase M): the block is the
    /// canonical kind-2 container (mask + zigzag DC/AC streams over coded
    /// aligned 4×4 blocks); the decoder inverse-transforms each coded block
    /// and **adds** the reconstructed samples to the interval's fresh render
    /// (results must stay inside the plane's active depth).
    TransformResidual {
        /// The transform block.
        block: Vec<u8>,
    },
}

/// One record of a plane program's **initial** motion/palette-binding state
/// (the V.1.4 family-extension tail; every kind mirrors the corresponding v1
/// checkpoint state element). Exactly one record per instance id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaneMotion {
    /// Persistent integer translation of the initial instance.
    Velocity {
        /// Instance id.
        instance: PlaneInstanceId,
        /// Per-advance x displacement.
        vx: i64,
        /// Per-advance y displacement.
        vy: i64,
    },
    /// Trajectory program of the initial instance.
    Trajectory {
        /// Instance id.
        instance: PlaneInstanceId,
        /// Segment program.
        segments: Vec<TrajectorySegment>,
    },
    /// Q8 affine placement of the initial instance.
    Affine {
        /// Instance id.
        instance: PlaneInstanceId,
        /// Canonical Q8 parameters.
        params: AffineParams,
    },
    /// Palette binding of the initial instance.
    Binding {
        /// Instance id.
        instance: PlaneInstanceId,
        /// Palette id (`NONE` is never stored).
        palette: PlanePaletteId,
    },
}

impl PlaneMotion {
    /// The instance this record refers to.
    pub fn instance(&self) -> PlaneInstanceId {
        match self {
            PlaneMotion::Velocity { instance, .. }
            | PlaneMotion::Trajectory { instance, .. }
            | PlaneMotion::Affine { instance, .. }
            | PlaneMotion::Binding { instance, .. } => *instance,
        }
    }
}

/// A per-plane procedural program: initial state plus interval groups.
#[derive(Debug, Clone)]
pub struct PlaneProgram {
    /// Initial background sample (in the plane's active domain).
    pub background: u32,
    /// Declared objects (initial).
    pub objects: BTreeMap<PlaneObjectId, PlaneObject>,
    /// Initial instances (paint order).
    pub instances: Vec<PlaneInstance>,
    /// Initial persistent overlay (sorted by `(x, y)`).
    pub overlay: Vec<(i64, i64, u32)>,
    /// Interval groups: `(t, ops)`, `t` strictly increasing from 1.
    pub intervals: Vec<(u64, Vec<PlaneOp>)>,
    /// Initial palette table (V.1.4 family extension; id → sample entries).
    pub palettes: BTreeMap<PlanePaletteId, Vec<u32>>,
    /// Initial per-instance motion / palette-binding records (V.1.4 family
    /// extension; at most one per instance).
    pub initial_motion: Vec<PlaneMotion>,
}

impl PlaneProgram {
    /// An empty program (background `bg`; no objects/instances/overlay/
    /// palettes/motion).
    pub fn new(background: u32) -> Self {
        PlaneProgram {
            background,
            objects: BTreeMap::new(),
            instances: Vec::new(),
            overlay: Vec::new(),
            intervals: Vec::new(),
            palettes: BTreeMap::new(),
            initial_motion: Vec::new(),
        }
    }

    /// Whether the program uses any V.1.4 family-extension surface (new
    /// content kinds, ops, or initial palette/motion state) — decides the
    /// stream's feature bit.
    pub fn uses_family_extension(&self) -> bool {
        let content_ext = |o: &PlaneObject| {
            matches!(
                o.content,
                PlaneContent::Index(_) | PlaneContent::Generator(_)
            )
        };
        if self.objects.values().any(content_ext) {
            return true;
        }
        if !self.palettes.is_empty() || !self.initial_motion.is_empty() {
            return true;
        }
        self.intervals.iter().any(|(_, ops)| {
            ops.iter().any(|op| {
                matches!(
                    op,
                    PlaneOp::SetVelocity { .. }
                        | PlaneOp::AdvanceTranslations
                        | PlaneOp::SetTrajectory { .. }
                        | PlaneOp::AdvanceTrajectories
                        | PlaneOp::SetPalette { .. }
                        | PlaneOp::PatchPalette { .. }
                        | PlaneOp::BindPalette { .. }
                        | PlaneOp::SetAffine { .. }
                        | PlaneOp::TransformResidual { .. }
                ) || matches!(
                    op,
                    PlaneOp::DeclareObject { object, .. } if content_ext(object)
                )
            })
        })
    }
}

/// Runtime state of one plane replay (background, objects, instances,
/// overlay, palettes, motion state).
#[derive(Debug, Clone)]
pub struct PlaneState {
    background: u32,
    objects: BTreeMap<PlaneObjectId, PlaneObject>,
    instances: Vec<PlaneInstance>,
    overlay: Vec<(i64, i64, u32)>,
    palettes: BTreeMap<PlanePaletteId, Vec<u32>>,
    velocities: BTreeMap<PlaneInstanceId, (i64, i64)>,
    trajectories: BTreeMap<PlaneInstanceId, PlaneTrajectoryState>,
    bindings: BTreeMap<PlaneInstanceId, PlanePaletteId>,
    affines: BTreeMap<PlaneInstanceId, AffineParams>,
}

/// Live trajectory state of one instance (mirrors the v1 stepper semantics
/// exactly: position advances by the current velocity, `Accel` segments add
/// their acceleration after each advance, an exhausted segment moves to the
/// next, and a finished program deactivates).
#[derive(Debug, Clone)]
pub struct PlaneTrajectoryState {
    /// The whole bounded program.
    pub program: Vec<TrajectorySegment>,
    /// Current segment index.
    pub seg: usize,
    /// Steps left in the current segment.
    pub remaining: u64,
    /// Current velocity x.
    pub vx: i64,
    /// Current velocity y.
    pub vy: i64,
}

impl PlaneTrajectoryState {
    fn start(program: Vec<TrajectorySegment>) -> Option<Self> {
        let first = *program.first()?;
        let (vx, vy) = match first {
            TrajectorySegment::Linear { vx, vy, .. }
            | TrajectorySegment::Accel {
                vx0: vx, vy0: vy, ..
            } => (vx, vy),
        };
        Some(PlaneTrajectoryState {
            program,
            seg: 0,
            remaining: first.steps(),
            vx,
            vy,
        })
    }
}

/// Validate an object's content against a plane depth.
fn check_object_depth(obj: &PlaneObject, depth: BitDepth) -> Result<(), VoleError> {
    let count = u64::from(obj.width)
        .checked_mul(u64::from(obj.height))
        .ok_or(VoleError::ArithmeticOverflow)?;
    if count == 0 {
        return Err(VoleError::InvalidSamples);
    }
    let max = depth.max_sample();
    match obj.content {
        PlaneContent::Fill(v) => {
            if v > max {
                return Err(VoleError::InvalidSamples);
            }
        }
        PlaneContent::Raster(ref data) => {
            if data.len() as u64 != count {
                return Err(VoleError::InvalidSamples);
            }
            match data {
                PlaneData::U8(v) => {
                    if !depth.is_byte_depth() {
                        return Err(VoleError::InvalidSamples);
                    }
                    if v.iter().any(|s| u32::from(*s) > max) {
                        return Err(VoleError::InvalidSamples);
                    }
                }
                PlaneData::U16(v) => {
                    if depth.is_byte_depth() {
                        return Err(VoleError::InvalidSamples);
                    }
                    if v.iter().any(|s| u32::from(*s) > max) {
                        return Err(VoleError::InvalidSamples);
                    }
                }
            }
        }
        // Palette-index content: one byte per sample (indices are bounded by
        // the frozen max_palette_entries = 256), independent of the plane
        // depth — the stored bytes are indices, not plane samples.
        PlaneContent::Index(ref indices) => {
            if indices.len() as u64 != count {
                return Err(VoleError::InvalidSamples);
            }
        }
        // Depth-aware generator content: canonical parameters in the plane's
        // sample domain.
        PlaneContent::Generator(gen) => gen.check(max)?,
    }
    Ok(())
}

/// Validate a program's initial values against a plane depth.
pub(crate) fn check_program_depth(
    prog: &PlaneProgram,
    depth: BitDepth,
    limits: &Limits,
) -> Result<(), VoleError> {
    if prog.background > depth.max_sample() {
        return Err(VoleError::InvalidSamples);
    }
    if prog.objects.len() as u64 > u64::from(limits.max_objects) {
        return Err(VoleError::DimensionTooLarge);
    }
    if prog.instances.len() as u64 > u64::from(limits.max_instances) {
        return Err(VoleError::DimensionTooLarge);
    }
    if prog.overlay.len() as u64 > limits.max_overlay_points {
        return Err(VoleError::DimensionTooLarge);
    }
    let mut prev_t: Option<u64> = None;
    for (t, _) in &prog.intervals {
        if *t == 0 {
            return Err(VoleError::NonConsecutiveInterval);
        }
        if prev_t.is_some_and(|p| *t <= p) {
            return Err(VoleError::NonConsecutiveInterval);
        }
        prev_t = Some(*t);
    }
    for obj in prog.objects.values() {
        check_object_depth(obj, depth)?;
    }
    for inst in &prog.instances {
        if !prog.objects.contains_key(&inst.object) {
            return Err(VoleError::UnknownObject);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for inst in &prog.instances {
        if !seen.insert(inst.id) {
            return Err(VoleError::DuplicateId);
        }
    }
    let mut prev: Option<(i64, i64)> = None;
    for &(x, y, v) in &prog.overlay {
        if v > depth.max_sample() {
            return Err(VoleError::InvalidSamples);
        }
        let key = (x, y);
        if prev.is_some_and(|q| key <= q) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        prev = Some(key);
    }
    // V.1.4 family extension: initial palette table and motion records.
    check_initial_state(prog, depth, limits)?;
    Ok(())
}

/// Validate the V.1.4 initial palette table and per-instance motion records
/// against the plane depth and the frozen envelope: palette ids nonzero and
/// unique, entry counts `1..=max_palette_entries` and within `max_palettes`,
/// entries inside the active depth; motion records at most one per existing
/// instance, canonical per kind (nonzero velocity; canonical trajectory
/// program with the adjacency rule; non-identity affine with in-domain
/// coefficients; binding to a declared palette).
fn check_initial_state(
    prog: &PlaneProgram,
    depth: BitDepth,
    limits: &Limits,
) -> Result<(), VoleError> {
    let max = depth.max_sample();
    if prog.palettes.len() as u64 > u64::from(limits.max_palettes) {
        return Err(VoleError::DimensionTooLarge);
    }
    for (id, entries) in &prog.palettes {
        if *id == PlanePaletteId::NONE {
            return Err(VoleError::NonCanonicalEncoding);
        }
        if entries.is_empty() || entries.len() as u64 > u64::from(limits.max_palette_entries) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        if entries.iter().any(|v| *v > max) {
            return Err(VoleError::InvalidSamples);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for rec in &prog.initial_motion {
        let iid = rec.instance();
        if !prog.instances.iter().any(|i| i.id == iid) {
            return Err(VoleError::UnknownInstance);
        }
        if !seen.insert(iid) {
            return Err(VoleError::DuplicateId);
        }
        match rec {
            PlaneMotion::Velocity { vx, vy, .. } => {
                if *vx == 0 && *vy == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
            PlaneMotion::Trajectory { segments, .. } => {
                if segments.is_empty() {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                crate::trajectory::check_program(segments, limits)?;
            }
            PlaneMotion::Affine { params, .. } => {
                if params.is_identity() {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                params.check()?;
            }
            PlaneMotion::Binding { palette, .. } => {
                if *palette == PlanePaletteId::NONE {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if !prog.palettes.contains_key(palette) {
                    return Err(VoleError::UnknownPalette);
                }
            }
        }
    }
    Ok(())
}

/// Apply state transitions of one interval group to `state`; canvas ops are
/// cloned back in order for the caller to apply after materialization.
fn apply_state_ops(
    state: &mut PlaneState,
    ops: &[PlaneOp],
    limits: &Limits,
    depth: BitDepth,
) -> Result<Vec<PlaneOp>, VoleError> {
    let mut canvas_ops = Vec::new();
    for op in ops {
        match op {
            PlaneOp::DeclareObject { id, object } => {
                check_object_depth(object, depth)?;
                if state.objects.contains_key(id) {
                    return Err(VoleError::DuplicateId);
                }
                if state.objects.len() as u64 >= u64::from(limits.max_objects) {
                    return Err(VoleError::DimensionTooLarge);
                }
                state.objects.insert(*id, object.clone());
            }
            PlaneOp::CreateInstance { id, object, x, y } => {
                if !state.objects.contains_key(object) {
                    return Err(VoleError::UnknownObject);
                }
                if state.instances.iter().any(|i| i.id == *id) {
                    return Err(VoleError::DuplicateId);
                }
                if state.instances.len() as u64 >= u64::from(limits.max_instances) {
                    return Err(VoleError::DimensionTooLarge);
                }
                state.instances.push(PlaneInstance {
                    id: *id,
                    object: *object,
                    x: *x,
                    y: *y,
                });
            }
            PlaneOp::SetPosition { id, x, y } => {
                let inst = state
                    .instances
                    .iter_mut()
                    .find(|i| i.id == *id)
                    .ok_or(VoleError::UnknownInstance)?;
                inst.x = *x;
                inst.y = *y;
            }
            PlaneOp::ClearInstances => {
                state.instances.clear();
                state.velocities.clear();
                state.trajectories.clear();
                state.bindings.clear();
                state.affines.clear();
            }
            PlaneOp::ClearOverlay => state.overlay.clear(),
            PlaneOp::SetVelocity { id, vx, vy } => {
                if !state.instances.iter().any(|i| i.id == *id) {
                    return Err(VoleError::UnknownInstance);
                }
                state.trajectories.remove(id);
                state.affines.remove(id);
                if *vx == 0 && *vy == 0 {
                    state.velocities.remove(id);
                } else {
                    state.velocities.insert(*id, (*vx, *vy));
                }
            }
            PlaneOp::AdvanceTranslations => {
                for inst in state.instances.iter_mut() {
                    let Some((vx, vy)) = state.velocities.get(&inst.id).copied() else {
                        continue;
                    };
                    if vx == 0 && vy == 0 {
                        continue;
                    }
                    inst.x = inst
                        .x
                        .checked_add(vx)
                        .ok_or(VoleError::ArithmeticOverflow)?;
                    inst.y = inst
                        .y
                        .checked_add(vy)
                        .ok_or(VoleError::ArithmeticOverflow)?;
                }
            }
            PlaneOp::SetTrajectory { id, segments } => {
                if !state.instances.iter().any(|i| i.id == *id) {
                    return Err(VoleError::UnknownInstance);
                }
                if segments.is_empty() {
                    state.trajectories.remove(id);
                } else {
                    for seg in segments {
                        seg.check()?;
                    }
                    let traj = PlaneTrajectoryState::start(segments.clone())
                        .ok_or(VoleError::NonCanonicalEncoding)?;
                    state.velocities.remove(id);
                    state.affines.remove(id);
                    state.trajectories.insert(*id, traj);
                }
            }
            PlaneOp::AdvanceTrajectories => advance_trajectories(state)?,
            PlaneOp::SetPalette { id, entries } => {
                if *id == PlanePaletteId::NONE {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if entries.is_empty()
                    || entries.len() as u64 > u64::from(limits.max_palette_entries)
                {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if entries.iter().any(|v| *v > depth.max_sample()) {
                    return Err(VoleError::InvalidSamples);
                }
                if !state.palettes.contains_key(id)
                    && state.palettes.len() as u64 >= u64::from(limits.max_palettes)
                {
                    return Err(VoleError::DimensionTooLarge);
                }
                state.palettes.insert(*id, entries.clone());
            }
            PlaneOp::PatchPalette { id, changes } => {
                let entries = state
                    .palettes
                    .get_mut(id)
                    .ok_or(VoleError::UnknownPalette)?;
                let mut prev_idx: Option<u32> = None;
                for (idx, v) in changes {
                    if *v > depth.max_sample() {
                        return Err(VoleError::InvalidSamples);
                    }
                    if prev_idx.is_some_and(|p| *idx <= p) {
                        return Err(VoleError::NonCanonicalEncoding);
                    }
                    prev_idx = Some(*idx);
                    let slot = entries
                        .get_mut(*idx as usize)
                        .ok_or(VoleError::OutOfBounds)?;
                    *slot = *v;
                }
            }
            PlaneOp::BindPalette { instance, palette } => {
                if !state.instances.iter().any(|i| i.id == *instance) {
                    return Err(VoleError::UnknownInstance);
                }
                if *palette == PlanePaletteId::NONE {
                    state.bindings.remove(instance);
                } else {
                    if !state.palettes.contains_key(palette) {
                        return Err(VoleError::UnknownPalette);
                    }
                    state.bindings.insert(*instance, *palette);
                }
            }
            PlaneOp::SetAffine { id, params } => {
                if !state.instances.iter().any(|i| i.id == *id) {
                    return Err(VoleError::UnknownInstance);
                }
                params.check()?;
                state.velocities.remove(id);
                state.trajectories.remove(id);
                if params.is_identity() {
                    state.affines.remove(id);
                } else {
                    state.affines.insert(*id, *params);
                }
            }
            PlaneOp::PatchOverlay { points } => {
                let mut prev: Option<(i64, i64)> = None;
                for &(x, y, v) in points {
                    if v > depth.max_sample() {
                        return Err(VoleError::InvalidSamples);
                    }
                    let key = (x, y);
                    if prev.is_some_and(|q| key <= q) {
                        return Err(VoleError::NonCanonicalEncoding);
                    }
                    prev = Some(key);
                }
                for &(x, y, v) in points {
                    if !state.overlay.iter().any(|&(ox, oy, _)| ox == x && oy == y) {
                        state.overlay.push((x, y, v));
                    }
                }
                if state.overlay.len() as u64 > limits.max_overlay_points {
                    return Err(VoleError::DimensionTooLarge);
                }
                state.overlay.sort_unstable_by_key(|&(x, y, _)| (x, y));
            }
            op => canvas_ops.push(op.clone()),
        }
    }
    Ok(canvas_ops)
}

/// Render the current state of one plane at `(width, height, depth)`.
/// Mirrors the v1 painter: background, instances in paint order (clipped
/// overwrite; fill/raster/index/generator content, palette-index resolution,
/// Q8 affine placement), then the persistent overlay (authoritative), all in
/// the plane's sample domain.
pub fn render_plane(
    state: &PlaneState,
    width: u32,
    height: u32,
    depth: BitDepth,
    limits: &Limits,
) -> Result<Plane, VoleError> {
    limits.check_canvas(width, height)?;
    let n = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(VoleError::ArithmeticOverflow)?;
    if state.background > depth.max_sample() {
        return Err(VoleError::InvalidSamples);
    }
    let data = match depth.storage() {
        PlaneStorage::U8 => PlaneData::U8(vec![state.background as u8; n as usize]),
        PlaneStorage::U16 => PlaneData::U16(vec![state.background as u16; n as usize]),
    };
    let mut pic = single_plane_picture(width, height, depth, data)?;
    let mut acc = 0u64;
    let mut affine_work = 0u64;
    let affine_sample_cost = u64::from(width) * u64::from(height);
    for inst in &state.instances {
        acc = acc.saturating_add(1);
        if acc > u64::from(limits.max_instances) {
            return Err(VoleError::MaterializationBudgetExceeded);
        }
        if state.affines.contains_key(&inst.id) {
            affine_work = affine_work.saturating_add(affine_sample_cost);
            if affine_work > limits.max_affine_work {
                return Err(VoleError::MaterializationBudgetExceeded);
            }
        }
        let Some(obj) = state.objects.get(&inst.object) else {
            continue; // mirrors v1's total skip for a missing object
        };
        paint_object_plane(&mut pic, state, obj, inst, depth, limits)?;
    }
    // Persistent overlay above every instance.
    for &(x, y, v) in &state.overlay {
        if x < 0 || y < 0 {
            continue;
        }
        let (x, y) = (x as u64, y as u64);
        if x < u64::from(width) && y < u64::from(height) {
            pic.put(0, x as u32, y as u32, v)?;
        }
    }
    Ok(pic.into_planes().pop().expect("single plane"))
}

/// Paint one object through one instance placement (v1 `paint_object` in the
/// plane's sample domain): affine placements scan the plane through the
/// canonical Q8 source map; plain placements blit/fill/compute the object's
/// box clipped to the plane.
fn paint_object_plane(
    pic: &mut Picture,
    state: &PlaneState,
    obj: &PlaneObject,
    inst: &PlaneInstance,
    depth: BitDepth,
    limits: &Limits,
) -> Result<(), VoleError> {
    if let Some(params) = state.affines.get(&inst.id) {
        return paint_affine_plane(pic, state, obj, inst, *params, depth, limits);
    }
    let dx = inst.x;
    let dy = inst.y;
    let w = obj.width;
    let h = obj.height;
    let max = depth.max_sample();
    match &obj.content {
        PlaneContent::Fill(v) => {
            pic.fill_rect_clipped(0, *v, dx, dy, dx + i64::from(w), dy + i64::from(h))?;
        }
        PlaneContent::Raster(data) => {
            let samples: Vec<u32> = match data {
                PlaneData::U8(v) => v.iter().map(|s| u32::from(*s)).collect(),
                PlaneData::U16(v) => v.iter().map(|s| u32::from(*s)).collect(),
            };
            pic.blit(0, &samples, w, h, dx, dy)?;
        }
        PlaneContent::Index(indices) => {
            let entries = palette_of(state, inst)?;
            paint_index_plane(pic, indices, w, h, dx, dy, entries, max)?;
        }
        PlaneContent::Generator(gen) => {
            paint_gen_plane(pic, *gen, w, h, dx, dy, max)?;
        }
    }
    Ok(())
}

/// Resolve the palette entries an index-content instance paints through:
/// the instance's binding and the bound palette must exist (typed otherwise).
fn palette_of<'a>(state: &'a PlaneState, inst: &PlaneInstance) -> Result<&'a [u32], VoleError> {
    let palette = state
        .bindings
        .get(&inst.id)
        .ok_or(VoleError::UnknownPalette)?;
    state
        .palettes
        .get(palette)
        .map(|e| e.as_slice())
        .ok_or(VoleError::UnknownPalette)
}

/// Palette-index blit in the sample domain (v1 `paint_index_raster`):
/// overwrite the box's clipped rectangle with `entries[idx]` for every stored
/// index; every index is validated against the palette length **before** any
/// pixel is written (a hostile index raster fails the whole frame typed,
/// never partially).
#[allow(clippy::too_many_arguments)] // ordered geometry + palette context
fn paint_index_plane(
    pic: &mut Picture,
    indices: &[u8],
    w: u32,
    h: u32,
    dx: i64,
    dy: i64,
    entries: &[u32],
    max: u32,
) -> Result<(), VoleError> {
    debug_assert_eq!(indices.len() as u64, u64::from(w) * u64::from(h));
    for idx in indices {
        if usize::from(*idx) >= entries.len() {
            return Err(VoleError::OutOfBounds);
        }
    }
    let (cw, ch) = plane_extent(pic);
    let y0 = dy.max(0);
    let y1 = (dy + i64::from(h)).min(ch);
    let x0 = dx.max(0);
    let x1 = (dx + i64::from(w)).min(cw);
    if y0 >= y1 || x0 >= x1 {
        return Ok(());
    }
    for cty in y0..y1 {
        let sy = (cty - dy) as usize;
        for ctox in x0..x1 {
            let sx = (ctox - dx) as usize;
            let idx = indices[sy * (w as usize) + sx];
            let value = entries[usize::from(idx)];
            if value > max {
                return Err(VoleError::InvalidSamples);
            }
            pic.put(0, ctox as u32, cty as u32, value)?;
        }
    }
    Ok(())
}

/// Procedural generator blit in the sample domain (v1 `paint_generator`):
/// compute every sample of the painted box from the depth-aware integer
/// program, clipped exactly like a fill blit; work is one sample per painted
/// pixel.
fn paint_gen_plane(
    pic: &mut Picture,
    gen: Gen,
    w: u32,
    h: u32,
    dx: i64,
    dy: i64,
    max: u32,
) -> Result<(), VoleError> {
    let (cw, ch) = plane_extent(pic);
    let y0 = dy.max(0);
    let y1 = (dy + i64::from(h)).min(ch);
    let x0 = dx.max(0);
    let x1 = (dx + i64::from(w)).min(cw);
    if y0 >= y1 || x0 >= x1 {
        return Ok(());
    }
    for cty in y0..y1 {
        let ly = cty - dy;
        for ctox in x0..x1 {
            let lx = ctox - dx;
            let v = gen.sample(lx, ly, max);
            pic.put(0, ctox as u32, cty as u32, v)?;
        }
    }
    Ok(())
}

/// Affine placement painter (v1 `paint_affine` in the sample domain): scan
/// every destination sample of the plane, compute the canonical Q8 source
/// sample, and paint the object sample when the source lies inside the object
/// rectangle. Deterministic and integer throughout; the caller has already
/// accounted the per-materialization affine work against
/// [`Limits::max_affine_work`].
fn paint_affine_plane(
    pic: &mut Picture,
    state: &PlaneState,
    obj: &PlaneObject,
    inst: &PlaneInstance,
    params: AffineParams,
    depth: BitDepth,
    limits: &Limits,
) -> Result<(), VoleError> {
    let _ = limits;
    let ow = i64::from(obj.width);
    let oh = i64::from(obj.height);
    let max = depth.max_sample();
    enum Kind<'a> {
        Raster(&'a PlaneData),
        Fill(u32),
        Index(&'a [u8], &'a [u32]), // indices + palette entries
        Generator(Gen),
    }
    let kind: Kind<'_> = match &obj.content {
        PlaneContent::Generator(gen) => Kind::Generator(*gen),
        PlaneContent::Index(indices) => Kind::Index(indices, palette_of(state, inst)?),
        PlaneContent::Raster(data) => Kind::Raster(data),
        PlaneContent::Fill(v) => Kind::Fill(*v),
    };
    let (cw, ch) = plane_extent(pic);
    for y in 0..ch {
        for x in 0..cw {
            let (su, sv) = params.source(x, y).ok_or(VoleError::ArithmeticOverflow)?;
            if su < 0 || sv < 0 || su >= ow || sv >= oh {
                continue;
            }
            let k = (sv * ow + su) as usize;
            let v = match kind {
                Kind::Raster(data) => match data {
                    PlaneData::U8(v) => u32::from(v[k]),
                    PlaneData::U16(v) => u32::from(v[k]),
                },
                Kind::Fill(value) => value,
                Kind::Generator(gen) => gen.sample(su, sv, max),
                Kind::Index(indices, entries) => {
                    let idx = indices[k];
                    if usize::from(idx) >= entries.len() {
                        return Err(VoleError::OutOfBounds);
                    }
                    entries[usize::from(idx)]
                }
            };
            if v > max {
                return Err(VoleError::InvalidSamples);
            }
            pic.put(0, x as u32, y as u32, v)?;
        }
    }
    Ok(())
}

/// The plane's `(width, height)`.
fn plane_extent(pic: &Picture) -> (i64, i64) {
    let p = pic.plane(0).expect("single plane");
    (i64::from(p.width()), i64::from(p.height()))
}

/// Build a one-plane picture over the given payload (helper).
fn single_plane_picture(
    width: u32,
    height: u32,
    depth: BitDepth,
    data: PlaneData,
) -> Result<Picture, VoleError> {
    let plane = Plane::new(
        crate::media::layout::Component::Gray,
        width,
        height,
        depth,
        0,
        0,
        data,
    )?;
    Picture::from_planes(&single_plane_epoch(width, height, depth)?, vec![plane])
}

/// A throwaway single-Gray-plane epoch matching a plane's geometry (helper
/// for standalone plane materialization).
fn single_plane_epoch(width: u32, height: u32, depth: BitDepth) -> Result<VideoEpoch, VoleError> {
    use crate::media::color::ColorDescription;
    use crate::media::epoch::EpochId;
    use crate::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
    VideoEpoch::new_uniform(
        EpochId(0),
        width,
        height,
        crate::media::layout::PixelLayout::Gray,
        depth,
        ColorDescription::unspecified(),
        SampleAspectRatio::square(),
        Orientation::Normal,
        FieldStructure::Progressive,
    )
}

/// Apply one residual block to a one-plane target: decode the Phase-F byte
/// container, parse the strict-sorted `(x i32, y i32, v u16)` point list,
/// validate bounds and the active depth, and overwrite each point.
fn apply_plane_residual(
    pic: &mut Picture,
    block: &[u8],
    limits: &Limits,
    depth: BitDepth,
) -> Result<(), VoleError> {
    let payload = rans::decode_block(block, limits.max_residual_bytes)?;
    if !payload.len().is_multiple_of(10) {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let max = depth.max_sample();
    let pw = pic.plane(0).expect("target plane").width();
    let ph = pic.plane(0).expect("target plane").height();
    let mut prev: Option<(i64, i64)> = None;
    for rec in payload.as_chunks::<10>().0 {
        let x = i64::from(i32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]));
        let y = i64::from(i32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]));
        let v = u32::from(u16::from_le_bytes([rec[8], rec[9]]));
        if x < 0 || y < 0 || x >= i64::from(pw) || y >= i64::from(ph) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let key = (x, y);
        if prev.is_some_and(|q| key <= q) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        prev = Some(key);
        if v > max {
            return Err(VoleError::InvalidSamples);
        }
        pic.put(0, x as u32, y as u32, v)?;
    }
    Ok(())
}

/// Apply one **transform-coded** residual block (Phase M, kind 2) to a
/// one-plane target: every coded aligned `4×4` block is inverse-transformed
/// (the normative integer lifting DCT, sealed in `crate::transform`) and its
/// reconstructed samples are **added** to the plane; a result outside the
/// plane's active depth is typed (`OutOfBounds`). Structure, mask padding,
/// coefficient counts, and container framing mirror the v1 checks exactly.
pub fn apply_plane_transform_block(
    pic: &mut Picture,
    block: &[u8],
    limits: &Limits,
    depth: BitDepth,
) -> Result<(), VoleError> {
    if block.len() < 2 {
        return Err(VoleError::Truncated);
    }
    if block[0] != rans::KIND_TSF || block[1] != crate::transform::TRANSFORM_ID_4X4 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let (w, h) = {
        let p = pic.plane(0).expect("target plane");
        (p.width(), p.height())
    };
    let (bx, by) = crate::transform::blocks_per_axis(w, h);
    let nblocks = bx.checked_mul(by).ok_or(VoleError::ArithmeticOverflow)?;
    let mlen = crate::transform::mask_len(w, h);
    let o = 2usize
        .checked_add(mlen)
        .ok_or(VoleError::ArithmeticOverflow)?;
    if block.len() < o + 8 {
        return Err(VoleError::Truncated);
    }
    let mask = &block[2..o];
    let used = nblocks % 8;
    if used != 0 && mask[mlen - 1] & !((1u8 << used) - 1) != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
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
    if dc_len > limits.max_residual_bytes || ac_len > limits.max_residual_bytes {
        return Err(VoleError::DimensionTooLarge);
    }
    let total = o as u64 + 8 + dc_len + ac_len;
    if total != block.len() as u64 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let dc_off = o + 8;
    let ac_off = dc_off
        .checked_add(dc_len as usize)
        .ok_or(VoleError::ArithmeticOverflow)?;
    let dc_payload = rans::decode_block(&block[dc_off..ac_off], limits.max_residual_bytes)?;
    let ac_payload = rans::decode_block(&block[ac_off..], limits.max_residual_bytes)?;
    if dc_payload.len() % 4 != 0 || ac_payload.len() % 60 != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let coded: usize = mask.iter().map(|b| b.count_ones() as usize).sum();
    if dc_payload.len() / 4 != coded || ac_payload.len() / 60 != coded {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let dc4 = dc_payload.as_chunks::<4>().0;
    let ac60 = ac_payload.as_chunks::<60>().0;
    let mut block_i = 0usize;
    let max = depth.max_sample();
    let cw = i64::from(w);
    let ch = i64::from(h);
    for k in 0..nblocks {
        if mask[k >> 3] & (1 << (k & 7)) == 0 {
            continue;
        }
        let mut coeffs = [0i32; 16];
        let dcb = dc4[block_i];
        coeffs[0] =
            crate::transform::unzigzag(u32::from_le_bytes([dcb[0], dcb[1], dcb[2], dcb[3]]));
        let acb = ac60[block_i];
        for j in 0..15 {
            let z =
                u32::from_le_bytes([acb[4 * j], acb[4 * j + 1], acb[4 * j + 2], acb[4 * j + 3]]);
            coeffs[j + 1] = crate::transform::unzigzag(z);
        }
        block_i += 1;
        let samples = crate::transform::inverse_block(&coeffs);
        let (kxx, kyy) = (k % bx, k / bx);
        for vy in 0..4i64 {
            let gy = i64::try_from(kyy).unwrap_or(i64::MAX) * 4 + vy;
            if gy < 0 || gy >= ch {
                continue;
            }
            for vx in 0..4i64 {
                let gx = i64::try_from(kxx).unwrap_or(i64::MAX) * 4 + vx;
                if gx < 0 || gx >= cw {
                    continue;
                }
                let r = samples[(vy * 4 + vx) as usize];
                let cur = i64::from(pic.get(0, gx as u32, gy as u32).expect("in bounds"));
                let nv = cur + r;
                if !(0..=i64::from(max)).contains(&nv) {
                    return Err(VoleError::OutOfBounds);
                }
                pic.put(0, gx as u32, gy as u32, nv as u32)?;
            }
        }
    }
    Ok(())
}

/// Build a kind-2 transform residual block (Phase M) that closes the exact
/// sample difference `target − base` of two equal-geometry planes: aligned
/// 4×4 blocks, one mask bit per block, and DC/AC zigzag coefficient streams
/// (each a self-describing RAW/rANS container). `None` when the planes are
/// identical (no residual). The lifted integer codec is the sealed v1
/// transform; sample deltas of any plane depth stay inside the codec's i64
/// arithmetic.
pub fn encode_plane_transform_block(base: &Plane, target: &Plane) -> Option<Vec<u8>> {
    let (w, h) = (base.width(), base.height());
    if target.width() != w || target.height() != h {
        return None;
    }
    if base.canonical_bytes() == target.canonical_bytes() {
        return None;
    }
    let cw = w as usize;
    let cn = cw * h as usize;
    let bd = base.data();
    let td = target.data();
    let get = |d: &PlaneData, k: usize| -> u32 {
        match d {
            PlaneData::U8(v) => u32::from(v[k]),
            PlaneData::U16(v) => u32::from(v[k]),
        }
    };
    let (bx, by) = crate::transform::blocks_per_axis(w, h);
    let nblocks = bx.checked_mul(by)?;
    let mlen = crate::transform::mask_len(w, h);
    let mut grid = vec![0i64; cn];
    let mut seen = vec![false; nblocks];
    let mut coded = 0usize;
    for y in 0..h as usize {
        for x in 0..cw {
            let k = y * cw + x;
            let d = i64::from(get(td, k)) - i64::from(get(bd, k));
            if d != 0 {
                grid[k] = d;
                let kk = (y / crate::transform::BLOCK) * bx + (x / crate::transform::BLOCK);
                if !seen[kk] {
                    seen[kk] = true;
                    coded += 1;
                }
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

/// Encode a residual block for a list of strict-sorted points.
/// Uses the sealed Phase-F byte container (RAW or rANS at the byte level).
pub fn encode_plane_residual(points: &[(i32, i32, u16)]) -> Result<Vec<u8>, VoleError> {
    let mut prev: Option<(i64, i64)> = None;
    let mut body = Vec::with_capacity(points.len() * 10);
    for &(x, y, v) in points {
        let key = (i64::from(x), i64::from(y));
        if prev.is_some_and(|q| key <= q) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        prev = Some(key);
        body.extend_from_slice(&x.to_le_bytes());
        body.extend_from_slice(&y.to_le_bytes());
        body.extend_from_slice(&v.to_le_bytes());
    }
    Ok(rans::encode_block(&body))
}

/// Materialize observation `idx` of a single-plane program: replay the
/// initial state and every interval `1..=idx` (state transitions then canvas
/// ops per group, COPY reading the plane's previous observation).
pub fn materialize_plane(
    prog: &PlaneProgram,
    width: u32,
    height: u32,
    depth: BitDepth,
    idx: u64,
    limits: &Limits,
) -> Result<Plane, VoleError> {
    let mut state = PlaneState {
        background: prog.background,
        objects: prog.objects.clone(),
        instances: prog.instances.clone(),
        overlay: prog.overlay.clone(),
        palettes: prog.palettes.clone(),
        velocities: BTreeMap::new(),
        trajectories: BTreeMap::new(),
        bindings: BTreeMap::new(),
        affines: BTreeMap::new(),
    };
    // V.1.4 family extension: seed the initial motion / binding records.
    for rec in &prog.initial_motion {
        match rec {
            PlaneMotion::Velocity { instance, vx, vy } => {
                state.velocities.insert(*instance, (*vx, *vy));
            }
            PlaneMotion::Trajectory { instance, segments } => {
                state.trajectories.insert(
                    *instance,
                    PlaneTrajectoryState::start(segments.clone())
                        .ok_or(VoleError::NonCanonicalEncoding)?,
                );
            }
            PlaneMotion::Affine { instance, params } => {
                state.affines.insert(*instance, *params);
            }
            PlaneMotion::Binding { instance, palette } => {
                state.bindings.insert(*instance, *palette);
            }
        }
    }
    // Initial observation.
    let mut prev = render_plane(&state, width, height, depth, limits)?;
    for (t, ops) in &prog.intervals {
        if *t > idx {
            break;
        }
        let canvas_ops = apply_state_ops(&mut state, ops, limits, depth)?;
        let mut base = render_plane(&state, width, height, depth, limits)?;
        for op in canvas_ops {
            match op {
                PlaneOp::CopyRect {
                    src_x,
                    src_y,
                    width: cw,
                    height: ch,
                    dst_x,
                    dst_y,
                } => {
                    let src = prev.data().clone();
                    let dst = base.data().clone();
                    let src_pic = single_plane_picture(width, height, depth, src)?;
                    let mut dst_pic = single_plane_picture(width, height, depth, dst)?;
                    copy_rect_u32(&mut dst_pic, &src_pic, src_x, src_y, cw, ch, dst_x, dst_y)?;
                    base = dst_pic.into_planes().pop().expect("single plane");
                }
                PlaneOp::Residual { block } => {
                    let dst = base.data().clone();
                    let mut pic = single_plane_picture(width, height, depth, dst)?;
                    apply_plane_residual(&mut pic, &block, limits, depth)?;
                    base = pic.into_planes().pop().expect("single plane");
                }
                PlaneOp::TransformResidual { block } => {
                    let dst = base.data().clone();
                    let mut pic = single_plane_picture(width, height, depth, dst)?;
                    apply_plane_transform_block(&mut pic, &block, limits, depth)?;
                    base = pic.into_planes().pop().expect("single plane");
                }
                _ => return Err(VoleError::NonCanonicalEncoding),
            }
        }
        prev = base;
    }
    Ok(prev)
}

/// Step every active trajectory exactly once (v1 `State::advance_trajectories`
/// semantics): for each trajectory-carrying instance, in instance list order,
/// advance the position by the current velocity (checked), then update the
/// segment state — an `Accel` segment adds `(ax, ay)` to its velocity; a
/// segment whose steps are exhausted moves to the next segment, and a program
/// whose final segment is exhausted deactivates. One O(instances) pass.
fn advance_trajectories(state: &mut PlaneState) -> Result<(), VoleError> {
    let mut finished: Vec<PlaneInstanceId> = Vec::new();
    for inst in state.instances.iter_mut() {
        let Some(traj) = state.trajectories.get_mut(&inst.id) else {
            continue;
        };
        inst.x = inst
            .x
            .checked_add(traj.vx)
            .ok_or(VoleError::ArithmeticOverflow)?;
        inst.y = inst
            .y
            .checked_add(traj.vy)
            .ok_or(VoleError::ArithmeticOverflow)?;
        match traj.program[traj.seg] {
            TrajectorySegment::Linear { .. } => {}
            TrajectorySegment::Accel { ax, ay, .. } => {
                traj.vx = traj
                    .vx
                    .checked_add(ax)
                    .ok_or(VoleError::ArithmeticOverflow)?;
                traj.vy = traj
                    .vy
                    .checked_add(ay)
                    .ok_or(VoleError::ArithmeticOverflow)?;
            }
        }
        traj.remaining -= 1;
        if traj.remaining == 0 {
            traj.seg += 1;
            match traj.program.get(traj.seg) {
                None => finished.push(inst.id),
                Some(next) => {
                    traj.remaining = next.steps();
                    match next {
                        TrajectorySegment::Linear { vx, vy, .. }
                        | TrajectorySegment::Accel {
                            vx0: vx, vy0: vy, ..
                        } => {
                            traj.vx = *vx;
                            traj.vy = *vy;
                        }
                    }
                }
            }
        }
    }
    for id in finished {
        state.trajectories.remove(&id);
    }
    Ok(())
}

/// Copy a rectangle from `src` into `dst` in the u32 sample domain with the
/// canonical clip rule (a sample is written only when both its source and
/// destination positions are inside their pictures).
#[allow(clippy::too_many_arguments)] // 8 ordered geometry ints, like v1's rect_copy
fn copy_rect_u32(
    dst: &mut Picture,
    src: &Picture,
    sx: i64,
    sy: i64,
    width: u32,
    height: u32,
    dx: i64,
    dy: i64,
) -> Result<(), VoleError> {
    let dw = i64::from(dst.plane(0).expect("dst").width());
    let dh = i64::from(dst.plane(0).expect("dst").height());
    let sw = i64::from(src.plane(0).expect("src").width());
    let sh = i64::from(src.plane(0).expect("src").height());
    for si in 0..height as i64 {
        for sj in 0..width as i64 {
            let px = sx + sj;
            let py = sy + si;
            if px < 0 || py < 0 || px >= sw || py >= sh {
                continue;
            }
            let qx = dx + sj;
            let qy = dy + si;
            if qx < 0 || qy < 0 || qx >= dw || qy >= dh {
                continue;
            }
            let v = src.get(0, px as u32, py as u32).expect("in bounds");
            dst.put(0, qx as u32, qy as u32, v)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Multi-plane video program (one epoch; independent per-plane programs)
// ---------------------------------------------------------------------------

/// A multi-plane video program: an epoch plus one independent plane program
/// per epoch plane. Observations are aligned intervals across planes: each
/// plane steps its own interval group per observation.
#[derive(Debug, Clone)]
pub struct MultiPlaneProgram {
    /// The epoch declaring the plane table and interpretation.
    pub epoch: VideoEpoch,
    /// One program per epoch plane (count must equal `epoch.plane_count()`).
    pub planes: Vec<PlaneProgram>,
}

impl MultiPlaneProgram {
    /// Validate: plane count matches the epoch and every plane program's
    /// values fit the plane's depth/geometry envelope.
    pub fn new(epoch: VideoEpoch, planes: Vec<PlaneProgram>) -> Result<Self, VoleError> {
        let limits = Limits::default();
        if planes.len() != epoch.plane_count() {
            return Err(VoleError::GeometryMismatch);
        }
        for (i, prog) in planes.iter().enumerate() {
            let depth = epoch.planes()[i].bit_depth;
            check_program_depth(prog, depth, &limits)?;
        }
        Ok(MultiPlaneProgram { epoch, planes })
    }

    /// Materialize observation `idx` as a [`Picture`] matching the epoch:
    /// frame 0 from the per-plane initial states, then one aligned interval
    /// step per plane per observation.
    pub fn materialize_observation(&self, idx: u64) -> Result<Picture, VoleError> {
        let limits = Limits::default();
        let mut planes = Vec::with_capacity(self.planes.len());
        for (i, prog) in self.planes.iter().enumerate() {
            let (pw, ph) = self.epoch.plane_dimensions(i)?;
            let tmpl = &self.epoch.planes()[i];
            let plane = materialize_plane(prog, pw, ph, tmpl.bit_depth, idx, &limits)?;
            let (_, _, _, data) = plane.into_parts();
            planes.push(Plane::new(
                tmpl.component,
                pw,
                ph,
                tmpl.bit_depth,
                tmpl.subsample_x,
                tmpl.subsample_y,
                data,
            )?);
        }
        Picture::from_planes(&self.epoch, planes)
    }

    /// The number of materializable observations: the max over planes of the
    /// last interval index + 1 (each program starts at observation 0), at
    /// least 1.
    pub fn observation_count(&self) -> u64 {
        let last = self
            .planes
            .iter()
            .map(|p| p.intervals.last().map(|(t, _)| *t).unwrap_or(0))
            .max()
            .unwrap_or(0);
        last + 1
    }
}

/// Convenience: materialize every observation as pictures.
pub fn materialize_all_observations(prog: &MultiPlaneProgram) -> Result<Vec<Picture>, VoleError> {
    let mut out = Vec::with_capacity(prog.observation_count() as usize);
    for idx in 0..prog.observation_count() {
        out.push(prog.materialize_observation(idx)?);
    }
    Ok(out)
}
