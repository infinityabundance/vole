//! Per-plane family encoder — Phase V.1.4 + V.1.5 (V.1 video programme, brief
//! §44–§50, §61–§63, §247–§248: the sealed v1 representation families
//! generalized to the canonical multiplane domain over the V.1.2 exact
//! raster-origin floor, plus the global-video-structure proposals).
//!
//! [`encode_pictures_families`] turns an observed sequence of canonical
//! [`Picture`]s (one epoch's plane table) into a [`MultiPlaneProgram`] that
//! reproduces them **exactly**, plane by plane and independently (§46). For
//! every observation the encoder proposes a bounded set of **family
//! candidates** per plane — each a complete, valid, exact interval program —
//! and picks the cheapest under the complete interval-byte cost `J_B` (§94)
//! with a deterministic tie order (the [`FAMILY_ORDER`] list). Every candidate
//! class mirrors a sealed v1 family generalized to the plane's sample domain:
//!
//! * `unchanged` — the observation equals the committed state render (empty
//!   interval group);
//! * `fill` / `raw` — a uniform / raster whole-plane content replacement
//!   (state sync; the V.1.2 floor's background/RAW machinery);
//! * `sparse` — a strict-sorted residual of the samples differing from the
//!   committed render;
//! * `transform` — a Phase-M 4×4 lifting-DCT transform residual over the same
//!   basis (V.1.4 op `0x31`);
//! * `translation` / `copy` / `regions` — rectangular content reused from the
//!   immediately previous observation at an integer displacement (`CopyRect`;
//!   the changed area is decomposed into connected-component boxes), with an
//!   exact sparse remainder;
//! * `palette` — content whose samples come from a small value set is
//!   declared as palette-index content + palette state (a replacement);
//! * `generator` — content an exact depth-aware program fit reproduces
//!   (gradient / checker) is declared as generator content (a replacement);
//! * `exact` — a target equal to an already-declared whole-plane object reuses
//!   it without re-declaring (replacement by re-instantiation).
//!
//! V.1.5 (brief §61–§63, §248) adds the **global-motion classes**: the whole
//! plane predicts from the immediately previous observation through a
//! canonical fixed-point map (`GlobalPredict`, feature bit `0x2`) proposed by
//! a deterministic translation / rotzoom / affine estimator
//! ([`crate::media::global::estimate_global`] — f64 is permitted there and
//! only there), quantized at the map precision whose exact normative
//! simulation prices least ([`MapShift`] registry, brief §62 court), and
//! closed by an exact sparse/transform residual. The proposal never has
//! authority: only the normative materialization + complete-byte cost
//! decides, and the chosen per-record precision is measured in the report.
//!
//! Candidates that do not change the committed state render (the residual and
//! copy classes) are one-shot canvas ops — mirroring v1, they never persist —
//! so a settled run after drift is served by one state sync + empty groups
//! (static-run economics, as in the V.1.2 floor). The encoder **proves** its
//! output: every observation is re-materialized through
//! [`MultiPlaneProgram::materialize_observation`] and compared sample-for-
//! sample before the program is returned.
//!
//! This is deliberately *not* the full hierarchical candidate DAG of §92–§93
//! (that is V.1.11, with DSFB governance in V.1.12): search here is a bounded,
//! deterministic per-family proposal over the plane domain with exact local
//! cost evaluation, and the RAW/SPARSE sentinels always stay alive (§110).
//! Trajectory *promotion over time* (temporal-span search) is later-subphase
//! work; the trajectory/affine semantics themselves are sealed in
//! [`crate::media::core`] and courted separately. Global-motion *decoder
//! semantics* are sealed here and in [`crate::media::global`]; the V.1.5
//! encoder estimates per plane (chroma planes on their own subsampled grids).

use std::collections::BTreeMap;

use crate::error::VoleError;
use crate::media::core::{
    encode_plane_residual, encode_plane_transform_block, MultiPlaneProgram, PlaneContent,
    PlaneInstance, PlaneInstanceId, PlaneObject, PlaneObjectId, PlaneOp, PlanePaletteId,
    PlaneProgram,
};
use crate::media::epoch::VideoEpoch;
use crate::media::gen::Gen;
use crate::media::global::{
    estimate_global, match_tolerance, GlobalHypothesis, GlobalMap, MapShift, MotionClass,
};
use crate::media::picture::Picture;
use crate::media::plane::{BitDepth, Plane, PlaneData};

/// Static family labels (evidence / courts / reports).
pub const FAMILY_UNCHANGED: &str = "unchanged";
pub const FAMILY_FILL: &str = "fill";
pub const FAMILY_RAW: &str = "raw";
pub const FAMILY_SPARSE: &str = "sparse";
pub const FAMILY_TRANSFORM: &str = "transform";
pub const FAMILY_COPY: &str = "copy";
pub const FAMILY_REGIONS: &str = "regions";
pub const FAMILY_TRANSLATION: &str = "translation";
pub const FAMILY_PALETTE: &str = "palette";
pub const FAMILY_GENERATOR: &str = "generator";
pub const FAMILY_EXACT: &str = "exact";
/// V.1.5 global-motion family labels (whole-plane prediction from the
/// previous observation through a canonical map).
pub const FAMILY_GLOBAL_TRANSLATION: &str = "global_translation";
pub const FAMILY_GLOBAL_ROTZOOM: &str = "global_rotzoom";
pub const FAMILY_GLOBAL_AFFINE: &str = "global_affine";

/// Deterministic family evaluation order (also the tie order).
const FAMILY_ORDER: [&str; 14] = [
    FAMILY_UNCHANGED,
    FAMILY_FILL,
    FAMILY_RAW,
    FAMILY_EXACT,
    FAMILY_PALETTE,
    FAMILY_GENERATOR,
    FAMILY_GLOBAL_TRANSLATION,
    FAMILY_GLOBAL_ROTZOOM,
    FAMILY_GLOBAL_AFFINE,
    FAMILY_SPARSE,
    FAMILY_TRANSFORM,
    FAMILY_TRANSLATION,
    FAMILY_COPY,
    FAMILY_REGIONS,
];

/// Largest value set a PALETTE candidate may propose.
const PALETTE_DETECT_MAX: usize = 16;
/// Displacement search window (± `DISPLACEMENT_WINDOW` per axis).
const DISPLACEMENT_WINDOW: i64 = 64;
/// Maximum region components one REGIONS candidate may copy.
const MAX_REGION_BOXES: usize = 4;
/// Deterministic work cap for one interval's displacement search.
const SEARCH_VERIFY_BUDGET: u64 = 1 << 22;
/// A copy candidate is skipped when the drift covers more than this share of
/// the plane (the sentinels win such intervals; measured, not hidden).
const DRIFT_SKIP_SHARE: u64 = 85;
/// A copy search runs only when the sparse drift is at least this many points
/// (small drift never pays a copy descriptor).
const COPY_MIN_SPARSE: usize = 25;

