# Materialization

## Definition

Materialization `M(U, G_t, V)` is the *only* place raster samples are produced.
It performs no search and no lossy choice; its output must be byte-for-byte
reproducible by any conforming decoder against the same `(U, G_t, V)`.

The normative painter (Phase A, `FullFrame` view) is specified in
`src/pixel.rs` (`Canvas`) and `src/materialize.rs` (`materialize_full`):

1. allocate a `width × height` Gray8 canvas and fill it with `state.background()`;
2. for each instance in paint order, paint its immutable object at `(x, y)`
   (or through its affine placement, Phase L),
   overwriting and clipping at the canvas border;
3. (Phase J) a **palette-index object** paints by resolving every stored
   index through the entries of the palette bound to the instance
   (`state.bindings` → `state.palettes`). A missing binding/palette is the
   typed error `UnknownPalette`; an index at or beyond the palette length is
   the typed error `OutOfBounds`. Index planes are immutable content; palettes
   are mutable-by-transition state, so the same plane re-renders with new
   values whenever its palette changes;
4. (Phase L) an instance carrying an **affine placement** paints by scanning
   every canvas pixel and sampling its object through the canonical Q8 source
   map `(su, sv) = ((a·x+b·y+c) >> 8, (d·x+e·y+f) >> 8)` — integer
   everywhere, floor rounding; a sample inside the object rectangle paints it
   (with the same fill/raster/palette-index kind semantics as the plain
   placement), a sample outside leaves the underlying canvas, and cumulative
   per-materialization affine sample work is capped by
   `Limits.max_affine_work`;
5. return the canvas.

Blit/copy-left semantics and clipping are defined by `Canvas::blit`,
`fill_rect_clipped`, and the fill expansion in `src/object.rs`. These are the
authoritative definitions; the encoder never "trusts" its own hypothesis — any
winning candidate is proven by running exactly this materializer.

## Exactness

For a lossless target `F*`:

```
M(U, G_t, V_canonical) ⊕ R_t = F*
```

Phase A streams carry zero residual requirement for the court content, but the
residual algebra is explicit the moment it is used (§4/§22 of the paper;
`docs/residuals.md` later phases).

## Independence

The materializer shares no logic with the independent reference painter used
by the Phase-A conformance court (`src/demo.rs::reference_painter`) or the
Phase-L affine court's structurally different incremental sampling painter
(`src/demo.rs::affine_reference_painter`). The court
tests therefore catch shared-blit bugs: any divergence between the two would
fail `tests/court.rs` / `tests/phase_l.rs`. Boundary frames are additionally
hashed under SHA-256
against an independently re-derived reference in `proof/`.

## Views

`View::FullFrame` is the only materialization target exposed by Phase A, but
the API is value-based so `Tile`/`Rect`/`Scale` views can be added later
without reshaping the decoder (`docs/architecture.md` §5). Partial
materialization claims are an *empirical* later-phase question, not an
assumed win.

## Cost

Materialization of a *full view* is O(canvas) fill plus the painted object
pixels (clipped). Phase-A reporting measures full-frame latency; tile/region
latency and cache effects are courted once partial views exist.
`#![forbid(unsafe_code)]` and checked arithmetic hold on every path.
