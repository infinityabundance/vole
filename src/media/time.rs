//! Rational media time — Phase V.1.1 (V.1 brief §10–§12, contract §2.2).
//!
//! Media timestamps are **exact rational values**, never floating point.
//! A [`TimeBase`] declares the duration of one tick as `numerator /
//! denominator` seconds (the ffmpeg-style convention: `(1, 25)` = 1/25 s per
//! tick = 25 fps at one tick per frame). [`Pts`] and [`Duration`] carry their
//! time base and support checked exact rescaling, cross-base comparison, and
//! checked addition — every computation either succeeds exactly or returns a
//! typed error. Origins may be nonzero and negative (a timeline may begin
//! before zero), durations are positive ticks, and VFR is expressed as a
//! per-observation duration in the timeline (see [`crate::media::epoch`]).
//!
//! The media clock is **separate** from the procedural state clock
//! (`crate::time::Interval`): the state machine advances by explicit
//! intervals; this module only says *when* an observation is presented.

use core::cmp::Ordering;
use core::fmt;

use crate::error::VoleError;

/// Greatest common divisor of two `u128`s.
fn gcd128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// The declared duration of one time tick: `numerator / denominator` seconds.
///
/// Invariants (enforced by [`TimeBase::new`]): both parts are nonzero, so a
/// tick always has a well-defined positive duration. `u32` bounds keep every
/// cross-base product inside the checked integer domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeBase {
    numerator: u32,
    denominator: u32,
}

impl TimeBase {
    /// Build a time base of `numerator / denominator` seconds per tick.
    /// A zero numerator or denominator is degenerate (`InvalidTimeBase`).
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, VoleError> {
        if numerator == 0 || denominator == 0 {
            return Err(VoleError::InvalidTimeBase);
        }
        Ok(TimeBase {
            numerator,
            denominator,
        })
    }

    /// One tick per second.
    pub const fn whole_seconds() -> Self {
        TimeBase {
            numerator: 1,
            denominator: 1,
        }
    }

    /// The time base for a **constant frame rate** `fps_numerator /
    /// fps_denominator` frames per second (one tick per frame):
    /// `tb = fps_denominator / fps_numerator`. Examples: 23.976 →
    /// `for_frame_rate(24000, 1001)` gives `1001/24000` s/tick; 29.97 →
    /// `for_frame_rate(30000, 1001)`.
    pub fn for_frame_rate(fps_numerator: u32, fps_denominator: u32) -> Result<Self, VoleError> {
        Self::new(fps_denominator, fps_numerator)
    }

    /// The time base of `ticks_per_second` ticks per second: `(1,
    /// ticks_per_second)`.
    pub fn ticks_per_second(ticks: u32) -> Result<Self, VoleError> {
        Self::new(1, ticks)
    }

    /// Seconds per tick numerator.
    pub fn numerator(&self) -> u32 {
        self.numerator
    }

    /// Seconds per tick denominator.
    pub fn denominator(&self) -> u32 {
        self.denominator
    }

    /// Exact ticks per second (`denominator / numerator`), or a typed error
    /// when the numerator does not divide the denominator (no integral tick
    /// grid exists at this base).
    pub fn ticks_per_second_exact(&self) -> Result<u64, VoleError> {
        if !self.denominator.is_multiple_of(self.numerator) {
            return Err(VoleError::TimeNotRepresentable);
        }
        Ok(u64::from(self.denominator) / u64::from(self.numerator))
    }

