//! Depth-aware procedural generators — Phase V.1.4 (V.1 video programme,
//! brief §247: the sealed Phase-N generator family generalized to the
//! canonical plane sample domain).
//!
//! A **generator** is immutable *content-as-program*: instead of storing the
//! samples of an object's box, the object carries a small deterministic
//! integer program that computes each sample at materialization. The four
//! programs are the v1 (Phase-N) kinds, generalized from the Gray8 mod-256
//! domain to any plane depth:
//!
//! * `Gradient` — `v = (base + sx·x + sy·y) mod (max+1)` over the box
//!   (mod-(max+1) is the canonical sample-domain wrap of a depth-`d` plane);
//! * `Checker` — alternating `a`/`b` samples in `cell × cell` squares
//!   (`a` when `⌊x/cell⌋ + ⌊y/cell⌋` is even, else `b`; floor division);
//! * `Periodic` — a sawtooth field with an explicit period;
//! * `Noise` — a bounded seeded integer hash of the coordinate (splitmix64
//!   finalizer, top byte) scaled into the sample domain.
//!
//! **Depth-8 identity (the specialization contract):** at `max = 255` every
//! program reproduces the v1 Phase-N generator byte-for-byte for equivalent
//! parameters — `Gradient`/`Periodic` wrap mod 256 exactly, `Checker`
//! `a`/`b` are the v1 samples, and `Noise` scales by `(b·256) >> 8 == b`.
//! The v1 module (`crate::generator`) is untouched and remains authoritative
//! for format v1.
//!
//! Work is one sample per painted pixel (the same class as a raster blit):
//! no loops, no recursion, no hidden state. Generator parameters are
//! validated against the plane's active depth at declaration/parse time —
//! out-of-domain values are typed errors, never wraps.

use crate::error::VoleError;

/// Normative program tags inside a generator object payload (media domain;
/// mirrors the v1 tags).
pub const GEN_GRADIENT: u8 = 0x00;
pub const GEN_CHECKER: u8 = 0x01;
pub const GEN_PERIODIC: u8 = 0x02;
pub const GEN_NOISE: u8 = 0x03;

/// Canonical signed-domain bound for slope coefficients (mirrors the frozen
/// v1 wire domain of every signed literal).
pub const MAX_GEN_COEFF: i64 = 1 << 24;
/// Upper bound of a checker cell or a sawtooth period.
pub const MAX_GEN_PERIOD: u32 = 4096;

/// A bounded procedural content program in the sample domain of one plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gen {
    /// `v(x, y) = (base + sx·x + sy·y) mod (max+1)` — a wrap-around linear
    /// ramp over the object box in the plane's sample domain.
    Gradient { base: u32, sx: i64, sy: i64 },
    /// Classic checkerboard of `cell × cell` squares alternating `a` and `b`.
    Checker { a: u32, b: u32, cell: u32 },
    /// Sawtooth field with an explicit period:
    /// `v = (base + sx·(x mod period) + sy·(y mod period)) mod (max+1)`.
    Periodic {
        base: u32,
        sx: i64,
        sy: i64,
        period: u32,
    },
    /// Bounded deterministic noise: a seeded integer hash of the coordinate
    /// (splitmix64 finalizer over a seed-mixed position; the top byte is
    /// scaled into the plane's sample domain). Used by authored content and
    /// by the negative control.
    Noise { seed: u64 },
}

