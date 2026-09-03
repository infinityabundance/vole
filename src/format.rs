//! Format v1: canonical wire grammar, writer, and parser for the standalone
//! `.vole` stream.
//!
//! No generic serialization is used in the normative format. It is a fixed,
//! little-endian, tag-prefixed structure that a hostile-input decoder parses
//! with typed errors at every boundary.
//!
//! # Grammar
//!
//! ```text
//! File         := Header ObjectDecl* Checkpoint Interval* Integrity
//! Header       := "VOLE" 0x00 fver:u16 univ:u32 prof:u8 feat:u32 w:u32 h:u32
//! ObjectDecl   := ( 0x01 obj:u32 w:u32 h:u32 (w*h gray)     # raster
//!                |  0x02 obj:u32 w:u32 h:u32 v:u8 )         # fill
//! Checkpoint   := 0x03 bg:u8 n:u32 (i:u32 o:u32 x:i32 y:i32)*
//! Interval     := 0x04 t:u64 n:u32 Transition*
//! Transition   := ( 0x21 i:u32 o:u32 x:i32 y:i32          # create instance
//!                |  0x22 i:u32 x:i32 y:i32              # set position
//!                |  0x23 n:u32 (x:i32 y:i32 v:u8)*      # sparse overlay patch
//!                |  0x24 sx:u32 sy:u32 w:u32 h:u32 dx:i32 dy:i32  # COPY_RECT
//!                |  0x25 sx:u32 sy:u32 w:u32 h:u32 dx:i32 dy:i32  # MOVE_RECT
//!                |  0x26 i:u32 vx:i32 vy:i32            # set translation
//!                |  0x27                                 # advance translations
//!                |  0x28                                 # clear instances
//!                |  0x29                                 # clear overlay
//!                |  0x2a len:u32 block                   # per-frame residual
//!                )
//! Integrity    := blake3 of every preceding byte (32 bytes)
//! ```
//!
//! v1 invariants: every object is declared before the single checkpoint;
//! interval indices strictly increase from `1` and the interval count is
//! bounded by `Limits.max_checkpoint_distance`; `x`/`y` lie in
//! `[-MAX_COORD, MAX_COORD]`; unknown universe/profile/feature/tag and any
//! reference to an undeclared object or instance are typed errors. A residual
//! `block` is a Phase-F self-describing payload (see `rans::encode_block`)
//! bounded by `Limits.max_residual_bytes`, structurally validated at parse
//! time and decoded only when the frame it appears in is materialized.

use std::collections::HashSet;

use crate::{
    checked::{ByteReader, ByteSink},
    error::VoleError,
    limits::{Limits, LIMIT_PROFILE_V1},
    object::{Object, ObjectId},
    state::{Instance, InstanceId, State},
    time::Interval,
    transition::Transition,
    universe::UNIVERSE_V1,
};

/// ASCII magic.
pub const MAGIC: [u8; 4] = *b"VOLE";
/// Format version carried by v1 files.
pub const FORMAT_VERSION: u16 = 1;
/// Reserved header byte (must be zero in canonical files).
pub const HEADER_RESERVED: u8 = 0x00;

// Record tags.
pub(crate) const TAG_OBJECT_RASTER: u8 = 0x01;
pub(crate) const TAG_OBJECT_FILL: u8 = 0x02;
pub(crate) const TAG_CHECKPOINT: u8 = 0x03;
pub(crate) const TAG_INTERVAL: u8 = 0x04;
pub(crate) const TAG_OBJECT_INDEX: u8 = 0x05;
pub(crate) const TAG_PALETTE: u8 = 0x06;
/// Checkpoint variant carrying per-instance palette bindings (Phase J).
/// Layout mirrors `TAG_CHECKPOINT` with one extra `palette:u32` per instance
/// record (0 = unbound). Only streams with at least one binding use it.
pub(crate) const TAG_CHECKPOINT_BINDINGS: u8 = 0x08;
// Transition tags.
pub(crate) const TR_CREATE_INSTANCE: u8 = 0x21;
pub(crate) const TR_SET_POSITION: u8 = 0x22;
pub(crate) const TR_PATCH_SPARSE: u8 = 0x23;
pub(crate) const TR_COPY_RECT: u8 = 0x24;
pub(crate) const TR_MOVE_RECT: u8 = 0x25;
pub(crate) const TR_SET_VELOCITY: u8 = 0x26;
pub(crate) const TR_ADVANCE_TRANSLATIONS: u8 = 0x27;
pub(crate) const TR_CLEAR_INSTANCES: u8 = 0x28;
pub(crate) const TR_CLEAR_OVERLAY: u8 = 0x29;
pub(crate) const TR_RESIDUAL: u8 = 0x2a;
pub(crate) const TR_SET_TRAJECTORY: u8 = 0x2b;
pub(crate) const TR_ADVANCE_TRAJECTORIES: u8 = 0x2c;
pub(crate) const TR_SET_PALETTE: u8 = 0x2d;
pub(crate) const TR_PATCH_PALETTE: u8 = 0x2e;
pub(crate) const TR_BIND_PALETTE: u8 = 0x2f;
pub(crate) const TR_SET_AFFINE: u8 = 0x30;

