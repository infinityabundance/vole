//! Phase T courts: the archive profile (master brief §67 + the Phase-T block
//! of §64) — strong integrity (whole-stream + per-record + checkpoint +
//! object + per-frame hashes), self-description, corruption localization,
//! bounded hostile-manifest handling, long-term universe pinning, and
//! representation-equivalence goldens across `vole optimize` rewrites.

use std::path::Path;

use vole_video::{
    archive::{self, encode, ArchiveManifest, RecordKind, VerifyStatus},
    decoder, encoder, identity,
    ingest::Ingest,
    integr,
    object::Object,
    optimize,
    pixel::Canvas,
    rans::KIND_RAW,
    state::Instance,
    transition::Transition,
    view::View,
    VoleError,
};

fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

fn raw_block(pts: &[(i32, i32, u8)]) -> Vec<u8> {
    let mut body = Vec::with_capacity(9 * pts.len());
    for (x, y, v) in pts {
        body.extend_from_slice(&x.to_le_bytes());
        body.extend_from_slice(&y.to_le_bytes());
        body.push(*v);
    }
    let mut block = vec![KIND_RAW];
    block.extend_from_slice(&(body.len() as u64).to_le_bytes());
    block.extend_from_slice(&body);
    block
}

/// The phase-A golden stream (frozen v1 semantics; must archive forever).
fn golden_vole() -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("proof/court-moving-rect.vole");
    std::fs::read(p).expect("golden stream present")
}

/// A mid-sized authored stream: moving sprite + palette-index content with
/// bindings (checkpoint tag 0x08), palette patches, sparse overlay, RAW
/// residual, copy rect.
fn court_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (96u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(17);
    a.declare_palette(1, vec![10, 60, 120, 200, 250])?;
    let idx: Vec<u8> = (0..(48 * 48))
        .map(|i| (((i / 48) * 3 + (i % 48) * 5) % 5) as u8)
        .collect();
    a.declare_index(1, 48, 48, idx)?;
    a.instance_binding(1, 1, 10, 10, 1)?;
    a.declare_raster(2, 12, 12, vec![7u8; 144])?;
    a.instance(2, 2, 40, 40)?;
    for t in 1..=10u64 {
        a.at(t)?;
        a.set_position(2, 40 + 2 * t as i64, 40)?;
        if t % 3 == 0 {
            let changes: Vec<(u8, u8)> = (0..5).map(|i| (i, (10 + t * 20) as u8)).collect();
            a.patch_palette(1, changes)?;
        }
        if t == 5 {
            a.patch_sparse(vec![(70, 70, 99)])?;
            a.push(Transition::CopyRect {
                src_x: 20,
                src_y: 0,
                width: 16,
                height: 16,
                dst_x: 60,
                dst_y: 60,
            })?;
        }
        if t == 8 {
            a.push(Transition::Residual {
                block: raw_block(&[(2, 2, 200), (80, 80, 11)]),
            })?;
        }
    }
    a.finish()
}

/// A kitchen-sink stream exercising every object kind and transition family
/// (velocity + advance, trajectory + advance, affine, palette set/bind,
/// generator object, clears, sparse, copy/move, residual).
fn kitchen_sink_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (128u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(3);
    a.declare_palette(1, vec![0, 40, 90, 160, 250])?;
    a.declare_fill(1, 8, 8, 120)?;
    a.declare_raster(2, 6, 6, vec![9u8; 36])?;
    a.declare_generator(
        3,
        16,
        16,
        vole_video::generator::Generator::Gradient {
            base: 1,
            sx: 5,
            sy: 2,
        },
    )?;
    let idx: Vec<u8> = (0..(16 * 8)).map(|i| (i % 5) as u8).collect();
    a.declare_index(4, 16, 8, idx)?;
    a.instance(1, 1, 10, 10)?;
    a.instance(2, 2, 60, 10)?;
    a.instance_binding(3, 4, 60, 60, 1)?;
    for t in 1..=8u64 {
        a.at(t)?;
        match t {
            1 => {
                a.set_velocity(1, 2, 1)?;
                a.set_trajectory(
                    2,
                    vec![vole_video::trajectory::TrajectorySegment::Linear {
                        vx: 3,
                        vy: 0,
                        steps: 6,
                    }],
                )?;
            }
            2 => {
                a.advance()?;
                a.advance_trajectories()?;
                a.set_affine(3, vole_video::affine::AffineParams::IDENTITY)?;
            }
            3 => {
                a.advance()?;
                a.advance_trajectories()?;
                a.set_palette(1, vec![250, 40, 90, 160, 0])?;
            }
            4 => {
                a.advance()?;
                a.advance_trajectories()?;
                a.patch_palette(1, vec![(1, 77), (3, 210)])?;
                a.patch_sparse(vec![(5, 90, 250)])?;
            }
            5 => {
                a.push(Transition::MoveRect {
                    src_x: 10,
                    src_y: 10,
                    width: 8,
                    height: 8,
                    dst_x: 40,
                    dst_y: 80,
                })?;
                a.create_instance(4, 3, 100, 30)?;
            }
            6 => {
                a.push(Transition::Residual {
                    block: raw_block(&[(1, 1, 5), (30, 30, 200)]),
                })?;
            }
            7 => {
                a.clear_overlay()?;
                a.push(Transition::CopyRect {
                    src_x: 0,
                    src_y: 60,
                    width: 20,
                    height: 12,
                    dst_x: 100,
                    dst_y: 70,
                })?;
            }
            _ => {
                a.clear_instances()?;
                a.create_instance(5, 1, 0, 0)?;
            }
        }
    }
    a.finish()
}

