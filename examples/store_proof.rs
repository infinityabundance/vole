//! Phase P evidence proof: the optional content-addressed persistence substrate.
//!
//! Prints a deterministic receipt over the Phase-P courts — EmbeddedStore
//! dedup + physical accounting, cross-video exact-object and palette sharing,
//! GC closure, external object declarations (byte-identical materialization
//! with the payload outside the stream), and — under the `entropyfs-store`
//! feature — the engine adapter equivalence (same content ids, one blob,
//! reopen durability).
//!
//! Run: `cargo run --release --example store_proof`
//! (and with `--features entropyfs-store` for the adapter courts)

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use vole_video::{
    decoder, encoder, identity,
    object::{Object, ObjectId},
    state::{Instance, InstanceId, PaletteId},
    store::{self, EmbeddedStore, ObjectStore},
    transition::Transition,
    VoleError,
};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

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

fn static_stream(
    w: u32,
    h: u32,
    bg: u8,
    objects: &[(u32, Object)],
    instances: &[Instance],
) -> Result<Vec<u8>, VoleError> {
    encoder::encode_stream(w, h, bg, objects, instances, &[])
}

fn main() -> Result<(), VoleError> {
    // ------------------------------------------------------------------
    // Court 1: embedded-store dedup + physical accounting.
    // ------------------------------------------------------------------
    let dir = temp_dir("proof-embed");
    let mut st = EmbeddedStore::create(&dir)?;
    let logo = Object::raster(32, 32, gradient_samples(32, 32, 200, -1, 2))?;
    let logo_rec = store::object_record(&logo);
    let o1 = st.put(&logo_rec)?;
    let o2 = st.put(&logo_rec)?;
    println!(
        "embedded-store: put-twice fresh=[{},{}] unique={} physical={}B payload={}B",
        o1.fresh,
        o2.fresh,
        st.unique_count(),
        st.physical_bytes(),
        st.unique_payload_bytes()
    );
    assert_eq!(o1.id, identity::content_id_of(&logo));
    assert!(!o2.fresh);
    assert_eq!(st.unique_count(), 1);

    // ------------------------------------------------------------------
    // Court 2: cross-video sharing — four videos, one shared logo.
    // ------------------------------------------------------------------
    let dir2 = temp_dir("proof-shared");
    let mut st = EmbeddedStore::create(&dir2)?;
    let panels = [
        Object::raster(24, 16, gradient_samples(24, 16, 10, 1, 0))?,
        Object::raster(24, 16, gradient_samples(24, 16, 90, 2, 3))?,
        Object::raster(24, 16, gradient_samples(24, 16, 170, -2, 1))?,
        Object::raster(24, 16, gradient_samples(24, 16, 40, 0, 4))?,
    ];
    let mut declared = 0u64;
    for (k, panel) in panels.iter().enumerate() {
        let bytes = static_stream(
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
        )?;
        let p = store::publish_stream(&mut st, &bytes)?;
        declared += p.declared_bytes();
        println!(
            "publish video-{}: declared={}B objects={} (new={} reused={})",
            k,
            p.declared_bytes(),
            p.object_ids.len(),
            p.new_objects,
            p.reused_objects
        );
    }
    // Palette sharing: two videos, identical palette tables.
    let entries = vec![10u8, 40, 90, 150, 220, 30, 60];
    let idx: Vec<u8> = (0..(16u32 * 16))
        .map(|i| ((i / 128 + i % 16 / 8) % 7) as u8)
        .collect();
    let idx_obj = Object::index_raster(16, 16, idx)?;
    for k in 0..2 {
        let bytes = encoder::encode_palette_stream(
            160,
            120,
            0,
            &[(1, idx_obj.clone())],
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
        declared += p.declared_bytes();
        println!(
            "publish palette-video-{}: declared={}B palette_snapshots={} (new={}) objects={} (new={} reused={})",
            k,
            p.declared_bytes(),
            p.palette_ids.len(),
            p.new_palettes,
            p.object_ids.len(),
            p.new_objects,
            p.reused_objects
        );
    }
    let acc = store::archive_accounting(&st, declared);
    println!(
        "archive: unique_payloads={} unique_payload_bytes={}B physical={}B declared={}B dedup_saved={}B",
        acc.unique_payloads,
        acc.unique_payload_bytes,
        acc.physical_bytes,
        acc.declared_bytes,
        acc.dedup_saved_bytes
    );
    assert_eq!(
        acc.unique_payloads, 7,
        "5 video objects + index object + palette snapshot"
    );

    // ------------------------------------------------------------------
    // Court 3: GC closure.
    // ------------------------------------------------------------------
    let dir3 = temp_dir("proof-gc");
    let mut st3 = EmbeddedStore::create(&dir3)?;
    let recs = ["a", "b", "c", "d"]
        .map(|t| store::object_record(&Object::fill(8, 8, t.as_bytes()[0]).expect("fill")));
    let ids: Vec<_> = recs
        .iter()
        .map(|r| st3.put(r).map(|o| o.id))
        .collect::<Result<_, _>>()?;
    let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
    st3.set_root("video-1", &[a, b])?;
    st3.set_root("video-2", &[a, c])?;
    let h = |id: &vole_video::store::ContentId| id.hex()[..8].to_string();
    let r1 = st3.gc()?;
    println!(
        "gc after roots video-1={},{} video-2={},{}: reclaimed={}B retained={} (d collected)",
        h(&a),
        h(&b),
        h(&a),
        h(&c),
        r1.reclaimed_bytes,
        r1.retained_ids
    );
    assert!(!st3.contains(d)?);
    assert!(st3.drop_root("video-1")?);
    let r2 = st3.gc()?;
    println!(
        "gc after dropping video-1: reclaimed={}B retained={} (b collected, a+c live)",
        r2.reclaimed_bytes, r2.retained_ids
    );
    assert!(!st3.contains(b)? && st3.contains(a)? && st3.contains(c)?);
    assert!(st3.drop_root("video-2")?);
    let r3 = st3.gc()?;
    println!(
        "gc after dropping video-2: reclaimed={}B retained={} (full closure)",
        r3.reclaimed_bytes, r3.retained_ids
    );
    assert_eq!(r3.retained_ids, 0);

    // ------------------------------------------------------------------
    // Court 4: external object declarations — identical materialization.
    // ------------------------------------------------------------------
    let (w, h) = (320u32, 200u32);
    let bg = 70u8;
    let tile = Object::raster(24, 16, gradient_samples(24, 16, 120, 2, -1))?;
    let backdrop = Object::procedural(
        w,
        h,
        vole_video::generator::Generator::Gradient {
            base: 10,
            sx: 1,
            sy: 1,
        },
    )?;
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
    let s0 = encoder::encode_stream(
        w,
        h,
        bg,
        &[(1, backdrop.clone()), (2, tile.clone())],
        &instances,
        &timeline,
    )?;
    let f0 = decoder::materialize_all(&decoder::decode_bytes(&s0)?)?;
    let dir4 = temp_dir("proof-extern");
    let mut st4 = EmbeddedStore::create(&dir4)?;
    let cid_b = st4.put(&store::object_record(&backdrop))?.id;
    let cid_t = st4.put(&store::object_record(&tile))?.id;
    let s1 = encoder::encode_stream_external(
        w,
        h,
        bg,
        &[(1, cid_b), (2, cid_t)],
        &instances,
        &timeline,
    )?;
    let parsed = decoder::decode_with_store(&s1, &st4)?;
    let f1 = decoder::materialize_all(&parsed)?;
    let identical = f0.len() == f1.len() && f0.iter().zip(&f1).all(|(a, b)| a.exactly_matches(b));
    let standalone_err = decoder::decode_bytes(&s1).is_err();
    println!(
        "extern: standalone={}B external={}B payload_moved_out={}B frames={} identical={} storeless_decode_rejected={}",
        s0.len(),
        s1.len(),
        s0.len() - s1.len(),
        f1.len(),
        identical,
        standalone_err
    );
    assert!(identical && standalone_err && s1.len() < s0.len());

    // ------------------------------------------------------------------
    // Court 5 (feature `entropyfs-store`): engine adapter equivalence.
    // ------------------------------------------------------------------
    #[cfg(feature = "entropyfs-store")]
    {
        use vole_video::store::EntropyFsStore;
        let dir5 = temp_dir("proof-efs");
        let mut es = EntropyFsStore::create(&dir5)?;
        let p0 = es.put(&logo_rec)?;
        let p1 = es.put(&logo_rec)?;
        let got = es.get(p0.id, 1 << 20)?;
        println!(
            "entropyfs-store: fresh=[{},{}] unique={} physical={}B get_exact={} id_matches_vole={}",
            p0.fresh,
            p1.fresh,
            es.unique_count(),
            es.physical_bytes(),
            got.as_deref() == Some(logo_rec.as_slice()),
            p0.id == identity::content_id_of(&logo)
        );
        assert!(p0.fresh && !p1.fresh && p0.id == o1.id);
        es.sync()?;
        es.close()?;
        let mut es2 = EntropyFsStore::open(&dir5)?;
        let got2 = es2.get(p0.id, 1 << 20)?;
        println!(
            "entropyfs-store: reopen durable={}",
            got2.as_deref() == Some(logo_rec.as_slice())
        );
        es2.close()?;
    }

    println!("store proof: OK");
    Ok(())
}
