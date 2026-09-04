//! Exact raster-origin ingest floor for the multi-plane core — Phase V.1.2.
//!
//! [`encode_pictures_exact`] turns an observed sequence of canonical
//! [`Picture`]s (one epoch's plane table) into a [`MultiPlaneProgram`] that
//! reproduces them **exactly**, per plane and independently (§46):
//!
//! * frame 0: a uniform plane becomes the background; otherwise the whole
//!   plane is declared as one immutable raster object (the RAW floor);
//! * every later observation is an **aligned interval group per plane**: a
//!   plane whose observation equals its committed state render emits an empty
//!   group (the render already is the observation); otherwise it emits either
//!   a strict-sorted sparse residual of the samples that differ from the
//!   state render, or a full content replacement (fresh object id + clear +
//!   create) — whichever has the lower estimated complete bytes over the
//!   observed identical run (a residual is a one-shot canvas op over the
//!   fresh state render, so once content settles to a repeat, one state sync
//!   plus empty groups beats re-emitting the same residual per frame);
//! * the result is **proven**: every observation is materialized back through
//!   [`MultiPlaneProgram::materialize_observation`] and compared
//!   sample-for-sample with the target before the program is returned.
//!
//! Residual basis: the v2 core mirrors v1 replay semantics exactly — every
//! interval renders the **persistent state** (background + instances +
//! overlay) fresh and applies that interval's canvas ops (COPY/RESIDUAL)
//! over the render; canvas ops never persist into later frames. A residual
//! therefore describes the target relative to the *committed state render*,
//! which only content replacements change. [`encode_pictures_exact`] tracks
//! that committed render per plane, so every emitted residual is exact by
//! construction (not merely by final proof).
//!
//! This is deliberately a correctness-first floor (RAW / state / residual).
//! The generalized inverse *search* over the deeper families (translation,
//! regions, generators, …) is V.1.4+ work per the brief's §247 ordering;
//! V.1.2's mandate is the exact multiplane core and its specialization
//! proof, and RAW/state/residual already give a universal exact floor at any
//! layout and depth.

use crate::error::VoleError;
use crate::media::core::{
    encode_plane_residual, MultiPlaneProgram, PlaneInstance, PlaneInstanceId, PlaneObject,
    PlaneObjectId, PlaneOp, PlaneProgram,
};
use crate::media::epoch::VideoEpoch;
use crate::media::picture::Picture;
use crate::media::plane::{BitDepth, Plane, PlaneData};

