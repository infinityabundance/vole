//! VOLE — Video Object Layer Engine.
//!
//! VOLE is a procedural video storage, transport, inverse-proceduralization,
//! and materialization engine. Raster frames are a *view* over persistent,
//! bounded, deterministic procedural state; they are not the primary stored
//! ontology of the video (see `docs/architecture.md` and the prior-art paper
//! in `research/vole_procedural_video_prior_art.md`).
//!
//! The normative crate contains the representation, the manual wire format,
//! the deterministic materializer, the typed-limits execution envelope, the
//! encoder, the reverse-proceduralizer, the search governor, and the
//! content-addressed object-store abstraction (`store`). DSFB (search
//! intelligence) is never a normative dependency; EntropyFS (persistence
//! substrate) is not part of the standalone build — an optional, default-OFF
//! `entropyfs-store` feature links the real entropyfs engine behind the same
//! `ObjectStore` abstraction (Phase P).
//!
//! Safety posture: the normative implementation forbids unsafe code. A
//! conforming decoder treats every input stream as hostile: it returns typed,
//! deterministic [`VoleError`]s under [`crate::limits::Limits`] and never
//! panics, exhausts the stack, or grows without bound.

#![forbid(unsafe_code)]
// (missing_docs lint deliberately left to per-item #[doc] hygiene in later
// hardening passes; enabling warn(missing_docs) here would block incremental
// courts until the public surfacing stabilizes.)

pub mod affine;
pub mod archive;
pub mod checked;
pub mod checkpoint;
pub mod collapse;
pub mod decoder;
pub mod demo;
pub mod dsfb;
pub mod encoder;
pub mod error;
pub mod format;
pub mod generator;
pub mod identity;
pub mod ingest;
pub mod integr;
pub mod inverse;
pub mod limits;
pub mod lossy;
pub mod materialize;
pub mod media;
pub mod object;
pub mod optimize;
pub mod partial;
pub mod pixel;
pub mod rans;
pub mod script;
pub mod state;
pub mod store;
pub mod time;
pub mod trajectory;
pub mod transform;
pub mod transition;
pub mod transport;
pub mod universe;
pub mod view;

pub use crate::error::VoleError;
pub use crate::limits::Limits;
