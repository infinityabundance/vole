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
        }
    }

    /// A short, machine-stable label (used by format tags and evidence).
    pub fn tag(&self) -> &'static str {
        match self {
            Transition::DeclareObject(..) | Transition::DeclareFill { .. } => "declare_object",
            Transition::CreateInstance { .. } => "create_instance",
            Transition::SetPosition { .. } => "set_position",
        }
    }
}
