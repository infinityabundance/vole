//! Deterministic partial materialization — Phase S (master brief §16, §37,
//! §66).
//!
//! A partial view materializes only the samples a caller asked for. The
//! defining semantic contract is:
//!
//! > `partial(idx, view)` returns exactly the samples that a whole-frame
//! > decode of frame `idx` would place inside the view's in-canvas region
//! > (the view's own top-left becomes the returned canvas origin), or a typed
//! > error.
//!
//! Frame semantics that make this tractable: every interval-group canvas op
//! (COPY_RECT / MOVE_RECT / residual, Phase D/G/M) reads **only** the
//! immediately previous frame and overpaints the freshly materialized base, so
//! the value of frame `t` at a position is either
//!
//! * the base state paint at `t` (background, instances, overlay),
//! * the value of frame `t−1` at the source position of the **last** canvas op
//!   whose destination covers the position (COPY/MOVE), or
//! * the value carried by that last op itself (residual).
//!
//! A *demand plan* therefore runs backward from the requested region: frame
//! `t−1` must be exact only where frame `t`'s ops read it. Then a forward
//! replay paints each level only inside its demanded region (plus exactly the
//! op reads), so decode work tracks the region of interest, not the canvas.
//!
//! Structure:
//!
//! * [`materialize_view`] / [`Decoder::materialize_view`]: the public random-
//!   access partial decode (demand plan + forward replay).
//! * [`crate::materialize::materialize`] handles `View::Rect`/`View::Tile` at
//!   the *state* level (frame 0 / no timeline) through the same base painter.
//!
//! ## Audit-scope semantics (normative, documented)
//!
//! * The [`View::FullFrame`] request is **byte- and error-identical** to
//!   whole-frame decode: it replays through the canonical step machinery.
//! * A sub-frame view validates and decodes everything that contributes to the
//!   requested region (state transitions, op framing, residual containers are
//!   decoded/validated in full; residual point lists are fully validated) and
//!   paints only the demanded samples. Content that **never contributes** to
//!   the view (e.g. an instance painted wholly outside the region, or an
//!   affine overflow outside it) is not audited — a view is a *sampling*
//!   contract; whole-frame decode remains the canonical audit path.
//! * Every partial result equals the whole-frame crop sample-for-sample
//!   (courted, including randomized streams with copy chains).
//!
//! ## Boundedness
//!
//! Demand regions are stored as merged per-row spans; when bookkeeping would
//! exceed [`REGION_SPAN_BUDGET`] spans a region saturates to the whole canvas
//! (an over-approximation that is always exact-output-safe and bounds memory
//! to the same class as whole-frame decode). Interval counts are bounded by
//! `Limits.max_checkpoint_distance` at parse, and replay is bounded exactly as
//! whole-frame decode's is.

use std::collections::{BTreeMap, HashSet};

use crate::{
    decoder,
    error::VoleError,
    format::ParsedStream,
    limits::Limits,
    object::Object,
    pixel::Canvas,
    state::{Instance, State},
    transition::Transition,
    view::{View, ViewBox},
};

/// Saturation cap for one demand region's row-span bookkeeping. Beyond this
/// the region saturates to the whole canvas — sound (over-approximation) and
/// bounded.
const REGION_SPAN_BUDGET: usize = 1 << 20;

// ---------------------------------------------------------------------------
// Region: a set of canvas positions as merged per-row half-open spans.
// Invariants: rows keyed by y (present only when non-empty); per row the spans
// are sorted, disjoint, merged, in canvas bounds, half-open [x0, x1).
// ---------------------------------------------------------------------------

/// A set of canvas positions (`0..width`, `0..height` frame coordinates).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Region {
    rows: BTreeMap<u32, Vec<(u32, u32)>>,
}

impl Region {
    /// The single rectangle `x, y, w × h` (caller guarantees in-canvas,
    /// nonzero).
    fn from_rect(x: u32, y: u32, w: u32, h: u32) -> Self {
        debug_assert!(w > 0 && h > 0);
        let x1 = x.saturating_add(w);
        let y1 = y.saturating_add(h);
        let mut rows = BTreeMap::new();
        for ry in y..y1 {
            rows.insert(ry, vec![(x, x1)]);
        }
        Region { rows }
    }

    /// The whole canvas.
    fn full(cw: u32, ch: u32) -> Self {
        Self::from_rect(0, 0, cw, ch)
    }

    fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Number of row spans (bookkeeping size).
    fn span_count(&self) -> usize {
        self.rows.values().map(Vec::len).sum()
    }

    /// Tight bounding box, if any.
    fn bbox(&self) -> Option<(u32, u32, u32, u32)> {
        let mut it = self.rows.iter();
        let (&y0, first) = it.next()?;
        let (mut x0, mut x1) = (first[0].0, first[0].1);
        let mut y1 = y0 + 1;
        for (&ry, spans) in it {
            y1 = ry + 1;
            for &(a, b) in spans {
                x0 = x0.min(a);
                x1 = x1.max(b);
            }
        }
        Some((x0, y0, x1 - x0, y1 - y0))
    }

    /// Saturate to the whole canvas (sound over-approximation; keeps the
    /// region representation trivially small).
    fn saturate_full(&mut self, cw: u32, ch: u32) {
        *self = Self::full(cw, ch);
    }

