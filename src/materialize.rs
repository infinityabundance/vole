//! The deterministic materializer: `state → raster view`.
//!
//! Materialization is the *only* place a raster is produced. It takes a
//! [`State`], the canonical canvas geometry (declared by the stream header),
//! and a requested [`View`] and returns requested samples. It performs no
//! search and makes no lossy choices — its semantics must be byte-for-byte
//! reproducible across implementations, which is why they are specified here
//! and mirrored by an independent reference used by the conformance court.

use crate::{
    error::VoleError, limits::Limits, object::Object, pixel::Canvas, state::Instance, state::State,
    view::View,
};

/// Result of a materialization for the canonical view.
#[derive(Debug, Clone)]
pub struct MaterializedFrame {
    /// The Gray8 full-frame raster.
    pub canvas: Canvas,
}

/// Copy a rectangle from `src` (a fully materialized prior full frame) into
/// `dst` (the current base frame). Canonical: iterate the declared source
/// rectangle, reading source samples before any dst write (two distinct
/// buffers in COPY_RECT use; caller guarantees no destructive aliasing), and
/// clip every sample to be in-bounds for BOTH frames so the declared geometry
/// may extend beyond the canvas harmlessly. Area bounded at parse time.
#[allow(clippy::too_many_arguments)] // 8 ordered ints: (src x/y, w/h, dst x/y) — grouped only if a CopyGeom struct is introduced
pub fn rect_copy(
    dst: &mut Canvas,
    src: &Canvas,
    sx: i64,
    sy: i64,
    width: u32,
    height: u32,
    dx: i64,
    dy: i64,
) {
    let dw = i64::from(dst.width());
    let dh = i64::from(dst.height());
    let sw = i64::from(src.width());
    let sh = i64::from(src.height());
    for si in 0..height as i64 {
        for sj in 0..width as i64 {
            let px = sx + sj;
            let py = sy + si;
            if px < 0 || py < 0 || px >= sw || py >= sh {
                continue;
            }
            let qx = dx + sj;
            let qy = dy + si;
            if qx < 0 || qy < 0 || qx >= dw || qy >= dh {
                continue;
            }
            let v = src.get(u32::try_from(px).unwrap(), u32::try_from(py).unwrap());
            dst.set(u32::try_from(qx).unwrap(), u32::try_from(qy).unwrap(), v);
        }
    }
}

pub(crate) fn apply_copy(
    dst: &mut Canvas,
    src: &Canvas,
    tr: &crate::transition::Transition,
) -> Result<(), VoleError> {
    let (sx, sy, width, height, dx, dy) = match tr {
        crate::transition::Transition::CopyRect {
            src_x,
            src_y,
            width,
            height,
            dst_x,
            dst_y,
        }
        | crate::transition::Transition::MoveRect {
            src_x,
            src_y,
            width,
            height,
            dst_x,
            dst_y,
        } => (*src_x, *src_y, *width, *height, *dst_x, *dst_y),
        other => {
            let _ = other;
            return Err(VoleError::NonCanonicalEncoding);
        }
    };
    rect_copy(dst, src, sx, sy, width, height, dx, dy);
    Ok(())
}

/// Decode and apply a per-frame residual block onto `dst` (Phase G). A
/// kind-0/1 block (RAW/rANS) decodes to a canonical, strict-sorted,
/// in-canvas sparse point list; a kind-2 block (Phase M) is a transform-coded
/// additive residual (see `crate::transform`). Any deviation is a typed
/// error. This is the `⊕_ρ` residual algebra applied to the materialized
/// base: it is authoritative for the frame and has no persistent-state side
/// effect.
pub(crate) fn apply_residual(
    dst: &mut Canvas,
    block: &[u8],
    limits: &Limits,
) -> Result<(), VoleError> {
    if block.first() == Some(&crate::rans::KIND_TSF) {
        return apply_transform_residual(dst, block, limits);
    }
    let payload = crate::rans::decode_block(block, limits.max_residual_bytes)?;
    if payload.len() % 9 != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let cw = i64::from(dst.width());
    let ch = i64::from(dst.height());
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
        dst.set(
            u32::try_from(x).expect("bounded above by width"),
            u32::try_from(y).expect("bounded above by height"),
            v,
        );
    }
    Ok(())
}

