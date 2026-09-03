//! Phase N — bounded procedural generators (normative content programs).
//!
//! A **generator** is an immutable *content program*: instead of storing the
//! samples of a region, an object may carry a small deterministic program
//! that **computes** each sample at materialization from its position inside
//! the object box. Generators are subject to the strict discipline of §21:
//!
//! * finite, deterministic, integer-only (no floating point anywhere);
//! * **bounded work**: one sample per pixel of the painted box, no loops,
//!   no recursion, no hidden state — the work is exactly the visible box
//!   area, the same class as a raster blit;
//! * not a claim that natural content is free: every generator candidate in
//!   the raster-origin encoder is validated by normative materialization, and
//!   a candidate whose samples do not reproduce the target exactly must carry
//!   its exact residual (or lose the cost court);
//! * the four v1 programs are `Gradient` (a wrap-around linear ramp), a
//!   `Checker`, a `Periodic` sawtooth field with an explicit period, and a
//!   `Noise` field (a bounded integer hash of the coordinates with a seed).
//!   `Noise` exists so the *negative control* is measurable: an unknown-seed
//!   noise field cannot be discovered by search, so RAW keeps those bytes —
//!   parameters that merely relocate information never win.
//!
//! Sample semantics are canonical mod-256 Gray8 arithmetic; two independent
//! implementations (the materializer and the court reference painters) must
//! agree byte-for-byte.

use crate::error::VoleError;

/// Normative program tags (wire form; see `docs/format-v1.md`).
pub const GEN_GRADIENT: u8 = 0x00;
pub const GEN_CHECKER: u8 = 0x01;
pub const GEN_PERIODIC: u8 = 0x02;
pub const GEN_NOISE: u8 = 0x03;

/// Canonical signed-domain bound for slope coefficients (mirrors every other
/// signed literal in format v1).
pub const MAX_GEN_COEFF: i64 = 1 << 24;
/// Upper bound of a checker cell or a sawtooth period (wire domain).
pub const MAX_GEN_PERIOD: u32 = 4096;

/// A bounded procedural content program (Phase N).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generator {
    /// `v(x, y) = (base + sx·x + sy·y) mod 256` — a wrap-around linear ramp
    /// over the object box (mod-256 arithmetic is the canonical Gray8 wrap).
    Gradient { base: u8, sx: i64, sy: i64 },
    /// Classic checkerboard of `cell × cell` squares alternating `a` and `b`:
    /// `a` when `((x/cell) + (y/cell))` is even, else `b` (floor division).
    Checker { a: u8, b: u8, cell: u32 },
    /// Sawtooth field with an explicit period:
    /// `v = (base + sx·(x mod period) + sy·(y mod period)) mod 256`.
    Periodic {
        base: u8,
        sx: i64,
        sy: i64,
        period: u32,
    },
    /// Bounded deterministic noise: a seeded integer hash of the coordinate
    /// (splitmix64 finalizer over a seed-mixed position). Total over all
    /// `i64` inputs; used by authored content and by the negative control.
    Noise { seed: u64 },
}

