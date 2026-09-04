//! High-level decode/replay API.
//!
//! Parsing + integrity + structural validation yield a [`ParsedStream`];
//! replay applies interval transitions against the checkpoint state; and only
//! materialization produces Gray8 frames. A hostile byte string never panics:
//! the parse layer returns typed [`VoleError`]s.

use crate::{
    error::VoleError,
    format::{parse_stream, parse_stream_resolving, ParsedStream},
    materialize,
    pixel::Canvas,
    state::State,
    Limits,
};

use crate::transition::Transition;

/// Parse a standalone `.vole` stream.
pub fn decode_bytes(bytes: &[u8]) -> Result<ParsedStream, VoleError> {
    parse_stream(bytes)
}

/// Parse a `.vole` stream, resolving every **external object declaration**
/// through `store` (Phase P). Streams that embed their objects behave exactly
/// as with [`decode_bytes`]; streams that reference store-held objects by
/// content id are fetched here — each fetched record's digest must match its
/// declared content id, and the resolved objects are ordinary
/// [`crate::object::Object`]s from then on, so replay and materialization
/// never touch the store again.
pub fn decode_with_store(
    bytes: &[u8],
    store: &dyn crate::store::ObjectStore,
) -> Result<ParsedStream, VoleError> {
    parse_stream_resolving(bytes, store)
}

/// Advance one interval: apply every state transition to `state` in listed
/// order, then materialize the canonical full frame and apply the interval's
/// canvas ops (COPY_RECT/MOVE_RECT reading from `prev`, the immediately
/// previous decoded frame, and the per-frame residual op) in listed order.
/// Returns the finished frame. This is the single normative replay step shared
/// by [`materialize_all`] and the Phase-G inverse encoder's simulation, so the
/// encoder's committed frames are produced by exactly the code the decoder
/// runs.
pub(crate) fn step_frame(
    state: &mut State,
    prev: &Canvas,
    trs: &[Transition],
    width: u32,
    height: u32,
    limits: &Limits,
) -> Result<Canvas, VoleError> {
    let mut ops = Vec::new();
    for tr in trs {
        if is_canvas_op(tr) {
            ops.push(tr);
        } else {
            tr.apply(state)?;
        }
    }
    let mut canvas = materialize::materialize_full(state, width, height, limits)?;
    for op in ops {
        materialize::apply_canvas_op(&mut canvas, prev, op, limits)?;
    }
    Ok(canvas)
}

/// Whether a transition is a frame-compositor (canvas) op rather than a state
/// mutation.
pub(crate) fn is_canvas_op(tr: &Transition) -> bool {
    matches!(
        tr,
        Transition::CopyRect { .. } | Transition::MoveRect { .. } | Transition::Residual { .. }
    )
}

/// Replay the checkpoint and every interval, returning materialized full
/// frames in timeline order. Frame `0` is the checkpoint view; every interval
/// group thereafter yields one further frame.
pub fn materialize_all(parsed: &ParsedStream) -> Result<Vec<Canvas>, VoleError> {
    let limits = parsed.limits();
    let w = parsed.width();
    let h = parsed.height();
    let mut state = parsed.clone_initial();
    let mut out = Vec::with_capacity(parsed.frame_count() as usize);
    let first = materialize::materialize_full(&state, w, h, limits)?;
    out.push(first);
    for (_, trs) in parsed.intervals() {
        let prior = out.last().expect("previous frame exists").clone();
        let canvas = step_frame(&mut state, &prior, trs, w, h, limits)?;
        out.push(canvas);
    }
    Ok(out)
}

/// Deterministic random/sequential access cursor over a parsed stream.
///
/// `frame(i)` reproduces frame `i` by replaying from the checkpoint; correctness
/// is prioritized over search cleverness at this phase (partial materialization
/// and seek cost accounting arrive with later transport phases).
#[derive(Clone)]
pub struct Decoder {
    parsed: ParsedStreamView,
    frames: u64,
}

/// Thin owned handle so `Decoder` can be cloned cheaply while replay stays
/// correct.
#[derive(Clone)]
struct ParsedStreamView {
    parsed: ParsedStream,
}

impl Decoder {
    /// Construct from a parsed stream.
    pub fn new(parsed: ParsedStream) -> Self {
        let frames = parsed.frame_count();
        Decoder {
            parsed: ParsedStreamView { parsed },
            frames,
        }
    }

    /// Number of materializable frames.
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// Materialize frame `idx` (0 => checkpoint). Deterministic: replays the
    /// whole stream forward (COPY_RECT refs the prior final frame) and returns
    /// frame `idx`. Partial/seek accounting arrives in a later transport phase.
    pub fn materialize(&self, idx: u64) -> Result<Canvas, VoleError> {
        if idx >= self.frames {
            return Err(VoleError::OutOfBounds);
        }
        let all = materialize_all(&self.parsed.parsed)?;
        all.into_iter()
            .nth(idx as usize)
            .ok_or(VoleError::OutOfBounds)
    }

    /// Materialize frame `idx` restricted to `view` (Phase S partial
    /// materialization). Returns the exact samples a whole-frame decode of
    /// `idx` would place in the view's in-canvas region, plus measured decode
    /// work ([`crate::partial::PartialStats`]). A `FullFrame` view replays
    /// the canonical step machinery (byte- and error-identical to whole-frame
    /// decode); a sub-frame view runs the demand-planned partial decoder
    /// (`crate::partial::materialize_view`).
    pub fn materialize_view(
        &self,
        idx: u64,
        view: crate::view::View,
    ) -> Result<crate::partial::PartialView, VoleError> {
        crate::partial::materialize_view(&self.parsed.parsed, idx, view)
    }
}
