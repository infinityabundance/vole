//! Phase P — the optional content-addressed persistence substrate
//! (`ObjectStore`), cross-video exact-object sharing, physical-vs-declared
//! accounting, and GC closure.
//!
//! # Architecture (master brief §1 / §45 / §46, ADR-0004)
//!
//! EntropyFS (the persistence substrate) is deliberately **optional**:
//! a standalone `.vole` stream decodes without any store. This module owns the
//! *abstraction* — the [`ObjectStore`] trait behind which the normative
//! materializer obtains immutable object bytes without learning their
//! provenance (file, store, or cache):
//!
//! ```text
//!   EmbeddedStore      the in-crate content-addressed store (always available)
//!   EntropyFsStore     adapter over the real entropyfs embeddable engine
//!                      (feature `entropyfs-store`, never required)
//! ```
//!
//! Sharing requires the **exact canonical content identity** (BLAKE3 over the
//! canonical record bytes), never appearance (§46): two streams that declare
//! byte-identical immutable objects share one physical record. Palette-table
//! *snapshots* are published under a distinct kind prefix so they can never
//! collide with object records. rANS model tables and dictionary tables are
//! not first-class v1 tables yet; sharing them is recorded as open surface
//! rather than silently claimed.
//!
//! Accounting follows §31: the store reports **actual store-level physical
//! cost** and each stream's **declared per-stream attribution** separately;
//! shared state is never counted as zero.

use crate::{
    error::VoleError,
    identity::{self},
    object::Object,
};

pub use crate::identity::ContentId;

/// Kind prefix of an immutable **palette-table snapshot** payload (Phase J
/// tables, Phase P sharing). `0xE0` can never begin a canonical object record
/// (object tags are `0x01/0x02/0x05/0x07`), so palette snapshots and object
/// records cannot share a content id by construction.
pub const PALETTE_SNAPSHOT_KIND: u8 = 0xE0;

/// Canonical record bytes of an immutable object — the payload the store holds
/// under [`identity::content_id_of`] and the bytes the decoder re-parses to
/// rehydrate an external declaration.
pub fn object_record(obj: &Object) -> Vec<u8> {
    identity::canonical_object_record(obj)
}

/// Canonical payload of a palette-table snapshot (kind prefix + entries). Two
/// videos whose palette tables have identical entries share one blob.
pub fn palette_snapshot(entries: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + entries.len());
    out.push(PALETTE_SNAPSHOT_KIND);
    out.extend_from_slice(entries);
    out
}

/// Outcome of one [`ObjectStore::put`]: the content identity plus whether the
/// bytes were physically new to the store (false = dedup hit, nothing written).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PutOutcome {
    /// Content identity of the stored bytes (BLAKE3).
    pub id: ContentId,
    /// True when the store appended a physical record for these bytes; false
    /// when byte-identical content was already present (dedup).
    pub fresh: bool,
}

/// The content-addressed object-store abstraction. Implementations are
/// byte-exact: `get` returns the exact bytes `put` received, gated by the
/// content id. The materializer and the format parser never learn whether an
/// object's bytes came from a `.vole` file, an [`EmbeddedStore`], an
/// [`EntropyFsStore`], or a memory cache — only through this trait.
pub trait ObjectStore {
    /// Fetch the exact stored bytes of `id`. `Ok(None)` when the store does
    /// not hold `id`; `Err` when the store is corrupt, the read failed, or the
    /// stored record exceeds `max_bytes` (callers bound hostile reads).
    fn get(&self, id: ContentId, max_bytes: u64) -> Result<Option<Vec<u8>>, VoleError>;

    /// Store exact bytes (deduplicated by content identity) and return the id.
    fn put(&mut self, bytes: &[u8]) -> Result<PutOutcome, VoleError>;

    /// Whether `id` is present.
    fn contains(&self, id: ContentId) -> Result<bool, VoleError>;

    /// Number of distinct blobs physically held.
    fn unique_count(&self) -> u64;

    /// Sum of the distinct payload lengths (no framing, no backend overhead):
    /// the *logical* unique content volume. Declared-minus-unique is the pure
    /// cross-stream dedup saving.
    fn unique_payload_bytes(&self) -> u64;

