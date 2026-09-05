//! Global video structure — Phase V.1.5 (V.1 video programme, brief §61–§63,
//! §248; contract §2.8: "global translation / rotzoom / affine proposals over
//! real video with fixed-point normative materialization"). Two layers,
//! strictly separated (brief §62, §100):
//!
//! * [`GlobalMap`] + [`MapShift`] are the **normative** fixed-point map: the
//!   new v2 canvas op `GlobalPredict` (wire tag `0x32`, feature bit `0x2`)
//!   predicts the whole plane of one observation from the **previous
//!   materialized observation** by sampling it through the declared map —
//!   destination `(x, y)` reads source `((a·x + b·y + c) >> shift,
//!   (d·x + e·y + f) >> shift)`, painted only when the source lies inside the
//!   previous plane (out-of-bounds samples keep the interval's fresh state
//!   render, exactly like the sealed `CopyRect` clip rule). The arithmetic is
//!   the sealed v1 Phase-L integer rule (no floating point anywhere);
//!   [`MapShift`] is the §62 precision registry (`Q8` = the v1 precision,
//!   `Q12`, `Q16`), chosen per record by byte cost, never assumed. Residual
//!   closure (`R = F − Ŝ`, per plane) stays mandatory, so any precision is
//!   exact; the precision only prices the residual.
//! * [`estimate_global`] is **encoder-only proposal analysis** (brief §63,
//!   §100 — floating point is permitted here and only here): a deterministic,
//!   bounded pyramid-free estimator that proposes the motion model classes
//!   TRANSLATION (integer search), ROTZOOM (zoom + rotation about the frame
//!   center), and AFFINE (six parameters), each refined by a deterministic
//!   damped least-squares fit over a bounded sample grid. Whatever it proposes
//!   is **quantized** into a normative [`GlobalMap`] and only ever lands in
//!   `.vole` after the encoder has materialized it with the normative rule,
//!   closed the exact residual, and compared the complete byte cost — the
//!   proposal never has authority.
//!
//! Cross-plane *shared* visual-motion state (one map declared once per frame,
//! the decoder deriving per-component coordinates — brief §47–§48) is designed
//! across V.1.5–V.1.7 and lands behind courts; the V.1.5 wire therefore keeps
//! the independent-plane doctrine (contract §2.6): every plane predicts from
//! its own previous observation through its own map, chroma planes estimate
//! their own geometry, and residual closure is per plane.

use crate::error::VoleError;
use crate::media::plane::BitDepth;

/// Fixed-point map precision registry (brief §62 court: Q8 vs Q12 vs Q16 —
/// never assume more precision wins; the encoder prices all three per record
/// and the wire stores the winner as one registry byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MapShift {
    /// The sealed v1 precision: one source pixel = 256 units.
    Q8,
    /// One source pixel = 4096 units.
    Q12,
    /// One source pixel = 65536 units.
    Q16,
}

impl MapShift {
    /// Every registry member, in ascending precision (also the deterministic
    /// tie order: equal byte cost prefers the *lower* precision).
    pub const ALL: [MapShift; 3] = [MapShift::Q8, MapShift::Q12, MapShift::Q16];

    /// The wire registry code (the fixed-point fractional bits).
    pub const fn code(self) -> u8 {
        match self {
            MapShift::Q8 => 8,
            MapShift::Q12 => 12,
            MapShift::Q16 => 16,
        }
    }

    /// Decode a wire registry byte (any other value is non-canonical).
    pub const fn from_code(code: u8) -> Option<MapShift> {
        match code {
            8 => Some(MapShift::Q8),
            12 => Some(MapShift::Q12),
            16 => Some(MapShift::Q16),
            _ => None,
        }
    }

    /// Fractional bits of this precision.
    pub const fn shift(self) -> u32 {
        self.code() as u32
    }

    /// One source pixel in fixed-point units of this precision.
    pub const fn scale(self) -> i64 {
        1i64 << self.shift()
    }

    /// Short label for reports/courts.
    pub const fn label(self) -> &'static str {
        match self {
            MapShift::Q8 => "Q8",
            MapShift::Q12 => "Q12",
            MapShift::Q16 => "Q16",
        }
    }
}

