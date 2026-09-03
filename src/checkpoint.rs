//! Checkpoints.
//!
//! A checkpoint is a bounded, self-sufficient snapshot of procedural state
//! ([`State`]) that lets a decoder (re)start from a known interval without
//! depending on any earlier transition. In v1 each stream carries exactly one
//! checkpoint anchoring interval 0; later phases add mid-stream checkpoints
//! with a dense/configurable cadence (`docs/transitions.md`).

use crate::state::State;

/// A phase-A checkpoint: an owned copy of a procedural state.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    /// The captured state.
    pub state: State,
}

impl Checkpoint {
    /// Capture the current `state` as a restart point.
    pub fn capture(state: &State) -> Self {
        Checkpoint {
            state: state.clone(),
        }
    }

    /// Consume the captured state (for restarting replay).
    pub fn into_state(self) -> State {
        self.state
    }

    /// Borrow the captured state.
    pub fn state(&self) -> &State {
        &self.state
    }
}