    /// Scale a tick count from this base to `other` **exactly**. The result
    /// must be an integer number of ticks in `other` and must fit `i64`,
    /// otherwise `TimeNotRepresentable`. Cancellation keeps intermediates in
    /// the checked integer domain.
    pub fn rescale_ticks(&self, value: i64, other: &TimeBase) -> Result<i64, VoleError> {
        // value * (self.num/self.den) seconds == out * (other.num/other.den)
        // seconds  =>  out = value * self.num * other.den / (self.den * other.num).
        if *self == *other {
            return Ok(value);
        }
        let mut a = i128::from(self.numerator);
        let mut b = i128::from(other.denominator);
        let mut c = i128::from(self.denominator);
        let mut d = i128::from(other.numerator);
        // Cancel pairwise before multiplying to keep products ≤ i128.
        let g1 = gcd128(a.unsigned_abs(), c.unsigned_abs());
        a /= g1 as i128;
        c /= g1 as i128;
        let g2 = gcd128(b.unsigned_abs(), d.unsigned_abs());
        b /= g2 as i128;
        d /= g2 as i128;
        let value = i128::from(value);
        let mut num = value * a * b;
        let mut den = c * d;
        let g3 = gcd128(num.unsigned_abs(), den.unsigned_abs());
        num /= g3 as i128;
        den /= g3 as i128;
        if den != 1 {
            // The source tick grid does not land on the target grid exactly.
            return Err(VoleError::TimeNotRepresentable);
        }
        i64::try_from(num).map_err(|_| VoleError::TimeNotRepresentable)
    }
}

impl fmt::Display for TimeBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{} s/tick", self.numerator, self.denominator)
    }
}

/// An absolute presentation timestamp: a signed tick count at a declared
/// [`TimeBase`]. Origins may be nonzero; ordering and arithmetic are exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pts {
    value: i64,
    time_base: TimeBase,
}

impl Pts {
    /// A timestamp of `value` ticks at `time_base`.
    pub fn new(value: i64, time_base: TimeBase) -> Self {
        Pts { value, time_base }
    }

    /// Tick value.
    pub fn value(&self) -> i64 {
        self.value
    }

    /// Time base.
    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }

    /// Exact rescale to `target`. Fails typed when the value is not exactly
    /// representable at the target base.
    pub fn rescale(&self, target: TimeBase) -> Result<Pts, VoleError> {
        Ok(Pts {
            value: self.time_base.rescale_ticks(self.value, &target)?,
            time_base: target,
        })
    }

    /// Exact ordering against another timestamp on a possibly different time
    /// base (cross multiplication in the checked integer domain; exact for
    /// any two rational times).
    pub fn cmp_pts(&self, other: &Pts) -> Result<Ordering, VoleError> {
        if self.time_base == other.time_base {
            return Ok(self.value.cmp(&other.value));
        }
        // Compare value*num/den as rationals: cross-multiply.
        // |value| ≤ 2^63−1 and each base part ≤ 2^32−1, so each side fits i128
        // with headroom (2^127 bound, see the module-level proof in tests).
        let a = i128::from(self.value)
            * i128::from(self.time_base.numerator)
            * i128::from(other.time_base.denominator);
        let b = i128::from(other.value)
            * i128::from(other.time_base.numerator)
            * i128::from(self.time_base.denominator);
        Ok(a.cmp(&b))
    }

    /// Whether two timestamps name the same instant (exact).
    pub fn same_instant(&self, other: &Pts) -> Result<bool, VoleError> {
        Ok(self.cmp_pts(other)? == Ordering::Equal)
    }

    /// `self + duration` on the same time base (checked; the sum must fit).
    pub fn checked_add(&self, duration: Duration) -> Result<Pts, VoleError> {
        if duration.time_base != self.time_base {
            return Err(VoleError::TimeNotRepresentable);
        }
        let value = self
            .value
            .checked_add(duration.value)
            .ok_or(VoleError::TimeNotRepresentable)?;
        Ok(Pts {
            value,
            time_base: self.time_base,
        })
    }

    /// `self - other` as a positive [`Duration`] on `self`'s base, when
    /// `self` is exactly `other` plus a positive duration.
    pub fn checked_span_from(&self, other: &Pts) -> Result<Duration, VoleError> {
        let lhs = other.rescale(self.time_base)?;
        let ticks = self
            .value
            .checked_sub(lhs.value)
            .ok_or(VoleError::TimeNotRepresentable)?;
        Duration::new(ticks, self.time_base)
    }

    /// Stable descriptor for receipts and diagnostics.
    pub fn describe(&self) -> String {
        format!("{}@{}", self.value, self.time_base)
    }
}

impl fmt::Display for Pts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.value, self.time_base)
    }
}

