//! Phase L courts: bounded fixed-point affine placement.
//!
//! An affine placement maps every destination pixel through a canonical Q8
//! source map `(a·x+b·y+c, d·x+e·y+f) >> 8` (integer everywhere, floor
//! rounding), so pan/zoom/rotation/camera-like transforms are *state*, not
//! per-frame rasters. Courts cover:
//! * quarter-turn rotation (exact in Q8) — byte-exact vs an independent
//!   incremental painter, with rotation-period invariants;
//! * integer 2× zoom and sub-pixel pan (exact Q8 semantics);
//! * a random-parameter cross-check of the two sampling implementations;
//! * the affine-vs-rasterization court (affine state streams are far smaller
//!   than re-encoding the same visual frames through the raster encoder);
//! * residual closure of a Q8 approximation against a float-rendered target
//!   (`F = M(state) ⊕_ρ R` with a persistent correction overlay);
//! * hostile bounds (out-of-domain coefficients, affine work budget,
//!   exclusivity with velocity/trajectory, unknown instances).

use vole_video::{
    affine::{AffineParams, AFFINE_SCALE},
    decoder, demo, encoder,
    error::VoleError,
    object::{Object, ObjectId},
    pixel::Canvas,
    state::{Instance, InstanceId, State},
    time::Interval,
    transition::Transition,
};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

/// Non-symmetric deterministic tile content with a distinctive mark at
/// content-local `(7, 3)` so rotations are visually testable.
fn tile_content(w: u32, h: u32) -> Vec<u8> {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = if x == 7 && y == 3 {
                250
            } else {
                ((x / 3 + y / 5) % 9) as u8 * 23 + 10
            };
            d.push(v);
        }
    }
    d
}

fn court_with_params(params: Vec<AffineParams>) -> demo::AffineCourt {
    let intervals = params.len() as u64;
    demo::AffineCourt {
        width: 160,
        height: 160,
        background: 90,
        tile_w: 64,
        tile_h: 64,
        content: tile_content(64, 64),
        plain_x: 48,
        plain_y: 48,
        object_id: 1,
        instance_id: 1,
        params,
        intervals,
    }
}

#[test]
fn quarter_turn_rotation_is_exact_and_periodic() -> Result<(), VoleError> {
    // One rotation per frame, cycling through the four quarter turns.
    let params: Vec<AffineParams> = (1..=40)
        .map(|k| demo::quarter_turn_params(k, 32, 32, 80, 80))
        .collect();
    let court = court_with_params(params);
    let parsed = decoder::decode_bytes(&court.vole()?)?;
    assert_eq!(parsed.frame_count(), 41);
    let frames = court.materialize_and_verify()?; // byte-exact vs painter
    assert_eq!(frames.len(), 41);

    // Distinct orientation every quarter turn; full period returns to frame 0.
    for a in 0..4 {
        assert_ne!(frames[a], frames[(a + 1) % 4], "quarter turns must differ");
    }
    assert_eq!(frames[4], frames[0], "four quarter turns = identity");
    assert_eq!(frames[8], frames[0]);
    assert_eq!(frames[40], frames[0]);

    // The mark (content-local (7,3) = 250) moves with the rotation.
    assert_eq!(frames[0].get(48 + 7, 48 + 3), 250); // plain placement
    assert_eq!(frames[1].get(109, 55), 250, "90 CW"); // x' = cx - (v-v0)
    assert_eq!(frames[2].get(105, 109), 250, "180"); // point reflection
                                                     // A 90-degree rotation of a centered square covers the same region: the
                                                     // mark's old spot now shows the content pixel that rotated into it
                                                     // (content-local (3, 57), pattern value 79), not the background.
    assert_eq!(frames[1].get(55, 51), 79);
    assert_eq!(frames[1].get(80, 80), frames[0].get(80, 80));
    Ok(())
}

