//! Phase M — deterministic integer transform residual floor.
//!
//! When procedural state cannot explain a dense residual economically, a
//! conventional **transform-coded floor** takes over: the per-frame residual
//! field (the signed difference between the materialized base and the target
//! observation) is partitioned into aligned `4×4` blocks and each block is
//! decorrelated by a separable **integer lifting DCT** before entropy coding.
//! The decoder runs the exact integer inverse transform and adds the
//! reconstructed samples back to the base (`F = M ⊕_ρ R` with the additive
//! transform-residual algebra). There is no floating point anywhere in the
//! normative path and no quantization: the transform is lossless by
//! construction (see below), so the floor behaves like a conventional codec
//! *without* leaving the lossless domain. Whether it wins is decided by the
//! complete-cost court, never assumed.
//!
//! # The transform (normative, exact, integer-only)
//!
//! The 1-D stage is the DCT-II flow graph with every rotation realized as
//! three reversible integer lifting steps. For a rotation by `θ` the
//! factorization
//!
//! ```text
//! R(θ) = L(A)·U(B)·L(A),   A = −tan(θ/2),  B = sin θ
//! ```
//!
//! is quantized to Q8 (`A,B` in units of 1/256); each lifting step
//!
//! ```text
//! u = a + ((A·b) >> 8)     v = b + ((B·u) >> 8)     w = u + ((A·v) >> 8)
//! ```
//!
//! is inverted exactly by reversing the steps and subtracting the *same*
//! floor terms (the signed `>> 8` is floor division, consistent in both
//! directions), so every stage is an invertible integer map with no division
//! and no rounding ambiguity. The two normative rotations are
//!
//! * even part (`Y0`, `Y2` from the pair sums): `θ = −π/4` ⇒
//!   `A = 106`, `B = −181`;
//! * odd part (`Y1`, `Y3` from the pair differences): `θ = −π/8` ⇒
//!   `A = 51`, `B = −98`.
//!
//! The remaining butterfly splits are exact integer adds/subtracts whose
//! inverse halves are always even for canonical streams (roundtrip property:
//! `inverse(forward(x)) == x` for every integer block), and deterministic
//! floor shifts on hostile non-canonical input.
//!
//! The 2-D block transform is separable: forward applies the 1-D stage to
//! each row then each column (`C = T·X·Tᵀ`); the inverse applies the 1-D
//! inverse to each column then each row. Coefficients are small signed
//! integers (a `4×4` block of `±255` samples stays far below `2^31`), stored
//! in canonical zigzag `u32` little-endian form.
//!
//! # The wire payload (residual block kind 2, tag `0x2a`)
//!
//! ```text
//! kind u8 = 2 | tfm u8 = 0 | mask (ceil(Bx·By/8) bytes)
//! u32 dc_len | u32 ac_len | dc container | ac container
//! ```
//!
//! * `Bx = ceil(w/4)`, `By = ceil(h/4)`; block `k = by·Bx + bx` is **coded**
//!   iff mask bit `k` (LSB-first) is 1; padding bits past `Bx·By` must be 0;
//! * the dc and ac containers are standard self-describing Phase-F payloads
//!   (`KIND_RAW`/`KIND_RANS`) whose decoded bytes are, for every coded block
//!   in row-major order, the zigzag `u32 LE` DC coefficient (`C00`, 4 bytes)
//!   respectively the 15 AC coefficients (`C01..C33` row-major, 60 bytes);
//! * the decoder inverse-transforms each coded block and **adds** the
//!   reconstructed samples to the canvas; a result outside `0..=255` is a
//!   typed error.

use crate::error::VoleError;

/// Normative transform id for the 4×4 lifting DCT (v1).
pub const TRANSFORM_ID_4X4: u8 = 0;
/// Block edge in pixels.
pub const BLOCK: usize = 4;
/// Lifting multipliers are Q8 (one unit = 1/256).
const Q: i64 = 8;

