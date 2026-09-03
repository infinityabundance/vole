//! Phase H: DSFB — the zero-authority encoder search governor (non-normative).
//!
//! DSFB is **never** part of normative decoding (ADR-0003): it may only reorder
//! and budget the encoder's candidate evaluation. It may not alter samples,
//! normative state semantics, or the exact final cost comparison; it can never
//! make an invalid reconstruction acceptable, suppress RAW permanently, or be
//! required to decode a stream.
//!
//! Three strategies run over the *same* candidate universe (the whole-frame
//! families: UNCHANGED · reset/EXACT_REF/RAW · SPARSE · one-shot
//! RESIDUAL/RANS_RESIDUAL · COPY_RECT (wrap, screen-scroll, prev-diff) ·
//! TRANSLATION · REGIONS (Phase K variable granularity) · clears):
//!
//! * **Exhaustive** — evaluate the full per-frame candidate space (the search-
//!   quality oracle).
//! * **FixedHeuristic** — evaluate a fixed, constant per-frame plan (no
//!   history, no adaptation).
//! * **DsfbGuided** — a deterministic trust model over recent winning
//!   hypotheses allocates the per-frame budget. The model tracks, per family,
//!   its recent explanatory quality `φ` (win rate in the recent window) and
//!   the drift `ω` of that quality; a regime change `α` (winner leaving the
//!   active set, or a large payload slew) triggers a full "broaden" frame so
//!   the search re-learns. A deterministic rotating sweep (§27: top trusted
//!   hypotheses + rotating sentinel hypothesis; no stochastic bandits) keeps a
//!   cheap path back into a family that fell out of trust, bounding adaptation
//!   latency after a silent regime change.
//!
//! Every evaluated candidate is still validated byte-exactly by the normative
//! materializer path in `crate::inverse`; DSFB only changes *which* candidates
//! are evaluated. The primary success criterion is `N_dsfb < N_exhaustive`
//! while `J_dsfb ≈ J_exhaustive` (fewer candidates, equal bytes).

use std::collections::VecDeque;

/// Evaluation intensity of one candidate family class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Do not evaluate the family this frame.
    Off,
    /// Evaluate a small deterministic probe of the family.
    Probe,
    /// Evaluate the family's full candidate set.
    Full,
}

/// The three search strategies (§28) over one candidate universe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderStrategy {
    /// Full per-frame evaluation; the search-quality oracle.
    Exhaustive,
    /// Fixed, history-free per-frame plan.
    FixedHeuristic,
    /// DSFB-governed budget allocation.
    DsfbGuided,
}

impl EncoderStrategy {
    /// Machine-stable label.
    pub fn label(self) -> &'static str {
        match self {
            EncoderStrategy::Exhaustive => "exhaustive",
            EncoderStrategy::FixedHeuristic => "fixed_heuristic",
            EncoderStrategy::DsfbGuided => "dsfb_guided",
        }
    }
}

/// Per-frame evaluation plan. Every plan keeps the deterministic sentinels
/// (RAW/incumbent/cheap-universal) so a correct winner always exists; the
/// optional families are gated by the strategy.
#[derive(Debug, Clone)]
pub struct FramePlan {
    /// Zero-transition lane (cheap universal sentinel).
    pub unchanged: bool,
    /// Clear-to-background / clear-only corrective resets.
    pub clears: Mode,
    /// Persistent sparse overlay + one-shot residual over the state base
    /// (cheap universal sentinel).
    pub sparse: bool,
    /// Whole-canvas copy + residual vs the previous frame (cheap universal
    /// sentinel).
    pub prev_diff: bool,
    /// Whole-pixel instance translation (probe = last winning delta only).
    pub translation: Mode,
    /// COPY_RECT families (toroidal wraps, screen scrolls). Probe replays the
    /// previous winner's rect ops (or a small default wrap set).
    pub copies: Mode,
    /// Phase K: variable-region repair family. Full walks the 64→32→16→8
    /// granularity ladder; Probe evaluates the fixed probe granularity only.
    pub regions: Mode,
    /// Phase M: transform-coded residual floor. Full evaluates the transform
    /// whenever it could beat the point-list baselines; Probe only when the
    /// per-frame diff is dense (≥ 1/16 canvas).
    pub transform: Mode,
    /// Phase N: whole-frame procedural-generator discovery. Full evaluates
    /// the deterministic content fits (gradient / checker / periodic, each
    /// validated by its normative render); Probe evaluates the gradient fit
    /// only.
    pub generators: Mode,
    /// Whether the previous frame's winner emitted copy ops available for a
    /// probe replay.
    pub replay_ops: bool,
    /// True when this plan is a cold start / regime-broaden full plan.
    pub broaden: bool,
}

