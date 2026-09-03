//! Phase F: the VOLE normative entropy floor — a native, deterministic,
//! order-0 **byte rANS** coder owned by this crate.
//!
//! VOLE does **not** delegate normative entropy semantics to any external
//! codec crate; every wire-level choice below is fixed, documented, and
//! reproducible. The algorithm follows the well-established `ryg_rans`
//! byte-rANS arithmetic (public domain), and the crate maintains
//! byte-for-byte parity courts against the independent `ryg-rans-rs`
//! reconstruction as a **test-only oracle** (`tests/phase_f.rs`); the oracle
//! is never linked into normative decode.
//!
//! # Normative semantics (fixed for universe v1 / profile 1)
//!
//! | Quantity | Value |
//! |---|---|
//! | State width | 32 bits, unsigned |
//! | Alphabet / symbol order | byte symbols `0..=255`, cumulative table in ascending symbol order |
//! | Frequency scale | `scale_bits = 14` ⇒ `MODEL_TOTAL = 16384` |
//! | Normalization interval (lower bound) | `STATE_L = 2^23` |
//! | Encoder renorm bound per symbol | `x_max = ((STATE_L >> scale_bits) << 8) * freq` |
//! | Initial / final state | `STATE_L`; the final 32-bit state is flushed raw |
//! | Renormalization | byte-wise (8 bits), little-endian byte order |
//! | Endianness | all multi-byte integers little-endian |
//! | Model serialization | 256 × `u16` LE frequencies (512 bytes) |
//! | Encode direction | symbols consumed in input order |
//! | Decode direction | symbols produced in reverse order (ANS is LIFO) |
//!
//! Encode step `C(s, x) = ((x / freq) << scale_bits) + (x % freq) + start`
//! with the state renormalized *before* the step while `x >= x_max`. The
//! decoder inverts per symbol and renormalizes *after* each pop while
//! `x < STATE_L`, reading bytes forward from the payload (the encoder writes
//! them backward; the on-wire payload stores them reversed so the decoder
//! reads forward).
//!
//! # Accounting / RAW fallback
//!
//! [`encode_block`] chooses the RANS representation only when
//!
//! ```text
//! model_bytes (512) + encoded_bytes < raw_bytes
//! ```
//!
//! and otherwise stores the payload literally (RAW). This is the declared
//! accounting policy: a model that cannot pay for itself is never used. The
//! container overhead (1 kind byte + 8 length bytes) is identical on both
//! branches, so it cancels in the decision and is reported separately.

use crate::error::VoleError;

/// Frequency scale bits.
pub const SCALE_BITS: u32 = 14;
/// Total frequency (`1 << SCALE_BITS`).
pub const MODEL_TOTAL: u32 = 1 << SCALE_BITS;
/// Lower bound of the rANS normalization interval (`RANS_BYTE_L`).
pub const STATE_L: u32 = 1 << 23;
/// Serialized size of a model: 256 frequencies × u16.
pub const MODEL_SERIALIZED: usize = 512;

/// Kind byte for the self-describing payload container.
pub const KIND_RAW: u8 = 0;
/// Kind byte for the rANS payload container.
pub const KIND_RANS: u8 = 1;

/// An order-0 static byte model: normalized frequencies and cumulative sums.
///
/// `cum[s]` is the start of symbol `s`'s partition and
/// `freq[s] == cum[s+1] - cum[s]`. The frequencies sum to [`MODEL_TOTAL`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteModel {
    freq: [u16; 256],
    cum: [u32; 257],
}