    /// Actual store-level physical cost in bytes (never the declared
    /// attribution — see [`ArchiveAccounting`]). Includes backend framing.
    fn physical_bytes(&self) -> u64;

    /// Durability barrier (backend flush / engine sync).
    fn sync(&mut self) -> Result<(), VoleError>;

    /// Release backend resources. Implementations make acknowledged puts
    /// durable before releasing.
    fn close(&mut self) -> Result<(), VoleError>;
}

// ---------------------------------------------------------------------------
// Archive publish: extract a standalone stream's immutable shareable payloads
// ---------------------------------------------------------------------------

/// What one stream declared into the store. `declared_*` are the per-stream
/// **attribution** (§31): the record bytes the stream itself embeds. `*_new`
/// counts payloads that were physically absent before this publish.
#[derive(Debug, Clone, Default)]
pub struct StreamPublish {
    /// Content ids of every immutable object the stream declared.
    pub object_ids: Vec<ContentId>,
    /// Content ids of every palette-table snapshot the stream declared.
    pub palette_ids: Vec<ContentId>,
    /// Sum of canonical object-record bytes attributed to this stream.
    pub declared_object_bytes: u64,
    /// Sum of palette-snapshot payload bytes attributed to this stream.
    pub declared_palette_bytes: u64,
    /// Object payloads physically added by this publish.
    pub new_objects: u64,
    /// Object payloads already present (shared with an earlier stream).
    pub reused_objects: u64,
    /// Palette payloads physically added by this publish.
    pub new_palettes: u64,
}

impl StreamPublish {
    /// Total declared attribution of this stream (objects + palette
    /// snapshots). Shared state is reported here as attributed, never zero.
    pub fn declared_bytes(&self) -> u64 {
        self.declared_object_bytes + self.declared_palette_bytes
    }
}

/// Publish every immutable shareable payload of a **standalone** `.vole` stream
/// into `store`: each distinct object's canonical record and each distinct
/// palette-table snapshot, deduplicated by exact content identity. The stream
/// itself is unchanged (standalone `.vole` semantics are untouched); this is
/// the object-level archive view used for cross-video sharing, GC closure, and
/// physical accounting. Streams with external declarations are rejected (they
/// are already store-backed; publish their source stream instead).
pub fn publish_stream<S: ObjectStore + ?Sized>(
    store: &mut S,
    bytes: &[u8],
) -> Result<StreamPublish, VoleError> {
    let parsed = crate::decoder::decode_bytes(bytes)?;
    let initial = parsed.clone_initial();
    let mut out = StreamPublish::default();
    for (_, obj) in initial.objects() {
        let rec = object_record(obj);
        out.declared_object_bytes = out
            .declared_object_bytes
            .checked_add(rec.len() as u64)
            .ok_or(VoleError::ArithmeticOverflow)?;
        let o = store.put(&rec)?;
        out.object_ids.push(o.id);
        if o.fresh {
            out.new_objects += 1;
        } else {
            out.reused_objects += 1;
        }
    }
    for (_, entries) in initial.palettes() {
        let payload = palette_snapshot(entries);
        out.declared_palette_bytes = out
            .declared_palette_bytes
            .checked_add(payload.len() as u64)
            .ok_or(VoleError::ArithmeticOverflow)?;
        let o = store.put(&payload)?;
        out.palette_ids.push(o.id);
        if o.fresh {
            out.new_palettes += 1;
        }
    }
    Ok(out)
}

/// Archive-level physical-vs-declared accounting (§31). `physical` is what the
/// store actually holds (one copy per distinct payload, backend framing
/// included); `unique_payload_bytes` is the distinct payload volume without
/// framing; `declared` is the sum of per-stream attributions across the
/// published streams. Shared objects are therefore reported in both, never
/// silently zeroed.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArchiveAccounting {
    /// Distinct payloads physically held.
    pub unique_payloads: u64,
    /// Sum of distinct payload lengths (no framing).
    pub unique_payload_bytes: u64,
    /// Actual store-level physical bytes (framing included).
    pub physical_bytes: u64,
    /// Sum of declared attribution bytes over all published streams.
    pub declared_bytes: u64,
    /// `declared - unique_payload_bytes`: bytes the physical store does not
    /// repeat across streams (payload-level, framing excluded).
    pub dedup_saved_bytes: u64,
}

