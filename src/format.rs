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
//!                |  0x22 i:u32 x:i32 y:i32 )              # set position
//! Integrity    := blake3 of every preceding byte (32 bytes)
//! ```
//!
//! v1 invariants: every object is declared before the single checkpoint;
//! interval indices strictly increase from `1`; `x`/`y` lie in
//! `[-MAX_COORD, MAX_COORD]`; unknown universe/profile/feature/tag and any
//! reference to an undeclared object or instance are typed errors.

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
// Transition tags.
pub(crate) const TR_CREATE_INSTANCE: u8 = 0x21;
pub(crate) const TR_SET_POSITION: u8 = 0x22;
pub(crate) const TR_PATCH_SPARSE: u8 = 0x23;
pub(crate) const TR_COPY_RECT: u8 = 0x24;
pub(crate) const TR_MOVE_RECT: u8 = 0x25;
pub(crate) const TR_SET_VELOCITY: u8 = 0x26;
pub(crate) const TR_ADVANCE_TRANSLATIONS: u8 = 0x27;

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

    let mut cur = State::new(Interval::ZERO);
    let mut initial_opt: Option<State> = None;
    let mut saw_checkpoint = false;
    let mut advance_work: u64 = 0;
    let mut intervals: Vec<(Interval, Vec<Transition>)> = Vec::new();
    let mut next_t = 0u64;
    let mut object_ids: HashSet<u32> = HashSet::new();

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
            TAG_CHECKPOINT => {
                if saw_checkpoint {
                    return Err(VoleError::NonCanonicalEncoding);
                }
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
                    tr.apply(&mut cur)?;
                    if let Transition::AdvanceTranslations = tr {
                        // Hostile bound: cumulative per-instance advance work must
                        // stay inside the envelope (checked after the advance, so
                        // the work itself is bounded by max_transition_work).
                        advance_work += cur.moving_count() as u64;
                        if advance_work > limits.max_transition_work {
                            return Err(VoleError::MaterializationBudgetExceeded);
                        }
                    }
                    group.push(tr);
                }
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

    /// Set the checkpoint background sample.
    pub fn background(mut self, bg: u8) -> Self {
        self.background = bg;
        self
    }

    /// Emit the checkpoint with the given live instances (paint order). All
    /// referenced object ids must already be declared.
    pub fn checkpoint_with(mut self, instances: &[Instance]) -> Result<Self, VoleError> {
        if self.have_checkpoint {
            return Err(VoleError::NonCanonicalEncoding);
        }
        // Write object declarations in declaration order.
        for (id, obj) in &self.objects {
            write_object_decl(&mut self.sink, *id, obj)?;
        }
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
        // These variants only appear in files in later v-formats; writing them
        // in a v1 file is rejected to keep the v1 grammar closed.
        _ => Err(VoleError::NonCanonicalEncoding),
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
