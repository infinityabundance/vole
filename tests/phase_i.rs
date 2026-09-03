//! Phase I courts: bounded parametric trajectories.
//!
//! A trajectory is a finite, deterministic motion *program* attached to one
//! instance (linear / acceleration / piecewise-linear segments, integer
//! arithmetic, exact discrete semantics). Courts cover:
//! * the accelerating §76-analogue flagship — one object + one instance + one
//!   checkpoint + one trajectory program reconstructing every frame exactly,
//!   byte-identical to an independent closed-form reference painter;
//! * byte comparison against the per-frame `SetPosition` and per-frame
//!   `SetVelocity` representations of the *same* exact frames (trajectory is
//!   strictly smaller; both baselines decode to the identical sequence);
//! * piecewise-linear motion (move → hold → reverse) with exact holds;
//! * trajectory deactivation after its program's duration (state becomes
//!   stationary; later frames are the unchanged lane);
//! * trajectory/translation state exclusivity on one instance;
//! * hostile work budgets (encoder and parser) and hostile program forms;
//! * the closed-form simulator vs the normative state stepper (two
//!   implementations must agree);
//! * negative controls: wrong parametric hypotheses are rejected, non-
//!   parametric position walks never collapse.

use vole_video::{
    collapse, decoder, demo,
    error::VoleError,
    object::{Object, ObjectId},
    pixel::Canvas,
    state::{Instance, InstanceId, State},
    time::Interval,
    trajectory::{self, TrajectorySegment},
    transition::Transition,
};

#[test]
fn accelerating_box_trajectory_reconstructs_exact_frames() -> Result<(), VoleError> {
    // Flagship (§76 analogue, accelerating): 1920x1080, one 200x100 box whose
    // velocity grows by (1,0) per interval, stored as one trajectory program.
    let court = demo::TrajectoryCourt::default();
    let parsed = decoder::decode_bytes(&court.vole()?)?;
    assert_eq!(parsed.frame_count(), 41);
    let frames = court.materialize_and_verify()?; // byte-exact vs closed form
    assert_eq!(frames.len(), 41);

    // Motion is real and accelerating: interior samples follow the analytic
    // positions x(t) = 100 + 2t + t(t-1)/2, y(t) = 60 + t.
    let first = frames.first().unwrap();
    let last = frames.last().unwrap();
    let positions = court.positions()?;
    let (x40, y40) = positions[40];
    assert_eq!(x40, 960);
    assert_eq!(y40, 100);
    assert_eq!(
        last.get(1000, 140),
        180,
        "box interior at the final position"
    );
    assert_eq!(last.get(80, 60), 0, "box has left its origin region");
    assert_eq!(first.get(150, 100), 180);
    // Frame 10: x = 100 + 20 + 45 = 165, y = 70.
    let f10 = &frames[10];
    assert_eq!(f10.get(220, 110), 180);
    assert_eq!(f10.get(140, 60), 0);
    Ok(())
}

#[test]
fn trajectory_beats_per_frame_representations_of_the_same_frames() -> Result<(), VoleError> {
    let court = demo::TrajectoryCourt::default();
    let traj = court.vole()?;
    let setpos = court.set_position_baseline_bytes()?;
    let vel = court.velocity_baseline_bytes()?;
    assert!(
        traj.len() < setpos.len(),
        "trajectory must beat per-frame SetPosition ({} vs {})",
        traj.len(),
        setpos.len()
    );
    assert!(
        traj.len() < vel.len(),
        "trajectory must beat per-frame velocity rewrites ({} vs {})",
        traj.len(),
        vel.len()
    );
    // All three streams decode to the identical exact sequence.
    let a = decoder::materialize_all(&decoder::decode_bytes(&traj)?)?;
    let b = decoder::materialize_all(&decoder::decode_bytes(&setpos)?)?;
    let c = decoder::materialize_all(&decoder::decode_bytes(&vel)?)?;
    assert_eq!(a, b);
    assert_eq!(a, c);
    // Representation is not raster-proportional.
    let raw_all = court.raw_bytes_all();
    assert!((traj.len() as u64) * 100_000 < raw_all);
    Ok(())
}

