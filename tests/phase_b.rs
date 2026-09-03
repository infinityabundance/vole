//! Phase-B courts: persistent object identity (content addressing) and the
//! unchanged-state lane with amortized-cost accounting.

use vole_video::{demo, error::VoleError, identity, object::Object};

#[test]
fn object_content_identity_is_exact_and_stable() -> Result<(), VoleError> {
    let a = Object::fill(16, 8, 77)?;
    let b = Object::fill(16, 8, 77)?;
    let c = Object::fill(16, 8, 78)?;
    assert_eq!(identity::content_id_of(&a), identity::content_id_of(&b));
    assert_ne!(identity::content_id_of(&a), identity::content_id_of(&c));
    // Deterministic across repeated computation.
    assert_eq!(
        identity::content_id_of(&a).hex(),
        identity::content_id_of(&a).hex()
    );
    Ok(())
}

#[test]
fn content_id_digests_are_64_hex() -> Result<(), VoleError> {
    let o = Object::raster(2, 2, vec![1, 2, 3, 4])?;
    assert_eq!(identity::content_id_of(&o).hex().len(), 64);
    Ok(())
}

#[test]
fn persistent_object_reuse_registry_counts_distinct() {
    let a = Object::fill(16, 8, 77).unwrap();
    let mut t = identity::ContentTable::default();
    t.insert(1, &a).unwrap();
    t.insert(2, &Object::fill(16, 8, 77).unwrap()).unwrap(); // same content reuse
    t.insert(3, &Object::fill(16, 8, 78).unwrap()).unwrap();
    assert_eq!(t.distinct(), 2);
    assert_eq!(t.total(), 3);
}

#[test]
fn static_scene_all_frames_identical() -> Result<(), VoleError> {
    let court = demo::StaticSceneCourt {
        intervals: 200,
        ..demo::StaticSceneCourt::default()
    };
    let (stream_bytes, frames, raw_all) = court.account()?;
    // All intervals are unchanged so materialize 201 identical views.
    assert_eq!(frames, 201);
    // Amortized unchanged-state cost: each empty interval is tiny (an interval
    // record), NOT a full raster. Verify the whole stream is far below raw.
    assert!(
        stream_bytes < raw_all / 100_000,
        "static stream too large relative to rasters"
    );
    Ok(())
}

#[test]
fn unchanged_frame_is_not_a_raster_repetition() -> Result<(), VoleError> {
    // 10k intervals + checkpoint + object + integrity (amortized ~13B/interval
    // here: 1 tag + 8 interval id + 4 transition count). Each unchanged frame
    // is NOT a full raster.
    let court = demo::StaticSceneCourt::default();
    let (stream_bytes, frames, raw_all) = court.account()?;
    assert_eq!(frames, 10_001);
    // Measured overhead is well under 1/100,000th of raster cost.
    assert!(
        stream_bytes * 100_000 < raw_all,
        "unchanged state must be amortized-cheap: {}B stream vs {}B raw",
        stream_bytes,
        raw_all
    );
    Ok(())
}
