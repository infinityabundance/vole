//! Phase L — bounded fixed-point 2D affine placement (normative mechanics).
//!
//! An **affine placement** maps every destination (canvas) pixel to a source
//! sample of the painted object:
//!
//! ```text
//! su = (a·x + b·y + c) >> 8        sv = (d·x + e·y + f) >> 8
//! ```
//!
//! where `a..f` are canonical Q8 fixed-point values (one source pixel =
//! 256 units; the `>> 8` of a signed integer is floor division, the canonical
//! rounding rule). The source sample is `(su, sv)`; when it lies inside the
//! object rectangle `[0, w) × [0, h)` the destination pixel is painted with
//! that sample (or with the fill value for a fill object), otherwise the
//! pixel shows the underlying canvas. There is **no floating point**
//! anywhere: every affine transform is exactly this integer rule, so the
//! materializer and an independent reference painter that evaluates the same
//! rule agree byte-for-byte, and a Q8 approximation of a continuous camera
//! move is closed exactly by the residual algebra (§22: `F = M(state) ⊕_ρ R`).
//!
//! The identity affine (`a = e = 256`, `b = d = 0`, `c = f = 0`) is exactly
//! the plain integer translation `(x, y)` mode and is never stored.
//! Whole-pixel translation, integer multiples of 90° rotation, and integer
//! zooms are *exact* in Q8 (their coefficients are integers); general
//! rotation/zoom/pan parameters are Q8 approximations whose exactness gap is
//! closed by residuals.

use crate::error::VoleError;

/// Fixed-point fractional bits (Q8): one source pixel = 256 units.
pub const AFFINE_SHIFT: u32 = 8;
/// `1 << AFFINE_SHIFT`.
pub const AFFINE_SCALE: i64 = 1 << AFFINE_SHIFT;

/// Canonical signed-domain bound for affine coefficients (mirrors the wire
/// `i32` domain guard of every other signed literal in format v1).
pub const MAX_AFFINE_COEFF: i64 = 1 << 24;

/// Canonical fixed-point 2D affine placement parameters.
///
/// Destination pixel `(x, y)` samples the object at
/// `((a·x + b·y + c) >> 8, (d·x + e·y + f) >> 8)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AffineParams {
    /// Source-x coefficient of the destination x (Q8).
    pub a: i64,
    /// Source-x coefficient of the destination y (Q8).
    pub b: i64,
    /// Source-x translation (Q8).
    pub c: i64,
    /// Source-y coefficient of the destination x (Q8).
    pub d: i64,
    /// Source-y coefficient of the destination y (Q8).
    pub e: i64,
    /// Source-y translation (Q8).
    pub f: i64,
}

impl AffineParams {
    /// The identity affine (plain integer placement).
    pub const IDENTITY: AffineParams = AffineParams {
        a: AFFINE_SCALE,
        b: 0,
        c: 0,
        d: 0,
        e: AFFINE_SCALE,
        f: 0,
    };

    /// Whether this is the identity (plain `(x, y)` placement).
    pub fn is_identity(&self) -> bool {
        *self == AffineParams::IDENTITY
    }

    /// Canonical-form check: every coefficient must fit the wire domain
    /// `±2^24`. Out-of-domain coefficients are a typed error, never a wrap.
    pub fn check(&self) -> Result<(), VoleError> {
        for v in [self.a, self.b, self.c, self.d, self.e, self.f] {
            if v.abs() > MAX_AFFINE_COEFF {
                return Err(VoleError::NonCanonicalEncoding);
            }
        }
        Ok(())
    }

    /// Serialized byte length of one `SetAffine` transition payload:
    /// `tag(1) + iid(4) + 6 × coeff(4)`.
    pub fn wire_bytes(&self) -> u64 {
        29
    }

    /// The source sample `(su, sv)` for destination pixel `(x, y)` (checked;
    /// an overflowing accumulation is a typed error). `None` signals an
    /// arithmetic overflow — callers treat it as an error, never a wrap.
    pub fn source(&self, x: i64, y: i64) -> Option<(i64, i64)> {
        let nu = self
            .a
            .checked_mul(x)?
            .checked_add(self.b.checked_mul(y)?)?
            .checked_add(self.c)?;
        let nv = self
            .d
            .checked_mul(x)?
            .checked_add(self.e.checked_mul(y)?)?
            .checked_add(self.f)?;
        // Signed arithmetic shift = floor division, the canonical rounding.
        Some((nu >> AFFINE_SHIFT, nv >> AFFINE_SHIFT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_maps_pixel_to_pixel() {
        let p = AffineParams::IDENTITY;
        for (x, y) in [(0i64, 0i64), (5, 9), (100, 200), (-3, -7)] {
            assert_eq!(p.source(x, y), Some((x, y)));
        }
        assert!(p.is_identity());
    }

    #[test]
    fn quarter_turn_and_integer_zoom_are_exact_integer_maps() {
        // A quarter turn about the origin: dest (x, y) samples source
        // (y, -x): su = (0·x + 256·y) >> 8 = y, sv = (-256·x) >> 8 = -x.
        let rot = AffineParams {
            a: 0,
            b: AFFINE_SCALE,
            c: 0,
            d: -AFFINE_SCALE,
            e: 0,
            f: 0,
        };
        assert_eq!(rot.source(3, 7), Some((7, -3)));

        // 2x integer zoom about the origin: dest samples source (x/2, y/2).
        let zoom = AffineParams {
            a: AFFINE_SCALE / 2,
            b: 0,
            c: 0,
            d: 0,
            e: AFFINE_SCALE / 2,
            f: 0,
        };
        assert_eq!(zoom.source(10, 6), Some((5, 3)));
        // Floor semantics for odd coordinates.
        assert_eq!(zoom.source(11, 5), Some((5, 2)));
    }

    #[test]
    fn canonical_checks_reject_out_of_domain_coefficients() {
        let bad = AffineParams {
            a: (1 << 24) + 1,
            ..AffineParams::IDENTITY
        };
        assert_eq!(bad.check().unwrap_err(), VoleError::NonCanonicalEncoding);
        let ok = AffineParams {
            a: 1 << 24,
            ..AffineParams::IDENTITY
        };
        assert!(ok.check().is_ok(), "the domain bound is inclusive");
        assert_eq!(AffineParams::IDENTITY.wire_bytes(), 29);
    }

    #[test]
    fn source_mapping_never_wraps_on_overflow() {
        // Coefficients within the wire domain and canvas coordinates within
        // limits keep products below i64 range, but a hostile *direct* call
        // with huge values must return None, never wrap.
        let p = AffineParams {
            a: 1 << 40,
            ..AffineParams::IDENTITY
        };
        assert!(p.source(1 << 40, 0).is_none());
    }
}
