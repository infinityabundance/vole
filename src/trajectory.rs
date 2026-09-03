//! Phase I — bounded parametric trajectories (normative mechanics).
//!
//! A **trajectory** is a finite, deterministic motion program attached to one
//! instance. It makes parametric motion first-class procedural state: instead
//! of describing a moving object frame-by-frame (`SET_POSITION` per interval),
//! the encoder may attach one compact program and step it with a one-byte
//! advance op. This module defines the program model, its canonical wire
//! forms, the exact integer evaluation semantics, and the **fitting** helpers
//! an encoder uses to discover whether an observed position sequence really is
//! parametric (exactness is always re-proven through the normative materializer
//! before any collapse is committed — see `crate::collapse`).
//!
//! # Normative semantics (integer, exact — no floating point anywhere)
//!
//! Time steps are *advances*: one advance is applied per `AdvanceTrajectories`
//! transition. A program is a list of segments executed in order. Each segment
//! runs for its declared `steps` advances, then the next segment starts; when
//! the final segment's steps are exhausted the trajectory deactivates and the
//! instance stays at its final position (an *empty* program deactivates
//! immediately).
//!
//! * [`TrajectorySegment::Linear`]: during each of its `steps` advances the
//!   instance position gains `(vx, vy)` (a constant velocity — "constant" and
//!   "linear" motion in §20 terms; a `(0,0)` velocity is an exact hold).
//! * [`TrajectorySegment::Accel`]: the velocity starts at `(vx0, vy0)` and
//!   gains `(ax, ay)` *after* each advance. After `t` advances of one segment
//!   the displacement is the exact integer closed form
//!
//!   ```text
//!   Δ(t) = t·v0 + a·t·(t−1)/2        (t = advances applied so far)
//!   ```
//!
//!   i.e. velocity during the `k`-th advance (0-based) is `v0 + k·a`, which is
//!   the discrete-time recurrence `pos += v; v += a` — the canonical integer
//!   form of `x(t) = x0 + v0·t + ½·a·t²` used in §20/§64 Phase I. All
//!   arithmetic is checked; an overflowing accumulation is a typed error, never
//!   a wrap.
//!
//! A piecewise-linear trajectory is a program whose segments are `Linear`
//! segments with different velocities (runs are maximal, so two adjacent
//! segments never share a velocity — that would be a non-canonical encoding).

use crate::error::VoleError;

/// Wire kind byte of a linear segment (canonical; `docs/format-v1.md`).
pub(crate) const SEG_LINEAR: u8 = 0x00;
/// Wire kind byte of an acceleration segment.
pub(crate) const SEG_ACCEL: u8 = 0x01;

/// One bounded segment of a trajectory program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrajectorySegment {
    /// Constant-velocity translation: the instance position gains `(vx, vy)`
    /// on each of `steps` consecutive trajectory advances.
    Linear { vx: i64, vy: i64, steps: u64 },
    /// Constant-acceleration segment: velocity starts at `(vx0, vy0)` and
    /// gains `(ax, ay)` after each advance; the position advances by the
    /// current velocity on each of `steps` advances.
    Accel {
        vx0: i64,
        vy0: i64,
        ax: i64,
        ay: i64,
        steps: u64,
    },
}

impl TrajectorySegment {
    /// Advances this segment occupies.
    pub fn steps(&self) -> u64 {
        match self {
            TrajectorySegment::Linear { steps, .. } | TrajectorySegment::Accel { steps, .. } => {
                *steps
            }
        }
    }

    /// Serialized byte length of this segment on the wire
    /// (`kind:u8` + signed `i32` fields + `steps:u64`).
    pub fn wire_bytes(&self) -> u64 {
        match self {
            TrajectorySegment::Linear { .. } => 17,
            TrajectorySegment::Accel { .. } => 25,
        }
    }