/// Apply a transform-coded residual (Phase M, kind 2) onto `dst`: every coded
/// `4×4` block is inverse-transformed (normative integer lifting DCT) and its
/// reconstructed samples are **added** to the canvas; a result outside the
/// Gray8 domain `0..=255` is `OutOfBounds` (typed). Structure, mask padding,
/// coefficient counts, and container framing are all canonical-checked here;
/// hostile input resolves to a typed error, never a panic.
fn apply_transform_residual(
    dst: &mut Canvas,
    block: &[u8],
    limits: &Limits,
) -> Result<(), VoleError> {
    if block.len() < 2 {
        return Err(VoleError::Truncated);
    }
    if block[1] != crate::transform::TRANSFORM_ID_4X4 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let (w, h) = (dst.width(), dst.height());
    let (bx, by) = crate::transform::blocks_per_axis(w, h);
    let nblocks = bx.checked_mul(by).ok_or(VoleError::ArithmeticOverflow)?;
    let mlen = crate::transform::mask_len(w, h);
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
    let mut block_i = 0usize;
    let cw = i64::from(w);
    let ch = i64::from(h);
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
        let samples = crate::transform::inverse_block(&coeffs);
        let (kxx, kyy) = (k % bx, k / bx);
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
                let r = samples[(vy * 4 + vx) as usize];
                let cur = i64::from(dst.get(gx as u32, gy as u32));
                let nv = cur + r;
                if !(0..=255).contains(&nv) {
                    return Err(VoleError::OutOfBounds);
                }
                dst.set(gx as u32, gy as u32, nv as u8);
            }
        }
    }
    Ok(())
}

/// Apply one canvas op (copy/move/residual) to the current frame canvas.
/// Copy ops read their source from `prev` (the immediately previous decoded
/// frame); the residual op is self-contained. Ops apply in listed order.
pub(crate) fn apply_canvas_op(
    dst: &mut Canvas,
    prev: &Canvas,
    tr: &crate::transition::Transition,
    limits: &Limits,
) -> Result<(), VoleError> {
    match tr {
        crate::transition::Transition::CopyRect { .. }
        | crate::transition::Transition::MoveRect { .. } => apply_copy(dst, prev, tr),
        crate::transition::Transition::Residual { block } => apply_residual(dst, block, limits),
        other => {
            let _ = other;
            Err(VoleError::NonCanonicalEncoding)
        }
    }
}
/// Materialize the canonical full frame of `state`.
///
/// # Semantics (normative)
///
/// 1. Allocate a `width x height` canvas and fill it with `state.background`.
/// 2. For each live instance in paint order, paint its immutable object over
///    the canvas using the object's top-left at the instance `(x, y)`; pixels
///    overwrite. Portions outside the canvas are clipped. A palette-index
///    object paints by resolving every stored index through the palette bound
///    to the instance (Phase J): a missing binding/palette is `UnknownPalette`
///    and an index at or beyond the palette length is `OutOfBounds` — both
///    typed, deterministic errors, never a wrap. When the instance carries an
///    affine placement (Phase L), painting instead scans every canvas pixel,
///    samples the object through the canonical Q8 source map
///    `(su, sv) = ((a·x+b·y+c) >> 8, (d·x+e·y+f) >> 8)`, and overwrites the
///    pixel when the source lies inside the object rectangle; the plain
///    placement `(x, y)` is dormant until the affine deactivates. The object's
///    kind semantics (fill value / raster sample / bound-palette entry lookup)
///    are identical under both placement rules. Total per-materialization
///    affine sample work is capped by `Limits.max_affine_work` (typed
///    `MaterializationBudgetExceeded` beyond it).
/// 3. The returned canvas is the exact full-frame view of `state`.
pub fn materialize_full(
    state: &State,
    width: u32,
    height: u32,
    limits: &Limits,
) -> Result<Canvas, VoleError> {
    limits.check_canvas(width, height)?;
    let mut canvas = Canvas::new(width, height, state.background(), limits)?;
    let mut acc = 0u64;
    let mut affine_work = 0u64;
    let affine_sample_cost = u64::from(width) * u64::from(height);
    for inst in state.instances() {
        acc = acc.saturating_add(1);
        if acc > u64::from(limits.max_instances) {
            return Err(VoleError::MaterializationBudgetExceeded);
        }
        if state.affine(inst.id).is_some() {
            // An affine placement scans the whole canvas; bound the total
            // per-materialization affine sample work before doing it.
            affine_work = affine_work.saturating_add(affine_sample_cost);
            if affine_work > limits.max_affine_work {
                return Err(VoleError::MaterializationBudgetExceeded);
            }
        }
        paint_instance(&mut canvas, state, inst)?;
    }
    // Sparse overlay: authoritative pixels painted above all instances, in
    // canonical coordinate order. Out-of-canvas coordinates are dropped.
    let (cw, ch) = (i64::from(width), i64::from(height));
    for (x, y, v) in state.overlay_iter() {
        if x < 0 || y < 0 || x >= cw || y >= ch {
            continue;
        }
        canvas.set(u32::try_from(x).unwrap(), u32::try_from(y).unwrap(), v);
    }
    Ok(canvas)
}

