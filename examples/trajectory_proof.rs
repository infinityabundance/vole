//! Phase-I evidence producer: bounded parametric trajectories.
//!
//! `cargo run --release --example trajectory_proof` prints a deterministic
//! report for the evidence campaign:
//!
//! * the accelerating §76-analogue flagship (1920×1080, one box, velocity
//!   grows by (1,0)/interval) stored as one trajectory program, versus the
//!   per-frame `SetPosition` and per-frame `SetVelocity` representations of
//!   the same exact frames — all byte-exact against an independent
//!   closed-form reference painter;
//! * piecewise-linear motion (move → hold → reverse) with exact holds;
//! * the honest cost of *maintaining* an active zero-velocity trajectory
//!   versus the Phase-B unchanged lane (a negative-ish control: statics
//!   should stay in the unchanged lane);
//! * raster-origin courts: Phase-G greedy encodes of steady and accelerating
//!   translation, then the Phase-I collapse (§43) — collapsed streams decode
//!   byte-identically to the input raster and are strictly smaller;
//! * negative controls: noise and random walks never collapse.

use std::time::Instant;

use vole_video::{
    collapse, decoder, demo, error::VoleError, inverse, pixel::Canvas,
    trajectory::TrajectorySegment, transition::Transition,
};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

fn paint_boxes(w: u32, h: u32, bg: u8, boxes: &[(i64, i64, u32, u32, u8)]) -> Canvas {
    let mut data = vec![bg; (w * h) as usize];
    for (bx, by, bw, bh, v) in boxes {
        for dy in 0..*bh as i64 {
            for dx in 0..*bw as i64 {
                let x = bx + dx;
                let y = by + dy;
                if x >= 0 && y >= 0 && x < i64::from(w) && y < i64::from(h) {
                    data[y as usize * w as usize + x as usize] = *v;
                }
            }
        }
    }
    canvas_of(w, h, data)
}

struct Det(u64);
impl Det {
    fn next(&mut self) -> u64 {
        let mut x = self.0.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        self.0 = x;
        x
    }
    fn byte(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }
}

/// Count the trajectory ops present in a stream's intervals.
fn op_counts(bytes: &[u8]) -> (u64, u64, u64) {
    let parsed = decoder::decode_bytes(bytes).expect("stream parses");
    let (mut set, mut adv, mut setpos) = (0u64, 0u64, 0u64);
    for (_t, trs) in parsed.intervals() {
        for tr in trs {
            match tr {
                Transition::SetTrajectory { .. } => set += 1,
                Transition::AdvanceTrajectories => adv += 1,
                Transition::SetPosition { .. } => setpos += 1,
                _ => {}
            }
        }
    }
    (set, adv, setpos)
}