impl Generator {
    /// Canonical-form check: coefficients inside the wire domain, cell and
    /// period in `1..=MAX_GEN_PERIOD`. Out-of-domain parameters are a typed
    /// error, never a wrap.
    pub fn check(&self) -> Result<(), VoleError> {
        match self {
            Generator::Gradient { sx, sy, .. } => {
                if sx.abs() > MAX_GEN_COEFF || sy.abs() > MAX_GEN_COEFF {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                Ok(())
            }
            Generator::Periodic { sx, sy, period, .. } => {
                if sx.abs() > MAX_GEN_COEFF || sy.abs() > MAX_GEN_COEFF {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if *period == 0 || *period > MAX_GEN_PERIOD {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                Ok(())
            }
            Generator::Checker { cell, .. } => {
                if *cell == 0 || *cell > MAX_GEN_PERIOD {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                Ok(())
            }
            Generator::Noise { .. } => Ok(()),
        }
    }

    /// The one sample at content-local coordinate `(x, y)` (in-box only; the
    /// function is total over all integers). Canonical integer semantics.
    pub fn sample(&self, x: i64, y: i64) -> u8 {
        match self {
            Generator::Gradient { base, sx, sy } => {
                (i64::from(*base) + sx * x + sy * y).rem_euclid(256) as u8
            }
            Generator::Checker { a, b, cell } => {
                let c = i64::from(*cell);
                let parity = x.div_euclid(c) + y.div_euclid(c);
                if parity & 1 == 0 {
                    *a
                } else {
                    *b
                }
            }
            Generator::Periodic {
                base,
                sx,
                sy,
                period,
            } => {
                let p = i64::from(*period);
                let px = x.rem_euclid(p);
                let py = y.rem_euclid(p);
                (i64::from(*base) + sx * px + sy * py).rem_euclid(256) as u8
            }
            Generator::Noise { seed } => noise_sample(*seed, x, y),
        }
    }

    /// Canonical wire bytes of the program: `kind u8` + parameters. This is
    /// the byte-for-byte form used by the object record (tag `0x07`), by
    /// content identity, and by the accounting walker.
    pub fn program_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24);
        match self {
            Generator::Gradient { base, sx, sy } => {
                out.push(GEN_GRADIENT);
                out.push(*base);
                out.extend_from_slice(&(*sx as i32).to_le_bytes());
                out.extend_from_slice(&(*sy as i32).to_le_bytes());
            }
            Generator::Checker { a, b, cell } => {
                out.push(GEN_CHECKER);
                out.push(*a);
                out.push(*b);
                out.extend_from_slice(&cell.to_le_bytes());
            }
            Generator::Periodic {
                base,
                sx,
                sy,
                period,
            } => {
                out.push(GEN_PERIODIC);
                out.push(*base);
                out.extend_from_slice(&(*sx as i32).to_le_bytes());
                out.extend_from_slice(&(*sy as i32).to_le_bytes());
                out.extend_from_slice(&period.to_le_bytes());
            }
            Generator::Noise { seed } => {
                out.push(GEN_NOISE);
                out.extend_from_slice(&seed.to_le_bytes());
            }
        }
        out
    }

    /// Parse a program from a reader positioned at its kind byte (normative
    /// wire form). Unknown kinds and out-of-domain parameters are typed
    /// errors; the reader must be exhausted by the caller's record framing.
    pub fn parse_program(r: &mut crate::checked::ByteReader<'_>) -> Result<Self, VoleError> {
        let kind = r.u8()?;
        let g = match kind {
            GEN_GRADIENT => {
                let base = r.u8()?;
                let sx = i64::from(r.pull::<i32>()?);
                let sy = i64::from(r.pull::<i32>()?);
                Generator::Gradient { base, sx, sy }
            }
            GEN_CHECKER => {
                let a = r.u8()?;
                let b = r.u8()?;
                let cell = r.pull::<u32>()?;
                Generator::Checker { a, b, cell }
            }
            GEN_PERIODIC => {
                let base = r.u8()?;
                let sx = i64::from(r.pull::<i32>()?);
                let sy = i64::from(r.pull::<i32>()?);
                let period = r.pull::<u32>()?;
                Generator::Periodic {
                    base,
                    sx,
                    sy,
                    period,
                }
            }
            GEN_NOISE => {
                let seed = r.pull::<u64>()?;
                Generator::Noise { seed }
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        };
        g.check()?;
        Ok(g)
    }

    /// Parse a program from its canonical wire bytes (kind + parameters).
    /// Unknown kinds and out-of-domain parameters are typed errors.
    pub fn from_program_bytes(bytes: &[u8]) -> Result<Self, VoleError> {
        let mut r = crate::checked::ByteReader::new(bytes);
        let g = Self::parse_program(&mut r)?;
        if r.remaining() != 0 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        Ok(g)
    }
}

/// Deterministic bounded integer noise sample: mix the seed with the
/// position, then run the splitmix64 finalizer and take the top byte.
fn noise_sample(seed: u64, x: i64, y: i64) -> u8 {
    let mut h = seed
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h >> 56) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_semantics_are_exact_mod_256() {
        let g = Generator::Gradient {
            base: 100,
            sx: 3,
            sy: -5,
        };
        assert_eq!(g.sample(0, 0), 100);
        assert_eq!(g.sample(1, 0), 103);
        assert_eq!(g.sample(0, 1), 95);
        // Wrap is canonical mod 256: (100 + 3*52) = 256 -> 0; +3 more -> 3.
        assert_eq!(g.sample(52, 0), 0);
        assert_eq!(g.sample(53, 0), 3);
        // Negative slope wraps canonically.
        let g2 = Generator::Gradient {
            base: 0,
            sx: -1,
            sy: 0,
        };
        assert_eq!(g2.sample(1, 0), 255);
        // rem_euclid semantics independent of the arithmetic shape.
        assert_eq!(
            g.sample(7, 11),
            (100i64 + 3 * 7 + (-5) * 11).rem_euclid(256) as u8
        );
    }

    #[test]
    fn checker_and_periodic_semantics_are_exact() {
        let c = Generator::Checker {
            a: 10,
            b: 200,
            cell: 4,
        };
        for (x, y, want) in [
            (0i64, 0i64, 10),
            (3, 0, 10),
            (4, 0, 200),
            (0, 4, 200),
            (4, 4, 10),
        ] {
            assert_eq!(c.sample(x, y), want, "checker ({x},{y})");
        }
        let p = Generator::Periodic {
            base: 0,
            sx: 2,
            sy: 1,
            period: 16,
        };
        // Sawtooth resets every 16 along each axis.
        assert_eq!(p.sample(0, 0), 0);
        assert_eq!(p.sample(3, 0), 6);
        assert_eq!(p.sample(16, 0), p.sample(0, 0));
        assert_eq!(p.sample(17, 0), p.sample(1, 0));
        assert_eq!(p.sample(15, 1), 31);
    }

    #[test]
    fn noise_is_deterministic_bounded_and_position_sensitive() {
        let n = Generator::Noise { seed: 42 };
        let mut seen = 0u64;
        let mut first = n.sample(0, 0);
        for x in 0..256 {
            for y in 0..4 {
                let v = n.sample(x, y);
                seen |= 1 << (v / 32);
                if x == 0 && y == 0 {
                    first = v;
                }
            }
        }
        assert_eq!(n.sample(0, 0), first, "deterministic");
        assert_eq!(Generator::Noise { seed: 42 }.sample(5, 9), n.sample(5, 9));
        assert_ne!(Generator::Noise { seed: 43 }.sample(5, 9), n.sample(5, 9));
        // Samples span the byte range (not a single narrow band).
        assert_eq!(seen.count_ones(), 8, "top byte of the hash spreads");
    }

    #[test]
    fn program_wire_roundtrips_and_hostile_forms_are_typed() {
        let gens = [
            Generator::Gradient {
                base: 7,
                sx: 3,
                sy: -2,
            },
            Generator::Checker {
                a: 1,
                b: 254,
                cell: 8,
            },
            Generator::Periodic {
                base: 200,
                sx: -3,
                sy: 1,
                period: 64,
            },
            Generator::Noise { seed: u64::MAX },
        ];
        for g in gens {
            let bytes = g.program_bytes();
            assert_eq!(Generator::from_program_bytes(&bytes).unwrap(), g);
        }
        // Truncated / unknown-kind / trailing bytes are typed errors.
        assert_eq!(
            Generator::from_program_bytes(&[]).unwrap_err(),
            VoleError::Truncated
        );
        assert_eq!(
            Generator::from_program_bytes(&[0x7F, 1]).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        let mut bad = Generator::Gradient {
            base: 0,
            sx: 0,
            sy: 0,
        }
        .program_bytes();
        bad.push(0);
        assert_eq!(
            Generator::from_program_bytes(&bad).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Out-of-domain parameters are typed.
        let badc = Generator::Checker {
            a: 0,
            b: 0,
            cell: 0,
        };
        assert_eq!(badc.check().unwrap_err(), VoleError::NonCanonicalEncoding);
        let bads = Generator::Gradient {
            base: 0,
            sx: (1 << 24) + 1,
            sy: 0,
        };
        assert_eq!(bads.check().unwrap_err(), VoleError::NonCanonicalEncoding);
        let ok = Generator::Gradient {
            base: 0,
            sx: 1 << 24,
            sy: -(1 << 24),
        };
        assert!(ok.check().is_ok(), "domain bounds are inclusive");
    }
}