/// Per-family accounting of one encoded video (per-plane observations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FamilyTotals {
    /// How many per-plane observations chose this family.
    pub observations: u64,
    /// Total interval-group bytes attributed to this family (ops + payloads;
    /// shared file-container costs are excluded — see the report docs).
    pub interval_bytes: u64,
}

/// The complete accounting of one [`encode_pictures_families`] run.
#[derive(Debug, Clone)]
pub struct EncodeReport {
    /// Per-family totals across every plane.
    pub families: BTreeMap<&'static str, FamilyTotals>,
    /// Total interval-group bytes across all planes and observations.
    pub total_interval_bytes: u64,
    /// Family-candidate evaluations across the whole run.
    pub candidate_evaluations: u64,
    /// Sample comparisons during displacement search (search work).
    pub search_work: u64,
    /// How many chosen candidates synced the committed state render.
    pub state_syncs: u64,
    /// The bytes a RAW whole-plane replacement would have spent per interval
    /// (the honest floor reference; measured, not hidden).
    pub raw_floor_bytes: u64,
    /// V.1.5: how many intervals chose each map precision (registry code →
    /// observations) — the §62 court's measured outcome, per run.
    pub map_shift_observations: BTreeMap<u8, u64>,
    /// V.1.5: interval bytes chosen at each map precision (registry code →
    /// bytes) — never assumed, always measured.
    pub map_shift_bytes: BTreeMap<u8, u64>,
}

impl EncodeReport {
    /// Total of the per-family observation counters.
    pub fn observations(&self) -> u64 {
        self.families.values().map(|f| f.observations).sum()
    }

    /// Sum of the per-family interval bytes (equals `total_interval_bytes`).
    pub fn family_bytes_sum(&self) -> u64 {
        self.families.values().map(|f| f.interval_bytes).sum()
    }
}

/// Options of one family-encode run (V.1.5 ablation + precision court).
#[derive(Debug, Clone, Copy, Default)]
pub struct EncodeOptions {
    /// Force every global-motion record to this map precision (the §62 court:
    /// encode the same footage at Q8/Q12/Q16 and measure). `None` = each
    /// record is priced at every registry precision and the cheapest wins
    /// (ties prefer the lower precision).
    pub map_shift: Option<MapShift>,
    /// Disable the global-motion family entirely (family ablation court: the
    /// same footage with and without the V.1.5 classes).
    pub disable_global: bool,
}

// ---------------------------------------------------------------------------
// Sample-domain helpers
// ---------------------------------------------------------------------------

fn sample_at(data: &PlaneData, w: u32, y: u32, x: u32) -> u32 {
    let k = (y * w + x) as usize;
    match data {
        PlaneData::U8(v) => u32::from(v[k]),
        PlaneData::U16(v) => u32::from(v[k]),
    }
}

fn sample_vec(data: &PlaneData) -> Vec<u32> {
    match data {
        PlaneData::U8(v) => v.iter().map(|s| u32::from(*s)).collect(),
        PlaneData::U16(v) => v.iter().map(|s| u32::from(*s)).collect(),
    }
}

fn vec_to_data(values: &[u32], depth: BitDepth) -> Result<PlaneData, VoleError> {
    let max = depth.max_sample();
    match depth.storage() {
        crate::media::plane::PlaneStorage::U8 => {
            if values.iter().any(|v| *v > max) {
                return Err(VoleError::InvalidSamples);
            }
            Ok(PlaneData::U8(values.iter().map(|v| *v as u8).collect()))
        }
        crate::media::plane::PlaneStorage::U16 => {
            if values.iter().any(|v| *v > max) {
                return Err(VoleError::InvalidSamples);
            }
            Ok(PlaneData::U16(values.iter().map(|v| *v as u16).collect()))
        }
    }
}

fn is_uniform(data: &PlaneData, w: u32, h: u32, v: u32) -> bool {
    let n = (u64::from(w) * u64::from(h)) as usize;
    match data {
        PlaneData::U8(s) => s.len() == n && s.iter().all(|x| u32::from(*x) == v),
        PlaneData::U16(s) => s.len() == n && s.iter().all(|x| u32::from(*x) == v),
    }
}

// ---------------------------------------------------------------------------
// Detection helpers (exact fits; every proposal is fully verified before it
// may be emitted)
// ---------------------------------------------------------------------------

/// Exact gradient fit in the sample domain: solves the mod-(max+1) slopes from
/// the first row/column differences (accepted only when unambiguous) and
/// verifies every sample. `None` for constant or non-fitting content.
fn fit_gradient(data: &PlaneData, w: u32, h: u32, max: u32) -> Option<Gen> {
    if w == 0 || h == 0 {
        return None;
    }
    let m = i128::from(max) + 1;
    let get = |x: u32, y: u32| i128::from(sample_at(data, w, y, x));
    // Canonical residue in (−m/2, m/2]: a negative slope wrapping close to
    // the modulus resolves to its negative magnitude. Full-sample
    // verification below decides whether the candidate is a real fit.
    let slope = |a: i128, b: i128| {
        let d = (b - a).rem_euclid(m);
        Some(if d > m / 2 { d - m } else { d })
    };
    let sx = if w > 1 {
        slope(get(0, 0), get(1, 0))?
    } else {
        0
    };
    let sy = if h > 1 {
        slope(get(0, 0), get(0, 1))?
    } else {
        0
    };
    if sx == 0 && sy == 0 {
        return None; // constant: FILL territory
    }
    let base = get(0, 0);
    for y in 0..h {
        for x in 0..w {
            let expect = (base + sx * i128::from(x) + sy * i128::from(y)).rem_euclid(m);
            if expect != get(x, y) {
                return None;
            }
        }
    }
    Some(Gen::Gradient {
        base: base as u32,
        sx: sx as i64,
        sy: sy as i64,
    })
}

/// Exact checker fit: smallest cell where the content toggles away from
/// `a = v(0,0)`; verifies every sample against the candidate program.
fn fit_checker(data: &PlaneData, w: u32, h: u32, max: u32) -> Option<Gen> {
    if w == 0 || h == 0 {
        return None;
    }
    let a = sample_at(data, w, 0, 0);
    let limit = w.max(h).min(crate::media::gen::MAX_GEN_PERIOD);
    let mut found = None;
    for c in 1..=limit {
        let hx = if c < w { sample_at(data, w, 0, c) } else { a };
        let vy = if c < h { sample_at(data, w, c, 0) } else { a };
        if hx != a || vy != a {
            found = Some((if hx != a { hx } else { vy }, c));
            break;
        }
    }
    let (b, cell) = found?;
    if b == a || b > max {
        return None;
    }
    let gen = Gen::Checker { a, b, cell };
    for y in 0..h {
        for x in 0..w {
            if gen.sample(i64::from(x), i64::from(y), max) != sample_at(data, w, y, x) {
                return None;
            }
        }
    }
    Some(gen)
}