/// Canonical fixed-point global-motion map (dest → source sampling of the
/// previous observation). Destination `(x, y)` samples the previous plane at
///
/// ```text
/// su = (a·x + b·y + c) >> shift      sv = (d·x + e·y + f) >> shift
/// ```
///
/// with signed floor division and `shift` from the [`MapShift`] registry.
/// When `(su, sv)` lies inside the previous plane the destination is painted
/// with that sample; otherwise it keeps the interval's fresh state render.
/// Coefficients are canonical i32-domain values (`|coeff| ≤ 2^24`, the sealed
/// v1 rule). `a = e = scale`, `b = d = c = f = 0` is the identity map (a
/// whole-plane hold of the previous observation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalMap {
    /// Fixed-point precision.
    pub shift: MapShift,
    /// Source-x coefficient of destination x.
    pub a: i64,
    /// Source-x coefficient of destination y.
    pub b: i64,
    /// Source-x translation (fixed point).
    pub c: i64,
    /// Source-y coefficient of destination x.
    pub d: i64,
    /// Source-y coefficient of destination y.
    pub e: i64,
    /// Source-y translation (fixed point).
    pub f: i64,
}

impl GlobalMap {
    /// The identity map (every destination samples the same coordinate).
    pub const fn identity(shift: MapShift) -> GlobalMap {
        let s = shift.scale();
        GlobalMap {
            shift,
            a: s,
            b: 0,
            c: 0,
            d: 0,
            e: s,
            f: 0,
        }
    }

    /// Whether this is the identity map.
    pub const fn is_identity(&self) -> bool {
        self.a == self.shift.scale()
            && self.b == 0
            && self.c == 0
            && self.d == 0
            && self.e == self.shift.scale()
            && self.f == 0
    }

    /// Canonical-form check: every coefficient must fit the sealed v1 wire
    /// domain `±2^24`. (The shift registry is a type, so no runtime check is
    /// needed for it here; the wire reader validates the registry byte.)
    pub fn check(&self) -> Result<(), VoleError> {
        for v in [self.a, self.b, self.c, self.d, self.e, self.f] {
            if v.abs() > crate::affine::MAX_AFFINE_COEFF {
                return Err(VoleError::NonCanonicalEncoding);
            }
        }
        Ok(())
    }

    /// Serialized wire length of one `GlobalPredict` payload:
    /// `tag(1) + shift(1) + 6 × coeff(4)`.
    pub const fn wire_bytes() -> u64 {
        26
    }

    /// The source sample `(su, sv)` for destination `(x, y)` (checked; an
    /// overflowing accumulation is `None` — callers treat it as a typed
    /// error, never a wrap). Signed `>>` is floor division, the canonical
    /// rounding rule, identical to the sealed v1 affine rule at `Q8`.
    pub fn source(self, x: i64, y: i64) -> Option<(i64, i64)> {
        let nu = self
            .a
            .checked_mul(x)?
            .checked_add(self.b.checked_mul(y)?)?
            .checked_add(self.c)?;
        let nv = self
            .d
            .checked_mul(x)?
            .checked_add(self.e.checked_mul(y)?)?
            .checked_add(self.f)?;
        Some((nu >> self.shift.shift(), nv >> self.shift.shift()))
    }

    /// If the map is an exact pure translation (`a = e = scale`, `b = d = 0`,
    /// translations integral in the fixed-point units), return the source
    /// offset `(sx − x, sy − y)` in whole samples.
    pub fn as_translation(&self) -> Option<(i64, i64)> {
        let s = self.shift.scale();
        if self.a != s || self.e != s || self.b != 0 || self.d != 0 {
            return None;
        }
        if self.c % s != 0 || self.f % s != 0 {
            return None;
        }
        Some((self.c / s, self.f / s))
    }

    /// Quantize a continuous proposal into a canonical map at `shift`:
    /// deterministic round-half-away-from-zero per coefficient, saturated to
    /// the wire domain `±2^24`. (Proposal-side; the result is verified by
    /// normative materialization before it may be stored.)
    pub fn quantize(shift: MapShift, params: &[f64; 6]) -> GlobalMap {
        let scale = shift.scale() as f64;
        let bound = crate::affine::MAX_AFFINE_COEFF as f64;
        let q = |v: f64| {
            let r = (v * scale).round();
            let r = if r > bound {
                bound
            } else if r < -bound {
                -bound
            } else {
                r
            };
            r as i64
        };
        GlobalMap {
            shift,
            a: q(params[0]),
            b: q(params[1]),
            c: q(params[2]),
            d: q(params[3]),
            e: q(params[4]),
            f: q(params[5]),
        }
    }
}

/// Motion-model class of an encoder proposal (family labels + reports). The
/// stored wire record is the same canonical map for every class — the class
/// only says which bounded model the estimator fit (brief §61 classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionClass {
    /// Pure whole-plane translation.
    Translation,
    /// Zoom + rotation about the frame center plus translation.
    Rotzoom,
    /// General six-parameter affine.
    Affine,
}

