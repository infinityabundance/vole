//! Phase-C courts: sparse mutation (blinking/strobe overlay), canonical sorted
//! enforcement, and cost that is proportional to changed pixels, not to full
//! raster repetition.

use vole_video::{
    decoder, demo,
    error::VoleError,
    object::{Object, ObjectId},
    state::{Instance, InstanceId},
    transition::Transition,
};

#[test]
fn blink_court_materializes_exactly_reference() -> Result<(), VoleError> {
    let court = demo::BlinkCourt::default();
    let parsed = decoder::decode_bytes(&court.vole()?)?;
    assert_eq!(parsed.frame_count(), 65);
    let frames = court.materialize_and_verify()?;
    assert_eq!(frames.len(), 65);
    Ok(())
}

#[test]
fn blink_pixel_flips_outcome_is_exact() -> Result<(), VoleError> {
    let court = demo::BlinkCourt::default();
    let parsed = decoder::decode_bytes(&court.vole()?)?;
    let frames = decoder::materialize_all(&parsed)?;
    // First overlay frame (f=1, odd) is value 0 over object 128; rest 255.
    assert_eq!(frames[1].get(50, 20), 0);
    // Even f=2 should be 255.
    assert_eq!(frames[2].get(50, 20), 255);
    // Non-overlay background for that pixel at f0 is the object 128.
    assert_eq!(frames[0].get(50, 20), 128);
    Ok(())
}

#[test]
fn unsorted_sparse_patch_rejected() {
    // overlay_batch rejects a non-canonical (unsorted/duplicate) ordering.
    let obj = Object::fill(4, 4, 9).unwrap();
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let tr = Transition::PatchSparse {
        points: vec![(2, 0, 5), (1, 0, 6)],
    };
    let res = vole_video::encoder::encode_stream(16, 16, 0, &[(1, obj)], &[inst], &[(1, vec![tr])]);
    assert_eq!(res.unwrap_err(), VoleError::NonCanonicalEncoding);
}

#[test]
fn sparse_stream_cost_tracks_changed_pixels_not_frames() -> Result<(), VoleError> {
    // 64 intervals each change a single pixel; the stream must stay far below
    // the equivalent raster cost and scale with patches, not full frames.
    let court = demo::BlinkCourt::default();
    let bytes = court.vole()?;
    let raw_all = court.reference_raw().len() as u64;
    assert!(
        bytes.len() as u64 * 1000 < raw_all,
        "sparse repr must be cheap"
    );
    Ok(())
}