/// The distinct ascending value set of a plane's content (palette candidate).
fn distinct_values(data: &PlaneData) -> Vec<u32> {
    let mut seen = std::collections::BTreeSet::new();
    match data {
        PlaneData::U8(v) => {
            for s in v {
                seen.insert(u32::from(*s));
            }
        }
        PlaneData::U16(v) => {
            for s in v {
                seen.insert(u32::from(*s));
            }
        }
    }
    seen.into_iter().collect()
}

/// Index-map the samples through an ascending value list (values ≤ 65535).
fn index_map(data: &PlaneData, values: &[u32]) -> Vec<u8> {
    let mut lut = vec![0u8; 65536];
    for (i, v) in values.iter().enumerate() {
        lut[*v as usize] = i as u8;
    }
    match data {
        PlaneData::U8(v) => v.iter().map(|s| lut[*s as usize]).collect(),
        PlaneData::U16(v) => v.iter().map(|s| lut[*s as usize]).collect(),
    }
}

// ---------------------------------------------------------------------------
// Wire-byte cost model (interval groups only; the shared file container is
// excluded and reported separately where relevant)
// ---------------------------------------------------------------------------

fn group_framing(op_count: u64) -> u64 {
    let _ = op_count;
    8 + 4 // t:u64 + op_count:u32
}

fn sample_bytes(data: &PlaneData) -> u64 {
    match data {
        PlaneData::U8(v) => v.len() as u64,
        PlaneData::U16(v) => (v.len() * 2) as u64,
    }
}

/// Accurate wire bytes of one ops list (mirrors `wire.rs` op encodings).
fn ops_wire_bytes(ops: &[PlaneOp]) -> u64 {
    let mut b = 0u64;
    for op in ops {
        match op {
            PlaneOp::DeclareObject { object, .. } => {
                b += 4 + 4 + 4 + 1;
                match &object.content {
                    PlaneContent::Fill(_) => b += 4,
                    PlaneContent::Raster(d) => b += 8 + sample_bytes(d),
                    PlaneContent::Index(v) => b += 8 + v.len() as u64,
                    PlaneContent::Generator(g) => b += g.program_bytes().len() as u64,
                }
            }
            PlaneOp::CreateInstance { .. } => b += 1 + 4 + 4 + 4 + 4,
            PlaneOp::SetPosition { .. } => b += 1 + 4 + 4 + 4,
            PlaneOp::ClearInstances | PlaneOp::ClearOverlay => b += 1,
            PlaneOp::PatchOverlay { points } => b += 1 + 4 + points.len() as u64 * 12,
            PlaneOp::CopyRect { .. } => b += 1 + 4 * 6,
            PlaneOp::Residual { block } | PlaneOp::TransformResidual { block } => {
                b += 1 + 8 + block.len() as u64;
            }
            PlaneOp::SetVelocity { .. } => b += 1 + 4 + 4 + 4,
            PlaneOp::AdvanceTranslations | PlaneOp::AdvanceTrajectories => b += 1,
            PlaneOp::SetTrajectory { segments, .. } => {
                b += 1 + 4 + 4;
                for seg in segments {
                    b += seg.wire_bytes();
                }
            }
            PlaneOp::SetPalette { entries, .. } => b += 1 + 4 + 4 + entries.len() as u64 * 4,
            PlaneOp::PatchPalette { changes, .. } => b += 1 + 4 + 4 + changes.len() as u64 * 8,
            PlaneOp::BindPalette { .. } => b += 1 + 4 + 4,
            PlaneOp::SetAffine { .. } => b += 1 + 4 + 4 * 6,
            PlaneOp::GlobalPredict { .. } => b += GlobalMap::wire_bytes(),
        }
    }
    b
}

fn interval_bytes(ops: &[PlaneOp]) -> u64 {
    group_framing(ops.len() as u64) + ops_wire_bytes(ops)
}

/// Copy a rectangle in the u32 sample domain (mirrors `core.rs::copy_rect_u32`
/// exactly: a sample is written only when both its source and destination
/// positions are in bounds).
#[allow(clippy::too_many_arguments)] // ordered geometry, like the core rule it mirrors
fn sim_copy(
    dst: &mut [u32],
    src: &[u32],
    w: u32,
    h: u32,
    sx: i64,
    sy: i64,
    cw: u32,
    ch: u32,
    dx: i64,
    dy: i64,
) {
    let (dw, dh) = (i64::from(w), i64::from(h));
    for si in 0..ch as i64 {
        for sj in 0..cw as i64 {
            let px = sx + sj;
            let py = sy + si;
            if px < 0 || py < 0 || px >= dw || py >= dh {
                continue;
            }
            let qx = dx + sj;
            let qy = dy + si;
            if qx < 0 || qy < 0 || qx >= dw || qy >= dh {
                continue;
            }
            dst[(qy * i64::from(w) + qx) as usize] = src[(py * i64::from(w) + px) as usize];
        }
    }
}

// ---------------------------------------------------------------------------
// Change decomposition (connected components over the changed-cell grid)
// ---------------------------------------------------------------------------

/// One axis-aligned box of changed content.
#[derive(Debug, Clone, Copy)]
struct BoxGeom {
    x0: i64,
    y0: i64,
    w: u32,
    h: u32,
}

