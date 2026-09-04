//! Canonical sample planes — Phase V.1.1 (V.1 brief §15–§18, contract §2.4).
//!
//! Canonical storage is **planar, tight, little-endian, no stride padding**:
//! ≤ 8 active bits per sample live in a `u8` plane; 9..=16 active bits live in
//! a `u16` plane whose active bits are the **low** bits (padding high bits
//! must be zero — enforced). Sample payloads are validated against the
//! declared geometry and bit depth at construction, so a [`Plane`] value is
//! canonical by type. Float sample sources (F16/F32) are out of V.1.1 scope
//! and will fall to an exact opaque raw-bit plane in a later subphase — never
//! a silent quantization.

use crate::error::VoleError;

/// Active sample depth in bits (1..=16). 8-bit content uses `u8` storage;
/// 9..=16-bit content uses `u16` storage with the sample in the low bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitDepth(u8);

impl BitDepth {
    /// A depth of `bits ∈ 1..=16`.
    pub fn new(bits: u8) -> Result<Self, VoleError> {
        if bits == 0 || bits > 16 {
            return Err(VoleError::InvalidSamples);
        }
        Ok(BitDepth(bits))
    }

    /// Active bit count.
    pub fn bits(self) -> u8 {
        self.0
    }

    /// Whether samples fit one byte (≤ 8 bits).
    pub fn is_byte_depth(self) -> bool {
        self.0 <= 8
    }

    /// The canonical storage width for this depth.
    pub fn storage(self) -> PlaneStorage {
        if self.is_byte_depth() {
            PlaneStorage::U8
        } else {
            PlaneStorage::U16
        }
    }

    /// The largest canonical sample value at this depth (`2^bits − 1`).
    pub fn max_sample(self) -> u32 {
        (1u32 << self.0) - 1
    }

    /// Whether `sample` is inside the active range of this depth (also the
    /// padding-bit rule for `u16` storage: high bits above the active depth
    /// must be zero).
    pub fn contains(self, sample: u32) -> bool {
        sample <= self.max_sample()
    }
}

/// Canonical storage width of a plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaneStorage {
    /// One byte per sample (depth ≤ 8).
    U8,
    /// One `u16` per sample (depth 9..=16), little-endian on the wire.
    U16,
}

impl PlaneStorage {
    /// Bytes per stored sample.
    pub fn bytes_per_sample(self) -> u64 {
        match self {
            PlaneStorage::U8 => 1,
            PlaneStorage::U16 => 2,
        }
    }
}

/// The tightly packed canonical sample payload of a plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaneData {
    /// `u8` samples (depth ≤ 8).
    U8(Vec<u8>),
    /// `u16` samples (depth 9..=16), active bits in the low bits.
    U16(Vec<u16>),
}

impl PlaneData {
    /// Sample count.
    pub fn len(&self) -> usize {
        match self {
            PlaneData::U8(v) => v.len(),
            PlaneData::U16(v) => v.len(),
        }
    }

    /// Whether the payload is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A canonical sample plane: named component, declared geometry (already the
/// subsampling-correct dimensions of its layout), bit depth, subsampling
/// exponents (informational; the authoritative geometry is `width`/`height`),
/// and the validated tight payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane {
    component: crate::media::layout::Component,
    width: u32,
    height: u32,
    bit_depth: BitDepth,
    subsample_x: u8,
    subsample_y: u8,
    data: PlaneData,
}