// Rotation by θ = −π/4 (even part): A = −tan(θ/2) = tan(π/8) ≈ 106.02 → 106;
// B = sin θ = −sin(π/4) ≈ −181.02 → −181.
const EVEN_A: i64 = 106;
const EVEN_B: i64 = -181;
// Rotation by θ = −π/8 (odd part): A = −tan(θ/2) = tan(π/16) ≈ 50.92 → 51;
// B = sin θ = −sin(π/8) ≈ −97.97 → −98.
const ODD_A: i64 = 51;
const ODD_B: i64 = -98;

/// Number of `4×4` blocks along each canvas axis.
pub fn blocks_per_axis(w: u32, h: u32) -> (usize, usize) {
    (
        usize::try_from(w).unwrap_or(usize::MAX).div_ceil(BLOCK),
        usize::try_from(h).unwrap_or(usize::MAX).div_ceil(BLOCK),
    )
}

/// Mask byte count for a canvas (one bit per block, LSB-first).
pub fn mask_len(w: u32, h: u32) -> usize {
    let (bx, by) = blocks_per_axis(w, h);
    bx.saturating_mul(by).div_ceil(8)
}

/// Canonical zigzag map of a signed coefficient to an unsigned `u32`
/// (small magnitudes stay small; `0 → 0`, `−1 → 1`, `1 → 2`, …).
pub fn zigzag(v: i32) -> u32 {
    let u = v as u32;
    u.wrapping_shl(1) ^ ((v >> 31) as u32)
}

/// Inverse of [`zigzag`] (`u32 → i32`, total over the full `u32` domain).
pub fn unzigzag(z: u32) -> i32 {
    let n = (z >> 1) as i32;
    if z & 1 == 1 {
        !n
    } else {
        n
    }
}

/// One forward lifting rotation (Q8 floor semantics; exactly inverted by
/// [`rot_inv`] with the same multipliers).
#[inline]
fn rot(a: i64, b: i64, ma: i64, mb: i64) -> (i64, i64) {
    let u = a + ((ma * b) >> Q);
    let v = b + ((mb * u) >> Q);
    let w = u + ((ma * v) >> Q);
    (w, v)
}

/// Inverse lifting rotation: reverses [`rot`] exactly (subtracts the same
/// floor terms), recovering `(a, b)` from `(w, v)` for every integer input.
#[inline]
fn rot_inv(w: i64, v: i64, ma: i64, mb: i64) -> (i64, i64) {
    let u = w - ((ma * v) >> Q);
    let b = v - ((mb * u) >> Q);
    let a = u - ((ma * b) >> Q);
    (a, b)
}

/// Forward 1-D DCT-II stage (integer, exact) over four samples.
fn dct4_fwd(x: [i64; 4]) -> [i64; 4] {
    let (r0, r1, r2, r3) = (x[0], x[1], x[2], x[3]);
    // Butterfly: pair sums (even part) and differences (odd part).
    let (b0, b1) = (r0 + r3, r1 + r2);
    let (b2, b3) = (r0 - r3, r1 - r2);
    let (y0, y2) = rot(b0, b1, EVEN_A, EVEN_B); // θ = −π/4
    let (y1, y3) = rot(b2, b3, ODD_A, ODD_B); // θ = −π/8
    [y0, y1, y2, y3]
}

/// Inverse 1-D DCT-II stage: reverses [`dct4_fwd`] exactly on canonical
/// coefficient vectors (deterministic floor halves otherwise).
fn dct4_inv(y: [i64; 4]) -> [i64; 4] {
    let (y0, y1, y2, y3) = (y[0], y[1], y[2], y[3]);
    let (b0, b1) = rot_inv(y0, y2, EVEN_A, EVEN_B);
    let (b2, b3) = rot_inv(y1, y3, ODD_A, ODD_B);
    // Inverse butterflies: the sums are always even on canonical streams.
    let r0 = (b0 + b2) >> 1;
    let r3 = (b0 - b2) >> 1;
    let r1 = (b1 + b3) >> 1;
    let r2 = (b1 - b3) >> 1;
    [r0, r1, r2, r3]
}

