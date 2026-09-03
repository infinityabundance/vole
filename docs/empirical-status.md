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
| Trajectory collapse | PROPOSED | pending (measured temporal gap; Phase I/O) |
| Palettes | PROPOSED | pending |
| Affine / global state | PROPOSED | pending |
| Transform residual | PROPOSED | pending |
| Procedural generators | PROPOSED | pending |
| Parametric dynamics | PROPOSED | pending |
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
