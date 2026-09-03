//! Phase I — trajectory collapse (§43, §64 Phase I): the first
//! equivalence-preserving re-optimization primitive.
//!
//! The greedy raster encoder (Phase G/H) emits one absolute `SetPosition`
//! transition per moving frame — cheap per frame (26 B) but *temporally
//! blind*: it never notices that the position sequence it is writing is
//! parametric. This module rewrites a maximal run of single-`SetPosition`
//! interval groups into **one `SetTrajectory` descriptor plus per-frame
//! `AdvanceTrajectories`** when — and only when — both conditions of §43 hold:
//!
//! 1. **Materialization stays exact.** The replacement stream is always
//!    decoded with the normative decoder and compared frame-for-frame against
//!    the original stream before it is accepted. The encoder never trusts a
//!    fitted hypothesis from appearance; reconstruction is proven.
//! 2. **Complete cost falls.** Only a rebuilt stream with strictly fewer
//!    total bytes than the input is returned (never an equal-or-larger one).
//!
//! Exact fit families, tried in canonical order per run:
//!
//! * [`crate::trajectory::fit_linear`] — constant velocity (`x += d` runs);
//! * [`crate::trajectory::fit_accel`] — constant acceleration
//!   (`pos += v; v += a` runs);
//! * [`crate::trajectory::fit_piecewise`] — maximal constant-velocity
//!   segments (only accepted when the segment count is small enough to pay).
//!
//! [`collapse_stream`] applies at most **one** improving run per call so each
//! decision is individually proven; callers iterate to a fixpoint (the stream
//! strictly shrinks, so the loop terminates). `vole optimize` (Phase O)
//! generalizes this pass and adds the other re-optimization families; this
//! module keeps the Phase-I proof of the mechanism.

use crate::{
    decoder, encoder,
    error::VoleError,
    format::ParsedStream,
    object::Object,
    pixel::Canvas,
    state::{Instance, InstanceId},
    trajectory::{self, TrajectorySegment},
    transition::Transition,
};

/// A run of consecutive single-`SetPosition` groups (candidate for collapse).
struct Run {
    /// Index of the first group of the run in the timeline.
    start: usize,
    /// Index of the last group of the run.
    end: usize,
    /// Length of the run in groups/frames.
    len: usize,
    /// Target instance of every group in the run.
    id: InstanceId,
    /// Instance position *before* the run began (the trajectory start).
    p0: (i64, i64),
    /// Positions emitted by the run's groups (frame `start + k` → `pos[k]`).
    pos: Vec<(i64, i64)>,
}

/// Attempt one improving trajectory collapse over `bytes`.
///
/// Scans the stream's timeline for the first maximal run of interval groups
/// that each carry exactly one `SetPosition` transition for the same instance
/// (and no canvas ops). When the position sequence (including the instance's
/// position before the run) admits an exact linear, acceleration, or
/// piecewise-linear fit whose replacement **strictly shrinks the stream** and
/// whose **decoded frames are byte-identical** to the original, the rebuilt
/// stream is returned as `Some`. `None` means no improving run exists (the
/// input is a fixpoint under this pass).
///
/// Deterministic and bounded: one pass over the timeline, a constant number of
/// candidate programs per run, and full normative decode verification before
/// any acceptance.
pub fn collapse_stream(bytes: &[u8]) -> Result<Option<Vec<u8>>, VoleError> {
    let original = decoder::decode_bytes(bytes)?;
    let original_frames = decoder::materialize_all(&original)?;
    let runs = find_runs(&original)?;
    for run in runs {
        let Some(program) = fit_run(&run) else {
            continue;
        };
        // Cheap byte gate before any rebuild (strict improvement required):
        // the run's old cost is 26 B/group; the new cost is one
        // `SetTrajectory` (descriptor + advance in the first group) and 14 B
        // per following group.
        let old_run_bytes = 26 * run.len as u64;
        let new_run_bytes = 14 * run.len as u64 + trajectory::program_wire_bytes(&program) + 1;
        if new_run_bytes >= old_run_bytes {
            continue;
        }
        let rebuilt = rebuild(&original, &run, &program)?;
        if rebuilt.len() >= bytes.len() {
            continue;
        }
        // Normative exactness proof: decode the rebuilt stream and compare
        // every frame with the original decode. Never trust the fit.
        let new_frames = decoder::materialize_all(&decoder::decode_bytes(&rebuilt)?)?;
        if frames_equal(&original_frames, &new_frames) {
            return Ok(Some(rebuilt));
        }
    }
    Ok(None)
}

