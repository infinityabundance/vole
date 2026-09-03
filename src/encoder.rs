//! High-level encode entry point (native procedural ingest → `.vole` bytes).
//!
//! Raster frames never enter this path: an encoder records immutable objects,
//! a checkpoint, and interval transitions. Rows are later *materialized*, not
//! stored. This function fully validates the timeline with the same normative
//! semantics the parser and decoder use (duplicate ids, unknown references,
//! invalid ordering all rejected) before serializing a canonical file.

use crate::{
    error::VoleError,
    format::StreamWriter,
    object::{Object, ObjectId},
    state::{Instance, InstanceId, State},
    time::Interval,
    transition::Transition,
};

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
    for (t, trs) in timeline {
        if *t == 0 || *t <= prev_t {
            return Err(VoleError::NonConsecutiveInterval);
        }
        prev_t = *t;
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
            tr.apply(&mut st)?;
            if let Transition::AdvanceTranslations = tr {
                advance_work += st.moving_count() as u64;
                if advance_work > limits.max_transition_work {
                    return Err(VoleError::MaterializationBudgetExceeded);
                }
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
