//! Central, typed execution and allocation limits.
//!
//! Every untrusted length or operation must be validated against [`Limits`]
//! before it may influence an allocation, a loop bound, or a recursion. This
//! keeps the materializer and wire parser bounded on hostile input.

use crate::error::VoleError;

/// The one limit-profile declared in format v1. A decoder that sees a profile
/// it does not support must fail closed with `UnsupportedLimitProfile`. The
/// concrete numbers are small by design to keep Phase-A full-frame and object
/// materialization memory obviously bounded; later phases tighten per-profile.
pub const LIMIT_PROFILE_V1: u8 = 1;

/// Execution / allocation envelope for the phase-A decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum canonical full-view width.
    pub max_width: u32,
    /// Maximum canonical full-view height.
    pub max_height: u32,
    /// Maximum bytes that may be materialized as one Gray8 canvas.
    pub max_canvas_bytes: u64,
    /// Maximum distinct objects that may coexist in one state.
    pub max_objects: u32,
    /// Maximum bytes in one object raster (raw literal payload).
    pub max_object_bytes: u64,
    /// Maximum live instances in one state.
    pub max_instances: u32,
    /// Maximum transitions encoded within a single interval.
    pub max_transitions_per_interval: u32,
    /// Maximum transitions replayed forward from a checkpoint before the
    /// decode envelope is considered exhausted.
    pub max_transition_replay: u64,
    /// Cumulative work budget for persistent-translation advances during one
    /// parse/decode (count of per-instance position steps). A hostile stream
    /// cannot force unbounded replay work through many moving instances.
    pub max_transition_work: u64,
    /// Maximum number of intervals between the checkpoint base and the
    /// furthest materializable frame.
    pub max_checkpoint_distance: u64,
    /// Maximum distinct dependency hops a reference may traverse.
    pub max_dependency_depth: u32,
    /// Maximum declared object index/width/height product already counted in
    /// `max_object_bytes`, kept here for symmetric read checks.
    pub max_copy_area: u64,
    /// Stream / file bytes the parser refuses to exceed.
    pub max_stream_bytes: u64,
    /// Maximum distinct persistent sparse-overlay points one state may carry
    /// (one point per canvas sample at the v1 profile). A hostile stream cannot
    /// grow the overlay without bound across many sparse patches.
    pub max_overlay_points: u64,
    /// Maximum *decoded* byte payload of one per-frame residual block (a
    /// residual may carry up to a whole-canvas sparse point list, whose point
    /// triplets are 9 bytes each). The wire block may exceed this only by the
    /// container envelope slack (see `RESIDUAL_WIRE_SLACK`).
    pub max_residual_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        // Phase-A profile. Enough to run the documented courts (a 1920x1080
        // canvas and object boxes of ~200x100 plus background) without ever
        // permitting an uncontrolled memory blow-up. All checked.
        Self {
            max_width: 1920,
            max_height: 1080,
            max_canvas_bytes: 1920 * 1080,
            max_objects: 65536,
            max_object_bytes: 1920 * 1080,
            max_instances: 1_048_576,
            max_transitions_per_interval: 1_000_000,
            max_transition_replay: 1_000_000,
            max_transition_work: 1 << 22,
            max_checkpoint_distance: 1_000_000,
            max_dependency_depth: 8,
            max_copy_area: 1920 * 1080,
            max_stream_bytes: 1 << 30,       // 1 GiB
            max_overlay_points: 1920 * 1080, // one persistent point per sample
            max_residual_bytes: 9 * (1920 * 1080) + RESIDUAL_WIRE_SLACK,
        }
    }
}

/// Envelope slack a residual wire block may exceed its decoded payload by:
/// `kind(1) + out_len(8)` container prefix for the RAW branch, or the 512-byte
/// inline model plus rANS prefix on the coded branch. Kept small and explicit.
pub(crate) const RESIDUAL_WIRE_SLACK: u64 = 1024;

impl Limits {
    /// Profile selector used by format v1 decoding.
    pub fn for_profile(profile: u8) -> Result<Self, VoleError> {
        match profile {
            LIMIT_PROFILE_V1 => Ok(Self::default()),
            _ => Err(VoleError::UnsupportedLimitProfile),
        }
    }

    /// Validate the canvas geometry once, exactly as the decoder does, so
    /// API callers share one canonical guard.
    pub fn check_canvas(&self, width: u32, height: u32) -> Result<(), VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::DimensionTooLarge);
        }
        if width > self.max_width || height > self.max_height {
            return Err(VoleError::DimensionTooLarge);
        }
        let samples = u64::from(width) * u64::from(height);
        if samples > self.max_canvas_bytes {
            return Err(VoleError::DimensionTooLarge);
        }
        Ok(())
    }

    /// Validate an object raster size.
    pub fn check_object(&self, bytes: u64) -> Result<(), VoleError> {
        if bytes > self.max_object_bytes {
            return Err(VoleError::DimensionTooLarge);
        }
        Ok(())
    }

    /// Validate a whole stream's byte length before parsing.
    pub fn check_stream_len(&self, bytes: u64) -> Result<(), VoleError> {
        if bytes > self.max_stream_bytes {
            return Err(VoleError::DimensionTooLarge);
        }
        Ok(())
    }

    /// Validate the persistent overlay point count after an overlay batch.
    pub fn check_overlay_points(&self, points: u64) -> Result<(), VoleError> {
        if points > self.max_overlay_points {
            return Err(VoleError::DimensionTooLarge);
        }
        Ok(())
    }

    /// Validate a decoded residual payload length (before point interpretation).
    pub fn check_residual_bytes(&self, bytes: u64) -> Result<(), VoleError> {
        if bytes > self.max_residual_bytes {
            return Err(VoleError::DimensionTooLarge);
        }
        Ok(())
    }

    /// Number of intervals a stream may carry from its checkpoint base.
    pub fn check_interval_distance(&self, intervals: u64) -> Result<(), VoleError> {
        if intervals > self.max_checkpoint_distance {
            return Err(VoleError::CheckpointOutOfEnvelope);
        }
        Ok(())
    }
}
