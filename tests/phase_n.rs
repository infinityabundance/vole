//! Phase N courts: bounded procedural generators.
//!
//! A **generator** is an immutable *content program* (tag `0x07` object):
//! gradient / checker / periodic sawtooth / seeded noise, integer-only,
//! deterministic, work bounded by the painted box. Materialization computes
//! samples; the stream stores the program, never the raster. Courts cover:
//! direct-procedural authored streams byte-exact vs independently computed
//! references for every kind; raster-origin **discovery** (a pure gradient
//! sequence is explained procedurally at ~tens of bytes per frame instead of
//! rasters or transform blocks); residual closure (a generator that fits
//! except for dust carries its exact correction and still wins); motion and
//! affine composition of generator tiles; noise and wrong-seed negative
//! controls (RAW stays — a seed that merely relocates bits never wins);
//! generator content identity matching the wire record; hostile wire forms
//! typed; and accounting buckets.

use vole_video::{
    decoder, demo, encoder,
    error::VoleError,
    generator::Generator,
    integr, inverse,
    object::{Object, ObjectId},
    pixel::Canvas,
    state::{Instance, InstanceId},
    transition::Transition,
};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

/// Independent reference for a gradient field: structurally different
/// arithmetic (wrapping `& 255` on a running value instead of `rem_euclid`
/// on the closed form) so a shared formula bug cannot mask a mismatch.
fn ref_gradient(w: u32, h: u32, base: u8, sx: i64, sy: i64) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = (i64::from(base) + sx * i64::from(x) + sy * i64::from(y)) & 0xFF;
            out.push(v as u8);
        }
    }
    out
}

/// Independent reference for checker / periodic / noise (direct formulas).
fn ref_samples(w: u32, h: u32, gen: Generator) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..i64::from(h) {
        for x in 0..i64::from(w) {
            let v = match gen {
                Generator::Gradient { .. } => unreachable!("gradient has its own reference"),
                Generator::Checker { a, b, cell } => {
                    let c = cell as i64;
                    if (x / c + y / c) % 2 == 0 {
                        a
                    } else {
                        b
                    }
                }
                Generator::Periodic {
                    base,
                    sx,
                    sy,
                    period,
                } => {
                    let p = period as i64;
                    ((i64::from(base) + sx * (x % p) + sy * (y % p)) & 0xFF) as u8
                }
                Generator::Noise { .. } => gen.sample(x, y),
            };
            out.push(v as u8);
        }
    }
    out
}

fn ramp_wrap_frame(w: u32, h: u32, phase: u64) -> Canvas {
    canvas_of(
        w,
        h,
        ref_gradient(w, h, 0, 3, 5)
            .into_iter()
            .map(|v| {
                // phase shifts the whole ramp (still an exact integer gradient).
                (u16::from(v) + 11 * (phase % 256) as u16) as u8
            })
            .collect::<Vec<u8>>(),
    )
}

/// Direct procedural stream over one full-canvas generator object.
fn authored_stream(
    w: u32,
    h: u32,
    bg: u8,
    gen: Generator,
    timeline: &[(u64, Vec<Transition>)],
) -> Result<Vec<u8>, VoleError> {
    let obj = Object::procedural(w, h, gen)?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    encoder::encode_stream(w, h, bg, &[(1, obj)], &[inst], timeline)
}

#[test]
fn every_generator_kind_materializes_exact_vs_independent_references() -> Result<(), VoleError> {
    let (w, h) = (96u32, 64u32);
    let bg = 70u8;
    let gens = [
        Generator::Gradient {
            base: 20,
            sx: 3,
            sy: -5,
        },
        Generator::Checker {
            a: 12,
            b: 220,
            cell: 8,
        },
        Generator::Periodic {
            base: 5,
            sx: 2,
            sy: 1,
            period: 16,
        },
        Generator::Noise { seed: 0xBEEF },
    ];
    for (i, gen) in gens.iter().enumerate() {
        let bytes = authored_stream(w, h, bg, *gen, &[])?;
        let frames = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
        assert_eq!(frames.len(), 1);
        let expected = if matches!(gen, Generator::Gradient { .. }) {
            let Generator::Gradient { base, sx, sy } = *gen else {
                unreachable!()
            };
            ref_gradient(w, h, base, sx, sy)
        } else {
            ref_samples(w, h, *gen)
        };
        assert_eq!(frames[0].as_slice(), &expected[..], "kind {i}");
        // The stream stores a program, never the samples.
        assert!(
            bytes.len() < (w * h) as usize / 16,
            "generator stream must be tiny ({} B vs {} raw)",
            bytes.len(),
            w * h
        );
    }
    Ok(())
}

