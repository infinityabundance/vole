//! Immutable content objects and their identity.
//!
//! An **object** is immutable visual/structural content that can be reused
//! across time (and, later phases, across streams). In phase A an object is
//! precisely a bounded Gray8 raster patch plus its geometry; the identity is
//! the id that is declared in the object table and referenced by instances.
//!
//! Distinguish:
//! * an immutable [`Object`] (content; declared once),
//! * an [`ObjectId`] (locally scoped, canonical index space), and
//! * an *instance* (a mutable-by-transition placement of an object in a state).
//!
//! The separation mirrors the architecture: content is content-addressed
//! territory for later phases; instances drive transitions and materialize.

use crate::error::VoleError;

/// Local object identity in format-v1 index space.
///
/// v1 u32 index. The zero value is unused; ids are assigned on declaration and
/// are dense only insofar as the encoder chooses — the model never assumes
/// nil references may be implicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub u32);

/// Immutable object content kind in format v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKindKind {
    /// A uniform fill of a declared rectangle. Stores one sample.
    Fill,
    /// A literal Gray8 raster, tight row-major.
    RawRaster,
    /// A palette-index raster (Phase J): tight row-major **indices** into a
    /// palette bound to the painting instance; the materializer maps every
    /// index through the active palette to produce Gray8 samples. Indices are
    /// one byte, so an index raster is a *structure* view: the same index
    /// plane re-renders with different gray values as the palette mutates.
    IndexRaster,
    /// A bounded procedural content program (Phase N): samples are computed
    /// at materialization from the canonical integer program (gradient /
    /// checker / periodic sawtooth / seeded noise), never stored. The box
    /// geometry is the painted extent; work is one sample per painted pixel.
    Generator,
    // Later phases: EXACT_REF, PALETTE, DICTIONARY, RESIDUAL_TEMPLATE
}

/// A declared immutable object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    kind: ObjectKindKind,
    width: u32,
    height: u32,
    /// Raster samples (RawRaster), index samples (IndexRaster), or one byte
    /// (Fill): length == w*h for raster/index kinds, 1 for Fill. Empty for
    /// Generator objects (the program lives in `gen`).
    storage: Vec<u8>,
    /// The bounded procedural program (Phase N); `None` for stored kinds.
    gen: Option<crate::generator::Generator>,
}

