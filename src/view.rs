//! The view abstraction: a materialization target expressed as a typed set of
//! parameters bound against the state's canonical canvas.
//!
//! Raster frames are *views* of state. The canonical view is the whole canvas
//! ([`View::FullFrame`]); Phase S adds the first partial views — an arbitrary
//! axis-aligned sub-rectangle ([`View::Rect`]) and a tile of a canonical tile
//! grid ([`View::Tile`]). Views are values: materialization is a function of a
//! `View` plus the state/timeline, not a family of bespoke functions, and
//! every view is defined as "the exact samples a whole-frame decode would put
//! in the requested region" — a partial view never changes what a stream
//! *means*, it only samples less of it (see `docs/materialization.md`,
//! `docs/phase-s.md` for the audit-scope semantics of partial views).

use crate::error::VoleError;

/// A requested rasterization target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The canonical whole canvas as a Gray8 full frame.
    FullFrame,
    /// An axis-aligned sub-rectangle in frame coordinates. The rectangle may
    /// extend beyond the canvas: the materialized view is the intersection
    /// (with the view's own top-left as its origin), exactly as a region that
    /// crosses a canvas border clips. A rectangle with no intersection with
    /// the canvas cannot be materialized (typed error).
    Rect {
        /// Frame-space left of the requested rectangle (may be negative).
        x: i64,
        /// Frame-space top of the requested rectangle (may be negative).
        y: i64,
        /// Requested width in samples (≥ 1).
        width: u32,
        /// Requested height in samples (≥ 1).
        height: u32,
    },
    /// One tile of a canonical `tile_w × tile_h` tile grid anchored at the
    /// canvas origin: the view is the intersection of the tile at grid cell
    /// `(tile_x, tile_y)` with the canvas (border tiles clip). Equivalent to
    /// requesting the rectangle `(tile_x·tile_w, tile_y·tile_h, tile_w,
    /// tile_h)`.
    Tile {
        /// Tile grid column.
        tile_x: u32,
        /// Tile grid row.
        tile_y: u32,
        /// Tile width in samples (≥ 1).
        tile_w: u32,
        /// Tile height in samples (≥ 1).
        tile_h: u32,
    },
    // Plane / Scale / ScanlineRange added in later phases.
}

/// The in-canvas intersection of a view with its canvas: a validated,
/// nonzero, canvas-bounded rectangle in frame coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewBox {
    /// In-canvas left.
    pub x: u32,
    /// In-canvas top.
    pub y: u32,
    /// In-canvas width (≥ 1).
    pub width: u32,
    /// In-canvas height (≥ 1).
    pub height: u32,
}

impl ViewBox {
    /// Whether this box covers the whole canvas.
    pub fn is_full(&self, canvas_width: u32, canvas_height: u32) -> bool {
        self.x == 0 && self.y == 0 && self.width == canvas_width && self.height == canvas_height
    }

    /// Sample count of the box.
    pub fn sample_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

impl View {
    /// The canonical view for stream playback.
    pub const CANONICAL: View = View::FullFrame;

    /// Clip the view against a `canvas_width × canvas_height` canvas. Returns
    /// the in-canvas intersection, or `Ok(None)` when the view does not
    /// intersect the canvas at all (a degenerate request — materialization
    /// refuses it). A zero requested width/height is a typed
    /// [`VoleError::DimensionTooLarge`]. All arithmetic is checked against the
    /// canvas so hostile geometry cannot overflow.
    pub fn clip(self, canvas_width: u32, canvas_height: u32) -> Result<Option<ViewBox>, VoleError> {
        let (cx, cy, cw, ch) = (
            0i128,
            0i128,
            i128::from(canvas_width),
            i128::from(canvas_height),
        );
        let (x0, y0, x1, y1) = match self {
            View::FullFrame => (cx, cy, cw, ch),
            View::Rect {
                x,
                y,
                width,
                height,
            } => {
                if width == 0 || height == 0 {
                    return Err(VoleError::DimensionTooLarge);
                }
                let x = i128::from(x);
                let y = i128::from(y);
                let w = i128::from(width);
                let h = i128::from(height);
                (x, y, x + w, y + h)
            }
            View::Tile {
                tile_x,
                tile_y,
                tile_w,
                tile_h,
            } => {
                if tile_w == 0 || tile_h == 0 {
                    return Err(VoleError::DimensionTooLarge);
                }
                let x = i128::from(tile_x) * i128::from(tile_w);
                let y = i128::from(tile_y) * i128::from(tile_h);
                (x, y, x + i128::from(tile_w), y + i128::from(tile_h))
            }
        };
        let x0 = x0.max(cx).min(cw);
        let y0 = y0.max(cy).min(ch);
        let x1 = x1.max(cx).min(cw);
        let y1 = y1.max(cy).min(ch);
        if x0 >= x1 || y0 >= y1 {
            return Ok(None);
        }
        Ok(Some(ViewBox {
            x: x0 as u32,
            y: y0 as u32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        }))
    }

    /// Returns the view kind for accounting/diagnostics.
    pub fn kind(self) -> ViewKind {
        match self {
            View::FullFrame => ViewKind::FullFrame,
            View::Rect { .. } => ViewKind::Rect,
            View::Tile { .. } => ViewKind::Tile,
        }
    }
}

/// View descriptor kind used by accounting/diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    /// Full canvas.
    FullFrame,
    /// Arbitrary axis-aligned sub-rectangle.
    Rect,
    /// One tile of a canonical tile grid.
    Tile,
}