#[test]
fn piecewise_motion_with_hold_is_exact() -> Result<(), VoleError> {
    // Move right 4 px/interval for 20, hold for 10, reverse 2 px/interval for
    // 30: a piecewise-linear program with an exact hold.
    let court = demo::TrajectoryCourt {
        width: 320,
        height: 200,
        box_w: 40,
        box_h: 20,
        value: 180,
        x0: 100,
        y0: 60,
        segments: vec![
            TrajectorySegment::Linear {
                vx: 4,
                vy: 0,
                steps: 20,
            },
            TrajectorySegment::Linear {
                vx: 0,
                vy: 0,
                steps: 10,
            },
            TrajectorySegment::Linear {
                vx: -2,
                vy: 0,
                steps: 30,
            },
        ],
        intervals: 60,
        ..demo::TrajectoryCourt::default()
    };
    let frames = court.materialize_and_verify()?;
    assert_eq!(frames.len(), 61);
    let positions = court.positions()?;
    assert_eq!(positions[0], (100, 60));
    assert_eq!(positions[20], (180, 60));
    assert_eq!(positions[30], (180, 60), "hold keeps the position");
    assert_eq!(positions[60], (120, 60));
    // The hold really holds: frames 20..=30 show the box at the same place.
    for f in &frames[20..=30] {
        assert_eq!(f.get(200, 60), 180);
    }
    assert_eq!(frames[40].get(145, 60), 0); // reversed away from x=180
    Ok(())
}

#[test]
fn trajectory_deactivates_after_duration_then_lane_is_static() -> Result<(), VoleError> {
    // Program runs for 8 intervals; the stream continues with 4 empty
    // (unchanged) intervals. After deactivation the position must not move.
    let obj = Object::fill(8, 8, 9)?;
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let mut groups = Vec::new();
    for k in 1..=12u64 {
        if k == 1 {
            groups.push((
                k,
                vec![
                    Transition::SetTrajectory {
                        id: InstanceId(1),
                        segments: vec![TrajectorySegment::Linear {
                            vx: 2,
                            vy: 0,
                            steps: 8,
                        }],
                    },
                    Transition::AdvanceTrajectories,
                ],
            ));
        } else if k <= 8 {
            groups.push((k, vec![Transition::AdvanceTrajectories]));
        } else {
            groups.push((k, Vec::new())); // unchanged lane after the program
        }
    }
    let bytes = vole_video::encoder::encode_stream(64, 16, 0, &[(1, obj)], &[inst], &groups)?;
    let frames = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
    assert_eq!(frames.len(), 13);
    // Frame k has the box at x = 2k (8 samples wide) while the program runs.
    assert_eq!(frames[7].get(14, 4), 9);
    assert_eq!(frames[7].get(13, 4), 0);
    // Deactivated after 8 advances: frames 8..=12 are identical (box at x=16).
    for f in &frames[8..=12] {
        assert_eq!(f.get(16, 4), 9);
        assert_eq!(f.get(15, 4), 0);
    }
    Ok(())
}

#[test]
fn static_hold_trajectory_produces_identical_frames() -> Result<(), VoleError> {
    // A pure zero-velocity Linear segment ("constant" motion: nothing moves)
    // must reproduce identical frames while its advances still run.
    let court = demo::TrajectoryCourt {
        segments: vec![TrajectorySegment::Linear {
            vx: 0,
            vy: 0,
            steps: 20,
        }],
        intervals: 20,
        ..demo::TrajectoryCourt::default()
    };
    let frames = court.materialize_and_verify()?;
    let f0 = frames.first().unwrap();
    assert!(frames.iter().all(|f| f.exactly_matches(f0)));
    Ok(())
}

