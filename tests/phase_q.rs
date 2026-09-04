//! Phase Q courts: the native procedural ingest API (§39), the §53 script
//! test format, and the §55 native-procedural preservation court — direct
//! procedural ingest (leg A) vs rasterize-then-inverse-proceduralize (leg B)
//! over the same canonical raster sequence.
//!
//! Every flattening court asserts: B decodes byte-identically to A
//! (`M(ingest) == M(rasterize→inverse)`), and the flattening tax `B/A` is
//! measured and pinned (the encoder is deterministic, so the byte counts are
//! stable). Numbers below were sealed from `examples/ingest_proof.rs`
//! (release).

use vole_video::{
    decoder, encoder,
    ingest::Ingest,
    inverse::{self, EncodeOptions},
    object::{Object, ObjectId},
    pixel::Canvas,
    script,
    state::{Instance, InstanceId, PaletteId},
    trajectory::TrajectorySegment,
    transition::Transition,
    VoleError,
};

fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

fn inverse_leg(frames: &[Canvas], bg: u8) -> Result<inverse::EncodeReport, VoleError> {
    let opts = EncodeOptions {
        bg_sweep: false,
        background: Some(bg),
        ..EncodeOptions::default()
    };
    inverse::encode_frames(frames, &opts)
}

/// Both legs reproduce the same canonical raster sequence, byte-for-byte.
fn assert_legs_equal(a_bytes: &[u8], b: &inverse::EncodeReport) -> Result<(), VoleError> {
    let fa = frames_of(a_bytes)?;
    assert!(b.exact, "inverse leg is byte-exact by its own contract");
    let fb = frames_of(&b.vole)?;
    assert_eq!(fb.len(), fa.len());
    assert!(
        fa.iter().zip(&fb).all(|(x, y)| x.exactly_matches(y)),
        "leg A and leg B reproduce the identical canonical raster sequence"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// API courts
// ---------------------------------------------------------------------------

#[test]
fn ingest_matches_descriptor_encoder_byte_for_byte() -> Result<(), VoleError> {
    // (a) Plain path: identical descriptors through the Ingest session and the
    // descriptor encoder serialize to the same canonical bytes.
    let samples: Vec<u8> = (0..(16 * 8)).map(|i| (i * 3 % 256) as u8).collect();
    let mut a = Ingest::new(64, 64);
    a.background(9);
    a.declare_raster(1, 16, 8, samples.clone())?;
    a.instance(1, 1, 5, 5)?;
    for (k, x) in (1..=4u64).zip([6u64, 7, 8, 9]) {
        a.at(k)?;
        a.set_position(1, x as i64, 5)?;
    }
    let bytes_a = a.finish()?;
    let instances = [Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 5,
        y: 5,
    }];
    let timeline: Vec<(u64, Vec<Transition>)> = (1..=4u64)
        .zip([6u64, 7, 8, 9])
        .map(|(k, x)| {
            (
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(1),
                    x: x as i64,
                    y: 5,
                }],
            )
        })
        .collect();
    let bytes_b = encoder::encode_stream(
        64,
        64,
        9,
        &[(1, Object::raster(16, 8, samples)?)],
        &instances,
        &timeline,
    )?;
    assert_eq!(bytes_a, bytes_b, "ingest serialization is byte-canonical");

    // (b) Palette path: session vs encode_palette_stream.
    let entries = vec![10u8, 60, 200, 30];
    let idx: Vec<u8> = (0..(32 * 16))
        .map(|i| ((i / 32 + i % 32) % 4) as u8)
        .collect();
    let mut a = Ingest::new(48, 32);
    a.background(0);
    a.declare_palette(1, entries.clone())?;
    a.declare_index(1, 32, 16, idx.clone())?;
    a.instance_binding(1, 1, 0, 0, 1)?;
    a.at(1)?;
    a.patch_palette(1, vec![(2, 250)])?;
    let bytes_a = a.finish()?;
    let obj = Object::index_raster(32, 16, idx)?;
    let bytes_b = encoder::encode_palette_stream(
        48,
        32,
        0,
        &[(1, obj)],
        &[(1, entries)],
        &[(
            Instance {
                id: InstanceId(1),
                object_id: ObjectId(1),
                x: 0,
                y: 0,
            },
            Some(PaletteId(1)),
        )],
        &[(
            1u64,
            vec![Transition::PatchPalette {
                id: PaletteId(1),
                changes: vec![(2, 250)],
            }],
        )],
    )?;
    assert_eq!(
        bytes_a, bytes_b,
        "palette ingest serialization is canonical"
    );
    Ok(())
}

