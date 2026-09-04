//! High-level encode entry point (native procedural ingest → `.vole` bytes).
//!
//! Raster frames never enter this path: an encoder records immutable objects,
//! a checkpoint, and interval transitions. Rows are later *materialized*, not
//! stored. This function fully validates the timeline with the same normative
//! semantics the parser and decoder use (duplicate ids, unknown references,
//! invalid ordering all rejected) before serializing a canonical file.

use crate::{
    error::VoleError,
    format::{StreamWriter, FEAT_QUANTIZED_CONTENT},
    integr,
    object::{Object, ObjectId},
    state::{Instance, InstanceId, State},
    time::Interval,
    transition::Transition,
};

/// Mark a standalone stream as carrying **quantized-content declarations**
/// (Phase U perceptual profile): set feature bit `0x2` (a declaration that the
/// stream's frames are the encoder's *chosen reconstruction* `F̂` — the
/// deterministic integer quantization of a source — not the original capture)
/// and re-seal the integrity trailer. The bit never changes reconstruction — a
/// conforming decoder reproduces exactly the same `F̂` frames with or without
/// it; it declares the frames' provenance. Exact (lossless) streams never call
/// this. Idempotent.
pub fn mark_quantized_content(bytes: &[u8]) -> Result<Vec<u8>, VoleError> {
    if bytes.len() < 24 + integr::DIGEST_LEN {
        return Err(VoleError::Truncated);
    }
    // Canonical header: feature_bits live at bytes 12..16.
    let mut fb = [0u8; 4];
    fb.copy_from_slice(&bytes[12..16]);
    let mut features = u32::from_le_bytes(fb);
    if features & crate::format::FEAT_EXTERNAL_OBJECTS != 0 {
        // External-object streams are not standalone; their payloads live in a
        // store, so a quantized-content declaration is meaningless here.
        return Err(VoleError::ApiConstraint(
            "cannot mark a store-backed stream as quantized content",
        ));
    }
    features |= FEAT_QUANTIZED_CONTENT;
    let mut out = bytes.to_vec();
    out[12..16].copy_from_slice(&features.to_le_bytes());
    let n = out.len();
    let d = integr::digest(&out[..n - integr::DIGEST_LEN]);
    out[n - integr::DIGEST_LEN..].copy_from_slice(&d);
    Ok(out)
}

/// Encode a complete procedural stream from explicit descriptors.
///
/// # Arguments
/// * `width`, `height` — canonical Gray8 canvas geometry.
/// * `background` — sample painted below every instance at materialization.
/// * `objects` — immutable objects declared before the checkpoint (`(id, obj)`).
/// * `checkpoint` — the live instances anchoring interval 0 (paint order).
/// * `timeline` — interval groups, each targeting an absolute `t` strictly
///   greater than every preceding group.
pub fn encode_stream(
    width: u32,
    height: u32,
    background: u8,
    objects: &[(u32, Object)],
    checkpoint: &[Instance],
    timeline: &[(u64, Vec<Transition>)],
) -> Result<Vec<u8>, VoleError> {
    // 1. Full normative validation over a scratch state.
    validate_timeline(width, height, background, objects, checkpoint, timeline)?;

    // 2. Serialize the validated descriptors canonically.
    let mut wr = StreamWriter::begin(width, height).background(background);
    for (id, obj) in objects {
        wr = wr.declare_object(ObjectId(*id), obj.clone())?;
    }
    wr = wr.checkpoint_with(checkpoint)?;
    for (t, trs) in timeline {
        wr = wr.interval(Interval(*t), trs)?;
    }
    wr.finish()
}

