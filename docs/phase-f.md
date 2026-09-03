# Phase F Receipt — native rANS entropy floor (SEALED)

## Deliverable

A native, deterministic, **order-0 byte rANS** coder owned by this crate
(`src/rans.rs`), with a declared RAW-fallback accounting policy, hostile-input
guards, byte-parity courts against the author's independent `ryg-rans-rs`
reconstruction (test-only oracle, never linked into normative decode), and a
self-describing payload container.

## Normative semantics (frozen for universe v1 / profile 1)

| Quantity | Value |
|---|---|
| State width | 32-bit unsigned |
| Frequency scale | `scale_bits = 14`, `MODEL_TOTAL = 16384` |
| Normalization lower bound | `STATE_L = 2^23` |
| Encoder renorm | per-symbol `x_max = ((STATE_L >> scale_bits) << 8) * freq`, byte-wise, before each `C(s,x)` step |
| Step | `C(s,x) = ((x / freq) << scale_bits) + (x % freq) + start` |
| Decoder | LIFO; inverts the step then renorms `while x < STATE_L` reading bytes forward |
| Initial/final | initial state `STATE_L`; final state flushed raw as `u32` LE |
| Endianness | little-endian everywhere |
| Model | 256 × `u16` LE frequencies (512 B); deterministic largest-remainder normalization (min 1 per present symbol, extras by fraction desc / index asc) |
| Payload layout | `[kind u8][out_len u64 LE][model?][rans body?]`; RANS body = `[state u32 LE][renorm bytes reversed]` |

**RAW fallback policy (declared):** choose RANS only when
`MODEL_SERIALIZED + rans_bytes < raw_len`; otherwise store RAW. Uniform /
incompressible input converges to RAW (measured: 262 144 uniform bytes →
RAW, 9-byte envelope overhead only).

## Courts (`tests/phase_f.rs`, 8 tests; `src/rans.rs` unit tests, 6)

- **Byte parity vs `ryg-rans-rs` 0.5.1** (dev-dependency oracle): our encoder
  output is byte-identical to the reference single-state byte rANS for the
  same model over an adversarial corpus (uniform, single-symbol runs, weighted
  binary, run patterns, text skew, 200k alternating runs, lengths 0..16384);
  cross-decode succeeds in both directions.
- Property round-trips of the self-describing block container; model
  serialization round-trip is canonical (re-encode byte-identical).
- Accounting: single-symbol 262 144 B → RANS 525 B (499×); skewed text
  262 144 B → RANS 4 458 B (59×); uniform → RAW with exact decode.
- Hostile: every prefix truncation is a typed error; corruption is
  deterministic and bounded (valid decode or typed error, never panic);
  declared-length bombs fail `DimensionTooLarge` before allocation under a
  caller-supplied bound; over-declared lengths on non-degenerate data fail
  structurally (renorm overread).
- Pinned normative constants test guards accidental semantic drift.

## Evidence

`evidence/campaigns/phase-f-rans-…/summary.json` (measured sizes, kinds,
`verify=true` decode equality) and `environment.json` (commit/rustc/platform).

## Open / next

Phase G will wire rANS as the `RANS_RESIDUAL` / raster-payload floor inside the
exhaustive inverse-proceduralization court (the block container above is the
coded-payload primitive it consumes). The 9-byte envelope and 512-byte inline
model are the reported accounting baseline; future phases may hoist shared
models into the object table where identity reuse pays.

## Verdict

```
SEALED
```
