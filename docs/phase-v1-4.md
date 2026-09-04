# Phase V.1.4 Receipt — existing-family generalization (V.1 video programme,
# contract `docs/phase-v1-video-architecture.md` §2.6/§2.8; master brief
# §45–§50, §247; `docs/format-v2.md` deliberately re-frozen with the V.1.4
# family extension)
# (SEALED)

## Deliverable

V.1.4 ports the remaining sealed v1 representation families onto the
canonical multiplane domain (`brief §247`: RAW/FILL/UNCHANGED/SPARSE/COPY/
REGIONS/TRANSLATION/TRAJECTORY/PALETTE/AFFINE/TRANSFORM/GENERATOR), sealing
independent-plane correctness for the **semantic surface** (every family
expressible and exact at any depth/layout) and adding the per-plane **family
encoder** over the V.1.2 exact floor.

* **Depth-aware procedural generators** (`media/gen.rs`): the sealed Phase-N
  programs (gradient/checker/periodic/noise) generalized from mod-256 to any
  plane's sample domain (`mod (max+1)`; noise scaled deterministically).
  **Depth-8 identity is courted** — equivalent parameters reproduce the v1
  Phase-N generators byte-for-byte at `max = 255`. Wire record mirrors the v1
  tags with u32 value parameters; parameters validate against the plane depth.
* **Core semantic extensions** (`media/core.rs`, all mirroring v1 meaning in
  the plane's u32 sample domain):
  * content kinds **palette-index** (`PlaneContent::Index`, tight index
    bytes, resolved through the instance's bound palette at render) and
    **generator** (`PlaneContent::Generator`);
  * **palette state** per plane (entries are depth-domain samples, ids ≥ 1)
    with `SetPalette` / `PatchPalette` / `BindPalette` ops and an initial
    palette table + bindings in the program's initial state;
  * **motion state** per instance — persistent translation
    (`SetVelocity`/`AdvanceTranslations`) and parametric trajectory
    (`SetTrajectory`/`AdvanceTrajectories`, the sealed v1 segment types and
    stepper semantics); motion kinds are mutually exclusive and die with
    their instance on `ClearInstances`, mirroring v1;
  * **Q8 affine placement** (`SetAffine`, the sealed v1 `AffineParams` and
    source-map rule) rendered by scanning the plane through the canonical
    fixed-point map, with per-materialization work capped by
    `Limits.max_affine_work`;
  * **transform-coded residual** op (`TransformResidual`, the sealed Phase-M
    4×4 lifting-DCT kind-2 container): decode adds inverse-transformed
    samples to the interval's fresh render, bounded by the plane's active
    depth; `encode_plane_transform_block` closes any plane delta exactly.
  Rendering of the new content kinds mirrors the v1 materializer semantics
  (palette pre-validation before any write; generator = one sample per
  painted pixel; affine scans the whole plane per affine instance).
* **v2 wire family extension** (`media/wire.rs`, `docs/format-v2.md`
  re-frozen): feature bit `0x1` declares the V.1.4 surface — object kinds
  `0x03` (index) and `0x04` (generator), ops `0x29`–`0x31`, and a per-plane
  initial-state tail (palette table + per-instance motion/binding records,
  canonical ascending orders). **Additive**: files without the bit keep their
  exact V.1.2 bytes and the V.1.2 golden; extension tags/kinds without the
  bit fail closed typed. A second golden (the canonical 16×12 Gray8 extension
  scenario) is pinned. The writer emits the minimal feature bits the content
  needs.
* **Per-plane family encoder** (`media/encode.rs`,
  `encode_pictures_families` + `EncodeReport`): every interval proposes
  bounded exact candidates — unchanged (empty group), fill/raw whole-plane
  replacements, exact object reuse, palette-index replacement (small value
  sets), generator replacement (exact gradient/checker fits), sparse and
  transform residuals over the committed render, and CopyRect region reuse
  from the previous observation (connected-component boxes of the drift,
  bounded displacement search minimizing the exact remainder) — and chooses
  the least interval bytes (`J_B`, deterministic family tie order); RAW and
  SPARSE stay alive as sentinels. The encoder **proves** its output by
  re-materializing every observation sample-for-sample and reports honest
  per-family accounting with the RAW-floor reference. This is deliberately
  not the §92/§93 candidate DAG (V.1.11) and not trajectory *promotion over
  time* or affine *proposals over real video* (V.1.5+) — the trajectory and
  affine semantics themselves are sealed here and courted.

## Courts

`tests/phase_v1_4.rs` (18) + `src/media` unit courts (gen 4 · encode 6). |
Result
|---|---|
| v1 specialization parity at depth 8 — velocity/advance translation, linear + acceleration trajectories, palette-index content with `SetPalette`/`PatchPalette` mutation, all four generator programs, Q8 quarter-turn + 2× zoom affine placement, and the transform-coded residual: each scenario authored once as a sealed v1 Gray8 stream and once as a v2 single-plane depth-8 program; **every materialized frame byte-identical** | PASS |
| Authored 10-bit YUV420 semantic surface (velocity sprites, trajectory, generator content, palette-index + mutation, palette binding in the initial state) matches an **independent per-plane compositor** written in the court (closed-form trajectory positions, no shared paint code) on every observation | PASS |
| Family encoder: static runs ride unchanged groups; 10-bit gradients are declared once as generator content; 4-value 10-bit fields are declared once as palette-index content; a translating textured sprite is served by CopyRect region reuse (translation) from its second frame, with the first appearance on a residual class; every run materializes exactly and the total interval bytes are measured against the RAW floor (never hidden) | PASS |
| Depth-8 generator identity: `media/gen` samples == the sealed v1 Phase-N generator on every coordinate for equivalent parameters (four kinds) | PASS (unit) |
| Exact fit courts: gradient fits recover base/slopes in the sample domain (wrapped and negative slopes canonicalized), checker fits recover cell/colors; perturbed content does not fit | PASS (unit) |
| Changed-area decomposition finds disjoint components; copy simulation mirrors the core clip rule (in-bounds-only writes) | PASS (unit) |
| Wire extension: byte roundtrips (`write ∘ parse == id`) across 8 layout×depth rows using generator/index content, velocity/trajectory/advance ops, palette ops, affine, and transform residuals — including odd geometries and 16-bit; minimal feature bits (0 without the extension surface, `0x1` with it); old-surface streams keep feature bits 0 | PASS |
| Extension golden pinned (`55c7f4cc…d625007`); V.1.2 golden unchanged (a5c1fb40…) | PASS |
| Hostile extension corpus typed, never a panic: feature bit cleared on an extension stream, an extension op tag without the bit (no tail), an unknown tail motion kind, semantic-reference errors (`SetVelocity` on an unknown instance ⇒ `UnknownInstance` at materialization), truncations and flips across the file | PASS |
| Transform floor: encode → op-0x31 decode == target at 10-bit through the wire roundtrip; randomized 16-bit delta fields round-trip exactly over four trials; truncated blocks are typed | PASS |
| Full A–U / V.1.1–V.1.3 regression: dev 369 / all-features 371 / release 371, 0 failures; v1 goldens unchanged; the frozen V.1.2 grammar's byte meaning is unchanged | PASS |

## Measured (release, `examples/family_proof.rs`)

Family encoder over an authored 8-observation 10-bit YUV420 run (textured +
sprite translation + palette fields + gradient): **3 664 B interval bytes vs
33 327 B RAW whole-plane floor (9.10×)** across 21 plane-observations —
unchanged 13 obs (156 B), translation 3 obs (111 B), generator 1 obs (56 B),
palette 2 obs (2 082 B), with 4 state syncs; the encoder output re-materializes
exactly fresh and through the re-parse. The 10-bit semantic-surface program
(velocity + trajectory + palette-index mutation + generator + Q8 affine +
transform-coded residual, 7 observations = 8 064 canonical sample bytes)
serializes to a 1 095 B frozen-v2 container with feature bits `0x1`; every
observation re-materializes exactly. Extension golden digest printed
(`55c7f4cc…d625007`) and pinned.

## Recorded, not hidden

* **The v2 grammar is deliberately extended and re-frozen at V.1.4**, per the
  format document's own rule: feature bit `0x1` is additive (old byte streams
  keep their exact meaning and the V.1.2 golden), the hostile corpus grew with
  the new surface, and the extension golden is pinned. Files without the bit
  parse exactly as before.
* **The copy-search score counts clipped cells.** A displacement whose copy
  is fully clipped would "match" trivially (no samples compared); the search
  counts every box cell that would remain wrong after the copy — cells with an
  out-of-bounds source keep the committed render and are compared against it.
  This was found and fixed during V.1.4's sprite court (step 2 chose a useless
  clipped copy before the fix).