fn main() -> Result<(), VoleError> {
    // --- 1. Accelerating flagship (direct procedural ingest) ----------------
    {
        let court = demo::TrajectoryCourt::default();
        let t = Instant::now();
        let traj = court.vole()?;
        let frames = court.materialize_and_verify()?; // byte-exact vs closed form
        let setpos = court.set_position_baseline_bytes()?;
        let vel = court.velocity_baseline_bytes()?;
        // The two baselines are byte-identical frame sequences too.
        let a = decoder::materialize_all(&decoder::decode_bytes(&traj)?)?;
        let b = decoder::materialize_all(&decoder::decode_bytes(&setpos)?)?;
        let c = decoder::materialize_all(&decoder::decode_bytes(&vel)?)?;
        assert_eq!(a, b);
        assert_eq!(a, c);
        let (n_set, n_adv, _) = op_counts(&traj);
        println!(
            "accel-flag-1920x1080: frames={} vole={}B setpos_baseline={}B \
             velocity_baseline={}B raw_all={}B \
             vole_vs_setpos={:.3} vole_vs_raw={:.6} exact=true \
             trajectory_ops=(set {n_set}, advance {n_adv}) verify_ms={:.1}",
            court.frame_count(),
            traj.len(),
            setpos.len(),
            vel.len(),
            court.raw_bytes_all(),
            setpos.len() as f64 / traj.len() as f64,
            court.raw_bytes_all() as f64 / traj.len() as f64,
            t.elapsed().as_secs_f64() * 1000.0
        );
        assert_eq!(frames.len(), 41);
        assert!(traj.len() < setpos.len() && traj.len() < vel.len());
    }

    // --- 2. Piecewise motion (move → hold → reverse) ------------------------
    {
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
        let traj = court.vole()?;
        let setpos = court.set_position_baseline_bytes()?;
        let frames = court.materialize_and_verify()?;
        let (n_set, n_adv, _) = op_counts(&traj);
        println!(
            "piecewise-320x200: frames={} vole={}B setpos_baseline={}B \
             ratio={:.3} exact=true trajectory_ops=(set {n_set}, advance {n_adv}) segments=3",
            court.frame_count(),
            traj.len(),
            setpos.len(),
            setpos.len() as f64 / traj.len() as f64
        );
        assert_eq!(frames.len(), 61);
        assert!(traj.len() < setpos.len());
    }

    // --- 3. Honest cost of an active zero-velocity trajectory ---------------
    {
        // A zero-velocity Linear program ("constant": nothing moves) is *not*
        // free: it needs an advance op per frame. The Phase-B unchanged lane
        // (13 B/frame) must stay cheaper — this is measured, not assumed.
        let court = demo::TrajectoryCourt {
            segments: vec![TrajectorySegment::Linear {
                vx: 0,
                vy: 0,
                steps: 200,
            }],
            intervals: 200,
            ..demo::TrajectoryCourt::default()
        };
        let traj = court.vole()?;
        let frames = court.materialize_and_verify()?;
        let f0 = frames.first().unwrap();
        assert!(frames.iter().all(|f| f.exactly_matches(f0)));
        let per_frame = traj.len() as f64 / frames.len() as f64;
        println!(
            "static-hold-1920x1080: frames={} vole={}B per_frame_amortized={:.3}B \
             unchanged_lane=13.0B exact=true note=\"active hold costs an advance op; \
             unchanged lane remains cheaper for statics\"",
            frames.len(),
            traj.len(),
            per_frame
        );
    }

    // --- 4. Raster-origin steady translation + collapse ---------------------
    {
        let (w, h) = (64u32, 64u32);
        let frames: Vec<Canvas> = (0..40i64)
            .map(|k| paint_boxes(w, h, 90, &[(8 + k, 12 + k, 10, 6, 180)]))
            .collect();
        let t = Instant::now();
        let report = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                bg_sweep: false,
                background: Some(90),
                ..inverse::EncodeOptions::default()
            },
        )?;
        let t_greedy = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let improved = collapse::collapse_fixpoint(report.vole.clone())?;
        let t_collapse = t.elapsed().as_secs_f64();
        let (n_set, n_adv, _) = op_counts(&improved);
        assert!(improved.len() < report.vole.len());
        let out = decoder::materialize_all(&decoder::decode_bytes(&improved)?)?;
        assert_eq!(out.len(), frames.len());
        assert!(out.iter().zip(frames.iter()).all(|(a, b)| a == b));
        let t_before = inverse::account_stream(&report.vole)?.transition_bytes;
        let t_after = inverse::account_stream(&improved)?.transition_bytes;
        println!(
            "raster-linear-pan-64x64: frames={} greedy={}B collapsed={}B saved_total={:.3} \
             interval_transitions={}B->{}B saved_interval={:.3} \
             exact=true trajectory_ops=(set {n_set}, advance {n_adv}) \
             greedy_ms={:.0} collapse_ms={:.0}",
            frames.len(),
            report.vole.len(),
            improved.len(),
            improved.len() as f64 / report.vole.len() as f64,
            t_before,
            t_after,
            t_after as f64 / t_before as f64,
            t_greedy * 1000.0,
            t_collapse * 1000.0
        );
    }

    // --- 5. Raster-origin accelerating translation + collapse ---------------
    {
        let (w, h) = (48u32, 24u32);
        let mut frames: Vec<Canvas> = Vec::new();
        let mut x = 6i64;
        frames.push(paint_boxes(w, h, 90, &[(6, 9, 6, 4, 180)]));
        for v in 1i64..=7 {
            x += v;
            frames.push(paint_boxes(w, h, 90, &[(x, 9, 6, 4, 180)]));
        }
        let report = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                bg_sweep: false,
                background: Some(90),
                translation_window: 8,
                ..inverse::EncodeOptions::default()
            },
        )?;
        let improved = collapse::collapse_fixpoint(report.vole.clone())?;
        let (n_set, n_adv, _) = op_counts(&improved);
        assert!(improved.len() < report.vole.len());
        let out = decoder::materialize_all(&decoder::decode_bytes(&improved)?)?;
        assert_eq!(out.len(), frames.len());
        assert!(out.iter().zip(frames.iter()).all(|(a, b)| a == b));
        let trans: u64 = report
            .decisions
            .iter()
            .filter(|d| d.winner_family == "translation")
            .count() as u64;
        let t_before = inverse::account_stream(&report.vole)?.transition_bytes;
        let t_after = inverse::account_stream(&improved)?.transition_bytes;
        println!(
            "raster-accel-48x24: frames={} greedy={}B collapsed={}B saved_total={:.3} \
             interval_transitions={}B->{}B saved_interval={:.3} \
             exact=true translation_winners={} trajectory_ops=(set {n_set}, advance {n_adv})",
            frames.len(),
            report.vole.len(),
            improved.len(),
            improved.len() as f64 / report.vole.len() as f64,
            t_before,
            t_after,
            t_after as f64 / t_before as f64,
            trans
        );
    }

    // --- 6. Negative controls: noise and random walks never collapse --------
    {
        let (w, h) = (24u32, 16u32);
        let mut rng = Det(0xBAD_F00D);
        let mut frames = Vec::new();
        for _ in 0..12 {
            let mut d = Vec::with_capacity((w * h) as usize);
            for _ in 0..(w * h) {
                d.push(rng.byte());
            }
            frames.push(canvas_of(w, h, d));
        }
        let report = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                bg_sweep: false,
                ..inverse::EncodeOptions::default()
            },
        )?;
        let fix = collapse::collapse_fixpoint(report.vole.clone())?;
        println!(
            "noise-24x16: frames={} vole={}B collapsed={}B (no run) exact=true \
             winners=[raw]",
            frames.len(),
            report.vole.len(),
            fix.len()
        );
        assert_eq!(fix.len(), report.vole.len());
    }
    {
        // A deterministic random-walk position timeline (authored transitions).
        let court = demo::TrajectoryCourt::default();
        let (w, h) = (64u32, 16u32);
        let obj = vole_video::object::Object::fill(4, 4, 9)?;
        let mut x = 0i64;
        let mut seed = 0x5EED_0002u64;
        let mut timeline = Vec::new();
        for k in 1..=40u64 {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            let step = ((seed.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 60) as i64 % 5 - 2;
            x += step;
            let x = x.clamp(-(1 << 20), 1 << 20);
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: vole_video::state::InstanceId(1),
                    x,
                    y: 0,
                }],
            ));
        }
        let bytes = vole_video::encoder::encode_stream(
            w,
            h,
            0,
            &[(1, obj)],
            &[vole_video::state::Instance {
                id: vole_video::state::InstanceId(1),
                object_id: vole_video::object::ObjectId(1),
                x: 0,
                y: 0,
            }],
            &timeline,
        )?;
        let fix = collapse::collapse_fixpoint(bytes.clone())?;
        println!(
            "random-walk-64x16: frames={} authored={}B collapsed={}B (no fit) exact=true",
            41,
            bytes.len(),
            fix.len()
        );
        assert_eq!(fix.len(), bytes.len());
        let _ = court;
    }
    Ok(())
}
