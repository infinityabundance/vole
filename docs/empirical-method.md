# Empirical method

Claim discipline:

1. A claim is about a **measured workload**, not "video in general".
2. Every empirical run writes a timestamped, never-overwritten campaign under
   `evidence/campaigns/<timestamp-commit>/` with `manifest.json`,
   `environment.json`, runs/hashes CSVs, `summary`, and `commands.log`.
3. Courts are reproducible from the working tree + seeded generators.
4. Each phase gate runs `cargo fmt --check`, `cargo check --all-targets`,
   `cargo clippy --all-targets --all-features -- -D warnings`,
   `cargo test --all-features`, its malformed-input test, its court, negative
   controls, and writes a receipt before `SEALED` is declared.
5. Negative results are kept; a favorite hypothesis is only as good as its
   surviving court measurement.

Phase-A campaign + receipt live under `evidence/campaigns/`. See
`docs/information-accounting.md` for byte-accounting discipline.