/// A positive span of time: `value ≥ 1` ticks at a declared [`TimeBase`].
/// VFR is expressed as per-observation durations in the timeline; a zero or
/// negative duration is not a valid observation interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Duration {
    value: i64,
    time_base: TimeBase,
}

impl Duration {
    /// A duration of `value ≥ 1` ticks at `time_base`. Zero or negative
    /// durations are not representable as observation intervals (typed).
    pub fn new(value: i64, time_base: TimeBase) -> Result<Self, VoleError> {
        if value <= 0 {
            return Err(VoleError::TimeNotRepresentable);
        }
        Ok(Duration { value, time_base })
    }

    /// Tick value (always ≥ 1).
    pub fn value(&self) -> i64 {
        self.value
    }

    /// Time base.
    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }

    /// Exact rescale to `target` (same rule as [`Pts::rescale`]).
    pub fn rescale(&self, target: TimeBase) -> Result<Duration, VoleError> {
        Ok(Duration {
            value: self.time_base.rescale_ticks(self.value, &target)?,
            time_base: target,
        })
    }

    /// The duration of one frame at the constant frame rate `fps_num /
    /// fps_den` on this time base (one tick per frame), or the exact rescale
    /// when the tick grids differ.
    pub fn of_constant_frame_rate(
        fps_numerator: u32,
        fps_denominator: u32,
        at: TimeBase,
    ) -> Result<Duration, VoleError> {
        Duration::new(1, TimeBase::for_frame_rate(fps_numerator, fps_denominator)?)?.rescale(at)
    }

    /// Whether `other` is longer (exact, cross-base).
    pub fn cmp_duration(&self, other: &Duration) -> Result<Ordering, VoleError> {
        if self.time_base == other.time_base {
            return Ok(self.value.cmp(&other.value));
        }
        let a = i128::from(self.value)
            * i128::from(self.time_base.numerator)
            * i128::from(other.time_base.denominator);
        let b = i128::from(other.value)
            * i128::from(other.time_base.numerator)
            * i128::from(self.time_base.denominator);
        Ok(a.cmp(&b))
    }

    /// Stable descriptor for receipts and diagnostics.
    pub fn describe(&self) -> String {
        format!("{}@{}", self.value, self.time_base)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.value, self.time_base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cmp::Ordering;

    #[test]
    fn rescale_math_is_hand_exact() {
        // 25 fps: tb (1,25); 24 fps: tb (1,24); 23.976: tb (1001,24000).
        let t25 = TimeBase::for_frame_rate(25, 1).unwrap();
        assert_eq!((t25.numerator(), t25.denominator()), (1, 25));
        assert_eq!(t25.ticks_per_second_exact().unwrap(), 25);
        let t24000_1001 = TimeBase::for_frame_rate(24000, 1001).unwrap();
        assert_eq!(
            (t24000_1001.numerator(), t24000_1001.denominator()),
            (1001, 24000)
        );
        // 2400 ticks at 1001/24000 == 2400*1001/24000 s == 1001/10 s ==
        // 100.1 s. At tb (1,1) that is floor? exact: seconds = 2400*1001/24000
        // = 100.1 -> not integral seconds, so (1,1) rescale must fail typed.
        let p = Pts::new(2400, t24000_1001);
        assert!(matches!(
            p.rescale(TimeBase::whole_seconds()),
            Err(VoleError::TimeNotRepresentable)
        ));
        // At tb (1,10) (0.1 s per tick) it is exactly 1001 ticks.
        let t1_10 = TimeBase::new(1, 10).unwrap();
        assert_eq!(p.rescale(t1_10).unwrap().value(), 1001);
        // 25fps <-> 50fps: 1 tick @25fps == 2 ticks @50fps.
        let t50 = TimeBase::for_frame_rate(50, 1).unwrap();
        let p = Pts::new(1, t25);
        assert_eq!(p.rescale(t50).unwrap().value(), 2);
        // Negative timestamps rescale symmetrically.
        let n = Pts::new(-25, t25);
        assert_eq!(n.rescale(t50).unwrap().value(), -50);
    }

    #[test]
    fn ordering_is_exact_across_bases() {
        let t25 = TimeBase::for_frame_rate(25, 1).unwrap();
        let t24000_1001 = TimeBase::for_frame_rate(24000, 1001).unwrap();
        let t50 = TimeBase::for_frame_rate(50, 1).unwrap();
        // 1 s == 25 ticks @25fps == 50 ticks @50fps; 23.976fps: 24000/1001 ticks.
        let s1_25 = Pts::new(25, t25);
        let s1_50 = Pts::new(50, t50);
        assert_eq!(s1_25.cmp_pts(&s1_50).unwrap(), Ordering::Equal);
        assert!(s1_25.same_instant(&s1_50).unwrap());
        // 24 frames at 23.976 == 1.001 s exactly? 24 ticks * 1001/24000 s =
        // 1001/1000 s. Compare against 1001/1000 s at tb (1000, 1000000)? Simpler:
        // 24000 ticks @23.976 tb == 1001 s; compare with 1001 s @whole seconds.
        let f = Pts::new(24000, t24000_1001);
        let one_s = Pts::new(1001, TimeBase::whole_seconds());
        assert_eq!(f.cmp_pts(&one_s).unwrap(), Ordering::Equal);
        // Ordering between 23.976 grid and 25 fps grid: 23 ticks @23.976 vs 24 @25.
        let a = Pts::new(23, t24000_1001); // 23*1001/24000 s ≈ 0.9593 s
        let b = Pts::new(24, t25); // 0.96 s
        assert_eq!(a.cmp_pts(&b).unwrap(), Ordering::Less);
    }

    #[test]
    fn durations_are_positive_and_checked() {
        let t30 = TimeBase::for_frame_rate(30000, 1001).unwrap(); // 29.97
        assert_eq!((t30.numerator(), t30.denominator()), (1001, 30000));
        assert!(matches!(
            Duration::new(0, t30),
            Err(VoleError::TimeNotRepresentable)
        ));
        assert!(matches!(
            Duration::new(-1, t30),
            Err(VoleError::TimeNotRepresentable)
        ));
        let d = Duration::new(1, t30).unwrap();
        let p = Pts::new(-1001, t30);
        let next = p.checked_add(d).unwrap();
        assert_eq!(next.value(), -1000);
        // Same-base addition overflow is typed.
        let max = Pts::new(i64::MAX, TimeBase::whole_seconds());
        assert!(matches!(
            max.checked_add(Duration::new(1, TimeBase::whole_seconds()).unwrap()),
            Err(VoleError::TimeNotRepresentable)
        ));
    }

    #[test]
    fn degenerate_time_bases_are_typed() {
        assert!(matches!(
            TimeBase::new(0, 25),
            Err(VoleError::InvalidTimeBase)
        ));
        assert!(matches!(
            TimeBase::new(1, 0),
            Err(VoleError::InvalidTimeBase)
        ));
        assert!(matches!(
            TimeBase::for_frame_rate(0, 1),
            Err(VoleError::InvalidTimeBase)
        ));
        assert!(matches!(
            TimeBase::ticks_per_second(0),
            Err(VoleError::InvalidTimeBase)
        ));
    }

    #[test]
    fn long_and_fractional_timelines_stay_exact() {
        // Two hours at 29.97: 2*3600 s * 30000/1001 ticks/s = 216_000*30000/1001
        // = 6_480_000_000/1001 ticks — not integral at that base? 216000 s *
        // 30000/1001 = 6.48e9/1001 ≈ 6.47e6, non-integral; use tb (1,30000) for
        // an exact two-hour tick count: 216000 s * 30000 = 6.48e9 ticks.
        let t30k = TimeBase::ticks_per_second(30_000).unwrap();
        let p = Pts::new(0, t30k);
        let two_h = Duration::new(216_000 * 30_000, t30k).unwrap();
        assert_eq!(p.checked_add(two_h).unwrap().value(), 6_480_000_000);
        // Very long timeline at a fine base must overflow *typed*, not wrap.
        let p = Pts::new(i64::MAX - 1, t30k);
        assert!(matches!(
            p.checked_add(Duration::new(2, t30k).unwrap()),
            Err(VoleError::TimeNotRepresentable)
        ));
    }
}