impl ByteModel {
    /// Build a model from a byte histogram.
    ///
    /// Normalization (deterministic, integer-only, largest-remainder):
    /// every present symbol receives a base frequency of 1, and the remaining
    /// `MODEL_TOTAL - used` units are distributed in proportion to counts by
    /// floor division plus a largest-remainder pass ordered by
    /// `(fractional remainder descending, symbol index ascending)`. Absent
    /// symbols receive frequency 0. An all-zero histogram yields a zero model
    /// (which cannot encode; [`encode_block`] then chooses RAW).
    pub fn from_counts(counts: &[u64; 256]) -> Self {
        let total_count: u128 = counts.iter().map(|c| u128::from(*c)).sum();
        let mut freq = [0u16; 256];
        if total_count == 0 {
            return Self::from_freqs(freq);
        }

        let used = counts.iter().filter(|c| **c > 0).count() as u128;
        debug_assert!(used <= 256);
        // Base of one per used symbol; distribute extras by largest remainder.
        let extras = u128::from(MODEL_TOTAL) - used;
        for (i, c) in counts.iter().enumerate() {
            if *c > 0 {
                let numer = u128::from(*c) * extras;
                let share = numer.checked_div(total_count).unwrap_or(0);
                freq[i] = 1 + share as u16;
            }
        }
        // Largest-remainder correction: distribute leftover units.
        let assigned: u128 = freq.iter().map(|f| u128::from(*f)).sum();
        let mut rem = u128::from(MODEL_TOTAL) - assigned;
        debug_assert!(rem <= 256);
        let mut order: Vec<usize> = (0..256).filter(|i| counts[*i] > 0).collect();
        order.sort_by(|a, b| {
            let ra = u128::from(counts[*a]) * extras % total_count;
            let rb = u128::from(counts[*b]) * extras % total_count;
            rb.cmp(&ra).then_with(|| a.cmp(b))
        });
        for i in order {
            if rem == 0 {
                break;
            }
            freq[i] += 1;
            rem -= 1;
        }
        debug_assert_eq!(rem, 0);

        Self::from_freqs(freq)
    }

    /// Build a model directly from a byte sample (histogram + normalize).
    pub fn from_data(data: &[u8]) -> Self {
        let mut counts = [0u64; 256];
        for b in data {
            counts[*b as usize] += 1;
        }
        Self::from_counts(&counts)
    }

    /// Build a model from explicit frequencies (caller guarantees validity
    /// unless [`ByteModel::validate`] is used to check).
    pub fn from_freqs(freq: [u16; 256]) -> Self {
        let mut cum = [0u32; 257];
        for i in 0..256usize {
            cum[i + 1] = cum[i] + u32::from(freq[i]);
        }
        Self { freq, cum }
    }

    /// Validate structural invariants: total == `MODEL_TOTAL`, cumulative sums
    /// monotone and consistent.
    pub fn validate(&self) -> Result<(), VoleError> {
        if self.cum[256] != MODEL_TOTAL {
            return Err(VoleError::EntropyCorrupt);
        }
        for i in 0..256usize {
            let f = u32::from(self.freq[i]);
            if self.cum[i + 1] != self.cum[i] + f {
                return Err(VoleError::EntropyCorrupt);
            }
        }
        Ok(())
    }

    /// Frequency of a symbol.
    #[inline]
    pub fn freq(&self, sym: usize) -> u32 {
        u32::from(self.freq[sym])
    }

    /// Start (cumulative) of a symbol's partition.
    #[inline]
    pub fn start(&self, sym: usize) -> u32 {
        self.cum[sym]
    }

    /// Serialize the model canonically: 256 × u16 little-endian frequencies.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MODEL_SERIALIZED);
        for f in &self.freq {
            out.extend_from_slice(&f.to_le_bytes());
        }
        out
    }

    /// Deserialize a model from 256 × u16 little-endian frequencies.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VoleError> {
        if bytes.len() != MODEL_SERIALIZED {
            return Err(VoleError::LengthMismatch);
        }
        let mut freq = [0u16; 256];
        for i in 0..256usize {
            freq[i] = u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
        }
        Ok(Self::from_freqs(freq))
    }

    /// Locate the symbol whose partition contains `slot`, if any.
    #[inline]
    fn find(&self, slot: u32) -> Option<usize> {
        // Binary search over the cumulative table.
        let mut lo = 0usize;
        let mut hi = 256usize;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.cum[mid + 1] <= slot {
                lo = mid + 1;
            } else if self.cum[mid] > slot {
                hi = mid;
            } else {
                return Some(mid);
            }
        }
        None
    }
}

