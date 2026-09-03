//! Phase E courts: persistent integer translation state.
//!
//! `position(t+1) = position(t) + (vx, vy)` is first-class procedural state
//! (a per-instance translation applied once per `AdvanceTranslations`), *not*
//! a codec-local block-motion trick. Courts cover:
//! * moving objects (constant-velocity instance) — byte-exact vs an
//!   independent painter, and strictly cheaper than per-frame `SetPosition`;
//! * camera-like translation (a large region translating by whole pixels);
//! * static control (zero translation ⇒ every frame identical);
//! * the noise negative control (a translation hypothesis that does not
//!   reproduce the target is rejected by the exactness gate, never silently
//!   approximated).

use vole_video::{
    decoder, demo,
    error::VoleError,
    object::{Object, ObjectId},
    state::{Instance, InstanceId},
    transition::Transition,
};

#[test]
fn persistent_translation_reconstructs_exact_frames() -> Result<(), VoleError> {
    let court = demo::TranslationCourt::default(); // vx=2, vy=1, 100 intervals
    let parsed = decoder::decode_bytes(&court.vole()?)?;
    assert_eq!(parsed.frame_count(), 101);
    let frames = court.materialize_and_verify()?; // byte-exact vs independent painter
    assert_eq!(frames.len(), 101);

    // Motion is real: first and last frames differ; interior sample follows the
    // analytic position.
    let first = frames.first().unwrap();
    let last = frames.last().unwrap();
    // box 200x100 starting (100,60); frame100 at (300,160).
    assert_eq!(last.get(320, 200), 180, "box interior at final position");
    assert_eq!(last.get(80, 60), 0, "box has left its origin region");
    assert_eq!(first.get(150, 100), 180);
    Ok(())
}

#[test]
fn persistent_translation_beats_per_frame_set_position_bytes() -> Result<(), VoleError> {
    let court = demo::TranslationCourt::default();
    let trans = court.vole()?;
    let baseline = court.delta_baseline_bytes()?; // same frames via SetPosition
    assert!(
        trans.len() < baseline.len(),
        "persistent translation must be smaller than per-frame SetPosition ({} vs {})",
        trans.len(),
        baseline.len()
    );
    let raw_all = court.raw_bytes_all();
    assert!(
        (trans.len() as u64) * 1000 < raw_all,
        "representation must not be raster-proportional"
    );
    // Both streams decode to the identical reference sequence.
    assert_eq!(
        decoder::materialize_all(&decoder::decode_bytes(&trans)?)?,
        decoder::materialize_all(&decoder::decode_bytes(&baseline)?)?
    );
    Ok(())
}

#[test]
fn camera_like_translation_is_exact() -> Result<(), VoleError> {
    // A large "scene region" translating horizontally in whole-pixel steps
    // across the canvas (camera-like translation of persistent content).
    let court = demo::TranslationCourt {
        width: 1280,
        height: 720,
        box_w: 640,
        box_h: 720, // tall region; translation exposes background at the trailing edge
        x0: 0,
        y0: 0,
        vx: 4,
        vy: 0,
        intervals: 200,
        ..demo::TranslationCourt::default()
    };
    let frames = court.materialize_and_verify()?;
    assert_eq!(frames.len(), 201);
    // Leading edge moved 4*200 = 800 px.
    assert_eq!(frames.last().unwrap().get(900, 360), 180);
    Ok(())
}

#[test]
fn static_control_zero_translation_all_frames_identical() -> Result<(), VoleError> {
    let court = demo::TranslationCourt {
        vx: 0,
        vy: 0,
        ..demo::TranslationCourt::default()
    };
    let frames = court.materialize_and_verify()?;
    let f0 = frames.first().unwrap();
    assert!(frames.iter().all(|f| f.exactly_matches(f0)));
    Ok(())
}