/// Deterministic hash (test/authoring helper).
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Exact raster-origin ingest floor over an observed picture sequence.
///
/// `observations` must all validate against `epoch` and share its plane
/// table. The returned program materializes exactly the observed sequence:
/// every observation is re-materialized and compared sample-for-sample (per
/// plane) before the program is returned, or a typed error is returned
/// instead of a program.
pub fn encode_pictures_exact(
    epoch: &VideoEpoch,
    observations: &[Picture],
) -> Result<MultiPlaneProgram, VoleError> {
    if observations.is_empty() {
        return Err(VoleError::ApiConstraint(
            "exact ingest needs at least one observation",
        ));
    }
    for pic in observations {
        pic.validate_against(epoch)?;
    }
    let n_obs = observations.len() as u64;

    // Phase 1: frame-0 state per plane (background, or a whole-plane RAW
    // raster object as the initial content), and the committed state render
    // every later interval's canvas ops will be applied over.
    let mut planes: Vec<PlaneProgram> = Vec::with_capacity(epoch.plane_count());
    let mut state: Vec<Plane> = Vec::with_capacity(epoch.plane_count());
    for p in 0..epoch.plane_count() {
        let depth = epoch.planes()[p].bit_depth;
        let (pw, ph) = epoch.plane_dimensions(p)?;
        let bg = observations[0].get(p, 0, 0).ok_or(VoleError::OutOfBounds)?;
        let mut prog = PlaneProgram::new(bg);
        // Frame 0: uniform -> the background value; otherwise a whole-plane
        // RAW raster object as the initial state.
        if plane_uniform(&observations[0], p).is_none() {
            let samples = plane_samples(&observations[0], p, pw, ph);
            let obj = PlaneObject::raster(pw, ph, depth, &samples)?;
            prog.objects.insert(PlaneObjectId(1), obj);
            prog.instances.push(PlaneInstance {
                id: PlaneInstanceId(1),
                object: PlaneObjectId(1),
                x: 0,
                y: 0,
            });
        }
        state.push(observations[0].plane(p).expect("validated").clone());
        planes.push(prog);
    }
    // First free object id per plane: frame 0 uses id 1 only when it created
    // a raster object; starts at 2 so ids are never reused either way.
    let mut obj_counters: Vec<u32> = vec![2; epoch.plane_count()];

    // Phase 2: one aligned interval group per plane per later observation.
    for t in 1..n_obs {
        for p in 0..epoch.plane_count() {
            let depth = epoch.planes()[p].bit_depth;
            let (pw, ph) = epoch.plane_dimensions(p)?;
            let target = observations[t as usize].plane(p).expect("validated");
            let base = &state[p];
            // Unchanged against the committed state render: an empty group is
            // exact (the interval's fresh render already equals the target).
            if base.canonical_bytes() == target.canonical_bytes() {
                planes[p].intervals.push((t, Vec::new()));
                continue;
            }
            // Changed: strict-sorted sparse residual over the fresh state
            // render, or a full content replacement (cheapest exact
            // description of the drift from the committed render). Count the
            // drift first so the decision never needs a full point list.
            let mut changed = 0u64;
            for y in 0..ph {
                for x in 0..pw {
                    if plane_sample(base.data(), pw, y, x) != plane_sample(target.data(), pw, y, x)
                    {
                        changed += 1;
                    }
                }
            }
            let plane_bytes = u64::from(pw) * u64::from(ph) * depth.storage().bytes_per_sample();
            let mut residual_est = changed * 10 + 64;
            let replace_est = plane_bytes + 90;
            // Static-run economics (both candidate descriptions are exact):
            // when the following observations repeat this one, a residual
            // would be re-emitted per frame (canvas ops are one-shot over the
            // fresh state render), while one state sync (replacement) lets
            // the repeats ride the empty-group lane at 12 B per plane per
            // observation. The cheaper description over the observed run wins
            // (complete-byte cost, never a lookahead-dependent semantic).
            if residual_est < replace_est {
                let target_bytes = target.canonical_bytes();
                let mut run = 1u64;
                while t + run < n_obs
                    && observations[(t + run) as usize]
                        .plane(p)
                        .expect("validated")
                        .canonical_bytes()
                        == target_bytes
                {
                    run += 1;
                }
                if run > 1 {
                    let run_residual = residual_est * run;
                    let run_replace = replace_est + 12 * (run - 1);
                    if run_replace < run_residual {
                        residual_est = u64::MAX; // force the replacement
                    }
                }
            }
            let mut ops: Vec<PlaneOp> = Vec::new();
            if residual_est < replace_est {
                let mut points: Vec<(i32, i32, u16)> = Vec::with_capacity(changed as usize);
                for y in 0..ph {
                    for x in 0..pw {
                        let a = plane_sample(base.data(), pw, y, x);
                        let b = plane_sample(target.data(), pw, y, x);
                        if a != b {
                            points.push((x as i32, y as i32, b as u16));
                        }
                    }
                }
                // The scan is row-major; the residual grammar is
                // strict-ascending by (x, y), so sort before encoding.
                points.sort_unstable_by_key(|&(x, y, _)| (x, y));
                ops.push(PlaneOp::Residual {
                    block: encode_plane_residual(&points)?,
                });
            } else {
                let samples = plane_samples(&observations[t as usize], p, pw, ph);
                let id = obj_counters[p];
                ops.push(PlaneOp::DeclareObject {
                    id: PlaneObjectId(id),
                    object: PlaneObject::raster(pw, ph, depth, &samples)?,
                });
                ops.push(PlaneOp::ClearInstances);
                ops.push(PlaneOp::CreateInstance {
                    id: PlaneInstanceId(1),
                    object: PlaneObjectId(id),
                    x: 0,
                    y: 0,
                });
                obj_counters[p] += 1;
                // The committed state render is now exactly this observation.
                state[p] = target.clone();
            }
            planes[p].intervals.push((t, ops));
        }
    }

    let program = MultiPlaneProgram::new(epoch.clone(), planes)?;
    if program.observation_count() != n_obs {
        return Err(VoleError::ApiConstraint(
            "exact ingest produced the wrong observation count",
        ));
    }
    // Proof: materialize every observation and compare sample-for-sample.
    for (idx, want) in observations.iter().enumerate() {
        let got = program.materialize_observation(idx as u64)?;
        for p in 0..epoch.plane_count() {
            if got.plane(p).expect("plane").canonical_bytes()
                != want.plane(p).expect("plane").canonical_bytes()
            {
                return Err(VoleError::ApiConstraint(
                    "exact ingest floor failed its materialization proof",
                ));
            }
        }
    }
    Ok(program)
}

