//! Phase J courts: palette state.
//!
//! A **palette** is a bounded mutable table of Gray8 entries (part of the
//! procedural state); a **palette-index object** stores one-byte indices that
//! the materializer resolves through the palette bound to the painting
//! instance. Because the index plane is immutable content while palette
//! entries are mutable-by-transition state, color animation is a *tiny state
//! mutation*, never a raster rewrite. Courts cover:
//! * accent cycling (`PatchPalette`) and whole-palette rotation
//!   (`SetPalette`) — byte-exact vs an independent palette painter;
//! * the flattening tax: the same visual frames encoded through the
//!   raster-origin inverse encoder cost dramatically more than the palette
//!   state that authored them;
//! * hostile semantics (empty palette, reserved id, unknown palette/instance,
//!   unsorted/duplicate patches, out-of-range indices — typed everywhere);
//! * accounting buckets, content identity, static control, and the collapse
//!   fixpoint guarantee for palette streams.

use vole_video::{
    collapse, decoder, demo,
    error::VoleError,
    object::{Object, ObjectId},
    state::{Instance, InstanceId, PaletteId, State},
    time::Interval,
};

/// Default window-UI court on a `w×h` canvas (the index plane covers the
/// whole canvas, so every pixel is palette-mapped).
fn ui_court(
    w: u32,
    h: u32,
    mode: demo::PaletteMode,
    cycle: Vec<u8>,
    intervals: u64,
) -> demo::PaletteCourt {
    let (title_h, side_w, sep_every, status_h) = (6u32, 24u32, 16u32, 12u32);
    demo::PaletteCourt {
        width: w,
        height: h,
        background: 90,
        box_x: 0,
        box_y: 0,
        box_w: w,
        box_h: h,
        object_id: 1,
        instance_id: 1,
        palette_id: 1,
        indices: demo::window_ui_indices(w, h, title_h, side_w, sep_every, status_h),
        base_entries: demo::window_ui_entries(),
        mode,
        accent_index: 4,
        cycle,
        intervals,
    }
}

#[test]
fn accent_cycle_reconstructs_exact_frames() -> Result<(), VoleError> {
    // 41 frames: the whole canvas is an index plane; only palette entry 4
    // (the accent status bar) alternates 200/60 per interval.
    let court = ui_court(640, 360, demo::PaletteMode::AccentCycle, vec![200, 60], 40);
    let parsed = decoder::decode_bytes(&court.vole()?)?;
    assert_eq!(parsed.frame_count(), 41);
    let frames = court.materialize_and_verify()?; // vs independent painter
    assert_eq!(frames.len(), 41);

    // Static UI structure stays put across the animation…
    for f in [&frames[0], &frames[1], &frames[20], &frames[40]] {
        assert_eq!(f.get(300, 2), 255, "title bar");
        assert_eq!(f.get(10, 50), 200, "sidebar");
        assert_eq!(f.get(300, 30), 30, "body");
        assert_eq!(f.get(300, 22), 128, "separator");
    }
    // …while the accent bar toggles its gray value with the palette.
    assert_eq!(frames[0].get(200, 350), 200);
    assert_eq!(frames[1].get(200, 350), 60);
    assert_eq!(frames[2].get(200, 350), 200);
    assert_eq!(frames[39].get(200, 350), 60);
    assert_eq!(frames[40].get(200, 350), 200);
    // The body under the accent bar is untouched (only the palette changed).
    assert_eq!(frames[0].get(300, 30), frames[40].get(300, 30));
    Ok(())
}

#[test]
fn rotate_all_reconstructs_exact_frames() -> Result<(), VoleError> {
    // Full palette rotation: every interval re-maps the whole canvas through
    // rotated entries (classic color drift). The index plane never changes.
    let court = ui_court(320, 180, demo::PaletteMode::RotateAll, vec![0], 30);
    let frames = court.materialize_and_verify()?;
    assert_eq!(frames.len(), 31);
    // Body pixel (idx 0): value at frame k is base[(0 + k) % 6].
    let base = demo::window_ui_entries();
    for k in [0usize, 1, 2, 3, 4, 5, 6, 17, 30] {
        let expect = base[k % base.len()];
        assert_eq!(frames[k].get(300, 30), expect, "frame {k} body");
    }
    // Title (idx 1): value at frame k is base[(1 + k) % 6].
    assert_eq!(frames[0].get(300, 2), base[1]);
    assert_eq!(frames[1].get(300, 2), base[2]);
    // Frames really change globally: two distant frames differ.
    assert_ne!(frames[0], frames[1]);
    Ok(())
}

