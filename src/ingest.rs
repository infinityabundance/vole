//! Phase Q — native procedural ingest API (master brief §39, §3.1).
//!
//! Applications that already possess procedural state — UI hierarchies, vector
//! animation, game/simulation state, motion graphics, dashboards,
//! deterministic scene composition — emit that state **directly** instead of
//! rendering to rasters and letting the inverse proceduralizer infer the
//! structure the source just destroyed. This module is that API: a typed
//! [`Ingest`] session over the same descriptors the normative encoder
//! consumes — immutable objects (fill / raster / palette-index / generator),
//! pre-checkpoint palette tables, checkpoint instances (optionally
//! palette-bound), and a timeline of interval transitions at explicit absolute
//! times.
//!
//! The session is deliberately a thin, ergonomic layer over the normative
//! encoder: [`Ingest::finish`] serializes through
//! [`crate::encoder::encode_stream`] / [`crate::encoder::encode_palette_stream`],
//! which re-validate every descriptor with the same semantics the parser and
//! decoder use. Nothing here re-implements wire semantics, so an
//! `Ingest`-built stream is byte-canonical by construction.
//!
//! Time model: [`Ingest::at`] opens the interval group targeting absolute
//! frame `t` (`t ≥ 1`; a repeat of the current `t` appends to the same group,
//! an earlier `t` is rejected). Transition helpers append into the open
//! group. `finish()` yields `intervals + 1` materialized frames, exactly as
//! the standalone decoder does.
//!
//! The direct-ingest vs raster-origin comparison (the §55 native-preservation
//! court) is measured in `tests/phase_q.rs` and `examples/ingest_proof.rs`:
//! the same authored state is (A) ingested directly and (B) rasterized then
//! re-proceduralized by the inverse encoder; both must reproduce the same
//! canonical raster sequence byte-for-byte, and the flattening tax of B over
//! A is measured, never assumed.

use crate::{
    affine::AffineParams,
    error::VoleError,
    generator::Generator,
    object::{Object, ObjectId},
    state::{Instance, InstanceId, PaletteId},
    trajectory::TrajectorySegment,
    transition::Transition,
};

/// A native procedural ingest session: declared objects/palettes/instances plus
/// an interval timeline, finished into a canonical standalone `.vole` stream.
#[derive(Debug, Clone)]
pub struct Ingest {
    width: u32,
    height: u32,
    background: u8,
    objects: Vec<(u32, Object)>,
    palettes: Vec<(u32, Vec<u8>)>,
    instances: Vec<(Instance, Option<PaletteId>)>,
    timeline: Vec<(u64, Vec<Transition>)>,
    /// Highest time opened so far.
    last_t: u64,
    /// Whether a transition may be appended right now (an interval is open).
    open: bool,
}

impl Ingest {
    /// Begin a session over a `width x height` Gray8 canvas.
    pub fn new(width: u32, height: u32) -> Self {
        Ingest {
            width,
            height,
            background: 0,
            objects: Vec::new(),
            palettes: Vec::new(),
            instances: Vec::new(),
            timeline: Vec::new(),
            last_t: 0,
            open: false,
        }
    }

    /// Canvas width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Canvas height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Background sample painted below every instance at materialization.
    pub fn background(&mut self, value: u8) -> &mut Self {
        self.background = value;
        self
    }

    // -- Objects ----------------------------------------------------------

    /// Declare an immutable object by id. Duplicate ids are rejected.
    pub fn declare_object(&mut self, id: u32, object: Object) -> Result<(), VoleError> {
        if self.objects.iter().any(|(i, _)| *i == id) {
            return Err(VoleError::DuplicateId);
        }
        self.objects.push((id, object));
        Ok(())
    }

    /// Declare a uniform-fill object.
    pub fn declare_fill(&mut self, id: u32, w: u32, h: u32, value: u8) -> Result<(), VoleError> {
        self.declare_object(id, Object::fill(w, h, value)?)
    }

    /// Declare a literal Gray8 raster object (`samples.len() == w*h`).
    pub fn declare_raster(
        &mut self,
        id: u32,
        w: u32,
        h: u32,
        samples: Vec<u8>,
    ) -> Result<(), VoleError> {
        self.declare_object(id, Object::raster(w, h, samples)?)
    }