impl Gen {
    /// Canonical-form check against the plane's maximum sample: value
    /// parameters (`base`, `a`, `b`) inside `0..=max`, slope coefficients in
    /// `±MAX_GEN_COEFF`, cell/period in `1..=MAX_GEN_PERIOD`. Out-of-domain
    /// parameters are a typed error.
    pub fn check(&self, max: u32) -> Result<(), VoleError> {
        let v_ok = |v: u32| v <= max;
        let c_ok = |c: i64| c.abs() <= MAX_GEN_COEFF;
        match self {
            Gen::Gradient { base, sx, sy } => {
                if !v_ok(*base) || !c_ok(*sx) || !c_ok(*sy) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                Ok(())
            }
            Gen::Checker { a, b, cell } => {
                if !v_ok(*a) || !v_ok(*b) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if *cell == 0 || *cell > MAX_GEN_PERIOD {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                Ok(())
            }
            Gen::Periodic {
                base,
                sx,
                sy,
                period,
            } => {
                if !v_ok(*base) || !c_ok(*sx) || !c_ok(*sy) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if *period == 0 || *period > MAX_GEN_PERIOD {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                Ok(())
            }
            Gen::Noise { .. } => Ok(()),
        }
    }

    /// The one sample at content-local coordinate `(x, y)` in a plane whose
    /// maximum sample is `max` (`max + 1` must be ≤ 2^16; the function is
    /// total over all integers). Canonical integer semantics.
    pub fn sample(&self, x: i64, y: i64, max: u32) -> u32 {
        let m = u64::from(max) + 1;
        match self {
            Gen::Gradient { base, sx, sy } => (u64::from(*base) as i128
                + i128::from(*sx) * i128::from(x)
                + i128::from(*sy) * i128::from(y))
            .rem_euclid(m as i128) as u32,
            Gen::Checker { a, b, cell } => {
                let c = i64::from(*cell);
                let parity = x.div_euclid(c) + y.div_euclid(c);
                if parity & 1 == 0 {
                    *a
                } else {
                    *b
                }
            }
            Gen::Periodic {
                base,
                sx,
                sy,
                period,
            } => {
                let p = i128::from(*period);
                let px = i128::from(x).rem_euclid(p);
                let py = i128::from(y).rem_euclid(p);
                (u64::from(*base) as i128 + i128::from(*sx) * px + i128::from(*sy) * py)
                    .rem_euclid(m as i128) as u32
            }
            Gen::Noise { seed } => {
                let b = noise_top_byte(*seed, x, y);
                // Depth-8 identity (b·256 >> 8 == b); deterministic scaling
                // into the sample domain at every depth.
                ((u64::from(b) * m) >> 8) as u32
            }
        }
    }

    /// Canonical wire bytes of the program: `kind u8` + little-endian
    /// parameters (u32 values, i32 coefficients, u64 seeds). This is the
    /// byte-for-byte form carried by a generator object (object kind `0x04`).
    pub fn program_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24);
        match self {
            Gen::Gradient { base, sx, sy } => {
                out.push(GEN_GRADIENT);
                out.extend_from_slice(&base.to_le_bytes());
                out.extend_from_slice(&(*sx as i32).to_le_bytes());
                out.extend_from_slice(&(*sy as i32).to_le_bytes());
            }
            Gen::Checker { a, b, cell } => {
                out.push(GEN_CHECKER);
                out.extend_from_slice(&a.to_le_bytes());
                out.extend_from_slice(&b.to_le_bytes());
                out.extend_from_slice(&cell.to_le_bytes());
            }
            Gen::Periodic {
                base,
                sx,
                sy,
                period,
            } => {
                out.push(GEN_PERIODIC);
                out.extend_from_slice(&base.to_le_bytes());
                out.extend_from_slice(&(*sx as i32).to_le_bytes());
                out.extend_from_slice(&(*sy as i32).to_le_bytes());
                out.extend_from_slice(&period.to_le_bytes());
            }
            Gen::Noise { seed } => {
                out.push(GEN_NOISE);
                out.extend_from_slice(&seed.to_le_bytes());
            }
        }
        out
    }