/// Validate descriptor semantics without materializing any raster.
fn validate_timeline(
    width: u32,
    height: u32,
    background: u8,
    objects: &[(u32, Object)],
    checkpoint: &[Instance],
    timeline: &[(u64, Vec<Transition>)],
) -> Result<(), VoleError> {
    let _ = (height, width, background); // geometry already guarded by writer + parser
    let mut seen = std::collections::HashSet::new();
    for (id, _) in objects {
        if !seen.insert(*id) {
            return Err(VoleError::DuplicateId);
        }
    }
    let mut st = State::new(Interval::ZERO);
    for (id, obj) in objects {
        st.declare_object(ObjectId(*id), obj.clone())?;
    }
    for inst in checkpoint {
        st.create_instance(inst.id, inst.object_id, inst.x, inst.y)?;
    }
    let mut prev_t = 0u64;
    let limits = crate::limits::Limits::default();
    let mut advance_work: u64 = 0;
    let mut trajectory_work: u64 = 0;
    let mut interval_no = 0u64;
    for (t, trs) in timeline {
        if *t == 0 || *t <= prev_t {
            return Err(VoleError::NonConsecutiveInterval);
        }
        prev_t = *t;
        interval_no += 1;
        limits.check_interval_distance(interval_no)?;
        if trs.len() as u64 > u64::from(limits.max_transitions_per_interval) {
            return Err(VoleError::MaterializationBudgetExceeded);
        }
        for tr in trs {
            // Frame-referencing ops carry their own geometry (they don't
            // touch painter state, so `apply` is a no-op); validate their
            // bounds here so an encoder can never serialize an out-of-limit
            // rectangle.
            if let Some((src_x, src_y, width, height, dst_x, dst_y)) = match tr {
                Transition::CopyRect {
                    src_x,
                    src_y,
                    width,
                    height,
                    dst_x,
                    dst_y,
                }
                | Transition::MoveRect {
                    src_x,
                    src_y,
                    width,
                    height,
                    dst_x,
                    dst_y,
                } => Some((*src_x, *src_y, *width, *height, *dst_x, *dst_y)),
                _ => None,
            } {
                const COORD: i64 = 1 << 24;
                if src_x.abs() > COORD
                    || src_y.abs() > COORD
                    || dst_x.abs() > COORD
                    || dst_y.abs() > COORD
                    || width == 0
                    || height == 0
                {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if u64::from(width) * u64::from(height)
                    > crate::limits::Limits::default().max_copy_area
                {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
            }
            if let Transition::Residual { block } = tr {
                if block.first() == Some(&crate::rans::KIND_TSF) {
                    crate::transform::check_block(block, limits.max_residual_bytes, width, height)?;
                } else {
                    crate::rans::check_block(block, limits.max_residual_bytes)?;
                }
            }
            if let Transition::SetTrajectory { segments, .. } = tr {
                crate::trajectory::check_program(segments, &limits)?;
            }
            if let Transition::SetVelocity { vx, vy, .. } = tr {
                // Canonical signed-domain guard (mirrors the parser's
                // `coord_guard`); the writer must never truncate a literal.
                if vx.abs() > crate::format::MAX_COORD || vy.abs() > crate::format::MAX_COORD {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
            // Advance work budgets are accounted *before* the advance applies
            // (a program that deactivates on this very step is still counted).
            if let Transition::AdvanceTranslations = tr {
                advance_work += st.moving_count() as u64;
                if advance_work > limits.max_transition_work {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
            }
            if let Transition::AdvanceTrajectories = tr {
                trajectory_work += st.trajectory_count() as u64;
                if trajectory_work > limits.max_trajectory_work {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
            }
            tr.apply(&mut st)?;
            if let Transition::PatchSparse { .. } = tr {
                limits.check_overlay_points(st.overlay_len() as u64)?;
            }
        }
    }
    Ok(())
}

/// Encode a procedural stream whose immutable objects are **external**
/// references (Phase P): each object is declared by its content id and its
/// canonical record bytes must be held by the [`crate::store::ObjectStore`]
/// used to decode the stream. The produced stream is deliberately **not
/// standalone**: it sets the external-objects feature bit, and store-less
/// decode fails with [`VoleError::StoreRequired`]. The timeline is validated
/// structurally (duplicate ids, unknown references, ordering); full content
/// validation happens at decode time when the store resolves every reference
/// and re-verifies each content id against the fetched record bytes.
pub fn encode_stream_external(
    width: u32,
    height: u32,
    background: u8,
    externs: &[(u32, crate::identity::ContentId)],
    checkpoint: &[Instance],
    timeline: &[(u64, Vec<Transition>)],
) -> Result<Vec<u8>, VoleError> {
    validate_external_timeline(width, height, background, externs, checkpoint, timeline)?;
    let mut wr = StreamWriter::begin(width, height).background(background);
    for (id, cid) in externs {
        wr = wr.declare_external(ObjectId(*id), *cid)?;
    }
    wr = wr.checkpoint_with(checkpoint)?;
    for (t, trs) in timeline {
        wr = wr.interval(Interval(*t), trs)?;
    }
    wr.finish()
}

/// Structural validation for an external-references stream. Placeholder
/// objects stand in for the (store-held) contents so every reference,
/// ordering, and budget rule the normative encoder enforces is checked; the
/// placeholder geometry never leaves this function.
#[allow(clippy::too_many_arguments)] // mirrors the encode_* surface
fn validate_external_timeline(
    width: u32,
    height: u32,
    background: u8,
    externs: &[(u32, crate::identity::ContentId)],
    checkpoint: &[Instance],
    timeline: &[(u64, Vec<Transition>)],
) -> Result<(), VoleError> {
    let _ = (width, height, background); // geometry guarded by writer + parser
    let mut seen = std::collections::HashSet::new();
    for (id, _) in externs {
        if !seen.insert(*id) {
            return Err(VoleError::DuplicateId);
        }
    }
    let mut st = State::new(Interval::ZERO);
    for (id, _) in externs {
        // Placeholder fill: `create_instance` only requires the id to exist;
        // contents are resolved and re-validated by `decode_with_store`.
        st.declare_object(ObjectId(*id), Object::fill(1, 1, 0)?)?;
    }
    for inst in checkpoint {
        st.create_instance(inst.id, inst.object_id, inst.x, inst.y)?;
    }
    let mut prev_t = 0u64;
    let limits = crate::limits::Limits::default();
    let mut advance_work: u64 = 0;
    let mut trajectory_work: u64 = 0;
    let mut interval_no = 0u64;
    for (t, trs) in timeline {
        if *t == 0 || *t <= prev_t {
            return Err(VoleError::NonConsecutiveInterval);
        }
        prev_t = *t;
        interval_no += 1;
        limits.check_interval_distance(interval_no)?;
        if trs.len() as u64 > u64::from(limits.max_transitions_per_interval) {
            return Err(VoleError::MaterializationBudgetExceeded);
        }
        for tr in trs {
            if let Transition::Residual { block } = tr {
                if block.first() == Some(&crate::rans::KIND_TSF) {
                    crate::transform::check_block(block, limits.max_residual_bytes, width, height)?;
                } else {
                    crate::rans::check_block(block, limits.max_residual_bytes)?;
                }
            }
            if let Transition::SetTrajectory { segments, .. } = tr {
                crate::trajectory::check_program(segments, &limits)?;
            }
            if let Transition::AdvanceTranslations = tr {
                advance_work += st.moving_count() as u64;
                if advance_work > limits.max_transition_work {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
            }
            if let Transition::AdvanceTrajectories = tr {
                trajectory_work += st.trajectory_count() as u64;
                if trajectory_work > limits.max_trajectory_work {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
            }
            tr.apply(&mut st)?;
            if let Transition::PatchSparse { .. } = tr {
                limits.check_overlay_points(st.overlay_len() as u64)?;
            }
        }
    }
    Ok(())
}

/// A small procedural source helper returning an [`InstanceId`] for a created
/// instance, mirroring the accepted direct-ingest shape (used by tooling and
/// courts). `id` is the caller-chosen instance identity.
pub fn make_instance(id: u32, object_id: ObjectId, x: i64, y: i64) -> Instance {
    Instance {
        id: InstanceId(id),
        object_id,
        x,
        y,
    }
}

/// Encode a procedural stream that begins with palette state (Phase J):
/// pre-checkpoint palette-table declarations and per-instance palette
/// bindings at the checkpoint, so palette-index content renders from frame 0.
/// Interval transitions may mutate palettes (`SetPalette`/`PatchPalette`) and
/// re-bind instances (`BindPalette`).
///
/// # Arguments
/// * `palettes` — `(palette id, initial entries)` declared before the
///   checkpoint (ids from 1; entries non-empty and ≤ `max_palette_entries`).
/// * `checkpoint` — the live instances anchoring interval 0 in paint order,
///   each with its optional palette binding (the bound palette must be in
///   `palettes`).
///
/// Everything else mirrors [`encode_stream`].
pub fn encode_palette_stream(
    width: u32,
    height: u32,
    background: u8,
    objects: &[(u32, Object)],
    palettes: &[(u32, Vec<u8>)],
    checkpoint: &[(Instance, Option<crate::state::PaletteId>)],
    timeline: &[(u64, Vec<Transition>)],
) -> Result<Vec<u8>, VoleError> {
    validate_palette_stream(
        width, height, background, objects, palettes, checkpoint, timeline,
    )?;
    let mut wr = crate::format::StreamWriter::begin(width, height).background(background);
    for (id, obj) in objects {
        wr = wr.declare_object(ObjectId(*id), obj.clone())?;
    }
    for (id, entries) in palettes {
        wr = wr.palette(crate::state::PaletteId(*id), entries.clone())?;
    }
    wr = wr.checkpoint_with_bindings(checkpoint)?;
    for (t, trs) in timeline {
        wr = wr.interval(Interval(*t), trs)?;
    }
    wr.finish()
}

/// Validate palette-aware descriptors with the same normative semantics the
/// parser uses, before serialization.
#[allow(clippy::too_many_arguments)] // mirrors the encode_* surface: 8 ordered descriptor lists
fn validate_palette_stream(
    width: u32,
    height: u32,
    background: u8,
    objects: &[(u32, Object)],
    palettes: &[(u32, Vec<u8>)],
    checkpoint: &[(Instance, Option<crate::state::PaletteId>)],
    timeline: &[(u64, Vec<Transition>)],
) -> Result<(), VoleError> {
    let _ = (width, height, background); // geometry guarded by writer + parser
    let mut seen = std::collections::HashSet::new();
    for (id, _) in objects {
        if !seen.insert(*id) {
            return Err(VoleError::DuplicateId);
        }
    }
    let mut st = State::new(Interval::ZERO);
    for (id, obj) in objects {
        st.declare_object(ObjectId(*id), obj.clone())?;
    }
    let limits = crate::limits::Limits::default();
    let mut seen_palettes = std::collections::HashSet::new();
    for (id, entries) in palettes {
        if !seen_palettes.insert(*id) {
            return Err(VoleError::DuplicateId);
        }
        if entries.is_empty() || *id == 0 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        if entries.len() as u64 > u64::from(limits.max_palette_entries) {
            return Err(VoleError::DimensionTooLarge);
        }
        st.set_palette(crate::state::PaletteId(*id), entries.clone())?;
        if st.palette_count() as u64 > u64::from(limits.max_palettes) {
            return Err(VoleError::DimensionTooLarge);
        }
    }
    let mut inst_ids = std::collections::HashSet::new();
    for (inst, binding) in checkpoint {
        if !inst_ids.insert(inst.id.0) {
            return Err(VoleError::DuplicateId);
        }
        st.create_instance(inst.id, inst.object_id, inst.x, inst.y)?;
        if let Some(p) = binding {
            st.bind_palette(inst.id, *p)?;
        }
    }
    let mut prev_t = 0u64;
    let mut advance_work: u64 = 0;
    let mut trajectory_work: u64 = 0;
    let mut interval_no = 0u64;
    for (t, trs) in timeline {
        if *t == 0 || *t <= prev_t {
            return Err(VoleError::NonConsecutiveInterval);
        }
        prev_t = *t;
        interval_no += 1;
        limits.check_interval_distance(interval_no)?;
        if trs.len() as u64 > u64::from(limits.max_transitions_per_interval) {
            return Err(VoleError::MaterializationBudgetExceeded);
        }
        for tr in trs {
            if let Some((src_x, src_y, width, height, dst_x, dst_y)) = match tr {
                Transition::CopyRect {
                    src_x,
                    src_y,
                    width,
                    height,
                    dst_x,
                    dst_y,
                }
                | Transition::MoveRect {
                    src_x,
                    src_y,
                    width,
                    height,
                    dst_x,
                    dst_y,
                } => Some((*src_x, *src_y, *width, *height, *dst_x, *dst_y)),
                _ => None,
            } {
                const COORD: i64 = 1 << 24;
                if src_x.abs() > COORD
                    || src_y.abs() > COORD
                    || dst_x.abs() > COORD
                    || dst_y.abs() > COORD
                    || width == 0
                    || height == 0
                {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if u64::from(width) * u64::from(height) > limits.max_copy_area {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
            }
            if let Transition::Residual { block } = tr {
                if block.first() == Some(&crate::rans::KIND_TSF) {
                    crate::transform::check_block(block, limits.max_residual_bytes, width, height)?;
                } else {
                    crate::rans::check_block(block, limits.max_residual_bytes)?;
                }
            }
            if let Transition::SetTrajectory { segments, .. } = tr {
                crate::trajectory::check_program(segments, &limits)?;
            }
            if let Transition::SetPalette { entries, .. } = tr {
                if entries.is_empty() {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if entries.len() as u64 > u64::from(limits.max_palette_entries) {
                    return Err(VoleError::DimensionTooLarge);
                }
            }
            if let Transition::AdvanceTranslations = tr {
                advance_work += st.moving_count() as u64;
                if advance_work > limits.max_transition_work {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
            }
            if let Transition::AdvanceTrajectories = tr {
                trajectory_work += st.trajectory_count() as u64;
                if trajectory_work > limits.max_trajectory_work {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
            }
            tr.apply(&mut st)?;
            if let Transition::PatchSparse { .. } = tr {
                limits.check_overlay_points(st.overlay_len() as u64)?;
            }
            if let Transition::SetPalette { .. } = tr {
                if st.palette_count() as u64 > u64::from(limits.max_palettes) {
                    return Err(VoleError::DimensionTooLarge);
                }
            }
        }
    }
    Ok(())
}