#[test]
fn trajectory_and_translation_state_are_exclusive() -> Result<(), VoleError> {
    let mut st = State::new(Interval::ZERO);
    st.set_background(0);
    st.declare_object(ObjectId(1), Object::fill(4, 4, 9)?)?;
    st.create_instance(InstanceId(1), ObjectId(1), 0, 0)?;
    let prog = vec![TrajectorySegment::Linear {
        vx: 1,
        vy: 0,
        steps: 5,
    }];

    // Attaching a trajectory clears a translation.
    st.set_velocity(InstanceId(1), 7, -2)?;
    assert_eq!(st.velocity(InstanceId(1)), (7, -2));
    st.set_trajectory(InstanceId(1), prog.clone())?;
    assert_eq!(st.velocity(InstanceId(1)), (0, 0));
    assert!(st.has_trajectory(InstanceId(1)));
    assert_eq!(st.trajectory_count(), 1);

    // Attaching a translation clears a trajectory.
    st.set_velocity(InstanceId(1), 1, 1)?;
    assert!(!st.has_trajectory(InstanceId(1)));

    // An empty program deactivates; unknown instances are typed errors.
    st.set_trajectory(InstanceId(1), prog)?;
    st.set_trajectory(InstanceId(1), Vec::new())?;
    assert!(!st.has_trajectory(InstanceId(1)));
    assert_eq!(
        st.set_trajectory(InstanceId(99), Vec::new()).unwrap_err(),
        VoleError::UnknownInstance
    );
    Ok(())
}

#[test]
fn advancing_steps_the_motion_and_state_stays_consistent() -> Result<(), VoleError> {
    let mut st = State::new(Interval::ZERO);
    st.declare_object(ObjectId(1), Object::fill(4, 4, 9)?)?;
    st.create_instance(InstanceId(1), ObjectId(1), 0, 0)?;
    st.set_trajectory(
        InstanceId(1),
        vec![TrajectorySegment::Accel {
            vx0: 1,
            vy0: 0,
            ax: 1,
            ay: 0,
            steps: 4,
        }],
    )?;
    // Positions accumulate the growing velocity: 1, 3, 6, 10.
    let expect = [(1, 0), (3, 0), (6, 0), (10, 0)];
    for (k, want) in expect.iter().enumerate() {
        st.advance_trajectories()?;
        let inst = st.instance(InstanceId(1))?;
        assert_eq!((inst.x, inst.y), *want, "after advance {}", k + 1);
    }
    // Program exhausted: the trajectory deactivates and further advances are
    // no-ops.
    assert!(!st.has_trajectory(InstanceId(1)));
    st.advance_trajectories()?;
    let inst = st.instance(InstanceId(1))?;
    assert_eq!((inst.x, inst.y), (10, 0));
    Ok(())
}

#[test]
fn closed_form_simulator_agrees_with_the_normative_state_stepper() -> Result<(), VoleError> {
    // Two independent implementations of the discrete trajectory semantics
    // must agree: `trajectory::simulate_positions` (closed-form per segment)
    // against `State::advance_trajectories` (incremental stepper).
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed = seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        seed >> 33
    };
    for trial in 0..200 {
        let mut segments = Vec::new();
        let mut total = 0u64;
        let nseg = 1 + (rnd() % 3);
        for _ in 0..nseg {
            let steps = 1 + rnd() % 6;
            total += steps;
            if rnd() % 2 == 0 {
                segments.push(TrajectorySegment::Linear {
                    vx: (rnd() % 5) as i64 - 2,
                    vy: (rnd() % 3) as i64 - 1,
                    steps,
                });
            } else {
                // Canonical acceleration needs at least one non-zero axis.
                let ax = (rnd() % 5) as i64 - 2;
                let (ax, ay) = if ax == 0 { (0, 1) } else { (ax, 0) };
                segments.push(TrajectorySegment::Accel {
                    vx0: (rnd() % 5) as i64 - 2,
                    vy0: (rnd() % 3) as i64 - 1,
                    ax,
                    ay,
                    steps,
                });
            }
        }
        let (x0, y0) = (100i64, 60i64);
        let closed =
            trajectory::simulate_positions(&segments, x0, y0, total).expect("closed form covers");
        let mut st = State::new(Interval::ZERO);
        st.declare_object(ObjectId(1), Object::fill(4, 4, 9)?)?;
        st.create_instance(InstanceId(1), ObjectId(1), x0, y0)?;
        st.set_trajectory(InstanceId(1), segments)?;
        assert_eq!(closed[0], (x0, y0), "trial {trial}");
        for k in 1..=total {
            st.advance_trajectories()?;
            let inst = st.instance(InstanceId(1))?;
            assert_eq!(
                (inst.x, inst.y),
                closed[k as usize],
                "trial {trial} advance {k}"
            );
        }
    }
    Ok(())
}

