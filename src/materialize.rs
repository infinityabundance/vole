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
/// Materialize the canonical full frame of `state`.
///
/// # Semantics (normative)
///
/// 1. Allocate a `width x height` canvas and fill it with `state.background`.
/// 2. For each live instance in paint order, paint its immutable object over
///    the canvas using the object’s top-left at the instance `(x, y)`; pixels
///    overwrite. Portions outside the canvas are clipped.
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
    for inst in state.instances() {
        acc = acc.saturating_add(1);
        if acc > u64::from(limits.max_instances) {
            return Err(VoleError::MaterializationBudgetExceeded);
        }
        paint_instance(&mut canvas, state, inst);
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
fn paint_instance(canvas: &mut Canvas, state: &State, inst: &Instance) {
    match state.object(inst.object_id) {
        Some(obj) => paint_object(canvas, obj, inst.x, inst.y),
        None => {
            // Instances are only insertable when their object is declared, so
            // an absent object here is a state invariant broken by a buggy
            // builder; treat as skip to stay total & non-panicking. Decoders
            // additionally enforce this at transition apply time.
        }
    }
}

fn paint_object(canvas: &mut Canvas, obj: &Object, dx: i64, dy: i64) {
    let w = obj.width();
    let h = obj.height();
    match obj.samples() {
        Some(raster) => canvas.blit(raster, w, h, dx, dy),
        None => {
            // Uniform fill object.
            let value = obj.fill_value().unwrap_or(0);
            canvas.fill_rect_clipped(value, dx, dy, dx + i64::from(w), dy + i64::from(h));
        }
    }
}

/// Entry matching [`View`]. Phase A supports `FullFrame` only.
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
    }
}
