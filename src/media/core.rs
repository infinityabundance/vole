//! Multi-plane procedural core — Phase V.1.2 (V.1 video programme, contract
//! §2.4, §2.6; V.1.1 receipt "next action").
//!
//! This module generalizes the sealed v1 core (object table, instance
//! painting, background, overlay, checkpoint/interval replay, COPY/RESIDUAL
//! canvas ops) from the Gray8 domain to the canonical **u32 sample domain**
//! of a plane: any [`BitDepth`] 1..=16, any plane geometry. It is written as
//! an **independent implementation** (no shared blit/paint code with the v1
//! materializer) so the V.1.2 specialization court — v1 Gray8 output vs this
//! core at depth 8 — is a meaningful oracle comparison, not a self-check.
//!
//! V.1.2 models **independent planes** (§46): a video is an epoch plus one
//! [`PlaneProgram`] per plane; each plane is proceduralized and materialized
//! separately at its own subsampling-correct geometry. Cross-plane shared
//! hypotheses and the advanced family ports (trajectory, affine, palette,
//! generator, transform residual, region partitions at plane level) are
//! later-subphase work (V.1.4+ per the brief's §247 ordering).
//!
//! Replay semantics mirror v1 exactly, generalized to the sample domain:
//!
//! * render = background fill, then every instance in paint order
//!   (fill/raster overwrite, clipped), then the persistent overlay
//!   (authoritative, above all instances; out-of-canvas points dropped);
//! * an interval group separates state transitions from canvas ops, applies
//!   state transitions in listed order, materializes the base, then applies
//!   canvas ops in listed order — COPY_RECT reads the plane's *previous*
//!   materialized observation, and the residual op is self-contained
//!   (sparse overwrite points, RAW or rANS at the byte level via the sealed
//!   Phase-F coder, values in the plane's active domain);
//! * object declarations may appear in interval groups (v2 design decision):
//!   immutable content, ids never reused, bounded by the active limits.
//!
//! All limits are the frozen v1 [`Limits`] envelope, applied per plane.

use std::collections::BTreeMap;

use crate::error::VoleError;
use crate::limits::Limits;
use crate::media::epoch::VideoEpoch;
use crate::media::picture::Picture;
use crate::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use crate::rans;

/// Object id in one plane program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaneObjectId(pub u32);

/// Instance id in one plane program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaneInstanceId(pub u32);

/// Immutable object content of one plane (v2 core: fill or raster).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaneContent {
    /// Uniform fill: paints its value over the object's clipped box.
    Fill(u32),
    /// Raster samples (canonical tight storage matching the object depth).
    Raster(PlaneData),
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
}

impl PlaneProgram {
    /// An empty program (background `bg`; no objects/instances/overlay).
    pub fn new(background: u32) -> Self {
        PlaneProgram {
            background,
            objects: BTreeMap::new(),
            instances: Vec::new(),
            overlay: Vec::new(),
            intervals: Vec::new(),
        }
    }
}

/// Runtime state of one plane replay (background, objects, instances,
/// overlay).
#[derive(Debug, Clone)]
pub struct PlaneState {
    background: u32,
    objects: BTreeMap<PlaneObjectId, PlaneObject>,
    instances: Vec<PlaneInstance>,
    overlay: Vec<(i64, i64, u32)>,
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
            PlaneOp::ClearInstances => state.instances.clear(),
            PlaneOp::ClearOverlay => state.overlay.clear(),
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
/// overwrite), then the persistent overlay (authoritative), all in the
/// plane's sample domain.
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
    for inst in &state.instances {
        acc = acc.saturating_add(1);
        if acc > u64::from(limits.max_instances) {
            return Err(VoleError::MaterializationBudgetExceeded);
        }
        let Some(obj) = state.objects.get(&inst.object) else {
            continue; // mirrors v1's total skip for a missing object
        };
        let (dx, dy) = (inst.x, inst.y);
        match obj.content {
            PlaneContent::Fill(v) => {
                pic.fill_rect_clipped(
                    0,
                    v,
                    dx,
                    dy,
                    dx + i64::from(obj.width),
                    dy + i64::from(obj.height),
                )?;
            }
            PlaneContent::Raster(ref data) => {
                let samples: Vec<u32> = match data {
                    PlaneData::U8(v) => v.iter().map(|s| u32::from(*s)).collect(),
                    PlaneData::U16(v) => v.iter().map(|s| u32::from(*s)).collect(),
                };
                pic.blit(0, &samples, obj.width, obj.height, dx, dy)?;
            }
        }
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
    };
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
                _ => return Err(VoleError::NonCanonicalEncoding),
            }
        }
        prev = base;
    }
    Ok(prev)
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