    /// Declare a palette-index plane (`indices.len() == w*h`; renders through
    /// the palette bound to the painting instance).
    pub fn declare_index(
        &mut self,
        id: u32,
        w: u32,
        h: u32,
        indices: Vec<u8>,
    ) -> Result<(), VoleError> {
        self.declare_object(id, Object::index_raster(w, h, indices)?)
    }

    /// Declare an object whose content is a bounded procedural program
    /// (samples are computed at materialization, never stored).
    pub fn declare_generator(
        &mut self,
        id: u32,
        w: u32,
        h: u32,
        gen: Generator,
    ) -> Result<(), VoleError> {
        self.declare_object(id, Object::procedural(w, h, gen)?)
    }

    /// Convenience: declare a wrap-gradient generator object.
    pub fn declare_gradient(
        &mut self,
        id: u32,
        w: u32,
        h: u32,
        base: u8,
        sx: i64,
        sy: i64,
    ) -> Result<(), VoleError> {
        let gen = Generator::Gradient { base, sx, sy };
        gen.check()?;
        self.declare_generator(id, w, h, gen)
    }

    // -- Palette tables ---------------------------------------------------

    /// Declare a pre-checkpoint palette table (id ≥ 1, entries non-empty and
    /// ≤ `max_palette_entries`). These entries are part of the checkpoint
    /// state; interval palette mutations use [`Ingest::set_palette`].
    pub fn declare_palette(&mut self, id: u32, entries: Vec<u8>) -> Result<(), VoleError> {
        if id == 0 || entries.is_empty() {
            return Err(VoleError::NonCanonicalEncoding);
        }
        if entries.len() as u64 > u64::from(crate::limits::Limits::default().max_palette_entries) {
            return Err(VoleError::DimensionTooLarge);
        }
        if self.palettes.iter().any(|(i, _)| *i == id) {
            return Err(VoleError::DuplicateId);
        }
        self.palettes.push((id, entries));
        Ok(())
    }

    // -- Checkpoint instances ---------------------------------------------

    /// Add a checkpoint instance in paint order. The referenced object must be
    /// declared by the time [`Ingest::finish`] runs.
    pub fn instance(&mut self, iid: u32, oid: u32, x: i64, y: i64) -> Result<(), VoleError> {
        self.instance_inner(iid, oid, x, y, None)
    }

    /// Add a checkpoint instance bound to a palette (for palette-index
    /// content; the palette must be declared by finish time).
    pub fn instance_binding(
        &mut self,
        iid: u32,
        oid: u32,
        x: i64,
        y: i64,
        palette: u32,
    ) -> Result<(), VoleError> {
        self.instance_inner(iid, oid, x, y, Some(PaletteId(palette)))
    }

    fn instance_inner(
        &mut self,
        iid: u32,
        oid: u32,
        x: i64,
        y: i64,
        palette: Option<PaletteId>,
    ) -> Result<(), VoleError> {
        coord_guard(x)?;
        coord_guard(y)?;
        if self.instances.iter().any(|(i, _)| i.id.0 == iid) {
            return Err(VoleError::DuplicateId);
        }
        self.instances.push((
            Instance {
                id: InstanceId(iid),
                object_id: ObjectId(oid),
                x,
                y,
            },
            palette,
        ));
        Ok(())
    }

    // -- Timeline ---------------------------------------------------------

    /// Open (or continue) the interval group targeting absolute frame `t`.
    /// `t` must be ≥ 1; a `t` before the highest already-opened time is
    /// rejected. Repeating the current `t` appends to the same group.
    pub fn at(&mut self, t: u64) -> Result<(), VoleError> {
        if t == 0 {
            return Err(VoleError::NonConsecutiveInterval);
        }
        if let Some((lt, _)) = self.timeline.last() {
            if t < *lt {
                return Err(VoleError::NonConsecutiveInterval);
            }
            if t == *lt {
                self.open = true;
                return Ok(());
            }
        }
        self.timeline.push((t, Vec::new()));
        self.last_t = t;
        self.open = true;
        Ok(())
    }

    /// Append one transition into the open interval group. Requires a prior
    /// [`Ingest::at`].
    pub fn push(&mut self, tr: Transition) -> Result<(), VoleError> {
        let group = self
            .timeline
            .last_mut()
            .filter(|_| self.open)
            .ok_or(VoleError::InvalidStatePhase)?;
        group.1.push(tr);
        Ok(())
    }

