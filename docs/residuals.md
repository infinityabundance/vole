# Residuals and the residual algebra

The residual `R_t` is the information a procedural explanation does **not**
reproduce: `F_t = M(U,G_t,V) ⊕_ρ R_t`. The algebraic operator `ρ` is always
explicit — never implied. Phase A streams carry no residual requirement for
court content; `R=∅` means the state alone reproduces the target exactly.

Design notes (details coded as those phases land):

* Residual is a **first-class information object**, not a "failure" label; its
  magnitude/support/spatial distribution/temporal persistence are encoder
  evidence for later DSFB governance.
* Two residual algebras are normative in v1 (both one-shot canvas ops, tag
  `0x2a`):
  * the **point algebra** (Phase G): a strict-sorted point list whose samples
    **overwrite** their pixels with the target value — the natural closure
    for sparse support;
  * the **additive block-transform algebra** (Phase M): the signed residual
    field `target − base` is coded per aligned 4×4 block by the normative
    integer lifting DCT and **added** back at decode — the conventional
    transform floor for dense smooth deltas (see `docs/format-v1.md` and
    `docs/phase-m.md`).
* Residual families planned include XOR/MODULAR_ADD/SIGNED_DELTA/SPARSE
  overwrite/palette-exception/RAW replacement.
* Accounting rule: a region that needs `|R| ≈ |F|` must choose an ordinary
  literal/entropy mode instead — never "pretend" the residual is small.

Status: IMPLEMENTED (point algebra since Phase G; transform algebra since
Phase M). See `docs/information-accounting.md`.