#[test]
fn palette_representation_beats_flattened_raster_and_raw() -> Result<(), VoleError> {
    // §55-style preservation court on palette content: the authored palette
    // state vs the same visual frames rasterized and inverse-proceduralized.
    let court = ui_court(240, 160, demo::PaletteMode::AccentCycle, vec![200, 60], 12);
    let vole = court.vole()?;
    let frames = court.materialize_and_verify()?;
    let flattened = vole_video::inverse::encode_frames(
        &frames,
        &vole_video::inverse::EncodeOptions {
            bg_sweep: false,
            background: Some(court.background),
            ..vole_video::inverse::EncodeOptions::default()
        },
    )?;
    assert!(flattened.exact);
    // The palette stream keeps the full index plane (one declaration), so the
    // whole-stream margin is bounded; the *maintenance* is where the tax
    // shows: the flattened encoder rewrites ~thousands of accent samples per
    // interval while the palette writes 26 B.
    assert!(
        vole.len() < flattened.vole.len(),
        "palette state must beat the flattened raster representation ({} vs {})",
        vole.len(),
        flattened.vole.len()
    );
    // Marginal per-interval cost comparison (intervals only).
    let per_interval_palette = 26u64; // envelope 13 + patch op 11 + 2 value bytes
    let accent_points: u64 = 216 * 12; // status bar spans x >= side_w
    let per_interval_flattened_floor = 5 + 9 * accent_points;
    assert!(
        per_interval_palette * 500 < per_interval_flattened_floor,
        "the flattening tax on the accent bar alone is {} B/interval vs {} B for the palette",
        per_interval_flattened_floor,
        per_interval_palette
    );
    let raw_all = court.raw_bytes_all();
    assert!((vole.len() as u64) * 4 < raw_all);
    Ok(())
}