/// Locate maximal single-`SetPosition` runs by replaying the state timeline.
fn find_runs(parsed: &ParsedStream) -> Result<Vec<Run>, VoleError> {
    let mut replay = parsed.clone_initial();
    let mut out: Vec<Run> = Vec::new();
    let mut open: Option<Run> = None;
    let groups: Vec<(u64, Vec<Transition>)> = parsed
        .intervals()
        .iter()
        .map(|(t, trs)| (t.0, trs.clone()))
        .collect();
    for (idx, (_t, trs)) in groups.iter().enumerate() {
        // A run group is exactly one state transition: SetPosition (no canvas
        // ops — a group with a canvas op cannot be part of a pure motion run).
        let single = match trs.as_slice() {
            [Transition::SetPosition { id, x, y }] => Some((*id, (*x, *y))),
            _ => None,
        };
        match single {
            Some((id, pos)) => {
                let continuing = open
                    .as_ref()
                    .is_some_and(|r| r.id == id && r.end + 1 == idx);
                if continuing {
                    let r = open.as_mut().expect("open run exists");
                    r.end = idx;
                    r.len += 1;
                    r.pos.push(pos);
                } else {
                    // Close the previous run and start a new one. The start
                    // position is the instance position *before* this group
                    // (the replay state still holds the pre-group state).
                    if let Some(r) = open.take() {
                        if r.len >= 3 {
                            out.push(r);
                        }
                    }
                    let inst = replay
                        .instance(id)
                        .map_err(|_| VoleError::NonCanonicalEncoding)?;
                    if replay.velocity(id) != (0, 0) || replay.has_trajectory(id) {
                        // Attaching a trajectory would clear velocity/trajectory
                        // state the original kept; such a run is never a safe
                        // collapse candidate (the decode proof would reject it).
                        continue;
                    }
                    let p0 = (inst.x, inst.y);
                    open = Some(Run {
                        start: idx,
                        end: idx,
                        len: 1,
                        id,
                        p0,
                        pos: vec![pos],
                    });
                }
            }
            None => {
                if let Some(r) = open.take() {
                    if r.len >= 3 {
                        out.push(r);
                    }
                }
            }
        }
        // Advance the replay state (state ops only, in listed order; canvas
        // ops leave the painter state untouched).
        for tr in trs {
            if !is_canvas_op(tr) {
                tr.apply(&mut replay)?;
            }
        }
    }
    if let Some(r) = open.take() {
        if r.len >= 3 {
            out.push(r);
        }
    }
    Ok(out)
}

/// Choose the cheapest exact parametric program for a run (canonical fit
/// order: linear, acceleration, piecewise); all fits are exact by
/// construction and the normative decode proof remains the authority.
fn fit_run(run: &Run) -> Option<Vec<TrajectorySegment>> {
    // The full sequence the program must reproduce: p0 (state before the run)
    // plus one emitted position per run group.
    let mut seq: Vec<(i64, i64)> = Vec::with_capacity(run.pos.len() + 1);
    seq.push(run.p0);
    seq.extend_from_slice(&run.pos);
    let mut best: Option<Vec<TrajectorySegment>> = None;
    let mut best_bytes = u64::MAX;
    for program in [
        trajectory::fit_linear(&seq).map(|s| vec![s]),
        trajectory::fit_accel(&seq).map(|s| vec![s]),
        trajectory::fit_piecewise(&seq),
    ]
    .into_iter()
    .flatten()
    {
        let bytes = trajectory::program_wire_bytes(&program);
        if bytes < best_bytes {
            best_bytes = bytes;
            best = Some(program);
        }
    }
    best
}

/// Rebuild the timeline with one run replaced by `SetTrajectory` + per-group
/// `AdvanceTrajectories`, and serialize the full canonical stream.
fn rebuild(
    parsed: &ParsedStream,
    run: &Run,
    program: &[TrajectorySegment],
) -> Result<Vec<u8>, VoleError> {
    let initial = parsed.clone_initial();
    let mut objects: Vec<(u32, Object)> = Vec::new();
    for (id, obj) in initial.objects() {
        objects.push((id.0, obj.clone()));
    }
    let instances: Vec<Instance> = initial.instances().cloned().collect();
    let bg = initial.background();
    let mut timeline: Vec<(u64, Vec<Transition>)> = Vec::with_capacity(parsed.intervals().len());
    for (idx, (t, trs)) in parsed.intervals().iter().enumerate() {
        if idx == run.start {
            timeline.push((
                t.0,
                vec![
                    Transition::SetTrajectory {
                        id: run.id,
                        segments: program.to_vec(),
                    },
                    Transition::AdvanceTrajectories,
                ],
            ));
        } else if idx > run.start && idx <= run.end {
            timeline.push((t.0, vec![Transition::AdvanceTrajectories]));
        } else {
            timeline.push((t.0, trs.clone()));
        }
    }
    encoder::encode_stream(
        parsed.width(),
        parsed.height(),
        bg,
        &objects,
        &instances,
        &timeline,
    )
}

fn is_canvas_op(tr: &Transition) -> bool {
    matches!(
        tr,
        Transition::CopyRect { .. } | Transition::MoveRect { .. } | Transition::Residual { .. }
    )
}

fn frames_equal(a: &[Canvas], b: &[Canvas]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.exactly_matches(y))
}