* **Region reuse is raster-origin COPY-family reuse** (content re-used from
  the previous materialized observation via `CopyRect`), not instance
  re-identification: sprite/instance tracking belongs to the V.1.11 temporal-
  span search. Trajectory *promotion over time* and affine *proposals over
  real video* are V.1.5+; the trajectory/affine/palette/generator semantics
  themselves are sealed now and are courted at depth 8 against the v1 decoder
  and at 10-bit against the independent compositor.
* The encoder's displacement search is bounded per interval (deterministic
  budget with early exit); a copy candidate is skipped when the drift covers
  > 85% of the plane or the sparse drift is ≤ 24 samples (the sentinels win
  such intervals). All measured, not hidden.
* Regression: dev 369 / all-features 371 / release 371 tests, 0 failures
  (was 341/343/343 at the V.1.3 seal); v1 goldens unchanged; the V.1.2 golden
  bytes unchanged.

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0) ·
`cargo test` (369, dev) · `cargo test --all-features` (371) ·
`cargo test --release --all-features` (371) · hostile extension corpus ·
v1-parity courts · independent-compositor courts · Phase-V.1.4 courts ·
extension golden · evidence (`evidence/campaigns/phase-v1-4-…/`) · docs
updated (`format-v2.md` re-frozen, `empirical-status.md`, `PROJECT_STATE.md`,
`CONFORMANCE.md`, README).

## Next

V.1.5 — global video structure: global translation / rotzoom / affine
proposals over real video with fixed-point normative materialization (brief
§248).

## Verdict

```
SEALED
```
