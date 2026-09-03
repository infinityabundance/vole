//! VOLE — Video Object Layer Engine.
//!
//! VOLE is a procedural video storage, transport, inverse-proceduralization,
//! and materialization engine. Raster frames are a *view* over persistent,
//! bounded, deterministic procedural state; they are not the primary stored
//! ontology of the video (see `docs/architecture.md` and the prior-art paper
//! in `research/vole_procedural_video_prior_art.md`).
//!
//! The normative crate contains the representation, the manual wire format,
//! the deterministic materializer, the typed-limits execution envelope, and
//! (over later phases) the encoder, reverse-proceduralizer, search governor,
//! and store abstraction. DSFB (search intelligence) and EntropyFS
//! (persistence substrate) are deliberately *not* normative dependencies and
//! are not part of this crate.
//!
//! Safety posture: the normative implementation forbids unsafe code. A
//! conforming decoder treats every input stream as hostile: it returns typed,
//! deterministic [`VoleError`]s under [`crate::limits::Limits`] and never
//! panics, exhausts the stack, or grows without bound.

#![forbid(unsafe_code)]
// (missing_docs lint deliberately left to per-item #[doc] hygiene in later
// hardening passes; enabling warn(missing_docs) here would block incremental
// courts until the public surfacing stabilizes.)

pub mod checked;
pub mod checkpoint;
pub mod decoder;
pub mod demo;
pub mod dsfb;
pub mod encoder;
pub mod error;
pub mod format;
pub mod identity;
pub mod integr;
pub mod inverse;
pub mod limits;
pub mod materialize;
pub mod object;
pub mod pixel;
pub mod rans;
pub mod state;
pub mod time;
pub mod transition;
pub mod universe;
pub mod view;

pub use crate::error::VoleError;
pub use crate::limits::Limits;