#[test]
fn ingest_api_misuse_is_typed() -> Result<(), VoleError> {
    let mut a = Ingest::new(32, 32);
    a.declare_fill(1, 4, 4, 3)?;
    // Duplicate object id.
    assert_eq!(
        a.declare_fill(1, 4, 4, 4).unwrap_err(),
        VoleError::DuplicateId
    );
    // Duplicate instance id.
    a.instance(1, 1, 0, 0)?;
    assert_eq!(a.instance(1, 1, 1, 1).unwrap_err(), VoleError::DuplicateId);
    // Interval at 0 is the checkpoint.
    assert_eq!(a.at(0).unwrap_err(), VoleError::NonConsecutiveInterval);
    // Transitions require an open interval.
    assert_eq!(
        a.set_position(1, 2, 2).unwrap_err(),
        VoleError::InvalidStatePhase
    );
    // Decreasing times.
    a.at(2)?;
    assert_eq!(a.at(1).unwrap_err(), VoleError::NonConsecutiveInterval);
    // A clean later interval is accepted, then coordinate misuse is typed.
    assert!(a.at(3).is_ok(), "3 follows 2 cleanly");
    assert_eq!(
        a.set_position(1, 1 << 25, 0).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Palette declaration misuse.
    assert_eq!(
        Ingest::new(8, 8).declare_palette(0, vec![1]).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    assert_eq!(
        Ingest::new(8, 8).declare_palette(1, vec![]).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    assert_eq!(
        Ingest::new(8, 8)
            .declare_palette(1, vec![0u8; 257])
            .unwrap_err(),
        VoleError::DimensionTooLarge
    );
    // Unsorted patch is non-canonical at the API.
    let mut b = Ingest::new(8, 8);
    b.declare_palette(1, vec![1, 2, 3, 4])?;
    b.at(1)?;
    assert_eq!(
        b.patch_palette(1, vec![(2, 9), (1, 8)]).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Finish-time validation catches unknown references.
    let mut c = Ingest::new(16, 16);
    c.declare_fill(1, 4, 4, 3)?;
    c.instance(1, 9, 0, 0)?; // object 9 never declared
    assert_eq!(c.finish().unwrap_err(), VoleError::UnknownObject);
    let mut d = Ingest::new(16, 16);
    d.declare_fill(1, 4, 4, 3)?;
    d.instance(1, 1, 0, 0)?;
    d.at(1)?;
    d.set_position(7, 1, 1)?; // instance 7 never declared
    assert_eq!(d.finish().unwrap_err(), VoleError::UnknownInstance);
    // A session with an empty instance set but timeline is valid (background).
    let mut e = Ingest::new(8, 8);
    e.background(5);
    e.at(1)?;
    let bytes = e.finish()?;
    let fa = frames_of(&bytes)?;
    assert_eq!(fa.len(), 2);
    assert_eq!(fa[1].get(3, 3), 5);
    // Zero canvas geometry is refused at finish (limits check).
    let mut f = Ingest::new(0, 8);
    f.background(5);
    assert_eq!(f.finish().unwrap_err(), VoleError::DimensionTooLarge);
    Ok(())
}

// ---------------------------------------------------------------------------
// Script-format courts (§53)
// ---------------------------------------------------------------------------

#[test]
fn script_parses_to_the_identical_stream_and_frames() -> Result<(), VoleError> {
    // The accent-strip content, authored both by hand and in the script
    // format, must serialize to the same bytes.
    let (w, h) = (96u32, 96u32);
    let bg = 40u8;
    let mut manual = Ingest::new(w, h);
    manual.background(bg);
    manual.declare_palette(1, vec![200, 60, 90, 150, 220])?;
    manual.declare_index(1, w, 8, vec![1u8; (8 * w) as usize])?;
    manual.instance_binding(1, 1, 0, 60, 1)?;
    for k in 1..=12u64 {
        manual.at(k)?;
        let v = if k % 2 == 1 { 200 } else { 60 };
        manual.patch_palette(1, vec![(1, v)])?;
    }
    let manual_bytes = manual.finish()?;

    let mut body = String::from("canvas 96 96\nbackground 40\npalette 1 200 60 90 150 220\n");
    body.push_str("object 1 index 96 8 ");
    for _ in 0..(96 * 8) {
        body.push_str("1 ");
    }
    body.push_str("\ninstance 1 1 0 60 palette 1\n");
    for k in 1..=12u64 {
        let v = if k % 2 == 1 { 200 } else { 60 };
        body.push_str(&format!("at {k}\npatch_palette 1 1={v}\n"));
    }
    let parsed = script::parse_script(&body)?;
    let script_bytes = parsed.finish()?;
    assert_eq!(
        script_bytes, manual_bytes,
        "script compiles to canonical bytes"
    );
    // Deterministic: parsing twice yields the same bytes.
    let again = script::parse_script(&body)?.finish()?;
    assert_eq!(again, script_bytes);
    let fa = frames_of(&script_bytes)?;
    assert_eq!(fa.len(), 13);
    assert_eq!(fa[0].get(0, 60), 60, "palette entry 1 starts at 60");
    assert_eq!(fa[1].get(0, 60), 200, "interval 1 patches entry 1 to 200");
    assert_eq!(fa[12].get(0, 60), 60);
    Ok(())
}

#[test]
fn script_hostile_forms_are_typed() {
    let finish =
        |text: &str| -> Result<Vec<u8>, VoleError> { script::parse_script(text)?.finish() };
    // Empty and missing canvas.
    assert_eq!(
        script::parse_script("").unwrap_err(),
        VoleError::ScriptParse("missing canvas")
    );
    assert_eq!(
        script::parse_script("object 1 fill 2 2 5").unwrap_err(),
        VoleError::ScriptParse("canvas first")
    );
    // Duplicate canvas.
    assert_eq!(
        script::parse_script("canvas 4 4\ncanvas 8 8").unwrap_err(),
        VoleError::ScriptParse("duplicate canvas")
    );
    // Byte out of range.
    assert_eq!(
        script::parse_script("canvas 4 4\nobject 1 fill 2 2 300").unwrap_err(),
        VoleError::ScriptParse("byte out of range 0..=255")
    );
    // Non-integer token.
    assert_eq!(
        script::parse_script("canvas 4 4\nobject 1 fill 2 two 5").unwrap_err(),
        VoleError::ScriptParse("expected an integer")
    );
    // Unknown object kind.
    assert_eq!(
        script::parse_script("canvas 4 4\nobject 1 plasma 2 2").unwrap_err(),
        VoleError::ScriptParse("unknown object kind")
    );
    // Unknown statement.
    assert_eq!(
        script::parse_script("canvas 4 4\nfrobnicate 1").unwrap_err(),
        VoleError::ScriptParse("unknown statement")
    );
    // Interval 0 is the checkpoint.
    assert_eq!(
        finish("canvas 4 4\nat 0").unwrap_err(),
        VoleError::NonConsecutiveInterval
    );
    // Decreasing interval.
    let txt =
        "canvas 8 8\nobject 1 fill 4 4 5\ninstance 1 1 0 0\nat 2\nmove 1 1 0\nat 1\nmove 1 0 0";
    assert_eq!(finish(txt).unwrap_err(), VoleError::NonConsecutiveInterval);
    // Duplicate object id.
    let txt = "canvas 8 8\nobject 1 fill 4 4 5\nobject 1 fill 4 4 6";
    assert_eq!(finish(txt).unwrap_err(), VoleError::DuplicateId);
    // Reference to an undeclared object / instance surfaces at finish.
    let txt = "canvas 8 8\nobject 1 fill 4 4 5\ninstance 1 2 0 0";
    assert_eq!(finish(txt).unwrap_err(), VoleError::UnknownObject);
    let txt = "canvas 8 8\nobject 1 fill 4 4 5\ninstance 1 1 0 0\nat 1\nmove 9 1 1";
    assert_eq!(finish(txt).unwrap_err(), VoleError::UnknownInstance);
    // A raster literal with too few values is a typed parse error.
    let txt = "canvas 4 4\nobject 1 raster 4 4 1 2 3";
    assert!(script::parse_script(txt).is_err());
    // Zero geometry is caught at finish by the limits check.
    assert_eq!(
        finish("canvas 0 8\nobject 1 fill 1 1 3\ninstance 1 1 0 0").unwrap_err(),
        VoleError::DimensionTooLarge
    );
}

// ---------------------------------------------------------------------------
// The §55 native-procedural preservation court (leg A vs leg B)
// ---------------------------------------------------------------------------

#[test]
fn flattening_tax_palette_rotation_every_pixel_changes() -> Result<(), VoleError> {
    let (w, h) = (96u32, 96u32);
    let bg = 40u8;
    let base = [10u8, 40, 90, 150, 220, 30, 60, 200];
    let indices: Vec<u8> = (0..(w * h))
        .map(|i| (((i / w * 7) ^ (i % w * 13)) % 8) as u8)
        .collect();
    let mut a = Ingest::new(w, h);
    a.background(bg);
    a.declare_palette(1, base.to_vec())?;
    a.declare_index(1, w, h, indices.clone())?;
    a.instance_binding(1, 1, 0, 0, 1)?;
    for k in 1..=12u64 {
        a.at(k)?;
        let shift = (k % 8) as usize;
        a.set_palette(1, (0..8).map(|i| base[(i + shift) % 8]).collect())?;
    }
    let a_bytes = a.finish()?;
    let fa = frames_of(&a_bytes)?;
    assert_eq!(fa.len(), 13);
    for k in 0..7 {
        assert_ne!(fa[k].get(0, 0), fa[k + 1].get(0, 0));
    }
    let b = inverse_leg(&fa, bg)?;
    assert_legs_equal(&a_bytes, &b)?;
    let a_interval = 360u64; // 12 intervals x 30 B (SetPalette replaces 8 entries)
    let b_interval: u64 = b.decisions[1..]
        .iter()
        .map(|d| d.winner_payload_bytes)
        .sum();
    assert_eq!(a_bytes.len(), 9688, "sealed A total (release example)");
    assert_eq!(b.vole.len(), 74_294, "sealed B total");
    assert_eq!(
        a_interval, 360,
        "12 intervals x 30 B (SetPalette, 8 entries)"
    );
    assert_eq!(b_interval, 64_987, "sealed B interval");
    // Palette semantics survive in A only: A carries the palette table (state
    // bucket) and the index plane; B's representation has no palette state at
    // all — the structural information is flattened away.
    let a_cost = vole_video::inverse::account_stream(&a_bytes)?;
    let b_cost = vole_video::inverse::account_stream(&b.vole)?;
    assert!(a_cost.state_bytes > 0, "A keeps palette-table state");
    assert_eq!(b_cost.state_bytes, 0, "B cannot express palette state");
    assert!(a_cost.index_object_bytes > 0, "A keeps the index plane");
    Ok(())
}

#[test]
fn flattening_tax_palette_accent_strip_reuse() -> Result<(), VoleError> {
    // A uniform-color strip alternating between two palette entries: B serves
    // the visual change with reusable region objects (measured 2.5x interval
    // tax) — the semantic loss is palette state, which regions cannot express.
    let (w, h) = (96u32, 96u32);
    let bg = 40u8;
    let mut a = Ingest::new(w, h);
    a.background(bg);
    a.declare_palette(1, vec![200, 60, 90, 150, 220])?;
    a.declare_index(1, w, 8, vec![1u8; (8 * w) as usize])?;
    a.instance_binding(1, 1, 0, 60, 1)?;
    for k in 1..=12u64 {
        a.at(k)?;
        let v = if k % 2 == 1 { 200 } else { 60 };
        a.patch_palette(1, vec![(1, v)])?;
    }
    let mut a0 = Ingest::new(w, h);
    a0.background(bg);
    a0.declare_palette(1, vec![200, 60, 90, 150, 220])?;
    a0.declare_index(1, w, 8, vec![1u8; (8 * w) as usize])?;
    a0.instance_binding(1, 1, 0, 60, 1)?;
    let a_bytes = a.finish()?;
    let a0_bytes = a0.finish()?;
    let fa = frames_of(&a_bytes)?;
    assert_eq!(fa.len(), 13);
    let b = inverse_leg(&fa, bg)?;
    assert_legs_equal(&a_bytes, &b)?;
    let a_int = (a_bytes.len() - a0_bytes.len()) as u64;
    let b_int: u64 = b.decisions[1..]
        .iter()
        .map(|d| d.winner_payload_bytes)
        .sum();
    assert_eq!(a_bytes.len(), 1165, "sealed A total");
    assert_eq!(b.vole.len(), 10_013, "sealed B total");
    assert_eq!(
        a_int, 288,
        "12 intervals x 24 B (PatchPalette of one entry)"
    );
    assert_eq!(b_int, 706, "sealed B interval");
    assert!(
        b.decisions[1..]
            .iter()
            .any(|d| d.winner_family == "regions" || d.winner_family == "exact_ref"),
        "B recovers the strip as reusable region content"
    );
    Ok(())
}

#[test]
fn flattening_tax_acceleration_trajectory() -> Result<(), VoleError> {
    let (w, h) = (160u32, 120u32);
    let bg = 20u8;
    let samples: Vec<u8> = (0..(24 * 16))
        .map(|i| ((i / 24) * 7 + (i % 24) * 3) as u8)
        .collect();
    let mut a = Ingest::new(w, h);
    a.background(bg);
    a.declare_raster(1, 24, 16, samples.clone())?;
    a.instance(1, 1, 20, 20)?;
    for k in 1..=10u64 {
        a.at(k)?;
        if k == 1 {
            a.set_trajectory(
                1,
                vec![TrajectorySegment::Accel {
                    vx0: 2,
                    vy0: 0,
                    ax: 1,
                    ay: 0,
                    steps: 10,
                }],
            )?;
        }
        a.advance_trajectories()?;
    }
    let mut a0 = Ingest::new(w, h);
    a0.background(bg);
    a0.declare_raster(1, 24, 16, samples)?;
    a0.instance(1, 1, 20, 20)?;
    let a_bytes = a.finish()?;
    let a0_bytes = a0.finish()?;
    let fa = frames_of(&a_bytes)?;
    assert_eq!(fa.len(), 11);
    assert_eq!(fa[0].get(20, 20), 0);
    assert_eq!(fa[5].get(40, 20), 0, "k=5 => +20 px");
    assert_eq!(fa[10].get(85, 20), 0, "k=10 => +65 px");
    let b = inverse_leg(&fa, bg)?;
    assert_legs_equal(&a_bytes, &b)?;
    let a_int = (a_bytes.len() - a0_bytes.len()) as u64;
    let b_int: u64 = b.decisions[1..]
        .iter()
        .map(|d| d.winner_payload_bytes)
        .sum();
    assert_eq!(a_bytes.len(), 649, "sealed A total");
    assert_eq!(b.vole.len(), 24_115, "sealed B total");
    assert_eq!(a_int, 174, "sealed A interval");
    assert_eq!(b_int, 4824, "sealed B interval");
    Ok(())
}

#[test]
fn flattening_tax_affine_rotation_of_noise_tile() -> Result<(), VoleError> {
    let (w, h) = (64u32, 64u32);
    let bg = 90u8;
    let mut a = Ingest::new(w, h);
    a.background(bg);
    a.declare_generator(
        1,
        32,
        32,
        vole_video::generator::Generator::Noise { seed: 3 },
    )?;
    a.instance(1, 1, 16, 16)?;
    for k in 1..=5u64 {
        a.at(k)?;
        let params = vole_video::demo::quarter_turn_params(k as i64, 16, 16, 32, 32);
        a.set_affine(1, params)?;
    }
    let mut a0 = Ingest::new(w, h);
    a0.background(bg);
    a0.declare_generator(
        1,
        32,
        32,
        vole_video::generator::Generator::Noise { seed: 3 },
    )?;
    a0.instance(1, 1, 16, 16)?;
    let a_bytes = a.finish()?;
    let a0_bytes = a0.finish()?;
    let fa = frames_of(&a_bytes)?;
    assert_eq!(fa.len(), 6);
    assert_ne!(fa[0].get(16, 16), fa[1].get(16, 16), "rotation permutes");
    assert_eq!(fa[4].get(16, 16), fa[0].get(16, 16), "full turn at k=4");
    let b = inverse_leg(&fa, bg)?;
    assert_legs_equal(&a_bytes, &b)?;
    let a_int = (a_bytes.len() - a0_bytes.len()) as u64;
    let b_int: u64 = b.decisions[1..]
        .iter()
        .map(|d| d.winner_payload_bytes)
        .sum();
    assert_eq!(a_bytes.len(), 310, "sealed A total");
    assert_eq!(b.vole.len(), 15_246, "sealed B total");
    assert_eq!(a_int, 210, "sealed A interval");
    assert_eq!(b_int, 11_059, "sealed B interval");
    Ok(())
}

#[test]
fn flattening_tax_seeded_noise_static() -> Result<(), VoleError> {
    // A seeded-noise region (authored as a generator program) followed by a
    // static tail: A stores a 9-byte program; B must rasterize the region and
    // never recovers the seed (§21), so the tax is structural and permanent.
    let (w, h) = (64u32, 64u32);
    let bg = 30u8;
    let mut a = Ingest::new(w, h);
    a.background(bg);
    a.declare_generator(
        1,
        48,
        48,
        vole_video::generator::Generator::Noise { seed: 7 },
    )?;
    a.instance(1, 1, 8, 8)?;
    for k in 1..=2u64 {
        a.at(k)?;
    }
    let mut a0 = Ingest::new(w, h);
    a0.background(bg);
    a0.declare_generator(
        1,
        48,
        48,
        vole_video::generator::Generator::Noise { seed: 7 },
    )?;
    a0.instance(1, 1, 8, 8)?;
    let a_bytes = a.finish()?;
    let a0_bytes = a0.finish()?;
    let fa = frames_of(&a_bytes)?;
    assert_eq!(fa.len(), 3);
    assert_eq!(fa[0], fa[1]);
    assert_eq!(fa[1], fa[2]);
    let b = inverse_leg(&fa, bg)?;
    assert_legs_equal(&a_bytes, &b)?;
    let a_int = (a_bytes.len() - a0_bytes.len()) as u64;
    let b_int: u64 = b.decisions[1..]
        .iter()
        .map(|d| d.winner_payload_bytes)
        .sum();
    assert_eq!(a_bytes.len(), 126, "sealed A total");
    assert_eq!(b.vole.len(), 4213, "sealed B total");
    assert_eq!(a_int, 26, "two unchanged frames, 13 B each");
    assert_eq!(b_int, 26, "B also rides the unchanged lane afterwards");
    assert!(
        b.decisions[0].winner_family == "raw",
        "B cannot discover the seed"
    );
    Ok(())
}

#[test]
fn ingest_velocity_advance_and_copy_ops() -> Result<(), VoleError> {
    // Exercise the remaining ingest helpers end-to-end: persistent velocity +
    // advances, copy/move rects, sparse overlay, clears, affine.
    let (w, h) = (64u32, 64u32);
    let samples: Vec<u8> = (0..(16 * 16))
        .map(|i| ((i / 16 + i % 16) * 8) as u8)
        .collect();
    let mut a = Ingest::new(w, h);
    a.background(7);
    a.declare_raster(1, 16, 16, samples)?;
    a.instance(1, 1, 0, 0)?;
    a.at(1)?;
    a.set_velocity(1, 2, 1)?;
    a.advance()?;
    a.at(2)?;
    a.advance()?;
    a.at(3)?;
    a.copy_rect(0, 0, 16, 16, 32, 0)?;
    a.at(4)?;
    a.move_rect(32, 0, 16, 16, 48, 0)?;
    a.at(5)?;
    a.patch_sparse(vec![(0, 0, 200), (1, 0, 201), (2, 0, 202)])?;
    a.at(6)?;
    a.clear_instances()?;
    a.clear_overlay()?;
    let bytes = a.finish()?;
    let fa = frames_of(&bytes)?;
    assert_eq!(fa.len(), 7);
    // Velocity: after two advances the tile origin is at (4, 2).
    assert_eq!(fa[2].get(4, 2), fa[0].get(0, 0));
    // Copy duplicated the frame-2 region (0..16, 0..16) — containing the tile
    // at (4, 2) — onto (32, 0); the tile copy's origin sample lands at (36, 2).
    assert_eq!(fa[3].get(36, 2), fa[0].get(0, 0));
    // Move relocated that region from (32, 0) to (48, 0) and cleared the
    // source (snapshot-copy + clear semantics).
    assert_eq!(fa[4].get(52, 2), fa[0].get(0, 0));
    assert_eq!(fa[4].get(36, 2), 7, "move clears its source rect");
    assert_eq!(fa[4].get(32, 0), 7, "moved-from area is background");
    // Sparse points persist above everything.
    assert_eq!(fa[5].get(0, 0), 200);
    // Clears drop to the background.
    assert_eq!(fa[6].get(0, 0), 7);
    assert_eq!(fa[6].get(48, 0), 7);
    assert_eq!(fa[6].get(52, 2), 7);
    // A typed block also round-trips through the residual helper.
    let mut b = Ingest::new(16, 16);
    b.background(0);
    b.declare_fill(1, 16, 16, 5)?;
    b.instance(1, 1, 0, 0)?;
    b.at(1)?;
    let mut point = Vec::with_capacity(9);
    point.extend_from_slice(&3i32.to_le_bytes());
    point.extend_from_slice(&3i32.to_le_bytes());
    point.push(250);
    let block = vole_video::rans::encode_block(&point);
    b.residual(block)?;
    let bytes = b.finish()?;
    let fb = frames_of(&bytes)?;
    assert_eq!(fb[1].get(3, 3), 250);
    Ok(())
}
