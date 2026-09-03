# DSFB search governance (design + note)

DSFB is a read-only observer over residual trajectories (φ = current quality,
ω = gradual drift, α = slew/regime change). In this crate DSFB is officially
**non-normative** (ADR-0003): it influences only which/how many procedural
candidates the encoder evaluates, never decode. Later phases add `DsfbGuided`
alongside `Exhaustive` and `FixedHeuristic` strategies over the *same*
candidate universe, with regret receipts.

Primary success criterion (courted later): `N_dsfb < N_exhaustive` while
`J_dsfb ≈ J_exhaustive`; if a fixed heuristic is cheaper and equal, DSFB loses
for that workload. Deterministic sentinels replace random bandits.

Status: PROPOSED; no DSFB code in this crate until its dedicated phase.