impl FramePlan {
    /// Full evaluation (exhaustive; also the DSFB cold-start / broaden plan).
    pub fn full() -> FramePlan {
        FramePlan {
            unchanged: true,
            clears: Mode::Full,
            sparse: true,
            prev_diff: true,
            translation: Mode::Full,
            copies: Mode::Full,
            regions: Mode::Full,
            transform: Mode::Full,
            generators: Mode::Full,
            replay_ops: false,
            broaden: false,
        }
    }

    /// Full evaluation flagged as a regime-broadening plan (diagnostics).
    pub fn broaden() -> FramePlan {
        FramePlan {
            broaden: true,
            ..FramePlan::full()
        }
    }

    /// The constant FixedHeuristic plan: sentinels + full translation window +
    /// a small default copy probe (wraps and screen scrolls by 1..3 rows/cols)
    /// and replay of any previous copy ops.
    pub fn fixed_heuristic() -> FramePlan {
        FramePlan {
            unchanged: true,
            clears: Mode::Off,
            sparse: true,
            prev_diff: true,
            translation: Mode::Full,
            copies: Mode::Probe,
            regions: Mode::Probe,
            transform: Mode::Probe,
            generators: Mode::Probe,
            replay_ops: true,
            broaden: false,
        }
    }
}

/// Per-family state in the DSFB trust model.
#[derive(Debug, Clone, Default)]
pub struct FamilyState {
    /// Times this family was evaluated (recent window).
    pub evaluated: u64,
    /// Times it won (recent window).
    pub wins: u64,
    /// Explanatory quality `φ`: wins / evaluated over the window.
    pub phi: f64,
}