/// Changed samples of `target` vs `base` (both row-major u32), decomposed into
/// connected-component boxes (4-connectivity on the changed grid), largest
/// first, capped at `cap`.
fn changed_boxes(base: &[u32], target: &[u32], w: u32, h: u32, cap: usize) -> (Vec<BoxGeom>, bool) {
    let n = (u64::from(w) * u64::from(h)) as usize;
    let mut diff = vec![false; n];
    let mut any = false;
    for k in 0..n {
        if base[k] != target[k] {
            diff[k] = true;
            any = true;
        }
    }
    if !any {
        return (Vec::new(), false);
    }
    let mut visited = vec![false; n];
    let mut comps: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut stack: Vec<(u32, u32)> = Vec::new();
    for k in 0..n {
        if !diff[k] || visited[k] {
            continue;
        }
        let (sx, sy) = ((k % w as usize) as u32, (k / w as usize) as u32);
        let (mut x0, mut y0, mut x1, mut y1) = (sx, sy, sx, sy);
        visited[k] = true;
        stack.push((sx, sy));
        while let Some((cx, cy)) = stack.pop() {
            let neighbors = [
                (cx.wrapping_sub(1), cy),
                (cx + 1, cy),
                (cx, cy.wrapping_sub(1)),
                (cx, cy + 1),
            ];
            for (nx, ny) in neighbors {
                if nx >= w || ny >= h {
                    continue;
                }
                let nk = (ny * w + nx) as usize;
                if diff[nk] && !visited[nk] {
                    visited[nk] = true;
                    stack.push((nx, ny));
                    x0 = x0.min(nx);
                    y0 = y0.min(ny);
                    x1 = x1.max(nx);
                    y1 = y1.max(ny);
                }
            }
        }
        comps.push((x0, y0, x1, y1));
    }
    comps.sort_by_key(|&(x0, y0, x1, y1)| {
        std::cmp::Reverse(u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1))
    });
    let truncated = comps.len() > cap;
    let out: Vec<BoxGeom> = comps
        .iter()
        .take(cap)
        .map(|&(x0, y0, x1, y1)| BoxGeom {
            x0: i64::from(x0),
            y0: i64::from(y0),
            w: x1 - x0 + 1,
            h: y1 - y0 + 1,
        })
        .collect();
    (out, truncated)
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// A complete per-plane interval decision (chosen family + its ops).
struct Decision {
    family: &'static str,
    ops: Vec<PlaneOp>,
    bytes: u64,
    syncs_state: bool,
    /// V.1.5: the map precision of a chosen global-motion record (registry
    /// code), `None` for every other family.
    map_shift: Option<u8>,
}

/// One plane of the encode in progress.
struct PlaneEncoder {
    prog: PlaneProgram,
    depth: BitDepth,
    w: u32,
    h: u32,
    /// Committed state render (the fresh-render basis of every interval).
    state: Vec<u32>,
    /// The previous materialized observation samples (CopyRect basis).
    prev: Vec<u32>,
    /// First free object id.
    next_object: u32,
    /// First free palette id.
    next_palette: u32,
    /// Palettes this encoder has declared (so identical tables are reused).
    palette_history: Vec<(PlanePaletteId, Vec<u32>)>,
    /// Sample comparisons spent on displacement search.
    search_work: u64,
}

impl PlaneEncoder {
    fn new(depth: BitDepth, w: u32, h: u32, frame0: &Plane) -> Self {
        let mut prog = PlaneProgram::new(sample_at(frame0.data(), w, 0, 0));
        let samples = sample_vec(frame0.data());
        if !is_uniform(frame0.data(), w, h, prog.background) {
            if let Ok(obj) = PlaneObject::raster(w, h, depth, &samples) {
                prog.objects.insert(PlaneObjectId(1), obj);
                prog.instances.push(PlaneInstance {
                    id: PlaneInstanceId(1),
                    object: PlaneObjectId(1),
                    x: 0,
                    y: 0,
                });
            }
        }
        PlaneEncoder {
            prog,
            depth,
            w,
            h,
            prev: samples.clone(),
            state: samples,
            next_object: 2,
            next_palette: 1,
            palette_history: Vec::new(),
            search_work: 0,
        }
    }

    /// A state-sync replacement decision whose declared content (if any)
    /// covers the whole plane, optionally with palette entries (a fresh id is
    /// allocated only when no identical table exists yet). `reuse` names an
    /// already-declared object instead of declaring one.
    fn sync_decision(
        &mut self,
        family: &'static str,
        content: PlaneContent,
        palette_entries: Option<Vec<u32>>,
        reuse: Option<PlaneObjectId>,
    ) -> Decision {
        let mut ops = Vec::new();
        ops.push(PlaneOp::ClearInstances);
        let palette = match palette_entries {
            Some(entries) => {
                let pid = if let Some((pid, _)) =
                    self.palette_history.iter().find(|(_, e)| *e == entries)
                {
                    *pid
                } else if let Some((pid, _)) =
                    self.prog.palettes.iter().find(|(_, e)| **e == entries)
                {
                    *pid
                } else {
                    let pid = PlanePaletteId(self.next_palette);
                    self.next_palette += 1;
                    self.palette_history.push((pid, entries.clone()));
                    pid
                };
                ops.push(PlaneOp::SetPalette { id: pid, entries });
                Some(pid)
            }
            None => None,
        };
        let oid = match reuse {
            Some(id) => id,
            None => {
                let id = PlaneObjectId(self.next_object);
                self.next_object += 1;
                ops.push(PlaneOp::DeclareObject {
                    id,
                    object: PlaneObject {
                        width: self.w,
                        height: self.h,
                        content,
                    },
                });
                id
            }
        };
        ops.push(PlaneOp::CreateInstance {
            id: PlaneInstanceId(1),
            object: oid,
            x: 0,
            y: 0,
        });
        if let Some(pid) = palette {
            ops.push(PlaneOp::BindPalette {
                instance: PlaneInstanceId(1),
                palette: pid,
            });
        }
        Decision {
            family,
            bytes: interval_bytes(&ops),
            ops,
            syncs_state: true,
            map_shift: None,
        }
    }
}

/// Encode a full observation sequence (one epoch's plane table) into an exact
/// multi-plane program with per-family accounting. Proof: every observation
/// is re-materialized sample-for-sample before the program is returned.
pub fn encode_pictures_families(
    epoch: &VideoEpoch,
    observations: &[Picture],
) -> Result<(MultiPlaneProgram, EncodeReport), VoleError> {
    encode_pictures_families_with(epoch, observations, EncodeOptions::default())
}