#[test]
fn trajectory_work_budget_rejected_by_encoder() {
    // Many trajectory-carrying instances advanced for many intervals exceed
    // the cumulative trajectory-work budget; the encoder must refuse with a
    // typed error quickly (never hang, never serialize a DoS stream).
    let w = 64u32;
    let h = 64u32;
    let obj = Object::fill(2, 2, 7).unwrap();
    let objects = vec![(1u32, obj)];
    let instances: Vec<Instance> = (0..3000u32)
        .map(|i| Instance {
            id: InstanceId(i + 1),
            object_id: ObjectId(1),
            x: i64::from(i % 60),
            y: 0,
        })
        .collect();
    let arm: Vec<Transition> = (0..3000u32)
        .map(|i| Transition::SetTrajectory {
            id: InstanceId(i + 1),
            segments: vec![TrajectorySegment::Linear {
                vx: 1,
                vy: 0,
                steps: 1 << 40,
            }],
        })
        .collect();
    let mut timeline = vec![(1u64, arm)];
    for k in 2..=3_000u64 {
        timeline.push((k, vec![Transition::AdvanceTrajectories]));
    }
    let res = vole_video::encoder::encode_stream(w, h, 0, &objects, &instances, &timeline);
    assert_eq!(res.unwrap_err(), VoleError::MaterializationBudgetExceeded);
}

#[test]
fn trajectory_work_budget_rejected_by_parser() -> Result<(), VoleError> {
    // Hostile-file court: a crafted stream whose cumulative trajectory work
    // exceeds the envelope must be rejected by the *parser* with a typed
    // error, quickly and without hanging.
    let mut wr = vole_video::format::StreamWriter::begin(64, 64);
    wr = wr.declare_object(ObjectId(1), Object::fill(2, 2, 5)?)?;
    let instances: Vec<Instance> = (0..3000u32)
        .map(|i| Instance {
            id: InstanceId(i + 1),
            object_id: ObjectId(1),
            x: i64::from(i % 60),
            y: 0,
        })
        .collect();
    wr = wr.checkpoint_with(&instances)?;
    let arm: Vec<Transition> = (0..3000u32)
        .map(|i| Transition::SetTrajectory {
            id: InstanceId(i + 1),
            segments: vec![TrajectorySegment::Linear {
                vx: 1,
                vy: 0,
                steps: 1 << 40,
            }],
        })
        .collect();
    wr = wr.interval(Interval(1), &arm)?;
    for k in 2..=3_000u64 {
        wr = wr.interval(Interval(k), &[Transition::AdvanceTrajectories])?;
    }
    let bytes = wr.finish()?;
    assert_eq!(
        vole_video::decoder::decode_bytes(&bytes).unwrap_err(),
        VoleError::MaterializationBudgetExceeded
    );
    Ok(())
}

#[test]
fn oversize_trajectory_program_rejected_by_encoder() {
    let obj = Object::fill(4, 4, 9).unwrap();
    let inst = Instance {
        id: InstanceId(1),
        object_id: ObjectId(1),
        x: 0,
        y: 0,
    };
    let mut segments = Vec::new();
    for k in 0..300u32 {
        // Distinct adjacent velocities keep the program otherwise canonical.
        segments.push(TrajectorySegment::Linear {
            vx: i64::from(k % 7) + 1,
            vy: 0,
            steps: 2,
        });
    }
    let res = vole_video::encoder::encode_stream(
        16,
        16,
        0,
        &[(1, obj)],
        &[inst],
        &[(
            1,
            vec![Transition::SetTrajectory {
                id: InstanceId(1),
                segments,
            }],
        )],
    );
    assert_eq!(res.unwrap_err(), VoleError::MaterializationBudgetExceeded);
}