/// Frame-level DSFB diagnostics (§24: `φ`, `ω`, `α`).
#[derive(Debug, Clone, Default)]
pub struct DsfbFrameDiag {
    /// Winner family this frame.
    pub winner: &'static str,
    /// Winner incremental payload.
    pub winner_payload: u64,
    /// Families trusted for this frame (winner set of the recent window).
    pub active: Vec<&'static str>,
    /// Per-family explanatory quality `φ` over the window.
    pub phi: Vec<(&'static str, f64)>,
    /// Drift `ω`: |Δφ| EWMA across families since the previous frame.
    pub omega: f64,
    /// Regime-change indicator `α`: 1 when a broaden plan ran or was armed.
    pub alpha: u8,
    /// Whether this frame's plan was a full broaden.
    pub broadened: bool,
    /// Cumulative DSFB evaluation count so far.
    pub total_evaluated: u64,
    /// Frames since the last broaden event.
    pub since_regime: u64,
}

/// DSFB model constants (deterministic; documented in the Phase-H receipt).
pub const WINDOW: usize = 12;
/// Regime payload slew factor: payload > `SLew_FACTOR` × trailing median arms a
/// broaden.
pub const SLEW_FACTOR: f64 = 3.0;
/// Rotating-sweep cadence: every N frames the model also evaluates the cheap
/// non-active families (bounds adaptation latency after silent regime change).
pub const SWEEP_CADENCE: u64 = 6;
/// Default probe wrap/scroll shifts (1..=DEFAULT_PROBE_SHIFTS rows/cols).
pub const DEFAULT_PROBE_SHIFTS: i64 = 3;

/// The deterministic DSFB model. Stateless across runs except for the ring
/// below; contains no randomness.
#[derive(Debug, Clone)]
pub struct DsfbModel {
    /// Recent `(winner family, payload)` ring, capped at [`WINDOW`].
    ring: VecDeque<(&'static str, u64)>,
    /// Armed: the next frame must be a full broaden.
    broaden_next: bool,
    /// Total frames observed.
    pub frames: u64,
    /// Total candidates evaluated so far.
    pub total_evaluated: u64,
    /// Drift EWMA `ω`.
    omega: f64,
    /// Previous per-family `φ` (for drift measurement).
    prev_phi: Vec<(&'static str, f64)>,
    /// Frames since the last broaden.
    pub since_regime: u64,
}

impl Default for DsfbModel {
    fn default() -> Self {
        Self::new()
    }
}

impl DsfbModel {
    /// Fresh model.
    pub fn new() -> DsfbModel {
        DsfbModel {
            ring: VecDeque::with_capacity(WINDOW),
            broaden_next: false,
            frames: 0,
            total_evaluated: 0,
            omega: 0.0,
            prev_phi: Vec::new(),
            since_regime: 0,
        }
    }

    /// Winner families of the recent window (the active/trusted set).
    pub fn active(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for (f, _) in &self.ring {
            if !out.contains(f) {
                out.push(f);
            }
        }
        out
    }

    /// Per-family explanatory quality over the window: wins / frames observed.
    pub fn phi(&self) -> Vec<(&'static str, f64)> {
        let n = self.ring.len().max(1) as f64;
        let mut counts: Vec<(&str, u64)> = Vec::new();
        for (f, _) in &self.ring {
            if let Some(slot) = counts.iter_mut().find(|(x, _)| x == f) {
                slot.1 += 1;
            } else {
                counts.push((f, 1));
            }
        }
        counts.into_iter().map(|(f, c)| (f, c as f64 / n)).collect()
    }

    /// Trailing median of the ring payloads (0 when the ring is empty).
    fn median_payload(&self) -> u64 {
        let mut v: Vec<u64> = self.ring.iter().map(|(_, p)| *p).collect();
        if v.is_empty() {
            return 0;
        }
        v.sort_unstable();
        v[v.len() / 2]
    }

    /// Plan the next frame's evaluation.
    ///
    /// The plan depends only on the model history (deterministic). A cold
    /// start (no history) and any armed regime broaden evaluate everything;
    /// otherwise trusted families get their full cheap candidate sets (or a
    /// copy-op replay probe), and the deterministic rotating sweep keeps a
    /// low-cost path into every non-active family.
    pub fn plan(&self) -> FramePlan {
        if self.broaden_next || self.ring.is_empty() {
            return FramePlan::broaden();
        }
        let active = self.active();
        let mut p = FramePlan {
            unchanged: true,
            clears: Mode::Off,
            sparse: true,
            prev_diff: true,
            translation: Mode::Off,
            copies: Mode::Off,
            regions: Mode::Off,
            transform: Mode::Off,
            generators: Mode::Off,
            replay_ops: true,
            broaden: false,
        };
        if active.contains(&"translation") {
            p.translation = Mode::Full;
        }
        if active.contains(&"copy_rect") || active.contains(&"copy_residual") {
            p.copies = Mode::Probe;
        }
        if active.contains(&"regions") {
            p.regions = Mode::Full;
        }
        if active.contains(&"transform_residual") {
            p.transform = Mode::Full;
        }
        if active.contains(&"generator") || active.contains(&"generator_residual") {
            p.generators = Mode::Full;
        }
        // Deterministic rotating sweep (sentinel hypothesis): periodically
        // re-probe the non-active families so a silent regime change is found
        // within a bounded number of frames.
        if self.frames.is_multiple_of(SWEEP_CADENCE) {
            if p.translation == Mode::Off {
                p.translation = Mode::Full;
            }
            if p.copies == Mode::Off {
                p.copies = Mode::Probe;
            }
            if p.regions == Mode::Off {
                p.regions = Mode::Probe;
            }
            if p.transform == Mode::Off {
                p.transform = Mode::Probe;
            }
            if p.generators == Mode::Off {
                p.generators = Mode::Probe;
            }
        }
        p
    }

    /// Observe one finished frame decision: winner family + incremental
    /// payload + how many candidates were evaluated this frame. Updates the
    /// ring, the drift EWMA, and arms a regime broaden when the winner left
    /// the pre-frame active set or the payload slew exceeded the threshold.
    pub fn observe(&mut self, winner: &'static str, payload: u64, evaluated: u64) {
        let active_before = self.active();
        self.frames += 1;
        self.total_evaluated += evaluated;
        self.ring.push_back((winner, payload));
        while self.ring.len() > WINDOW {
            self.ring.pop_front();
        }
        // Regime detection α: winner not trusted before this frame, or a large
        // payload slew against the trailing median. The very first observation
        // (empty active set) is a cold start, not a regime change.
        let mut alpha = false;
        if !active_before.is_empty() && !active_before.contains(&winner) {
            alpha = true;
        }
        let med = self.median_payload();
        if med > 0 && self.ring.len() >= 5 && (payload as f64) > SLEW_FACTOR * med as f64 {
            alpha = true;
        }
        if alpha {
            self.broaden_next = true;
            self.since_regime = 0;
        } else {
            self.broaden_next = false;
            self.since_regime += 1;
        }
        // Drift ω: EWMA of the per-family |Δφ| between consecutive frames.
        let phi = self.phi();
        let mut delta = 0.0f64;
        for (f, v) in &phi {
            let prev = self
                .prev_phi
                .iter()
                .find(|(pf, _)| pf == f)
                .map(|(_, pv)| *pv)
                .unwrap_or(0.0);
            delta += (v - prev).abs();
        }
        if self.frames > 1 {
            self.omega = 0.5 * delta + 0.5 * self.omega;
        } else {
            self.omega = delta;
        }
        self.prev_phi = phi;
    }

    /// Frame diagnostics for evidence.
    pub fn diagnostics(&self, winner: &'static str, payload: u64) -> DsfbFrameDiag {
        let mut active = self.active();
        active.sort_unstable();
        DsfbFrameDiag {
            winner,
            winner_payload: payload,
            active,
            phi: self.phi(),
            omega: self.omega,
            alpha: u8::from(self.broaden_next),
            broadened: self.broaden_next,
            total_evaluated: self.total_evaluated,
            since_regime: self.since_regime,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_start_plans_full_and_learns_winner() {
        let mut m = DsfbModel::new();
        // No history: everything is evaluated.
        assert_eq!(m.plan().copies, Mode::Full);
        assert_eq!(m.plan().translation, Mode::Full);
        // A steady translation winner narrows the plan.
        for _ in 0..4 {
            let p = m.plan();
            let _ = p;
            m.observe("translation", 26, 400);
        }
        let p = m.plan();
        assert_eq!(p.translation, Mode::Full);
        assert_eq!(p.copies, Mode::Off);
        assert!(p.unchanged && p.sparse && p.prev_diff);
        assert!(!p.broaden);
    }

    #[test]
    fn winner_leaving_the_active_set_arms_a_broaden() {
        let mut m = DsfbModel::new();
        // Five static-lane frames (not on the rotating-sweep cadence).
        for _ in 0..5 {
            m.observe("unchanged", 13, 8);
        }
        let p = m.plan();
        assert_eq!(p.translation, Mode::Off);
        assert_eq!(p.copies, Mode::Off);
        // Regime change: a translation wins.
        m.observe("translation", 26, 40);
        assert!(
            m.broaden_next,
            "winner outside the active set must arm a broaden"
        );
        let p = m.plan();
        assert!(p.broaden);
        assert_eq!(p.copies, Mode::Full);
    }

    #[test]
    fn payload_slew_arms_a_broaden() {
        let mut m = DsfbModel::new();
        for _ in 0..8 {
            m.observe("sparse", 27, 10);
        }
        assert!(!m.broaden_next);
        // Residual cost blows up (a creeping delta): slew > 3x median.
        m.observe("sparse", 4000, 10);
        assert!(m.broaden_next);
    }

    #[test]
    fn rotating_sweep_keeps_a_path_to_cold_families() {
        let mut m = DsfbModel::new();
        for f in 0..30 {
            m.observe("raw", 1000, 20);
            let p = m.plan();
            if f > 0 && (f + 1) % SWEEP_CADENCE == 0 {
                assert_eq!(
                    p.translation,
                    Mode::Full,
                    "cadence frame must re-probe non-active translation"
                );
            }
        }
    }

    #[test]
    fn phi_omega_are_deterministic_and_bounded() {
        let mut m = DsfbModel::new();
        // Enough frames for the drift EWMA to decay to ~0 on steady content.
        for _ in 0..60 {
            m.observe("translation", 26, 30);
        }
        let d = m.diagnostics("translation", 26);
        assert_eq!(d.winner, "translation");
        assert!(d.omega < 1e-6, "steady drift must decay, got {}", d.omega);
        let phi = d.phi.iter().find(|(f, _)| *f == "translation").unwrap().1;
        assert!((phi - 1.0).abs() < 1e-9);
        assert!(d.alpha == 0);
        assert_eq!(d.total_evaluated, 60 * 30);
    }
}