#[test]
fn integer_zoom_and_subpixel_pan_are_exact() -> Result<(), VoleError> {
    // Zoom 2x about the canvas center for a few frames, then a sub-pixel
    // (0.5 px/frame) pan; both are exact Q8 maps, verified vs the painter.
    let zoom = demo::zoom2_params(32, 32, 80, 80);
    let panned = demo::pan_params(48, 48, 1, 2);
    let params = vec![zoom, panned, demo::quarter_turn_params(0, 32, 32, 80, 80)];
    let court = court_with_params(params);
    let frames = court.materialize_and_verify()?;
    assert_eq!(frames.len(), 4);
    // Zoom: the mark (content (7,3)) is sampled at dest (80 + 2*(7-32),
    // 80 + 2*(3-32)) = (30, 22) under the floor rule used by both sides.
    assert_eq!(frames[1].get(30, 22), 250);
    // Sub-pixel pan: content moved right by half a pixel (floor semantics).
    let _ = frames[2];
    Ok(())
}

#[test]
fn affine_state_matches_reference_under_random_parameters() -> Result<(), VoleError> {
    // Deterministic pseudo-random valid parameter sets: the two independent
    // sampling implementations must agree pixel-for-pixel on every frame.
    let mut seed = 0xFEED_0123u64;
    let mut rnd = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed = seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        seed >> 40
    };
    let mut params = Vec::new();
    for _ in 0..12 {
        let base = rnd() % 513; // 0..512 offset
        params.push(AffineParams {
            a: AFFINE_SCALE / 2 + (rnd() % 9) as i64, // near 0.5..1.0 px/px
            b: (rnd() % 41) as i64 - 20,
            c: -(AFFINE_SCALE * 48) + base as i64,
            d: (rnd() % 41) as i64 - 20,
            e: AFFINE_SCALE / 2 + (rnd() % 9) as i64,
            f: -(AFFINE_SCALE * 48) + base as i64,
        });
    }
    let court = court_with_params(params);
    court.check().expect("params valid");
    let frames = court.materialize_and_verify()?;
    assert_eq!(frames.len(), 13);
    Ok(())
}

#[test]
fn affine_state_beats_re_encoding_the_same_frames() -> Result<(), VoleError> {
    // The 40-frame rotation sequence as affine state vs the same visual
    // frames re-encoded through the raster-origin encoder (which has no
    // affine discovery — the flattening tax is the Phase-O/Optimize surface).
    let params: Vec<AffineParams> = (1..=40)
        .map(|k| demo::quarter_turn_params(k, 32, 32, 80, 80))
        .collect();
    let court = court_with_params(params);
    let vole = court.vole()?;
    let frames = court.materialize_and_verify()?;
    assert_eq!(frames.len(), 41);
    let flattened = vole_video::inverse::encode_frames(
        &frames,
        &vole_video::inverse::EncodeOptions {
            bg_sweep: false,
            background: Some(court.background),
            ..vole_video::inverse::EncodeOptions::default()
        },
    )?;
    assert!(flattened.exact);
    assert!(
        vole.len() * 4 < flattened.vole.len(),
        "affine state must be far smaller than re-encoding the rotation \
         ({} vs {})",
        vole.len(),
        flattened.vole.len()
    );
    Ok(())
}

