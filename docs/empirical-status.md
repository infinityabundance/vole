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
| Palette state (palette-index objects + mutable palette table + per-instance bindings) | ADOPTED | Phase J: tags 0x05/0x06/0x08/0x2d–0x2f; accent cycle 24 B/interval vs 204 773 B/interval palette-less sparse floor (8 532×) and 2 073 600 B RAW on 1920×1080; full rotation 28 B/interval while every pixel changes; static palette content = 13 B/frame unchanged lane (`tests/phase_j.rs`) |
| Palette flattening-tax court (§55 on UI content) | ADOPTED (measured) | Phase J: same visual frames — authored palette intervals 288 B vs raster-origin inverse encode 50 016 B (174×); palette search in the raster encoder is Phase O/Q |
| Variable regions (64→32→16→8 granularity, rectangular bounding boxes) in the inverse encoder | ADOPTED | Phase K: localized-change flagship 1920×1080 — 40 region frames, **zero whole-frame rebases** after frame 0 (26× vs raw); exact-ref region reuse with zero declarations (reuse floor 30 B/interval); noise stays RAW (diff gate) (`tests/phase_k.rs`) |
| Region reuse by exact content identity | ADOPTED | Phase K: alternating glyph area served by two objects reused across 35 frames with 0 declaration bytes (`tests/phase_k.rs`) |
| DSFB governance of the region family | ADOPTED (non-normative) | Phase K: reuse court J_dsfb == J_exhaustive byte-identical at N = 0.378×; fixed-heuristic probe granularity blindness measured at J 1.036 |
| Bounded fixed-point affine / global state (per-instance Q8 placement; pan/zoom/rotation/camera-like as state, not rasters) | ADOPTED | Phase L: tag 0x30; 320×180 rotating-tile flagship — 81 frames as one object + one instance + one `SetAffine`/interval, exact vs an independent incremental painter; affine state 20× smaller than raw and far smaller than re-encoding the same visual frames through the raster encoder; hostile wire + work-budget courts (`tests/phase_l.rs`) |
| Affine residual closure (`F = M(state) ⊕_ρ R` for a Q8 camera approximation) | ADOPTED | Phase L: Q8 30°-rotation approx vs a float-rendered target — the gap is a bounded edge set (<1 500 of 4 096 tile px) closed exactly by one persistent sparse correction; stream decodes byte-identical to the float target (`tests/phase_l.rs`) |
| Affine over palette-index / fill objects | ADOPTED | Phase L: sampled index resolution through the bound palette and uniform-value fills under Q8 maps, byte-exact vs independent references (`tests/phase_l.rs`) |
| Deterministic integer transform residual floor (reversible 4×4 lifting DCT; residual block kind 2; DC/AC coefficient streams + skip mask) | ADOPTED | Phase M: 1920×1080 brightness-drift flagship — 69 848 B/interval vs 2 073 645 B RAW reset (29.7×) and 10.5 MB point residual (150×), all frames decode byte-exact; noise stays RAW; sparse gate (tiny diffs never evaluate it); hostile parse + materialization courts (`tests/phase_m.rs`, `src/transform.rs`) |
| Transform-vs-point same-delta comparison | ADOPTED (measured) | Phase M: 480×270 dense smooth delta — transform block 5 906 B vs 549 268 B point container (93×); the floor is probe-reachable (Exhaustive == FixedHeuristic J 1.000 on drift content) |
| Accounting sub-bucket for inline entropy models | ADOPTED | Phase M: `model_bytes` excluded from `residual_bytes` so the ten buckets sum exactly; fixes the latent double count of 512 B rANS models (recorded in the Phase M receipt) |
| Transform residual | ADOPTED | Phase M (see rows above) |
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