/// Account a store after a series of publishes whose declared attributions sum
/// to `declared_bytes`.
pub fn archive_accounting<S: ObjectStore + ?Sized>(
    store: &S,
    declared_bytes: u64,
) -> ArchiveAccounting {
    let unique = store.unique_payload_bytes();
    ArchiveAccounting {
        unique_payloads: store.unique_count(),
        unique_payload_bytes: unique,
        physical_bytes: store.physical_bytes(),
        declared_bytes,
        dedup_saved_bytes: declared_bytes.saturating_sub(unique),
    }
}

// ---------------------------------------------------------------------------
// EmbeddedStore: the in-crate content-addressed store
// ---------------------------------------------------------------------------

/// The always-available, in-crate store implementation: a single append-only
/// content-addressed log plus named snapshot roots and mark-compact GC.
///
/// ## Layout (manual little-endian, deterministic, hostile-safe)
///
/// ```text
/// <dir>/header      b"VSTO" + version u8 = 1
/// <dir>/blobs.log   one record per distinct payload:
///                       [content_id 32][len u64][payload len]
/// <dir>/roots/<n>   one 32-byte content id per line (sorted, deduped): the
///                   named snapshot pins exactly these blobs
/// ```
///
/// Reads are hash-gated (a stored payload whose digest does not match its id
/// is [`VoleError::IntegrityMismatch`]); record lengths are bounded so a
/// hostile or truncated log is a typed error, never an allocation bomb; a torn
/// trailing record is a typed error (the EntropyFS engine, not this court-grade
/// store, is the crash-durable substrate). `physical_bytes()` is the actual
/// on-disk log length — the honest store-level physical cost.
#[derive(Debug)]
pub struct EmbeddedStore {
    dir: std::path::PathBuf,
    /// id → (record offset in the log, payload length).
    index: std::collections::BTreeMap<ContentId, (u64, u64)>,
    log_len: u64,
}

const HEADER_MAGIC: &[u8; 4] = b"VSTO";
const HEADER_VERSION: u8 = 1;
/// Header file name.
const HEADER_FILE: &str = "header";
/// Blob log file name.
const LOG_FILE: &str = "blobs.log";
/// Roots directory name.
const ROOTS_DIR: &str = "roots";
/// Hard bound on one stored record payload (object records are bounded by
/// `Limits.max_object_bytes`; palette snapshots by `max_palette_entries`).
const MAX_BLOB_BYTES: u64 = 1 << 28;
/// Hard bound on the whole log (keeps hostile opens bounded).
const MAX_LOG_BYTES: u64 = 1 << 34;
/// Per-record framing overhead: content id (32) + length word (8).
const RECORD_FRAMING: u64 = 40;

impl EmbeddedStore {
    /// Create a fresh store at `dir` (creating it if absent).
    pub fn create(dir: &std::path::Path) -> Result<Self, VoleError> {
        std::fs::create_dir_all(dir).map_err(|_| VoleError::StoreFailure("create dir"))?;
        let header = dir.join(HEADER_FILE);
        if header.exists() {
            return Err(VoleError::StoreFailure("store already exists"));
        }
        write_header(&header)?;
        // An empty log exists from birth so `open` is total over created
        // stores and `gc` always has a file to rewrite.
        let log = dir.join(LOG_FILE);
        std::fs::write(&log, []).map_err(|_| VoleError::StoreFailure("create log"))?;
        Ok(EmbeddedStore {
            dir: dir.to_path_buf(),
            index: std::collections::BTreeMap::new(),
            log_len: 0,
        })
    }