impl Plane {
    /// Build a canonical plane. Validates:
    ///
    /// * nonzero geometry and checked sample count;
    /// * payload length == `width × height`;
    /// * payload storage matches the depth (`u8` for ≤ 8 bits, `u16` for
    ///   9..=16 bits) — `InvalidSamples` otherwise;
    /// * every `u16` sample is inside the active depth (`InvalidSamples` on
    ///   any padding-bit violation);
    /// * subsampling exponents ≤ 3.
    pub fn new(
        component: crate::media::layout::Component,
        width: u32,
        height: u32,
        bit_depth: BitDepth,
        subsample_x: u8,
        subsample_y: u8,
        data: PlaneData,
    ) -> Result<Self, VoleError> {
        if width == 0 || height == 0 || subsample_x > 3 || subsample_y > 3 {
            return Err(VoleError::GeometryMismatch);
        }
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(VoleError::ArithmeticOverflow)?;
        match &data {
            PlaneData::U8(v) => {
                if !bit_depth.is_byte_depth() {
                    return Err(VoleError::InvalidSamples);
                }
                if v.len() as u64 != count {
                    return Err(VoleError::InvalidSamples);
                }
            }
            PlaneData::U16(v) => {
                if bit_depth.is_byte_depth() {
                    return Err(VoleError::InvalidSamples);
                }
                if v.len() as u64 != count {
                    return Err(VoleError::InvalidSamples);
                }
                let max = bit_depth.max_sample();
                if v.iter().any(|s| u32::from(*s) > max) {
                    return Err(VoleError::InvalidSamples);
                }
            }
        }
        Ok(Plane {
            component,
            width,
            height,
            bit_depth,
            subsample_x,
            subsample_y,
            data,
        })
    }

