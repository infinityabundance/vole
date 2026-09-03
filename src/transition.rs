//! Procedural transition operators applied forward to advance state.
//!
//! A transition remaps `G_t → G_{t+1}` inside the deterministic envelope
//! [`State`]. In phase A the operator language is deliberately small but the
//! *infrastructure* — typed transitions, incremental state application, and
//! replay — is real. Later phases extend the language (COPY_RECT, trajectories,
//! palette, generators) without changing the replay architecture.

use crate::{
    error::VoleError,
    object::{Object, ObjectId},
    state::{InstanceId, State},
};

/// One procedural transition step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// Declare an immutable object into the state at this interval.
    DeclareObject(ObjectId, Object),
    /// Declare a fill object (convenience: same as DeclareObject(fill)).
    DeclareFill {
        id: ObjectId,
        width: u32,
        height: u32,
        value: u8,
    },
    /// Instantiate an object, appending a draw instance in paint order.
    CreateInstance {
        id: InstanceId,
        object: ObjectId,
        x: i64,
        y: i64,
    },
    /// Move an instance to an absolute canvas position.
    SetPosition { id: InstanceId, x: i64, y: i64 },
    /// Set a persistent integer translation on an instance (Phase E).
    SetVelocity { id: InstanceId, vx: i64, vy: i64 },
    /// Apply every active integer translation once (Phase E).
    AdvanceTranslations,
    /// Sparse overlay patch: authoritative pixel set above all instances.
    /// Points must be canonical sorted; each applied pixel persists until
    /// overwritten by a later sparse patch for that coordinate (Phase C).
    PatchSparse { points: Vec<(i64, i64, u8)> },
    /// 2D copy of a canvas rectangle from the **immediately previous decoded
    /// frame** onto this frame's base (Phase D). Snapshot-copy avoids overlap
    /// aliasing; rectangle is clipped to the canvas; area bounded by
    /// `Limits.max_copy_area`.
    CopyRect {
        /// Source top-left (previous frame).
        src_x: i64,
        src_y: i64,
        /// Source rectangle extent.
        width: u32,
        height: u32,
        /// Destination top-left in the current base frame.
        dst_x: i64,
        dst_y: i64,
    },
    /// 2D move of a rectangle from the previous frame onto this frame;
    /// MOVE_RECT in Phase D is CopyRect (the natural MoveRect mask semantics
    /// are documented as future work); validated identically on bounds.
    MoveRect {
        src_x: i64,
        src_y: i64,
        width: u32,
        height: u32,
        dst_x: i64,
        dst_y: i64,
    },
    /// Remove every live instance (Phase G full-content replacement). Instance
    /// ids are freed for reuse; objects, background, overlay untouched.
    ClearInstances,
    /// Remove every persistent overlay point (Phase G).
    ClearOverlay,
    /// Per-frame residual information (Phase G). The payload is a self-
    /// describing Phase-F block (`rans::encode_block`) whose decoded bytes are
    /// a canonical strict-sorted sparse point list `(x:i32, y:i32, v:u8)`.
    /// This is a **canvas op** (applied after materialization, in listed op
    /// order after any COPY_RECT/MOVE_RECT); it is one-shot for the frame it
    /// appears in and does not mutate persistent state — the residual algebra
    /// `F = M(state) ⊕_ρ R` closes the gap between the materialized base and
    /// the target observation without a persistent side effect.
    Residual { block: Vec<u8> },
}

impl Transition {
    /// Apply the transition to `state`, mutating it in place. Invalid
    /// references and geometric errors surface as typed errors; no partial
    /// mutation is left behind for a rejected transition.
    pub fn apply(&self, state: &mut State) -> Result<(), VoleError> {
        match self {
            Transition::DeclareObject(id, object) => state.declare_object(*id, object.clone()),
            Transition::DeclareFill {
                id,
                width,
                height,
                value,
            } => {
                let object = Object::fill(*width, *height, *value)?;
                state.declare_object(*id, object)
            }
            Transition::CreateInstance { id, object, x, y } => {
                state.create_instance(*id, *object, *x, *y)
            }
            Transition::SetPosition { id, x, y } => state.set_position(*id, *x, *y),
            Transition::SetVelocity { id, vx, vy } => state.set_velocity(*id, *vx, *vy),
            Transition::AdvanceTranslations => state.advance_translations(),
            Transition::PatchSparse { points } => state.overlay_batch(points),
            // Frame-referencing ops act on the decode canvas, not on the
            // painter State, so they are no-ops here. Their bounds geometry is
            // validated during parse (see format.rs CopyRect geometry checks)
            // and they are only interpreted by the sequential replayer in
            // src/decoder.rs / src/materialize::compositor.
            Transition::CopyRect { .. } | Transition::MoveRect { .. } => Ok(()),
            // Canvas ops (see module docs): no persistent-state effect.
            Transition::Residual { .. } => Ok(()),
            Transition::ClearInstances => {
                state.clear_instances();
                Ok(())
            }
            Transition::ClearOverlay => {
                state.clear_overlay();
                Ok(())
            }
        }
    }

    /// A short, machine-stable label (used by format tags and evidence).
    pub fn tag(&self) -> &'static str {
        match self {
            Transition::DeclareObject(..) | Transition::DeclareFill { .. } => "declare_object",
            Transition::CreateInstance { .. } => "create_instance",
            Transition::SetPosition { .. } => "set_position",
            Transition::SetVelocity { .. } => "set_velocity",
            Transition::AdvanceTranslations => "advance_translations",
            Transition::PatchSparse { .. } => "patch_sparse",
            Transition::CopyRect { .. } => "copy_rect",
            Transition::MoveRect { .. } => "move_rect",
            Transition::ClearInstances => "clear_instances",
            Transition::ClearOverlay => "clear_overlay",
            Transition::Residual { .. } => "residual",
        }
    }
}