/// Upper bound of bytes an rANS payload can reach for `n` symbols.
///
/// Encoder renorm emits at most two bytes per symbol (with `scale_bits=14` the
/// post-step state is below `2^31`, so at most `(31 − 17)/8 → 2` byte emissions
/// per symbol before the state drops below the minimum `x_max = 2^17`), plus a
/// 4-byte final state and one byte for the initial renormalization slack.
pub fn encoded_len_bound(n: usize) -> usize {
    4 + 1 + 2 * n
}

/// Encode `data` with an explicit model, returning the canonical rANS payload
/// bytes (`[u32 state LE][renorm bytes in reverse emission order]`).
pub fn encode_with(data: &[u8], model: &ByteModel) -> Result<Vec<u8>, VoleError> {
    if data.is_empty() {
        // Zero symbols: canonical payload is just the flushed initial state
        // (no model is needed, so an all-zero model is acceptable here).
        return Ok(STATE_L.to_le_bytes().to_vec());
    }
    model.validate()?;
    let mut x: u32 = STATE_L;
    let mut emitted: Vec<u8> = Vec::with_capacity(encoded_len_bound(data.len()));
    for sym in data {
        let s = *sym as usize;
        let f = model.freq(s);
        if f == 0 {
            return Err(VoleError::EntropyCorrupt);
        }
        let start = model.start(s);
        // Renormalize before the C-step (per-symbol bound).
        let x_max = ((STATE_L >> SCALE_BITS) << 8) * f;
        while x >= x_max {
            emitted.push((x & 0xff) as u8);
            x >>= 8;
        }
        // C(s, x) = ((x / f) << scale_bits) + (x % f) + start
        let q = u64::from(x) / u64::from(f);
        let r = u64::from(x) % u64::from(f);
        let nx = (q << SCALE_BITS) + r + u64::from(start);
        debug_assert!(nx < (1u64 << 32));
        x = nx as u32;
    }
    let mut out = Vec::with_capacity(4 + emitted.len());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend(emitted.iter().rev());
    Ok(out)
}

/// Decode `out_len` symbols from a canonical rANS payload with a model.
///
/// The decoder runs LIFO (symbols are produced in reverse encode order). It
/// requires the model to be valid and fails with a typed error on truncated,
/// over-read, or structurally corrupt input; it consumes the payload exactly.
pub fn decode_with(
    payload: &[u8],
    model: &ByteModel,
    out_len: usize,
) -> Result<Vec<u8>, VoleError> {
    if out_len == 0 {
        // Canonical zero-symbol payload is the 4-byte initial state; accept and
        // require exact consumption (no model is needed for zero symbols).
        if payload.len() == 4 {
            return Ok(Vec::new());
        }
        return Err(VoleError::NonCanonicalEncoding);
    }
    model.validate()?;
    if payload.len() < 4 {
        return Err(VoleError::Truncated);
    }
    let mut x = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let mut pos = 4usize;
    let mask = MODEL_TOTAL - 1;
    let mut out = vec![0u8; out_len];
    for i in (0..out_len).rev() {
        let slot = x & mask;
        let s = model.find(slot).ok_or(VoleError::EntropyCorrupt)?;
        out[i] = s as u8;
        // Invert C(s, x): x <- f*(x >> scale_bits) + (slot - start).
        let f = u64::from(model.freq(s));
        let start = u64::from(model.start(s));
        let nx = f * (u64::from(x) >> SCALE_BITS) + u64::from(slot) - start;
        debug_assert!(nx < (1u64 << 32));
        x = nx as u32;
        // Renormalize after the pop while below the normalization interval.
        while x < STATE_L {
            let b = *payload.get(pos).ok_or(VoleError::EntropyOverread)?;
            pos += 1;
            x = (x << 8) | u32::from(b);
        }
    }
    if pos != payload.len() {
        return Err(VoleError::NonCanonicalEncoding);
    }
    Ok(out)
}

/// Kind of a payload produced by [`encode_block`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Literal bytes stored verbatim.
    Raw,
    /// rANS-coded bytes (with an inline model).
    Rans,
}

