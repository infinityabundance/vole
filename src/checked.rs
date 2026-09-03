//! Checked arithmetic and small byte-order primitives for the canonical wire
//! format and materialization. Normative reconstruction never depends on
//! unspecified wrapping or platform integer width; all counting that feeds
//! allocation or dependency depth is checked.

use crate::{error::VoleError, limits::Limits};

/// Result alias scoped to the crate's error surface.
pub type Res<T> = Result<T, VoleError>;

/// Checked `mul` returning the typed overflow error.
#[inline]
pub fn checked_mul_u32(a: u32, b: u32) -> Res<u32> {
    a.checked_mul(b).ok_or(VoleError::ArithmeticOverflow)
}

/// Checked widening product `a * b` into `u64` (safe on 32- and 64-bit hosts).
#[inline]
pub fn widening_mul_u64(a: u64, b: u64) -> u128 {
    u128::from(a) * u128::from(b)
}

/// The number of samples for a Gray8 canvas of the given dimensions, checked
/// against the enclosing limits.
#[inline]
pub fn gray_sample_count(w: u32, h: u32, limits: &Limits) -> Res<u64> {
    if w == 0 || h == 0 {
        return Err(VoleError::DimensionTooLarge);
    }
    if u64::from(w) > u64::from(limits.max_width) || u64::from(h) > u64::from(limits.max_height) {
        return Err(VoleError::DimensionTooLarge);
    }
    let n = u64::from(w) * u64::from(h);
    if n > limits.max_canvas_bytes {
        return Err(VoleError::DimensionTooLarge);
    }
    Ok(n)
}

/// Write primitives over a byte slice. Read/write are little-endian, the
/// canonical endianness of format v1. Length prefixes are fixed width.
pub trait Wire: private::Sealed + Copy + Sized {
    /// Number of bytes this type occupies canonically.
    const SIZE: usize;
    /// Encode into the byte slice (length must equal `Self::SIZE`).
    fn encode(self, out: &mut [u8]);
    /// Decode from the byte slice (length must equal `Self::SIZE`).
    fn decode(input: &[u8]) -> Self;
}

mod private {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for u32 {}
    impl Sealed for u64 {}
    impl Sealed for i32 {}
}

impl Wire for u8 {
    const SIZE: usize = 1;
    fn encode(self, out: &mut [u8]) {
        out[0] = self;
    }
    fn decode(input: &[u8]) -> Self {
        input[0]
    }
}
impl Wire for u16 {
    const SIZE: usize = 2;
    fn encode(self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_le_bytes());
    }
    fn decode(input: &[u8]) -> Self {
        u16::from_le_bytes([input[0], input[1]])
    }
}
impl Wire for u32 {
    const SIZE: usize = 4;
    fn encode(self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_le_bytes());
    }
    fn decode(input: &[u8]) -> Self {
        u32::from_le_bytes([input[0], input[1], input[2], input[3]])
    }
}
impl Wire for u64 {
    const SIZE: usize = 8;
    fn encode(self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_le_bytes());
    }
    fn decode(input: &[u8]) -> Self {
        u64::from_le_bytes([
            input[0], input[1], input[2], input[3], input[4], input[5], input[6], input[7],
        ])
    }
}
impl Wire for i32 {
    const SIZE: usize = 4;
    fn encode(self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_le_bytes());
    }
    fn decode(input: &[u8]) -> Self {
        i32::from_le_bytes([input[0], input[1], input[2], input[3]])
    }
}

pub struct ByteSink {
    buf: Vec<u8>,
    /// Hard cell above which the sink refuses to grow (0 = guarded only by
    /// declared bound where callers pre-size; see callers).
    cap: Option<usize>,
}

impl ByteSink {
    /// New sink with no extra hard cap (bounding handled at call sites and by
    /// limits on bytes we know are canonical).
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            cap: None,
        }
    }

    /// New sink that fails once its internal buffer would exceed `cap`.
    pub fn with_cap(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap.min(1 << 20)),
            cap: Some(cap),
        }
    }

    /// Push a single byte.
    pub fn byte(&mut self, b: u8) -> Res<()> {
        if let Some(c) = self.cap {
            if self.buf.len() >= c {
                return Err(VoleError::ApiConstraint("sink exceeded declared capacity"));
            }
        }
        self.buf.push(b);
        Ok(())
    }

    /// Push a little-endian-encoded fixed integer.
    pub fn push<T: Wire>(&mut self, value: T) -> Res<()> {
        // Encode the full little-endian scalar into a fixed 8-byte scratch,
        // then push exactly the canonical low bytes for this type width.
        let mut tmp = [0u8; 8];
        value.encode(&mut tmp[..T::SIZE]);
        for b in &tmp[..T::SIZE] {
            self.byte(*b)?;
        }
        Ok(())
    }

    /// Extend from a buffer, checked against the capacity cell.
    pub fn extend(&mut self, bytes: &[u8]) -> Res<()> {
        if let Some(c) = self.cap {
            if self
                .buf
                .len()
                .checked_add(bytes.len())
                .ok_or(VoleError::ArithmeticOverflow)?
                > c
            {
                return Err(VoleError::ApiConstraint("sink exceeded declared capacity"));
            }
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    /// Borrow the written bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    /// Consume and return the written bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    /// Current length.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl Default for ByteSink {
    fn default() -> Self {
        Self::new()
    }
}

/// Read primitives over a cursor, always failing typed rather than panicking
/// on truncated input.
pub struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    /// New reader over a slice.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Current position.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Remaining bytes.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Number of bytes required for the read is present.
    pub fn has(&self, n: usize) -> bool {
        self.remaining() >= n
    }

    /// Skip exactly `n` bytes.
    pub fn skip(&mut self, n: usize) -> Res<()> {
        if self.remaining() < n {
            return Err(VoleError::Truncated);
        }
        self.pos += n;
        Ok(())
    }

    /// Read a single byte.
    pub fn u8(&mut self) -> Res<u8> {
        let v = self.byte().ok_or(VoleError::Truncated)?;
        Ok(v)
    }

    fn byte(&mut self) -> Option<u8> {
        if self.pos >= self.data.len() {
            return None;
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Some(b)
    }

    /// Read a fixed integer of type `T`.
    pub fn pull<T: Wire>(&mut self) -> Res<T> {
        if self.remaining() < T::SIZE {
            return Err(VoleError::Truncated);
        }
        let mut tmp = [0u8; 8];
        tmp[..T::SIZE].copy_from_slice(&self.data[self.pos..self.pos + T::SIZE]);
        self.pos += T::SIZE;
        Ok(T::decode(&tmp[..T::SIZE]))
    }

    /// Read `n` bytes into a fresh allocation.
    pub fn take_vec(&mut self, n: usize) -> Res<Vec<u8>> {
        if self.remaining() < n {
            return Err(VoleError::Truncated);
        }
        let v = self.data[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(v)
    }

    /// Borrow the next `n` bytes without advancing (for hashing). Errors typed.
    pub fn peek(&self, n: usize) -> Res<&'a [u8]> {
        if self.remaining() < n {
            return Err(VoleError::Truncated);
        }
        Ok(&self.data[self.pos..self.pos + n])
    }

    /// Borrow the next `n` bytes and advance.
    pub fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        if self.remaining() < n {
            return Err(VoleError::Truncated);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
}
