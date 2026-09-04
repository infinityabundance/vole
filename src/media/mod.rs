//! Canonical video media domain — Phase V.1.1 (V.1 video programme, contract
//! `docs/phase-v1-video-architecture.md` §2.2–§2.5).
//!
//! This module is the **in-memory media interpretation layer** that V.1.2's
//! multiplane core, V.1.3's import bridge, and every later subphase build on.
//! It deliberately holds **no wire grammar** (the v2 byte format is frozen at
//! the end of V.1.2 in `docs/format-v2.md`) and **no foreign import** (V.1.3).
//! V.1.1's contract is the exact, validated, integer-only description of:
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
//! The separation of the two clocks is deliberate and normative (contract
//! §2.2): the procedural state machine keeps v1's explicit-interval semantics
//! (`crate::time::Interval`); the media timeline in this module is a
//! *declarative mapping* from presentation time to observation — it never
//! changes what the state machine computes. The V.1.1 scope is synthetic
//! canonical vectors only; nothing here reads a file.

pub mod color;
pub mod epoch;
pub mod layout;
pub mod meta;
pub mod plane;
pub mod time;

pub use color::{
    ChromaLocation, ColorDescription, ColorPrimaries, ColorRange, ContentLightLevel, HdrMetadata,
    MasteringDisplay, MatrixCoefficients, TransferCharacteristic,
};
pub use epoch::{CanonicalVideo, CanonicalVideoObservation, EpochId, PlaneTemplate, VideoEpoch};
pub use layout::{Component, PackedSourceLayout, PixelLayout};
pub use meta::{
    FieldStructure, Orientation, SampleAspectRatio, VisualSideData, VisualSideDataKind,
};
pub use plane::{BitDepth, Plane, PlaneData, PlaneStorage};
pub use time::{Duration, Pts, TimeBase};
