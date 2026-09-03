//! Discrete time model.
//!
//! Time in format v1 is a monotone integer interval index. A checkpoint
//! defines the state at a base interval; transitions advance the state from
//! interval `t` to interval `t + 1`. A decoder running a checkpoint forward
//! replays an explicit, bounded number of transitions — never an unbounded
//! chain.

use core::ops::Add;

/// Absolute, monotonically increasing discrete time index.
///
/// The first materialized view (from the initial checkpoint) is at interval
/// `0` unless the stream says otherwise (format v1 always anchors the frame
/// timeline at `0` for the canonical full view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Interval(pub u64);

impl Interval {
    /// The initial interval.
    pub const ZERO: Interval = Interval(0);

    /// The successor interval.
    #[inline]
    pub fn next(self) -> Interval {
        Interval(self.0 + 1)
    }

    /// Integer difference assuming `self >= other` (checked).
    #[inline]
    pub fn checked_sub(self, other: Interval) -> Option<u64> {
        self.0.checked_sub(other.0)
    }

    /// Value.
    #[inline]
    pub fn value(self) -> u64 {
        self.0
    }
}

impl Add<u64> for Interval {
    type Output = Interval;
    fn add(self, rhs: u64) -> Interval {
        Interval(self.0 + rhs)
    }
}