/// [`encode_pictures_families`] with explicit options (V.1.5 ablations: force
/// a map precision for the §62 court, or disable the global-motion family).
pub fn encode_pictures_families_with(
    epoch: &VideoEpoch,
    observations: &[Picture],
    opts: EncodeOptions,
) -> Result<(MultiPlaneProgram, EncodeReport), VoleError> {
    if observations.is_empty() {
        return Err(VoleError::ApiConstraint(
            "family encode needs at least one observation",
        ));
    }
    for pic in observations {
        pic.validate_against(epoch)?;
    }
    let n_obs = observations.len() as u64;
    let mut encs: Vec<PlaneEncoder> = Vec::with_capacity(epoch.plane_count());
    for (p, _) in epoch.planes().iter().enumerate() {
        let depth = epoch.planes()[p].bit_depth;
        let (pw, ph) = epoch.plane_dimensions(p)?;
        let frame0 = observations[0].plane(p).expect("validated").clone();
        encs.push(PlaneEncoder::new(depth, pw, ph, &frame0));
    }

    let mut report = EncodeReport {
        families: BTreeMap::new(),
        total_interval_bytes: 0,
        candidate_evaluations: 0,
        search_work: 0,
        state_syncs: 0,
        raw_floor_bytes: 0,
        map_shift_observations: BTreeMap::new(),
        map_shift_bytes: BTreeMap::new(),
    };

    for t in 1..n_obs {
        for (p, enc) in encs.iter_mut().enumerate() {
            let target = observations[t as usize]
                .plane(p)
                .expect("validated")
                .clone();
            let target_samples = sample_vec(target.data());
            let work_before = enc.search_work;
            let (decision, evaluations) = encode_interval(enc, &target, &target_samples, &opts);
            let work_after = enc.search_work;
            report.search_work += work_after - work_before;
            report.candidate_evaluations += evaluations;
            if let Some(d) = decision {
                let fam = d.family;
                let bytes = d.bytes;
                if d.syncs_state {
                    report.state_syncs += 1;
                    enc.state = target_samples.clone();
                }
                if let Some(s) = d.map_shift {
                    *report.map_shift_observations.entry(s).or_insert(0) += 1;
                    *report.map_shift_bytes.entry(s).or_insert(0) += bytes;
                }
                enc.prog.intervals.push((t, d.ops));
                report.raw_floor_bytes += raw_floor_bytes_of(enc.w, enc.h, enc.depth);
                report.families.entry(fam).or_default().observations += 1;
                report.families.entry(fam).or_default().interval_bytes += bytes;
                report.total_interval_bytes += bytes;
            }
            // The previous materialized observation for the next interval.
            enc.prev = target_samples;
        }
    }

    let planes: Vec<PlaneProgram> = encs.into_iter().map(|e| e.prog).collect();
    let program = MultiPlaneProgram::new(epoch.clone(), planes)?;
    if program.observation_count() != n_obs {
        return Err(VoleError::ApiConstraint(
            "family encode produced the wrong observation count",
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
                    "family encode failed its materialization proof",
                ));
            }
        }
    }
    Ok((program, report))
}

fn raw_floor_bytes_of(w: u32, h: u32, depth: BitDepth) -> u64 {
    // DeclareObject(raster) + ClearInstances + CreateInstance + framing.
    let payload = u64::from(w) * u64::from(h) * depth.storage().bytes_per_sample();
    12 + (4 + 4 + 4 + 1 + 8 + payload) + 1 + (1 + 4 + 4 + 4 + 4)
}

/// The deterministic family order position.
fn order_of(family: &str) -> usize {
    FAMILY_ORDER
        .iter()
        .position(|f| *f == family)
        .unwrap_or(usize::MAX)
}

/// Evaluate one interval of one plane: propose every family candidate, choose
/// the least-byte valid one (ties by family order), and return its ops plus
/// the number of candidates evaluated.
fn encode_interval(
    enc: &mut PlaneEncoder,
    target: &Plane,
    target_samples: &[u32],
    opts: &EncodeOptions,
) -> (Option<Decision>, u64) {
    let mut best: Option<Decision> = None;
    let mut evaluations = 0u64;
    let mut consider = |d: Decision, best: &mut Option<Decision>| {
        evaluations += 1;
        let better = match best {
            None => true,
            Some(b) => (d.bytes, order_of(d.family)) < (b.bytes, order_of(b.family)),
        };
        if better {
            *best = Some(d);
        }
    };

    // UNCHANGED — the observation equals the committed state render.
    if enc.state == target_samples {
        return (
            Some(Decision {
                family: FAMILY_UNCHANGED,
                ops: Vec::new(),
                bytes: group_framing(0),
                syncs_state: false,
                map_shift: None,
            }),
            1,
        );
    }

    // SPARSE residual over the committed render (always-valid sentinel).
    let mut sparse_points: Vec<(i32, i32, u16)> = Vec::new();
    for y in 0..enc.h {
        for x in 0..enc.w {
            let k = (y * enc.w + x) as usize;
            if enc.state[k] != target_samples[k] {
                sparse_points.push((x as i32, y as i32, target_samples[k] as u16));
            }
        }
    }
    sparse_points.sort_unstable_by_key(|&(x, y, _)| (x, y));
    if let Ok(block) = encode_plane_residual(&sparse_points) {
        let ops = vec![PlaneOp::Residual { block }];
        let bytes = interval_bytes(&ops);
        consider(
            Decision {
                family: FAMILY_SPARSE,
                ops,
                bytes,
                syncs_state: false,
                map_shift: None,
            },
            &mut best,
        );
    }

    // TRANSFORM residual over the committed render (V.1.4 Phase-M floor).
    if let (Ok(sp), Ok(tp)) = (plane_like(enc, &enc.state), plane_like(enc, target_samples)) {
        if let Some(block) = encode_plane_transform_block(&sp, &tp) {
            let ops = vec![PlaneOp::TransformResidual { block }];
            let bytes = interval_bytes(&ops);
            consider(
                Decision {
                    family: FAMILY_TRANSFORM,
                    ops,
                    bytes,
                    syncs_state: false,
                    map_shift: None,
                },
                &mut best,
            );
        }
    }

    // Whole-plane replacement classes (all sync the committed render).
    let td = target.data();

    // FILL: uniform target.
    if is_uniform(td, enc.w, enc.h, target_samples[0]) {
        let d = enc.sync_decision(
            FAMILY_FILL,
            PlaneContent::Fill(target_samples[0]),
            None,
            None,
        );
        consider(d, &mut best);
    }

    // GENERATOR: exact depth-aware program fit (gradient / checker).
    let max = enc.depth.max_sample();
    if let Some(gen) =
        fit_gradient(td, enc.w, enc.h, max).or_else(|| fit_checker(td, enc.w, enc.h, max))
    {
        let d = enc.sync_decision(FAMILY_GENERATOR, PlaneContent::Generator(gen), None, None);
        consider(d, &mut best);
    }

    // PALETTE: two to PALETTE_DETECT_MAX distinct values.
    let values = distinct_values(td);
    if (2..=PALETTE_DETECT_MAX).contains(&values.len()) {
        let indices = index_map(td, &values);
        let d = enc.sync_decision(
            FAMILY_PALETTE,
            PlaneContent::Index(indices),
            Some(values),
            None,
        );
        consider(d, &mut best);
    }

    // RAW: whole-plane raster replacement (always-valid sentinel).
    if let Ok(data) = vec_to_data(target_samples, enc.depth) {
        let d = enc.sync_decision(FAMILY_RAW, PlaneContent::Raster(data), None, None);
        consider(d, &mut best);
    }

    // EXACT: reuse an existing declared whole-plane raster object without
    // re-declaring its content.
    {
        let candidates: Vec<PlaneObjectId> = enc
            .prog
            .objects
            .iter()
            .filter(|(_, o)| o.width == enc.w && o.height == enc.h)
            .filter(|(_, o)| match &o.content {
                PlaneContent::Raster(d) => sample_vec(d) == target_samples,
                _ => false,
            })
            .map(|(id, _)| *id)
            .collect();
        if let Some(id) = candidates.first() {
            // `content` is ignored when `reuse` names an existing object.
            let d = enc.sync_decision(FAMILY_EXACT, PlaneContent::Fill(0), None, Some(*id));
            consider(d, &mut best);
        }
    }

    // Region reuse from the previous observation (TRANSLATION / COPY /
    // REGIONS) with an exact sparse remainder.
    if sparse_points.len() > COPY_MIN_SPARSE {
        if let Some(d) = copy_candidate(enc, target_samples) {
            consider(d, &mut best);
        }
    }

    // V.1.5 global video structure: predict the whole plane from the previous
    // observation through a canonical fixed-point map (translation / rotzoom /
    // affine proposals, brief §61–§63) with an exact residual remainder. Also
    // attempted only when the sparse drift is non-trivial; the exact byte cost
    // decides against every other family above.
    if !opts.disable_global && sparse_points.len() > COPY_MIN_SPARSE {
        if let Some(d) = global_candidate(enc, target_samples, opts) {
            consider(d, &mut best);
        }
    }

    (best, evaluations)
}