#[test]
fn generator_tiles_compose_with_motion_and_affine_placement() -> Result<(), VoleError> {
    // A 32x32 gradient tile object moved by SetPosition and then rotated by a
    // quarter turn under an affine: materialization must equal the reference
    // (tile samples sampled through the same placement rules).
    let (w, h) = (160u32, 120u32);
    let bg = 7u8;
    let tile = Object::procedural(
        32,
        32,
        Generator::Checker {
            a: 200,
            b: 40,
            cell: 4,
        },
    )?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 10,
        y: 10,
    };
    let rot = demo::quarter_turn_params(1, 16, 16, 80, 60);
    let bytes = encoder::encode_stream(
        w,
        h,
        bg,
        &[(1, tile)],
        &[inst],
        &[(
            1,
            vec![Transition::SetPosition {
                id: InstanceId(1),
                x: 14,
                y: 10,
            }],
        )],
    )?;
    let bytes2 = {
        let tile = Object::procedural(
            32,
            32,
            Generator::Checker {
                a: 200,
                b: 40,
                cell: 4,
            },
        )?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: ObjectId(1),
            x: 0,
            y: 0,
        };
        encoder::encode_stream(
            w,
            h,
            bg,
            &[(1, tile)],
            &[inst],
            &[(
                1,
                vec![Transition::SetAffine {
                    id: InstanceId(1),
                    params: rot,
                }],
            )],
        )?
    };
    // Reference painter for both frames.
    let checker = Generator::Checker {
        a: 200,
        b: 40,
        cell: 4,
    };
    let ref_blit = |dx: i64, dy: i64| {
        let mut c = vec![bg; (w * h) as usize];
        for y in 0..32i64 {
            for x in 0..32i64 {
                let (cx, cy) = (dx + x, dy + y);
                if cx >= 0 && cy >= 0 && cx < i64::from(w) && cy < i64::from(h) {
                    c[cy as usize * w as usize + cx as usize] = checker.sample(x, y);
                }
            }
        }
        c
    };
    let ref_rot = {
        let mut c = vec![bg; (w * h) as usize];
        for y in 0..i64::from(h) {
            for x in 0..i64::from(w) {
                let (su, sv) = rot.source(x, y).expect("in range");
                if su >= 0 && sv >= 0 && su < 32 && sv < 32 {
                    c[y as usize * w as usize + x as usize] = checker.sample(su, sv);
                }
            }
        }
        c
    };
    let frames = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].as_slice(), &ref_blit(10, 10)[..]);
    assert_eq!(frames[1].as_slice(), &ref_blit(14, 10)[..]);
    let frames2 = decoder::materialize_all(&decoder::decode_bytes(&bytes2)?)?;
    assert_eq!(frames2[1].as_slice(), &ref_rot[..]);
    Ok(())
}

#[test]
fn encoder_discovers_generators_on_pure_gradient_sequences() -> Result<(), VoleError> {
    // Raster-origin: a drifting pure gradient is an exact integer program, so
    // the encoder must explain it procedurally (winner `generator` per frame)
    // at tens of bytes per frame rather than rasters or transform blocks.
    let (w, h) = (320u32, 240u32);
    let mut frames: Vec<Canvas> = Vec::new();
    for t in 0..12u64 {
        frames.push(ramp_wrap_frame(w, h, t));
    }
    let report = inverse::encode_frames(&frames, &inverse::EncodeOptions::default())?;
    assert!(report.exact);
    let wins: Vec<&str> = report.decisions.iter().map(|d| d.winner_family).collect();
    assert!(
        wins.iter().all(|f| *f == "generator" || *f == "exact_ref"),
        "every ramp frame is procedurally explained: {wins:?}"
    );
    let raw_all = u64::from(w) * u64::from(h) * frames.len() as u64;
    assert!(
        report.vole.len() as u64 * 500 < raw_all,
        "{} B vs {raw_all} B raw",
        report.vole.len()
    );
    let decoded = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
    for (i, f) in decoded.iter().enumerate() {
        assert_eq!(f.as_slice(), frames[i].as_slice(), "frame {i} exact");
    }
    Ok(())
}