/// Sample `(x, y)` of a tight row-major payload of width `w`.
fn plane_sample(data: &PlaneData, w: u32, y: u32, x: u32) -> u32 {
    let k = (y * w + x) as usize;
    match data {
        PlaneData::U8(v) => u32::from(v[k]),
        PlaneData::U16(v) => u32::from(v[k]),
    }
}

/// Whether a picture is uniform on one plane.
fn plane_uniform(pic: &Picture, plane: usize) -> Option<u32> {
    let w = pic.plane(plane)?.width();
    let h = pic.plane(plane)?.height();
    let v0 = pic.get(plane, 0, 0)?;
    for y in 0..h {
        for x in 0..w {
            if pic.get(plane, x, y) != Some(v0) {
                return None;
            }
        }
    }
    Some(v0)
}

/// The u32-domain sample row-major content of one plane of a picture.
fn plane_samples(pic: &Picture, plane: usize, w: u32, h: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            out.push(pic.get(plane, x, y).expect("in bounds"));
        }
    }
    out
}

/// A picture with every plane uniformly filled with its value (authored
/// courts).
pub fn uniform_picture(epoch: &VideoEpoch, values: &[u32]) -> Result<Picture, VoleError> {
    Picture::from_epoch(epoch, values)
}

/// A deterministic per-plane sample ramp (within each plane's active depth,
/// with a small deterministic dither) for raster-origin courts.
pub fn ramp_picture(epoch: &VideoEpoch, seed: u64) -> Result<Picture, VoleError> {
    let mut planes = Vec::with_capacity(epoch.plane_count());
    for i in 0..epoch.plane_count() {
        let (pw, ph) = epoch.plane_dimensions(i)?;
        let depth: BitDepth = epoch.planes()[i].bit_depth;
        let max = depth.max_sample();
        let n = (pw * ph) as usize;
        let span = u32::max(pw, 2) - 1;
        let mut values = Vec::with_capacity(n);
        for k in 0..n {
            let col = k as u32 % pw;
            let base = (u64::from(col) * u64::from(max) / u64::from(span)) as u32;
            let dither = (mix(seed ^ k as u64) % 5) as u32;
            values.push((base + dither).min(max));
        }
        let data = match depth.storage() {
            crate::media::plane::PlaneStorage::U8 => {
                if values.iter().any(|v| *v > max) {
                    return Err(VoleError::InvalidSamples);
                }
                PlaneData::U8(values.iter().map(|v| *v as u8).collect())
            }
            crate::media::plane::PlaneStorage::U16 => {
                PlaneData::U16(values.iter().map(|v| *v as u16).collect())
            }
        };
        planes.push(Plane::new(
            epoch.planes()[i].component,
            pw,
            ph,
            depth,
            epoch.planes()[i].subsample_x,
            epoch.planes()[i].subsample_y,
            data,
        )?);
    }
    Picture::from_planes(epoch, planes)
}