    /// Whether the region has at least one sample in the box (in-canvas).
    fn intersects_box(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> bool {
        if x0 >= x1 || y0 >= y1 {
            return false;
        }
        for (_, spans) in self.rows.range(y0..y1) {
            for &(a, b) in spans {
                if b > x0 && a < x1 {
                    return true;
                }
            }
        }
        false
    }

    /// Retain only the part inside the rectangle (in-canvas).
    fn intersect_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if w == 0 || h == 0 {
            self.rows.clear();
            return;
        }
        let x1 = x.saturating_add(w);
        let y1 = y.saturating_add(h);
        let kept: BTreeMap<u32, Vec<(u32, u32)>> = self
            .rows
            .range(y..y1)
            .map(|(&ry, spans)| {
                let clipped: Vec<(u32, u32)> = spans
                    .iter()
                    .filter_map(|&(a, b)| {
                        let l = a.max(x);
                        let r = b.min(x1);
                        (l < r).then_some((l, r))
                    })
                    .collect();
                (ry, clipped)
            })
            .filter(|(_, s)| !s.is_empty())
            .collect();
        self.rows = kept;
    }

    /// Remove the rectangle (in-canvas) from the region.
    fn subtract_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        let x1 = x.saturating_add(w);
        let y1 = y.saturating_add(h);
        // Collect the affected rows first (avoids aliasing the map while
        // mutating entries).
        let affected: Vec<u32> = self.rows.range(y..y1).map(|(&ry, _)| ry).collect();
        for ry in affected {
            let Some(spans) = self.rows.get_mut(&ry) else {
                continue;
            };
            let mut out: Vec<(u32, u32)> = Vec::with_capacity(spans.len() + 1);
            for &(a, b) in spans.iter() {
                if b <= x || a >= x1 {
                    out.push((a, b));
                } else {
                    if a < x {
                        out.push((a, x));
                    }
                    if b > x1 {
                        out.push((x1, b));
                    }
                }
            }
            if out.is_empty() {
                self.rows.remove(&ry);
            } else {
                *spans = out;
            }
        }
    }

    /// Add the rectangle (in-canvas) to the region (union, merged).
    fn add_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        let x1 = x.saturating_add(w);
        let y1 = y.saturating_add(h);
        for ry in y..y1 {
            let spans = self.rows.entry(ry).or_default();
            spans.push((x, x1));
            *spans = merge_spans(spans);
        }
    }

    /// Shift every sample by `(dx, dy)` and clip to the canvas. Positions
    /// shifted outside the canvas are dropped.
    fn translate(&mut self, dx: i64, dy: i64, cw: u32, ch: u32) {
        if self.is_empty() {
            return;
        }
        let (cw, ch) = (i64::from(cw), i64::from(ch));
        let mut out: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
        for (&ry, spans) in &self.rows {
            let ny = i64::from(ry) + dy;
            if ny < 0 || ny >= ch {
                continue;
            }
            let mut shifted: Vec<(u32, u32)> = Vec::with_capacity(spans.len());
            for &(a, b) in spans {
                let na = i64::from(a) + dx;
                let nb = i64::from(b) + dx;
                if nb <= 0 || na >= cw {
                    continue;
                }
                let na = na.max(0) as u32;
                let nb = nb.min(cw) as u32;
                if na < nb {
                    shifted.push((na, nb));
                }
            }
            if !shifted.is_empty() {
                let merged = merge_spans(&mut shifted);
                out.insert(ny as u32, merged);
            }
        }
        self.rows = out;
    }

    /// Whether the sample is inside the region.
    fn contains(&self, x: u32, y: u32) -> bool {
        let Some(spans) = self.rows.get(&y) else {
            return false;
        };
        let i = spans.partition_point(|&(a, _)| a <= x);
        i > 0 && spans[i - 1].1 > x
    }

    /// Rows within `y0..y1` with their span slices.
    fn rows_in(&self, y0: u32, y1: u32) -> impl Iterator<Item = (u32, &[(u32, u32)])> {
        self.rows
            .range(y0..y1)
            .map(|(&ry, spans)| (ry, spans.as_slice()))
    }

    /// Every row with its span slices.
    fn rows(&self) -> impl Iterator<Item = (u32, &[(u32, u32)])> {
        self.rows.iter().map(|(&ry, spans)| (ry, spans.as_slice()))
    }
}

/// Merge a sorted-by-start list of spans (overlapping or adjacent) into
/// disjoint merged spans.
fn merge_spans(spans: &mut [(u32, u32)]) -> Vec<(u32, u32)> {
    if spans.is_empty() {
        return Vec::new();
    }
    spans.sort_unstable_by_key(|&(a, _)| a);
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(spans.len());
    let mut cur = spans[0];
    for &(a, b) in &spans[1..] {
        if a <= cur.1 {
            cur.1 = cur.1.max(b);
        } else {
            out.push(cur);
            cur = (a, b);
        }
    }
    out.push(cur);
    out
}

// ---------------------------------------------------------------------------
// Partial frame: a working raster over the bounding box of one level's demand
// region. Only demanded samples are written; samples outside the demand but
// inside the box hold the background fill (never read by a correct plan).
// ---------------------------------------------------------------------------