/// Rebuild a canonical single-Gray plane over `samples` (geometry of this
/// encoder) so the sealed transform codec can run on it.
fn plane_like(enc: &PlaneEncoder, samples: &[u32]) -> Result<Plane, VoleError> {
    let data = vec_to_data(samples, enc.depth)?;
    Plane::new(
        crate::media::layout::Component::Gray,
        enc.w,
        enc.h,
        enc.depth,
        0,
        0,
        data,
    )
}

/// Search region reuse from the previous observation: decompose the changed
/// area (target vs committed render) into connected-component boxes, find an
/// integer displacement per box whose copy from the previous observation
/// minimizes the sparse remainder, and build the CopyRect(+Residual)
/// candidate. Returns `None` when no displacement explains any box.
fn copy_candidate(enc: &mut PlaneEncoder, target: &[u32]) -> Option<Decision> {
    let (w, h) = (enc.w, enc.h);
    let (boxes, _truncated) = changed_boxes(&enc.state, target, w, h, MAX_REGION_BOXES);
    if boxes.is_empty() {
        return None;
    }
    // Whole-plane-dominant drift: the sentinels cover it (measured, not
    // hidden — a copy over most of the plane cannot beat RAW's economics).
    let total_area = u64::from(w) * u64::from(h);
    let drift_area: u64 = boxes.iter().map(|b| u64::from(b.w) * u64::from(b.h)).sum();
    if total_area > 0 && drift_area * 100 > total_area * DRIFT_SKIP_SHARE {
        return None;
    }
    let mut ops: Vec<PlaneOp> = Vec::new();
    let mut spent = 0u64; // per-interval search budget
    for b in &boxes {
        let best = search_box(&enc.state, &enc.prev, target, w, h, *b, &mut spent);
        let (dx, dy) = best?;
        ops.push(PlaneOp::CopyRect {
            src_x: b.x0 - dx,
            src_y: b.y0 - dy,
            width: b.w,
            height: b.h,
            dst_x: b.x0,
            dst_y: b.y0,
        });
    }
    enc.search_work += spent;
    // Simulate the copies over the committed render and close the remainder
    // with an exact sparse residual (the copy source is always the previous
    // observation, mirroring the decoder's CopyRect semantics).
    let mut sim = enc.state.clone();
    for op in &ops {
        if let PlaneOp::CopyRect {
            src_x,
            src_y,
            width,
            height,
            dst_x,
            dst_y,
        } = op
        {
            sim_copy(
                &mut sim, &enc.prev, w, h, *src_x, *src_y, *width, *height, *dst_x, *dst_y,
            );
        }
    }
    let mut points: Vec<(i32, i32, u16)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let k = (y * w + x) as usize;
            if sim[k] != target[k] {
                points.push((x as i32, y as i32, target[k] as u16));
            }
        }
    }
    if points.is_empty() {
        // The whole drift is explained by pure box translations.
        let family = if ops.len() == 1 {
            FAMILY_TRANSLATION
        } else {
            FAMILY_REGIONS
        };
        return Some(Decision {
            family,
            bytes: interval_bytes(&ops),
            ops,
            syncs_state: false,
            map_shift: None,
        });
    }
    points.sort_unstable_by_key(|&(x, y, _)| (x, y));
    let block = encode_plane_residual(&points).ok()?;
    let family = if ops.len() == 1 {
        FAMILY_COPY
    } else {
        FAMILY_REGIONS
    };
    ops.push(PlaneOp::Residual { block });
    Some(Decision {
        family,
        bytes: interval_bytes(&ops),
        ops,
        syncs_state: false,
        map_shift: None,
    })
}

// ---------------------------------------------------------------------------
// V.1.5 global video structure (whole-plane prediction from the previous
// observation through a canonical fixed-point map; brief §61–§63, §248)
// ---------------------------------------------------------------------------

/// The deterministic family label of a motion-model class.
fn class_label(class: MotionClass) -> &'static str {
    match class {
        MotionClass::Translation => FAMILY_GLOBAL_TRANSLATION,
        MotionClass::Rotzoom => FAMILY_GLOBAL_ROTZOOM,
        MotionClass::Affine => FAMILY_GLOBAL_AFFINE,
    }
}

/// V.1.5: simulate a whole-plane `GlobalPredict` over the committed render
/// (normative mirror of the core op): every destination sample whose mapped
/// source lies inside the previous plane is overwritten with that sample;
/// out-of-bounds destinations keep the committed render. `false` when a
/// mapped coordinate overflows the canonical arithmetic (such a map is not
/// materializable and its candidate is skipped).
fn sim_warp(base: &mut [u32], prev: &[u32], w: u32, h: u32, map: GlobalMap) -> bool {
    let (dw, dh) = (i64::from(w), i64::from(h));
    for y in 0..dh {
        for x in 0..dw {
            let Some((su, sv)) = map.source(x, y) else {
                return false;
            };
            if su < 0 || sv < 0 || su >= dw || sv >= dh {
                continue;
            }
            let k = (y * i64::from(w) + x) as usize;
            base[k] = prev[(sv * i64::from(w) + su) as usize];
        }
    }
    true
}