    /// Open an existing store, verifying the header, every record's content id
    /// against its payload digest, and every bound. A store whose log ends
    /// mid-record (torn) is a typed error.
    pub fn open(dir: &std::path::Path) -> Result<Self, VoleError> {
        let header = dir.join(HEADER_FILE);
        let hb = std::fs::read(&header).map_err(|_| VoleError::StoreFailure("open header"))?;
        if hb.len() != 5 || &hb[..4] != HEADER_MAGIC || hb[4] != HEADER_VERSION {
            return Err(VoleError::StoreFailure("bad store header"));
        }
        let log_path = dir.join(LOG_FILE);
        let log = std::fs::read(&log_path).map_err(|_| VoleError::StoreFailure("open log"))?;
        if log.len() as u64 > MAX_LOG_BYTES {
            return Err(VoleError::DimensionTooLarge);
        }
        let mut index: std::collections::BTreeMap<ContentId, (u64, u64)> =
            std::collections::BTreeMap::new();
        let mut pos = 0u64;
        while pos < log.len() as u64 {
            let remain = log.len() as u64 - pos;
            if remain < RECORD_FRAMING {
                return Err(VoleError::Truncated);
            }
            let cid = read_cid(&log, pos)?;
            let len = read_u64(&log, pos + 32)?;
            if len > MAX_BLOB_BYTES || RECORD_FRAMING + len > remain {
                return Err(VoleError::Truncated);
            }
            let start = (pos + RECORD_FRAMING) as usize;
            let end = start + len as usize;
            let payload = &log[start..end];
            if crate::integr::digest(payload) != *cid.as_bytes() {
                return Err(VoleError::IntegrityMismatch);
            }
            if index.insert(cid, (pos, len)).is_some() {
                return Err(VoleError::NonCanonicalEncoding);
            }
            pos += RECORD_FRAMING + len;
        }
        Ok(EmbeddedStore {
            dir: dir.to_path_buf(),
            index,
            log_len: pos,
        })
    }

    /// Directory the store lives in.
    pub fn path(&self) -> &std::path::Path {
        &self.dir
    }

    /// Fixed on-disk overhead (header + empty-log cells; roots are pinned
    /// state, not blob storage).
    pub fn overhead_bytes(&self) -> u64 {
        5
    }

    /// Pin a named snapshot root to exactly `ids` (replaces any earlier root
    /// of the same name). Root names are restricted to safe file names.
    pub fn set_root(&mut self, name: &str, ids: &[ContentId]) -> Result<(), VoleError> {
        validate_root_name(name)?;
        let root_dir = self.dir.join(ROOTS_DIR);
        std::fs::create_dir_all(&root_dir)
            .map_err(|_| VoleError::StoreFailure("create roots dir"))?;
        let mut sorted: Vec<ContentId> = ids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut out = Vec::with_capacity(sorted.len().saturating_mul(32));
        for id in &sorted {
            out.extend_from_slice(id.as_bytes());
        }
        std::fs::write(root_dir.join(name), out)
            .map_err(|_| VoleError::StoreFailure("write root"))?;
        Ok(())
    }