/// Maximum representable absolute coordinate on the wire.
pub(crate) const MAX_COORD: i64 = 1 << 24;

/// Canonical stream header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    format_version: u16,
    universe_id: u32,
    limit_profile: u8,
    feature_bits: u32,
    /// Samples per row of the canonical Gray8 view.
    pub width: u32,
    /// Rows of the canonical Gray8 view.
    pub height: u32,
}

impl Header {
    /// Write header bytes.
    pub fn write(&self, sink: &mut ByteSink) -> Result<(), VoleError> {
        sink.extend(&MAGIC)?;
        sink.byte(HEADER_RESERVED)?;
        sink.push(self.format_version)?;
        sink.push(self.universe_id)?;
        sink.byte(self.limit_profile)?;
        sink.push(self.feature_bits)?;
        sink.push(self.width)?;
        sink.push(self.height)?;
        Ok(())
    }
}

fn read_header(r: &mut ByteReader<'_>) -> Result<Header, VoleError> {
    if r.take(MAGIC.len())? != MAGIC {
        return Err(VoleError::BadMagic);
    }
    if r.u8()? != HEADER_RESERVED {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let format_version = r.pull::<u16>()?;
    let universe_id = r.pull::<u32>()?;
    let limit_profile = r.u8()?;
    let feature_bits = r.pull::<u32>()?;
    let width = r.pull::<u32>()?;
    let height = r.pull::<u32>()?;

    if format_version != FORMAT_VERSION {
        return Err(VoleError::UnsupportedFeature);
    }
    if universe_id != UNIVERSE_V1 {
        return Err(VoleError::UnsupportedUniverse);
    }
    if limit_profile != LIMIT_PROFILE_V1 {
        return Err(VoleError::UnsupportedLimitProfile);
    }
    if feature_bits != 0 {
        return Err(VoleError::UnsupportedFeature);
    }
    Ok(Header {
        format_version,
        universe_id,
        limit_profile,
        feature_bits,
        width,
        height,
    })
}

fn box_bytes(w: u32, h: u32) -> Result<usize, VoleError> {
    usize::try_from(u64::from(w) * u64::from(h)).map_err(|_| VoleError::LengthMismatch)
}

/// Encode a bounded `i64` position as its canonical `i32` wire word.
fn wpos(v: i64) -> i32 {
    debug_assert!(v.abs() <= MAX_COORD);
    v as i32
}

fn coord_guard(v: i64) -> Result<(), VoleError> {
    if v.abs() > MAX_COORD {
        Err(VoleError::NonCanonicalEncoding)
    } else {
        Ok(())
    }
}

/// A fully parsed, standalone `.vole` stream ready for replay.
#[derive(Debug, Clone)]
pub struct ParsedStream {
    header: Header,
    limits: Limits,
    /// Interval-0 state laid down by the checkpoint.
    initial: State,
    /// Interval groups in strictly increasing `t` order.
    intervals: Vec<(Interval, Vec<Transition>)>,
}

impl ParsedStream {
    fn new(
        header: Header,
        limits: Limits,
        initial: State,
        intervals: Vec<(Interval, Vec<Transition>)>,
    ) -> Self {
        Self {
            header,
            limits,
            initial,
            intervals,
        }
    }

    /// Header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Active limits.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Canvas width.
    pub fn width(&self) -> u32 {
        self.header.width
    }

    /// Canvas height.
    pub fn height(&self) -> u32 {
        self.header.height
    }

    /// Number of materializable full frames: the checkpoint frame plus one per
    /// interval group.
    pub fn frame_count(&self) -> u64 {
        1 + self.intervals.len() as u64
    }

    /// Clone of the interval-0 state.
    pub fn clone_initial(&self) -> State {
        self.initial.clone()
    }

    /// Borrow interval groups.
    pub fn intervals(&self) -> &[(Interval, Vec<Transition>)] {
        &self.intervals
    }
}

/// Parse and validate a whole file (verifies the trailing BLAKE3 digest).
pub fn parse_stream(bytes: &[u8]) -> Result<ParsedStream, VoleError> {
    if bytes.len() < 32 {
        return Err(VoleError::Truncated);
    }
    // The final 32 bytes are the integrity trailer; parse the trusted prefix.
    let content_len = bytes.len() - 32;
    let mut r = ByteReader::new(&bytes[..content_len]);

    let header = read_header(&mut r)?;
    let limits = Limits::for_profile(header.limit_profile)?;
    limits.check_canvas(header.width, header.height)?;
    limits.check_stream_len(bytes.len() as u64)?;

    let mut cur = State::new(Interval::ZERO);
    let mut initial_opt: Option<State> = None;
    let mut saw_checkpoint = false;
    let mut advance_work: u64 = 0;
    let mut trajectory_work: u64 = 0;
    let mut intervals: Vec<(Interval, Vec<Transition>)> = Vec::new();
    let mut next_t = 0u64;
    let mut object_ids: HashSet<u32> = HashSet::new();
    let mut palette_ids: HashSet<u32> = HashSet::new();

    while r.remaining() > 0 {
        let tag = r.u8()?;
        match tag {
            TAG_OBJECT_RASTER => {
                if saw_checkpoint {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let id = ObjectId(r.pull::<u32>()?);
                let w = r.pull::<u32>()?;
                let h = r.pull::<u32>()?;
                let n = box_bytes(w, h)?;
                if n as u64 > limits.max_object_bytes {
                    return Err(VoleError::DimensionTooLarge);
                }
                let data = r.take_vec(n)?;
                let obj = Object::raster(w, h, data)?;
                if !object_ids.insert(id.0) {
                    return Err(VoleError::DuplicateId);
                }
                cur.declare_object(id, obj)?;
            }
            TAG_OBJECT_INDEX => {
                if saw_checkpoint {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let id = ObjectId(r.pull::<u32>()?);
                let w = r.pull::<u32>()?;
                let h = r.pull::<u32>()?;
                let n = box_bytes(w, h)?;
                if n as u64 > limits.max_object_bytes {
                    return Err(VoleError::DimensionTooLarge);
                }
                let data = r.take_vec(n)?;
                let obj = Object::index_raster(w, h, data)?;
                if !object_ids.insert(id.0) {
                    return Err(VoleError::DuplicateId);
                }
                cur.declare_object(id, obj)?;
            }
            TAG_OBJECT_FILL => {
                if saw_checkpoint {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let id = ObjectId(r.pull::<u32>()?);
                let w = r.pull::<u32>()?;
                let h = r.pull::<u32>()?;
                let value = r.u8()?;
                if u64::from(w) * u64::from(h) > limits.max_object_bytes {
                    return Err(VoleError::DimensionTooLarge);
                }
                let obj = Object::fill(w, h, value)?;
                if !object_ids.insert(id.0) {
                    return Err(VoleError::DuplicateId);
                }
                cur.declare_object(id, obj)?;
            }
            TAG_PALETTE => {
                if saw_checkpoint {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let id = crate::state::PaletteId(r.pull::<u32>()?);
                let len = r.pull::<u32>()?;
                if u64::from(len) > u64::from(limits.max_palette_entries) {
                    return Err(VoleError::DimensionTooLarge);
                }
                if len == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let entries = r.take_vec(len as usize)?;
                if !palette_ids.insert(id.0) {
                    return Err(VoleError::DuplicateId);
                }
                cur.set_palette(id, entries)?;
                if cur.palette_count() as u64 > u64::from(limits.max_palettes) {
                    return Err(VoleError::DimensionTooLarge);
                }
            }
            TAG_CHECKPOINT | TAG_CHECKPOINT_BINDINGS => {
                if saw_checkpoint {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let with_bindings = tag == TAG_CHECKPOINT_BINDINGS;
                let background = r.u8()?;
                cur.set_background(background);
                let n = r.pull::<u32>()?;
                if n as u64 > u64::from(limits.max_instances) {
                    return Err(VoleError::DimensionTooLarge);
                }
                let mut inst_ids: HashSet<u32> = HashSet::new();
                for _ in 0..n {
                    let i = InstanceId(r.pull::<u32>()?);
                    let o = ObjectId(r.pull::<u32>()?);
                    let x = i64::from(r.pull::<i32>()?);
                    let y = i64::from(r.pull::<i32>()?);
                    coord_guard(x)?;
                    coord_guard(y)?;
                    if !inst_ids.insert(i.0) {
                        return Err(VoleError::DuplicateId);
                    }
                    cur.create_instance(i, o, x, y)?;
                    if with_bindings {
                        let palette = crate::state::PaletteId(r.pull::<u32>()?);
                        if palette != crate::state::PaletteId::NONE {
                            // The palette must already be declared (palette
                            // records precede the checkpoint).
                            cur.bind_palette(i, palette)?;
                        }
                    }
                }
                // Snapshot the pristine interval-0 state here, before any
                // interval below mutates `cur`.
                initial_opt = Some(cur.clone());
                saw_checkpoint = true;
            }
            TAG_INTERVAL => {
                if !saw_checkpoint {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let t = r.pull::<u64>()?;
                if t == 0 || t <= next_t {
                    return Err(VoleError::NonConsecutiveInterval);
                }
                next_t = t;
                let n = r.pull::<u32>()?;
                if n as u64 > limits.max_transitions_per_interval as u64 {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
                let mut group = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let t2 = r.u8()?;
                    let tr = match t2 {
                        TR_CREATE_INSTANCE => {
                            let id = InstanceId(r.pull::<u32>()?);
                            let o = ObjectId(r.pull::<u32>()?);
                            let x = i64::from(r.pull::<i32>()?);
                            let y = i64::from(r.pull::<i32>()?);
                            coord_guard(x)?;
                            coord_guard(y)?;
                            Transition::CreateInstance {
                                id,
                                object: o,
                                x,
                                y,
                            }
                        }
                        TR_SET_POSITION => {
                            let id = InstanceId(r.pull::<u32>()?);
                            let x = i64::from(r.pull::<i32>()?);
                            let y = i64::from(r.pull::<i32>()?);
                            coord_guard(x)?;
                            coord_guard(y)?;
                            Transition::SetPosition { id, x, y }
                        }
                        TR_SET_VELOCITY => {
                            let id = InstanceId(r.pull::<u32>()?);
                            let vx = i64::from(r.pull::<i32>()?);
                            let vy = i64::from(r.pull::<i32>()?);
                            coord_guard(vx)?;
                            coord_guard(vy)?;
                            Transition::SetVelocity { id, vx, vy }
                        }
                        TR_ADVANCE_TRANSLATIONS => Transition::AdvanceTranslations,
                        TR_PATCH_SPARSE => {
                            let m = r.pull::<u32>()?;
                            if m as u64 > limits.max_canvas_bytes {
                                return Err(VoleError::NonCanonicalEncoding);
                            }
                            let mut points = Vec::with_capacity(m as usize);
                            let mut prev: Option<(i64, i64)> = None;
                            for _ in 0..m {
                                let x = i64::from(r.pull::<i32>()?);
                                let y = i64::from(r.pull::<i32>()?);
                                let v = r.u8()?;
                                coord_guard(x)?;
                                coord_guard(y)?;
                                let key = (x, y);
                                if prev.as_ref().is_some_and(|p| key <= *p) {
                                    return Err(VoleError::NonCanonicalEncoding);
                                }
                                prev = Some(key);
                                points.push((x, y, v));
                            }
                            Transition::PatchSparse { points }
                        }
                        TR_CLEAR_INSTANCES => Transition::ClearInstances,
                        TR_CLEAR_OVERLAY => Transition::ClearOverlay,
                        TR_RESIDUAL => {
                            let block_len = r.pull::<u32>()?;
                            if u64::from(block_len) > limits.max_residual_bytes {
                                return Err(VoleError::DimensionTooLarge);
                            }
                            let block = r.take_vec(block_len as usize)?;
                            if block.first() == Some(&crate::rans::KIND_TSF) {
                                crate::transform::check_block(
                                    &block,
                                    limits.max_residual_bytes,
                                    header.width,
                                    header.height,
                                )?;
                            } else {
                                crate::rans::check_block(&block, limits.max_residual_bytes)?;
                            }
                            Transition::Residual { block }
                        }
                        TR_SET_TRAJECTORY => {
                            let id = InstanceId(r.pull::<u32>()?);
                            let n = r.pull::<u32>()?;
                            if u64::from(n) > u64::from(limits.max_trajectory_segments) {
                                return Err(VoleError::MaterializationBudgetExceeded);
                            }
                            let mut segments = Vec::with_capacity(n as usize);
                            for _ in 0..n {
                                let kind = r.u8()?;
                                let seg = match kind {
                                    crate::trajectory::SEG_LINEAR => {
                                        let vx = i64::from(r.pull::<i32>()?);
                                        let vy = i64::from(r.pull::<i32>()?);
                                        let steps = r.pull::<u64>()?;
                                        coord_guard(vx)?;
                                        coord_guard(vy)?;
                                        crate::trajectory::TrajectorySegment::Linear {
                                            vx,
                                            vy,
                                            steps,
                                        }
                                    }
                                    crate::trajectory::SEG_ACCEL => {
                                        let vx0 = i64::from(r.pull::<i32>()?);
                                        let vy0 = i64::from(r.pull::<i32>()?);
                                        let ax = i64::from(r.pull::<i32>()?);
                                        let ay = i64::from(r.pull::<i32>()?);
                                        let steps = r.pull::<u64>()?;
                                        coord_guard(vx0)?;
                                        coord_guard(vy0)?;
                                        coord_guard(ax)?;
                                        coord_guard(ay)?;
                                        crate::trajectory::TrajectorySegment::Accel {
                                            vx0,
                                            vy0,
                                            ax,
                                            ay,
                                            steps,
                                        }
                                    }
                                    _ => return Err(VoleError::NonCanonicalEncoding),
                                };
                                seg.check()?;
                                segments.push(seg);
                            }
                            Transition::SetTrajectory { id, segments }
                        }
                        TR_ADVANCE_TRAJECTORIES => Transition::AdvanceTrajectories,
                        TR_SET_PALETTE => {
                            let id = crate::state::PaletteId(r.pull::<u32>()?);
                            let len = r.pull::<u32>()?;
                            if u64::from(len) > u64::from(limits.max_palette_entries) {
                                return Err(VoleError::DimensionTooLarge);
                            }
                            if len == 0 {
                                return Err(VoleError::NonCanonicalEncoding);
                            }
                            let entries = r.take_vec(len as usize)?;
                            Transition::SetPalette { id, entries }
                        }
                        TR_PATCH_PALETTE => {
                            let id = crate::state::PaletteId(r.pull::<u32>()?);
                            let m = r.pull::<u32>()?;
                            // Strictly ascending u8 indices: at most 256 distinct
                            // entries can ever be canonical.
                            if u64::from(m) > 256 {
                                return Err(VoleError::NonCanonicalEncoding);
                            }
                            let mut changes = Vec::with_capacity(m as usize);
                            let mut prev: Option<u8> = None;
                            for _ in 0..m {
                                let idx = r.u8()?;
                                let v = r.u8()?;
                                if prev.is_some_and(|p| idx <= p) {
                                    return Err(VoleError::NonCanonicalEncoding);
                                }
                                prev = Some(idx);
                                changes.push((idx, v));
                            }
                            Transition::PatchPalette { id, changes }
                        }
                        TR_BIND_PALETTE => {
                            let instance = InstanceId(r.pull::<u32>()?);
                            let palette = crate::state::PaletteId(r.pull::<u32>()?);
                            Transition::BindPalette { instance, palette }
                        }
                        TR_SET_AFFINE => {
                            let id = InstanceId(r.pull::<u32>()?);
                            let coeff = |r: &mut ByteReader<'_>| -> Result<i64, VoleError> {
                                let v = i64::from(r.pull::<i32>()?);
                                if v.abs() > MAX_COORD {
                                    return Err(VoleError::NonCanonicalEncoding);
                                }
                                Ok(v)
                            };
                            let params = crate::affine::AffineParams {
                                a: coeff(&mut r)?,
                                b: coeff(&mut r)?,
                                c: coeff(&mut r)?,
                                d: coeff(&mut r)?,
                                e: coeff(&mut r)?,
                                f: coeff(&mut r)?,
                            };
                            params.check()?;
                            Transition::SetAffine { id, params }
                        }
                        TR_COPY_RECT | TR_MOVE_RECT => {
                            let is_copy = t2 == TR_COPY_RECT;
                            let src_x = i64::from(r.pull::<i32>()?);
                            let src_y = i64::from(r.pull::<i32>()?);
                            let width = r.pull::<u32>()?;
                            let height = r.pull::<u32>()?;
                            let dst_x = i64::from(r.pull::<i32>()?);
                            let dst_y = i64::from(r.pull::<i32>()?);
                            coord_guard(src_x)?;
                            coord_guard(src_y)?;
                            coord_guard(dst_x)?;
                            coord_guard(dst_y)?;
                            if width == 0 || height == 0 {
                                return Err(VoleError::NonCanonicalEncoding);
                            }
                            if u64::from(width) * u64::from(height) > limits.max_copy_area {
                                return Err(VoleError::MaterializationBudgetExceeded);
                            }
                            if is_copy {
                                Transition::CopyRect {
                                    src_x,
                                    src_y,
                                    width,
                                    height,
                                    dst_x,
                                    dst_y,
                                }
                            } else {
                                Transition::MoveRect {
                                    src_x,
                                    src_y,
                                    width,
                                    height,
                                    dst_x,
                                    dst_y,
                                }
                            }
                        }
                        _ => return Err(VoleError::NonCanonicalEncoding),
                    };
                    // Hostile work budgets are accounted *before* the advance is
                    // applied, so a program that deactivates on this very step is
                    // still counted (mirrors `max_transition_work` semantics).
                    if let Transition::AdvanceTranslations = tr {
                        advance_work += cur.moving_count() as u64;
                        if advance_work > limits.max_transition_work {
                            return Err(VoleError::MaterializationBudgetExceeded);
                        }
                    }
                    if let Transition::AdvanceTrajectories = tr {
                        trajectory_work += cur.trajectory_count() as u64;
                        if trajectory_work > limits.max_trajectory_work {
                            return Err(VoleError::MaterializationBudgetExceeded);
                        }
                    }
                    tr.apply(&mut cur)?;
                    if let Transition::PatchSparse { .. } = tr {
                        limits.check_overlay_points(cur.overlay_len() as u64)?;
                    }
                    if let Transition::SetPalette { .. } = tr {
                        if cur.palette_count() as u64 > u64::from(limits.max_palettes) {
                            return Err(VoleError::DimensionTooLarge);
                        }
                    }
                    group.push(tr);
                }
                limits.check_interval_distance(intervals.len() as u64)?;
                intervals.push((Interval(t), group));
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        }
    }

    if !saw_checkpoint {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let initial = initial_opt.ok_or(VoleError::NonCanonicalEncoding)?;
    let parsed = ParsedStream::new(header, limits, initial, intervals);
    // Integrity protects every byte of the file after structural checks; run it
    // last so header-semantic errors surface with their specific reason.
    crate::integr::verify_trailer(bytes)?;
    Ok(parsed)
}

/// Assemble a full canonical `.vole` file.
#[derive(Default)]
pub struct StreamWriter {
    sink: ByteSink,
    width: u32,
    height: u32,
    objects: Vec<(ObjectId, Object)>,
    palettes: Vec<(crate::state::PaletteId, Vec<u8>)>,
    background: u8,
    have_checkpoint: bool,
    last_interval: u64,
}

impl StreamWriter {
    /// Begin a stream over a `width x height` Gray8 canvas.
    pub fn begin(width: u32, height: u32) -> Self {
        StreamWriter {
            sink: ByteSink::new(),
            width,
            height,
            objects: Vec::new(),
            palettes: Vec::new(),
            background: 0,
            have_checkpoint: false,
            last_interval: 0,
        }
    }

    /// Declare one pre-checkpoint object.
    pub fn declare_object(mut self, id: ObjectId, obj: Object) -> Result<Self, VoleError> {
        if self.have_checkpoint || self.objects.iter().any(|(i, _)| *i == id) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        self.objects.push((id, obj));
        Ok(self)
    }

    /// Declare one pre-checkpoint palette (Phase J): the palette's initial
    /// entries are part of the interval-0 state. Duplicate ids are rejected.
    pub fn palette(
        mut self,
        id: crate::state::PaletteId,
        entries: Vec<u8>,
    ) -> Result<Self, VoleError> {
        if self.have_checkpoint
            || id == crate::state::PaletteId::NONE
            || entries.is_empty()
            || self.palettes.iter().any(|(i, _)| *i == id)
        {
            return Err(VoleError::NonCanonicalEncoding);
        }
        if entries.len() as u64 > u64::from(crate::limits::Limits::default().max_palette_entries) {
            return Err(VoleError::DimensionTooLarge);
        }
        self.palettes.push((id, entries));
        Ok(self)
    }

    /// Set the checkpoint background sample.
    pub fn background(mut self, bg: u8) -> Self {
        self.background = bg;
        self
    }

    /// Emit the checkpoint with the given live instances (paint order). All
    /// referenced object ids must already be declared; declared palettes (if
    /// any) are written just before the checkpoint record.
    pub fn checkpoint_with(mut self, instances: &[Instance]) -> Result<Self, VoleError> {
        self.write_decls()?;
        self.sink.byte(TAG_CHECKPOINT)?;
        self.sink.byte(self.background)?;
        self.sink.push(instances.len() as u32)?;
        for inst in instances {
            self.sink.push(inst.id.0)?;
            self.sink.push(inst.object_id.0)?;
            self.sink.push(wpos(inst.x))?;
            self.sink.push(wpos(inst.y))?;
        }
        self.have_checkpoint = true;
        Ok(self)
    }

    /// Emit the palette-aware checkpoint variant (Phase J): every instance
    /// record carries its palette binding (`None` is written as `0`). Palettes
    /// referenced by a binding must already be declared (writer order).
    pub fn checkpoint_with_bindings(
        mut self,
        instances: &[(Instance, Option<crate::state::PaletteId>)],
    ) -> Result<Self, VoleError> {
        if self.have_checkpoint {
            return Err(VoleError::NonCanonicalEncoding);
        }
        for (_, binding) in instances {
            if let Some(p) = binding {
                if !self.palettes.iter().any(|(id, _)| id == p) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
        }
        self.write_decls()?;
        self.sink.byte(TAG_CHECKPOINT_BINDINGS)?;
        self.sink.byte(self.background)?;
        self.sink.push(instances.len() as u32)?;
        for (inst, binding) in instances {
            self.sink.push(inst.id.0)?;
            self.sink.push(inst.object_id.0)?;
            self.sink.push(wpos(inst.x))?;
            self.sink.push(wpos(inst.y))?;
            self.sink.push(binding.map_or(0, |p| p.0))?;
        }
        self.have_checkpoint = true;
        Ok(self)
    }

    /// Write all pre-checkpoint declarations (objects, then palettes) in
    /// declaration order.
    fn write_decls(&mut self) -> Result<(), VoleError> {
        for (id, obj) in &self.objects {
            write_object_decl(&mut self.sink, *id, obj)?;
        }
        for (id, entries) in &self.palettes {
            self.sink.byte(TAG_PALETTE)?;
            self.sink.push(id.0)?;
            self.sink.push(entries.len() as u32)?;
            self.sink.extend(entries)?;
        }
        Ok(())
    }

    /// Append an interval with transitions targeting absolute `t` (strictly
    /// increasing, starting at 1).
    pub fn interval(mut self, t: Interval, transitions: &[Transition]) -> Result<Self, VoleError> {
        if !self.have_checkpoint || t.0 == 0 || t.0 <= self.last_interval {
            return Err(VoleError::NonConsecutiveInterval);
        }
        self.last_interval = t.0;
        self.sink.byte(TAG_INTERVAL)?;
        self.sink.push(t.0)?;
        self.sink.push(transitions.len() as u32)?;
        for tr in transitions {
            write_transition(&mut self.sink, tr)?;
        }
        Ok(self)
    }

    /// Finish the file (appends the integrity trailer) and return the bytes.
    pub fn finish(mut self) -> Result<Vec<u8>, VoleError> {
        if !self.have_checkpoint {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let header = Header {
            format_version: FORMAT_VERSION,
            universe_id: UNIVERSE_V1,
            limit_profile: LIMIT_PROFILE_V1,
            feature_bits: 0,
            width: self.width,
            height: self.height,
        };
        let body = std::mem::take(&mut self.sink);
        let mut full = ByteSink::new();
        header.write(&mut full)?;
        full.extend(body.as_slice())?;
        crate::integr::append_trailer(&mut full)?;
        Ok(full.into_vec())
    }
}

fn write_object_decl(sink: &mut ByteSink, id: ObjectId, obj: &Object) -> Result<(), VoleError> {
    match obj.fill_value() {
        Some(v) => {
            sink.byte(TAG_OBJECT_FILL)?;
            sink.push(id.0)?;
            sink.push(obj.width())?;
            sink.push(obj.height())?;
            sink.byte(v)
        }
        None => match obj.indices() {
            Some(indices) => {
                if indices.len() as u64 != obj.sample_count() {
                    return Err(VoleError::ObjectGeometryMismatch);
                }
                sink.byte(TAG_OBJECT_INDEX)?;
                sink.push(id.0)?;
                sink.push(obj.width())?;
                sink.push(obj.height())?;
                sink.extend(indices)
            }
            None => {
                let raster = obj.samples().ok_or(VoleError::ObjectGeometryMismatch)?;
                if raster.len() as u64 != obj.sample_count() {
                    return Err(VoleError::ObjectGeometryMismatch);
                }
                sink.byte(TAG_OBJECT_RASTER)?;
                sink.push(id.0)?;
                sink.push(obj.width())?;
                sink.push(obj.height())?;
                sink.extend(raster)
            }
        },
    }
}

fn write_transition(sink: &mut ByteSink, t: &Transition) -> Result<(), VoleError> {
    match t {
        Transition::CreateInstance { id, object, x, y } => {
            sink.byte(TR_CREATE_INSTANCE)?;
            sink.push(id.0)?;
            sink.push(object.0)?;
            sink.push(wpos(*x))?;
            sink.push(wpos(*y))
        }
        Transition::SetPosition { id, x, y } => {
            sink.byte(TR_SET_POSITION)?;
            sink.push(id.0)?;
            sink.push(wpos(*x))?;
            sink.push(wpos(*y))
        }
        Transition::SetVelocity { id, vx, vy } => {
            sink.byte(TR_SET_VELOCITY)?;
            sink.push(id.0)?;
            sink.push(wpos(*vx))?;
            sink.push(wpos(*vy))
        }
        Transition::AdvanceTranslations => sink.byte(TR_ADVANCE_TRANSLATIONS),
        Transition::AdvanceTrajectories => sink.byte(TR_ADVANCE_TRAJECTORIES),
        Transition::SetTrajectory { id, segments } => {
            let n = u32::try_from(segments.len()).map_err(|_| VoleError::ArithmeticOverflow)?;
            crate::trajectory::check_program(segments, &crate::limits::Limits::default())?;
            sink.byte(TR_SET_TRAJECTORY)?;
            sink.push(id.0)?;
            sink.push(n)?;
            for seg in segments {
                match seg {
                    crate::trajectory::TrajectorySegment::Linear { vx, vy, steps } => {
                        sink.byte(crate::trajectory::SEG_LINEAR)?;
                        sink.push(wpos(*vx))?;
                        sink.push(wpos(*vy))?;
                        sink.push(*steps)?;
                    }
                    crate::trajectory::TrajectorySegment::Accel {
                        vx0,
                        vy0,
                        ax,
                        ay,
                        steps,
                    } => {
                        sink.byte(crate::trajectory::SEG_ACCEL)?;
                        sink.push(wpos(*vx0))?;
                        sink.push(wpos(*vy0))?;
                        sink.push(wpos(*ax))?;
                        sink.push(wpos(*ay))?;
                        sink.push(*steps)?;
                    }
                }
            }
            Ok(())
        }
        Transition::SetPalette { id, entries } => {
            if *id == crate::state::PaletteId::NONE || entries.is_empty() {
                return Err(VoleError::NonCanonicalEncoding);
            }
            if entries.len() as u64
                > u64::from(crate::limits::Limits::default().max_palette_entries)
            {
                return Err(VoleError::DimensionTooLarge);
            }
            sink.byte(TR_SET_PALETTE)?;
            sink.push(id.0)?;
            sink.push(entries.len() as u32)?;
            sink.extend(entries)
        }
        Transition::PatchPalette { id, changes } => {
            if changes.len() > 256 {
                return Err(VoleError::NonCanonicalEncoding);
            }
            sink.byte(TR_PATCH_PALETTE)?;
            sink.push(id.0)?;
            sink.push(changes.len() as u32)?;
            let mut prev: Option<u8> = None;
            for (idx, v) in changes {
                if prev.is_some_and(|p| *idx <= p) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                prev = Some(*idx);
                sink.byte(*idx)?;
                sink.byte(*v)?;
            }
            Ok(())
        }
        Transition::BindPalette { instance, palette } => {
            sink.byte(TR_BIND_PALETTE)?;
            sink.push(instance.0)?;
            sink.push(palette.0)
        }
        Transition::SetAffine { id, params } => {
            params.check()?;
            let coeff = |sink: &mut ByteSink, v: i64| -> Result<(), VoleError> {
                let w = i32::try_from(v).map_err(|_| VoleError::NonCanonicalEncoding)?;
                sink.push(w)
            };
            sink.byte(TR_SET_AFFINE)?;
            sink.push(id.0)?;
            coeff(sink, params.a)?;
            coeff(sink, params.b)?;
            coeff(sink, params.c)?;
            coeff(sink, params.d)?;
            coeff(sink, params.e)?;
            coeff(sink, params.f)
        }
        Transition::ClearInstances => sink.byte(TR_CLEAR_INSTANCES),
        Transition::ClearOverlay => sink.byte(TR_CLEAR_OVERLAY),
        Transition::Residual { block } => {
            let n = u32::try_from(block.len()).map_err(|_| VoleError::ArithmeticOverflow)?;
            sink.byte(TR_RESIDUAL)?;
            sink.push(n)?;
            sink.extend(block)
        }
        Transition::PatchSparse { points } => {
            sink.byte(TR_PATCH_SPARSE)?;
            sink.push(points.len() as u32)?;
            for (x, y, v) in points {
                sink.push(wpos(*x))?;
                sink.push(wpos(*y))?;
                sink.byte(*v)?;
            }
            Ok(())
        }
        Transition::CopyRect {
            src_x,
            src_y,
            width,
            height,
            dst_x,
            dst_y,
        } => write_copy(
            sink,
            TR_COPY_RECT,
            *src_x,
            *src_y,
            *width,
            *height,
            *dst_x,
            *dst_y,
        ),
        Transition::MoveRect {
            src_x,
            src_y,
            width,
            height,
            dst_x,
            dst_y,
        } => write_copy(
            sink,
            TR_MOVE_RECT,
            *src_x,
            *src_y,
            *width,
            *height,
            *dst_x,
            *dst_y,
        ),
        Transition::DeclareObject(..) | Transition::DeclareFill { .. } => {
            // Object declarations are written pre-checkpoint by
            // `write_object_decl`; inside an interval group they would break
            // the v1 grammar and are rejected.
            Err(VoleError::NonCanonicalEncoding)
        }
    }
}

/// Common writer for COPY_RECT/MOVE_RECT geometry.
#[allow(clippy::too_many_arguments)] // canonical geometry totals 8 ordered fields; kept inline to match the wire layout
fn write_copy(
    sink: &mut ByteSink,
    tag: u8,
    src_x: i64,
    src_y: i64,
    width: u32,
    height: u32,
    dst_x: i64,
    dst_y: i64,
) -> Result<(), VoleError> {
    sink.byte(tag)?;
    sink.push(wpos(src_x))?;
    sink.push(wpos(src_y))?;
    sink.push(width)?;
    sink.push(height)?;
    sink.push(wpos(dst_x))?;
    sink.push(wpos(dst_y))?;
    Ok(())
}
