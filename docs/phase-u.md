# Phase U Receipt — perceptual profile: deterministic integer quantization `Q`, declared chosen reconstruction `F̂`, measured bytes↔distortion trade, exact profiles intact (§64 Phase-U block / §72 language) (SEALED)

## Deliverable

`vole_video::lossy` (`src/lossy.rs`) + the `vole quant` CLI (Phase-U
perceptual profile over raster-origin input), with a **v1 extension that is a
declaration only**:

* **feature bit `0x2`** (`FEAT_QUANTIZED_CONTENT`) — the stream's frames are
  the encoder's *chosen reconstruction* `F̂`, not the original capture. The
  grammar, materializer, and reconstruction are unchanged; the bit is never
  set by an exact stream and never changes decoding. `read_header` accepts
  `0x1 | 0x2` as the known-bit mask; unknown bits still fail closed.

The lossy architecture keeps the normative decoder **exact and
authoritative**: lossiness lives entirely in the deterministic integer
quantization `Q` applied at *encode time*:

```text
F̂ = Q(F)                chosen reconstruction (integer lattice ± pre-filter)
stream = encode_exactly(F̂)     exhaustive inverse encoder on F̂ (unchanged)
proof: decode(stream) == F̂     proven per stream through the materializer
declared: feature bit 0x2      provenance statement; never enforced as content
```

* [`QuantProfile`] — `shift 0..=7` (lattice `2^shift`; shift 0 with no filter
  = the exact profile), [`Rounding::HalfUp`] (round half away from zero,
  saturating the top half-bin at the Gray8 maximum `255` — the one
  non-lattice output, exact and documented) or [`Rounding::DeadZone`] (never
  leaves the lattice), and an optional [`Filter::Box3`] integer `[1 2 1] ≫ 2`
  horizontal low-pass with edge replication (uniform rows preserved exactly).
  **No floating point anywhere in the normative path.**
* [`quantize_frames`] applies `Q`; [`distortion`] measures the integer
  MAE×1000 / MSE / peak statistics of `F̂` against the source.
* [`encode_lossy`] quantizes, runs the **exhaustive inverse encoder on `F̂`**,
  marks the stream (unless the profile is exact), and **proves** the
  normative decoder reproduces `F̂` byte-for-byte — a stream for which the
  proof fails is a typed error, never a claim. Residual dropping is realized
  *through* the lattice: detail below the step never reaches a residual
  because the encoder's target is already on the lattice.
* [`rate_distortion`] evaluates the deterministic bytes+distortion ladder
  over shifts `0..=max_shift`; [`choose_rd`] selects under a byte budget the
  **least-distorted evaluated row that fits** (tie: smaller stream) — the
  RD-optimal choice over the *measured* ladder, with an honest
  `budget_met == false` when even the exact profile cannot fit (never a
  silent violation). No budget = the smallest stream (tie: least distorted).
* The declaration survives the pipeline: transport (`Packet::Header` carries
  `feature_bits`; reassembly byte-identical), the archive profile
  (`ArchiveManifest::build` + deep verify `Complete`), and `vole optimize`
  (rewrites are decode-identical, so the output is re-marked — the
  provenance statement survives). Exact profiles and every earlier-phase
  stream are untouched.

`vole quant --width W --height H [--frames N] --shift S [--rounding
halfup|deadzone] [--filter none|box3] in.raw out.vole` writes the marked
stream and prints the measured bytes, MAE/MSE/peak, declaration status, and
reconstruction-proof status.

## Courts (`tests/phase_u.rs`, 14 tests)

| Court | Result |
|---|---|
| Quantizer == documented integer lattice over the full Gray8 domain × shifts 0..=7 × both roundings (saturation formula `min(255, ((v+2^(k−1))>>k)<<k)`); spot semantics incl. shift-7 top half-bin → `255`, dead-zone never leaves the lattice, shift 0 identity, shift > 7 typed | PASS |
| Box3 pre-filter hand-computed (`[10,20,30,40] → [13,20,30,38]`, edge replication); uniform rows preserved exactly; deterministic; Box3 at shift 0 is a lossy profile | PASS |
| `quantize_frames`: geometry preserved, every sample on-lattice (or the documented `255`), deterministic; identity profile exact | PASS |
| Distortion metrics hand-computed (`[0,1,2,3] → q2 → [0,0,4,4]`: MAE×1000 1000, MSE 1, peak 2); zero for identical; mismatches typed | PASS |
| Exact profile: lossless, no declaration, byte-identical to the plain Phase-G encode, decode == source | PASS |
| Lossy `encode_lossy`: exact=false, bit set, **decoder output == F̂ == Q(source)** (independent re-proof), marker idempotent, deterministic | PASS |
| Shift-7 half-up saturation on full-range content: outputs exactly `{0,128,255}`, reconstruction proof holds | PASS |
| RD ladder: rows 0..=max_shift, row 0 distortion 0, distortion monotone non-decreasing in shift (`Filter::None`), rows agree with direct encodes, declaration rule per row; `choose_rd` semantics against the *measured* surface (least-distorted fit, recomputed expectation; mid budgets where exact wins when it fits; budget at cheapest row; budget 0 → honest unmet; no budget → smallest) | PASS |
| Declaration is a pure declaration: fake-set on an exact stream and fake-clear on a quantized stream both decode to identical frames | PASS |
| Marker refuses store-backed (bit 0x1) and truncated input typed | PASS |
| Quantized streams survive transport (byte-identical reassembly), archive (build + deep verify Complete), and `vole optimize` (declaration preserved, decode-identical) | PASS |
| Noise negative control: quantized noise decodes to `F̂` exactly; encoder decisions stay RAW (procedural fraction < 0.15) — never "explained" | PASS |
| Regression: Phase-A golden (101 full-HD frames, feature_bits 0) and every earlier surface decode unchanged; hostile flip of a marked stream is still `IntegrityMismatch` | PASS |
| Authoring misuse typed (shift > 7, empty sources) | PASS |