/// Working raster for one level: a `bw × bh` canvas whose `(0,0)` is frame
/// coordinate `(ox, oy)`.
struct PartialFrame {
    canvas: Canvas,
    ox: u32,
    oy: u32,
}

impl PartialFrame {
    /// Allocate a box-sized canvas initialized to `bg`.
    fn new(bbox: (u32, u32, u32, u32), bg: u8, limits: &Limits) -> Result<Self, VoleError> {
        let (ox, oy, bw, bh) = bbox;
        Ok(PartialFrame {
            canvas: Canvas::new(bw, bh, bg, limits)?,
            ox,
            oy,
        })
    }

    /// Write one sample (frame coordinates; the caller guarantees the sample
    /// is inside the demand region, hence inside this frame's box).
    #[inline]
    fn put(&mut self, x: u32, y: u32, v: u8, writes: &mut u64) {
        debug_assert!(x >= self.ox && x - self.ox < self.canvas.width());
        debug_assert!(y >= self.oy && y - self.oy < self.canvas.height());
        self.canvas.set(x - self.ox, y - self.oy, v);
        *writes += 1;
    }

    /// Read a sample (frame coordinates). `None` outside the box — callers
    /// only read positions a correct demand plan guarantees are inside.
    #[inline]
    fn get(&self, x: u32, y: u32) -> Option<u8> {
        if x < self.ox || y < self.oy {
            return None;
        }
        let lx = x - self.ox;
        let ly = y - self.oy;
        if lx >= self.canvas.width() || ly >= self.canvas.height() {
            return None;
        }
        Some(self.canvas.get(lx, ly))
    }

    /// Copy the in-box rectangle `(gx, gy, gw, gh)` out as a fresh canvas.
    fn crop(&self, gx: u32, gy: u32, gw: u32, gh: u32) -> Result<Canvas, VoleError> {
        let mut data = Vec::with_capacity(
            usize::try_from(u64::from(gw) * u64::from(gh))
                .map_err(|_| VoleError::ArithmeticOverflow)?,
        );
        for y in gy..gy.saturating_add(gh) {
            for x in gx..gx.saturating_add(gw) {
                let v = self.get(x, y).ok_or(VoleError::OutOfBounds)?;
                data.push(v);
            }
        }
        Canvas::from_parts(gw, gh, data)
    }
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

/// Measured work of one partial (or whole-frame) view materialization. All
/// fields are counts; nothing here is a rate or an entropy claim.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PartialStats {
    /// Number of frames replayed from the checkpoint to reach the target
    /// (state work is proportional to this, as in whole-frame decode).
    pub frames_replayed: u64,
    /// Number of levels that actually painted samples (levels whose demand
    /// region was empty are skipped).
    pub levels_materialized: u64,
    /// Gray8 sample writes performed (base + copy + residual passes). On the
    /// whole-frame path this is the canvas-sample coverage per replayed frame
    /// (a lower bound of physical writes; overpaints are not counted).
    pub painted_samples: u64,
    /// Writes in the base pass (background, instances, overlay).
    pub base_samples_written: u64,
    /// Writes performed by COPY_RECT / MOVE_RECT op application.
    pub copy_samples_written: u64,
    /// Writes performed by residual application.
    pub residual_samples_written: u64,
    /// Distinct immutable objects whose samples were actually painted for the
    /// view (the decode-time analogue of "object fetches": an object wholly
    /// outside the demanded region is never touched). Not tracked on the
    /// whole-frame path (reported 0).
    pub objects_touched: u64,
    /// Peak working raster memory of one level: the largest per-level demand
    /// bounding-box sample count.
    pub peak_view_samples: u64,
    /// Total per-level demand row spans across materialized levels
    /// (planning bookkeeping size).
    pub demand_spans_total: u64,
}

/// A materialized view: the exact crop of the whole-frame decode restricted
/// to the view's in-canvas region, plus measured work.
#[derive(Debug, Clone)]
pub struct PartialView {
    /// The crop. Its `(0,0)` is the view region's top-left in frame
    /// coordinates; width/height are the clipped in-canvas size.
    pub canvas: Canvas,
    /// Measured decode work.
    pub stats: PartialStats,
}

// ---------------------------------------------------------------------------
// View geometry → demand plan
// ---------------------------------------------------------------------------

/// Absolute in-canvas destination rectangle a COPY/MOVE op actually writes:
/// the dst pixels whose source *and* destination are inside the canvas (full
/// decode skips every other sample). Returns `None` when no pixel is written.
#[allow(clippy::too_many_arguments)] // 8 ordered geometry ints, like `rect_copy`
fn copy_write_rect(
    sx: i64,
    sy: i64,
    wd: u32,
    ht: u32,
    dx: i64,
    dy: i64,
    cw: u32,
    ch: u32,
) -> Option<(u32, u32, u32, u32)> {
    let (cw, ch) = (i64::from(cw), i64::from(ch));
    let u0 = 0i64.max(-sx).max(-dx);
    let u1 = i64::from(wd).min(cw - sx).min(cw - dx);
    let v0 = 0i64.max(-sy).max(-dy);
    let v1 = i64::from(ht).min(ch - sy).min(ch - dy);
    if u0 >= u1 || v0 >= v1 {
        return None;
    }
    Some((
        (dx + u0) as u32,
        (dy + v0) as u32,
        (u1 - u0) as u32,
        (v1 - v0) as u32,
    ))
}

