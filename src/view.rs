//! The view abstraction: a materialization target expressed as a typed set of
//! parameters bound against the state's canonical canvas.
//!
//! Phase A exposes exactly the whole-frame view. The type is authored so that
//! tile / rectangle / scanline views can be added in a later phase without
//! reshuffling the materializer signature: views are values, and materialize
//! is a function of a `View` plus the state, not a family of bespoke
//! functions.

/// A requested rasterization target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// The canonical whole canvas as a Gray8 full frame.
    FullFrame,
    // Rect / Tile / Plane / Scale added in later phases.
}

impl View {
    /// The canonical view for stream playback.
    pub const CANONICAL: View = View::FullFrame;

    /// Returns `Ok(width, height)` future-proofing name; the canonical full
    /// frame is described entirely by the state canvas so no payload here.
    pub fn kind(self) -> ViewKind {
        match self {
            View::FullFrame => ViewKind::FullFrame,
        }
    }
}

/// View descriptor kind used by accounting/diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    /// Full canvas.
    FullFrame,
}