#[test]
fn noise_negative_control_translation_hypothesis_rejected() {
    // Deterministic random walk: a translation hypothesis cannot be lossless.
    let mut positions: Vec<(i64, i64)> = Vec::new();
    let mut x = 100i64;
    let mut y = 60i64;
    let mut seed = 0x5EED_1234u64;
    for _ in 0..64 {
        positions.push((x, y));
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        let step = ((seed.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 60) as i64 % 7 - 3; // -3..3
        x += step;
        y += step ^ 1;
    }
    // Exact-velocity hypothesis check: constant velocity cannot explain a walk.
    assert!(!demo::translation_hypothesis_exact(
        100, 60, 2, 1, &positions
    ));
    // Sanity: an exactly-constant trajectory *is* accepted by the gate.
    let const_pos: Vec<(i64, i64)> = (0..64)
        .map(|k| (100 + 2 * k as i64, 60 + k as i64))
        .collect();
    assert!(demo::translation_hypothesis_exact(
        100, 60, 2, 1, &const_pos
    ));
}

#[test]
fn unknown_instance_velocity_is_typed_error() {
    let obj = Object::fill(4, 4, 9).unwrap();
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let res = vole_video::encoder::encode_stream(
        16,
        16,
        0,
        &[(1, obj)],
        &[inst],
        &[(
            1,
            vec![Transition::SetVelocity {
                id: InstanceId(99),
                vx: 1,
                vy: 0,
            }],
        )],
    );
    assert_eq!(res.unwrap_err(), VoleError::UnknownInstance);
}

#[test]
fn velocity_work_budget_rejected_by_encoder() {
    // Many moving instances advanced for many intervals exceed the cumulative
    // translation-work budget; the encoder must refuse with a typed error
    // quickly (never hang, never serialize a DoS stream).
    let w = 64u32;
    let h = 64u32;
    let obj = Object::fill(8, 8, 7).unwrap();
    let objects = vec![(1u32, obj)];
    let mut instances = Vec::new();
    for i in 0..200u32 {
        instances.push(Instance {
            id: InstanceId(i + 1),
            object_id: ObjectId(1),
            x: i as i64 * 2,
            y: 0,
        });
    }
    let mut velocities = Vec::new();
    for i in 0..200u32 {
        velocities.push(Transition::SetVelocity {
            id: InstanceId(i + 1),
            vx: 1,
            vy: 0,
        });
    }
    let mut timeline = vec![(1u64, velocities)];
    for k in 2..=50_000u64 {
        timeline.push((k, vec![Transition::AdvanceTranslations]));
    }
    let res = vole_video::encoder::encode_stream(w, h, 0, &objects, &instances, &timeline);
    assert_eq!(res.unwrap_err(), VoleError::MaterializationBudgetExceeded);
}

#[test]
fn velocity_work_budget_rejected_by_parser() -> Result<(), VoleError> {
    // Hostile-file court: a crafted stream whose cumulative translation work
    // exceeds the envelope must be rejected by the *parser* with a typed error,
    // quickly and without hanging.
    let mut wr = vole_video::format::StreamWriter::begin(64, 64);
    wr = wr.declare_object(ObjectId(1), Object::fill(2, 2, 5)?)?;
    let instances: Vec<Instance> = (0..2000u32)
        .map(|i| Instance {
            id: InstanceId(i + 1),
            object_id: ObjectId(1),
            x: i64::from(i % 60),
            y: 0,
        })
        .collect();
    wr = wr.checkpoint_with(&instances)?;
    let velocities: Vec<Transition> = (0..2000u32)
        .map(|i| Transition::SetVelocity {
            id: InstanceId(i + 1),
            vx: 1,
            vy: 0,
        })
        .collect();
    wr = wr.interval(vole_video::time::Interval(1), &velocities)?;
    for k in 2..=3_000u64 {
        wr = wr.interval(
            vole_video::time::Interval(k),
            &[Transition::AdvanceTranslations],
        )?;
    }
    let bytes = wr.finish()?;
    assert_eq!(
        vole_video::decoder::decode_bytes(&bytes).unwrap_err(),
        VoleError::MaterializationBudgetExceeded
    );
    Ok(())
}