/// Copy/move op fields, or `None` for a non-copy transition.
fn copy_fields(tr: &Transition) -> Option<(i64, i64, u32, u32, i64, i64)> {
    match tr {
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
    }
}

/// Frame `t`'s ops write only positions of frame `t−1` that frame `t` copies
/// from; everything else at `t` is painted from the state at `t`. This walks
/// backward from the target region collecting, per level, the exact region of
/// frame `t−1` that frame `t` will read (a union over copy/move ops of their
/// writable destination ∩ demand, shifted back to the source). Residuals are
/// self-contained and add no demand. Regions saturate to the whole canvas when
/// their span bookkeeping would exceed [`REGION_SPAN_BUDGET`] (sound
/// over-approximation, bounded memory).
fn plan_demands(parsed: &ParsedStream, idx: usize, clip: ViewBox, cw: u32, ch: u32) -> Vec<Region> {
    let mut demands: Vec<Region> = Vec::with_capacity(idx + 1);
    demands.resize_with(idx + 1, Region::default);
    demands[idx] = Region::from_rect(clip.x, clip.y, clip.width, clip.height);
    let intervals = parsed.intervals();
    for t in (1..=idx).rev() {
        if demands[t].is_empty() {
            // Nothing at this level needs an earlier frame; earlier levels are
            // empty by construction.
            break;
        }
        let group = &intervals[t - 1].1;
        let mut acc = Region::default();
        for tr in group {
            let Some((sx, sy, wd, ht, dx, dy)) = copy_fields(tr) else {
                continue;
            };
            let Some((wx, wy, ww, wh)) = copy_write_rect(sx, sy, wd, ht, dx, dy, cw, ch) else {
                continue;
            };
            // dst ∩ demand[t] (both in canvas), shifted to the source.
            let mut sub = demands[t].clone();
            sub.intersect_rect(wx, wy, ww, wh);
            sub.translate(sx - dx, sy - dy, cw, ch);
            if sub.is_empty() {
                continue;
            }
            // Union into the level accumulator, saturating on bookkeeping
            // blow-up.
            acc = merge_regions(acc, sub);
            if acc.span_count() > REGION_SPAN_BUDGET {
                acc.saturate_full(cw, ch);
                break;
            }
        }
        demands[t - 1] = acc;
        if demands[t - 1].span_count() > REGION_SPAN_BUDGET {
            demands[t - 1].saturate_full(cw, ch);
        }
    }
    demands
}