/// Bounded helper reused by per-frame and (later) partial views.
fn paint_instance(canvas: &mut Canvas, state: &State, inst: &Instance) -> Result<(), VoleError> {
    match state.object(inst.object_id) {
        Some(obj) => paint_object(canvas, state, obj, inst),
        None => {
            // Instances are only insertable when their object is declared, so
            // an absent object here is a state invariant broken by a buggy
            // builder; treat as skip to stay total & non-panicking. Decoders
            // additionally enforce this at transition apply time.
            Ok(())
        }
    }
}

fn paint_object(
    canvas: &mut Canvas,
    state: &State,
    obj: &Object,
    inst: &Instance,
) -> Result<(), VoleError> {
    if let Some(params) = state.affine(inst.id) {
        return paint_affine(canvas, state, obj, inst, params);
    }
    let dx = inst.x;
    let dy = inst.y;
    let w = obj.width();
    let h = obj.height();
    if let Some(gen) = obj.generator() {
        // Phase N: compute every sample of the painted box from the bounded
        // integer program (clipped to the canvas, exactly like a fill).
        return paint_generator(canvas, gen, w, h, dx, dy);
    }
    match obj.indices() {
        // Palette-index raster: resolve every index through the instance's
        // bound palette. Unbound instance or undeclared palette => typed
        // error; index >= palette length => typed error (never a wrap).
        Some(indices) => {
            let palette_id = state.binding(inst.id).ok_or(VoleError::UnknownPalette)?;
            let entries = state.palette(palette_id).ok_or(VoleError::UnknownPalette)?;
            paint_index_raster(canvas, indices, w, h, dx, dy, entries)
        }
        None => match obj.samples() {
            Some(raster) => {
                canvas.blit(raster, w, h, dx, dy);
                Ok(())
            }
            None => {
                // Uniform fill object.
                let value = obj.fill_value().unwrap_or(0);
                canvas.fill_rect_clipped(value, dx, dy, dx + i64::from(w), dy + i64::from(h));
                Ok(())
            }
        },
    }
}

/// Affine placement painter (Phase L): scan every destination pixel of the
/// canvas, compute the canonical Q8 source sample, and paint the object
/// sample when the source lies inside the object rectangle. Deterministic and
/// integer throughout; the per-materialization sample work is bounded by
/// `Limits.max_affine_work` (checked by the caller before this runs).
fn paint_affine(
    canvas: &mut Canvas,
    state: &State,
    obj: &Object,
    inst: &Instance,
    params: crate::affine::AffineParams,
) -> Result<(), VoleError> {
    let ow = i64::from(obj.width());
    let oh = i64::from(obj.height());
    // Resolve the sample source once: a Gray8 raster, a fill value, a
    // palette-entry lookup (index rasters need the instance's binding), or a
    // bounded procedural program (Phase N: the sampled source value is the
    // generator's value at the source coordinate).
    enum Kind<'a> {
        Raster(&'a [u8]),
        Fill(u8),
        Index(&'a [u8], &'a [u8]), // indices + palette entries
        Generator(crate::generator::Generator),
    }
    let kind: Kind<'_> = if let Some(gen) = obj.generator() {
        Kind::Generator(gen)
    } else {
        match obj.indices() {
            Some(indices) => {
                let palette_id = state.binding(inst.id).ok_or(VoleError::UnknownPalette)?;
                let entries = state.palette(palette_id).ok_or(VoleError::UnknownPalette)?;
                Kind::Index(indices, entries)
            }
            None => match obj.samples() {
                Some(raster) => Kind::Raster(raster),
                None => Kind::Fill(obj.fill_value().unwrap_or(0)),
            },
        }
    };
    let cw = i64::from(canvas.width());
    let ch = i64::from(canvas.height());
    for y in 0..ch {
        for x in 0..cw {
            let (su, sv) = params.source(x, y).ok_or(VoleError::ArithmeticOverflow)?;
            if su < 0 || sv < 0 || su >= ow || sv >= oh {
                continue;
            }
            let v = match kind {
                Kind::Raster(raster) => raster[(sv * ow + su) as usize],
                Kind::Fill(value) => value,
                Kind::Generator(gen) => gen.sample(su, sv),
                Kind::Index(indices, entries) => {
                    let idx = indices[(sv * ow + su) as usize];
                    if usize::from(idx) >= entries.len() {
                        return Err(VoleError::OutOfBounds);
                    }
                    entries[usize::from(idx)]
                }
            };
            canvas.set(
                u32::try_from(x).expect("x in canvas"),
                u32::try_from(y).expect("y in canvas"),
                v,
            );
        }
    }
    Ok(())
}

