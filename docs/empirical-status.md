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
| COPY_RECT / MOVE_RECT | PROPOSED | pending |
| Integer translation as persistent state | PARTIAL (Phase-A SET_POSITION) | court; trajectory form PROPOSED |
| rANS / entropy floor | PROPOSED | pending |
| Trajectory collapse | PROPOSED | pending |
| Palettes | PROPOSED | pending |
| Affine / global state | PROPOSED | pending |
| Transform residual | PROPOSED | pending |
| Procedural generators | PROPOSED | pending |
| Parametric dynamics | PROPOSED | pending |
| Partial materialization (tile/rect) | PROPOSED | pending |
| Resolution-independent procedural state | PROPOSED | pending |
| Inverse-proceduralization encoder | PROPOSED | pending |
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