#[test]
fn residual_closes_the_approximation_gap_exactly() -> Result<(), VoleError> {
    // Target: the tile rotated 30 degrees by a *float* renderer (nearest
    // sampling, court-side only). VOLE state carries the best Q8 affine
    // approximation of that rotation; the residual algebra closes the exact
    // gap with a persistent sparse correction. The decoded stream must equal
    // the float-rendered target byte-for-byte.
    let (w, h) = (160u32, 160u32);
    let content = tile_content(64, 64);
    let bg = 90u8;

    // Float render of the 30-degree-rotated tile (content local center 32,32;
    // destination center 80,80). Rotation by +30 deg CW maps content (u,v):
    //   x = 80 + cos*(u-32) - sin*(v-32)   ... inverse for sampling below.
    // Sample rule: dest (x,y) samples source
    //   u = 32 + cos*(x-80) + sin*(y-80);  v = 32 - sin*(x-80) + cos*(y-80)
    // (rotation of the *content* by -30 about the center) with the same
    // floor sampling the normative map uses.
    let (cos, sin) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let mut float_frame = vec![bg; (w * h) as usize];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let uf = 32.0 + cos * ((x - 80) as f64) + sin * ((y - 80) as f64);
            let vf = 32.0 - sin * ((x - 80) as f64) + cos * ((y - 80) as f64);
            let u = uf.floor() as i64;
            let v = vf.floor() as i64;
            if u < 0 || v < 0 || u >= 64 || v >= 64 {
                continue;
            }
            float_frame[y as usize * w as usize + x as usize] = content[(v * 64 + u) as usize];
        }
    }

    // Q8 approximation of the same map.
    let approx = AffineParams {
        a: (256.0 * cos).round() as i64,
        b: (256.0 * sin).round() as i64,
        c: 256 * 32 - (256.0 * cos).round() as i64 * 80 - (256.0 * sin).round() as i64 * 80,
        d: -(256.0 * sin).round() as i64,
        e: (256.0 * cos).round() as i64,
        f: 256 * 32 + (256.0 * sin).round() as i64 * 80 - (256.0 * cos).round() as i64 * 80,
    };

    // Materialize the approx-only stream to find the exact correction set.
    let obj = Object::raster(64, 64, content.clone())?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 48,
        y: 48,
    };
    let approx_only = encoder::encode_stream(
        w,
        h,
        bg,
        &[(1, obj.clone())],
        std::slice::from_ref(&inst),
        &[(
            1,
            vec![Transition::SetAffine {
                id: InstanceId(1),
                params: approx,
            }],
        )],
    )?;
    let base = decoder::materialize_all(&decoder::decode_bytes(&approx_only)?)?;
    // Diff the approx materialization against the float target: the exact
    // residual the approximation cannot explain.
    let target = canvas_of(w, h, float_frame.clone());
    let mut pts = Vec::new();
    let b = &base[1];
    // Strict x-major order (the canonical sparse point list order).
    for x in 0..w as i64 {
        for y in 0..h as i64 {
            let bv = b.get(x as u32, y as u32);
            let tv = target.get(x as u32, y as u32);
            if bv != tv {
                pts.push((x, y, tv));
            }
        }
    }
    assert!(
        !pts.is_empty(),
        "a Q8 30-degree approximation must leave a measurable gap"
    );
    // The gap is the Q8 rounding of a continuous rotation — a small fraction
    // of the tile (boundary crossings), never the whole raster.
    assert!(
        pts.len() < 1500,
        "residual must stay a bounded edge closure, got {} points",
        pts.len()
    );

    // Final stream: plain frame 0, then SetAffine + a persistent sparse
    // correction overlay; everything after is the unchanged lane.
    let mut groups = Vec::new();
    let mut trs = vec![
        Transition::SetAffine {
            id: InstanceId(1),
            params: approx,
        },
        Transition::PatchSparse {
            points: pts.clone(),
        },
    ];
    groups.push((1u64, std::mem::take(&mut trs)));
    for k in 2..=20u64 {
        groups.push((k, Vec::new()));
    }
    let bytes = encoder::encode_stream(w, h, bg, &[(1, obj)], &[inst], &groups)?;
    let frames = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
    // Frame 0 is the plain tile; frames 1.. are the float-rendered target.
    assert_eq!(frames.len(), 21);
    assert_eq!(frames[0].get(55, 51), 250);
    for f in &frames[1..] {
        assert_eq!(
            f.as_slice(),
            &float_frame[..],
            "affine + residual must reproduce the float render exactly"
        );
    }
    Ok(())
}