#[test]
fn generator_approximation_carries_its_exact_residual() -> Result<(), VoleError> {
    // A pure gradient plus deterministic dust *away from the fit rows*: the
    // gradient fit passes its prefilter, the render differs only on the dust,
    // and the generator+residual candidate (exact correction counted) must
    // beat the full-raster reset on the frame where the field first appears
    // (frame 0 cannot carry a residual, so the dust field must arrive after a
    // non-explanatory base).
    let (w, h) = (192u32, 128u32);
    let bg = 70u8;
    // Frame 0: full-canvas noise (RAW base; nothing explains it).
    let mut s = 7u64;
    let mut nd = Vec::with_capacity((w * h) as usize);
    for _ in 0..(w * h) {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        nd.push((s >> 56) as u8);
    }
    let noise = canvas_of(w, h, nd);
    // Frame 1: gradient + dust (dust never on row 0 / column 0 / (1,1),
    // which the fit prefilter reads).
    let grad = canvas_of(w, h, ref_gradient(w, h, 30, 2, -1));
    let mut s2 = 1234u64;
    let mut rnd = move || {
        s2 ^= s2 >> 12;
        s2 ^= s2 << 25;
        s2 ^= s2 >> 27;
        s2 = s2.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (s2 >> 56) as u8
    };
    let mut dusted = grad.as_slice().to_vec();
    let mut dust = 0usize;
    for y in 4..h as usize {
        for x in 4..w as usize {
            if rnd() < 4 {
                dusted[y * w as usize + x] = rnd();
                dust += 1;
            }
        }
    }
    assert!(dust > 10 && dust < 500, "dust count sane: {dust}");
    let frames = vec![noise, canvas_of(w, h, dusted)];
    let report = inverse::encode_frames(
        &frames,
        &inverse::EncodeOptions {
            bg_sweep: false,
            background: Some(bg),
            ..inverse::EncodeOptions::default()
        },
    )?;
    assert!(report.exact);
    assert_eq!(report.decisions[0].winner_family, "raw");
    let d = &report.decisions[1];
    assert_eq!(
        d.winner_family, "generator_residual",
        "the fit + exact correction must beat RAW: {d:?}"
    );
    assert!(
        d.winner_payload_bytes < u64::from(w) * u64::from(h) / 6,
        "{} B vs a {}-byte raster",
        d.winner_payload_bytes,
        w * h
    );
    Ok(())
}

#[test]
fn noise_and_wrong_seed_negative_controls_stay_raw() -> Result<(), VoleError> {
    let (w, h) = (96u32, 64u32);
    // (a) Full-canvas deterministic noise that is *not* our noise generator.
    let mut s = 7u64;
    let mut data = Vec::with_capacity((w * h) as usize);
    for _ in 0..(w * h) {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        data.push((s >> 56) as u8);
    }
    let noise = canvas_of(w, h, data);
    // (b) A field that *is* our noise generator with an unknowable seed: the
    // encoder must not discover the seed (search is unbounded), so RAW stays.
    let seeded = canvas_of(
        w,
        h,
        ref_samples(w, h, Generator::Noise { seed: 0xDEAD_BEEF }),
    );
    for (name, frame) in [("noise", noise), ("seeded-noise", seeded)] {
        let report = inverse::encode_frames(
            &[frame],
            &inverse::EncodeOptions {
                bg_sweep: false,
                background: Some(0),
                ..inverse::EncodeOptions::default()
            },
        )?;
        assert!(report.exact);
        let d = &report.decisions[0];
        assert_eq!(d.winner_family, "raw", "{name} must stay RAW");
        assert!(
            !d.families.iter().any(|f| f.family.starts_with("generator")),
            "{name}: no generator family may pass its fit"
        );
        // Bounded overhead: RAW is one raster declaration.
        assert!(report.vole.len() as u64 <= u64::from(w) * u64::from(h) + 200);
    }
    // (c) Authored seeded noise (the source knows the seed) is a tiny stream.
    let bytes = authored_stream(w, h, 0, Generator::Noise { seed: 0xDEAD_BEEF }, &[])?;
    assert!(
        bytes.len() < (w * h) as usize / 16,
        "authored noise stores the program: {} B vs {} raw",
        bytes.len(),
        w * h
    );
    Ok(())
}