    /// Instance transitions.
    pub fn create_instance(&mut self, iid: u32, oid: u32, x: i64, y: i64) -> Result<(), VoleError> {
        coord_guard(x)?;
        coord_guard(y)?;
        self.push(Transition::CreateInstance {
            id: InstanceId(iid),
            object: ObjectId(oid),
            x,
            y,
        })
    }

    /// Absolute position set (Phase-A op).
    pub fn set_position(&mut self, iid: u32, x: i64, y: i64) -> Result<(), VoleError> {
        coord_guard(x)?;
        coord_guard(y)?;
        self.push(Transition::SetPosition {
            id: InstanceId(iid),
            x,
            y,
        })
    }

    /// Persistent integer translation state (Phase E): the instance moves by
    /// `(vx, vy)` per [`Ingest::advance`].
    pub fn set_velocity(&mut self, iid: u32, vx: i64, vy: i64) -> Result<(), VoleError> {
        coord_guard(vx)?;
        coord_guard(vy)?;
        self.push(Transition::SetVelocity {
            id: InstanceId(iid),
            vx,
            vy,
        })
    }

    /// Apply one advance of every active translation (Phase E).
    pub fn advance(&mut self) -> Result<(), VoleError> {
        self.push(Transition::AdvanceTranslations)
    }

    /// Attach a bounded trajectory program (Phase I). Trajectory and
    /// translation state on one instance are mutually exclusive (normative).
    pub fn set_trajectory(
        &mut self,
        iid: u32,
        segments: Vec<TrajectorySegment>,
    ) -> Result<(), VoleError> {
        for seg in &segments {
            seg.check()?;
        }
        self.push(Transition::SetTrajectory {
            id: InstanceId(iid),
            segments,
        })
    }

    /// Apply one advance of every active trajectory program (Phase I).
    pub fn advance_trajectories(&mut self) -> Result<(), VoleError> {
        self.push(Transition::AdvanceTrajectories)
    }

    /// Attach a canonical Q8 affine placement (Phase L).
    pub fn set_affine(&mut self, iid: u32, params: AffineParams) -> Result<(), VoleError> {
        params.check()?;
        self.push(Transition::SetAffine {
            id: InstanceId(iid),
            params,
        })
    }

    /// Palette transitions.
    pub fn set_palette(&mut self, id: u32, entries: Vec<u8>) -> Result<(), VoleError> {
        if id == 0 || entries.is_empty() {
            return Err(VoleError::NonCanonicalEncoding);
        }
        self.push(Transition::SetPalette {
            id: PaletteId(id),
            entries,
        })
    }

    /// Patch palette entries (`(index, value)` in strictly ascending index
    /// order).
    pub fn patch_palette(&mut self, id: u32, changes: Vec<(u8, u8)>) -> Result<(), VoleError> {
        let mut prev: Option<u8> = None;
        for (idx, _) in &changes {
            if prev.is_some_and(|p| *idx <= p) {
                return Err(VoleError::NonCanonicalEncoding);
            }
            prev = Some(*idx);
        }
        self.push(Transition::PatchPalette {
            id: PaletteId(id),
            changes,
        })
    }

    /// Bind an instance to a palette (`palette == 0` unbinds).
    pub fn bind_palette(&mut self, iid: u32, palette: u32) -> Result<(), VoleError> {
        self.push(Transition::BindPalette {
            instance: InstanceId(iid),
            palette: PaletteId(palette),
        })
    }

    /// Canvas-frame ops (applied after state transitions of the interval).
    pub fn copy_rect(
        &mut self,
        src_x: i64,
        src_y: i64,
        width: u32,
        height: u32,
        dst_x: i64,
        dst_y: i64,
    ) -> Result<(), VoleError> {
        copy_guard(src_x, src_y, width, height, dst_x, dst_y)?;
        self.push(Transition::CopyRect {
            src_x,
            src_y,
            width,
            height,
            dst_x,
            dst_y,
        })
    }

