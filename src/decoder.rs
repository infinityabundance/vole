//! High-level decode/replay API.
//!
//! Parsing + integrity + structural validation yield a [`ParsedStream`];
//! replay applies interval transitions against the checkpoint state; and only
//! materialization produces Gray8 frames. A hostile byte string never panics:
//! the parse layer returns typed [`VoleError`]s.

use crate::{
    error::VoleError,
    format::{parse_stream, ParsedStream},
    materialize,
    pixel::Canvas,
};

use crate::transition::Transition;

/// Parse a standalone `.vole` stream.
pub fn decode_bytes(bytes: &[u8]) -> Result<ParsedStream, VoleError> {
    parse_stream(bytes)
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
    // `prev` is the previous interval's final frame; COPY_RECT/MOVE_RECT compose
    // from it before we enqueue the finished frame.
    for (_, trs) in parsed.intervals() {
        let mut copies = Vec::new();
        for tr in trs {
            if matches!(
                tr,
                Transition::CopyRect { .. } | Transition::MoveRect { .. }
            ) {
                copies.push(tr);
            } else {
                tr.apply(&mut state)?;
            }
        }
        let mut canvas = materialize::materialize_full(&state, w, h, limits)?;
        for op in copies {
            let prior = out.last().ok_or(VoleError::Truncated)?;
            materialize::apply_copy(&mut canvas, prior, op)?;
        }
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
}