/// Forward separable 4×4 block transform: rows then columns (`C = T·X·Tᵀ`).
/// `samples` is row-major with values in the canonical residual domain
/// (`±255`; padded cells 0); every coefficient stays far inside `i32`.
pub fn forward_block(samples: &[i64; 16]) -> [i32; 16] {
    let mut t = *samples;
    for r in 0..4 {
        let o = dct4_fwd([t[4 * r], t[4 * r + 1], t[4 * r + 2], t[4 * r + 3]]);
        t[4 * r..4 * r + 4].copy_from_slice(&o);
    }
    for c in 0..4 {
        let o = dct4_fwd([t[c], t[4 + c], t[8 + c], t[12 + c]]);
        t[c] = o[0];
        t[4 + c] = o[1];
        t[8 + c] = o[2];
        t[12 + c] = o[3];
    }
    let mut out = [0i32; 16];
    for (dst, src) in out.iter_mut().zip(t) {
        *dst = src as i32;
    }
    out
}

/// Inverse separable 4×4 block transform: columns then rows (`X = T⁻¹·C·T⁻ᵀ`).
/// Total over the full `i32` coefficient domain (hostile inputs are bounded,
/// never wrapping): intermediate arithmetic runs in `i64`.
pub fn inverse_block(coeffs: &[i32; 16]) -> [i64; 16] {
    let mut t = [0i64; 16];
    for (dst, src) in t.iter_mut().zip(coeffs) {
        *dst = i64::from(*src);
    }
    for c in 0..4 {
        let o = dct4_inv([t[c], t[4 + c], t[8 + c], t[12 + c]]);
        t[c] = o[0];
        t[4 + c] = o[1];
        t[8 + c] = o[2];
        t[12 + c] = o[3];
    }
    for r in 0..4 {
        let o = dct4_inv([t[4 * r], t[4 * r + 1], t[4 * r + 2], t[4 * r + 3]]);
        t[4 * r..4 * r + 4].copy_from_slice(&o);
    }
    t
}