fn assert_records_tile_stream(records: &[archive::RecordRef], bytes: &[u8]) {
    assert_eq!(records[0].kind, RecordKind::Header);
    assert_eq!(records[0].offset, 0);
    assert_eq!(records[0].length, 24);
    for pair in records.windows(2) {
        assert_eq!(
            pair[0].offset + pair[0].length,
            pair[1].offset,
            "records tile the stream exactly"
        );
    }
    assert_eq!(records.last().unwrap().kind, RecordKind::Integrity);
    assert_eq!(
        records.last().unwrap().offset + records.last().unwrap().length,
        bytes.len() as u64
    );
    assert_eq!(records.last().unwrap().length, 32);
}

// ---------------------------------------------------------------------------
// Record scan courts
// ---------------------------------------------------------------------------

#[test]
fn record_scan_tiles_every_stream_shape() -> Result<(), VoleError> {
    let streams = vec![
        ("golden-a", golden_vole()),
        ("court", court_stream()?),
        ("kitchen-sink", kitchen_sink_stream()?),
    ];
    for (name, bytes) in &streams {
        let parsed = decoder::decode_bytes(bytes)?;
        let records = archive::scan_stream(bytes)?;
        assert_records_tile_stream(&records, bytes);
        // Order: header, at least one declaration, checkpoint, intervals, integrity.
        assert_eq!(records[1].kind, RecordKind::Object);
        let cp = records.iter().find(|r| r.kind == RecordKind::Checkpoint);
        assert!(cp.is_some(), "{name}: checkpoint present");
        // Interval records agree with the parsed timeline, in order.
        let parsed_t: Vec<u64> = parsed.intervals().iter().map(|(iv, _)| iv.0).collect();
        let rec_t: Vec<u64> = records
            .iter()
            .filter(|r| r.kind == RecordKind::Interval)
            .map(|r| r.t.expect("interval t"))
            .collect();
        assert_eq!(rec_t, parsed_t, "{name}: interval times match parse");
        // Record count: header + decls + checkpoint + intervals + integrity.
        let decls =
            parsed.clone_initial().objects().count() + parsed.clone_initial().palette_count();
        assert_eq!(
            records.len(),
            1 + decls + 1 + parsed.intervals().len() + 1,
            "{name}: record count"
        );
        // Each digest is stable (deterministic).
        let again = archive::scan_stream(bytes)?;
        assert_eq!(records, again);
    }
    Ok(())
}

