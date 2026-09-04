//! Phase P courts: the optional content-addressed persistence substrate
//! (ObjectStore), cross-video exact-object sharing, physical-vs-declared
//! accounting, GC closure, and external object declarations (master brief
//! §1 / §31 / §45 / §46).
//!
//! Standalone `.vole` semantics are untouched: every existing stream decodes
//! with no store; the store is an optional archive substrate, and a stream
//! that *references* external objects (feature bit + `TAG_OBJECT_EXTERN`) is
//! deliberately not standalone and fails closed without a store binding.
//! Sharing is by exact BLAKE3 canonical-record identity, never appearance.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use vole_video::{
    checked::ByteSink,
    decoder, encoder, identity, integr,
    limits::Limits,
    object::{Object, ObjectId},
    pixel::Canvas,
    state::{Instance, InstanceId, PaletteId},
    store::{self, ContentId, EmbeddedStore, ObjectStore},
    transition::Transition,
    VoleError,
};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Map raw filesystem results onto the typed error surface (courts return
/// `Result<_, VoleError>`).
fn fs<T>(r: std::io::Result<T>) -> Result<T, VoleError> {
    r.map_err(|_| VoleError::StoreFailure("test fs io"))
}

/// One unique temp dir per call (no extra dev-dependencies; tests clean
/// nothing, matching the corpus convention of keeping evidence on disk).
fn temp_dir(tag: &str) -> PathBuf {
    let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("vole-phase-p-{}-{}-{}", std::process::id(), tag, n))
}

fn gradient_samples(w: u32, h: u32, base: u8, sx: i64, sy: i64) -> Vec<u8> {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            d.push(((i64::from(base) + sx * i64::from(x) + sy * i64::from(y)) & 0xFF) as u8);
        }
    }
    d
}

fn frames_of(parsed: &vole_video::format::ParsedStream) -> Result<Vec<Canvas>, VoleError> {
    decoder::materialize_all(parsed)
}

// ---------------------------------------------------------------------------
// A minimal in-memory ObjectStore for hostile-provenance courts (bytes can be
// injected under arbitrary content ids, which a hash-gated disk store refuses
// by construction).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MapStore {
    map: BTreeMap<ContentId, Vec<u8>>,
}

impl MapStore {
    fn insert_raw(&mut self, id: ContentId, bytes: Vec<u8>) {
        self.map.insert(id, bytes);
    }
}

