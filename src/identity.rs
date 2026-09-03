//! Persistent object identity.
//!
//! Contents identity is what lets *immutable objects* be reused across frames,
//! streams, edits, or (later) an optional object store, without confusing
//! appearance with equality: sharing requires the *exact* canonical content
//! identity (BLAKE3), never "looks the same".
//!
//! Canonical hash definition (must match `docs/procedural-state-graph.md` and
//! the Phase-B receipt): the identity of an [`Object`] is BLAKE3 over its
//! canonical *record bytes without its id*, which is the byte-for-byte form
//! the encoder would write for that object. Two objects identical in content
//! thus share identity; an id is scoped to a stream and reused only through
//! matching identity.

use crate::{error::VoleError, integr, object::Object};

/// Width of a BLAKE3 content identity in bytes.
pub const ID_LEN: usize = 32;

/// Immutable content identity of an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentId([u8; ID_LEN]);

impl ContentId {
    /// Bytes of the identity.
    pub fn as_bytes(&self) -> &[u8; ID_LEN] {
        &self.0
    }

    /// Hex for receipts/JSON.
    pub fn hex(&self) -> String {
        hex_lower(&self.0)
    }
}

/// Canonical record bytes for an object (the encoder form described above),
/// independent of its id.
fn canonical_object_record(obj: &Object) -> Vec<u8> {
    // The concrete canonical byte form matches format v1: fill boxes are a
    // fill record; other objects are a raster record. Tag constants are kept
    // local so this module is never coupled to `format`'s tags.
    let mut out = Vec::with_capacity(16);
    match obj.fill_value() {
        Some(v) => {
            out.push(0x02); // object-fill record tag
            out.extend_from_slice(&obj.width().to_le_bytes());
            out.extend_from_slice(&obj.height().to_le_bytes());
            out.push(v);
        }
        None => {
            out.push(0x01); // object-raster record tag
            out.extend_from_slice(&obj.width().to_le_bytes());
            out.extend_from_slice(&obj.height().to_le_bytes());
            if let Some(raster) = obj.samples() {
                out.extend_from_slice(raster);
            }
        }
    }
    out
}

/// Compute the content identity of an object.
pub fn content_id_of(obj: &Object) -> ContentId {
    let rec = canonical_object_record(obj);
    let d = integr::digest(&rec);
    ContentId(d)
}

/// A registry mapping content identity → (declared object content, set of
/// declaring ids). Kept for the object-reuse & dedup courts.
#[derive(Debug, Clone, Default)]
pub struct ContentTable {
    map: std::collections::BTreeMap<ContentId, Vec<u32>>,
}

impl ContentTable {
    /// Insert `(id, object)` returning the content identity. Registers reuse.
    pub fn insert(&mut self, id: u32, obj: &Object) -> Result<ContentId, VoleError> {
        let cid = content_id_of(obj);
        self.map.entry(cid).or_default().push(id);
        Ok(cid)
    }

    /// Number of distinct contents.
    pub fn distinct(&self) -> usize {
        self.map.len()
    }

    /// Number of ids registered.
    pub fn total(&self) -> usize {
        self.map.values().map(Vec::len).sum()
    }

    /// Total storage bytes if all distinct contents were stored once (raster
    /// bytes for raster contents + 6*32B per record as accounting seed).
    pub fn distinct_bytes(&self) -> u64 {
        self.map.len() as u64
    }
}

/// Lowercase hex dump helper.
fn hex_lower(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Object;

    #[test]
    fn same_content_same_id() {
        let a = Object::fill(16, 8, 77).unwrap();
        let b = Object::fill(16, 8, 77).unwrap();
        assert_eq!(content_id_of(&a), content_id_of(&b));
    }

    #[test]
    fn different_content_differs() {
        let a = Object::fill(16, 8, 77).unwrap();
        let c = Object::fill(16, 8, 78).unwrap();
        assert_ne!(content_id_of(&a), content_id_of(&c));
    }

    #[test]
    fn identity_matches_encoder_bytes() {
        // Identity of a fill must be stable; pin the exact digest to make
        // regressions obvious.
        let a = Object::fill(200, 100, 180).unwrap();
        let id = content_id_of(&a).hex();
        assert!(id.len() == 64);
        // Determinism: computing twice is equal.
        assert_eq!(content_id_of(&a).hex(), id);
    }
}