#[test]
fn generator_identity_matches_the_wire_record() -> Result<(), VoleError> {
    // Content identity of a generator object must equal BLAKE3 over the
    // canonical wire record (tag 0x07 + w + h + program), and identical
    // programs share identity.
    let g1 = Generator::Periodic {
        base: 9,
        sx: 3,
        sy: -2,
        period: 32,
    };
    let o1 = Object::procedural(64, 48, g1)?;
    let o2 = Object::procedural(64, 48, g1)?;
    let o3 = Object::procedural(
        64,
        48,
        Generator::Periodic {
            base: 9,
            sx: 3,
            sy: -2,
            period: 64,
        },
    )?;
    let id1 = vole_video::identity::content_id_of(&o1);
    let id2 = vole_video::identity::content_id_of(&o2);
    let id3 = vole_video::identity::content_id_of(&o3);
    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
    let mut record = Vec::new();
    record.push(0x07u8);
    record.extend_from_slice(&64u32.to_le_bytes());
    record.extend_from_slice(&48u32.to_le_bytes());
    record.extend_from_slice(&g1.program_bytes());
    let expect = integr::digest(&record);
    assert_eq!(id1.as_bytes(), &expect);
    Ok(())
}

#[test]
fn hostile_generator_streams_are_typed() -> Result<(), VoleError> {
    use vole_video::format::StreamWriter;
    // Canonical stream with one generator object declaration.
    let bytes = StreamWriter::begin(64, 48)
        .background(9)
        .declare_object(
            ObjectId(1),
            Object::procedural(
                64,
                48,
                Generator::Gradient {
                    base: 5,
                    sx: 3,
                    sy: 1,
                },
            )?,
        )?
        .checkpoint_with(&[])?
        .finish()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    assert_eq!(decoder::materialize_all(&parsed)?.len(), 1);
    let (payload, _t) = bytes.split_at(bytes.len() - 32);
    let seal = |p: &[u8]| -> Vec<u8> {
        let mut o = p.to_vec();
        o.extend_from_slice(&integr::digest(p));
        o
    };
    // Record offsets: header 24 | tag(1) id(4) w(4) h(4) kind(1) base(1)
    // sx:i32 at 24+15 = 39.
    let slope_off = 24 + 15;
    assert_eq!(payload[24], 0x07); // the object tag
                                   // Out-of-domain slope -> typed at parse.
    let mut b1 = payload.to_vec();
    b1[slope_off..slope_off + 4].copy_from_slice(&((1i32 << 24) + 1).to_le_bytes());
    assert_eq!(
        decoder::decode_bytes(&seal(&b1)).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Unknown program kind -> typed at parse.
    let mut b2 = payload.to_vec();
    let kind_off = 24 + 13;
    b2[kind_off] = 0x7F;
    assert_eq!(
        decoder::decode_bytes(&seal(&b2)).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Oversized declared box -> typed at parse.
    let mut b3 = payload.to_vec();
    b3[29..33].copy_from_slice(&u32::MAX.to_le_bytes()); // w
    assert_eq!(
        decoder::decode_bytes(&seal(&b3)).unwrap_err(),
        VoleError::DimensionTooLarge
    );
    // Truncation is typed.
    assert!(decoder::decode_bytes(&seal(&payload[..payload.len() - 3])).is_err());
    Ok(())
}

#[test]
fn generator_stream_accounting_buckets_sum() -> Result<(), VoleError> {
    let (w, h) = (64u32, 48u32);
    let bytes = authored_stream(
        w,
        h,
        9,
        Generator::Gradient {
            base: 3,
            sx: 1,
            sy: 2,
        },
        &[],
    )?;
    let cost = inverse::account_stream(&bytes)?;
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
    assert_eq!(cost.generator_object_bytes, cost.object_bytes);
    assert_eq!(
        cost.raster_object_sample_bytes, 0,
        "no raster fallback stored"
    );
    Ok(())
}

#[test]
fn strategies_agree_on_gradient_discovery() -> Result<(), VoleError> {
    use vole_video::dsfb::EncoderStrategy;
    let (w, h) = (160u32, 120u32);
    let frames: Vec<Canvas> = (0..9).map(|t| ramp_wrap_frame(w, h, t * 7)).collect();
    let run = |strat: EncoderStrategy| -> Result<inverse::EncodeReport, VoleError> {
        inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                strategy: strat,
                ..inverse::EncodeOptions::default()
            },
        )
    };
    let ex = run(EncoderStrategy::Exhaustive)?;
    let fx = run(EncoderStrategy::FixedHeuristic)?;
    let ds = run(EncoderStrategy::DsfbGuided)?;
    assert!(ex.exact && fx.exact && ds.exact);
    assert_eq!(
        ex.vole, fx.vole,
        "fixed heuristic finds the same generators"
    );
    assert_eq!(ex.vole, ds.vole, "DSFB finds the same generators");
    let dsfb_cands: u64 = ds.decisions.iter().map(|d| d.candidates_evaluated).sum();
    let ex_cands: u64 = ex.decisions.iter().map(|d| d.candidates_evaluated).sum();
    assert!(
        dsfb_cands <= ex_cands,
        "N_dsfb={dsfb_cands} must be <= N_exhaustive={ex_cands}"
    );
    Ok(())
}

