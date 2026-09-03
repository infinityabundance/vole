//! Versioned deterministic universe binding.
//!
//! The universe id names the exact set of normative algorithms, tables,
//! integer widths, rounding rules, generators, and limits a conforming decoder
//! must apply. Format v1 binds `UNIVERSE_V1`. A decoder receiving any other
//! universe id refuses to decode (`UnsupportedUniverse`) rather than guess at
//! semantics.

use crate::error::VoleError;

/// The universe version declared by format v1.
///
/// Layout is fixed width to make the binding canonical and inspectable.
pub const UNIVERSE_V1: u32 = 0x0000_0001;

/// Bind to the sole known universe.
pub fn bind(universe_id: u32) -> Result<UniverseBinding, VoleError> {
    if universe_id == UNIVERSE_V1 {
        Ok(UniverseBinding { id: UNIVERSE_V1 })
    } else {
        Err(VoleError::UnsupportedUniverse)
    }
}

/// A validated universe binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UniverseBinding {
    /// The accepted universe id.
    pub id: u32,
}