    /// Canonical-form check (hostile rule: a non-canonical segment is a typed
    /// error, never accepted):
    /// * `steps` must be ≥ 1 (a zero-step segment is dead weight);
    /// * every signed literal must fit the v1 coordinate domain `±2^24`;
    /// * an `Accel` with `(ax, ay) == (0, 0)` is rejected — it is a constant
    ///   velocity and must be written canonically as `Linear`.
    pub(crate) fn check(&self) -> Result<(), VoleError> {
        let coord_ok = |v: i64| v.abs() <= crate::format::MAX_COORD;
        match self {
            TrajectorySegment::Linear { vx, vy, steps } => {
                if *steps == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if !coord_ok(*vx) || !coord_ok(*vy) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
            TrajectorySegment::Accel {
                vx0,
                vy0,
                ax,
                ay,
                steps,
            } => {
                if *steps == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if *ax == 0 && *ay == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if !coord_ok(*vx0) || !coord_ok(*vy0) || !coord_ok(*ax) || !coord_ok(*ay) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
        }
        Ok(())
    }
}

/// Validate a whole trajectory program: segment-count bound against `limits`,
/// per-segment canonical form, and the adjacency rule (two adjacent `Linear`
/// segments with the same velocity must have been merged — one canonical
/// encoding only).
pub fn check_program(
    segments: &[TrajectorySegment],
    limits: &crate::limits::Limits,
) -> Result<(), VoleError> {
    if segments.len() as u64 > u64::from(limits.max_trajectory_segments) {
        return Err(VoleError::MaterializationBudgetExceeded);
    }
    for seg in segments {
        seg.check()?;
    }
    for pair in segments.windows(2) {
        if let [TrajectorySegment::Linear { vx: a, vy: b, .. }, TrajectorySegment::Linear { vx: c, vy: d, .. }] =
            pair
        {
            if a == c && b == d {
                return Err(VoleError::NonCanonicalEncoding);
            }
        }
    }
    Ok(())
}

/// Serialized byte length of one `SetTrajectory` payload holding `segments`:
/// `tag(1) + iid(4) + count(4) + Σ segments` (mirrors `format.rs`).
pub fn program_wire_bytes(segments: &[TrajectorySegment]) -> u64 {
    9 + segments
        .iter()
        .map(TrajectorySegment::wire_bytes)
        .sum::<u64>()
}

/// Exact displacement of one segment over exactly `n` advances
/// (`0 ≤ n ≤ steps`), closed-form, checked. `None` on overflow (callers treat
/// an overflow as "no exact answer", never a wrap).
fn segment_delta(seg: &TrajectorySegment, n: u64) -> Option<(i64, i64)> {
    if n == 0 {
        return Some((0, 0));
    }
    match seg {
        TrajectorySegment::Linear { vx, vy, .. } => scale2(*vx, *vy, i128::from(n)),
        TrajectorySegment::Accel {
            vx0, vy0, ax, ay, ..
        } => {
            // t = n·(n−1)/2 (exact integer; fits u128 then i128 for n ≤ u64).
            let t = u128::from(n) * u128::from(n - 1) / 2;
            let t = i128::try_from(t).ok()?;
            let nn = i128::from(n);
            let x = nn
                .checked_mul(i128::from(*vx0))?
                .checked_add(t.checked_mul(i128::from(*ax))?)?;
            let y = nn
                .checked_mul(i128::from(*vy0))?
                .checked_add(t.checked_mul(i128::from(*ay))?)?;
            shrink2(x, y)
        }
    }
}

fn scale2(vx: i64, vy: i64, n: i128) -> Option<(i64, i64)> {
    shrink2(
        n.checked_mul(i128::from(vx))?,
        n.checked_mul(i128::from(vy))?,
    )
}

fn shrink2(x: i128, y: i128) -> Option<(i64, i64)> {
    Some((i64::try_from(x).ok()?, i64::try_from(y).ok()?))
}

/// Exact position of an instance that starts at `(x0, y0)` and runs `steps`
/// advances of `segments` (closed-form per segment; independent of the
/// incremental stepper in `State` so the two implementations cross-check each
/// other in the courts). `None` when the program is shorter than `steps` or an
/// intermediate accumulation overflows.
pub fn position_after(
    segments: &[TrajectorySegment],
    x0: i64,
    y0: i64,
    steps: u64,
) -> Option<(i64, i64)> {
    let (mut x, mut y) = (x0, y0);
    let mut left = steps;
    for seg in segments {
        let n = left.min(seg.steps());
        let (dx, dy) = segment_delta(seg, n)?;
        x = x.checked_add(dx)?;
        y = y.checked_add(dy)?;
        left -= n;
        if left == 0 {
            return Some((x, y));
        }
    }
    None
}

/// The full position sequence `(p_0 … p_steps)` an instance starting at
/// `(x0, y0)` visits under `segments` (used by the reference painters).
pub fn simulate_positions(
    segments: &[TrajectorySegment],
    x0: i64,
    y0: i64,
    steps: u64,
) -> Option<Vec<(i64, i64)>> {
    (0..=steps)
        .map(|k| position_after(segments, x0, y0, k))
        .collect()
}

// ---------------------------------------------------------------------------
// Exact fitting (the "is this really parametric?" question)
// ---------------------------------------------------------------------------

/// Exact **constant-velocity (linear)** fit of a position sequence. Requires
/// `positions.len() ≥ 2`; returns a `Linear` segment of `len−1` steps that
/// reproduces every listed position exactly, or `None` when the sequence is
/// not linear or its velocity is not wire-encodable (`|v| ≤ 2^24`).
pub fn fit_linear(positions: &[(i64, i64)]) -> Option<TrajectorySegment> {
    let steps = positions.len().checked_sub(1)?;
    if steps == 0 {
        return None;
    }
    let (vx, vy) = sub_pair(positions[1], positions[0])?;
    if !coord_ok(vx) || !coord_ok(vy) {
        return None;
    }
    for k in 0..=steps {
        let p = positions[k];
        let expect_x = positions[0]
            .0
            .checked_add(vx.checked_mul(i64::try_from(k).ok()?)?)?;
        let expect_y = positions[0]
            .1
            .checked_add(vy.checked_mul(i64::try_from(k).ok()?)?)?;
        if p != (expect_x, expect_y) {
            return None;
        }
    }
    Some(TrajectorySegment::Linear {
        vx,
        vy,
        steps: steps as u64,
    })
}

/// Exact **constant-acceleration** fit (discrete semantics above). Requires at
/// least three positions; the first differences must form an arithmetic
/// progression with a *non-zero* common difference `(ax, ay)` (a zero
/// difference is a constant velocity and belongs to `fit_linear`). All
/// literals must be wire-encodable.
pub fn fit_accel(positions: &[(i64, i64)]) -> Option<TrajectorySegment> {
    let steps = positions.len().checked_sub(1)?;
    if steps < 2 {
        return None;
    }
    let d0 = sub_pair(positions[1], positions[0])?;
    let d1 = sub_pair(positions[2], positions[1])?;
    let ax = d1.0.checked_sub(d0.0)?;
    let ay = d1.1.checked_sub(d0.1)?;
    if ax == 0 && ay == 0 {
        return None; // constant velocity: linear, not accel
    }
    if !coord_ok(d0.0) || !coord_ok(d0.1) || !coord_ok(ax) || !coord_ok(ay) {
        return None;
    }
    // Velocity during advance k (0-based) is v0 + k·a; verify every step.
    for k in 0..steps {
        let vx = d0.0.checked_add(ax.checked_mul(i64::try_from(k).ok()?)?)?;
        let vy = d0.1.checked_add(ay.checked_mul(i64::try_from(k).ok()?)?)?;
        let expect_x = positions[k].0.checked_add(vx)?;
        let expect_y = positions[k].1.checked_add(vy)?;
        if positions[k + 1] != (expect_x, expect_y) {
            return None;
        }
    }
    Some(TrajectorySegment::Accel {
        vx0: d0.0,
        vy0: d0.1,
        ax,
        ay,
        steps: steps as u64,
    })
}

/// Exact **piecewise-linear** fit: maximal runs of identical step deltas become
/// `Linear` segments, so the program reproduces every position exactly by
/// construction. Returns `None` when fewer than two segments are needed (a
/// single run is `fit_linear` territory), a run velocity is not
/// wire-encodable, or a step delta overflows.
pub fn fit_piecewise(positions: &[(i64, i64)]) -> Option<Vec<TrajectorySegment>> {
    let steps = positions.len().checked_sub(1)?;
    if steps == 0 {
        return None;
    }
    let mut deltas: Vec<(i64, i64)> = Vec::with_capacity(steps);
    for k in 0..steps {
        let d = sub_pair(positions[k + 1], positions[k])?;
        if !coord_ok(d.0) || !coord_ok(d.1) {
            return None;
        }
        deltas.push(d);
    }
    let mut out: Vec<TrajectorySegment> = Vec::new();
    let mut i = 0usize;
    while i < steps {
        let (vx, vy) = deltas[i];
        let mut j = i + 1;
        while j < steps && deltas[j] == (vx, vy) {
            j += 1;
        }
        out.push(TrajectorySegment::Linear {
            vx,
            vy,
            steps: (j - i) as u64,
        });
        i = j;
    }
    if out.len() < 2 {
        return None;
    }
    Some(out)
}

fn coord_ok(v: i64) -> bool {
    v.abs() <= crate::format::MAX_COORD
}

fn sub_pair(a: (i64, i64), b: (i64, i64)) -> Option<(i64, i64)> {
    Some((a.0.checked_sub(b.0)?, a.1.checked_sub(b.1)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_bytes_are_fixed() {
        assert_eq!(
            TrajectorySegment::Linear {
                vx: 1,
                vy: 0,
                steps: 5
            }
            .wire_bytes(),
            17
        );
        assert_eq!(
            TrajectorySegment::Accel {
                vx0: 1,
                vy0: 0,
                ax: 1,
                ay: 0,
                steps: 5
            }
            .wire_bytes(),
            25
        );
        assert_eq!(program_wire_bytes(&[]), 9);
    }

    #[test]
    fn displacement_matches_the_discrete_recurrence() {
        // Closed form vs a manual recurrence over a short accel segment.
        let seg = TrajectorySegment::Accel {
            vx0: 2,
            vy0: 1,
            ax: 1,
            ay: 0,
            steps: 10,
        };
        let (mut x, mut y, mut vx, vy) = (100i64, 60i64, 2i64, 1i64);
        for n in 1..=10u64 {
            x += vx;
            y += vy;
            vx += 1;
            let (ex, ey) = segment_delta(&seg, n).expect("closed form");
            assert_eq!((x - 100, y - 60), (ex, ey), "advance {n}");
        }
    }

    #[test]
    fn position_after_handles_segment_boundaries_and_deactivation() {
        let program = vec![
            TrajectorySegment::Linear {
                vx: 2,
                vy: 0,
                steps: 3,
            },
            TrajectorySegment::Linear {
                vx: 0,
                vy: 0,
                steps: 2,
            },
            TrajectorySegment::Linear {
                vx: -1,
                vy: 0,
                steps: 4,
            },
        ];
        assert_eq!(position_after(&program, 0, 0, 0), Some((0, 0)));
        assert_eq!(position_after(&program, 0, 0, 3), Some((6, 0)));
        assert_eq!(position_after(&program, 0, 0, 5), Some((6, 0))); // hold
        assert_eq!(position_after(&program, 0, 0, 9), Some((2, 0)));
        assert_eq!(position_after(&program, 0, 0, 10), None); // program done
    }

    #[test]
    fn fits_are_exact_and_mutually_exclusive() {
        let linear: Vec<(i64, i64)> = (0..20).map(|k| (100 + 2 * k, 60 + k)).collect();
        let f = fit_linear(&linear).expect("linear fits");
        assert_eq!(f.steps(), 19);
        assert!(fit_accel(&linear).is_none(), "linear must not fit accel");
        assert!(
            fit_piecewise(&linear).is_none(),
            "single run is not piecewise"
        );

        // Constant acceleration: v(t) = 2 + t (velocities 2,3,4,...).
        let mut accel = vec![(100i64, 60i64)];
        let (mut x, mut vx) = (100i64, 2i64);
        for _ in 0..12 {
            x += vx;
            vx += 1;
            accel.push((x, 60));
        }
        assert!(fit_linear(&accel).is_none(), "accel must not fit linear");
        let a = fit_accel(&accel).expect("accel fits");
        assert_eq!(a.steps(), 12);
        assert!(fit_piecewise(&accel).is_none() || fit_piecewise(&accel).unwrap().len() >= 12);

        // Random walk: no exact parametric fit of any kind.
        let mut walk = vec![(0i64, 0i64)];
        let mut seed = 0x1234_5678_9ABC_DEF0u64;
        for _ in 0..30 {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            seed = seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
            let dx = (seed >> 60) as i64 % 3 - 1;
            let dy = ((seed >> 40) as i64) % 3 - 1;
            let last = *walk.last().unwrap();
            walk.push((last.0 + dx, last.1 + dy));
        }
        // A random walk can contain short accidental runs, so the honest
        // assertion is that *reproduction is exact whenever a fit exists*:
        // re-simulate any candidate and compare against the walk.
        if let Some(f) = fit_linear(&walk) {
            let got = simulate_positions(&[f], 0, 0, walk.len() as u64 - 1).unwrap();
            assert_eq!(got, walk);
        }
        if let Some(f) = fit_accel(&walk) {
            let got = simulate_positions(&[f], 0, 0, walk.len() as u64 - 1).unwrap();
            assert_eq!(got, walk);
        }
        if let Some(p) = fit_piecewise(&walk) {
            let got = simulate_positions(&p, 0, 0, walk.len() as u64 - 1).unwrap();
            assert_eq!(got, walk);
        }
    }

    #[test]
    fn canonical_checks_reject_hostile_segments() {
        let ok_limits = crate::limits::Limits::default();
        // Zero steps.
        assert_eq!(
            TrajectorySegment::Linear {
                vx: 1,
                vy: 0,
                steps: 0
            }
            .check()
            .unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Accel with zero acceleration must be written Linear.
        assert_eq!(
            TrajectorySegment::Accel {
                vx0: 1,
                vy0: 0,
                ax: 0,
                ay: 0,
                steps: 3
            }
            .check()
            .unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Coordinate overflow of the canonical signed domain.
        assert_eq!(
            TrajectorySegment::Linear {
                vx: (1 << 24) + 1,
                vy: 0,
                steps: 3
            }
            .check()
            .unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Adjacent equal-velocity linear segments must be merged.
        let merged = vec![
            TrajectorySegment::Linear {
                vx: 2,
                vy: 0,
                steps: 3,
            },
            TrajectorySegment::Linear {
                vx: 2,
                vy: 0,
                steps: 4,
            },
        ];
        assert_eq!(
            check_program(&merged, &ok_limits).unwrap_err(),
            VoleError::NonCanonicalEncoding
        );
        // Segment-count bound.
        let mut too_many = Vec::new();
        for k in 0..=ok_limits.max_trajectory_segments {
            too_many.push(TrajectorySegment::Linear {
                vx: 1,
                vy: 0,
                steps: u64::from(k % 3) + 1,
            });
        }
        assert_eq!(
            check_program(&too_many, &ok_limits).unwrap_err(),
            VoleError::MaterializationBudgetExceeded
        );
    }
}