/// Build the complete exact global-motion decision for one quantized map:
/// warp the previous observation over the committed render, close the
/// mismatch with the cheaper of the sparse / transform residuals, and report
/// the family label + chosen precision. `None` when the map is not
/// materializable under the canonical rule.
fn warp_decision(
    enc: &PlaneEncoder,
    target: &[u32],
    class: MotionClass,
    map: GlobalMap,
) -> Option<Decision> {
    let (w, h) = (enc.w, enc.h);
    let mut sim = enc.state.clone();
    if !sim_warp(&mut sim, &enc.prev, w, h, map) {
        return None;
    }
    let mut points: Vec<(i32, i32, u16)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let k = (y * w + x) as usize;
            if sim[k] != target[k] {
                points.push((x as i32, y as i32, target[k] as u16));
            }
        }
    }
    let mut ops = vec![PlaneOp::GlobalPredict { map }];
    if !points.is_empty() {
        points.sort_unstable_by_key(|&(x, y, _)| (x, y));
        let sparse = encode_plane_residual(&points).ok();
        // A transform residual is priced too (the natural-video case: a dense
        // but smooth whole-plane delta is far cheaper coded than sparse); the
        // cheaper of the two closes the warp exactly.
        let transform = {
            let sp = plane_like(enc, &sim).ok()?;
            let tp = plane_like(enc, target).ok()?;
            encode_plane_transform_block(&sp, &tp)
        };
        let sparse_bytes = sparse.as_ref().map(|b| 1 + 8 + b.len() as u64);
        let transform_bytes = transform.as_ref().map(|b| 1 + 8 + b.len() as u64);
        match (sparse_bytes, transform_bytes) {
            (Some(sb), Some(tb)) if tb < sb => {
                ops.push(PlaneOp::TransformResidual {
                    block: transform.expect("present"),
                });
            }
            (Some(_), _) => ops.push(PlaneOp::Residual {
                block: sparse.expect("present"),
            }),
            (None, Some(_)) => ops.push(PlaneOp::TransformResidual {
                block: transform.expect("present"),
            }),
            (None, None) => return None,
        }
    }
    let bytes = interval_bytes(&ops);
    if bytes == 0 {
        return None;
    }
    Some(Decision {
        family: class_label(class),
        bytes,
        ops,
        syncs_state: false,
        map_shift: Some(map.shift.code()),
    })
}

/// V.1.5 global-motion candidate for one interval of one plane: run the
/// deterministic bounded estimator over (previous observation → target),
/// select the best model class at Q8 by complete bytes, then price that
/// class at every registry precision (or the forced one) and return the
/// cheapest complete decision. The estimator is a proposal only — the
/// normative simulation + residual closure above decide everything.
fn global_candidate(
    enc: &mut PlaneEncoder,
    target: &[u32],
    opts: &EncodeOptions,
) -> Option<Decision> {
    let (w, h) = (enc.w, enc.h);
    let mut spent = 0u64;
    let tol = match_tolerance(enc.depth);
    let hyps: Vec<GlobalHypothesis> = estimate_global(&enc.prev, target, w, h, tol, &mut spent)?;
    enc.search_work = enc.search_work.saturating_add(spent);
    let plane_work = u64::from(w) * u64::from(h) * 2;
    // Class selection at Q8 (cheapest complete bytes; family-order tie).
    let mut best_q8: Option<(Decision, MotionClass, [f64; 6])> = None;
    for hyp in hyps {
        let map = GlobalMap::quantize(MapShift::Q8, &hyp.params);
        enc.search_work = enc.search_work.saturating_add(plane_work);
        if let Some(d) = warp_decision(enc, target, hyp.class, map) {
            let better = match &best_q8 {
                None => true,
                Some((b, _, _)) => (d.bytes, order_of(d.family)) < (b.bytes, order_of(b.family)),
            };
            if better {
                best_q8 = Some((d, hyp.class, hyp.params));
            }
        }
    }
    let (_, class, params) = best_q8?;
    // Precision pricing of the winning class (§62 court — measured, never
    // assumed; ties prefer the lower precision because ALL is ascending).
    let shifts: Vec<MapShift> = match opts.map_shift {
        Some(s) => vec![s],
        None => MapShift::ALL.to_vec(),
    };
    let mut best: Option<Decision> = None;
    for shift in shifts {
        let map = GlobalMap::quantize(shift, &params);
        enc.search_work = enc.search_work.saturating_add(plane_work);
        if let Some(d) = warp_decision(enc, target, class, map) {
            let better = match &best {
                None => true,
                Some(b) => (d.bytes, order_of(d.family)) < (b.bytes, order_of(b.family)),
            };
            if better {
                best = Some(d);
            }
        }
    }
    best
}

