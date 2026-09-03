# Empirical status ledger

Mechanisms are tracked through explicit states. A mechanism is **never marked
IMPLEMENTED or ADOPTED on the basis of intent** — only a passing court moves it
forward. Negative results are recorded, never erased.

States: `PROPOSED · IMPLEMENTED · COURT-PENDING · ADOPTED · RECORDED · REJECTED`

| Mechanism | State | Evidence |
|---|---|---|
| RAW escape hatch (fill/raw object bases) | ADOPTED | v1 grammar; court |
| FILL (background + uniform objects) | ADOPTED | v1 grammar; court |
| Persistent object identity (object table) | ADOPTED | Phase A |
| Checkpoint | ADOPTED | Phase A restore/replay |
| Interval transitions | ADOPTED | Phase A `SET_POSITION`/`CREATE_INSTANCE` |
| Mutable instance position over time | ADOPTED | Phase A moving-rect court |
| Exact content identity (BLAKE3 immutable objects) | ADOPTED | Phase B `content_id_of` |
| Same-content exact reuse (dedup registry counts) | ADOPTED | Phase B court |
| Unchanged-state lane (zero-transition intervals) | ADOPTED | Phase B static court: 13.0 B/frame amortized |
| Sparse mutation (persistent overlay; strict-sorted patch) | ADOPTED | Phase C blink court: 1 820 B vs 14.98 MB (8 229×) |
| COPY_RECT / MOVE_RECT (prior-frame snapshot compositor) | ADOPTED | Phase D: wrap-scroll court oracle-exact; hostile pass (`tests/phase_d.rs`) |
| Integer translation as persistent state (per-instance `(vx,vy)` + advance) | ADOPTED | Phase E: 101 exact frames, 1 505 B vs 2 692 B per-frame `SetPosition`; static + noise controls (`tests/phase_e.rs`) |
| rANS / entropy floor (owned byte rANS, RAW fallback policy) | ADOPTED | Phase F: byte parity vs `ryg-rans-rs` oracle; skew 59×, uniform→RAW (`tests/phase_f.rs`) |
| Content replacement (clear instances / clear overlay) | ADOPTED | Phase G: tags 0x28/0x29; full-frame replacement semantics (`tests/malformed.rs`) |
| Per-frame residual algebra (one-shot `⊕_ρ`, RAW or rANS block) | ADOPTED | Phase G: tag 0x2a; hostile courts + RANS_RESIDUAL winners on skewed deltas (`tests/phase_g.rs`) |
| Inverse-proceduralization encoder (exhaustive raster→VOLE) | ADOPTED | Phase G: exhaustive per-frame court, winner == min over families (regret 0), end-to-end decode-verified; noise → RAW +1.2% (`tests/phase_g.rs`, `examples/inverse_proof.rs`) |
| Search strategies over one candidate universe (Exhaustive / FixedHeuristic / DsfbGuided) | ADOPTED | Phase H: N_dsfb ≤ 0.18× N_exhaustive with J_dsfb == J_exhaustive byte-identical on steady courts; regime J 1.055× oracle, 0–1 frame recovery; fixed-heuristic probe misses measured at J 11.5× (`tests/phase_h.rs`, `examples/dsfb_proof.rs`) |
| DSFB governor (deterministic trust model; φ/ω/α; regime broadening; rotating sweep) | ADOPTED (non-normative) | Phase H: never in decode; exact-final-cost and RAW-sentinel authority preserved; diagnostics recorded per frame |
| Local procedural rebase (whole-frame RAW recapture) | ADOPTED (measured) | Phase H: rebase events counted per strategy (noise 14 + switch frames); bounded recovery latency |
| Parametric dynamics — bounded trajectory programs (Linear/Accel segments, integer, exact) as first-class state | ADOPTED | Phase I: tags 0x2b/0x2c; accel flagship 686 B vs 1 132 B per-frame `SetPosition` / 1 172 B per-frame `SetVelocity` baselines (123 932× vs raw); piecewise holds exact; hostile budget courts (`tests/phase_i.rs`) |
| Trajectory collapse (§43) — repeated `SetPosition` runs → one trajectory, exactness proven by normative decode, strict byte fall | ADOPTED | Phase I: raster linear pan interval transitions 1 014 → 572 B (0.564×); raster accel 182 → 132 B (0.725×); noise & random-walk fixpoints (`src/collapse.rs`, `tests/phase_i.rs`) |
| Palettes | PROPOSED | pending |
| Affine / global state | PROPOSED | pending |
| Transform residual | PROPOSED | pending |
| Procedural generators | PROPOSED | pending |
| Partial materialization (tile/rect) | PROPOSED | pending |
| Resolution-independent procedural state | PROPOSED | pending |
| DSFB-governed search | PROPOSED (non-normative) | pending courts |
| EntropyFS persistence / cross-video sharing | PROPOSED (optional substrate) | pending |
| Representation re-optimization (`vole optimize`) | PROPOSED | pending |
| Procedural transport streaming | PROPOSED | pending |
| Archive / perceptual profiles | PROPOSED | pending (last) |

## Note on success/failure criteria

Per `docs/architecture.md` and ADRs, a positive result above is a *measured
claim in the stated domain*, never a claim about arbitrary video or entropy
elimination. Full falsification criteria are tracked for each future mechanism
when its phase is entered.