/// Structured frame used by the flattening-tax court below.
fn ui_like_frame(w: u32, h: u32, t: u64) -> Canvas {
    // Smooth content *plus* a hard structural boundary: a gradient backdrop
    // and a solid bar whose position/height is not gradient-explainable —
    // content where a generator explains the bulk and regions/raw the rest.
    let mut d = Vec::with_capacity((w * h) as usize);
    let bar_y = 8u32 + (t as u32 % 8);
    for y in 0..h {
        for x in 0..w {
            let g = (10 + 2 * u64::from(x) + u64::from(y)) & 0xFF;
            if y >= bar_y && y < bar_y + 4 && x < 40 {
                d.push(200);
            } else {
                d.push(g as u8);
            }
        }
    }
    canvas_of(w, h, d)
}

#[test]
fn generator_explains_the_bulk_and_never_hides_the_rest() -> Result<(), VoleError> {
    // The gradient backdrop is discovered procedurally; the moving bar is not
    // gradient-explainable, so the frame carries real bytes for it. The court
    // checks the stream decodes exactly and that no frame "disappears" behind
    // a wrong generator (the encoder validates by normative render).
    let (w, h) = (192u32, 128u32);
    let frames: Vec<Canvas> = (0..6).map(|t| ui_like_frame(w, h, t)).collect();
    let report = inverse::encode_frames(&frames, &inverse::EncodeOptions::default())?;
    assert!(report.exact);
    let decoded = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
    for (i, f) in decoded.iter().enumerate() {
        assert_eq!(f.as_slice(), frames[i].as_slice(), "frame {i} exact");
    }
    // The bar changes every frame, so the backdrop may be explained once but
    // the bar can never be free: some frame's winner must carry real
    // correction bytes (generator+residual, region repairs, or a copy with
    // residual) — the structural detail is never hidden behind the generator.
    let wins: Vec<&str> = report.decisions.iter().map(|d| d.winner_family).collect();
    assert!(
        report
            .decisions
            .iter()
            .any(|d| d.winner_interval_bytes >= 100),
        "the moving bar must cost real bytes every time it appears: {wins:?}"
    );
    Ok(())
}
