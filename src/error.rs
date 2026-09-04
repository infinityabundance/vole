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

    /// A palette with the given id (or an instance's palette binding) was not
    /// found at materialization or bind time.
    UnknownPalette,

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

    /// An entropy decoder needed a renormalization byte but the payload ended
    /// (overread past the declared entropy stream).
    EntropyOverread,

    /// An entropy payload was structurally corrupt: an invalid model, a
    /// cumulative slot outside every symbol partition, or an arithmetic
    /// inconsistency that cannot be a canonical stream.
    EntropyCorrupt,

    /// The materializer encounter satisfied a limit that bounds an individual
    /// procedural step (fill/object/instance work budget).
    MaterializationBudgetExceeded,

    /// An operation was refused because state is not currently in the
    /// expected phase (e.g., a transition after finalization).
    InvalidStatePhase,

    /// A value passed to a public API is out of the operator-defined limit.
    ApiConstraint(&'static str),

    /// A stream declares one or more *external* objects (Phase P) but no
    /// [`crate::store::ObjectStore`] is bound. Such a stream is deliberately
    /// not standalone: its immutable object payloads live in a store and must
    /// be fetched through the store abstraction during parse.
    StoreRequired,

    /// An external object declaration referenced a content id that the bound
    /// store does not hold. The stream cannot be decoded until the object is
    /// published to the store.
    StoreObjectMissing,

    /// The store backend reported a failure (open/create/read/write/GC, or a
    /// mapped backend error class). The payload is a stable condition name;
    /// store errors never carry ambient OS state into the typed surface.
    StoreFailure(&'static str),

    /// The research-harness procedural script (§53 / Phase Q) is malformed.
    /// The payload is a stable condition name; script errors are typed and
    /// deterministic and never identify content that already wrote bytes.
    ScriptParse(&'static str),

    /// A transport frame arrived out of sequence: a packet was lost between
    /// the transmitter and this receiver. Recovery re-feeds from the gap
    /// (the receiver reports the expected sequence via its accessors).
    TransportGap,

    /// The transport framing layer rejected a frame (unknown kind, malformed
    /// length, non-canonical packet order, or a packet that contradicts the
    /// already-applied prefix). The payload is a stable condition name.
    TransportFormat(&'static str),

    // --- Phase V.1.1 canonical media domain (V.1 video programme) ---
    /// A rational media time base is degenerate (zero numerator/denominator),
    /// so no tick value has a defined duration.
    InvalidTimeBase,

    /// A time computation cannot be represented in the requested rational
    /// domain: a rescale is inexact (the source tick grid does not divide the
    /// target grid) or an intermediate product/offset overflows the canonical
    /// integer domain.
    TimeNotRepresentable,

    /// A plane/layout/epoch/observation geometry disagreement: wrong plane
    /// count, wrong plane dimensions for the declared subsampling, sample
    /// counts inconsistent with the geometry, or an observation that does not
    /// match its epoch's declared interpretation.
    GeometryMismatch,

    /// A canonical sample-domain violation: sample payload length does not
    /// equal the declared geometry's tight storage, a `u16` sample carries
    /// bits above its declared active depth, or storage width contradicts the
    /// declared bit depth.
    InvalidSamples,

    /// A pixel layout / component code is reserved or unknown. Fail closed:
    /// an unknown layout never gets a guessed interpretation.
    UnsupportedPixelLayout,

    /// An observation references an epoch the sequence does not declare, an
    /// epoch id is reused, or the timeline ordering contract is violated
    /// (presentation order requires strictly increasing PTS).
    EpochViolation,
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
            Self::UnknownPalette => {
                write!(
                    f,
                    "reference to a palette id or binding that is not declared"
                )
            }
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
            Self::EntropyOverread => {
                write!(
                    f,
                    "entropy stream ended before renormalization could complete"
                )
            }
            Self::EntropyCorrupt => {
                write!(f, "entropy stream or model is structurally corrupt")
            }
            Self::MaterializationBudgetExceeded => {
                write!(f, "procedural materialization step exceeded its bound")
            }
            Self::InvalidStatePhase => write!(f, "operation not valid in current stream phase"),
            Self::ApiConstraint(msg) => write!(f, "api constraint violated: {}", msg),
            Self::StoreRequired => write!(
                f,
                "stream declares external objects but no object store is bound"
            ),
            Self::StoreObjectMissing => {
                write!(
                    f,
                    "external object content id is not present in the bound store"
                )
            }
            Self::StoreFailure(cond) => write!(f, "object store failure: {}", cond),
            Self::ScriptParse(cond) => write!(f, "procedural script error: {}", cond),
            Self::TransportGap => write!(
                f,
                "transport packet lost: receiver expects the next sequence frame"
            ),
            Self::TransportFormat(cond) => write!(f, "transport framing error: {}", cond),
            Self::InvalidTimeBase => write!(
                f,
                "rational time base is degenerate (zero numerator or denominator)"
            ),
            Self::TimeNotRepresentable => {
                write!(
                    f,
                    "time value is not exactly representable in the target domain"
                )
            }
            Self::GeometryMismatch => {
                write!(f, "plane/layout/epoch/observation geometry disagreement")
            }
            Self::InvalidSamples => write!(f, "canonical sample-domain violation"),
            Self::UnsupportedPixelLayout => {
                write!(f, "pixel layout or component code is reserved or unknown")
            }
            Self::EpochViolation => write!(f, "epoch or presentation-timeline contract violated"),
        }
    }
}

impl std::error::Error for VoleError {}