/// Iterate `collapse_stream` to a fixpoint (bounded by the strictly shrinking
/// byte count). Returns the smallest stream reached.
pub fn collapse_fixpoint(mut bytes: Vec<u8>) -> Result<Vec<u8>, VoleError> {
    for _ in 0..256 {
        match collapse_stream(&bytes)? {
            Some(next) => {
                if next.len() >= bytes.len() {
                    break;
                }
                bytes = next;
            }
            None => break,
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Object;
    use crate::state::InstanceId;

    #[test]
    fn empty_and_too_short_timelines_are_fixpoints() -> Result<(), VoleError> {
        let obj = Object::fill(4, 4, 9)?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: crate::object::ObjectId(1),
            x: 0,
            y: 0,
        };
        // Static scene: no SetPosition runs at all.
        let mut timeline = Vec::new();
        for k in 1..=10u64 {
            timeline.push((k, Vec::<Transition>::new()));
        }
        let bytes = encoder::encode_stream(
            16,
            16,
            0,
            &[(1, obj.clone())],
            std::slice::from_ref(&inst),
            &timeline,
        )?;
        assert!(collapse_stream(&bytes)?.is_none());
        // A two-frame SetPosition run is too short to pay for its descriptor.
        let mut timeline2 = Vec::new();
        for k in 1..=2u64 {
            timeline2.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(1),
                    x: 2 * i64::try_from(k).unwrap(),
                    y: 0,
                }],
            ));
        }
        let bytes2 = encoder::encode_stream(
            16,
            16,
            0,
            &[(1, obj.clone())],
            std::slice::from_ref(&inst),
            &timeline2,
        )?;
        assert!(collapse_stream(&bytes2)?.is_none());
        Ok(())
    }

    #[test]
    fn constant_velocity_run_collapses_exactly_and_cheaper() -> Result<(), VoleError> {
        let obj = Object::fill(4, 4, 9)?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: crate::object::ObjectId(1),
            x: 0,
            y: 0,
        };
        let mut timeline = Vec::new();
        for k in 1..=40u64 {
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(1),
                    x: 2 * i64::try_from(k).unwrap(),
                    y: 0,
                }],
            ));
        }
        let original = encoder::encode_stream(96, 16, 0, &[(1, obj)], &[inst], &timeline)?;
        let improved = collapse_fixpoint(original.clone())?;
        assert!(improved.len() < original.len());
        // Byte-exactness through the normative decoder.
        let a = decoder::materialize_all(&decoder::decode_bytes(&original)?)?;
        let b = decoder::materialize_all(&decoder::decode_bytes(&improved)?)?;
        assert!(frames_equal(&a, &b));
        // The object really moved in both: final frame has the box at x=80.
        assert_eq!(a[40].get(81, 0), 9);
        assert_eq!(b[40].get(81, 0), 9);
        Ok(())
    }

    #[test]
    fn accelerating_run_collapses_when_parametric() -> Result<(), VoleError> {
        let obj = Object::fill(4, 4, 9)?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: crate::object::ObjectId(1),
            x: 0,
            y: 0,
        };
        // Constant-acceleration positions: p(k) = 2k + k(k-1)/2 (v0=2, a=1).
        let pos_of = |k: u64| -> i64 {
            let kk = i128::from(k);
            let x = 2 * kk + kk * (kk - 1) / 2;
            i64::try_from(x).unwrap()
        };
        let mut timeline = Vec::new();
        for k in 1..=24u64 {
            let x = pos_of(k);
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(1),
                    x,
                    y: 0,
                }],
            ));
        }
        let original = encoder::encode_stream(420, 16, 0, &[(1, obj)], &[inst], &timeline)?;
        let improved = collapse_fixpoint(original.clone())?;
        assert!(improved.len() < original.len());
        let a = decoder::materialize_all(&decoder::decode_bytes(&original)?)?;
        let b = decoder::materialize_all(&decoder::decode_bytes(&improved)?)?;
        assert!(frames_equal(&a, &b));
        assert_eq!(a.len(), 25);
        Ok(())
    }

    #[test]
    fn non_parametric_run_never_collapses() -> Result<(), VoleError> {
        let obj = Object::fill(4, 4, 9)?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: crate::object::ObjectId(1),
            x: 0,
            y: 0,
        };
        // A random walk of positions: not linear/accel in aggregate; the
        // piecewise fit is too long to pay, so the stream must be a fixpoint
        // and every decode remains exact.
        let mut x = 0i64;
        let mut seed = 0x5EED_0001u64;
        let mut timeline = Vec::new();
        for k in 1..=30u64 {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            let step = ((seed.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 60) as i64 % 5 - 2;
            x += step;
            let x = x.clamp(-(1 << 20), 1 << 20);
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(1),
                    x,
                    y: 0,
                }],
            ));
        }
        let original = encoder::encode_stream(64, 16, 0, &[(1, obj)], &[inst], &timeline)?;
        assert!(collapse_stream(&original)?.is_none());
        Ok(())
    }
}