/// Paint a procedural generator object (Phase N): compute every sample of the
/// declared box from the bounded integer program. Bounds behave exactly like
/// a fill blit (clipped at the borders, out-of-canvas dropped); work is one
/// sample per painted pixel, the same class as a raster blit.
fn paint_generator(
    canvas: &mut Canvas,
    gen: crate::generator::Generator,
    w: u32,
    h: u32,
    dx: i64,
    dy: i64,
) -> Result<(), VoleError> {
    let cw = i64::from(canvas.width());
    let ch = i64::from(canvas.height());
    let y0 = dy.max(0);
    let y1 = (dy + i64::from(h)).min(ch);
    let x0 = dx.max(0);
    let x1 = (dx + i64::from(w)).min(cw);
    if y0 >= y1 || x0 >= x1 {
        return Ok(());
    }
    for cty in y0..y1 {
        let ly = cty - dy;
        for ctox in x0..x1 {
            let lx = ctox - dx;
            canvas.set(
                u32::try_from(ctox).unwrap(),
                u32::try_from(cty).unwrap(),
                gen.sample(lx, ly),
            );
        }
    }
    Ok(())
}

/// Palette-index blit: overwrite the box's clipped rectangle with
/// `entries[idx]` for every stored index `idx`. Bounds behave exactly like
/// `Canvas::blit` (clipped at the borders, out-of-canvas dropped); every
/// index is validated against the palette length before it is written.
fn paint_index_raster(
    canvas: &mut Canvas,
    indices: &[u8],
    w: u32,
    h: u32,
    dx: i64,
    dy: i64,
    entries: &[u8],
) -> Result<(), VoleError> {
    debug_assert_eq!(indices.len() as u64, u64::from(w) * u64::from(h));
    let cw = i64::from(canvas.width());
    let ch = i64::from(canvas.height());
    let y0 = dy.max(0);
    let y1 = (dy + i64::from(h)).min(ch);
    let x0 = dx.max(0);
    let x1 = (dx + i64::from(w)).min(cw);
    if y0 >= y1 || x0 >= x1 {
        return Ok(());
    }
    // Reject any out-of-range index before writing a single pixel (a hostile
    // index raster fails the whole frame deterministically, never partially).
    for idx in indices {
        if usize::from(*idx) >= entries.len() {
            return Err(VoleError::OutOfBounds);
        }
    }
    for cty in y0..y1 {
        let sy = (cty - dy) as usize;
        for ctox in x0..x1 {
            let sx = (ctox - dx) as usize;
            let idx = indices[sy * (w as usize) + sx];
            let value = entries[usize::from(idx)];
            canvas.set(
                u32::try_from(ctox).unwrap(),
                u32::try_from(cty).unwrap(),
                value,
            );
        }
    }
    Ok(())
}

/// Entry matching [`View`]. `FullFrame` materializes the whole canvas; a
/// `Rect`/`Tile` (Phase S partial views) returns the exact state samples in
/// the view's in-canvas region as a fresh canvas whose origin is the region's
/// top-left. State-level views carry no timeline; for a *stream* frame at
/// index `idx` use [`crate::partial::materialize_view`], which additionally
/// resolves interval canvas-op history exactly.
pub fn materialize(
    state: &State,
    view: View,
    width: u32,
    height: u32,
    limits: &Limits,
) -> Result<MaterializedFrame, VoleError> {
    match view {
        View::FullFrame => Ok(MaterializedFrame {
            canvas: materialize_full(state, width, height, limits)?,
        }),
        View::Rect { .. } | View::Tile { .. } => {
            let clip = view.clip(width, height)?.ok_or(VoleError::ApiConstraint(
                "view does not intersect the canvas",
            ))?;
            let canvas = crate::partial::state_crop(state, clip, width, height, limits)?;
            Ok(MaterializedFrame { canvas })
        }
    }
}
