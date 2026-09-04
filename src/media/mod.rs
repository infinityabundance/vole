//! Canonical video media domain — Phase V.1.1/V.1.2/V.1.3 (V.1 video
//! programme, contract `docs/phase-v1-video-architecture.md` §2.1–§2.6).
//!
//! V.1.1 established the **in-memory media interpretation layer** that the
//! multiplane core, the import bridge (V.1.3), and every later subphase build
//! on: exact, validated, integer-only description of:
//!
//! * **rational media time** ([`time`]) — [`TimeBase`], [`Pts`], [`Duration`]:
//!   signed timestamps, exact checked rescaling, no floating point anywhere;
//! * **component-plane model** ([`layout`], [`plane`]) — named components,
//!   the canonical pixel-layout registry, exact subsampling geometry
//!   (ceil rules, courted on odd dimensions), bit depths 1..=16 with
//!   `u8`/`u16` tight canonical storage and active-bit validation;
//! * **color semantics** ([`color`]) — primaries, transfer characteristic,
//!   matrix, range, chroma sample location, and HDR static metadata, with
//!   `UNSPECIFIED` meaning *unspecified* — never a guessed truth;
//! * **picture interpretation** ([`meta`]) — orientation, sample aspect
//!   ratio, field structure, and the bounded typed/opaque side-data registry;
//! * **epochs and canonical observations** ([`epoch`]) — every observation
//!   binds to an epoch that declares the full media interpretation; timeline
//!   ordering is presentation order with strictly increasing rational PTS.
//!
//! V.1.2 adds the **multi-plane procedural core** over that domain:
//!
//! * **sample-domain pictures** ([`picture`]) — the multi-plane canvas with
//!   depth-validated `u32`-domain access;
//! * **independent-plane programs** ([`core`]) — the v1 families (object
//!   table, instance painting, background, persistent overlay, interval
//!   replay, COPY/RESIDUAL canvas ops) generalized per plane, written as an
//!   independent implementation so the v1 Gray8 specialization court is a
//!   real oracle; replay semantics mirror v1 exactly (every interval renders
//!   the persistent state fresh and applies that interval's canvas ops over
//!   it — canvas ops never persist);
//! * **the exact raster-origin ingest floor** ([`ingest`]) — observations →
//!   an exact per-plane program (background / whole-plane RAW object /
//!   state-relative sparse residual / content replacement), proven
//!   sample-for-sample before return;
//! * **the v2 core wire** ([`wire`]) — the frozen format-v2 core container
//!   (header + media descriptor + plane blocks + BLAKE3 trailer; normative
//!   grammar in `docs/format-v2.md`), with typed hostile-safe parsing.
//!
//! V.1.3 adds the **foreign ingest bridge** ([`bridge`]) over that core:
//! ffprobe manifests (evidence), bounded ffmpeg subprocess runs (software
//! decode, retained pixel format, no silent transforms), the independent
//! framehash SHA-256 oracle, the narrow NUT pipe reader (exact PTS + tight
//! rawvideo payloads), and the reversible source-layout canonicalizer —
//! culminating in [`import_video`], which verifies every canonical
//! observation against the oracle before returning a validated
//! [`CanonicalVideo`]. NUT/FFmpeg are non-normative and never appear inside
//! `.vole`.
//!
//! The separation of the two clocks is deliberate and normative (contract
//! §2.2): the procedural state machine keeps v1's explicit-interval semantics
//! (`crate::time::Interval`); the media timeline in this module is a
//! *declarative mapping* from presentation time to observation — it never
//! changes what the state machine computes. Synthetic canonical vectors and
//! foreign-media imports (V.1.3, via the bridge) both feed that domain.

pub mod bridge;
pub mod color;
pub mod core;
pub mod encode;
pub mod epoch;
pub mod gen;
pub mod ingest;
pub mod layout;
pub mod meta;
pub mod picture;
pub mod plane;
pub mod time;
pub mod wire;

pub use bridge::{
    import_video, verify_frames, ImportChecks, ImportLimits, ImportOptions, VerifiedImport,
};
pub use color::{
    ChromaLocation, ColorDescription, ColorPrimaries, ColorRange, ContentLightLevel, HdrMetadata,
    MasteringDisplay, MatrixCoefficients, TransferCharacteristic,
};
pub use core::{
    encode_plane_residual, encode_plane_transform_block, materialize_plane, MultiPlaneProgram,
    PlaneContent, PlaneInstance, PlaneInstanceId, PlaneMotion, PlaneObject, PlaneObjectId, PlaneOp,
    PlanePaletteId, PlaneProgram, PlaneTrajectoryState,
};
pub use encode::{
    encode_pictures_families, EncodeReport, FamilyTotals, FAMILY_COPY, FAMILY_EXACT, FAMILY_FILL,
    FAMILY_GENERATOR, FAMILY_PALETTE, FAMILY_RAW, FAMILY_REGIONS, FAMILY_SPARSE, FAMILY_TRANSFORM,
    FAMILY_TRANSLATION, FAMILY_UNCHANGED,
};
pub use epoch::{CanonicalVideo, CanonicalVideoObservation, EpochId, PlaneTemplate, VideoEpoch};
pub use ingest::{encode_pictures_exact, ramp_picture, uniform_picture};
pub use layout::{Component, PackedSourceLayout, PixelLayout};
pub use meta::{
    FieldStructure, Orientation, SampleAspectRatio, VisualSideData, VisualSideDataKind,
};
pub use picture::Picture;
pub use plane::{BitDepth, Plane, PlaneData, PlaneStorage};
pub use time::{Duration, Pts, TimeBase};
pub use wire::{parse_multiplane, write_multiplane, V2_FEATURES};