impl ObjectStore for MapStore {
    fn get(&self, id: ContentId, max_bytes: u64) -> Result<Option<Vec<u8>>, VoleError> {
        match self.map.get(&id) {
            Some(bytes) => {
                if bytes.len() as u64 > max_bytes {
                    return Err(VoleError::DimensionTooLarge);
                }
                Ok(Some(bytes.clone()))
            }
            None => Ok(None),
        }
    }
    fn put(&mut self, bytes: &[u8]) -> Result<store::PutOutcome, VoleError> {
        let id = ContentId::from_array(integr::digest(bytes));
        let fresh = !self.map.contains_key(&id);
        self.map.insert(id, bytes.to_vec());
        Ok(store::PutOutcome { id, fresh })
    }
    fn contains(&self, id: ContentId) -> Result<bool, VoleError> {
        Ok(self.map.contains_key(&id))
    }
    fn unique_count(&self) -> u64 {
        self.map.len() as u64
    }
    fn unique_payload_bytes(&self) -> u64 {
        self.map.values().map(|b| b.len() as u64).sum()
    }
    fn physical_bytes(&self) -> u64 {
        self.map.values().map(|b| b.len() as u64).sum()
    }
    fn sync(&mut self) -> Result<(), VoleError> {
        Ok(())
    }
    fn close(&mut self) -> Result<(), VoleError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Canonical records round-trip every object kind
// ---------------------------------------------------------------------------

#[test]
fn canonical_record_roundtrips_all_object_kinds() -> Result<(), VoleError> {
    let limits = Limits::default();
    let objs = [
        Object::fill(32, 16, 77)?,
        Object::raster(16, 8, gradient_samples(16, 8, 3, 2, 1))?,
        Object::index_raster(
            8,
            8,
            vec![0, 1, 2, 3, 0, 1, 2, 3]
                .into_iter()
                .cycle()
                .take(64)
                .collect(),
        )?,
        Object::procedural(
            24,
            12,
            vole_video::generator::Generator::Gradient {
                base: 9,
                sx: 3,
                sy: -1,
            },
        )?,
        Object::procedural(
            16,
            16,
            vole_video::generator::Generator::Checker {
                a: 5,
                b: 250,
                cell: 4,
            },
        )?,
    ];
    for obj in objs {
        let rec = store::object_record(&obj);
        // The store id of the record is exactly the object's content identity.
        assert_eq!(
            ContentId::from_array(integr::digest(&rec)),
            identity::content_id_of(&obj)
        );
        let back = Object::from_canonical_record(&rec, &limits)?;
        assert_eq!(back.width(), obj.width());
        assert_eq!(back.height(), obj.height());
        assert_eq!(back.kind(), obj.kind());
        assert_eq!(back.expand(), obj.expand());
        assert_eq!(
            identity::content_id_of(&back),
            identity::content_id_of(&obj)
        );
    }
    Ok(())
}

#[test]
fn canonical_record_hostile_forms_are_typed() {
    let limits = Limits::default();
    // Truncated.
    assert_eq!(
        Object::from_canonical_record(&[], &limits).unwrap_err(),
        VoleError::Truncated
    );
    // Unknown kind (geometry nonzero so the kind check is what fires).
    let mut bad = vec![0x7F, 4, 0, 0, 0, 4, 0, 0, 0];
    assert_eq!(
        Object::from_canonical_record(&bad, &limits).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Trailing bytes after a fill record are non-canonical.
    let obj = Object::fill(4, 4, 9).unwrap();
    let mut rec = store::object_record(&obj);
    rec.push(0);
    assert_eq!(
        Object::from_canonical_record(&rec, &limits).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Zero geometry.
    bad = vec![0x02, 0, 0, 0, 0, 0, 0, 0, 0, 3];
    assert_eq!(
        Object::from_canonical_record(&bad, &limits).unwrap_err(),
        VoleError::DimensionTooLarge
    );
}

// ---------------------------------------------------------------------------
// EmbeddedStore: round trip, dedup, reopen, hash gate, physical accounting
// ---------------------------------------------------------------------------

#[test]
fn embedded_store_roundtrip_dedup_reopen_and_hash_gate() -> Result<(), VoleError> {
    let dir = temp_dir("embedded");
    let mut st = EmbeddedStore::create(&dir)?;
    let payload = gradient_samples(24, 12, 40, 1, 1);
    let o1 = st.put(&payload)?;
    assert!(o1.fresh);
    let o2 = st.put(&payload)?;
    assert_eq!(o1.id, o2.id, "content identity is exact");
    assert!(!o2.fresh, "identical bytes are never stored twice");
    assert_eq!(st.unique_count(), 1);
    assert_eq!(
        st.physical_bytes(),
        40 + payload.len() as u64,
        "physical cost is the actual log length (framing + payload)"
    );
    assert_eq!(st.get(o1.id, 1 << 20)?, Some(payload.clone()));
    assert_eq!(st.get(o1.id, 10).unwrap_err(), VoleError::DimensionTooLarge);
    let absent = ContentId::from_array([0xAA; 32]);
    assert!(!st.contains(absent)?);
    assert_eq!(st.get(absent, 1 << 20)?, None);
    st.sync()?;
    st.close()?;

    // Reopen: blobs survive; content id is stable; a *different* payload has a
    // different id.
    let st2 = EmbeddedStore::open(&dir)?;
    assert_eq!(st2.get(o1.id, 1 << 20)?, Some(payload));
    assert_eq!(st2.unique_count(), 1);
    let mut other = gradient_samples(24, 12, 41, 1, 1);
    other[0] = other[0].wrapping_add(1);
    let mut st3 = EmbeddedStore::open(&dir)?;
    let o3 = st3.put(&other)?;
    assert_ne!(o1.id, o3.id);
    assert_eq!(st3.unique_count(), 2);
    Ok(())
}

#[test]
fn embedded_store_corruption_is_typed() -> Result<(), VoleError> {
    // Each corruption is exercised on a fresh store so the cases never share
    // state.
    let payload = gradient_samples(64, 32, 1, 1, 3);
    let seeded = |tag: &str| -> Result<(PathBuf, ContentId), VoleError> {
        let dir = temp_dir(tag);
        let mut st = EmbeddedStore::create(&dir)?;
        let o = st.put(&payload)?;
        st.sync()?;
        st.close()?;
        Ok((dir, o.id))
    };

    // (a) A flipped payload byte fails the hash gate at open.
    let (dir_a, _) = seeded("corrupt-a")?;
    let log = dir_a.join("blobs.log");
    let mut bytes = fs(std::fs::read(&log))?;
    *bytes.last_mut().expect("non-empty log") ^= 0x01;
    fs(std::fs::write(&log, &bytes))?;
    assert_eq!(
        EmbeddedStore::open(&dir_a).unwrap_err(),
        VoleError::IntegrityMismatch
    );

    // (b) Truncated log (cut mid-payload) is Truncated.
    let (dir_b, _) = seeded("corrupt-b")?;
    let log = dir_b.join("blobs.log");
    let bytes = fs(std::fs::read(&log))?;
    fs(std::fs::write(&log, &bytes[..bytes.len() - 5]))?;
    assert_eq!(
        EmbeddedStore::open(&dir_b).unwrap_err(),
        VoleError::Truncated
    );

    // (c) A record with a length that runs past the file end is Truncated.
    let (dir_c, _) = seeded("corrupt-c")?;
    let log = dir_c.join("blobs.log");
    let bytes = fs(std::fs::read(&log))?;
    let mut forged = bytes[..40].to_vec(); // id + length word of record 1
    let huge = (1u64 << 40).to_le_bytes();
    forged.extend_from_slice(&huge);
    fs(std::fs::write(&log, &forged))?;
    assert_eq!(
        EmbeddedStore::open(&dir_c).unwrap_err(),
        VoleError::Truncated
    );

    // (d) Duplicate content-id record is non-canonical.
    let (dir_d, _) = seeded("corrupt-d")?;
    let log = dir_d.join("blobs.log");
    let bytes = fs(std::fs::read(&log))?;
    let mut dup = bytes.clone();
    dup.extend_from_slice(&bytes);
    fs(std::fs::write(&log, &dup))?;
    assert_eq!(
        EmbeddedStore::open(&dir_d).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );

    // (e) Bad header magic is a store failure.
    let (dir_e, _) = seeded("corrupt-e")?;
    fs(std::fs::write(dir_e.join("header"), b"NOPE1"))?;
    assert!(matches!(
        EmbeddedStore::open(&dir_e).unwrap_err(),
        VoleError::StoreFailure(_)
    ));

    // (f) Empty dir (no store) is a store failure.
    let empty = temp_dir("corrupt-f");
    fs(std::fs::create_dir_all(&empty))?;
    assert!(matches!(
        EmbeddedStore::open(&empty).unwrap_err(),
        VoleError::StoreFailure(_)
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Roots and GC closure
// ---------------------------------------------------------------------------

#[test]
fn store_roots_and_gc_closure() -> Result<(), VoleError> {
    let dir = temp_dir("gc");
    let mut st = EmbeddedStore::create(&dir)?;
    let rec_a = store::object_record(&Object::fill(8, 8, 1)?);
    let rec_b = store::object_record(&Object::fill(8, 8, 2)?);
    let rec_c = store::object_record(&Object::fill(8, 8, 3)?);
    let rec_d = store::object_record(&Object::fill(8, 8, 4)?);
    let (a, b, c, d) = (
        st.put(&rec_a)?.id,
        st.put(&rec_b)?.id,
        st.put(&rec_c)?.id,
        st.put(&rec_d)?.id,
    );
    assert_eq!(st.unique_count(), 4);
    let phys_all = st.physical_bytes();

    // No roots: everything is unreachable; gc empties the log.
    let r = st.gc()?;
    assert_eq!(r.retained_ids, 0);
    assert_eq!(r.reclaimed_bytes, phys_all);
    assert_eq!(st.physical_bytes(), 0);
    assert!(!st.contains(a)?);
    st.put(&rec_a)?;
    st.put(&rec_b)?;
    st.put(&rec_c)?;
    st.put(&rec_d)?;
    assert_eq!(st.unique_count(), 4);

    // Root1 pins {a, b}; root2 pins {a, c}. d is unreachable.
    st.set_root("video-1", &[a, b])?;
    st.set_root("video-2", &[a, c])?;
    let r = st.gc()?;
    assert_eq!(r.retained_ids, 3);
    assert!(r.reclaimed_bytes > 0);
    assert!(!st.contains(d)?, "unreferenced blob is reclaimed");
    assert!(st.contains(a)? && st.contains(b)? && st.contains(c)?);
    assert_eq!(st.get(b, 1 << 20)?, Some(rec_b));

    // Dropping root1 makes b unreachable while a stays pinned by root2.
    assert!(st.drop_root("video-1")?);
    let r = st.gc()?;
    assert_eq!(r.retained_ids, 2);
    assert!(r.reclaimed_bytes > 0);
    assert!(!st.contains(b)?);
    assert!(st.contains(a)? && st.contains(c)?);

    // Dropping root2 makes everything unreachable: full closure.
    assert!(st.drop_root("video-2")?);
    let r = st.gc()?;
    assert_eq!(r.retained_ids, 0);
    assert!(!st.contains(a)? && !st.contains(c)?);
    assert_eq!(st.root_names()?, Vec::<String>::new());
    Ok(())
}

#[test]
fn root_names_are_validated() -> Result<(), VoleError> {
    let dir = temp_dir("roots");
    let mut st = EmbeddedStore::create(&dir)?;
    let id = st.put(&store::object_record(&Object::fill(2, 2, 5)?))?.id;
    assert!(matches!(
        st.set_root("../evil", &[id]).unwrap_err(),
        VoleError::ApiConstraint(_)
    ));
    assert!(matches!(
        st.set_root("", &[id]).unwrap_err(),
        VoleError::ApiConstraint(_)
    ));
    assert!(matches!(
        st.set_root("a/b", &[id]).unwrap_err(),
        VoleError::ApiConstraint(_)
    ));
    st.set_root("ok-name-1.video", &[id])?;
    assert_eq!(st.root("ok-name-1.video")?, Some(vec![id]));
    assert_eq!(st.root("missing")?, None);
    Ok(())
}

// ---------------------------------------------------------------------------
// Cross-video sharing and physical-vs-declared accounting
// ---------------------------------------------------------------------------

/// Author a standalone single-object static stream.
/// Author a standalone single-object static stream.
#[cfg(feature = "entropyfs-store")]
fn static_stream(
    w: u32,
    h: u32,
    bg: u8,
    obj_id: u32,
    obj: Object,
    x: i64,
    y: i64,
) -> Result<Vec<u8>, VoleError> {
    encoder::encode_stream(
        w,
        h,
        bg,
        &[(obj_id, obj)],
        &[Instance {
            id: InstanceId(obj_id),
            object_id: ObjectId(obj_id),
            x,
            y,
        }],
        &[],
    )
}

#[test]
fn cross_video_objects_and_palettes_publish_once() -> Result<(), VoleError> {
    let dir = temp_dir("shared");
    let mut st = EmbeddedStore::create(&dir)?;

    // Four "videos" sharing one byte-identical 32x32 logo; each has its own
    // 24x16 panel content.
    let logo = Object::raster(32, 32, gradient_samples(32, 32, 200, -1, 2))?;
    let panels = [
        Object::raster(24, 16, gradient_samples(24, 16, 10, 1, 0))?,
        Object::raster(24, 16, gradient_samples(24, 16, 90, 2, 3))?,
        Object::raster(24, 16, gradient_samples(24, 16, 170, -2, 1))?,
        Object::raster(24, 16, gradient_samples(24, 16, 40, 0, 4))?,
    ];
    let mut declared_total = 0u64;
    let mut publish_expected: Vec<Vec<ContentId>> = Vec::new();
    for (k, panel) in panels.iter().enumerate() {
        let bytes = encoder::encode_stream(
            480,
            320,
            60,
            &[(1, logo.clone()), (2, panel.clone())],
            &[
                Instance {
                    id: InstanceId(1),
                    object_id: ObjectId(1),
                    x: 10,
                    y: 10,
                },
                Instance {
                    id: InstanceId(2),
                    object_id: ObjectId(2),
                    x: 100 + 40 * k as i64,
                    y: 200,
                },
            ],
            &[],
        )?;
        // Standalone decode never needs the store.
        let parsed = decoder::decode_bytes(&bytes)?;
        assert_eq!(parsed.clone_initial().object_count(), 2);
        let p = store::publish_stream(&mut st, &bytes)?;
        assert_eq!(p.object_ids.len(), 2);
        assert_eq!(p.new_objects + p.reused_objects, 2);
        if k == 0 {
            assert_eq!(p.new_objects, 2);
        } else {
            assert_eq!(p.new_objects, 1, "the panel is new, the logo is shared");
            assert_eq!(p.reused_objects, 1);
        }
        declared_total += p.declared_bytes();
        publish_expected.push(p.object_ids.clone());
    }
    assert_eq!(publish_expected[0][0], publish_expected[1][0]);
    assert_eq!(publish_expected[0][0], publish_expected[3][0]);

    let acc = store::archive_accounting(&st, declared_total);
    // 1 logo + 4 panels = 5 unique object payloads.
    assert_eq!(acc.unique_payloads, 5);
    let logo_rec_len = store::object_record(&logo).len() as u64;
    let panel_lens: u64 = panels
        .iter()
        .map(|p| store::object_record(p).len() as u64)
        .sum();
    // Payload-level accounting: declared (8 records across 4 streams) minus
    // unique (5 distinct records) is exactly the three repeated logo records —
    // the shared object is attributed per stream (never zeroed) and physically
    // stored once.
    assert_eq!(acc.unique_payload_bytes, logo_rec_len + panel_lens);
    assert_eq!(acc.declared_bytes, 4 * logo_rec_len + panel_lens);
    assert_eq!(acc.dedup_saved_bytes, 3 * logo_rec_len);
    assert_eq!(
        acc.physical_bytes,
        40 * 5 + acc.unique_payload_bytes,
        "embedded physical cost = per-record framing + unique payloads"
    );
    assert!(acc.physical_bytes > acc.unique_payload_bytes);

    // Palette tables share too: two videos with identical palette entries.
    let entries = vec![10u8, 40, 90, 150, 220, 30, 60];
    let idx = Object::index_raster(16, 16, {
        let mut d = Vec::with_capacity(256);
        for y in 0..16u8 {
            for x in 0..16u8 {
                d.push((x / 8 + y / 8) % 7);
            }
        }
        d
    })?;
    let mut pal_ids = Vec::new();
    for k in 0..2 {
        let bytes = encoder::encode_palette_stream(
            160,
            120,
            0,
            &[(1, idx.clone())],
            &[(1, entries.clone())],
            &[(
                Instance {
                    id: InstanceId(1),
                    object_id: ObjectId(1),
                    x: 0,
                    y: 0,
                },
                Some(PaletteId(1)),
            )],
            &[],
        )?;
        let p = store::publish_stream(&mut st, &bytes)?;
        assert_eq!(p.palette_ids.len(), 1);
        assert_eq!(p.object_ids.len(), 1);
        if k == 0 {
            assert_eq!(p.new_palettes, 1);
        } else {
            assert_eq!(p.new_palettes, 0, "identical palette snapshot shared");
        }
        declared_total += p.declared_bytes();
        pal_ids.push(p.palette_ids[0]);
    }
    assert_eq!(pal_ids[0], pal_ids[1]);
    // Palette-snapshot payloads never collide with object records.
    let snap = store::palette_snapshot(&entries);
    assert_eq!(snap[0], store::PALETTE_SNAPSHOT_KIND);
    assert_eq!(
        ContentId::from_array(integr::digest(&snap)),
        pal_ids[0],
        "the stored id is the digest of the kind-prefixed payload"
    );
    assert_ne!(pal_ids[0], identity::content_id_of(&idx));
    // Final accounting over every publish: 5 video objects + the shared index
    // object + one palette snapshot are unique; the repeated logo records, the
    // repeated index object, and the repeated palette snapshot are the pure
    // dedup saving.
    let acc_final = store::archive_accounting(&st, declared_total);
    assert_eq!(acc_final.unique_payloads, 7);
    let idx_rec_len = store::object_record(&idx).len() as u64;
    let snap_len = store::palette_snapshot(&entries).len() as u64;
    assert_eq!(
        acc_final.dedup_saved_bytes,
        3 * logo_rec_len + idx_rec_len + snap_len,
        "dedup is exact at the payload level: logo x3, index object x1, palette snapshot x1"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// External object declarations: the materializer's provenance abstraction
// ---------------------------------------------------------------------------

#[test]
fn extern_stream_materializes_byte_identical_frames() -> Result<(), VoleError> {
    let (w, h) = (320u32, 200u32);
    let bg = 70u8;
    // A moving-textured-tile stream plus a static generator backdrop object.
    let tile = Object::raster(24, 16, gradient_samples(24, 16, 120, 2, -1))?;
    let backdrop = Object::procedural(
        320,
        200,
        vole_video::generator::Generator::Gradient {
            base: 10,
            sx: 1,
            sy: 1,
        },
    )?;
    let objects: Vec<(u32, Object)> = vec![(1, backdrop.clone()), (2, tile.clone())];
    let instances = vec![
        Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        },
        Instance {
            id: InstanceId(2),
            object_id: ObjectId(2),
            x: 40,
            y: 60,
        },
    ];
    let mut timeline = Vec::new();
    for k in 1..=10u64 {
        timeline.push((
            k,
            vec![Transition::SetPosition {
                id: InstanceId(2),
                x: 40 + 3 * k as i64,
                y: 60,
            }],
        ));
    }

    // S0: fully standalone (objects embedded).
    let s0 = encoder::encode_stream(w, h, bg, &objects, &instances, &timeline)?;
    let f0 = frames_of(&decoder::decode_bytes(&s0)?)?;
    assert_eq!(f0.len(), 11);

    // Publish the canonical records into a store, then write S1 with external
    // references to exactly those content ids.
    let mut st = EmbeddedStore::create(&temp_dir("extern"))?;
    let cid_backdrop = st.put(&store::object_record(&backdrop))?.id;
    let cid_tile = st.put(&store::object_record(&tile))?.id;
    assert_eq!(cid_backdrop, identity::content_id_of(&backdrop));

    let s1 = encoder::encode_stream_external(
        w,
        h,
        bg,
        &[(1, cid_backdrop), (2, cid_tile)],
        &instances,
        &timeline,
    )?;
    assert!(
        s1.len() < s0.len(),
        "external payloads leave the stream: {} -> {}",
        s0.len(),
        s1.len()
    );

    // Store-less decode of a non-standalone stream fails closed and typed.
    assert_eq!(
        decoder::decode_bytes(&s1).unwrap_err(),
        VoleError::StoreRequired
    );

    // With the store bound, materialization is byte-identical to S0. The
    // materializer never sees the store: provenance ends at the trait.
    let parsed1 = decoder::decode_with_store(&s1, &st)?;
    let f1 = frames_of(&parsed1)?;
    assert_eq!(f1.len(), f0.len());
    for (a, b) in f0.iter().zip(&f1) {
        assert!(a.exactly_matches(b), "extern frame == standalone frame");
    }

    // Rehydrating the *object table* from the store reproduces the standalone
    // bytes' semantics exactly (integrity trailer re-verified by parse).
    assert_eq!(parsed1.clone_initial().object_count(), 2);

    // A missing object in the store is a typed, deterministic error.
    let empty = EmbeddedStore::create(&temp_dir("extern-empty"))?;
    assert_eq!(
        decoder::decode_with_store(&s1, &empty).unwrap_err(),
        VoleError::StoreObjectMissing
    );
    Ok(())
}

#[test]
fn extern_hostile_wire_forms_are_typed() -> Result<(), VoleError> {
    let obj = Object::fill(8, 8, 3)?;
    let rec = store::object_record(&obj);
    let cid = identity::content_id_of(&obj);
    let mut good_store = MapStore::default();
    good_store.insert_raw(cid, rec.clone());

    // Manual header + extern + checkpoint, integrity trailer appended.
    let header = |bits: u32| -> ByteSink {
        let mut s = ByteSink::new();
        s.extend(b"VOLE").expect("magic");
        s.byte(0).expect("reserved");
        s.push(1u16).expect("version");
        s.push(1u32).expect("universe");
        s.byte(1).expect("profile");
        s.push(bits).expect("feature bits");
        s.push(8u32).expect("width");
        s.push(8u32).expect("height");
        s
    };
    let checkpoint = |s: &mut ByteSink| {
        s.byte(0x03).expect("checkpoint tag"); // TAG_CHECKPOINT
        s.byte(0).expect("background");
        s.push(1u32).expect("instance count");
        s.push(1u32).expect("instance id");
        s.push(1u32).expect("object id");
        s.push(0i32).expect("x");
        s.push(0i32).expect("y");
    };
    let trailer = |mut s: ByteSink| -> Vec<u8> {
        integr::append_trailer(&mut s).expect("trailer");
        s.into_vec()
    };

    // (1) Canonical extern stream: bit set + one extern + checkpoint.
    let mut s = header(1);
    s.byte(0x09).expect("extern tag");
    s.push(1u32).expect("object id");
    s.extend(cid.as_bytes()).expect("content id");
    checkpoint(&mut s);
    let bytes = trailer(s);
    let parsed = decoder::decode_with_store(&bytes, &good_store)?;
    assert_eq!(frames_of(&parsed)?.len(), 1);
    let standalone = encoder::encode_stream(
        8,
        8,
        0,
        &[(1, obj.clone())],
        &[Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        }],
        &[],
    )?;
    let fs = frames_of(&decoder::decode_bytes(&standalone)?)?;
    let fe = frames_of(&parsed)?;
    assert!(fs[0].exactly_matches(&fe[0]));

    // (2) Feature bit set but no extern declaration: non-canonical.
    // (Zero-instance checkpoint: the checkpoint itself must parse cleanly so
    // the canonicality check at end-of-stream is what fires.)
    let mut s = header(1);
    s.byte(0x03).expect("checkpoint tag");
    s.byte(0).expect("background");
    s.push(0u32).expect("zero instances");
    assert_eq!(
        decoder::decode_with_store(&trailer(s), &good_store).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );

    // (3) Extern declaration without the feature bit: non-canonical.
    let mut s = header(0);
    s.byte(0x09).expect("extern tag");
    s.push(1u32).expect("object id");
    s.extend(cid.as_bytes()).expect("content id");
    checkpoint(&mut s);
    assert_eq!(
        decoder::decode_with_store(&trailer(s), &good_store).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );

    // (4) Unknown mandatory feature bit: fail closed. (0x1 is the known
    // external-objects bit and 0x2 the known Phase-U quantized-content
    // declaration, so 0x4 is a genuinely unknown mandatory bit here.)
    let mut s = header(4);
    s.byte(0x09).expect("extern tag");
    s.push(1u32).expect("object id");
    s.extend(cid.as_bytes()).expect("content id");
    checkpoint(&mut s);
    assert_eq!(
        decoder::decode_with_store(&trailer(s), &good_store).unwrap_err(),
        VoleError::UnsupportedFeature
    );

    // (5) Extern after the checkpoint: non-canonical. (Zero-instance
    // checkpoint so the checkpoint itself cannot fail on an unknown object.)
    let mut s = header(1);
    s.byte(0x03).expect("checkpoint tag");
    s.byte(0).expect("background");
    s.push(0u32).expect("zero instances");
    s.byte(0x09).expect("extern tag");
    s.push(1u32).expect("object id");
    s.extend(cid.as_bytes()).expect("content id");
    assert_eq!(
        decoder::decode_with_store(&trailer(s), &good_store).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );

    // (6) Duplicate extern object id: DuplicateId.
    let mut s = header(1);
    s.byte(0x09).expect("extern tag");
    s.push(1u32).expect("object id");
    s.extend(cid.as_bytes()).expect("content id");
    s.byte(0x09).expect("extern tag");
    s.push(1u32).expect("object id");
    s.extend(cid.as_bytes()).expect("content id");
    checkpoint(&mut s);
    assert_eq!(
        decoder::decode_with_store(&trailer(s), &good_store).unwrap_err(),
        VoleError::DuplicateId
    );

    // (7) Truncated content id: Truncated (the file ends inside the reference
    // before any store lookup can occur).
    let mut s = header(1);
    s.byte(0x09).expect("extern tag");
    s.push(1u32).expect("object id");
    s.extend(&cid.as_bytes()[..20]).expect("short content id");
    assert_eq!(
        decoder::decode_with_store(&trailer(s), &good_store).unwrap_err(),
        VoleError::Truncated
    );

    // (8) Store does not hold the referenced object: StoreObjectMissing.
    assert_eq!(
        decoder::decode_with_store(&bytes, &MapStore::default()).unwrap_err(),
        VoleError::StoreObjectMissing
    );

    // (9) Store bytes whose digest does not match the declared content id:
    // IntegrityMismatch (the store's hash gate is re-applied at the stream).
    let mut lying = MapStore::default();
    let other = Object::fill(8, 8, 200)?;
    lying.insert_raw(cid, store::object_record(&other));
    assert_eq!(
        decoder::decode_with_store(&bytes, &lying).unwrap_err(),
        VoleError::IntegrityMismatch
    );
    Ok(())
}

#[test]
fn publish_rejects_non_standalone_streams() -> Result<(), VoleError> {
    // An extern stream is already store-backed; publishing it (which would
    // double-count its payloads) is refused.
    let tile = Object::raster(8, 8, gradient_samples(8, 8, 5, 1, 1))?;
    let mut st = EmbeddedStore::create(&temp_dir("publish-extern"))?;
    let cid = st.put(&store::object_record(&tile))?.id;
    let s1 = encoder::encode_stream_external(
        16,
        16,
        0,
        &[(1, cid)],
        &[Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        }],
        &[],
    )?;
    assert_eq!(
        store::publish_stream(&mut st, &s1).unwrap_err(),
        VoleError::StoreRequired
    );
    Ok(())
}

#[test]
fn optimize_rejects_non_standalone_streams_typed() -> Result<(), VoleError> {
    // `vole optimize` operates on standalone streams; a store-backed stream is
    // rejected typed, never silently rewritten.
    let tile = Object::raster(8, 8, gradient_samples(8, 8, 5, 1, 1))?;
    let mut st = EmbeddedStore::create(&temp_dir("opt-extern"))?;
    let cid = st.put(&store::object_record(&tile))?.id;
    let s1 = encoder::encode_stream_external(
        16,
        16,
        0,
        &[(1, cid)],
        &[Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        }],
        &[],
    )?;
    assert!(vole_video::optimize::optimize_stream(&s1).is_err());
    Ok(())
}

// ---------------------------------------------------------------------------
// EntropyFsStore adapter (feature `entropyfs-store`)
// ---------------------------------------------------------------------------

#[cfg(feature = "entropyfs-store")]
#[test]
fn entropyfs_adapter_matches_embedded_semantics() -> Result<(), VoleError> {
    use vole_video::store::EntropyFsStore;

    let dir = temp_dir("efs-engine");
    let obj = Object::raster(16, 8, gradient_samples(16, 8, 1, 1, 1))?;
    let payload = store::object_record(&obj);
    let mut es = EntropyFsStore::create(&dir)?;
    let o1 = es.put(&payload)?;
    assert!(o1.fresh);
    let o2 = es.put(&payload)?;
    assert_eq!(o1.id, o2.id, "same bytes -> same blob id");
    assert!(!o2.fresh, "engine dedups identical content");
    assert_eq!(es.get(o1.id, 1 << 20)?, Some(payload.clone()));
    assert!(es.contains(o1.id)?);
    assert_eq!(es.unique_count(), 1);
    assert!(es.physical_bytes() > 0);
    assert_eq!(
        o1.id,
        identity::content_id_of(&obj),
        "engine blob id == VOLE object content id (same BLAKE3 of same bytes)"
    );
    es.sync()?;
    es.close()?;

    // Cross-store identity: the same record through the EmbeddedStore and the
    // entropyfs engine has the same content id (both are BLAKE3 of the bytes).
    let mut emb = EmbeddedStore::create(&temp_dir("efs-embedded"))?;
    let o3 = emb.put(&payload)?;
    assert_eq!(o1.id, o3.id);

    // Reopen durability: after sync + close, a fresh engine sees the blob.
    let mut es2 = EntropyFsStore::open(&dir)?;
    assert_eq!(es2.get(o1.id, 1 << 20)?, Some(payload));
    assert!(!es2.contains(ContentId::from_array([0x11; 32]))?);
    es2.close()?;
    Ok(())
}

#[cfg(feature = "entropyfs-store")]
#[test]
fn entropyfs_adapter_missing_and_publish_roundtrip() -> Result<(), VoleError> {
    use vole_video::store::EntropyFsStore;

    // Publishing real prior-phase stream shapes through the entropyfs-backed
    // store must behave exactly as through the embedded store: sharing is by
    // content identity, independent of the backend.
    let dir = temp_dir("efs-publish");
    let mut es = EntropyFsStore::create(&dir)?;
    let logo = Object::raster(32, 32, gradient_samples(32, 32, 30, 1, -1))?;
    let mut declared = 0u64;
    for k in 0..3u32 {
        let bytes = static_stream(96, 96, 9, 1, logo.clone(), k as i64, k as i64)?;
        let p = store::publish_stream(&mut es, &bytes)?;
        declared += p.declared_bytes();
        if k == 0 {
            assert_eq!(p.new_objects, 1);
        } else {
            assert_eq!(p.new_objects, 0, "identical logo shared across videos");
            assert_eq!(p.reused_objects, 1);
        }
    }
    // The engine deduplicates to one blob whatever its internal representation
    // reports; exact payload-level byte accounting is asserted on the embedded
    // store, whose physical layout VOLE owns. Engine metrics stay advisory.
    assert_eq!(es.unique_count(), 1);
    assert!(es.physical_bytes() > 0);
    let acc = store::archive_accounting(&es, declared);
    assert_eq!(acc.unique_payloads, 1);
    assert!(
        acc.unique_payload_bytes <= declared,
        "unique payload volume never exceeds the declared attribution"
    );
    es.close()?;
    Ok(())
}
