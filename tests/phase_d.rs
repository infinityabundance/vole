//! Phase-D courts: COPY_RECT / MOVE_RECT machinery.
//!
//! COPY_RECT copies a declared rectangle from the *previous decoded frame*
//! into the current base frame via a sequential compositor (see
//! `src/decoder.rs::materialize_all` + `src/materialize.rs::rect_copy`). These
//! tests cover an end-to-end hand-verifiable scenario, parser tolerance of
//! MOVE_RECT, hostile geometry rejection, and the snapshot/clipping primitive.
//! The domain-winning terminal-scroll court needs a *transient-patch* operator
//! that belongs with the terminal/editor phase; that gap is recorded in
//! `docs/empirical-status.md`, not hidden.

use vole_video::{
    decoder,
    error::VoleError,
    object::{Object, ObjectId},
    pixel::Canvas,
    state::{Instance, InstanceId},
    transition::Transition,
};

const OBJECT_ID: u32 = 1;

fn fill4(v: u8) -> Object {
    Object::fill(4, 4, v).expect("fill fits")
}

fn instance_at(x: i64, y: i64) -> Instance {
    Instance {
        id: InstanceId(1),
        object_id: ObjectId(OBJECT_ID),
        x,
        y,
    }
}

#[test]
fn copy_rect_from_previous_frame_reconstructs_markers() -> Result<(), VoleError> {
    let w = 16u32;
    let h = 8u32;
    let objects = vec![(OBJECT_ID, fill4(9))];
    let c0 = instance_at(2, 1);
    let timeline = vec![(
        1u64,
        vec![
            Transition::SetPosition {
                id: InstanceId(1),
                x: 5,
                y: 1,
            },
            Transition::CopyRect {
                src_x: 2,
                src_y: 1,
                width: 4,
                height: 4,
                dst_x: 2,
                dst_y: 1,
            },
        ],
    )];

    let bytes = vole_video::encoder::encode_stream(w, h, 0, &objects, &[c0], &timeline)?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    assert_eq!(frames.len(), 2);

    // After the SetPosition + CopyRect-from-frame0 both placements are solid.
    block_present(&frames[1], 2, 1, 9);
    block_present(&frames[1], 5, 1, 9);
    assert_eq!(frames[1].get(1, 1), 0); // background left of markers
    assert_eq!(frames[1].get(9, 1), 0); // background right of the moved marker
                                        // frame0 holds the marker only at its original spot (cols 2..=5).
    assert_eq!(frames[0].get(1, 1), 0);
    assert_eq!(frames[0].get(5, 1), 9);
    block_present(&frames[0], 2, 1, 9);
    Ok(())
}

fn block_present(f: &Canvas, x0: i64, y0: i64, v: u8) {
    for dy in 0..4 {
        for dx in 0..4 {
            assert_eq!(f.get((x0 + dx) as u32, (y0 + dy) as u32), v);
        }
    }
}

#[test]
fn move_rect_parses_and_materializes() -> Result<(), VoleError> {
    let objects = vec![(OBJECT_ID, fill4(9))];
    let c0 = instance_at(2, 1);
    let timeline = vec![(
        1u64,
        vec![Transition::MoveRect {
            src_x: 2,
            src_y: 1,
            width: 4,
            height: 4,
            dst_x: 2,
            dst_y: 1,
        }],
    )];
    let bytes = vole_video::encoder::encode_stream(16, 8, 0, &objects, &[c0], &timeline)?;
    let parsed = decoder::decode_bytes(&bytes)?;
    assert_eq!(decoder::materialize_all(&parsed)?.len(), 2);
    Ok(())
}

#[test]
fn oversized_copy_area_rejected() {
    let objects = vec![(OBJECT_ID, fill4(9))];
    let c0 = instance_at(2, 1);
    let timeline = vec![(
        1u64,
        vec![Transition::CopyRect {
            src_x: 2,
            src_y: 1,
            width: 2000,
            height: 2000,
            dst_x: 2,
            dst_y: 1,
        }],
    )];
    let res = vole_video::encoder::encode_stream(16, 8, 0, &objects, &[c0], &timeline);
    assert_eq!(res.unwrap_err(), VoleError::MaterializationBudgetExceeded);
}

#[test]
fn zero_size_copy_rejected() {
    let objects = vec![(OBJECT_ID, fill4(9))];
    let c0 = instance_at(2, 1);
    let timeline = vec![(
        1u64,
        vec![Transition::CopyRect {
            src_x: 0,
            src_y: 0,
            width: 0,
            height: 0,
            dst_x: 0,
            dst_y: 0,
        }],
    )];
    let res = vole_video::encoder::encode_stream(16, 8, 0, &objects, &[c0], &timeline);
    assert_eq!(res.unwrap_err(), VoleError::NonCanonicalEncoding);
}

#[test]
fn rect_copy_is_snapshot_and_clips() {
    let mut src = Canvas::from_parts(4, 3, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]).unwrap();
    let mut dst = Canvas::from_parts(4, 3, vec![0; 12]).unwrap();
    vole_video::materialize::rect_copy(&mut dst, &src, 1, 1, 2, 2, 1, 1);
    assert_eq!(dst.get(1, 1), 6);
    assert_eq!(dst.get(2, 1), 7);
    assert_eq!(dst.get(1, 2), 10);
    assert_eq!(dst.get(2, 2), 11);
    assert_eq!(dst.get(0, 0), 0);
    src.set(1, 1, 42);
    assert_eq!(dst.get(1, 1), 6);
}