    /// Parse a program from a reader positioned at its kind byte (normative
    /// wire form). Unknown kinds and out-of-domain parameters are typed
    /// errors; the reader must be exhausted by the caller's record framing.
    pub fn parse_program(
        r: &mut crate::checked::ByteReader<'_>,
        max: u32,
    ) -> Result<Self, VoleError> {
        let kind = r.u8()?;
        let g = match kind {
            GEN_GRADIENT => {
                let base = r.pull::<u32>()?;
                let sx = i64::from(r.pull::<i32>()?);
                let sy = i64::from(r.pull::<i32>()?);
                Gen::Gradient { base, sx, sy }
            }
            GEN_CHECKER => {
                let a = r.pull::<u32>()?;
                let b = r.pull::<u32>()?;
                let cell = r.pull::<u32>()?;
                Gen::Checker { a, b, cell }
            }
            GEN_PERIODIC => {
                let base = r.pull::<u32>()?;
                let sx = i64::from(r.pull::<i32>()?);
                let sy = i64::from(r.pull::<i32>()?);
                let period = r.pull::<u32>()?;
                Gen::Periodic {
                    base,
                    sx,
                    sy,
                    period,
                }
            }
            GEN_NOISE => {
                let seed = r.pull::<u64>()?;
                Gen::Noise { seed }
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        };
        g.check(max)?;
        Ok(g)
    }
}

/// Deterministic bounded integer noise top byte: mix the seed with the
/// position, then run the splitmix64 finalizer and take the top byte
/// (identical to the sealed v1 noise hash).
fn noise_top_byte(seed: u64, x: i64, y: i64) -> u8 {
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

    /// Depth-8 identity: equivalent v1 (Phase-N) and media generators agree
    /// on every sample.
    #[test]
    fn depth8_matches_the_sealed_v1_generator() {
        let pairs: Vec<(Gen, crate::generator::Generator)> = vec![
            (
                Gen::Gradient {
                    base: 100,
                    sx: 3,
                    sy: -5,
                },
                crate::generator::Generator::Gradient {
                    base: 100,
                    sx: 3,
                    sy: -5,
                },
            ),
            (
                Gen::Checker {
                    a: 10,
                    b: 200,
                    cell: 4,
                },
                crate::generator::Generator::Checker {
                    a: 10,
                    b: 200,
                    cell: 4,
                },
            ),
            (
                Gen::Periodic {
                    base: 200,
                    sx: 2,
                    sy: 1,
                    period: 16,
                },
                crate::generator::Generator::Periodic {
                    base: 200,
                    sx: 2,
                    sy: 1,
                    period: 16,
                },
            ),
            (
                Gen::Noise { seed: 42 },
                crate::generator::Generator::Noise { seed: 42 },
            ),
        ];
        for (g2, g1) in pairs {
            for x in -8i64..40 {
                for y in -8i64..40 {
                    assert_eq!(
                        g2.sample(x, y, 255),
                        u32::from(g1.sample(x, y)),
                        "depth-8 identity at ({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn sample_domain_wrap_is_mod_max_plus_one() {
        let g = Gen::Gradient {
            base: 1000,
            sx: 3,
            sy: -5,
        };
        // 10-bit plane: max 1023, wrap at 1024.
        assert_eq!(g.sample(0, 0, 1023), 1000);
        // (1000 + 3·8) = 1024 -> 0.
        assert_eq!(g.sample(8, 0, 1023), 0);
        // 12-bit: max 4095.
        let p = Gen::Periodic {
            base: 0,
            sx: 2,
            sy: 1,
            period: 16,
        };
        assert_eq!(p.sample(0, 0, 4095), 0);
        assert_eq!(p.sample(16, 0, 4095), p.sample(0, 0, 4095));
        let c = Gen::Checker {
            a: 500,
            b: 4000,
            cell: 8,
        };
        assert_eq!(c.sample(0, 0, 4095), 500);
        assert_eq!(c.sample(8, 0, 4095), 4000);
        assert_eq!(c.sample(16, 0, 4095), 500);
    }

    #[test]
    fn noise_scales_deterministically_into_the_sample_domain() {
        let n = Gen::Noise { seed: 7 };
        // Depth-8: exact top byte.
        assert_eq!(
            n.sample(3, 9, 255),
            u32::from(crate::generator::Generator::Noise { seed: 7 }.sample(3, 9))
        );
        // Depth-10 max 1023: (b·1024) >> 8 = b·4, always a multiple of 4.
        let v10 = n.sample(3, 9, 1023);
        assert!(v10 < 1024);
        assert_eq!(v10 % 4, 0);
        assert_eq!(n.sample(3, 9, 1023), n.sample(3, 9, 1023), "deterministic");
        assert_ne!(
            Gen::Noise { seed: 8 }.sample(3, 9, 1023),
            v10,
            "seed-sensitive"
        );
    }

    #[test]
    fn wire_roundtrips_and_hostile_forms_are_typed() {
        let gens = [
            Gen::Gradient {
                base: 1000,
                sx: 3,
                sy: -2,
            },
            Gen::Checker {
                a: 1,
                b: 65535,
                cell: 8,
            },
            Gen::Periodic {
                base: 200,
                sx: -3,
                sy: 1,
                period: 64,
            },
            Gen::Noise { seed: u64::MAX },
        ];
        for g in gens {
            let bytes = g.program_bytes();
            let mut r = crate::checked::ByteReader::new(&bytes);
            let back = Gen::parse_program(&mut r, 65535).unwrap();
            assert_eq!(r.remaining(), 0);
            assert_eq!(back, g);
        }
        // Out-of-domain value parameters are typed against the plane depth.
        let bad = Gen::Gradient {
            base: 1024,
            sx: 0,
            sy: 0,
        };
        assert_eq!(
            bad.check(1023).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        assert!(bad.check(1024).is_ok());
        let badc = Gen::Checker {
            a: 0,
            b: 0,
            cell: 0,
        };
        assert_eq!(
            badc.check(255).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        let bads = Gen::Gradient {
            base: 0,
            sx: (1 << 24) + 1,
            sy: 0,
        };
        assert_eq!(
            bads.check(255).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Truncated / unknown kind.
        let mut r = crate::checked::ByteReader::new(&[]);
        assert_eq!(
            Gen::parse_program(&mut r, 255).unwrap_err(),
            VoleError::Truncated
        );
        let mut r = crate::checked::ByteReader::new(&[0x7F, 1, 0, 0, 0]);
        assert_eq!(
            Gen::parse_program(&mut r, 255).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
    }
}
