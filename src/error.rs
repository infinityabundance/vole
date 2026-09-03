//! Typed, deterministic error surface for VOLE.
//!
//! A conforming decoder never panics. Every hostile-input condition resolves
//! to a [`VoleError`] that identifies the failing condition. The same input
//! always produces the same error: the type carries no timestamps, no
//! process-specific state, and no `io::ErrorKind`-style ambient coupling.

use core::fmt;

/// Typed error condition. Non-exhaustive so future phases may extend it
/// without breaking exhaustive matches carried by downstream code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VoleError {
    /// Byte source ended before the declared structure was fully consumed.
    Truncated,

    /// A declared length, width, height, count, or coordinate was not
    /// representable in the canonical integer domain of the operation.
    ArithmeticOverflow,

    /// The magic identifier did not match the VOLE v1 signature.
    BadMagic,

    /// The declared universe id is not known to this decoder.
    UnsupportedUniverse,

    /// The declared limit-profile is not supported by this decoder.
    UnsupportedLimitProfile,

    /// One or more upstream feature bits are set that this decoder does not
    /// implement. VOLE fails closed on unknown mandatory features.
    UnsupportedFeature,

    /// The stream declared dimensions that exceed [`crate::Limits`].
    DimensionTooLarge,

    /// An object with the given id was not found but was referenced.
    UnknownObject,

    /// An instance with the given id was not found but was referenced.
    UnknownInstance,

    /// A packet referenced an object id that conflicts with a distinct
    /// existing declaration (duplicate / contradictory declaration).
    ConflictingObjectId,

    /// An object or instance was re-declared against an already occupied id.
    DuplicateId,

    /// An object's declared geometry did not fit the referenced operation.
    ObjectGeometryMismatch,

    /// A transition targeted an interval that was not the immediate successor
    /// of the current decode position.
    NonConsecutiveInterval,

    /// A checkpoint would reset decoding onto a state out of the bounded
    /// replay envelope (too many dependent transitions, deep dependency,
    /// oversized checkpoint).
    CheckpointOutOfEnvelope,

    /// The canonical encoding was not parseable (non-canonical length,
    /// non-canonical option byte, etc.).
    NonCanonicalEncoding,

    /// The length prefix of a payload disagreed with how many bytes are
    /// available to be fully consumed by it.
    LengthMismatch,

    /// A write targeted the buffer past its declared bounds.
    OutOfBounds,

    /// The declared integrity hash did not match the recomputed digest.
    IntegrityMismatch,

    /// The materializer encounter satisfied a limit that bounds an individual
    /// procedural step (fill/object/instance work budget).
    MaterializationBudgetExceeded,

    /// An operation was refused because state is not currently in the
    /// expected phase (e.g., a transition after finalization).
    InvalidStatePhase,

    /// A value passed to a public API is out of the operator-defined limit.
    ApiConstraint(&'static str),
}

impl fmt::Display for VoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated => write!(f, "stream ended before declared structure was consumed"),
            Self::ArithmeticOverflow => {
                write!(f, "arithmetic overflow in canonical integer domain")
            }
            Self::BadMagic => write!(f, "magic bytes did not match the VOLE signature"),
            Self::UnsupportedUniverse => write!(f, "unsupported universe id"),
            Self::UnsupportedLimitProfile => write!(f, "unsupported limit profile"),
            Self::UnsupportedFeature => {
                write!(f, "stream requires an unsupported mandatory feature")
            }
            Self::DimensionTooLarge => write!(f, "declared dimension exceeds the active limits"),
            Self::UnknownObject => write!(f, "reference to an object id that is not declared"),
            Self::UnknownInstance => write!(f, "reference to an instance id that is not declared"),
            Self::ConflictingObjectId => {
                write!(f, "object id redeclared with conflicting identical bytes")
            }
            Self::DuplicateId => write!(f, "object or instance id already occupied"),
            Self::ObjectGeometryMismatch => {
                write!(f, "object geometry is incompatible with the operation")
            }
            Self::NonConsecutiveInterval => {
                write!(f, "transition interval is not the immediate successor")
            }
            Self::CheckpointOutOfEnvelope => {
                write!(f, "checkpoint exceeds the bounded decode envelope")
            }
            Self::NonCanonicalEncoding => write!(f, "non-canonical encoding in stream"),
            Self::LengthMismatch => write!(f, "declared payload length disagrees with content"),
            Self::OutOfBounds => write!(f, "operation wrote outside its declared bounds"),
            Self::IntegrityMismatch => write!(f, "declared integrity hash does not match digest"),
            Self::MaterializationBudgetExceeded => {
                write!(f, "procedural materialization step exceeded its bound")
            }
            Self::InvalidStatePhase => write!(f, "operation not valid in current stream phase"),
            Self::ApiConstraint(msg) => write!(f, "api constraint violated: {}", msg),
        }
    }
}

impl std::error::Error for VoleError {}