/// One estimator proposal: the model class and its continuous
/// dest→source map parameters `(a, b, c, d, e, f)` in **source-pixel units**
/// (the same layout as [`GlobalMap`] before fixed-point quantization).
#[derive(Debug, Clone, Copy)]
pub struct GlobalHypothesis {
    /// The model class that produced this proposal.
    pub class: MotionClass,
    /// Continuous map parameters (source-pixel units).
    pub params: [f64; 6],
}

/// Largest sample grid the estimator may evaluate (bounded proposal work).
pub const EST_GRID_MAX: usize = 4096;
/// Whole-plane displacement search window per axis (± `GLOBAL_DISP_WINDOW`).
pub const GLOBAL_DISP_WINDOW: i64 = 64;
/// Deterministic damped-least-squares iterations per model fit.
const GN_ITERS: u32 = 24;
/// Deterministic acceptance tries per iteration before the step is rejected.
const LM_MAX_TRIES: u32 = 6;
/// Share of in-bounds grid points the best translation must explain *within
/// the value tolerance* before the costlier models are skipped (an exact or
/// near-exact pan needs no refinement).
const TRANSLATION_DONE_SHARE: f64 = 0.98;
/// Share below which the pair is treated as unrelated (scene cut / noise /
/// occlusion everywhere): the costlier models are skipped; the translation
/// candidate is still returned for the byte cost to reject.
const TRANSLATION_HOPELESS_SHARE: f64 = 0.02;

/// The sample-value tolerance used to judge whether a translation already
/// explains a raster pair (proposal pruning only; exactness is always decided
/// by the normative residual). Scaled to the plane depth, clamped small.
pub(crate) fn match_tolerance(depth: BitDepth) -> u32 {
    ((depth.max_sample() + 1) >> 7).clamp(1, 8)
}

// ---------------------------------------------------------------------------
// Estimator internals (non-normative; deterministic; f64 permitted)
// ---------------------------------------------------------------------------

/// Deterministic splitmix-style lattice value in `0..=255` (content courts).
#[cfg(test)]
fn lattice(x: i64, y: i64, seed: u64) -> u32 {
    let mut z = (x as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(seed);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z as u32) & 0xFF
}

/// Bilinear sample of a u32 plane at continuous coordinates, clamped to the
/// plane edge (estimator-side only; deterministic).
fn bilerp(src: &[u32], w: u32, h: u32, u: f64, v: f64) -> f64 {
    if w == 0 || h == 0 {
        return 0.0;
    }
    let (fw, fh) = (f64::from(w), f64::from(h));
    let u = u.clamp(0.0, fw - 1.0);
    let v = v.clamp(0.0, fh - 1.0);
    let x0 = u.floor() as u32;
    let y0 = v.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = u - f64::from(x0);
    let fy = v - f64::from(y0);
    let a = f64::from(src[(y0 * w + x0) as usize]);
    let b = f64::from(src[(y0 * w + x1) as usize]);
    let c = f64::from(src[(y1 * w + x0) as usize]);
    let d = f64::from(src[(y1 * w + x1) as usize]);
    let top = a + (b - a) * fx;
    let bot = c + (d - c) * fx;
    top + (bot - top) * fy
}

/// The grid of destination samples the estimator fits over (deterministic
/// stride so the point count stays within [`EST_GRID_MAX`]).
fn sample_grid(w: u32, h: u32) -> Vec<(u32, u32)> {
    let total = u64::from(w) * u64::from(h);
    let stride = (total.div_ceil(EST_GRID_MAX as u64)).max(1) as u32;
    let mut pts = Vec::new();
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            pts.push((x, y));
            x += stride;
        }
        y += stride;
    }
    pts
}

