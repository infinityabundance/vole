//! Pixel / frame model. Phase A supports exactly Gray8.
//!
//! Canonical raster semantics (see `docs/format-v1.md`):
//!
//! * one unsigned 8-bit luma sample per pixel (0 = darkest, 255 = brightest);
//! * rows are top-to-bottom; each row is exactly `width` samples tightly
//!   packed (stride == width), no padding;
//! * a canonical full frame for a `(width, height)` stream is exactly
//!   `width*height` bytes in that order;
//! * no palette, no alpha transfer function, no color conversion in v1.

use crate::{checked::Res, error::VoleError, limits::Limits};

/// Pixel format supported in format v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Gray8: single unsigned 8-bit luma plane.
    Gray8,
}

/// A canonical Gray8 raster buffer with declared geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    width: u32,
    height: u32,
    /// Exactly `width * height` samples, row-major, tightly packed.
    data: Vec<u8>,
}

impl Canvas {
    /// Allocate a `width x height` canvas initialized to `fill`.
    pub fn new(width: u32, height: u32, fill: u8, limits: &Limits) -> Res<Self> {
        limits.check_canvas(width, height)?;
        let n = byte_len(width, height)?;
        if n > usize::try_from(limits.max_canvas_bytes).unwrap_or(usize::MAX) {
            return Err(VoleError::DimensionTooLarge);
        }
        Ok(Self {
            width,
            height,
            data: vec![fill; n],
        })
    }

    /// Zero-filled canvas.
    pub fn zeroed(width: u32, height: u32, limits: &Limits) -> Res<Self> {
        Self::new(width, height, 0, limits)
    }

    /// Build directly from a row-major raster whose geometry must be nonzero
    /// and whose length must equal `width * height`.
    pub fn from_parts(width: u32, height: u32, data: Vec<u8>) -> Res<Self> {
        if width == 0 || height == 0 {
            return Err(VoleError::DimensionTooLarge);
        }
        let want = byte_len(width, height)?;
        if data.len() != want {
            return Err(VoleError::LengthMismatch);
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Sample count (== byte length for Gray8).
    pub fn sample_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Immutable raster.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Read sample `(x,y)`; bounds are the caller's contract (materialize
    /// clips first).
    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.data[self.idx(x, y)]
    }

    /// Sample-for-sample equality (geometry and bytes).
    pub fn exactly_matches(&self, other: &Canvas) -> bool {
        self.width == other.width && self.height == other.height && self.data == other.data
    }

    /// Reset the whole canvas to `colour`.
    pub fn fill_all(&mut self, colour: u8) {
        self.data.fill(colour);
    }

    /// Fill a clipped axis-aligned region.
    pub fn fill_rect_clipped(&mut self, colour: u8, x0: i64, y0: i64, x1: i64, y1: i64) {
        let cw = i64::from(self.width);
        let ch = i64::from(self.height);
        let x0 = x0.clamp(0, cw);
        let y0 = y0.clamp(0, ch);
        let x1 = x1.clamp(x0, cw);
        let y1 = y1.clamp(y0, ch);
        let w = usize::try_from(self.width).unwrap();
        for y in y0..y1 {
            let s = usize::try_from(y).unwrap() * w + usize::try_from(x0).unwrap();
            let n = usize::try_from(x1 - x0).unwrap();
            self.data[s..s + n].fill(colour);
        }
    }

    /// Canonical overwrite blit. Copies the tight row-major `src` (an
    /// `sw x sh` sample box) onto the canvas with its top-left corner at
    /// canvas coordinate `(dx, dy)`, clipping at the borders. Out-of-canvas
    /// regions are dropped. Used by the materializer for object instances and
    /// by the independent reference rasterizer used in the conformance court.
    pub fn blit(&mut self, src: &[u8], sw: u32, sh: u32, dx: i64, dy: i64) {
        debug_assert_eq!(src.len() as u64, u64::from(sw) * u64::from(sh));
        let cw = i64::from(self.width);
        let ch = i64::from(self.height);
        // Iterate destination rows in canvas range, mapping to source rows.
        let y0 = dy.max(0);
        let y1 = (dy + i64::from(sh)).min(ch);
        let x0 = dx.max(0);
        let x1 = (dx + i64::from(sw)).min(cw);
        if y0 >= y1 || x0 >= x1 {
            return;
        }
        let w = usize::try_from(self.width).unwrap();
        for cty in y0..y1 {
            let sy = (cty - dy) as usize; // >= 0 because cty >= dy
            let dst_row = usize::try_from(cty).unwrap();
            for ctox in x0..x1 {
                let sx = (ctox - dx) as usize;
                let src_val = src[sy * (sw as usize) + sx];
                self.data[dst_row * w + usize::try_from(ctox).unwrap()] = src_val;
            }
        }
    }

    fn idx(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.width as usize) + (x as usize)
    }

    /// Consume to parts.
    pub fn into_parts(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.data)
    }
}

fn byte_len(w: u32, h: u32) -> Res<usize> {
    usize::try_from(u64::from(w) * u64::from(h)).map_err(|_| VoleError::ArithmeticOverflow)
}
