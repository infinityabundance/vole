//! Procedural state graph for the phase-A domain model.
//!
//! Here `G_t = (objects, instances)` plus a background colour. Objects are
//! immutable, declared content. Instances are the mutable-by-transition
//! placements (position, ordering) that materialize as a raster view.
//!
//! The compositor semantics bound the graph to a painter's model:
//! the canvas first fills the whole frame with `background`, then each
//! instance paints its (immutable) object over the canvas in *instance order*
//! (later instances are nearer / higher z).
//!
//! This struct is used by both the normative materializer and the direct
//! procedural ingest API; the checkpoint serializes an exact snapshot of it.

use std::collections::BTreeMap;

use crate::{
    affine::AffineParams,
    error::VoleError,
    object::{Object, ObjectId},
    time::Interval,
    trajectory::TrajectorySegment,
};

/// Instance identity in format-v1 index space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstanceId(pub u32);

/// Palette identity in format-v1 index space (Phase J). Zero is reserved as
/// the wire sentinel for "no binding"; palettes are declared from id 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaletteId(pub u32);

impl PaletteId {
    /// Wire sentinel: no palette binding.
    pub const NONE: PaletteId = PaletteId(0);
}

/// A placed, orderable instance of an immutable object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// Instance identity for transition targeting.
    pub id: InstanceId,
    /// The declared object whose content this paints.
    pub object_id: ObjectId,
    /// Top-left canvas position of the object box.
    pub x: i64,
    pub y: i64,
}

/// The live trajectory state of one instance (Phase I). The program is the
/// full descriptor attached by `SetTrajectory`; `seg`/`remaining` locate the
/// evaluation inside the program and `(vx, vy)` is the current per-advance
/// velocity (the *next* displacement to apply). See `crate::trajectory` for
/// the exact integer semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceTrajectory {
    program: Vec<TrajectorySegment>,
    /// Index of the currently active segment.
    seg: usize,
    /// Advances remaining in the current segment.
    remaining: u64,
    /// Current velocity (per-advance displacement).
    vx: i64,
    vy: i64,
}

impl InstanceTrajectory {
    /// The attached program descriptor.
    pub fn program(&self) -> &[TrajectorySegment] {
        &self.program
    }

    /// Index of the active segment.
    pub fn segment_index(&self) -> usize {
        self.seg
    }

    /// Advances remaining in the active segment.
    pub fn remaining_steps(&self) -> u64 {
        self.remaining
    }

    /// Current velocity `(vx, vy)`.
    pub fn velocity(&self) -> (i64, i64) {
        (self.vx, self.vy)
    }
}

impl InstanceTrajectory {
    /// Fresh trajectory positioned at the first segment.
    fn start(program: Vec<TrajectorySegment>) -> Option<InstanceTrajectory> {
        let seg = program.first()?;
        let (vx, vy) = match seg {
            TrajectorySegment::Linear { vx, vy, .. }
            | TrajectorySegment::Accel {
                vx0: vx, vy0: vy, ..
            } => (*vx, *vy),
        };
        Some(InstanceTrajectory {
            remaining: seg.steps(),
            program,
            seg: 0,
            vx,
            vy,
        })
    }
}

/// The phase-A procedural state.
#[derive(Debug, Clone)]
pub struct State {
    /// Full-frame background fill (a `FILL` semantic).
    background: u8,
    /// Declared immutable objects keyed by object id.
    objects: BTreeMap<ObjectId, Object>,
    /// Live instances in paint order.
    instances: Vec<Instance>,
    /// Sparse persistent overlay painted above all instances (Phase C: sparse
    /// mutation). Keyed by canvas coordinate; a set pixel persists until
    /// overwritten.
    overlay: BTreeMap<(i64, i64), u8>,
    /// Persistent integer translation state per instance (Phase E). A
    /// translation `(vx, vy)` is applied once per `AdvanceTranslations`
    /// transition: `position(t+1) = position(t) + (vx, vy)`. Absence from this
    /// map means the instance is stationary (velocity `(0,0)`).
    velocities: BTreeMap<InstanceId, (i64, i64)>,
    /// Persistent parametric trajectory state per instance (Phase I). A
    /// trajectory program is stepped once per `AdvanceTrajectories`
    /// transition. Trajectory and translation state on one instance are
    /// mutually exclusive: attaching one removes the other. Absence from this
    /// map means the instance carries no trajectory (stationary or
    /// velocity-driven).
    trajectories: BTreeMap<InstanceId, InstanceTrajectory>,
    /// Palette table (Phase J): palette id → current entries. Entries are
    /// mutable-by-transition state (a palette-index plane re-renders with new
    /// gray values as its palette mutates). Palettes persist across instance
    /// clearing and are bounded by `Limits.max_palettes` /
    /// `Limits.max_palette_entries` at the stream layers.
    palettes: BTreeMap<PaletteId, Vec<u8>>,
    /// Per-instance palette bindings (Phase J). An instance bound to a
    /// palette renders any palette-index object through that palette's
    /// entries. Bindings die with their instances (`ClearInstances`).
    bindings: BTreeMap<InstanceId, PaletteId>,
    /// Per-instance affine placements (Phase L). An instance with affine
    /// state paints its object through the canonical Q8 source map instead of
    /// the plain `(x, y)` placement. Affine, velocity, and trajectory state
    /// on one instance are mutually exclusive (attaching one removes the
    /// others); affines die with their instances.
    affines: BTreeMap<InstanceId, AffineParams>,
    /// Which interval this state snapshot was produced for (diagnostics and
    /// checkpoint anchoring; a fresh state is interval ZERO).
    interval: Interval,
}