    /// Build a canonical plane from its **tight canonical byte form**:
    /// row-major samples, little-endian `u16` words for depths > 8. This is
    /// the deterministic packing [`Plane::canonical_bytes`] produces and the
    /// form hashing/export operate on.
    pub fn from_canonical_bytes(
        component: crate::media::layout::Component,
        width: u32,
        height: u32,
        bit_depth: BitDepth,
        subsample_x: u8,
        subsample_y: u8,
        bytes: &[u8],
    ) -> Result<Self, VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::GeometryMismatch);
        }
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(VoleError::ArithmeticOverflow)?;
        let storage = bit_depth.storage();
        let want = count
            .checked_mul(storage.bytes_per_sample())
            .ok_or(VoleError::ArithmeticOverflow)?;
        if bytes.len() as u64 != want {
            return Err(VoleError::InvalidSamples);
        }
        let data = match storage {
            PlaneStorage::U8 => PlaneData::U8(bytes.to_vec()),
            PlaneStorage::U16 => {
                let max = bit_depth.max_sample();
                let words: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                if words.iter().any(|s| u32::from(*s) > max) {
                    return Err(VoleError::InvalidSamples);
                }
                PlaneData::U16(words)
            }
        };
        Self::new(
            component,
            width,
            height,
            bit_depth,
            subsample_x,
            subsample_y,
            data,
        )
    }

    /// Component.
    pub fn component(&self) -> crate::media::layout::Component {
        self.component
    }

    /// Width in samples.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in samples.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Sample count (`width × height`).
    pub fn sample_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Active bit depth.
    pub fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    /// Horizontal subsampling exponent (informational).
    pub fn subsample_x(&self) -> u8 {
        self.subsample_x
    }

    /// Vertical subsampling exponent (informational).
    pub fn subsample_y(&self) -> u8 {
        self.subsample_y
    }

    /// Storage width.
    pub fn storage(&self) -> PlaneStorage {
        self.bit_depth.storage()
    }

    /// The tight canonical byte form: row-major, `u16` words little-endian
    /// for depths > 8. Deterministic and length-exact
    /// (`sample_count × bytes_per_sample`).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        match &self.data {
            PlaneData::U8(v) => v.clone(),
            PlaneData::U16(v) => {
                let mut out = Vec::with_capacity(v.len() * 2);
                for s in v {
                    out.extend_from_slice(&s.to_le_bytes());
                }
                out
            }
        }
    }

    /// Raw sample payload.
    pub fn data(&self) -> &PlaneData {
        &self.data
    }

    /// Consume into parts.
    pub fn into_parts(self) -> (u32, u32, BitDepth, PlaneData) {
        (self.width, self.height, self.bit_depth, self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::layout::Component;

    #[test]
    fn storage_width_follows_depth() {
        for bits in 1..=16u8 {
            let d = BitDepth::new(bits).unwrap();
            assert_eq!(d.is_byte_depth(), bits <= 8);
            assert_eq!(
                d.storage(),
                if bits <= 8 {
                    PlaneStorage::U8
                } else {
                    PlaneStorage::U16
                }
            );
            assert_eq!(d.max_sample(), (1u32 << bits) - 1);
        }
        assert!(BitDepth::new(0).is_err());
        assert!(BitDepth::new(17).is_err());
    }

    #[test]
    fn plane_validation_is_typed() {
        let d8 = BitDepth::new(8).unwrap();
        let d10 = BitDepth::new(10).unwrap();
        // Correct 8-bit plane.
        let p = Plane::new(
            Component::Gray,
            2,
            2,
            d8,
            0,
            0,
            PlaneData::U8(vec![0, 1, 2, 3]),
        )
        .unwrap();
        assert_eq!(p.canonical_bytes(), vec![0, 1, 2, 3]);
        // Length mismatch.
        assert_eq!(
            Plane::new(
                Component::Gray,
                2,
                2,
                d8,
                0,
                0,
                PlaneData::U8(vec![0, 1, 2])
            )
            .unwrap_err(),
            VoleError::InvalidSamples
        );
        // u16 storage at 8-bit depth is refused.
        assert_eq!(
            Plane::new(Component::Gray, 1, 1, d8, 0, 0, PlaneData::U16(vec![1])).unwrap_err(),
            VoleError::InvalidSamples
        );
        // u8 storage at 10-bit depth is refused.
        assert_eq!(
            Plane::new(Component::Gray, 1, 1, d10, 0, 0, PlaneData::U8(vec![1])).unwrap_err(),
            VoleError::InvalidSamples
        );
        // 10-bit sample above the active range (padding bits set) is refused.
        assert_eq!(
            Plane::new(
                Component::Gray,
                1,
                1,
                d10,
                0,
                0,
                PlaneData::U16(vec![1 << 10])
            )
            .unwrap_err(),
            VoleError::InvalidSamples
        );
        // Zero geometry is refused.
        assert_eq!(
            Plane::new(Component::Gray, 0, 1, d8, 0, 0, PlaneData::U8(vec![])).unwrap_err(),
            VoleError::GeometryMismatch
        );
    }

    #[test]
    fn canonical_bytes_roundtrip_exactly() {
        for bits in [8u8, 9, 10, 12, 14, 16] {
            let d = BitDepth::new(bits).unwrap();
            let max = d.max_sample();
            let values: Vec<u32> = (0..16u32).map(|i| i * max / 15).collect();
            let p = if bits <= 8 {
                let data: Vec<u8> = values.iter().map(|v| *v as u8).collect();
                Plane::new(Component::Cb, 4, 4, d, 1, 1, PlaneData::U8(data)).unwrap()
            } else {
                let words: Vec<u16> = values.iter().map(|v| *v as u16).collect();
                Plane::new(Component::Cb, 4, 4, d, 1, 1, PlaneData::U16(words)).unwrap()
            };
            let bytes = p.canonical_bytes();
            let per = d.storage().bytes_per_sample() as usize;
            assert_eq!(bytes.len(), 16 * per);
            let back = Plane::from_canonical_bytes(Component::Cb, 4, 4, d, 1, 1, &bytes).unwrap();
            assert_eq!(back.canonical_bytes(), bytes);
            assert_eq!(back.sample_count(), 16);
            assert_eq!(back.storage(), d.storage());
        }
        // A truncated canonical byte form is refused.
        let d10 = BitDepth::new(10).unwrap();
        assert_eq!(
            Plane::from_canonical_bytes(Component::Y, 2, 2, d10, 0, 0, &[0; 7]).unwrap_err(),
            VoleError::InvalidSamples
        );
        // Padding-bit violations survive the byte round trip as typed errors.
        assert_eq!(
            Plane::from_canonical_bytes(Component::Y, 1, 1, d10, 0, 0, &(1u16 << 10).to_le_bytes())
                .unwrap_err(),
            VoleError::InvalidSamples
        );
    }
}