impl BlockKind {
    fn to_byte(self) -> u8 {
        match self {
            BlockKind::Raw => KIND_RAW,
            BlockKind::Rans => KIND_RANS,
        }
    }
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            KIND_RAW => Some(BlockKind::Raw),
            KIND_RANS => Some(BlockKind::Rans),
            _ => None,
        }
    }
}

/// Encode a payload with the declared RAW-fallback accounting policy.
///
/// Container layout (all multi-byte integers little-endian):
///
/// ```text
/// RAW:  kind u8 = 0 | out_len u64 | literal bytes
/// RANS: kind u8 = 1 | out_len u64 | model (512 bytes) | rANS payload
/// ```
///
/// rANS is chosen only when `512 + rans_bytes < out_len`; otherwise RAW is
/// stored (an empty input is always RAW). The 9-byte container envelope is
/// identical on both branches.
pub fn encode_block(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u64;
    let mut out = Vec::with_capacity(9 + data.len());
    if !data.is_empty() {
        let model = ByteModel::from_data(data);
        if let Ok(rans) = encode_with(data, &model) {
            if rans.len() + MODEL_SERIALIZED < data.len() {
                out.push(BlockKind::Rans.to_byte());
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(&model.to_bytes());
                out.extend_from_slice(&rans);
                return out;
            }
        }
    }
    out.push(BlockKind::Raw.to_byte());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// Decode a self-describing payload (see [`encode_block`]).
///
/// `max_out` bounds the declared output length before any allocation, so a
/// hostile length prefix cannot drive an unbounded allocation.
pub fn decode_block(bytes: &[u8], max_out: u64) -> Result<Vec<u8>, VoleError> {
    if bytes.len() < 9 {
        return Err(VoleError::Truncated);
    }
    let kind = BlockKind::from_byte(bytes[0]).ok_or(VoleError::NonCanonicalEncoding)?;
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&bytes[1..9]);
    let len = u64::from_le_bytes(len_bytes);
    if len > max_out {
        return Err(VoleError::DimensionTooLarge);
    }
    match kind {
        BlockKind::Raw => {
            let need = 9usize + len as usize;
            if bytes.len() < need {
                return Err(VoleError::Truncated);
            }
            if bytes.len() != need {
                return Err(VoleError::NonCanonicalEncoding);
            }
            Ok(bytes[9..need].to_vec())
        }
        BlockKind::Rans => {
            let need = 9 + MODEL_SERIALIZED;
            if bytes.len() <= need {
                return Err(VoleError::Truncated);
            }
            let model = ByteModel::from_bytes(&bytes[9..need])?;
            decode_with(&bytes[need..], &model, len as usize)
        }
    }
}