/// Parse-time structural validation of a residual block of kind [`KIND_TSF`]
/// (see the module docs for the layout). Bounds the mask and the two
/// self-describing sub-containers before any allocation; deeper corruption
/// (entropy overread, coefficient-count mismatch, out-of-range reconstruction)
/// surfaces as a typed error at materialization, mirroring the Phase-G policy.
pub fn check_block(bytes: &[u8], max_out: u64, w: u32, h: u32) -> Result<(), VoleError> {
    if bytes.len() < 2 {
        return Err(VoleError::Truncated);
    }
    if bytes[0] != crate::rans::KIND_TSF {
        return Err(VoleError::NonCanonicalEncoding);
    }
    if bytes[1] != TRANSFORM_ID_4X4 {
        // Unknown mandatory transform: fail closed, typed.
        return Err(VoleError::NonCanonicalEncoding);
    }
    let (bx, by) = blocks_per_axis(w, h);
    let nblocks = bx.checked_mul(by).ok_or(VoleError::ArithmeticOverflow)?;
    let mlen = nblocks.div_ceil(8);
    let need = 2usize
        .checked_add(mlen)
        .and_then(|v| v.checked_add(8))
        .ok_or(VoleError::ArithmeticOverflow)?;
    if bytes.len() < need {
        return Err(VoleError::Truncated);
    }
    // Padding bits past the last block must be zero (canonical).
    let used = nblocks % 8;
    if used != 0 {
        let last = bytes[1 + mlen];
        if last & !((1u8 << used) - 1) != 0 {
            return Err(VoleError::NonCanonicalEncoding);
        }
    }
    let o = 2 + mlen;
    let dc_len = u64::from(u32::from_le_bytes([
        bytes[o],
        bytes[o + 1],
        bytes[o + 2],
        bytes[o + 3],
    ]));
    let ac_len = u64::from(u32::from_le_bytes([
        bytes[o + 4],
        bytes[o + 5],
        bytes[o + 6],
        bytes[o + 7],
    ]));
    if dc_len > max_out || ac_len > max_out {
        return Err(VoleError::DimensionTooLarge);
    }
    let total = o as u64 + 8 + dc_len + ac_len;
    if total != bytes.len() as u64 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    // Each sub-container must at least carry its kind + length prefix, and a
    // rANS container its inline model and 4-byte initial state.
    let check_sub = |len: u64, kind_off: usize| -> Result<(), VoleError> {
        if len < 9 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let kind = bytes[kind_off];
        match kind {
            crate::rans::KIND_RAW => {}
            crate::rans::KIND_RANS => {
                if len < 9 + crate::rans::MODEL_SERIALIZED as u64 + 4 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        }
        Ok(())
    };
    check_sub(dc_len, o + 8)?;
    check_sub(ac_len, o + 8 + dc_len as usize)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Limits;

    /// Deterministic pseudo-random generator (fixed seed).
    fn rnd(seed: &mut u64) -> u64 {
        *seed ^= *seed >> 12;
        *seed ^= *seed << 25;
        *seed ^= *seed >> 27;
        *seed = seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        *seed >> 33
    }

    #[test]
    fn zigzag_roundtrips_over_the_full_domain() {
        let vals = [
            0,
            1,
            -1,
            2,
            -2,
            255,
            -255,
            i32::MAX,
            i32::MIN,
            1 << 30,
            -(1 << 30),
            12345,
            -6789,
        ];
        for v in vals {
            assert_eq!(unzigzag(zigzag(v)), v, "zigzag roundtrip of {v}");
        }
        // Magnitude monotonicity near zero (what the entropy floor exploits).
        assert_eq!(zigzag(0), 0);
        assert_eq!(zigzag(-1), 1);
        assert_eq!(zigzag(1), 2);
        assert!(zigzag(1) < zigzag(2));
        assert!(zigzag(-1) < zigzag(-2));
        let mut seed = 42u64;
        for _ in 0..10_000 {
            let v = rnd(&mut seed) as u32 as i32;
            assert_eq!(unzigzag(zigzag(v)), v);
        }
    }

    #[test]
    fn lifting_rotations_invert_exactly_for_any_integers() {
        let mut seed = 7u64;
        for _ in 0..10_000 {
            let a = rnd(&mut seed) as i64 - (1 << 40);
            let b = rnd(&mut seed) as i64 - (1 << 40);
            for (ma, mb) in [(EVEN_A, EVEN_B), (ODD_A, ODD_B)] {
                let (w, v) = rot(a, b, ma, mb);
                assert_eq!(rot_inv(w, v, ma, mb), (a, b));
            }
        }
    }

    #[test]
    fn one_dimensional_stage_roundtrips_and_orders_frequencies() {
        // Constant input: only the DC (index 0) is large.
        let c = dct4_fwd([100, 100, 100, 100]);
        assert!(c[0].abs() > 100);
        assert!(
            c[1..].iter().all(|v| v.abs() < 4),
            "flat row stays DC: {c:?}"
        );
        // Linear ramp: energy concentrates in DC + the first-order AC (the
        // DCT basis is cosines, so a line keeps a small third-order term;
        // the even second-order term is exactly zero).
        let l = dct4_fwd([0, 40, 80, 120]);
        assert!(
            l[0].abs() > 150 && l[1].abs() > 100,
            "ramp is low-order: {l:?}"
        );
        assert_eq!(l[2], 0, "ramp has no even second-order term");
        assert!(
            l[3].abs() < l[1].abs() / 5,
            "third-order term stays small: {l:?}"
        );
        // Roundtrip over arbitrary integers in a bounded domain.
        let mut seed = 99u64;
        for _ in 0..10_000 {
            let x = [
                rnd(&mut seed) as i64 - (1 << 20),
                rnd(&mut seed) as i64 - (1 << 20),
                rnd(&mut seed) as i64 - (1 << 20),
                rnd(&mut seed) as i64 - (1 << 20),
            ];
            let y = dct4_fwd(x);
            let back = dct4_inv(y);
            assert_eq!(back, x, "1-D canonical roundtrip");
        }
    }

    #[test]
    fn block_transform_roundtrips_and_compacts_gradients() {
        let mut seed = 1234u64;
        for _ in 0..500 {
            let mut x = [0i64; 16];
            for v in &mut x {
                *v = (rnd(&mut seed) % 511) as i64 - 255; // residual domain
            }
            let c = forward_block(&x);
            let back = inverse_block(&c);
            assert_eq!(back, x, "2-D canonical roundtrip");
            // Coefficients stay far inside i32 for the ±255 domain.
            assert!(c
                .iter()
                .all(|v| v.checked_abs().is_some_and(|a| a < 1 << 20)));
        }
        // A linear ramp block concentrates into DC + first-order ACs.
        let mut ramp = [0i64; 16];
        for y in 0..4 {
            for x in 0..4 {
                ramp[y * 4 + x] = 40 * (x as i64) + 10 * (y as i64);
            }
        }
        let c = forward_block(&ramp);
        let energy: i64 = c
            .iter()
            .map(|v| i64::from(v.checked_mul(*v).unwrap()))
            .sum();
        // A separable cosine basis cannot make a plane exactly 2-sparse, but
        // the low-order half of the coefficient square carries ~all of it.
        let low: i64 = c[0..6]
            .iter()
            .map(|v| i64::from(v.checked_mul(*v).unwrap()))
            .sum();
        assert!(
            low * 100 >= energy * 95,
            "ramp energy must concentrate in the low-order coefficients: {c:?}"
        );
        // And it roundtrips back exactly.
        assert_eq!(inverse_block(&c), ramp);
    }

    #[test]
    fn inverse_is_total_and_bounded_on_hostile_coefficients() {
        // Arbitrary i32 coefficients (worst hostile range) must decode
        // deterministically with no panic and bounded intermediate values.
        let mut seed = 0xDEAD_BEEFu64;
        for _ in 0..2000 {
            let mut c = [0i32; 16];
            for v in &mut c {
                *v = rnd(&mut seed) as u32 as i32;
            }
            let out = inverse_block(&c);
            assert!(out
                .iter()
                .all(|v| v.checked_abs().is_some_and(|a| a < 1 << 40)));
        }
        // Extreme single coefficient.
        let mut c = [0i32; 16];
        c[0] = i32::MIN;
        let _ = inverse_block(&c);
        c[0] = i32::MAX;
        let _ = inverse_block(&c);
    }

    #[test]
    fn check_block_validates_structure_and_hostile_forms() {
        // A minimal canonical all-skip payload: two empty RAW containers.
        let mut b = vec![
            crate::rans::KIND_TSF,
            TRANSFORM_ID_4X4,
            0, // mask: no blocks coded (w = h = 4 -> 1 block -> 1 mask byte)
        ];
        let empty_raw = crate::rans::encode_block(&[]);
        b.extend_from_slice(&(empty_raw.len() as u32).to_le_bytes());
        b.extend_from_slice(&(empty_raw.len() as u32).to_le_bytes());
        b.extend_from_slice(&empty_raw);
        b.extend_from_slice(&empty_raw);
        assert!(check_block(&b, 1 << 20, 4, 4).is_ok());
        // Unknown transform id fails closed.
        let mut bad = b.clone();
        bad[1] = 7;
        assert_eq!(
            check_block(&bad, 1 << 20, 4, 4).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // A set padding bit is non-canonical.
        let mut bad2 = b.clone();
        bad2[2] = 0x80; // 1 block -> only bit 0 valid
        assert_eq!(
            check_block(&bad2, 1 << 20, 4, 4).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Length prefix disagreement is non-canonical.
        let mut bad3 = b.clone();
        bad3[3] = 99;
        assert_eq!(
            check_block(&bad3, 1 << 20, 4, 4).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Truncation is typed: cutting below the structural minimum is
        // Truncated; cutting inside the containers is a length disagreement.
        let t = &b[..5];
        assert_eq!(
            check_block(t, 1 << 20, 4, 4).unwrap_err(),
            VoleError::Truncated
        );
        let t2 = &b[..b.len() - 2];
        assert_eq!(
            check_block(t2, 1 << 20, 4, 4).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
    }

    #[test]
    fn residual_payload_applies_exactly_and_hostile_forms_are_typed() {
        use crate::materialize;
        use crate::pixel::Canvas;
        // 12x8 canvas: 3x2 = 6 blocks (mask 1 byte, pad bits 6..8 must stay 0).
        let (w, h) = (12u32, 8u32);
        let base = Canvas::from_parts(w, h, vec![90u8; (w * h) as usize]).expect("canvas");
        // Target: base plus a per-pixel ramp difference (dense, smooth).
        let mut tgt = base.clone();
        let mut data = tgt.as_slice().to_vec();
        for y in 0..h as usize {
            for x in 0..w as usize {
                data[y * w as usize + x] =
                    (u16::from(data[y * w as usize + x]) + 3 * (x as u16) + 2 * (y as u16)) as u8;
            }
        }
        tgt = Canvas::from_parts(w, h, data).expect("canvas");
        let block = crate::inverse::build_transform_block(&base, &tgt).expect("built");
        assert_eq!(block[0], crate::rans::KIND_TSF);
        // Normative application reproduces the target byte-for-byte.
        let mut dst = base.clone();
        materialize::apply_residual(&mut dst, &block, &Limits::default()).expect("apply");
        assert_eq!(dst.as_slice(), tgt.as_slice());

        // Hostile: unknown transform id.
        let mut b1 = block.clone();
        b1[1] = 99;
        let mut dst = base.clone();
        assert_eq!(
            materialize::apply_residual(&mut dst, &b1, &Limits::default()).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Hostile: set a padding mask bit (bit 6 of the single mask byte).
        let mut b2 = block.clone();
        b2[2] |= 0x40;
        let mut dst = base.clone();
        assert_eq!(
            materialize::apply_residual(&mut dst, &b2, &Limits::default()).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Hostile: ac_len prefix disagrees with the container bytes.
        let mlen = crate::transform::mask_len(w, h);
        let o = 2 + mlen;
        let mut b3 = block.clone();
        let ac_len_off = o + 4;
        let wrong_ac = (b3.len() as u32).wrapping_add(40);
        b3[ac_len_off..ac_len_off + 4].copy_from_slice(&wrong_ac.to_le_bytes());
        let mut dst = base.clone();
        assert_eq!(
            materialize::apply_residual(&mut dst, &b3, &Limits::default()).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Hostile: coefficients that reconstruct outside Gray8 -> OutOfBounds.
        // Locate the first DC coefficient byte: the DC container is RAW here
        // (small stream), so its payload starts 9 bytes into the container,
        // which begins right after the two length prefixes.
        let dc_container_off = o + 8;
        assert_eq!(block[dc_container_off], crate::rans::KIND_RAW);
        let first_dc = dc_container_off + 9;
        let mut b4 = block.clone();
        let huge = crate::transform::zigzag(1 << 29).to_le_bytes();
        b4[first_dc..first_dc + 4].copy_from_slice(&huge);
        let mut dst = base.clone();
        assert_eq!(
            materialize::apply_residual(&mut dst, &b4, &Limits::default()).unwrap_err(),
            VoleError::OutOfBounds
        );
        // Truncation of the block body is typed at apply.
        let mut dst = base.clone();
        let t = &block[..block.len() - 1];
        assert!(matches!(
            materialize::apply_residual(&mut dst, t, &Limits::default()),
            Err(VoleError::NonCanonicalEncoding | VoleError::Truncated)
        ));
    }
}
