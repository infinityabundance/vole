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
    state::State,
};

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
    out.push(materialize::materialize_full(&state, w, h, limits)?);
    for (_, trs) in parsed.intervals() {
        for tr in trs {
            tr.apply(&mut state)?;
        }
        out.push(materialize::materialize_full(&state, w, h, limits)?);
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

    /// Materialize frame `idx` (0 => checkpoint). Deterministic and cheap to
    /// reason about: full replay from the checkpoint to `idx`.
    pub fn materialize(&self, idx: u64) -> Result<Canvas, VoleError> {
        if idx >= self.frames {
            return Err(VoleError::OutOfBounds);
        }
        let parsed = &self.parsed.parsed;
        let limits = parsed.limits();
        let w = parsed.width();
        let h = parsed.height();
        let mut state: State = parsed.clone_initial();
        // Apply interval groups 0..idx (each yields exactly one frame).
        for (gidx, (_, trs)) in parsed.intervals().iter().enumerate() {
            if gidx as u64 >= idx {
                break;
            }
            for tr in trs {
                tr.apply(&mut state)?;
            }
        }
        materialize::materialize_full(&state, w, h, limits)
    }
}