    /// MOVE_RECT with canonical snapshot-copy + overlap semantics (Phase D).
    pub fn move_rect(
        &mut self,
        src_x: i64,
        src_y: i64,
        width: u32,
        height: u32,
        dst_x: i64,
        dst_y: i64,
    ) -> Result<(), VoleError> {
        copy_guard(src_x, src_y, width, height, dst_x, dst_y)?;
        self.push(Transition::MoveRect {
            src_x,
            src_y,
            width,
            height,
            dst_x,
            dst_y,
        })
    }

    /// One-shot residual for this frame (a raw/rANS/transform block, produced
    /// by the rANS layer or a caller; see `docs/phase-f.md`/`docs/phase-m.md`).
    pub fn residual(&mut self, block: Vec<u8>) -> Result<(), VoleError> {
        self.push(Transition::Residual { block })
    }

    /// Persistent sparse overlay points in strictly ascending order (Phase C).
    pub fn patch_sparse(&mut self, points: Vec<(i64, i64, u8)>) -> Result<(), VoleError> {
        let mut prev: Option<(i64, i64)> = None;
        for (x, y, _) in &points {
            coord_guard(*x)?;
            coord_guard(*y)?;
            let key = (*x, *y);
            if prev.is_some_and(|p| key <= p) {
                return Err(VoleError::NonCanonicalEncoding);
            }
            prev = Some(key);
        }
        self.push(Transition::PatchSparse { points })
    }

    /// Content-replacement clears (Phase G).
    pub fn clear_instances(&mut self) -> Result<(), VoleError> {
        self.push(Transition::ClearInstances)
    }

    /// Content-replacement clears (Phase G).
    pub fn clear_overlay(&mut self) -> Result<(), VoleError> {
        self.push(Transition::ClearOverlay)
    }

    // -- Finalize ---------------------------------------------------------

    /// Serialize the session into a canonical standalone `.vole` stream. Runs
    /// the full normative encoder validation (duplicate ids, unknown object /
    /// instance / palette references, interval ordering, budgets), so API
    /// misuse surfaces as typed errors before any bytes are produced.
    pub fn finish(self) -> Result<Vec<u8>, VoleError> {
        // Geometry is guarded at the stream layers; a session over a zero or
        // over-limit canvas is refused here with the same typed error.
        crate::limits::Limits::default().check_canvas(self.width, self.height)?;
        let palette_used =
            !self.palettes.is_empty() || self.instances.iter().any(|(_, b)| b.is_some());
        if palette_used {
            crate::encoder::encode_palette_stream(
                self.width,
                self.height,
                self.background,
                &self.objects,
                &self.palettes,
                &self.instances,
                &self.timeline,
            )
        } else {
            let instances: Vec<Instance> =
                self.instances.into_iter().map(|(inst, _)| inst).collect();
            crate::encoder::encode_stream(
                self.width,
                self.height,
                self.background,
                &self.objects,
                &instances,
                &self.timeline,
            )
        }
    }

    /// Number of declared objects.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Declared object ids (declaration order).
    pub fn object_ids(&self) -> Vec<u32> {
        self.objects.iter().map(|(i, _)| *i).collect()
    }

    /// Number of declared palette tables.
    pub fn palette_count(&self) -> usize {
        self.palettes.len()
    }

    /// Number of checkpoint instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Number of interval groups opened.
    pub fn interval_count(&self) -> usize {
        self.timeline.len()
    }
}

/// Canonical signed-domain guard for positions and deltas.
fn coord_guard(v: i64) -> Result<(), VoleError> {
    if v.abs() > crate::format::MAX_COORD {
        Err(VoleError::NonCanonicalEncoding)
    } else {
        Ok(())
    }
}

/// Bounds/domain guard for the copy-family geometry (mirrors the encoder's).
#[allow(clippy::too_many_arguments)]
fn copy_guard(
    src_x: i64,
    src_y: i64,
    width: u32,
    height: u32,
    dst_x: i64,
    dst_y: i64,
) -> Result<(), VoleError> {
    coord_guard(src_x)?;
    coord_guard(src_y)?;
    coord_guard(dst_x)?;
    coord_guard(dst_y)?;
    if width == 0 || height == 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    if u64::from(width) * u64::from(height) > crate::limits::Limits::default().max_copy_area {
        return Err(VoleError::MaterializationBudgetExceeded);
    }
    Ok(())
}