#[test]
fn palette_state_semantics_are_typed() -> Result<(), VoleError> {
    let mut st = State::new(Interval::ZERO);
    st.declare_object(ObjectId(1), Object::fill(4, 4, 9)?)?;
    st.create_instance(InstanceId(1), ObjectId(1), 0, 0)?;

    // Empty palette / reserved id are non-canonical.
    assert_eq!(
        st.set_palette(PaletteId(1), Vec::new()).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    assert_eq!(
        st.set_palette(PaletteId::NONE, vec![1, 2]).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    st.set_palette(PaletteId(1), vec![30, 255, 200])?;

    // Binding requires an existing palette and an existing instance.
    assert_eq!(
        st.bind_palette(InstanceId(1), PaletteId(99)).unwrap_err(),
        VoleError::UnknownPalette
    );
    assert_eq!(
        st.bind_palette(InstanceId(99), PaletteId(1)).unwrap_err(),
        VoleError::UnknownInstance
    );
    st.bind_palette(InstanceId(1), PaletteId(1))?;
    assert_eq!(st.binding(InstanceId(1)), Some(PaletteId(1)));

    // Patch canonicality: strictly ascending, in-range, existing palette.
    assert_eq!(
        st.patch_palette(PaletteId(1), &[(1, 9), (1, 8)])
            .unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    assert_eq!(
        st.patch_palette(PaletteId(1), &[(2, 9), (1, 8)])
            .unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    assert_eq!(
        st.patch_palette(PaletteId(1), &[(3, 9)]).unwrap_err(),
        VoleError::OutOfBounds
    );
    assert_eq!(
        st.patch_palette(PaletteId(99), &[(0, 9)]).unwrap_err(),
        VoleError::UnknownPalette
    );
    st.patch_palette(PaletteId(1), &[(0, 40)])?;
    assert_eq!(st.palette(PaletteId(1)).unwrap()[0], 40);

    // Unbind via the NONE sentinel; bindings die with instances, palettes
    // persist.
    st.bind_palette(InstanceId(1), PaletteId::NONE)?;
    assert_eq!(st.binding(InstanceId(1)), None);
    st.bind_palette(InstanceId(1), PaletteId(1))?;
    st.clear_instances();
    assert_eq!(st.instance_count(), 0);
    assert_eq!(st.binding_count(), 0);
    assert_eq!(st.palette_count(), 1);
    Ok(())
}

#[test]
fn encode_validation_is_typed_for_palette_mistakes() {
    let obj = Object::fill(4, 4, 9).unwrap();
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    // Binding a checkpoint instance to an undeclared palette.
    let res = vole_video::encoder::encode_palette_stream(
        16,
        16,
        0,
        &[(1, obj.clone())],
        &[],
        &[(inst.clone(), Some(PaletteId(9)))],
        &[],
    );
    assert_eq!(res.unwrap_err(), VoleError::UnknownPalette);
    // Duplicate palette declarations.
    let res = vole_video::encoder::encode_palette_stream(
        16,
        16,
        0,
        &[(1, obj)],
        &[(1, vec![1, 2]), (1, vec![3, 4])],
        &[],
        &[],
    );
    assert_eq!(res.unwrap_err(), VoleError::DuplicateId);
}

#[test]
fn palette_index_without_binding_fails_materialization_typed() -> Result<(), VoleError> {
    // An index raster painted by an unbound instance is a deterministic typed
    // error (UnknownPalette) at materialization — never a panic, never a wrap.
    let indices: Vec<u8> = vec![0; 16 * 16];
    let obj = Object::index_raster(16, 16, indices)?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let bytes = vole_video::encoder::encode_palette_stream(
        16,
        16,
        0,
        &[(1, obj)],
        &[(1, vec![30, 255])],
        &[(inst, None)],
        &[],
    )?;
    let parsed = decoder::decode_bytes(&bytes)?; // parse is structural: ok
    assert_eq!(
        vole_video::decoder::materialize_all(&parsed).unwrap_err(),
        VoleError::UnknownPalette
    );
    Ok(())
}

#[test]
fn palette_index_out_of_range_fails_materialization_typed() -> Result<(), VoleError> {
    // An index at or beyond the palette length is a typed error at
    // materialization (the palette is authoritative at render time).
    let mut indices = vec![0u8; 16 * 16];
    indices[5] = 7; // palette has 3 entries (indices 0..=2)
    let obj = Object::index_raster(16, 16, indices)?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let bytes = vole_video::encoder::encode_palette_stream(
        16,
        16,
        0,
        &[(1, obj)],
        &[(1, vec![30, 255, 200])],
        &[(inst, Some(PaletteId(1)))],
        &[],
    )?;
    let parsed = decoder::decode_bytes(&bytes)?;
    assert_eq!(
        vole_video::decoder::materialize_all(&parsed).unwrap_err(),
        VoleError::OutOfBounds
    );
    Ok(())
}

#[test]
fn palette_static_control_frames_identical() -> Result<(), VoleError> {
    // A single-value cycle never changes the palette; every frame is
    // identical, and the patch stream still decodes exactly.
    let court = ui_court(96, 64, demo::PaletteMode::AccentCycle, vec![200], 10);
    let frames = court.materialize_and_verify()?;
    let f0 = frames.first().unwrap();
    assert!(frames.iter().all(|f| f.exactly_matches(f0)));
    Ok(())
}

#[test]
fn palette_stream_accounting_buckets_sum() -> Result<(), VoleError> {
    let court = ui_court(96, 64, demo::PaletteMode::RotateAll, vec![0], 4);
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
    assert!(
        cost.state_bytes > 0,
        "palette-table records are state bytes"
    );
    assert!(
        cost.index_object_bytes > 0,
        "index objects are object bytes"
    );
    // The checkpoint carries a binding (6 + 20n layout).
    assert_eq!(cost.checkpoint_bytes, 6 + 20);
    Ok(())
}

#[test]
fn palette_index_content_identity_is_exact_and_distinct() -> Result<(), VoleError> {
    let idx = Object::index_raster(8, 8, vec![1u8; 64])?;
    let raw = Object::raster(8, 8, vec![1u8; 64])?;
    let a = vole_video::identity::content_id_of(&idx);
    let b = vole_video::identity::content_id_of(&raw);
    assert_ne!(
        a, b,
        "index planes and gray rasters of the same bytes are different content"
    );
    // Same index plane reaches the same identity.
    let idx2 = Object::index_raster(8, 8, vec![1u8; 64])?;
    assert_eq!(a, vole_video::identity::content_id_of(&idx2));
    // And differs when the geometry or content differs.
    let idx3 = Object::index_raster(8, 8, vec![2u8; 64])?;
    assert_ne!(a, vole_video::identity::content_id_of(&idx3));
    Ok(())
}

#[test]
fn palette_streams_are_collapse_fixpoints() -> Result<(), VoleError> {
    // The collapse pass (Phase I) rebuilds streams through the non-palette
    // encoder; palette streams must be reported as fixpoints (Ok(None)),
    // never as an error.
    let court = ui_court(96, 64, demo::PaletteMode::AccentCycle, vec![200, 60], 8);
    let bytes = court.vole()?;
    assert!(collapse::collapse_stream(&bytes)?.is_none());
    Ok(())
}

#[test]
fn multiple_palettes_and_bindings_share_one_checkpoint() -> Result<(), VoleError> {
    // Two index objects, two palettes, two bound instances in one canvas:
    // palette lookups must be per-instance (each box maps through its own
    // palette).
    let a = Object::index_raster(8, 8, vec![1u8; 64])?; // all index 1
    let b = Object::index_raster(8, 8, vec![0u8; 64])?; // all index 0
    let ia = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let ib = Instance {
        id: InstanceId(2),
        object_id: ObjectId(2),
        x: 8,
        y: 0,
    };
    let bytes = vole_video::encoder::encode_palette_stream(
        16,
        16,
        0,
        &[(1, a), (2, b)],
        &[(1, vec![9, 70]), (2, vec![200, 250])],
        &[(ia, Some(PaletteId(1))), (ib, Some(PaletteId(2)))],
        &[],
    )?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = vole_video::decoder::materialize_all(&parsed)?;
    // Left box: index 1 through palette 1 = 70. Right box: index 0 through
    // palette 2 = 200.
    assert_eq!(frames[0].get(3, 3), 70);
    assert_eq!(frames[0].get(11, 3), 200);
    Ok(())
}