/// Union two regions (both canonical) into a canonical merged region.
fn merge_regions(a: Region, b: Region) -> Region {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    let mut out = a;
    for (y, spans) in b.rows() {
        for &(x0, x1) in spans {
            out.add_rect(x0, y, x1 - x0, 1);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Base painting (background, instances, overlay) inside a region
// ---------------------------------------------------------------------------

/// The paint source of one object instance: a Gray8 raster, a uniform fill, a
/// palette-index plane (resolved through the instance's binding), or a bounded
/// procedural generator. Mirror of the canonical kind resolution in
/// `materialize.rs`.
enum Kind<'a> {
    Raster(&'a [u8]),
    Fill(u8),
    Index(&'a [u8], &'a [u8]),
    Generator(crate::generator::Generator),
}

impl<'a> Kind<'a> {
    fn of(obj: &'a Object, state: &'a State, inst: &Instance) -> Result<Kind<'a>, VoleError> {
        if let Some(gen) = obj.generator() {
            return Ok(Kind::Generator(gen));
        }
        match obj.indices() {
            Some(indices) => {
                let palette_id = state.binding(inst.id).ok_or(VoleError::UnknownPalette)?;
                let entries = state.palette(palette_id).ok_or(VoleError::UnknownPalette)?;
                Ok(Kind::Index(indices, entries))
            }
            None => match obj.samples() {
                Some(raster) => Ok(Kind::Raster(raster)),
                None => Ok(Kind::Fill(obj.fill_value().unwrap_or(0))),
            },
        }
    }

    /// The value of the sample at object-local coordinate `(lx, ly)`.
    /// Mirrors the canonical per-kind semantics exactly (including the
    /// palette-index bounds error).
    #[inline]
    fn sample(&self, lx: u32, ly: u32, ow: u32) -> Result<u8, VoleError> {
        let i = usize::try_from(ly).map_err(|_| VoleError::ArithmeticOverflow)? * ow as usize
            + lx as usize;
        match self {
            Kind::Raster(raster) => Ok(raster[i]),
            Kind::Fill(v) => Ok(*v),
            Kind::Index(indices, entries) => {
                let idx = indices[i];
                if usize::from(idx) >= entries.len() {
                    return Err(VoleError::OutOfBounds);
                }
                Ok(entries[usize::from(idx)])
            }
            Kind::Generator(gen) => Ok(gen.sample(i64::from(lx), i64::from(ly))),
        }
    }
}

/// Paint a non-affine instance's object into `part` wherever its box
/// intersects the paint region. Budgets/charges mirror `materialize_full`.
fn paint_instance_partial(
    part: &mut PartialFrame,
    state: &State,
    obj: &Object,
    inst: &Instance,
    region: &Region,
    writes: &mut u64,
    touched: &mut HashSet<u32>,
) -> Result<(), VoleError> {
    let ow = obj.width();
    let oh = obj.height();
    let (dx, dy) = (inst.x, inst.y);
    let cw = i64::from(part.canvas.width()) + i64::from(part.ox);
    let ch = i64::from(part.canvas.height()) + i64::from(part.oy);
    // Object box clipped to the canvas.
    let bx0 = dx.max(0);
    let by0 = dy.max(0);
    let bx1 = (dx + i64::from(ow)).min(cw);
    let by1 = (dy + i64::from(oh)).min(ch);
    if bx0 >= bx1 || by0 >= by1 {
        return Ok(()); // box entirely outside the canvas: nothing to paint
    }
    let (bx0, by0, bx1, by1) = (bx0 as u32, by0 as u32, bx1 as u32, by1 as u32);
    if !region.intersects_box(bx0, by0, bx1, by1) {
        return Ok(()); // outside the demanded region: never touched (audit-scope)
    }
    let kind = Kind::of(obj, state, inst)?;
    let mut painted_any = false;
    for (y, spans) in region.rows_in(by0, by1) {
        let sy = i64::from(y) - dy; // object row (>= 0 inside by0..by1)
        let sy = u32::try_from(sy).map_err(|_| VoleError::ArithmeticOverflow)?;
        for &(a, b) in spans {
            let l = a.max(bx0);
            let r = b.min(bx1);
            if l >= r {
                continue;
            }
            for x in l..r {
                let lx = (i64::from(x) - dx) as u32;
                let v = kind.sample(lx, sy, ow)?;
                part.put(x, y, v, writes);
                painted_any = true;
            }
        }
    }
    if painted_any {
        touched.insert(inst.object_id.0);
    }
    Ok(())
}

/// Paint an affine instance (Phase L) inside the region: every demanded pixel
/// samples the object through the canonical Q8 source map; pixels whose source
/// falls inside the object box overwrite. Budget semantics mirror
/// `materialize_full` (charged by the caller).
#[allow(clippy::too_many_arguments)] // context + object + region + counters
fn paint_affine_partial(
    part: &mut PartialFrame,
    state: &State,
    obj: &Object,
    inst: &Instance,
    params: crate::affine::AffineParams,
    region: &Region,
    writes: &mut u64,
    touched: &mut HashSet<u32>,
) -> Result<(), VoleError> {
    let ow = i64::from(obj.width());
    let oh = i64::from(obj.height());
    let kind = Kind::of(obj, state, inst)?;
    let mut painted_any = false;
    for (y, spans) in region.rows() {
        for &(a, b) in spans {
            for x in a..b {
                let (su, sv) = params
                    .source(i64::from(x), i64::from(y))
                    .ok_or(VoleError::ArithmeticOverflow)?;
                if su < 0 || sv < 0 || su >= ow || sv >= oh {
                    continue;
                }
                let v = kind.sample(su as u32, sv as u32, obj.width())?;
                part.put(x, y, v, writes);
                painted_any = true;
            }
        }
    }
    if painted_any {
        touched.insert(inst.object_id.0);
    }
    Ok(())
}

/// Paint the base state (background over the region, every instance in paint
/// order over its intersection with the region, then the overlay) into `part`.
/// Instance/affine budget accounting mirrors `materialize_full` exactly so
/// whole-frame and partial materializations share one envelope.
#[allow(clippy::too_many_arguments)] // frame + state + region + counters
fn paint_base(
    part: &mut PartialFrame,
    state: &State,
    region: &Region,
    width: u32,
    height: u32,
    limits: &Limits,
    writes: &mut u64,
    touched: &mut HashSet<u32>,
) -> Result<(), VoleError> {
    // Background across the region.
    let bg = state.background();
    for (y, spans) in region.rows() {
        for &(a, b) in spans {
            for x in a..b {
                part.put(x, y, bg, writes);
            }
        }
    }
    // Instances in paint order (budgets identical to materialize_full).
    let mut acc = 0u64;
    let mut affine_work = 0u64;
    let affine_sample_cost = u64::from(width) * u64::from(height);
    for inst in state.instances() {
        acc = acc.saturating_add(1);
        if acc > u64::from(limits.max_instances) {
            return Err(VoleError::MaterializationBudgetExceeded);
        }
        if let Some(params) = state.affine(inst.id) {
            affine_work = affine_work.saturating_add(affine_sample_cost);
            if affine_work > limits.max_affine_work {
                return Err(VoleError::MaterializationBudgetExceeded);
            }
            if let Some(obj) = state.object(inst.object_id) {
                paint_affine_partial(part, state, obj, inst, params, region, writes, touched)?;
            }
            continue;
        }
        let Some(obj) = state.object(inst.object_id) else {
            continue; // mirrors paint_instance's total skip for a missing object
        };
        paint_instance_partial(part, state, obj, inst, region, writes, touched)?;
    }
    // Sparse overlay above every instance (region-filtered).
    for (x, y, v) in state.overlay_iter() {
        if x < 0 || y < 0 {
            continue;
        }
        let (x, y) = (x as u32, y as u32);
        if region.contains(x, y) {
            part.put(x, y, v, writes);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Canvas-op application inside a region
// ---------------------------------------------------------------------------

/// Validate a kind-0/1 residual payload the way the canonical materializer
/// does (bounds + strict ascending order) and apply the points that fall
/// inside `region`. Bounds are validated against the **canvas**, never the
/// partial frame's box, so validation is view-independent. Full validation
/// keeps the error surface of a residual op identical across views; only the
/// writes are region-filtered.
fn apply_sparse_residual(
    part: &mut PartialFrame,
    block: &[u8],
    limits: &Limits,
    region: &Region,
    writes: &mut u64,
    cw: u32,
    ch: u32,
) -> Result<(), VoleError> {
    let payload = crate::rans::decode_block(block, limits.max_residual_bytes)?;
    if payload.len() % 9 != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let (cw, ch) = (i64::from(cw), i64::from(ch));
    let mut prev: Option<(i64, i64)> = None;
    for p in payload.as_chunks::<9>().0 {
        let x = i64::from(i32::from_le_bytes([p[0], p[1], p[2], p[3]]));
        let y = i64::from(i32::from_le_bytes([p[4], p[5], p[6], p[7]]));
        let v = p[8];
        if x < 0 || y < 0 || x >= cw || y >= ch {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let key = (x, y);
        if prev.is_some_and(|q| key <= q) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        prev = Some(key);
        if region.contains(x as u32, y as u32) {
            part.put(x as u32, y as u32, v, writes);
        }
    }
    Ok(())
}

/// Apply a kind-2 transform residual (Phase M) inside the region. The whole
/// container is structurally validated against the **canvas** geometry and the
/// DC/AC streams are decoded (the same bounded work the canonical materializer
/// does); the inverse transform and its additive write are applied only where
/// the 4×4 tile intersects the demanded region.
fn apply_transform_residual_partial(
    part: &mut PartialFrame,
    block: &[u8],
    limits: &Limits,
    region: &Region,
    writes: &mut u64,
    cw: u32,
    ch: u32,
) -> Result<(), VoleError> {
    if block.len() < 2 {
        return Err(VoleError::Truncated);
    }
    if block[1] != crate::transform::TRANSFORM_ID_4X4 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let (bx, by) = crate::transform::blocks_per_axis(cw, ch);
    let nblocks = bx.checked_mul(by).ok_or(VoleError::ArithmeticOverflow)?;
    let mlen = crate::transform::mask_len(cw, ch);
    let o = 2usize
        .checked_add(mlen)
        .ok_or(VoleError::ArithmeticOverflow)?;
    if block.len() < o + 8 {
        return Err(VoleError::Truncated);
    }
    let mask = &block[2..o];
    let used = nblocks % 8;
    if used != 0 && mask[mlen - 1] & !((1u8 << used) - 1) != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let dc_len = u64::from(u32::from_le_bytes([
        block[o],
        block[o + 1],
        block[o + 2],
        block[o + 3],
    ]));
    let ac_len = u64::from(u32::from_le_bytes([
        block[o + 4],
        block[o + 5],
        block[o + 6],
        block[o + 7],
    ]));
    if dc_len > limits.max_residual_bytes || ac_len > limits.max_residual_bytes {
        return Err(VoleError::DimensionTooLarge);
    }
    let total = o as u64 + 8 + dc_len + ac_len;
    if total != block.len() as u64 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let dc_off = o + 8;
    let ac_off = dc_off
        .checked_add(dc_len as usize)
        .ok_or(VoleError::ArithmeticOverflow)?;
    let dc_payload = crate::rans::decode_block(&block[dc_off..ac_off], limits.max_residual_bytes)?;
    let ac_payload = crate::rans::decode_block(&block[ac_off..], limits.max_residual_bytes)?;
    if dc_payload.len() % 4 != 0 || ac_payload.len() % 60 != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let coded: usize = mask.iter().map(|b| b.count_ones() as usize).sum();
    if dc_payload.len() / 4 != coded || ac_payload.len() / 60 != coded {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let dc4 = dc_payload.as_chunks::<4>().0;
    let ac60 = ac_payload.as_chunks::<60>().0;
    let (cw, ch) = (i64::from(cw), i64::from(ch));
    let mut block_i = 0usize;
    for k in 0..nblocks {
        if mask[k >> 3] & (1 << (k & 7)) == 0 {
            continue;
        }
        let mut coeffs = [0i32; 16];
        let dcb = dc4[block_i];
        coeffs[0] =
            crate::transform::unzigzag(u32::from_le_bytes([dcb[0], dcb[1], dcb[2], dcb[3]]));
        let acb = ac60[block_i];
        for j in 0..15 {
            let z =
                u32::from_le_bytes([acb[4 * j], acb[4 * j + 1], acb[4 * j + 2], acb[4 * j + 3]]);
            coeffs[j + 1] = crate::transform::unzigzag(z);
        }
        block_i += 1;
        let (kxx, kyy) = (k % bx, k / bx);
        // Skip tiles whose in-canvas pixels are entirely outside the demand.
        let (tx0, ty0) = (
            u32::try_from(kxx * crate::transform::BLOCK).unwrap_or(u32::MAX),
            u32::try_from(kyy * crate::transform::BLOCK).unwrap_or(u32::MAX),
        );
        let (tx1, ty1) = (
            tx0.saturating_add(crate::transform::BLOCK as u32)
                .min(part.canvas.width() + part.ox),
            ty0.saturating_add(crate::transform::BLOCK as u32)
                .min(part.canvas.height() + part.oy),
        );
        if !region.intersects_box(tx0, ty0, tx1, ty1) {
            continue;
        }
        let samples = crate::transform::inverse_block(&coeffs);
        for vy in 0..4i64 {
            let gy = i64::try_from(kyy).unwrap_or(i64::MAX) * 4 + vy;
            if gy < 0 || gy >= ch {
                continue;
            }
            for vx in 0..4i64 {
                let gx = i64::try_from(kxx).unwrap_or(i64::MAX) * 4 + vx;
                if gx < 0 || gx >= cw {
                    continue;
                }
                let (gx, gy) = (gx as u32, gy as u32);
                if !region.contains(gx, gy) {
                    continue;
                }
                let r = samples[(vy * 4 + vx) as usize];
                let cur = i64::from(part.get(gx, gy).ok_or(VoleError::OutOfBounds)?);
                let nv = cur + r;
                if !(0..=255).contains(&nv) {
                    return Err(VoleError::OutOfBounds);
                }
                part.put(gx, gy, nv as u8, writes);
            }
        }
    }
    Ok(())
}

/// Apply one canvas op restricted to `region` (COPY/MOVE read the previous
/// level's partial frame; residuals are self-contained). Mirrors the order and
/// value semantics of the canonical op application.
#[allow(clippy::too_many_arguments)] // frame + op context + counters
fn apply_op_partial(
    part: &mut PartialFrame,
    prev: Option<&PartialFrame>,
    tr: &Transition,
    region: &Region,
    cw: u32,
    ch: u32,
    limits: &Limits,
    copy_writes: &mut u64,
    residual_writes: &mut u64,
) -> Result<(), VoleError> {
    if let Some((sx, sy, wd, ht, dx, dy)) = copy_fields(tr) {
        let Some((wx, wy, ww, wh)) = copy_write_rect(sx, sy, wd, ht, dx, dy, cw, ch) else {
            return Ok(()); // nothing writable
        };
        if !region.intersects_box(wx, wy, wx.saturating_add(ww), wy.saturating_add(wh)) {
            return Ok(()); // no demanded destination pixel: nothing to apply
        }
        let Some(prev) = prev else {
            // A correct demand plan guarantees reads exist whenever a write
            // exists; reaching here would be an internal invariant break and
            // must never silently produce wrong pixels.
            return Err(VoleError::ApiConstraint(
                "partial decode missing previous-level history",
            ));
        };
        for (y, spans) in region.rows_in(wy, wy.saturating_add(wh)) {
            for &(a, b) in spans {
                let l = a.max(wx);
                let r = b.min(wx.saturating_add(ww));
                if l >= r {
                    continue;
                }
                for x in l..r {
                    let u = i64::from(x) - dx;
                    let v = i64::from(y) - dy;
                    let src_x = sx + u;
                    let src_y = sy + v;
                    if let Some(value) = prev.get(src_x as u32, src_y as u32) {
                        part.put(x, y, value, copy_writes);
                    }
                    // Out-of-previous-box source cannot occur for a demanded
                    // destination; skipped defensively like an out-of-canvas
                    // source in whole-frame decode.
                }
            }
        }
        return Ok(());
    }
    if let Transition::Residual { block } = tr {
        if block.first() == Some(&crate::rans::KIND_TSF) {
            apply_transform_residual_partial(part, block, limits, region, residual_writes, cw, ch)
        } else {
            apply_sparse_residual(part, block, limits, region, residual_writes, cw, ch)
        }
    } else {
        Err(VoleError::NonCanonicalEncoding)
    }
}

// ---------------------------------------------------------------------------
// Whole-frame canonical path (View::FullFrame and full-canvas boxes)
// ---------------------------------------------------------------------------

/// Replay the checkpoint and the first `idx` intervals through the canonical
/// step machinery (`materialize_full` + `decoder::step_frame`) and return
/// frame `idx`. This is byte- and error-identical to whole-frame decode by
/// construction, so the FullFrame view keeps the canonical audit contract.
fn whole_frame_at(parsed: &ParsedStream, idx: usize) -> Result<Canvas, VoleError> {
    let w = parsed.width();
    let h = parsed.height();
    let limits = parsed.limits();
    let mut state = parsed.clone_initial();
    let mut frame = crate::materialize::materialize_full(&state, w, h, limits)?;
    for (k, (_, trs)) in parsed.intervals().iter().enumerate() {
        if k + 1 > idx {
            break;
        }
        frame = decoder::step_frame(&mut state, &frame, trs, w, h, limits)?;
    }
    Ok(frame)
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Materialize frame `idx` restricted to `view`.
///
/// The returned canvas holds exactly the samples a whole-frame decode of
/// `idx` would place in the view's in-canvas region (origin at the region's
/// top-left), so every view agrees with whole-frame decode sample-for-sample.
/// A `FullFrame` (or any box covering the whole canvas) request replays the
/// canonical step machinery; a sub-frame request runs the demand-planned
/// partial decoder. See the module docs for the audit-scope semantics.
pub fn materialize_view(
    parsed: &ParsedStream,
    idx: u64,
    view: View,
) -> Result<PartialView, VoleError> {
    if idx >= parsed.frame_count() {
        return Err(VoleError::OutOfBounds);
    }
    let w = parsed.width();
    let h = parsed.height();
    let clip = view.clip(w, h)?.ok_or(VoleError::ApiConstraint(
        "view does not intersect the canvas",
    ))?;
    if clip.is_full(w, h) {
        // Canonical whole-frame path (byte- and error-identical to
        // whole-frame decode).
        let canvas = whole_frame_at(parsed, idx as usize)?;
        let area = clip.sample_count();
        let frames = idx + 1;
        let stats = PartialStats {
            frames_replayed: frames,
            levels_materialized: frames,
            painted_samples: frames * area,
            base_samples_written: frames * area,
            copy_samples_written: 0,
            residual_samples_written: 0,
            objects_touched: 0,
            peak_view_samples: area,
            demand_spans_total: 0,
        };
        return Ok(PartialView { canvas, stats });
    }
    partial_at(parsed, idx as usize, clip)
}

/// Demand-planned partial decode for a strict sub-frame view.
fn partial_at(parsed: &ParsedStream, idx: usize, clip: ViewBox) -> Result<PartialView, VoleError> {
    let w = parsed.width();
    let h = parsed.height();
    let limits = parsed.limits();
    let demands = plan_demands(parsed, idx, clip, w, h);

    let intervals = parsed.intervals();
    let mut state = parsed.clone_initial();
    let mut acc = PartialStats::default();
    let mut touched: HashSet<u32> = HashSet::new();
    let mut prev: Option<PartialFrame> = None;

    // Per-level write subcounts accumulated into `acc`.
    let mut base_writes = 0u64;
    let mut copy_writes = 0u64;
    let mut residual_writes = 0u64;

    for t in 0..=idx {
        // Separate canvas ops from state transitions (mirror step_frame).
        let mut ops: Vec<&Transition> = Vec::new();
        if t > 0 {
            for tr in &intervals[t - 1].1 {
                if decoder::is_canvas_op(tr) {
                    ops.push(tr);
                } else {
                    tr.apply(&mut state)?;
                }
            }
        }
        acc.frames_replayed = t as u64 + 1;
        let demand = &demands[t];
        if demand.is_empty() {
            // Nothing at this level is needed: no paint, no op application.
            // Later levels cannot read it (their reads are subsets of this
            // level's demand, which is empty).
            prev = None;
            continue;
        }
        // Base paint region: the demand minus every writable copy destination
        // (those pixels are fully overwritten by the ops).
        let mut paint = demand.clone();
        for tr in &ops {
            if let Some((sx, sy, wd, ht, dx, dy)) = copy_fields(tr) {
                if let Some((wx, wy, ww, wh)) = copy_write_rect(sx, sy, wd, ht, dx, dy, w, h) {
                    paint.subtract_rect(wx, wy, ww, wh);
                }
            }
        }
        let bbox = demand.bbox().expect("non-empty demand has a bbox");
        let mut part = PartialFrame::new(bbox, state.background(), limits)?;
        paint_base(
            &mut part,
            &state,
            &paint,
            w,
            h,
            limits,
            &mut base_writes,
            &mut touched,
        )?;
        if t > 0 {
            for tr in &ops {
                apply_op_partial(
                    &mut part,
                    prev.as_ref(),
                    tr,
                    demand,
                    w,
                    h,
                    limits,
                    &mut copy_writes,
                    &mut residual_writes,
                )?;
            }
        }
        acc.levels_materialized += 1;
        acc.demand_spans_total = acc
            .demand_spans_total
            .saturating_add(demand.span_count() as u64);
        let bbox_area = u64::from(bbox.2).saturating_mul(u64::from(bbox.3));
        acc.peak_view_samples = acc.peak_view_samples.max(bbox_area);
        prev = Some(part);
    }

    let part = prev.ok_or(VoleError::OutOfBounds)?;
    // The demanded region of the target level is exactly the clip rectangle,
    // so crop it out of the level's box.
    let canvas = part.crop(clip.x, clip.y, clip.width, clip.height)?;
    acc.painted_samples = base_writes + copy_writes + residual_writes;
    acc.base_samples_written = base_writes;
    acc.copy_samples_written = copy_writes;
    acc.residual_samples_written = residual_writes;
    acc.objects_touched = touched.len() as u64;
    Ok(PartialView { canvas, stats: acc })
}

/// State-level crop (no timeline): materialize the exact state samples inside
/// `box` (already canvas-clipped). Used by [`crate::materialize::materialize`]
/// for `View::Rect` / `View::Tile`; equivalent to `materialize_view(frame 0)`
/// restricted to the box when no canvas op ever reads earlier frames.
pub(crate) fn state_crop(
    state: &State,
    box_: ViewBox,
    width: u32,
    height: u32,
    limits: &Limits,
) -> Result<Canvas, VoleError> {
    let region = Region::from_rect(box_.x, box_.y, box_.width, box_.height);
    let mut part = PartialFrame::new(
        (box_.x, box_.y, box_.width, box_.height),
        state.background(),
        limits,
    )?;
    let mut writes = 0u64;
    let mut touched: HashSet<u32> = HashSet::new();
    paint_base(
        &mut part,
        state,
        &region,
        width,
        height,
        limits,
        &mut writes,
        &mut touched,
    )?;
    debug_assert!(writes > 0);
    part.crop(box_.x, box_.y, box_.width, box_.height)
}