#[test]
fn store_backed_streams_scan_but_do_not_archive() -> Result<(), VoleError> {
    // Phase P: an external-object stream (feature bit 0x1). Its top-level
    // record structure is indexable (record windows are pure bytes), but
    // archiving (which needs the objects for frame hashes) refuses it.
    let obj = Object::fill(4, 4, 9)?;
    let cid = identity::content_id_of(&obj);
    let inst = Instance {
        id: vole_video::state::InstanceId(1),
        object_id: vole_video::object::ObjectId(1),
        x: 0,
        y: 0,
    };
    let bytes = encoder::encode_stream_external(
        16,
        16,
        0,
        &[(1, cid)],
        &[inst],
        &[(1, vec![] as Vec<Transition>)],
    )?;
    let records = archive::scan_stream(&bytes)?;
    assert_records_tile_stream(&records, &bytes);
    assert_eq!(records[1].kind, RecordKind::Object);
    assert_eq!(records[1].length, 1 + 4 + 32);
    assert_eq!(
        ArchiveManifest::build(&bytes).unwrap_err(),
        VoleError::ApiConstraint("archiving requires a standalone stream (no external objects)")
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Manifest wire courts
// ---------------------------------------------------------------------------

#[test]
fn manifest_roundtrip_is_canonical_and_self_authenticating() -> Result<(), VoleError> {
    let bytes = court_stream()?;
    let m = ArchiveManifest::build(&bytes)?;
    assert_eq!(m.stream.frame_count, 11);
    assert_eq!(m.frame_hashes.len(), 11);
    assert!(!m.objects.is_empty());
    assert_eq!(
        m.records.len(),
        m.records.iter().map(|r| r.index).max().unwrap() as usize + 1
    );

    let wire = encode(&m)?;
    // Self-authenticating: any raw flip fails decode before semantics.
    for pos in [0usize, 7, wire.len() / 2, wire.len() - 1] {
        let mut bad = wire.clone();
        bad[pos] ^= 0x01;
        assert_eq!(
            archive::decode(&bad).unwrap_err(),
            VoleError::IntegrityMismatch,
            "tampered manifest at {pos}"
        );
    }
    // Truncation is typed.
    assert_eq!(
        archive::decode(&wire[..wire.len() - 40]).unwrap_err(),
        VoleError::IntegrityMismatch
    );
    assert_eq!(
        archive::decode(&wire[..10]).unwrap_err(),
        VoleError::Truncated
    );

    // Canonical roundtrip: decode(encode(m)) == m; encode is a fixpoint.
    let back = archive::decode(&wire)?;
    assert_eq!(back, m);
    assert_eq!(encode(&back)?, wire);

    // Version pinning: a v2 manifest (recomputed digest) fails closed.
    let mut v2 = wire.clone();
    v2[8..12].copy_from_slice(&2u32.to_le_bytes());
    let n = v2.len();
    let d = integr::digest(&v2[..n - 32]);
    v2[n - 32..].copy_from_slice(&d);
    assert_eq!(
        archive::decode(&v2).unwrap_err(),
        VoleError::UnsupportedFeature
    );

    // Bad magic (recomputed digest) is typed.
    let mut bm = wire.clone();
    bm[0] ^= 0xFF;
    let n = bm.len();
    let d = integr::digest(&bm[..n - 32]);
    bm[n - 32..].copy_from_slice(&d);
    assert_eq!(archive::decode(&bm).unwrap_err(), VoleError::BadMagic);
    Ok(())
}

#[test]
fn hostile_manifest_counts_are_typed_and_bounded() -> Result<(), VoleError> {
    let bytes = court_stream()?;
    let m = ArchiveManifest::build(&bytes)?;
    let mut wire = encode(&m)?;
    // Inflate the record count far beyond what the payload can hold (the
    // self-authenticating trailer is recomputed so the count check runs).
    let rc_pos = {
        // After magic(8) + version(4) + self-description fields + checkpoint
        // digest: 8 + 4 + (2+4+1+4+4+4+1+8+8) + 32 + 32.
        8 + 4 + 2 + 4 + 1 + 4 + 4 + 4 + 1 + 8 + 8 + 32 + 32
    };
    wire[rc_pos..rc_pos + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let n = wire.len();
    let d = integr::digest(&wire[..n - 32]);
    wire[n - 32..].copy_from_slice(&d);
    assert_eq!(
        archive::decode(&wire).unwrap_err(),
        VoleError::DimensionTooLarge
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Build / verify courts
// ---------------------------------------------------------------------------

#[test]
fn pristine_stream_verifies_complete_structural_and_deep() -> Result<(), VoleError> {
    let bytes = court_stream()?;
    let m = ArchiveManifest::build(&bytes)?;
    let report = archive::verify(&bytes, &m, true)?;
    assert_eq!(report.status, VerifyStatus::Complete);
    assert!(report.decode_ok);
    assert!(report.self_description_ok);
    assert!(report.structural_ok);
    assert!(report.record_count_ok);
    assert!(report.objects_ok);
    assert!(report.stream_digest_ok);
    assert_eq!(report.records_checked, m.records.len() as u64);
    assert_eq!(report.first_bad_record, None);
    assert_eq!(report.frames_checked, m.stream.frame_count);
    assert_eq!(report.first_frame_divergence, None);

    // Golden v1 file (Phase A, frozen semantics) still builds and verifies.
    let golden = golden_vole();
    let mg = ArchiveManifest::build(&golden)?;
    assert_eq!(mg.stream.frame_count, 101);
    assert_eq!(mg.stream.universe_id, 1);
    assert_eq!(mg.stream.format_version, 1);
    let rg = archive::verify(&golden, &mg, true)?;
    assert_eq!(rg.status, VerifyStatus::Complete);
    assert_eq!(rg.frames_checked, 101);
    Ok(())
}

#[test]
fn self_description_fields_are_complete_and_pinned() -> Result<(), VoleError> {
    let bytes = kitchen_sink_stream()?;
    let m = ArchiveManifest::build(&bytes)?;
    let sd = &m.stream;
    assert_eq!(sd.width, 128);
    assert_eq!(sd.height, 96);
    assert_eq!(sd.pixel_code, archive::PIXEL_GRAY8);
    assert_eq!(sd.format_version, 1);
    assert_eq!(sd.universe_id, 1);
    assert_eq!(sd.limit_profile, 1);
    assert_eq!(sd.feature_bits, 0);
    assert_eq!(sd.stream_len, bytes.len() as u64);
    assert_eq!(sd.stream_digest, integr::digest(&bytes));
    assert_eq!(m.universe_id(), 1);
    assert_eq!(m.format_version(), 1);
    // Object table: the declared objects' content identities.
    let parsed = decoder::decode_bytes(&bytes)?;
    let state = parsed.clone_initial();
    assert_eq!(m.objects.len(), state.object_count());
    for o in &m.objects {
        let obj = state
            .object(vole_video::object::ObjectId(o.id))
            .expect("declared");
        assert_eq!(o.content_id, *identity::content_id_of(obj).as_bytes());
    }
    Ok(())
}

#[test]
fn frame_hashes_are_the_golden_equivalence_across_representations() -> Result<(), VoleError> {
    // A per-frame SetPosition stream and its `vole optimize` rewrite decode to
    // identical rasters. The manifest's frame hashes (built from the original)
    // match the optimized stream's reconstruction hashes frame-for-frame —
    // the §67 "expected reconstruction hashes" oracle across representations.
    let (w, h) = (1920u32, 1080u32);
    let mut a = Ingest::new(w, h);
    a.background(40);
    a.declare_fill(1, 200, 100, 180)?;
    a.instance(1, 1, 100, 500)?;
    for t in 1..=60u64 {
        a.at(t)?;
        a.set_position(1, 100 + 4 * t as i64, 500)?;
    }
    let bytes = a.finish()?;
    let m = ArchiveManifest::build(&bytes)?;
    let opt = optimize::optimize_stream(&bytes)?;
    assert!(opt.stream.len() < bytes.len(), "optimize shrank the stream");
    let parsed_opt = decoder::decode_bytes(&opt.stream)?;
    let opt_hashes = archive::compute_frame_hashes(&parsed_opt)?;
    assert_eq!(
        opt_hashes, m.frame_hashes,
        "identical rasters, identical hashes"
    );
    assert_eq!(opt_hashes.len(), 61);
    // The optimize rewrite itself is not byte-identical (representation
    // changed), which verify reports as a structural mismatch — the frame
    // hashes are the *reconstruction* oracle.
    let rep = archive::verify(&opt.stream, &m, false)?;
    assert_eq!(rep.status, VerifyStatus::StructuralMismatch);
    assert!(!rep.structural_ok);
    Ok(())
}

#[test]
fn corruption_localizes_to_exact_records() -> Result<(), VoleError> {
    let bytes = court_stream()?;
    let m = ArchiveManifest::build(&bytes)?;

    // Helper: flip one byte strictly inside record `idx`'s window and report
    // the first bad record the verify sees.
    let flip = |idx: usize, at: usize| -> Result<archive::VerifyReport, VoleError> {
        let r = m.records[idx];
        let mut bad = bytes.clone();
        let p = (r.offset as usize + at).min(bytes.len() - 1);
        bad[p] ^= 0x01;
        archive::verify(&bad, &m, false)
    };

    // Header content (width field): the self-description disagrees first.
    let r = archive::verify(&bytes, &m, false)?;
    assert_eq!(r.status, VerifyStatus::Complete);
    let hdr = archive::verify(
        &{
            let mut bad = bytes.clone();
            bad[16] ^= 0x01;
            bad
        },
        &m,
        false,
    )?;
    assert_eq!(hdr.status, VerifyStatus::SelfDescriptionMismatch);
    assert_eq!(hdr.mismatch_field, Some(archive::SelfField::Width));

    // Object declaration payload byte → the object record (flip the record's
    // last payload byte — a content sample — keeping record windows intact).
    let obj_idx = m
        .records
        .iter()
        .position(|r| r.kind == RecordKind::Object)
        .unwrap();
    let rep = flip(obj_idx, m.records[obj_idx].length as usize - 1)?;
    assert_eq!(rep.status, VerifyStatus::StructuralMismatch);
    assert_eq!(rep.first_bad_record, Some(obj_idx as u32));
    assert!(
        !rep.decode_ok,
        "trailer mismatch means decode fails cleanly"
    );
    assert!(
        !rep.decode_ok,
        "trailer mismatch means decode fails cleanly"
    );

    // Interval payload byte (its `t`) → that interval record.
    let iv_idx = m
        .records
        .iter()
        .position(|r| r.kind == RecordKind::Interval && r.t == Some(7))
        .unwrap();
    let rep = flip(iv_idx, 1)?; // the `t` low byte
    assert_eq!(rep.status, VerifyStatus::StructuralMismatch);
    assert_eq!(rep.first_bad_record, Some(iv_idx as u32));
    let bad_ref = rep.first_bad_record_ref(&m).expect("record");
    assert_eq!(bad_ref.kind, RecordKind::Interval);
    assert_eq!(bad_ref.t, Some(7));
    assert_eq!(bad_ref.offset, m.records[iv_idx].offset);

    // Integrity trailer byte → the final record.
    let last = m.records.len() - 1;
    let rep = flip(last, 3)?;
    assert_eq!(rep.status, VerifyStatus::StructuralMismatch);
    assert_eq!(rep.first_bad_record, Some(last as u32));
    assert!(!rep.decode_ok);
    Ok(())
}

#[test]
fn cross_stream_manifest_mismatch_is_localized() -> Result<(), VoleError> {
    let a_bytes = court_stream()?;
    let m = ArchiveManifest::build(&a_bytes)?;
    // A stream with the same shape but different content (sprite raster value
    // differs): verification reports the first disagreeing record.
    let mut a2 = Ingest::new(96, 96);
    a2.background(17);
    a2.declare_palette(1, vec![10, 60, 120, 200, 250])?;
    let idx: Vec<u8> = (0..(48 * 48))
        .map(|i| (((i / 48) * 3 + (i % 48) * 5) % 5) as u8)
        .collect();
    a2.declare_index(1, 48, 48, idx)?;
    a2.instance_binding(1, 1, 10, 10, 1)?;
    a2.declare_raster(2, 12, 12, vec![8u8; 144])?; // differs from A (7)
    a2.instance(2, 2, 40, 40)?;
    a2.at(1)?;
    a2.set_position(2, 42, 40)?;
    let b_bytes = a2.finish()?;
    let rep = archive::verify(&b_bytes, &m, false)?;
    assert_eq!(rep.status, VerifyStatus::StructuralMismatch);
    assert!(!rep.structural_ok);
    let first = rep.first_bad_record.expect("a record disagrees");
    let rec = m.records[first as usize];
    assert_eq!(
        rec.kind,
        RecordKind::Object,
        "the sprite declaration differs"
    );
    // decode of B is fine; the mismatch is representation/content, reported.
    assert!(rep.decode_ok);
    Ok(())
}

#[test]
fn tampered_manifest_pinned_universe_is_reported() -> Result<(), VoleError> {
    let bytes = court_stream()?;
    let m = ArchiveManifest::build(&bytes)?;
    let mut wire = encode(&m)?;
    // Rewrite the pinned universe id (offset 8+4+2) and re-seal the manifest.
    let univ_pos = 8 + 4 + 2;
    wire[univ_pos..univ_pos + 4].copy_from_slice(&7u32.to_le_bytes());
    let n = wire.len();
    let d = integr::digest(&wire[..n - 32]);
    wire[n - 32..].copy_from_slice(&d);
    let forged = archive::decode(&wire)?;
    assert_eq!(forged.stream.universe_id, 7);
    // The pristine v1 stream disagrees on the pinned universe binding.
    let rep = archive::verify(&bytes, &forged, false)?;
    assert_eq!(rep.status, VerifyStatus::SelfDescriptionMismatch);
    assert_eq!(rep.mismatch_field, Some(archive::SelfField::Universe));
    // And the un-tampered manifest verifies Complete against the same bytes.
    let rep2 = archive::verify(&bytes, &m, true)?;
    assert_eq!(rep2.status, VerifyStatus::Complete);
    Ok(())
}

#[test]
fn kitchen_sink_builds_and_verifies() -> Result<(), VoleError> {
    let bytes = kitchen_sink_stream()?;
    let frames = frames_of(&bytes)?;
    let m = ArchiveManifest::build(&bytes)?;
    assert_eq!(m.stream.frame_count, frames.len() as u64);
    let report = archive::verify(&bytes, &m, true)?;
    assert_eq!(report.status, VerifyStatus::Complete);
    assert_eq!(report.frames_checked, frames.len() as u64);
    // Every scan record digest matches its manifest twin (parity between the
    // two walks of the same stream).
    let scanned = archive::scan_stream(&bytes)?;
    for (a, b) in scanned.iter().zip(m.records.iter()) {
        assert_eq!(a, b);
    }
    Ok(())
}

#[test]
fn archive_of_a_partial_view_stream_is_consistent() -> Result<(), VoleError> {
    // Streams with canvas ops (partial-decode content) archive identically.
    let (w, h) = (160u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(5);
    a.declare_raster(1, 8, 8, vec![200u8; 64])?;
    a.instance(1, 1, 30, 30)?;
    for t in 1..=5u64 {
        a.at(t)?;
        a.set_position(1, 30 + t as i64, 30)?;
        a.push(Transition::CopyRect {
            src_x: 20 + t as i64,
            src_y: 0,
            width: 8,
            height: 8,
            dst_x: 21 + t as i64,
            dst_y: 0,
        })?;
    }
    let bytes = a.finish()?;
    let m = ArchiveManifest::build(&bytes)?;
    let report = archive::verify(&bytes, &m, true)?;
    assert_eq!(report.status, VerifyStatus::Complete);
    // And the deep verification agrees with a full materialization.
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(m.frame_hashes[i], archive::hash_canvas(f));
    }
    Ok(())
}

/// Cross-check archive hashing against the canonical decode path's own
/// frames on the golden file (hashes stored by the manifest equal direct
/// `materialize_all` hashes).
#[test]
fn golden_frame_hashes_equal_canonical_decode() -> Result<(), VoleError> {
    let bytes = golden_vole();
    let m = ArchiveManifest::build(&bytes)?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    assert_eq!(m.frame_hashes.len(), frames.len());
    for (h, f) in m.frame_hashes.iter().zip(frames.iter()) {
        assert_eq!(*h, archive::hash_canvas(f));
    }
    Ok(())
}

/// Views and archives are orthogonal: partial views keep working on archived
/// (structurally verified) streams, and archiving never changes semantics.
#[test]
fn archive_verification_does_not_disturb_view_materialization() -> Result<(), VoleError> {
    let bytes = court_stream()?;
    let m = ArchiveManifest::build(&bytes)?;
    assert_eq!(
        archive::verify(&bytes, &m, true)?.status,
        VerifyStatus::Complete
    );
    let parsed = decoder::decode_bytes(&bytes)?;
    let full = decoder::materialize_all(&parsed)?;
    let pv = vole_video::partial::materialize_view(
        &parsed,
        7,
        View::Rect {
            x: 10,
            y: 10,
            width: 40,
            height: 40,
        },
    )?;
    let (w, h) = (pv.canvas.width(), pv.canvas.height());
    let mut expect = Vec::new();
    for y in 0..h {
        for x in 0..w {
            expect.push(full[7].get(x + 10, y + 10));
        }
    }
    assert_eq!(pv.canvas.as_slice(), expect.as_slice());
    Ok(())
}