#[test]
fn non_parametric_hypotheses_are_rejected() -> Result<(), VoleError> {
    // Negative control: parametric fits must never claim a sequence that does
    // not follow the model. A random walk fits nothing — any fit that *is*
    // returned must still reproduce the walk exactly when re-simulated; a
    // wrong model is never silently accepted.
    let mut positions: Vec<(i64, i64)> = vec![(100, 60)];
    let mut x = 100i64;
    let mut seed = 0xDEAD_BEEFu64;
    for _ in 0..47 {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        let step = ((seed.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 60) as i64 % 5 - 2;
        x += step;
        positions.push((x, 60));
    }
    if let Some(f) = trajectory::fit_linear(&positions) {
        let got = trajectory::simulate_positions(&[f], 100, 60, 47).unwrap();
        assert_eq!(got, positions);
    }
    if let Some(f) = trajectory::fit_accel(&positions) {
        let got = trajectory::simulate_positions(&[f], 100, 60, 47).unwrap();
        assert_eq!(got, positions);
    }
    if let Some(p) = trajectory::fit_piecewise(&positions) {
        let got = trajectory::simulate_positions(&p, 100, 60, 47).unwrap();
        assert_eq!(got, positions);
    }
    Ok(())
}

#[test]
fn collapse_keeps_raster_encoded_motion_exact_and_smaller() -> Result<(), VoleError> {
    // Raster-origin court: encode an accelerating whole-canvas sprite with the
    // Phase-G encoder (per-frame translation winners), then run the Phase-I
    // collapse. The collapsed stream must decode byte-identically to the
    // input raster and be strictly smaller than the greedy stream.
    let (w, h) = (32u32, 16u32);
    let mut frames: Vec<Canvas> = Vec::new();
    // Constant acceleration: velocity grows by 1 each interval, v0 = 1, six
    // motion frames; the box stays inside the canvas.
    let mut x = 4i64;
    frames.push(paint_box(w, h, 90, 4, 6, 6, 4, 180));
    for v in 1i64..=6 {
        x += v;
        frames.push(paint_box(w, h, 90, x, 6, 6, 4, 180));
    }
    let report = vole_video::inverse::encode_frames(
        &frames,
        &vole_video::inverse::EncodeOptions {
            bg_sweep: false,
            background: Some(90),
            translation_window: 6,
            ..vole_video::inverse::EncodeOptions::default()
        },
    )?;
    assert!(report.exact);
    assert!(
        report
            .decisions
            .iter()
            .all(|d| d.frame == 0 || d.winner_family == "translation"),
        "the greedy encoder must see the motion as per-frame translations"
    );
    let improved = collapse::collapse_fixpoint(report.vole.clone())?;
    assert!(
        improved.len() < report.vole.len(),
        "collapse must strictly shrink the stream ({} vs {})",
        improved.len(),
        report.vole.len()
    );
    // Normative decode of the collapsed stream equals the input raster.
    let frames_out = decoder::materialize_all(&decoder::decode_bytes(&improved)?)?;
    assert_eq!(frames_out.len(), frames.len());
    for (a, b) in frames_out.iter().zip(frames.iter()) {
        assert_eq!(a, b);
    }
    Ok(())
}

#[test]
fn raster_noise_never_collapses() -> Result<(), VoleError> {
    // Negative control on the full path: noise rasters are RAW frames, there
    // is no SetPosition run to collapse, and the stream is a fixpoint.
    let (w, h) = (24u32, 16u32);
    let mut seed = 0x0BAD_F00Du64;
    let mut frames = Vec::new();
    for _ in 0..10 {
        let mut d = Vec::with_capacity((w * h) as usize);
        for _ in 0..(w * h) {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            d.push((seed >> 56) as u8);
        }
        frames.push(Canvas::from_parts(w, h, d)?);
    }
    let report = vole_video::inverse::encode_frames(
        &frames,
        &vole_video::inverse::EncodeOptions {
            bg_sweep: false,
            ..vole_video::inverse::EncodeOptions::default()
        },
    )?;
    assert!(report.exact);
    assert!(collapse::collapse_stream(&report.vole)?.is_none());
    Ok(())
}

/// Reference painter helper (independent of the materializer internals).
#[allow(clippy::too_many_arguments)] // geometry tuple kept inline for clarity
fn paint_box(w: u32, h: u32, bg: u8, bx: i64, by: i64, bw: u32, bh: u32, value: u8) -> Canvas {
    let mut data = vec![bg; (w * h) as usize];
    for dy in 0..bh as i64 {
        for dx in 0..bw as i64 {
            let x = bx + dx;
            let y = by + dy;
            if x >= 0 && y >= 0 && x < i64::from(w) && y < i64::from(h) {
                data[y as usize * w as usize + x as usize] = value;
            }
        }
    }
    Canvas::from_parts(w, h, data).expect("canvas")
}