/// Search one box for the displacement whose copy from the previous
/// observation best explains the target box, bounded by a deterministic
/// verification budget with early exit once the incumbent is beaten. The
/// score counts every box cell that would remain wrong after the copy:
/// cells whose source is in bounds compare the copied previous-observation
/// sample; cells whose source is clipped keep the committed render (they are
/// compared against `base`). Deterministic order: dy then dx from `−W` to
/// `W`. Returns `(dx, dy)` such that the copy source is `(x0 − dx, y0 − dy)`;
/// `None` when the budget is exhausted before any candidate completes.
fn search_box(
    base: &[u32],
    prev: &[u32],
    target: &[u32],
    w: u32,
    h: u32,
    b: BoxGeom,
    budget: &mut u64,
) -> Option<(i64, i64)> {
    let (x0, y0) = (b.x0, b.y0);
    let (bw, bh) = (b.w, b.h);
    let idx = |k: u32| k as usize;
    let area = u64::from(bw) * u64::from(bh);
    let mut best: Option<(i64, i64, u64)> = None;
    for dy in -DISPLACEMENT_WINDOW..=DISPLACEMENT_WINDOW {
        for dx in -DISPLACEMENT_WINDOW..=DISPLACEMENT_WINDOW {
            *budget = budget.saturating_add(area);
            if *budget > SEARCH_VERIFY_BUDGET {
                // Budget exhausted: return the best completed candidate, if
                // any (a half-searched window cannot beat an incumbent that
                // already completed).
                return best.map(|(dx, dy, _)| (dx, dy));
            }
            let mut mism = 0u64;
            'outer: for i in 0..bh {
                for j in 0..bw {
                    let tcell = idx((y0 + i64::from(i)) as u32 * w + (x0 + i64::from(j)) as u32);
                    let c = target[tcell];
                    let sx = x0 + i64::from(j) - dx;
                    let sy = y0 + i64::from(i) - dy;
                    let a = if sx < 0 || sx >= i64::from(w) || sy < 0 || sy >= i64::from(h) {
                        // The copy cannot write this cell: it keeps the
                        // committed render.
                        base[tcell]
                    } else {
                        prev[idx(sy as u32 * w + sx as u32)]
                    };
                    if a != c {
                        mism += 1;
                        if let Some((_, _, bm)) = best {
                            if mism >= bm {
                                break 'outer; // cannot beat the incumbent
                            }
                        }
                    }
                }
            }
            let better = match best {
                None => true,
                Some((_, _, bm)) => mism < bm,
            };
            if better {
                best = Some((dx, dy, mism));
            }
        }
    }
    best.map(|(dx, dy, _)| (dx, dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_fit_is_exact_in_the_sample_domain() {
        let mut v = Vec::new();
        for y in 0..8u32 {
            for x in 0..16u32 {
                let val = (1000 + 3 * i64::from(x) - 5 * i64::from(y)).rem_euclid(1024) as u32;
                v.push(val);
            }
        }
        let d = vec_to_data(&v, BitDepth::new(10).unwrap()).unwrap();
        let g = fit_gradient(&d, 16, 8, 1023).expect("fits");
        assert_eq!(
            g,
            Gen::Gradient {
                base: 1000,
                sx: 3,
                sy: -5,
            }
        );
        // A one-sample perturbation breaks the exact fit.
        let mut v2 = v.clone();
        v2[5] ^= 7;
        let d2 = vec_to_data(&v2, BitDepth::new(10).unwrap()).unwrap();
        assert!(fit_gradient(&d2, 16, 8, 1023).is_none());
        // A constant field is FILL territory, not a generator.
        let d3 = vec_to_data(&vec![77; 16 * 8], BitDepth::new(8).unwrap()).unwrap();
        assert!(fit_gradient(&d3, 16, 8, 255).is_none());
    }

    #[test]
    fn checker_fit_recovers_cell_and_colors() {
        let gen = Gen::Checker {
            a: 500,
            b: 4000,
            cell: 8,
        };
        let mut v = Vec::new();
        for y in 0..24u32 {
            for x in 0..32u32 {
                v.push(gen.sample(i64::from(x), i64::from(y), 4095));
            }
        }
        let d = vec_to_data(&v, BitDepth::new(12).unwrap()).unwrap();
        let got = fit_checker(&d, 32, 24, 4095).expect("fits");
        assert_eq!(got, gen);
    }

    #[test]
    fn changed_boxes_find_disjoint_components() {
        // Two separated 2x2 blocks on a uniform field.
        let base = vec![0u32; 16 * 16];
        let mut target = base.clone();
        for dy in 0..2 {
            for dx in 0..2 {
                target[(2 + dy) * 16 + (2 + dx)] = 9;
                target[(10 + dy) * 16 + (10 + dx)] = 9;
            }
        }
        let (boxes, truncated) = changed_boxes(&base, &target, 16, 16, 4);
        assert!(!truncated);
        assert_eq!(boxes.len(), 2);
        assert!(boxes
            .iter()
            .any(|b| b.x0 == 2 && b.y0 == 2 && b.w == 2 && b.h == 2));
        assert!(boxes
            .iter()
            .any(|b| b.x0 == 10 && b.y0 == 10 && b.w == 2 && b.h == 2));
    }

    #[test]
    fn sim_copy_mirrors_the_core_clip_rule() {
        let mut dst = vec![0u32; 32 * 24];
        let mut src = vec![0u32; 32 * 24];
        for y in 0..4u32 {
            for x in 0..4u32 {
                src[(y * 32 + x) as usize] = 100 + y * 4 + x;
            }
        }
        sim_copy(&mut dst, &src, 32, 24, 0, 0, 4, 4, 6, 4);
        for y in 0..4u32 {
            for x in 0..4u32 {
                assert_eq!(dst[((y + 4) * 32 + (x + 6)) as usize], 100 + y * 4 + x);
            }
        }
        // A second copy whose source rows start at 22: only source rows
        // 22..23 are inside the canvas, and those source rows hold zeros (the
        // patch lives in rows 0..3), so the copied columns become zero there.
        sim_copy(&mut dst, &src, 32, 24, 0, 22, 4, 8, 2, 0);
        assert_eq!(dst[21 * 32 + 2], 0, "row above the source stays untouched");
        assert_eq!(
            dst[22 * 32 + 2],
            0,
            "in-canvas clipped source row is copied"
        );
        assert_eq!(dst[23 * 32 + 2], 0, "second in-canvas clipped source row");
        // The first copy's patch is untouched by the second.
        assert_eq!(dst[4 * 32 + 8], 102);
    }

    #[test]
    fn wire_bytes_are_consistent() {
        // A single CreateInstance + ClearInstances + framing lengths.
        let ops = vec![
            PlaneOp::ClearInstances,
            PlaneOp::CreateInstance {
                id: PlaneInstanceId(1),
                object: PlaneObjectId(1),
                x: 0,
                y: 0,
            },
        ];
        assert_eq!(interval_bytes(&ops), 12 + 1 + (1 + 4 + 4 + 4 + 4));
        // Empty group framing only.
        assert_eq!(interval_bytes(&[]), 12);
    }

    /// Probe: a translating sprite over a textured background must be served
    /// by CopyRect region reuse from the previous observation (the sprite's
    /// own texture makes the copy exact).
    #[test]
    fn translating_sprite_is_served_by_region_reuse() {
        let depth = BitDepth::new(8).unwrap();
        let (w, h) = (40u32, 24u32);
        let mk = |v: &[u32]| -> Plane {
            Plane::new(
                crate::media::layout::Component::Gray,
                w,
                h,
                depth,
                0,
                0,
                vec_to_data(v, depth).unwrap(),
            )
            .unwrap()
        };
        let mut tex = Vec::new();
        for k in 0..(w * h) as usize {
            let mut z = (k as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(7);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            tex.push((z % 256) as u32);
        }
        let spr: Vec<u32> = (0..64).map(|i| 200 + (i * 5) % 56).collect();
        let put = |samples: &mut Vec<u32>, x: usize| {
            for sy in 0..8usize {
                for sx in 0..8usize {
                    samples[(8 + sy) * w as usize + x + sx] = spr[sy * 8 + sx];
                }
            }
        };
        let frame0 = mk(&tex);
        let mut enc = PlaneEncoder::new(depth, w, h, &frame0);
        for step in 1usize..=3 {
            let mut samples = tex.clone();
            put(&mut samples, 4 + step * 2);
            let target = mk(&samples);
            let samples = sample_vec(target.data());
            let (decision, _) =
                encode_interval(&mut enc, &target, &samples, &EncodeOptions::default());
            let d = decision.expect("a decision");
            if step == 1 {
                // The sprite's first appearance cannot come from the previous
                // observation; a residual class serves it.
                assert!(
                    matches!(d.family, FAMILY_SPARSE | FAMILY_TRANSFORM),
                    "first appearance residual, got {}",
                    d.family
                );
            } else {
                assert_eq!(d.family, FAMILY_TRANSLATION, "step {step}");
            }
            enc.state = if d.syncs_state {
                samples.clone()
            } else {
                enc.state.clone()
            };
            enc.prev = samples;
        }
    }
}