impl Object {
    /// Object whose raster is a single uniform gray of `value`.
    pub fn fill(width: u32, height: u32, value: u8) -> Result<Self, VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::DimensionTooLarge);
        }
        Ok(Self {
            kind: ObjectKindKind::Fill,
            width,
            height,
            storage: vec![value],
            gen: None,
        })
    }

    /// Object carrying explicit Gray8 raster content.
    pub fn raster(width: u32, height: u32, data: Vec<u8>) -> Result<Self, VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::DimensionTooLarge);
        }
        let n = u64::from(width) * u64::from(height);
        if data.len() as u64 != n {
            return Err(VoleError::LengthMismatch);
        }
        Ok(Self {
            kind: ObjectKindKind::RawRaster,
            width,
            height,
            storage: data,
            gen: None,
        })
    }

    /// Object carrying a palette-index raster (Phase J): every stored byte is
    /// an index into the palette bound to the painting instance (valid range
    /// `0..palette_len`, enforced at materialization). Geometry/length rules
    /// mirror [`Object::raster`].
    pub fn index_raster(width: u32, height: u32, data: Vec<u8>) -> Result<Self, VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::DimensionTooLarge);
        }
        let n = u64::from(width) * u64::from(height);
        if data.len() as u64 != n {
            return Err(VoleError::LengthMismatch);
        }
        Ok(Self {
            kind: ObjectKindKind::IndexRaster,
            width,
            height,
            storage: data,
            gen: None,
        })
    }

    /// Object carrying a bounded procedural content program (Phase N): every
    /// sample of the declared `width x height` box is **computed** at
    /// materialization by the canonical integer program. Geometry rules mirror
    /// [`Object::raster`]; the program parameters are validated canonically.
    pub fn procedural(
        width: u32,
        height: u32,
        gen: crate::generator::Generator,
    ) -> Result<Self, VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::DimensionTooLarge);
        }
        gen.check()?;
        Ok(Self {
            kind: ObjectKindKind::Generator,
            width,
            height,
            storage: Vec::new(),
            gen: Some(gen),
        })
    }

    /// The bounded procedural program of a Generator object, if any.
    pub fn generator(&self) -> Option<crate::generator::Generator> {
        self.gen
    }

    /// Object construction kind.
    pub fn kind(&self) -> ObjectKindKind {
        self.kind
    }

    /// Raster width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Raster height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Sample count.
    pub fn sample_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Materialize the object's samples into a tight row-major buffer of
    /// `width*height` bytes. Cheap for Fill (replicates), cheap for RawRaster
    /// and IndexRaster (both store their content tightly); a Generator object
    /// renders its program (bounded by the box area).
    pub fn expand(&self) -> Vec<u8> {
        match self.kind {
            ObjectKindKind::Fill => {
                let n = (self.width as usize) * (self.height as usize);
                vec![self.storage[0]; n]
            }
            ObjectKindKind::RawRaster | ObjectKindKind::IndexRaster => self.storage.clone(),
            ObjectKindKind::Generator => {
                let (w, h) = (i64::from(self.width), i64::from(self.height));
                let mut out = Vec::with_capacity((self.width as usize) * (self.height as usize));
                let gen = self.gen.expect("generator object carries a program");
                for y in 0..h {
                    for x in 0..w {
                        out.push(gen.sample(x, y));
                    }
                }
                out
            }
        }
    }

    /// Reference to the object's canonical expanded bytes without copying.
    /// For Fill this returns a view of a single sample repeated logically;
    /// we return None so callers use `expand` explicitly when they need a
    /// physical rectangle. (Kept minimal: materializer uses the table façade.)
    pub fn fill_value(&self) -> Option<u8> {
        match self.kind {
            ObjectKindKind::Fill => Some(self.storage[0]),
            ObjectKindKind::RawRaster | ObjectKindKind::IndexRaster | ObjectKindKind::Generator => {
                None
            }
        }
    }

    /// Tight row-major Gray8 raster bytes *iff* this object already stores its
    /// expanded Gray8 samples (RawRaster). Returns `None` for fills (which
    /// expand on demand) and for index rasters (whose bytes are palette
    /// indices, not samples — see [`Object::indices`]).
    pub fn samples(&self) -> Option<&[u8]> {
        match self.kind {
            ObjectKindKind::RawRaster => Some(&self.storage),
            ObjectKindKind::Fill | ObjectKindKind::IndexRaster | ObjectKindKind::Generator => None,
        }
    }

    /// Tight row-major palette-index bytes *iff* this object is an index
    /// raster (Phase J).
    pub fn indices(&self) -> Option<&[u8]> {
        match self.kind {
            ObjectKindKind::IndexRaster => Some(&self.storage),
            ObjectKindKind::Fill | ObjectKindKind::RawRaster | ObjectKindKind::Generator => None,
        }
    }

    /// Parse an object from its **canonical record bytes** — the byte-for-byte
    /// form `[kind:u8][width:u32][height:u32][payload]` that
    /// `identity::canonical_object_record` produces, that
    /// [`crate::identity::content_id_of`] hashes, and that the Phase-P object
    /// store holds under that content id. Kinds mirror the v1 declaration tags
    /// (`0x02` fill, `0x01` Gray8 raster, `0x05` palette-index raster, `0x07`
    /// generator). Geometry and payload length are validated against `limits`;
    /// an unknown kind, trailing bytes, or out-of-domain program parameters are
    /// typed errors. This is the rehydration path for external object
    /// declarations (Phase P): the store returns a record, the caller verifies
    /// its digest against the declared content id, and the materializer sees
    /// an ordinary [`Object`] — provenance never leaks past the store
    /// abstraction.
    pub fn from_canonical_record(
        bytes: &[u8],
        limits: &crate::limits::Limits,
    ) -> Result<Self, VoleError> {
        let mut r = crate::checked::ByteReader::new(bytes);
        let kind = r.u8()?;
        let w = r.pull::<u32>()?;
        let h = r.pull::<u32>()?;
        if w == 0 || h == 0 {
            return Err(VoleError::DimensionTooLarge);
        }
        let n = u64::from(w)
            .checked_mul(u64::from(h))
            .ok_or(VoleError::ArithmeticOverflow)?;
        if n > limits.max_object_bytes {
            return Err(VoleError::DimensionTooLarge);
        }
        let obj = match kind {
            0x01 => {
                let data = r.take_vec(n as usize)?;
                Object::raster(w, h, data)?
            }
            0x02 => {
                let v = r.u8()?;
                Object::fill(w, h, v)?
            }
            0x05 => {
                let data = r.take_vec(n as usize)?;
                Object::index_raster(w, h, data)?
            }
            0x07 => {
                let gen = crate::generator::Generator::parse_program(&mut r)?;
                Object::procedural(w, h, gen)?
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        };
        if r.remaining() != 0 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        Ok(obj)
    }
}
