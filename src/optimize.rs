//! Phase O — equivalence-preserving representation re-optimization
//! (`vole optimize`, §44).
//!
//! Given a decoded `.vole` stream, optimize searches a bounded set of
//! **representation rewrites** and applies the first improving one. Every
//! candidate rewrite is accepted only when both hold:
//!
//! * **equivalence** — the rebuilt stream is decoded with the normative
//!   decoder and every materialized frame is byte-identical to the original
//!   (the `M(D0) == M(D1)` proof); and
//! * **improvement** — the rebuilt stream is strictly smaller (`J(D1) < J(D0)`).
//!
//! Rewrites are applied one at a time to a fixpoint (the stream strictly
//! shrinks, so the loop terminates). The families searched (deterministic
//! order per iteration):
//!
//! 1. **velocity collapse** — a run of per-frame `SetPosition` groups with a
//!    constant delta becomes one `SetVelocity` + one `AdvanceTranslations` per
//!    frame (cheaper than the trajectory descriptor for pure linear runs);
//! 2. **trajectory collapse** — the Phase-I parametric-program pass (accel /
//!    piecewise runs that velocity cannot serve);
//! 3. **residual promotion** — repeated identical one-shot residual blocks are
//!    replaced by one persistent sparse overlay, turning the repeated
//!    intervals into the unchanged lane;
//! 4. **generator substitution** — a declared raster object whose content is
//!    exactly a bounded procedural program (gradient / checker / periodic)
//!    is re-declared as that generator (samples never stored);
//! 5. **duplicate merge** — objects with identical content share one
//!    declaration (references remapped), the "shared object instead of
//!    repeated literals" family.
//!
//! Palette-bearing streams (pre-checkpoint palette records + checkpoint
//! bindings) are preserved verbatim: the rebuild path re-emits objects,
//! instances, and intervals only, so those streams are fixpoints (recorded
//! limitation, never a silent change). Noise is never substituted (seed
//! discovery is unbounded search; §21).

use crate::{
    collapse, decoder,
    error::VoleError,
    format::ParsedStream,
    generator::Generator,
    identity,
    object::{Object, ObjectId},
    pixel::Canvas,
    rans,
    state::Instance,
    transition::Transition,
};

/// The result of one `vole optimize` run.
#[derive(Debug, Clone)]
pub struct OptimizeReport {
    /// Bytes before optimization.
    pub before: Vec<u8>,
    /// Bytes after optimization (equal to `before` when nothing applied).
    pub stream: Vec<u8>,
    /// Rewrite families applied (in order).
    pub rewrites: Vec<&'static str>,
    /// True when the optimized stream decodes byte-identically to the input
    /// (always true by construction; re-checked at the end).
    pub exact: bool,
}

/// Optimize a standalone stream to a fixpoint of the bounded rewrite set.
/// Never grows the stream; never changes the decoded frames.
pub fn optimize_stream(bytes: &[u8]) -> Result<OptimizeReport, VoleError> {
    let mut cur = bytes.to_vec();
    let mut rewrites: Vec<&'static str> = Vec::new();
    for _ in 0..512 {
        match optimize_once(&cur)? {
            Some((next, label)) => {
                if next.len() >= cur.len() {
                    break;
                }
                rewrites.push(label);
                cur = next;
            }
            None => break,
        }
    }
    let before_frames = decoder::materialize_all(&decoder::decode_bytes(bytes)?)?;
    let after_frames = decoder::materialize_all(&decoder::decode_bytes(&cur)?)?;
    let exact = frames_equal_slices(&before_frames, &after_frames);
    Ok(OptimizeReport {
        before: bytes.to_vec(),
        stream: cur,
        rewrites,
        exact,
    })
}