## Measured (release)

### Flagship A — flat panel + 2-bit temporal sensor jitter (480×270 ×17, raw 2 203 200 B)

| profile | .vole B | MAE | MSE | peak |
|---|---|---|---|---|
| q0 exact | 1 806 807 | 0 | 0 | 0 |
| q1 | 1 910 094 | 0.499 | 0 | 1 |
| q2 | 2 068 264 | 0.999 | 1 | 2 |
| q3 | **270** | 1.499 | 3 | 3 |
| q4 | 270 | 6.500 | 43 | 8 |

**Exact → q3 = 6 692× at MAE 1.5** (per-frame amortized: 106 282.8 B exact →
**15.9 B/frame** q3 — the step-8 lattice clears the jitter, `F̂` is a flat
panel, and the stream is one fill + the unchanged lane). q4 ties q3's bytes
at strictly higher distortion — a **dominated row**; the RD choice never
picks it (budget ¼ of exact chose q3 at 270 B, budget_met).

### Recorded findings (measured, not hidden)

* **Bytes are NOT monotone in the lattice step.** On both flagships the
  intermediate lattices *exceed the exact stream* (q1/q2 > q0): an
  intermediate lattice keeps the residual dense while destroying the exact
  residual's structure (the transform floor codes the pristine ±{0..3}
  jitter field at ~0.8 B/sample; the q1/q2 fields carry wider-spaced
  outliers at higher cost). q3 then snaps the jitter off entirely. The RD
  choice is defined against the measured ladder for exactly this reason.
* **Authored-procedural control (moving rect, 13 frames): exact 33 173 B ==
  q2 33 173 B** with q2 MAE 0.975 — quantization adds measured distortion
  without removing procedural-state bytes. Recorded negative: on content the
  exact path already explains procedurally, lossy offers nothing; the exact
  profile stays intact and the RD choice falls back to exact when it fits
  the budget.
* **Noise negative control** (192×128 ×3): 73 909 B at q0/q2/q4 — identical
  bytes (whole-canvas RAW raster objects dominate and alphabet reduction
  does not shrink them), distortion grows (MAE 1.5 → 7.5); quantization never
  turns noise into state (§62). Exact-reconstruction proof holds at every
  shift.
* Flagship B (smooth ramp + jitter, 320×180 ×9): exact 433 042 B → q3
  346 392 B at MAE 2.0, with the same recorded q1/q2 non-monotonicity.

Every stream in every ladder row was decoded back through the normative
decoder and proven byte-equal to `F̂` before being reported. The exact
(lossless) ladder is untouched: no declaration, decode byte-identical to the
source.

## Recorded, not hidden

* No `.vole` grammar change beyond the *declaration* bit `0x2`; v1 goldens
  and all earlier-phase streams decode unchanged (`feature_bits` 0). The
  bit never changes reconstruction and is never enforced as a content check
  (fake-set/fake-clear courts) — only canonicality is enforced.
* Half-up quantization saturates the top half-bin at the Gray8 maximum
  (`255` is the one non-lattice output); dead-zone never leaves the lattice.
  Both behaviors are exact and courted over the full sample domain.
* This phase makes **no perceptual-quality claim** (no subjective
  evaluation, no PSNR-vs-codec comparison): it measures a declared,
  deterministic integer trade of bytes for MAE/MSE/peak on raster-origin
  content. Conventional lossy codecs and perceptual evaluation remain
  external-harness territory (§72 language throughout).
* "Residual dropping" is realized through the lattice (the target of the
  encoder is `F̂`), never by suppressing residual bits the exact profile
  needs — the decoder's reconstruction obligation is unchanged.

## Gate

`cargo fmt --check` · `cargo check --all-targets` (dev + all-features) ·
`cargo clippy --all-targets --all-features -- -D warnings` (0 warnings) ·
`cargo test` (277, dev) · `cargo test --all-features` (279) ·
`cargo test --release --all-features` (279) · hostile courts · Phase-U court
· evidence (`evidence/campaigns/phase-u-perceptual-…/`) · docs updated
(`format-v1.md`, `empirical-status.md`, `CONFORMANCE.md`, `PROJECT_STATE.md`,
README).

## Verdict

```
SEALED — the mandated ladder A → U is complete
```
