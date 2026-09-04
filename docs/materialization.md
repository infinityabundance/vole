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
5. (Phase N) a **procedural generator object** paints by computing each
   sample of its box from the bounded integer program at materialization
   (clipped exactly like a fill); the program is validated canonically and
   its work is the painted area;
6. return the canvas.

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

`View` is a typed materialization target (`src/view.rs`): `FullFrame`, an
arbitrary in-canvas sub-rectangle (`Rect`), and one tile of a canonical tile
grid (`Tile`). Views are *requests* — never stream syntax — and every view is
defined by one contract: the returned samples are exactly the samples a
whole-frame decode would place in the view's in-canvas region (the view's own
top-left becomes the returned canvas origin).

### Whole-frame (canonical audit)

`View::FullFrame` (and any box covering the whole canvas) replays the
canonical step machinery (`materialize_full` + the interval stepper): it is
byte- and error-identical with whole-frame decode by construction. Whole-frame
decode remains the canonical audit path for a stream.

### Partial views (Phase S)

A sub-frame view (`Rect`/`Tile`, `src/partial.rs`) is materialized by a
**demand plan** followed by a forward replay. Frame semantics make this
exact: every interval canvas op (COPY_RECT/MOVE_RECT/residual) reads **only**
the immediately previous frame and overpaints the freshly materialized base,
so the value of frame `t` at a position is the base state paint, the value of
frame `t−1` at the source of the **last** canvas op covering the position, or
the value carried by that op itself. Walking backward from the requested
region therefore yields, per level, exactly the region of the previous frame
that the next frame reads; the forward replay then paints only those regions
(background, instances, overlay) and applies the ops over them. Levels whose
region is empty are skipped entirely, and state transitions are replayed
exactly as in whole-frame decode. Demand regions are merged per-row spans
that saturate to the whole canvas beyond a bookkeeping budget — a sound
over-approximation that keeps hostile streams in the whole-frame memory
class.

Measured work is reported by `PartialStats`: painted sample writes
(base/copy/residual), levels materialized, frames replayed, distinct objects
touched (the decode-time analogue of object fetches — an object wholly
outside the demanded region is never touched), and peak per-level raster
memory.

### Audit-scope semantics of a partial view

A partial view validates everything that contributes to its region: state
transitions, op framing, and whole residual containers (a residual block's
point list is fully validated and a kind-2 transform residual's DC/AC streams
are fully decoded; bounds are always checked against the canvas, never the
partial frame's box). Content that never contributes to the view — an
instance painted wholly outside the demanded region, an affine overflow
outside it, or a residual error on a level the view never touches — is not
audited. A view is therefore a *sampling* contract: its samples are exact
and agree with whole-frame decode sample-for-sample, while whole-frame
decode remains the canonical audit path. Courts pin both sides of this
boundary (`tests/phase_s.rs`).

Partial materialization claims are an *empirical* question, measured in
`docs/phase-s.md`: on the sealed 1920×1080 court a tracking 260×140 viewport
painted 2.72% of the whole-frame lane and reached frame 40 in 0.068 ms vs
13.5 ms for the whole-frame random-access path (release example
`examples/partial_proof.rs`).

## Cost

Materialization of a *full view* is O(canvas) fill plus the painted object
pixels (clipped). A partial view is O(replayed transitions) plus O(region
paint) — the raster work tracks the region of interest, not the canvas.
`#![forbid(unsafe_code)]` and checked arithmetic hold on every path.