/// Deterministic gaussian elimination (partial pivot; ties keep the first
/// row) for the `n × n` system `A·δ = b` (row-major `a`). `None` when
/// singular.
fn solve_linear(a: &[f64], b: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut m: Vec<f64> = a.to_vec();
    let mut rhs: Vec<f64> = b.to_vec();
    for col in 0..n {
        let mut piv = col;
        let mut best = m[col * n + col].abs();
        for r in (col + 1)..n {
            let v = m[r * n + col].abs();
            if v > best {
                best = v;
                piv = r;
            }
        }
        if best < 1e-12 {
            return None;
        }
        if piv != col {
            for k in 0..n {
                m.swap(col * n + k, piv * n + k);
            }
            rhs.swap(col, piv);
        }
        let d = m[col * n + col];
        for r in (col + 1)..n {
            let f = m[r * n + col] / d;
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                m[r * n + k] -= f * m[col * n + k];
            }
            rhs[r] -= f * rhs[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let mut s = rhs[r];
        for k in (r + 1)..n {
            s -= m[r * n + k] * x[k];
        }
        x[r] = s / m[r * n + r];
    }
    Some(x)
}

/// Deterministic damped least-squares refinement of `N` continuous map
/// parameters: numeric-Jacobian Gauss-Newton with a Levenberg-style damping
/// (`f` maps parameters to the per-point residual vector over the fixed
/// sample grid). Returns the best `(cost, parameters, residuals)` reached.
fn refine_ls<const N: usize>(
    p0: [f64; N],
    f: &dyn Fn([f64; N]) -> Vec<f64>,
) -> (f64, [f64; N], Vec<f64>) {
    let cost_of = |e: &[f64]| e.iter().map(|v| v * v).sum::<f64>();
    let mut p = p0;
    let mut e = f(p);
    let mut cost = cost_of(&e);
    let mut lambda = 1e-3;
    for _ in 0..GN_ITERS {
        // Numeric Jacobian columns over the fixed residual vector: each
        // column is (f(p + eps·eₖ) − f(p)) / eps for the whole grid.
        let mut cols: Vec<Vec<f64>> = Vec::with_capacity(N);
        for k in 0..N {
            let eps = 1e-3 * (1.0 + p[k].abs());
            let mut pp = p;
            pp[k] += eps;
            let ep = f(pp);
            cols.push(ep.iter().zip(&e).map(|(hi, lo)| (hi - lo) / eps).collect());
        }
        // Normal equations: JᵀJ and Jᵀe over the fixed point set.
        let mut jtj = vec![0.0; N * N];
        let mut jte = vec![0.0; N];
        for i in 0..e.len() {
            for k in 0..N {
                jte[k] -= cols[k][i] * e[i];
                for l in k..N {
                    jtj[k * N + l] += cols[k][i] * cols[l][i];
                }
            }
        }
        for k in 0..N {
            for l in 0..k {
                jtj[k * N + l] = jtj[l * N + k];
            }
        }
        // (JᵀJ + λ·diag) δ = −Jᵀe  (a small ridge keeps singular fits still).
        let mut accepted = false;
        for _try in 0..LM_MAX_TRIES {
            let mut damped = jtj.clone();
            for k in 0..N {
                damped[k * N + k] += lambda * (jtj[k * N + k].abs() + 1e-12);
            }
            let Some(delta) = solve_linear(&damped, &jte, N) else {
                lambda *= 10.0;
                continue;
            };
            let mut cand = p;
            for k in 0..N {
                cand[k] += delta[k];
            }
            let ec = f(cand);
            let cc = cost_of(&ec);
            if cc < cost {
                p = cand;
                e = ec;
                cost = cc;
                lambda *= 0.4;
                accepted = true;
                break;
            }
            lambda *= 10.0;
        }
        if !accepted {
            break;
        }
        if cost < 1e-12 {
            break;
        }
    }
    (cost, p, e)
}

/// Evaluate the residuals of a continuous dest→source map over the fixed grid
/// (bilinear prev sampling, edge-clamped; estimator-side only).
fn map_residuals(
    prev: &[u32],
    target: &[u32],
    w: u32,
    h: u32,
    grid: &[(u32, u32)],
    m: &[f64; 6],
) -> Vec<f64> {
    grid.iter()
        .map(|&(x, y)| {
            let su = m[0] * f64::from(x) + m[1] * f64::from(y) + m[2];
            let sv = m[3] * f64::from(x) + m[4] * f64::from(y) + m[5];
            let pred = bilerp(prev, w, h, su, sv);
            let t = f64::from(target[(y * w + x) as usize]);
            t - pred
        })
        .collect()
}

/// Continuous parameters of a translation hypothesis (source = dest + t).
fn translation_params(dx: i64, dy: i64) -> [f64; 6] {
    [1.0, 0.0, dx as f64, 0.0, 1.0, dy as f64]
}

/// Continuous parameters of a center-parameterized rotzoom
/// `(zoom, θ, tx, ty)`: source = center + Rot(−θ)·(dest − center)/zoom + t.
fn rotzoom_params(z: f64, theta: f64, tx: f64, ty: f64, cx: f64, cy: f64) -> [f64; 6] {
    let (c, s) = (theta.cos(), theta.sin());
    let a = c / z;
    let b = s / z;
    let d = -s / z;
    let e = c / z;
    [
        a,
        b,
        cx + tx - a * cx - b * cy,
        d,
        e,
        cy + ty - d * cx - e * cy,
    ]
}

/// Refine the four rotzoom parameters about the frame center (deterministic
/// damped LS over the fixed grid). Returns the best continuous map found.
fn refine_rotzoom(
    prev: &[u32],
    target: &[u32],
    w: u32,
    h: u32,
    grid: &[(u32, u32)],
    init_t: (f64, f64),
) -> Option<[f64; 6]> {
    let cx = f64::from(w - 1) / 2.0;
    let cy = f64::from(h - 1) / 2.0;
    let wrap = |p: [f64; 4]| rotzoom_params(p[0], p[1], p[2], p[3], cx, cy);
    let f = |p: [f64; 4]| map_residuals(prev, target, w, h, grid, &wrap(p));
    let p0 = [1.0, 0.0, init_t.0, init_t.1];
    let (_cost, best, _e) = refine_ls(p0, &f);
    let m = wrap(best);
    if m[0].abs() > 1e-6 && m[4].abs() > 1e-6 {
        Some(m)
    } else {
        None
    }
}

/// Refine the full six-parameter affine from a starting map (deterministic
/// damped LS over the fixed grid). Returns the best continuous map found.
fn refine_affine(
    prev: &[u32],
    target: &[u32],
    w: u32,
    h: u32,
    grid: &[(u32, u32)],
    init: [f64; 6],
) -> Option<[f64; 6]> {
    let f = |p: [f64; 6]| map_residuals(prev, target, w, h, grid, &p);
    let (_cost, best, _e) = refine_ls(init, &f);
    Some(best)
}

/// Whole-plane integer translation search (two deterministic passes: stride-2
/// over `±GLOBAL_DISP_WINDOW`, then ±1 refinement), scoring the exact sample
/// One 2×2 box-downsample (mean, floor) of a plane — the estimator's pyramid
/// levels are built from this (deterministic, integer-only).
fn box_downsample(src: &[u32], w: u32, h: u32) -> (Vec<u32>, u32, u32) {
    let (w2, h2) = (w / 2, h / 2);
    if w2 == 0 || h2 == 0 {
        return (Vec::new(), 0, 0);
    }
    let mut out = Vec::with_capacity((w2 * h2) as usize);
    for y in 0..h2 {
        for x in 0..w2 {
            let a = src[(y * 2 * w + x * 2) as usize];
            let b = src[(y * 2 * w + x * 2 + 1) as usize];
            let c = src[((y * 2 + 1) * w + x * 2) as usize];
            let d = src[((y * 2 + 1) * w + x * 2 + 1) as usize];
            out.push(((u64::from(a) + u64::from(b) + u64::from(c) + u64::from(d)) / 4) as u32);
        }
    }
    (out, w2, h2)
}

/// Whole-plane integer translation search over a deterministic image pyramid
/// (a bounded coarse window at the smallest level, then ±1-scale refinement
/// per level down to full resolution): scores the mean absolute difference of
/// `target(x, y)` against `prev(x + dx, y + dy)` over the in-bounds part of
/// each level's sample grid. Returns the best displacement in full-resolution
/// samples. Work (compared samples) is charged to the budget.
fn search_translation(prev: &[u32], target: &[u32], w: u32, h: u32, work: &mut u64) -> (i64, i64) {
    // Pyramid: halve while both dimensions stay ≥ 32 after halving.
    let mut lvls: Vec<(Vec<u32>, Vec<u32>, u32, u32)> = Vec::new();
    let (mut pw, mut ph) = (prev.to_vec(), target.to_vec());
    let (mut wl, mut hl) = (w, h);
    loop {
        lvls.push((pw, ph, wl, hl));
        let (nw, nh) = (wl / 2, hl / 2);
        if nw < 32 || nh < 32 {
            break;
        }
        let (p2, w2, h2) = box_downsample(&lvls.last().expect("level").0, wl, hl);
        let (t2, _, _) = box_downsample(&lvls.last().expect("level").1, wl, hl);
        debug_assert_eq!(w2, nw);
        debug_assert_eq!(h2, nh);
        pw = p2;
        ph = t2;
        wl = w2;
        hl = h2;
    }
    let top = lvls.len() - 1;
    let mut d = (0i64, 0i64); // best displacement so far, in fine samples
    for l in (0..=top).rev() {
        let (pv, tv, wl, hl) = &lvls[l];
        let grid = sample_grid(*wl, *hl);
        let scale = 1i64 << l;
        let w_axis = if l == top {
            let need = (GLOBAL_DISP_WINDOW + scale - 1) / scale;
            need.clamp(1, 24)
        } else {
            1
        };
        let mut best: Option<(i64, i64, f64)> = None;
        for kd in -w_axis..=w_axis {
            for kx in -w_axis..=w_axis {
                // The candidate fine displacement stays a multiple of this
                // level's scale, so its coarse-plane index is exact.
                let dx = d.0 + kx * scale;
                let dy = d.1 + kd * scale;
                let (dxc, dyc) = (dx / scale, dy / scale);
                let mut hit = 0u64;
                let mut sad = 0u64;
                for &(x, y) in &grid {
                    let sx = i64::from(x) + dxc;
                    let sy = i64::from(y) + dyc;
                    if sx < 0 || sy < 0 || sx >= i64::from(*wl) || sy >= i64::from(*hl) {
                        continue;
                    }
                    hit += 1;
                    let k = (y * wl + x) as usize;
                    let a = pv[(sy as u32 * wl + sx as u32) as usize];
                    let b = tv[k];
                    sad += u64::from(a.abs_diff(b));
                }
                *work = work.saturating_add(hit.max(1));
                if hit == 0 {
                    continue;
                }
                let mean = sad as f64 / hit as f64;
                let better = match best {
                    None => true,
                    Some((_, _, bm)) => mean < bm,
                };
                if better {
                    best = Some((dx, dy, mean));
                }
            }
        }
        let (dx, dy, mean) = best.expect("a candidate");
        d = (dx, dy);
        if l == 0 && mean == 0.0 {
            break; // an exact whole-pixel alignment at full resolution
        }
    }
    d
}

/// Propose global motion models between two consecutive observations of one
/// plane (encoder-only; deterministic; bounded; f64 permitted here and only
/// here, brief §100). The returned hypotheses are ordered
/// TRANSLATION → ROTZOOM → AFFINE; the costlier models are proposed only when
/// the integer translation leaves a meaningful share of the grid unexplained
/// (an exact pan is done; a scene cut / noise field is hopeless for any
/// model — the translation candidate is still returned for the exact byte
/// cost to reject, and the costlier fits are skipped). `work` is charged
/// sample comparisons and is bounded by the caller's budget.
pub fn estimate_global(
    prev: &[u32],
    target: &[u32],
    w: u32,
    h: u32,
    tol: u32,
    work: &mut u64,
) -> Option<Vec<GlobalHypothesis>> {
    if w == 0 || h == 0 {
        return None;
    }
    if (u64::from(w) * u64::from(h)) as usize != prev.len().min(target.len()) {
        return None;
    }
    let grid = sample_grid(w, h);
    let (dx, dy) = search_translation(prev, target, w, h, work);
    let mut out = vec![GlobalHypothesis {
        class: MotionClass::Translation,
        params: translation_params(dx, dy),
    }];
    // Share of in-bounds grid points the best translation explains within the
    // value tolerance (an exact pan is 1.0; smooth / quantized content with a
    // true rigid shift is close to 1.0; unrelated content is ~0).
    let mut hit = 0u64;
    let mut matched = 0u64;
    for &(x, y) in &grid {
        let sx = i64::from(x) + dx;
        let sy = i64::from(y) + dy;
        if sx < 0 || sy < 0 || sx >= i64::from(w) || sy >= i64::from(h) {
            continue;
        }
        hit += 1;
        let a = prev[(sy as u32 * w + sx as u32) as usize];
        let b = target[(y * w + x) as usize];
        if a.abs_diff(b) <= tol {
            matched += 1;
        }
    }
    if hit == 0 {
        return Some(out);
    }
    let share = matched as f64 / hit as f64;
    if share >= TRANSLATION_DONE_SHARE || share <= TRANSLATION_HOPELESS_SHARE {
        return Some(out);
    }
    // Rotzoom fit from the translation init, then affine from the rotzoom
    // result (and from the translation init as a deterministic second start).
    let t = (dx as f64, dy as f64);
    if let Some(rz) = refine_rotzoom(prev, target, w, h, &grid, t) {
        let m = rz;
        *work = work.saturating_add(grid.len() as u64);
        out.push(GlobalHypothesis {
            class: MotionClass::Rotzoom,
            params: m,
        });
        if let Some(af) = refine_affine(prev, target, w, h, &grid, m) {
            *work = work.saturating_add(grid.len() as u64);
            out.push(GlobalHypothesis {
                class: MotionClass::Affine,
                params: af,
            });
        }
    }
    // Deterministic second affine start directly from the translation (a
    // rotation can stall the 4-parameter fit's first step; the 6-parameter
    // fit is free to leave the rotzoom manifold).
    let tx = translation_params(dx, dy);
    if let Some(af) = refine_affine(prev, target, w, h, &grid, tx) {
        *work = work.saturating_add(grid.len() as u64);
        out.push(GlobalHypothesis {
            class: MotionClass::Affine,
            params: af,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affine::AffineParams;

    fn plane(w: u32, h: u32, values: impl Fn(u32, u32) -> u32) -> Vec<u32> {
        (0..(w * h)).map(|k| values(k % w, k / w)).collect()
    }

    #[test]
    fn q8_source_matches_the_sealed_affine_rule() {
        // At Q8 the map rule must be identical to the sealed v1 AffineParams
        // rule on every coordinate, including negative ones.
        let map = GlobalMap {
            shift: MapShift::Q8,
            a: 251,
            b: 7,
            c: -500,
            d: -3,
            e: 260,
            f: 900,
        };
        let aff = AffineParams {
            a: 251,
            b: 7,
            c: -500,
            d: -3,
            e: 260,
            f: 900,
        };
        for x in -40i64..40 {
            for y in -40i64..40 {
                assert_eq!(map.source(x, y), aff.source(x, y));
            }
        }
        assert_eq!(GlobalMap::wire_bytes(), 26);
        assert!(map.check().is_ok());
    }

    #[test]
    fn map_shift_registry_and_identity() {
        assert_eq!(MapShift::ALL.len(), 3);
        for s in MapShift::ALL {
            assert_eq!(MapShift::from_code(s.code()), Some(s));
            assert_eq!(s.scale(), 1i64 << s.shift());
        }
        assert_eq!(MapShift::from_code(7), None);
        assert_eq!(MapShift::from_code(9), None);
        for s in MapShift::ALL {
            let id = GlobalMap::identity(s);
            assert!(id.is_identity());
            assert!(id.check().is_ok());
            for (x, y) in [(0i64, 0i64), (5, -3), (100, 200)] {
                assert_eq!(id.source(x, y), Some((x, y)));
            }
        }
    }

    #[test]
    fn as_translation_requires_integral_pure_translation() {
        let m = GlobalMap {
            shift: MapShift::Q8,
            a: 256,
            b: 0,
            c: -512,
            d: 0,
            e: 256,
            f: 256,
        };
        assert_eq!(m.as_translation(), Some((-2, 1)));
        // A map with a rotation is not a translation.
        let r = GlobalMap {
            shift: MapShift::Q8,
            a: 256,
            b: 1,
            ..GlobalMap::identity(MapShift::Q8)
        };
        assert_eq!(r.as_translation(), None);
        // A non-integral translation is not a whole-sample translation.
        let h = GlobalMap {
            shift: MapShift::Q8,
            c: -128,
            ..GlobalMap::identity(MapShift::Q8)
        };
        assert_eq!(h.as_translation(), None);
    }

    #[test]
    fn quantize_rounds_and_saturates_deterministically() {
        let p = [1.0, 0.0, -2.25, 0.0, 1.0, 0.5];
        let m = GlobalMap::quantize(MapShift::Q8, &p);
        assert_eq!((m.a, m.c, m.f), (256, -576, 128));
        // Half-away-from-zero on the negative side.
        let p = [1.0, 0.0, -2.5, 0.0, 1.0, 0.0];
        let m = GlobalMap::quantize(MapShift::Q8, &p);
        assert_eq!(m.c, -640);
        // Saturation to the wire domain.
        let big = [1e6, 0.0, 0.0, 0.0, 1.0, 0.0];
        let m = GlobalMap::quantize(MapShift::Q8, &big);
        assert!(m.check().is_ok());
        assert_eq!(m.a, crate::affine::MAX_AFFINE_COEFF);
    }

    #[test]
    fn source_mapping_never_wraps_on_overflow() {
        let m = GlobalMap {
            shift: MapShift::Q8,
            a: 1 << 40,
            ..GlobalMap::identity(MapShift::Q8)
        };
        assert!(m.source(1 << 40, 0).is_none());
    }

    #[test]
    fn check_rejects_out_of_domain_coefficients() {
        let bad = GlobalMap {
            shift: MapShift::Q8,
            a: (1 << 24) + 1,
            ..GlobalMap::identity(MapShift::Q8)
        };
        assert_eq!(bad.check().unwrap_err(), VoleError::NonCanonicalEncoding);
        let ok = GlobalMap {
            shift: MapShift::Q8,
            a: 1 << 24,
            ..GlobalMap::identity(MapShift::Q8)
        };
        assert!(ok.check().is_ok());
    }

    /// Deterministic smooth content (sum of lattice-noise octaves, bilinear
    /// interpolated) in `0..=255`.
    fn smooth_content(w: u32, h: u32, seed: u64) -> Vec<u32> {
        plane(w, h, |x, y| {
            let mut acc = 0.0f64;
            let mut amp = 1.0;
            let mut scale = 4.0f64;
            for oct in 0..4u32 {
                let sx = f64::from(x) / scale;
                let sy = f64::from(y) / scale;
                let x0 = sx.floor() as i64;
                let y0 = sy.floor() as i64;
                let fx = sx - sx.floor();
                let fy = sy - sy.floor();
                let s = seed.wrapping_add(u64::from(oct).wrapping_mul(0x9E3779B97F4A7C15));
                let bl = f64::from(lattice(x0, y0, s));
                let br = f64::from(lattice(x0 + 1, y0, s));
                let tl = f64::from(lattice(x0, y0 + 1, s));
                let tr = f64::from(lattice(x0 + 1, y0 + 1, s));
                let top = bl + (br - bl) * fx;
                let bot = tl + (tr - tl) * fx;
                acc += amp * (top + (bot - top) * fy);
                amp *= 0.5;
                scale *= 2.0;
            }
            (acc / 1.9375) as u32 // 1 + .5 + .25 + .125 + ... ≈ 1.9375
        })
    }

    #[test]
    fn estimator_recovers_exact_integer_translation() {
        let (w, h) = (96u32, 64u32);
        // Content with margin on every side so a shifted view never leaves it.
        let (cw, ch, m) = (w + 32, h + 32, 16i64);
        let content = smooth_content(cw, ch, 7);
        let view = |ox: i64, oy: i64| -> Vec<u32> {
            plane(w, h, |x, y| {
                content[((y as i64 + oy + m) as u32 * cw + (x as i64 + ox + m) as u32) as usize]
            })
        };
        let (dx, dy) = (3i64, -2i64);
        let prev = view(0, 0);
        let target = view(dx, dy);
        let mut work = 0u64;
        let hyps = estimate_global(&prev, &target, w, h, 1, &mut work).expect("hypotheses");
        assert_eq!(hyps[0].class, MotionClass::Translation);
        // The exact pan explains every overlapping sample: no costlier models.
        assert_eq!(hyps.len(), 1);
        assert!(
            (hyps[0].params[2] - 3.0).abs() < 1e-9,
            "{:?}",
            hyps[0].params
        );
        assert!(
            (hyps[0].params[5] + 2.0).abs() < 1e-9,
            "{:?}",
            hyps[0].params
        );
        assert!(work > 0);
    }

    #[test]
    fn estimator_recovers_rotzoom_from_rendered_frames() {
        let (w, h) = (96u32, 96u32);
        let content = smooth_content(w, h, 11);
        // Render `prev` as-is and `target` as the same content magnified
        // about the frame center by z = 1.05 (dest samples source/1.05).
        let z = 1.05f64;
        let render = |zoom: f64| -> Vec<u32> {
            plane(w, h, |x, y| {
                let cx = f64::from(w - 1) / 2.0;
                let su = (f64::from(x) - cx) / zoom + cx;
                let sv = (f64::from(y) - cx) / zoom + cx;
                bilerp(&content, w, h, su, sv).round() as u32
            })
        };
        let prev = render(1.0);
        let target = render(z);
        let mut work = 0u64;
        let hyps = estimate_global(&prev, &target, w, h, 2, &mut work).expect("hypotheses");
        // The rotzoom (and affine) fits should have been proposed and their
        // zoom should be near the ground truth (a 6-parameter affine can also
        // absorb a pure zoom).
        let rz = hyps.iter().find(|h| h.class == MotionClass::Rotzoom);
        let af = hyps.iter().find(|h| h.class == MotionClass::Affine);
        let (rz_zoom, af_zoom) = (rz.map(|h| 1.0 / h.params[0]), af.map(|h| 1.0 / h.params[0]));
        let best = rz_zoom.or(af_zoom).expect("a zooming model");
        assert!(
            (best - z).abs() < 0.05,
            "recovered zoom {best} vs truth {z} (rotzoom {rz_zoom:?}, affine {af_zoom:?})"
        );
        // A translation-only proposal must exist first.
        assert_eq!(hyps[0].class, MotionClass::Translation);
    }
}