/// Apply at most one improving rewrite (deterministic family order).
fn optimize_once(bytes: &[u8]) -> Result<Option<(Vec<u8>, &'static str)>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    let initial = parsed.clone_initial();
    if initial.palette_count() > 0 || initial.binding_count() > 0 {
        return Ok(None); // palette streams are preserved verbatim (documented)
    }
    let original_frames = decoder::materialize_all(&parsed)?;
    let original_len = bytes.len();

    // 1. Velocity collapse (pure linear runs).
    if let Some(rt) = velocity_collapse(&parsed, &original_frames, original_len)? {
        return Ok(Some(rt));
    }
    // 2. Phase-I trajectory collapse (accel / piecewise runs).
    if let Some(next) = collapse::collapse_stream(bytes)? {
        if next.len() < bytes.len() {
            return Ok(Some((next, "trajectory_collapse")));
        }
    }
    // 3. Residual promotion (repeated one-shot residual -> persistent overlay).
    if let Some(rt) = residual_promotion(&parsed, &original_frames, original_len)? {
        return Ok(Some(rt));
    }
    // 4. Generator substitution on declared raster objects.
    if let Some(rt) = generator_substitution(&parsed, &original_frames, original_len)? {
        return Ok(Some(rt));
    }
    // 5. Duplicate-content object merge.
    if let Some(rt) = duplicate_merge(&parsed, &original_frames, original_len)? {
        return Ok(Some(rt));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Shared rebuild machinery (mirrors the Phase-I rebuild proof shape)
// ---------------------------------------------------------------------------

/// Re-serialize a full canonical stream from descriptor lists (non-palette).
fn rebuild(
    parsed: &ParsedStream,
    objects: &[(u32, Object)],
    instances: &[Instance],
    timeline: &[(u64, Vec<Transition>)],
) -> Result<Vec<u8>, VoleError> {
    crate::encoder::encode_stream(
        parsed.width(),
        parsed.height(),
        parsed.clone_initial().background(),
        objects,
        instances,
        timeline,
    )
}

fn groups_of(parsed: &ParsedStream) -> Vec<(u64, Vec<Transition>)> {
    parsed
        .intervals()
        .iter()
        .map(|(t, trs)| (t.0, trs.clone()))
        .collect()
}

fn initial_descriptors(parsed: &ParsedStream) -> (Vec<(u32, Object)>, Vec<Instance>) {
    let initial = parsed.clone_initial();
    let objects: Vec<(u32, Object)> = initial
        .objects()
        .map(|(id, obj)| (id.0, obj.clone()))
        .collect();
    let instances: Vec<Instance> = initial.instances().cloned().collect();
    (objects, instances)
}

fn frames_equal_slices(a: &[Canvas], b: &[Canvas]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.exactly_matches(y))
}

/// Accept a candidate only when it is strictly smaller and decodes to the
/// identical frame sequence.
fn proven(
    candidate: Vec<u8>,
    original_len: usize,
    original_frames: &[Canvas],
    label: &'static str,
) -> Result<Option<(Vec<u8>, &'static str)>, VoleError> {
    if candidate.len() >= original_len {
        return Ok(None);
    }
    let new_frames = decoder::materialize_all(&decoder::decode_bytes(&candidate)?)?;
    if frames_equal_slices(original_frames, &new_frames) {
        Ok(Some((candidate, label)))
    } else {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// 1. Velocity collapse
// ---------------------------------------------------------------------------

/// A constant-delta run of per-frame `SetPosition` groups becomes one
/// `SetVelocity` + per-frame `AdvanceTranslations` (13 + len bytes vs
/// 13·len), gated by the ordinary guards (no prior velocity/trajectory on the
/// instance) and proven by decode.
fn velocity_collapse(
    parsed: &ParsedStream,
    original_frames: &[Canvas],
    original_len: usize,
) -> Result<Option<(Vec<u8>, &'static str)>, VoleError> {
    let runs = collapse::find_runs(parsed)?;
    for run in runs {
        let Some((vx, vy)) = run.constant_velocity() else {
            continue;
        };
        if vx == 0 && vy == 0 {
            continue; // a hold is the unchanged lane's business
        }
        // Cheap byte gate: SetVelocity(13) + len advances vs len SetPosition
        // (13 B each; the interval envelope cancels on both sides).
        let old_run_bytes = 13 * run.len as u64;
        let new_run_bytes = 13 + run.len as u64;
        if new_run_bytes >= old_run_bytes {
            continue;
        }
        let groups = groups_of(parsed);
        let mut timeline: Vec<(u64, Vec<Transition>)> = Vec::with_capacity(groups.len());
        for (idx, (t, trs)) in groups.iter().enumerate() {
            if idx == run.start {
                timeline.push((
                    *t,
                    vec![
                        Transition::SetVelocity { id: run.id, vx, vy },
                        Transition::AdvanceTranslations,
                    ],
                ));
            } else if idx > run.start && idx <= run.end {
                timeline.push((*t, vec![Transition::AdvanceTranslations]));
            } else {
                timeline.push((*t, trs.clone()));
            }
        }
        let (objects, instances) = initial_descriptors(parsed);
        let candidate = rebuild(parsed, &objects, &instances, &timeline)?;
        if let Some(rt) = proven(
            candidate,
            original_len,
            original_frames,
            "velocity_collapse",
        )? {
            return Ok(Some(rt));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// 3. Residual promotion
// ---------------------------------------------------------------------------

/// Decode a kind-0/1 point residual block into its canonical point list.
fn decode_points(block: &[u8], limits: &crate::limits::Limits) -> Option<Vec<(i64, i64, u8)>> {
    if block.first() == Some(&rans::KIND_TSF) {
        return None; // transform residuals are not point lists
    }
    let payload = rans::decode_block(block, limits.max_residual_bytes).ok()?;
    if payload.len() % 9 != 0 {
        return None;
    }
    let mut pts = Vec::with_capacity(payload.len() / 9);
    let mut prev: Option<(i64, i64)> = None;
    for p in payload.as_chunks::<9>().0 {
        let x = i64::from(i32::from_le_bytes([p[0], p[1], p[2], p[3]]));
        let y = i64::from(i32::from_le_bytes([p[4], p[5], p[6], p[7]]));
        let v = p[8];
        if x < 0 || y < 0 {
            return None;
        }
        let key = (x, y);
        if prev.is_some_and(|q| key <= q) {
            return None;
        }
        prev = Some(key);
        pts.push((x, y, v));
    }
    Some(pts)
}

/// Consecutive groups carrying the *same* one-shot residual block describe a
/// persistent visual difference against a static base: promote the block to
/// one persistent sparse overlay and let the later intervals ride the
/// unchanged lane (the recorded Phase-G/K gap "stable residuals pay one-shot
/// per frame until Phase O promotes them").
fn residual_promotion(
    parsed: &ParsedStream,
    original_frames: &[Canvas],
    original_len: usize,
) -> Result<Option<(Vec<u8>, &'static str)>, VoleError> {
    let limits = crate::limits::Limits::default();
    let groups = groups_of(parsed);
    let mut i = 0usize;
    while i < groups.len() {
        // Open a maximal run of identical single-residual groups.
        let first = match &groups[i].1[..] {
            [Transition::Residual { block }] => block.clone(),
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i + 1;
        while j < groups.len() {
            match &groups[j].1[..] {
                [Transition::Residual { block }] if *block == first => j += 1,
                _ => break,
            }
        }
        let m = j - i;
        if m < 2 {
            i = j;
            continue;
        }
        let Some(pts) = decode_points(&first, &limits) else {
            i = j;
            continue;
        };
        if pts.len() > limits.max_overlay_points as usize {
            i = j;
            continue;
        }
        // Byte gate: original run bytes vs one patch + unchanged lanes.
        let old_run = m as u64 * (18 + first.len() as u64);
        let new_run = (18 + 9 * pts.len() as u64) + (m as u64 - 1) * 13;
        if new_run >= old_run {
            i = j;
            continue;
        }
        // Rebuild: first group carries the persistent patch; the rest are the
        // unchanged lane. The decode proof owns correctness (a later group
        // that needs the unpainted base rejects the rewrite).
        let mut timeline: Vec<(u64, Vec<Transition>)> = Vec::with_capacity(groups.len());
        for (idx, (t, trs)) in groups.iter().enumerate() {
            if idx == i {
                timeline.push((
                    *t,
                    vec![Transition::PatchSparse {
                        points: pts.clone(),
                    }],
                ));
            } else if idx > i && idx < j {
                timeline.push((*t, Vec::new()));
            } else {
                timeline.push((*t, trs.clone()));
            }
        }
        let (objects, instances) = initial_descriptors(parsed);
        let candidate = rebuild(parsed, &objects, &instances, &timeline)?;
        if let Some(rt) = proven(
            candidate,
            original_len,
            original_frames,
            "residual_promotion",
        )? {
            return Ok(Some(rt));
        }
        i = j;
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// 4. Generator substitution
// ---------------------------------------------------------------------------

/// The exact generator program of a stored raster object, if its samples are
/// precisely a bounded program (gradient / checker / periodic; never noise).
fn generator_of(obj: &Object) -> Option<Generator> {
    let samples = obj.samples()?;
    let (w, h) = (obj.width(), obj.height());
    if w < 2 || h < 2 {
        return None;
    }
    // Reuse the encoder's deterministic content fits over the object raster.
    let canvas = Canvas::from_parts(w, h, samples.to_vec()).ok()?;
    for gen in crate::inverse::fit_generators(&canvas, false) {
        // Normative byte-for-byte check over the whole box.
        let all = {
            let mut ok = true;
            'outer: for y in 0..i64::from(h) {
                for x in 0..i64::from(w) {
                    if gen.sample(x, y) != samples[(y * i64::from(w) + x) as usize] {
                        ok = false;
                        break 'outer;
                    }
                }
            }
            ok
        };
        if all {
            return Some(gen);
        }
    }
    None
}

/// Re-declare raster objects whose content is exactly a bounded program as
/// generator objects (the declaration stores the program, never the samples).
fn generator_substitution(
    parsed: &ParsedStream,
    original_frames: &[Canvas],
    original_len: usize,
) -> Result<Option<(Vec<u8>, &'static str)>, VoleError> {
    let (objects, instances) = initial_descriptors(parsed);
    let mut objects2 = objects;
    let mut changed = false;
    for slot in objects2.iter_mut() {
        if slot.1.samples().is_none() {
            continue; // already a fill / index / generator object
        }
        if slot.1.sample_count() < 32 {
            continue; // a generator declaration cannot pay below ~one row
        }
        let Some(gen) = generator_of(&slot.1) else {
            continue;
        };
        let old_decl = 13 + slot.1.sample_count();
        let new_decl = 13 + gen.program_bytes().len() as u64;
        if new_decl >= old_decl {
            continue;
        }
        if let Ok(obj) = Object::procedural(slot.1.width(), slot.1.height(), gen) {
            slot.1 = obj;
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    let timeline = groups_of(parsed);
    let candidate = rebuild(parsed, &objects2, &instances, &timeline)?;
    proven(
        candidate,
        original_len,
        original_frames,
        "generator_substitution",
    )
}

// ---------------------------------------------------------------------------
// 5. Duplicate merge
// ---------------------------------------------------------------------------

/// Objects with byte-identical content share one declaration; every reference
/// (checkpoint instances and interval `CreateInstance` transitions) is
/// remapped to the kept id.
fn duplicate_merge(
    parsed: &ParsedStream,
    original_frames: &[Canvas],
    original_len: usize,
) -> Result<Option<(Vec<u8>, &'static str)>, VoleError> {
    let (objects, instances) = initial_descriptors(parsed);
    // First occurrence keeps the id; duplicates map onto it.
    let mut keep: Vec<(u32, Object)> = Vec::new();
    let mut map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut seen: std::collections::HashMap<identity::ContentId, u32> =
        std::collections::HashMap::new();
    for (id, obj) in &objects {
        if let Some(&first) = seen.get(&identity::content_id_of(obj)) {
            map.insert(*id, first);
        } else {
            seen.insert(identity::content_id_of(obj), *id);
            keep.push((*id, obj.clone()));
        }
    }
    if map.is_empty() {
        return Ok(None);
    }
    let remap = |oid: u32| map.get(&oid).copied().unwrap_or(oid);
    let instances2: Vec<Instance> = instances
        .iter()
        .map(|i| Instance {
            id: i.id,
            object_id: ObjectId(remap(i.object_id.0)),
            x: i.x,
            y: i.y,
        })
        .collect();
    let timeline: Vec<(u64, Vec<Transition>)> = groups_of(parsed)
        .into_iter()
        .map(|(t, trs)| {
            let trs = trs
                .into_iter()
                .map(|tr| match tr {
                    Transition::CreateInstance { id, object, x, y } => Transition::CreateInstance {
                        id,
                        object: ObjectId(remap(object.0)),
                        x,
                        y,
                    },
                    other => other,
                })
                .collect();
            (t, trs)
        })
        .collect();
    let candidate = rebuild(parsed, &keep, &instances2, &timeline)?;
    proven(candidate, original_len, original_frames, "duplicate_merge")
}
