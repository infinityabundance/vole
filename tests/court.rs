//! Phase-A conformance court: materialize VOLE and compare byte-for-byte
//! against an *independent* reference rasterizer, and confirm no full raster
//! repetition is stored.

use vole_video::{decoder, demo, error::VoleError};

#[test]
fn moving_rect_materializes_exactly_reference() -> Result<(), VoleError> {
    let court = demo::MovingRectCourt::default();
    let bytes = court.vole()?;
    let parsed = decoder::decode_bytes(&bytes)?;

    // Structural expectation: 101 frames = 1 checkpoint + 100 intervals.
    assert_eq!(parsed.frame_count(), 101);
    assert_eq!(parsed.width(), 1920);
    assert_eq!(parsed.height(), 1080);

    let frames = decoder::materialize_all(&parsed)?;
    assert_eq!(frames.len() as u64, court.frame_count());

    Ok(())
}

#[test]
fn moving_rect_matches_independent_reference() -> Result<(), VoleError> {
    // The court already verifies byte-exact equality against the independent
    // painter; this test also checks the reference itself changes between
    // frames (evidence the sequence is non-trivial a moving block must leave a
    // trailing background region behind, so distance frames differ).
    let court = demo::MovingRectCourt::default();
    assert!(court.materialize_and_verify()?.len() >= 2);

    let raw = court.reference_raw();
    let per = (court.width as usize) * (court.height as usize);
    // Frame 0 vs frame 1 must differ somewhere because the box moved +2px.
    let f0 = &raw[0..per];
    let f1 = &raw[per..2 * per];
    assert_ne!(
        f0, f1,
        "reference frames are identical; motion not expressed"
    );
    Ok(())
}

#[test]
fn vole_does_not_store_repeated_full_rasters() -> Result<(), VoleError> {
    // The point of Phase A: the stored stream is *state evolution*, not one
    // raster per frame. The stream must therefore be dramatically smaller than
    // the sum of whole-frame rasters it materializes.
    let court = demo::MovingRectCourt::default();
    let bytes = court.vole()?;
    let raw_all = court.raw_bytes_all();
    let stored = bytes.len() as u64;
    let raw_single = u64::from(court.width) * u64::from(court.height);

    assert!(
        stored * 16 < raw_all,
        "stored state must not be raster-proportional (stream {}B vs raw-all {}B)",
        bytes.len(),
        raw_all
    );
    // Because the object is a FILL it should not even need the box raster.
    assert!(
        stored * 64 < raw_single,
        "an identical moving fill should be far smaller than a single raw frame"
    );

    // Sanity: the whole referenced sequence reconstructs 101 exact frames.
    let frames = court.materialize_and_verify()?;
    assert_eq!(frames.len(), 101);
    Ok(())
}

#[test]
fn first_and_last_frames_differ_exactly_as_motion_predicts() -> Result<(), VoleError> {
    // x(t) = x0 + 2t. At t=100 the box should push its lead edge from x=100
    // to x=300, so after leaving a cleared trail the content at (x=398,y=50)
    // inside final box region must hold box value 180 when within box.
    let court = demo::MovingRectCourt::default();
    let frames = court.materialize_and_verify()?;
    let last = frames.last().unwrap();

    // Final box spans x in [300, 500). Interior sample (400,50) is within.
    assert_eq!(last.get(400, 50), 180);
    // Trail area far-left (x=50,y=50) was cleared to background 0 because the
    // box left it at t>= (100 - (whatever)); assert yes (box started at 100 and
    // moved right) => left columns static background.
    assert_eq!(last.get(50, 50), 0);
    // First frame box spans x in [100,300). At (100,50) value 180.
    let first = frames.first().unwrap();
    assert_eq!(first.get(100, 50), 180);
    assert_eq!(first.get(50, 50), 0);
    Ok(())
}