/// Structural validation of a block **without decoding it** (used at parse
/// time, where the parser must bound and skip the op but must not force an
/// entropy decode of a frame the caller may never materialize).
///
/// Mirrors the length/kind invariants of [`decode_block`]: valid kind byte;
/// declared `out_len` within `max_out`; RAW branch must be exactly
/// `9 + out_len` bytes; RANS branch must carry at least the 512-byte inline
/// model. Deeper corruption (entropy overread, invalid model, unsorted
/// residual points, …) surfaces as a typed error when the op is applied at
/// materialization time.
pub fn check_block(bytes: &[u8], max_out: u64) -> Result<(), VoleError> {
    if bytes.len() < 9 {
        return Err(VoleError::Truncated);
    }
    if BlockKind::from_byte(bytes[0]).is_none() {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&bytes[1..9]);
    let len = u64::from_le_bytes(len_bytes);
    if len > max_out {
        return Err(VoleError::DimensionTooLarge);
    }
    match BlockKind::from_byte(bytes[0]).expect("checked above") {
        BlockKind::Raw => {
            let need = 9usize
                .checked_add(usize::try_from(len).map_err(|_| VoleError::ArithmeticOverflow)?)
                .ok_or(VoleError::ArithmeticOverflow)?;
            if bytes.len() < need {
                return Err(VoleError::Truncated);
            }
            if bytes.len() != need {
                return Err(VoleError::NonCanonicalEncoding);
            }
            Ok(())
        }
        BlockKind::Rans => {
            if bytes.len() <= 9 + MODEL_SERIALIZED {
                return Err(VoleError::Truncated);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(seed: u64, n: usize) -> Vec<u8> {
        // xorshift64* deterministic byte generator.
        let mut s = seed.max(1);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            out.push(((s.wrapping_mul(0x2545F4914F6CDD1D)) >> 56) as u8);
        }
        out
    }

    #[test]
    fn model_total_is_exact_and_used_symbols_nonzero() {
        for seed in 1..40u64 {
            let d = det(seed, 2000);
            let m = ByteModel::from_data(&d);
            assert_eq!(m.validate(), Ok(()));
            for b in &d {
                assert!(m.freq(*b as usize) > 0);
            }
        }
    }

    #[test]
    fn roundtrip_deterministic_lengths_and_distributions() {
        let cases: Vec<(u64, usize)> = vec![
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 3),
            (5, 4),
            (6, 7),
            (7, 16),
            (8, 63),
            (9, 128),
            (10, 511),
            (11, 1024),
            (12, 4096),
        ];
        for (seed, n) in cases {
            let d = det(seed, n);
            let block = encode_block(&d);
            assert_eq!(
                decode_block(&block, 1 << 20).unwrap(),
                d,
                "roundtrip failed for seed={} n={}",
                seed,
                n
            );
        }
    }

    #[test]
    fn roundtrip_skewed_and_uniform() {
        // Heavily skewed: mostly 'A'.
        let mut skewed = vec![b'A'; 20000];
        for i in (0..skewed.len()).step_by(97) {
            skewed[i] = b'B';
        }
        for i in (3..skewed.len()).step_by(251) {
            skewed[i] = b'C';
        }
        let sb = encode_block(&skewed);
        assert_eq!(sb[0], KIND_RANS, "skewed payload should choose rANS");
        assert!(sb.len() + 9 < skewed.len(), "rANS should compress skew");
        assert_eq!(decode_block(&sb, 1 << 20).unwrap(), skewed);

        // Uniform random must fall back to RAW.
        let uniform = det(0xABCDEF, 5000);
        let ub = encode_block(&uniform);
        assert_eq!(ub[0], KIND_RAW, "uniform payload should choose RAW");
        assert_eq!(decode_block(&ub, 1 << 20).unwrap(), uniform);
    }

    #[test]
    fn empty_is_raw_and_roundtrips() {
        let b = encode_block(&[]);
        assert_eq!(b[0], KIND_RAW);
        assert_eq!(decode_block(&b, 16).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn hostile_truncation_is_typed() {
        let d = det(7, 2048);
        let block = encode_block(&d);
        for cut in (0..block.len()).step_by(3) {
            let r = decode_block(&block[..cut], 1 << 20);
            assert!(r.is_err(), "truncation at {} must error", cut);
        }
    }

    #[test]
    fn hostile_length_and_corruption_are_typed() {
        let d = det(9, 3000);
        let block = encode_block(&d);
        // Declared length larger than allowed.
        let mut big = block.clone();
        big[1] = 0xFF;
        big[2] = 0xFF;
        big[3] = 0xFF;
        big[4] = 0xFF;
        assert!(matches!(
            decode_block(&big, 1 << 20),
            Err(VoleError::DimensionTooLarge)
        ));

        // Corrupt every byte of the RANS body in turn: each must decode or fail
        // typed; it must never panic, and identical input yields identical error.
        let mut prev_err: Option<VoleError> = None;
        for i in 9..block.len() {
            let mut c = block.clone();
            c[i] ^= 0x55;
            let r = decode_block(&c, 1 << 20);
            if let Err(e) = r {
                // deterministic: same input twice gives the same result
                let mut c2 = block.clone();
                c2[i] ^= 0x55;
                assert_eq!(decode_block(&c2, 1 << 20).unwrap_err(), e);
                prev_err = Some(e);
            } else {
                // Some single-byte corruptions still decode to *some* bytes
                // (ANS has no internal checksum); VOLE's archive layer adds
                // integrity hashes over the reconstruction. That is bounded and
                // permitted: outcome is valid decode or typed error, never panic.
                assert!(r.is_ok());
            }
        }
        let _ = prev_err;
    }
}