#[test]
fn affine_over_palette_index_object_materializes_exactly() -> Result<(), VoleError> {
    // A palette-index object under an affine placement resolves each sampled
    // index through the instance's bound palette (the affine `Kind::Index`
    // painter branch). Quarter turns about the box center are exact in Q8.
    let (w, h) = (96u32, 96u32);
    let bg = 40u8;
    let (iw, ih) = (48u32, 48u32);
    let entries: Vec<u8> = vec![11, 44, 77, 110, 143, 176, 209];
    let mut indices = Vec::with_capacity((iw * ih) as usize);
    for y in 0..ih {
        for x in 0..iw {
            indices.push(((x * 3 + y * 5) % 7) as u8);
        }
    }
    let obj = Object::index_raster(iw, ih, indices.clone())?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 24,
        y: 24,
    };
    // Frame 1: one quarter turn about the box center (48,48) == canvas center.
    let rot = demo::quarter_turn_params(1, 24, 24, 48, 48);
    let bytes = encoder::encode_palette_stream(
        w,
        h,
        bg,
        &[(1, obj)],
        &[(1, entries.clone())],
        &[(inst, Some(vole_video::state::PaletteId(1)))],
        &[(
            1,
            vec![Transition::SetAffine {
                id: InstanceId(1),
                params: rot,
            }],
        )],
    )?;
    let frames = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
    assert_eq!(frames.len(), 2);

    // Independent reference: per-destination sample through the same source
    // map, resolved through the palette — a distinct loop shape from the
    // materializer's scan.
    let mut expected = vec![bg; (w * h) as usize];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let (su, sv) = rot.source(x, y).expect("in-range coeffs");
            if su < 0 || sv < 0 || su >= i64::from(iw) || sv >= i64::from(ih) {
                continue;
            }
            let idx = indices[(sv * i64::from(iw) + su) as usize];
            expected[(y * i64::from(w) + x) as usize] = entries[usize::from(idx)];
        }
    }
    // Frame 0 is the plain palette blit; frame 1 is the quarter-turn view.
    assert_ne!(frames[0].as_slice(), &expected[..]);
    assert_eq!(frames[1].as_slice(), &expected[..]);
    // Spot checks: plain placement paints index-resolved values; the rotated
    // frame differs exactly where the map says so.
    assert_eq!(frames[0].get(24, 24), entries[0]);
    assert_eq!(frames[0].get(24 + 47, 24), entries[(47 * 3) % 7]);
    assert_eq!(frames[1].get(48, 48), entries[(24 * 3 + 24 * 5) % 7]);
    Ok(())
}

#[test]
fn affine_over_fill_object_materializes_exactly() -> Result<(), VoleError> {
    // A uniform fill object under an affine placement paints its value at
    // every destination whose source sample lies inside the declared box (the
    // affine `Kind::Fill` painter branch). An integer 2x zoom of an 8x8 fill
    // yields a solid 16x16 value square, exact in Q8.
    let (w, h) = (64u32, 64u32);
    let bg = 3u8;
    let value = 200u8;
    let obj = Object::fill(8, 8, value)?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 10,
        y: 10,
    };
    let zoom = demo::zoom2_params(4, 4, 32, 32); // 2x about the canvas center
    let bytes = encoder::encode_stream(
        w,
        h,
        bg,
        &[(1, obj)],
        &[inst],
        &[(
            1,
            vec![Transition::SetAffine {
                id: InstanceId(1),
                params: zoom,
            }],
        )],
    )?;
    let frames = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
    assert_eq!(frames.len(), 2);

    // Frame 0: plain 8x8 fill blit at (10,10).
    assert_eq!(frames[0].get(10, 10), value);
    assert_eq!(frames[0].get(17, 17), value);
    assert_eq!(frames[0].get(9, 9), bg);
    assert_eq!(frames[0].get(18, 10), bg);

    // Frame 1: dest (x,y) samples (x/2, y/2); zoom2 about the canvas center
    // maps content (u,v) to dest (2u+24, 2v+24), so the 8x8 fill covers the
    // dest square x,y in [24, 40).
    let mut expected = vec![bg; (w * h) as usize];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let (su, sv) = zoom.source(x, y).expect("in-range coeffs");
            if (0..8).contains(&su) && (0..8).contains(&sv) {
                expected[(y * i64::from(w) + x) as usize] = value;
            }
        }
    }
    assert_eq!(frames[1].as_slice(), &expected[..]);
    assert_eq!(frames[1].get(24, 24), value);
    assert_eq!(frames[1].get(39, 39), value);
    assert_eq!(frames[1].get(23, 24), bg);
    assert_eq!(frames[1].get(40, 24), bg);
    Ok(())
}