    /// Remove a named root. Returns whether the root existed.
    pub fn drop_root(&mut self, name: &str) -> Result<bool, VoleError> {
        validate_root_name(name)?;
        let p = self.dir.join(ROOTS_DIR).join(name);
        match std::fs::remove_file(&p) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Ids pinned by the named root, if it exists.
    pub fn root(&self, name: &str) -> Result<Option<Vec<ContentId>>, VoleError> {
        validate_root_name(name)?;
        let p = self.dir.join(ROOTS_DIR).join(name);
        let raw = match std::fs::read(&p) {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        if raw.len() % 32 != 0 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let mut ids = Vec::with_capacity(raw.len() / 32);
        for b in raw.as_chunks::<32>().0 {
            ids.push(ContentId::from_array(*b));
        }
        Ok(Some(ids))
    }

    /// Every named root, sorted.
    pub fn root_names(&self) -> Result<Vec<String>, VoleError> {
        let dir = self.dir.join(ROOTS_DIR);
        let mut names = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    names.push(e.file_name().to_string_lossy().into_owned());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    /// Mark-compact GC: rewrite the log keeping exactly the blobs pinned by the
    /// union of all named roots. Returns the reclamation report. Live blobs
    /// (pinned by at least one root) are never collected; a blob with no
    /// pinning root is unreachable and reclaimed.
    pub fn gc(&mut self) -> Result<GcReport, VoleError> {
        let mut live: std::collections::BTreeSet<ContentId> = std::collections::BTreeSet::new();
        for name in self.root_names()? {
            if let Some(ids) = self.root(&name)? {
                live.extend(ids);
            }
        }
        // Rewrite in original record order, keeping only pinned blobs.
        let log_path = self.dir.join(LOG_FILE);
        let log = std::fs::read(&log_path).map_err(|_| VoleError::StoreFailure("open log"))?;
        let mut kept: Vec<u8> = Vec::new();
        let mut pos = 0u64;
        while pos < log.len() as u64 {
            let cid = read_cid(&log, pos)?;
            let len = read_u64(&log, pos + 32)?;
            if len > MAX_BLOB_BYTES || pos + RECORD_FRAMING + len > log.len() as u64 {
                return Err(VoleError::Truncated);
            }
            let start = (pos + RECORD_FRAMING) as usize;
            let end = start + len as usize;
            if live.contains(&cid) {
                kept.extend_from_slice(cid.as_bytes());
                kept.extend_from_slice(&len.to_le_bytes());
                kept.extend_from_slice(&log[start..end]);
            }
            pos += RECORD_FRAMING + len;
        }
        let tmp = self.dir.join("blobs.log.tmp");
        std::fs::write(&tmp, &kept).map_err(|_| VoleError::StoreFailure("gc write"))?;
        std::fs::rename(&tmp, &log_path).map_err(|_| VoleError::StoreFailure("gc rename"))?;
        // Rebuild the in-memory index over the surviving log (no panics: all
        // slicing is bounds-checked against `kept.len()`).
        let mut index: std::collections::BTreeMap<ContentId, (u64, u64)> =
            std::collections::BTreeMap::new();
        let mut npos = 0u64;
        let mut n = 0usize;
        while n < kept.len() {
            if kept.len() - n < RECORD_FRAMING as usize {
                return Err(VoleError::Truncated);
            }
            let cid = read_cid(&kept, n as u64)?;
            let len = read_u64(&kept, n as u64 + 32)?;
            if len > MAX_BLOB_BYTES || n as u64 + RECORD_FRAMING + len > kept.len() as u64 {
                return Err(VoleError::Truncated);
            }
            index.insert(cid, (npos, len));
            n += RECORD_FRAMING as usize + len as usize;
            npos += RECORD_FRAMING + len;
        }
        let reclaimed = self.log_len.saturating_sub(npos);
        self.index = index;
        self.log_len = npos;
        Ok(GcReport {
            reclaimed_bytes: reclaimed,
            physical_after_bytes: npos,
            retained_ids: self.index.len() as u64,
        })
    }
}

/// Result of one GC pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcReport {
    /// Physical bytes reclaimed by the pass.
    pub reclaimed_bytes: u64,
    /// Log length after the pass.
    pub physical_after_bytes: u64,
    /// Distinct blobs retained (all pinned by at least one root).
    pub retained_ids: u64,
}

impl ObjectStore for EmbeddedStore {
    fn get(&self, id: ContentId, max_bytes: u64) -> Result<Option<Vec<u8>>, VoleError> {
        let Some(&(off, len)) = self.index.get(&id) else {
            return Ok(None);
        };
        if len > max_bytes {
            return Err(VoleError::DimensionTooLarge);
        }
        let log_path = self.dir.join(LOG_FILE);
        use std::io::{Read, Seek, SeekFrom};
        let mut f =
            std::fs::File::open(&log_path).map_err(|_| VoleError::StoreFailure("open log"))?;
        f.seek(SeekFrom::Start(off + RECORD_FRAMING))
            .map_err(|_| VoleError::StoreFailure("log seek"))?;
        let mut payload = vec![0u8; len as usize];
        f.read_exact(&mut payload)
            .map_err(|_| VoleError::StoreFailure("log read"))?;
        if crate::integr::digest(&payload) != *id.as_bytes() {
            return Err(VoleError::IntegrityMismatch);
        }
        Ok(Some(payload))
    }

    fn put(&mut self, bytes: &[u8]) -> Result<PutOutcome, VoleError> {
        let id = ContentId::from_array(crate::integr::digest(bytes));
        if self.index.contains_key(&id) {
            return Ok(PutOutcome { id, fresh: false });
        }
        if bytes.len() as u64 > MAX_BLOB_BYTES {
            return Err(VoleError::DimensionTooLarge);
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.dir.join(LOG_FILE))
            .map_err(|_| VoleError::StoreFailure("open log append"))?;
        file.write_all(id.as_bytes())
            .and_then(|_| file.write_all(&(bytes.len() as u64).to_le_bytes()))
            .and_then(|_| file.write_all(bytes))
            .map_err(|_| VoleError::StoreFailure("log write"))?;
        self.index.insert(id, (self.log_len, bytes.len() as u64));
        self.log_len += RECORD_FRAMING + bytes.len() as u64;
        Ok(PutOutcome { id, fresh: true })
    }

    fn contains(&self, id: ContentId) -> Result<bool, VoleError> {
        Ok(self.index.contains_key(&id))
    }

    fn unique_count(&self) -> u64 {
        self.index.len() as u64
    }

    fn unique_payload_bytes(&self) -> u64 {
        self.index.values().map(|&(_, len)| len).sum()
    }

    fn physical_bytes(&self) -> u64 {
        self.log_len
    }

    fn sync(&mut self) -> Result<(), VoleError> {
        if let Ok(f) = std::fs::File::open(self.dir.join(LOG_FILE)) {
            f.sync_all().map_err(|_| VoleError::StoreFailure("sync"))?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), VoleError> {
        self.sync()
    }
}

fn write_header(path: &std::path::Path) -> Result<(), VoleError> {
    let mut h = Vec::with_capacity(5);
    h.extend_from_slice(HEADER_MAGIC);
    h.push(HEADER_VERSION);
    std::fs::write(path, h).map_err(|_| VoleError::StoreFailure("write header"))
}

fn read_cid(log: &[u8], pos: u64) -> Result<ContentId, VoleError> {
    let p = pos as usize;
    if p + 32 > log.len() {
        return Err(VoleError::Truncated);
    }
    let mut b = [0u8; 32];
    b.copy_from_slice(&log[p..p + 32]);
    Ok(ContentId::from_array(b))
}

fn read_u64(log: &[u8], pos: u64) -> Result<u64, VoleError> {
    let p = pos as usize;
    if p + 8 > log.len() {
        return Err(VoleError::Truncated);
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(&log[p..p + 8]);
    Ok(u64::from_le_bytes(b))
}

/// Root names are restricted to `[A-Za-z0-9._-]`, non-empty, ≤ 64 chars, and
/// never `.`/`..` (they become file names).
fn validate_root_name(name: &str) -> Result<(), VoleError> {
    if name.is_empty() || name.len() > 64 || name == "." || name == ".." {
        return Err(VoleError::ApiConstraint("invalid root name"));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
    {
        return Err(VoleError::ApiConstraint("invalid root name"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// EntropyFsStore: adapter over the real entropyfs embeddable engine
// ---------------------------------------------------------------------------

/// Adapter over the published `entropyfs` embeddable engine (feature
/// `entropyfs-store`, default OFF). The engine is a content-addressed blob
/// store keyed by BLAKE3 with a hash gate on reads, typed error classes, an
/// exclusive mount lock, an explicit durability barrier (`sync`), and its own
/// compaction/GC and physical accounting. This adapter maps exactly those
/// semantics onto [`ObjectStore`]; VOLE never touches the engine's filesystem
/// internals. Engine `BlobId`s and VOLE content ids are the same BLAKE3 digest
/// of the same bytes, so cross-store identity is exact.
#[cfg(feature = "entropyfs-store")]
pub struct EntropyFsStore {
    engine: entropyfs::engine::Engine,
    dir: std::path::PathBuf,
}

#[cfg(feature = "entropyfs-store")]
impl EntropyFsStore {
    /// Create a fresh engine store at `dir`.
    pub fn create(dir: &std::path::Path) -> Result<Self, VoleError> {
        std::fs::create_dir_all(dir).map_err(|_| VoleError::StoreFailure("create dir"))?;
        let opts = entropyfs::engine::EngineOpenOptions::default();
        let engine = entropyfs::engine::Engine::create(dir, &opts).map_err(engine_error)?;
        Ok(EntropyFsStore {
            engine,
            dir: dir.to_path_buf(),
        })
    }

    /// Open an existing engine store read/write.
    pub fn open(dir: &std::path::Path) -> Result<Self, VoleError> {
        let opts = entropyfs::engine::EngineOpenOptions::default();
        let engine = entropyfs::engine::Engine::open(dir, &opts).map_err(engine_error)?;
        Ok(EntropyFsStore {
            engine,
            dir: dir.to_path_buf(),
        })
    }

    /// Directory the engine store lives in.
    pub fn path(&self) -> &std::path::Path {
        &self.dir
    }

    /// The engine's own metrics DTO (blob count, physical used bytes, …).
    pub fn metrics(&self) -> Result<entropyfs::engine::EngineMetrics, VoleError> {
        self.engine.metrics().map_err(engine_error)
    }

    /// Run the engine's compaction/GC pass and return its reclamation report
    /// (engine-level reclamation; VOLE-level GC closure — roots it controls —
    /// is demonstrated on the [`EmbeddedStore`]).
    pub fn compact(&self) -> Result<entropyfs::engine::CompactionReport, VoleError> {
        self.engine.compact().map_err(engine_error)
    }
}

#[cfg(feature = "entropyfs-store")]
impl ObjectStore for EntropyFsStore {
    fn get(&self, id: ContentId, max_bytes: u64) -> Result<Option<Vec<u8>>, VoleError> {
        let blob = entropyfs::engine::BlobId(*id.as_bytes());
        match self.engine.get_blob(blob) {
            Ok(bytes) => {
                if bytes.len() as u64 > max_bytes {
                    return Err(VoleError::DimensionTooLarge);
                }
                Ok(Some(bytes))
            }
            Err(e) if e.code == entropyfs::engine::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(engine_error(e)),
        }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<PutOutcome, VoleError> {
        let id = ContentId::from_array(crate::integr::digest(bytes));
        // The engine dedups by identity, but `contains` tells us whether the
        // payload was physically new so accounting stays exact.
        let fresh = !self
            .engine
            .contains(entropyfs::engine::BlobId(*id.as_bytes()))
            .map_err(engine_error)?;
        self.engine.put_blob(bytes).map_err(engine_error)?;
        Ok(PutOutcome { id, fresh })
    }

    fn contains(&self, id: ContentId) -> Result<bool, VoleError> {
        self.engine
            .contains(entropyfs::engine::BlobId(*id.as_bytes()))
            .map_err(engine_error)
    }

    fn unique_count(&self) -> u64 {
        self.engine
            .metrics()
            .map(|m| m.accounting.blob_count)
            .unwrap_or(0)
    }

    fn unique_payload_bytes(&self) -> u64 {
        // The engine's physical reconciliation reports the sum of root-
        // reachable canonical record bytes (each engine blob is one file whose
        // content is the stored record), which is the payload-level unique
        // volume. `accounting.logical_bytes` is user-namespace state and is
        // not used here.
        self.engine
            .metrics()
            .map(|m| m.physical.live_bytes)
            .unwrap_or(0)
    }

    fn physical_bytes(&self) -> u64 {
        self.engine
            .metrics()
            .map(|m| m.accounting.physical_used_bytes)
            .unwrap_or(0)
    }

    fn sync(&mut self) -> Result<(), VoleError> {
        self.engine.sync().map_err(engine_error)
    }

    fn close(&mut self) -> Result<(), VoleError> {
        // Make acknowledged puts power-durable, then release the exclusive
        // store lock so another engine can open the same directory.
        self.engine.sync().map_err(engine_error)?;
        self.engine.close().map_err(engine_error)
    }
}

#[cfg(feature = "entropyfs-store")]
fn engine_error(e: entropyfs::engine::EngineError) -> VoleError {
    use entropyfs::engine::ErrorCode::*;
    let cond: &'static str = match e.code {
        Ok => "ok",
        NotFound => "not found",
        InvalidArgument => "invalid argument",
        CorruptStore => "corrupt store",
        IncompatibleFormat => "incompatible format",
        ResourceLimit => "resource limit",
        Io => "io",
        Busy => "busy",
        Unsupported => "unsupported",
        Internal => "internal",
        Closed => "closed",
    };
    VoleError::StoreFailure(cond)
}
