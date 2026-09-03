//! Integrity trailer (a [`VoleError`] to keep module a code unit; an actual
//! VOLE `INTEGRITY` record commits to a BLAKE3 digest of the stream that
//! precedes it).

use crate::{
    checked::{ByteSink, Res},
    error::VoleError,
};

/// Digest length in bytes.
pub const DIGEST_LEN: usize = 32;

/// Compute the BLAKE3 digest of `data`.
pub fn digest(data: &[u8]) -> [u8; DIGEST_LEN] {
    let h = blake3::hash(data);
    let mut out = [0u8; DIGEST_LEN];
    out.copy_from_slice(h.as_bytes());
    out
}

/// Append the canonical integrity trailer to a body. The digest commits to
/// every byte written into `body` before this call.
pub fn append_trailer(body: &mut ByteSink) -> Res<()> {
    let d = digest(body.as_slice());
    for b in d {
        body.byte(b)?;
    }
    Ok(())
}

/// Verify that the last `DIGEST_LEN` bytes of `bytes` are the BLAKE3 digest of
/// the prefix. Returns the length of the trusted payload on success.
pub fn verify_trailer(bytes: &[u8]) -> Result<usize, VoleError> {
    if bytes.len() < DIGEST_LEN {
        return Err(VoleError::Truncated);
    }
    let (payload, trailer) = bytes.split_at(bytes.len() - DIGEST_LEN);
    let d = digest(payload);
    if trailer == d {
        Ok(payload.len())
    } else {
        Err(VoleError::IntegrityMismatch)
    }
}