#[test]
fn affine_state_semantics_are_typed() -> Result<(), VoleError> {
    let mut st = State::new(Interval::ZERO);
    st.declare_object(ObjectId(1), Object::fill(4, 4, 9)?)?;
    st.create_instance(InstanceId(1), ObjectId(1), 0, 0)?;

    let rot = demo::quarter_turn_params(1, 2, 2, 8, 8);
    st.set_affine(InstanceId(1), rot)?;
    assert_eq!(st.affine(InstanceId(1)), Some(rot));
    assert_eq!(st.affine_count(), 1);

    // Exclusive: attaching a velocity removes the affine (and vice versa).
    st.set_velocity(InstanceId(1), 1, 1)?;
    assert_eq!(st.affine(InstanceId(1)), None);
    st.set_affine(InstanceId(1), rot)?;
    assert_eq!(st.velocity(InstanceId(1)), (0, 0));
    st.set_trajectory(
        InstanceId(1),
        vec![vole_video::trajectory::TrajectorySegment::Linear {
            vx: 1,
            vy: 0,
            steps: 3,
        }],
    )?;
    assert_eq!(st.affine(InstanceId(1)), None);

    // Identity affine deactivates; unknown instance is typed; out-of-domain
    // coefficients are typed.
    st.set_affine(InstanceId(1), AffineParams::IDENTITY)?;
    assert_eq!(st.affine_count(), 0);
    assert_eq!(
        st.set_affine(InstanceId(99), rot).unwrap_err(),
        VoleError::UnknownInstance
    );
    let bad = AffineParams {
        a: (1 << 24) + 1,
        ..AffineParams::IDENTITY
    };
    assert_eq!(
        st.set_affine(InstanceId(1), bad).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );

    // Affines die with their instances.
    st.set_affine(InstanceId(1), rot)?;
    st.clear_instances();
    assert_eq!(st.affine_count(), 0);
    Ok(())
}

#[test]
fn affine_work_budget_is_enforced_at_materialization() -> Result<(), VoleError> {
    // Nine affine instances on the flagship canvas exceed the default affine
    // work budget (8 full-canvas affine paints per materialization): parse
    // succeeds (each op is bounded) but materialization fails typed, never
    // panicking.
    let obj = Object::fill(1, 1, 7)?;
    let mut trs = Vec::new();
    let mut instances = Vec::new();
    for i in 1..=9u32 {
        instances.push(Instance {
            id: InstanceId(i),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        });
        trs.push(Transition::SetAffine {
            id: InstanceId(i),
            params: demo::quarter_turn_params(1, 1, 1, 960, 540),
        });
    }
    let bytes = encoder::encode_stream(1920, 1080, 0, &[(1, obj)], &instances, &[(1u64, trs)])?;
    let parsed = decoder::decode_bytes(&bytes)?; // parse is fine
    assert_eq!(
        vole_video::decoder::materialize_all(&parsed).unwrap_err(),
        VoleError::MaterializationBudgetExceeded
    );
    Ok(())
}

#[test]
fn affine_wire_hostile_forms_are_typed() -> Result<(), VoleError> {
    // Out-of-domain coefficient patched into a canonical affine stream must
    // fail typed at parse.
    let court = court_with_params(vec![demo::quarter_turn_params(1, 32, 32, 80, 80)]);
    let bytes = court.vole()?;
    let content = &bytes[..bytes.len() - 32];
    let tag = content
        .windows(1)
        .position(|w| w[0] == 0x30)
        .expect("one SetAffine tag");
    let mut b = bytes;
    // tag(1) + iid(4): first coefficient (a) at tag + 5.
    let at = tag + 5;
    b[at..at + 4].copy_from_slice(&((1i32 << 24) + 1).to_le_bytes());
    assert_eq!(
        decoder::decode_bytes(&b).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    Ok(())
}

#[test]
fn affine_stream_accounts_and_roundtrips() -> Result<(), VoleError> {
    let court = court_with_params(vec![
        demo::quarter_turn_params(1, 32, 32, 80, 80),
        demo::quarter_turn_params(2, 32, 32, 80, 80),
    ]);
    let bytes = court.vole()?;
    let cost = vole_video::inverse::account_stream(&bytes)?;
    let sum = cost.header_bytes
        + cost.object_bytes
        + cost.checkpoint_bytes
        + cost.transition_bytes
        + cost.residual_bytes
        + cost.model_bytes
        + cost.state_bytes
        + cost.dictionary_bytes
        + cost.index_bytes
        + cost.integrity_bytes;
    assert_eq!(sum, cost.total_bytes);
    assert_eq!(cost.total_bytes, bytes.len() as u64);
    Ok(())
}