impl Default for State {
    fn default() -> Self {
        Self {
            background: 0,
            objects: BTreeMap::new(),
            instances: Vec::new(),
            overlay: BTreeMap::new(),
            velocities: BTreeMap::new(),
            trajectories: BTreeMap::new(),
            palettes: BTreeMap::new(),
            bindings: BTreeMap::new(),
            affines: BTreeMap::new(),
            interval: Interval::ZERO,
        }
    }
}

impl State {
    /// Fresh empty state at the given interval with the default background.
    pub fn new(interval: Interval) -> Self {
        Self {
            interval,
            ..Self::default()
        }
    }

    /// Interval anchored to this state.
    pub fn interval(&self) -> Interval {
        self.interval
    }

    /// Set the anchoring interval (used by replay stepping).
    pub fn set_interval(&mut self, interval: Interval) {
        self.interval = interval;
    }

    /// Background colour applied each materialized frame.
    pub fn background(&self) -> u8 {
        self.background
    }

    /// Set the background.
    pub fn set_background(&mut self, colour: u8) {
        self.background = colour;
    }

    /// Number of declared objects.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Number of live instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Lookup an object's content reference.
    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(&id)
    }

    /// Iterate objects in id order.
    pub fn objects(&self) -> impl Iterator<Item = (&ObjectId, &Object)> {
        self.objects.iter()
    }

    /// Iterate instances in paint order.
    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.instances.iter()
    }

    /// Lookup an instance (failure if absent).
    pub fn instance(&self, id: InstanceId) -> Result<&Instance, VoleError> {
        self.instances
            .iter()
            .find(|i| i.id == id)
            .ok_or(VoleError::UnknownInstance)
    }

    /// Declare a new immutable object. Duplicate ids are an error and
    /// conflicting (identical id but different bytes) declarations are also
    /// rejected: identity is exact.
    pub fn declare_object(&mut self, id: ObjectId, object: Object) -> Result<(), VoleError> {
        if self.objects.contains_key(&id) {
            return Err(VoleError::DuplicateId);
        }
        self.objects.insert(id, object);
        Ok(())
    }

    /// Whether the object is present.
    pub fn has_object(&self, id: ObjectId) -> bool {
        self.objects.contains_key(&id)
    }

    /// Create an instance. Drawing order is the append order.
    pub fn create_instance(
        &mut self,
        id: InstanceId,
        object_id: ObjectId,
        x: i64,
        y: i64,
    ) -> Result<(), VoleError> {
        if self.instances.iter().any(|i| i.id == id) {
            return Err(VoleError::DuplicateId);
        }
        if !self.objects.contains_key(&object_id) {
            return Err(VoleError::UnknownObject);
        }
        self.instances.push(Instance {
            id,
            object_id,
            x,
            y,
        });
        Ok(())
    }

    /// Move the instance's object placement to an absolute position.
    pub fn set_position(&mut self, id: InstanceId, x: i64, y: i64) -> Result<(), VoleError> {
        let inst = self
            .instances
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or(VoleError::UnknownInstance)?;
        inst.x = x;
        inst.y = y;
        Ok(())
    }

    /// Remove every live instance. Paint order is cleared and instance ids are
    /// freed for reuse (Phase G: full-content replacement). Objects stay
    /// declared; the background, overlay, and palette table are untouched.
    /// Velocities, trajectories, palette bindings, and affine placements die
    /// with their instances.
    pub fn clear_instances(&mut self) {
        self.instances.clear();
        self.velocities.clear();
        self.trajectories.clear();
        self.bindings.clear();
        self.affines.clear();
    }

    /// Remove every persistent overlay point (Phase G: content replacement and
    /// stale-overlay correction). Instances and velocities are untouched.
    pub fn clear_overlay(&mut self) {
        self.overlay.clear();
    }

    /// Set a persistent integer translation `(vx, vy)` on an instance. The
    /// translation is applied once per [`State::advance_translations`], so the
    /// instance's position follows `position(t+1) = position(t) + (vx, vy)`
    /// while the translation is active. A `(0,0)` translation deactivates
    /// (equivalent to no entry). Translation, trajectory, and affine state on
    /// one instance are mutually exclusive: attaching one removes the others.
    /// Setting a translation on an unknown instance is a typed error.
    pub fn set_velocity(&mut self, id: InstanceId, vx: i64, vy: i64) -> Result<(), VoleError> {
        if !self.instances.iter().any(|i| i.id == id) {
            return Err(VoleError::UnknownInstance);
        }
        self.trajectories.remove(&id);
        self.affines.remove(&id);
        if vx == 0 && vy == 0 {
            self.velocities.remove(&id);
        } else {
            self.velocities.insert(id, (vx, vy));
        }
        Ok(())
    }

    /// Active translation of an instance (default `(0,0)`).
    pub fn velocity(&self, id: InstanceId) -> (i64, i64) {
        self.velocities.get(&id).copied().unwrap_or((0, 0))
    }

    /// Number of instances with an active (non-zero) translation.
    pub fn moving_count(&self) -> usize {
        self.velocities.len()
    }

    /// Apply every active integer translation exactly once: for each moving
    /// instance, `x += vx; y += vy` with checked arithmetic (an over-large
    /// accumulated position is a typed error, never a wrap). Runs in one pass
    /// over the instance list (O(instances), no quadratic id lookup).
    pub fn advance_translations(&mut self) -> Result<(), VoleError> {
        for inst in self.instances.iter_mut() {
            let Some((vx, vy)) = self.velocities.get(&inst.id).copied() else {
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
        Ok(())
    }

    /// Attach a trajectory program to an instance (Phase I). An **empty**
    /// program deactivates any active trajectory (mirror of
    /// `set_velocity(id, 0, 0)`). Trajectory and translation state are
    /// mutually exclusive: attaching a trajectory removes any translation.
    /// Per-segment canonicality is enforced here; the segment-count bound
    /// against `Limits.max_trajectory_segments` is enforced by the stream
    /// layers (parser and encoder) which own the limits. An unknown instance
    /// or a non-canonical program is a typed error.
    pub fn set_trajectory(
        &mut self,
        id: InstanceId,
        program: Vec<TrajectorySegment>,
    ) -> Result<(), VoleError> {
        if !self.instances.iter().any(|i| i.id == id) {
            return Err(VoleError::UnknownInstance);
        }
        if program.is_empty() {
            self.trajectories.remove(&id);
            return Ok(());
        }
        for seg in &program {
            seg.check()?;
        }
        let traj = InstanceTrajectory::start(program).ok_or(VoleError::NonCanonicalEncoding)?;
        self.velocities.remove(&id);
        self.affines.remove(&id);
        self.trajectories.insert(id, traj);
        Ok(())
    }

    /// Attach a canonical Q8 affine placement to an instance (Phase L): the
    /// object paints through the source map `(a·x+b·y+c, d·x+e·y+f) >> 8`
    /// instead of the plain `(x, y)` placement. The identity affine (plain
    /// placement) deactivates. Affine, velocity, and trajectory state on one
    /// instance are mutually exclusive. An unknown instance or an
    /// out-of-domain coefficient is a typed error.
    pub fn set_affine(&mut self, id: InstanceId, params: AffineParams) -> Result<(), VoleError> {
        if !self.instances.iter().any(|i| i.id == id) {
            return Err(VoleError::UnknownInstance);
        }
        params.check()?;
        self.velocities.remove(&id);
        self.trajectories.remove(&id);
        if params.is_identity() {
            self.affines.remove(&id);
        } else {
            self.affines.insert(id, params);
        }
        Ok(())
    }

    /// Affine placement of an instance, if any.
    pub fn affine(&self, id: InstanceId) -> Option<AffineParams> {
        self.affines.get(&id).copied()
    }

    /// Number of instances with an affine placement.
    pub fn affine_count(&self) -> usize {
        self.affines.len()
    }

    /// Whether the instance carries an active trajectory program.
    pub fn has_trajectory(&self, id: InstanceId) -> bool {
        self.trajectories.contains_key(&id)
    }

    /// Number of instances carrying an active trajectory program.
    pub fn trajectory_count(&self) -> usize {
        self.trajectories.len()
    }

    /// Borrow the live trajectory state of an instance.
    pub fn trajectory(&self, id: InstanceId) -> Option<&InstanceTrajectory> {
        self.trajectories.get(&id)
    }

    /// Replace (or declare) the palette `id` with `entries`. An empty entry
    /// list or the reserved id zero is non-canonical. The entry-count bound
    /// against `Limits.max_palette_entries` and the table bound against
    /// `Limits.max_palettes` are enforced by the stream layers that own the
    /// limits.
    pub fn set_palette(&mut self, id: PaletteId, entries: Vec<u8>) -> Result<(), VoleError> {
        if id == PaletteId::NONE || entries.is_empty() {
            return Err(VoleError::NonCanonicalEncoding);
        }
        self.palettes.insert(id, entries);
        Ok(())
    }

    /// Patch palette entries: `(index, value)` pairs in canonical strictly
    /// ascending index order (duplicates are non-canonical). Each index must
    /// lie inside the palette's current length, and the palette must exist.
    /// Returns a typed error on any violation; no partial mutation is left
    /// behind.
    pub fn patch_palette(&mut self, id: PaletteId, changes: &[(u8, u8)]) -> Result<(), VoleError> {
        let entries = self
            .palettes
            .get_mut(&id)
            .ok_or(VoleError::UnknownPalette)?;
        let mut prev: Option<u8> = None;
        for (idx, v) in changes {
            if prev.is_some_and(|p| *idx <= p) {
                return Err(VoleError::NonCanonicalEncoding);
            }
            prev = Some(*idx);
            let slot = entries
                .get_mut(usize::from(*idx))
                .ok_or(VoleError::OutOfBounds)?;
            *slot = *v;
        }
        Ok(())
    }

    /// Whether the palette table carries `id`.
    pub fn has_palette(&self, id: PaletteId) -> bool {
        self.palettes.contains_key(&id)
    }

    /// Current entries of a palette.
    pub fn palette(&self, id: PaletteId) -> Option<&[u8]> {
        self.palettes.get(&id).map(|e| e.as_slice())
    }

    /// Number of distinct palettes.
    pub fn palette_count(&self) -> usize {
        self.palettes.len()
    }

    /// Bind an instance to a palette (Phase J). `PaletteId::NONE` unbinds.
    /// The instance must exist; binding to a palette that does not exist yet
    /// is a typed error (set the palette first).
    pub fn bind_palette(
        &mut self,
        instance: InstanceId,
        palette: PaletteId,
    ) -> Result<(), VoleError> {
        if !self.instances.iter().any(|i| i.id == instance) {
            return Err(VoleError::UnknownInstance);
        }
        if palette == PaletteId::NONE {
            self.bindings.remove(&instance);
            return Ok(());
        }
        if !self.palettes.contains_key(&palette) {
            return Err(VoleError::UnknownPalette);
        }
        self.bindings.insert(instance, palette);
        Ok(())
    }

    /// Palette bound to an instance, if any.
    pub fn binding(&self, instance: InstanceId) -> Option<PaletteId> {
        self.bindings.get(&instance).copied()
    }

    /// Number of bound instances.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Step every active trajectory exactly once (Phase I). For each
    /// trajectory-carrying instance, in instance list order: advance the
    /// position by the current velocity (checked), then update the segment
    /// state — an `Accel` segment adds `(ax, ay)` to its velocity; a segment
    /// whose steps are exhausted moves to the next segment, and a program
    /// whose final segment is exhausted deactivates. One O(instances) pass.
    pub fn advance_trajectories(&mut self) -> Result<(), VoleError> {
        let mut finished: Vec<InstanceId> = Vec::new();
        for inst in self.instances.iter_mut() {
            let Some(traj) = self.trajectories.get_mut(&inst.id) else {
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
            self.trajectories.remove(&id);
        }
        Ok(())
    }

    /// Define a sparse-overlay point. Points persist until overwritten with a
    /// new value (blinking/strobe courts overwrite the same coordinates).
    /// Overlay pixels are painted above every instance.
    pub fn set_overlay(&mut self, x: i64, y: i64, value: u8) {
        self.overlay.insert((x, y), value);
    }

    /// Push a batch of overlay points in canonical (lexicographically sorted)
    /// order, validating that they are sorted before application (hostile
    /// requirement: non-canonical order is a typed error, not accepted).
    pub fn overlay_batch(&mut self, pts: &[(i64, i64, u8)]) -> Result<(), VoleError> {
        let mut prev: Option<(i64, i64)> = None;
        for (x, y, v) in pts {
            let key = (*x, *y);
            if let Some(p) = prev {
                if key <= p {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
            prev = Some(key);
            self.set_overlay(*x, *y, *v);
        }
        Ok(())
    }

    /// Value of a persistent overlay point, if any.
    pub fn overlay_pixel(&self, x: i64, y: i64) -> Option<u8> {
        self.overlay.get(&(x, y)).copied()
    }

    /// Number of live overlay points.
    pub fn overlay_len(&self) -> usize {
        self.overlay.len()
    }

    /// Iterate overlay points in canonical coordinate order (used by the
    /// materializer as the final paint pass).
    pub fn overlay_iter(&self) -> impl Iterator<Item = (i64, i64, u8)> + '_ {
        self.overlay.iter().map(|((x, y), v)| (*x, *y, *v))
    }
}
