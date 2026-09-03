# Phase J Receipt — palette state (SEALED)

## Deliverable

Palette state as first-class procedural state, so that color animation is a
*tiny state mutation* rather than a raster rewrite:

* **palette-index objects** (`0x05`; `Object::index_raster`) — immutable
  content whose bytes are one-byte indices into a palette;
* a mutable **palette table** in `G_t` (`0x06` pre-checkpoint records +
  `SetPalette` 0x2d whole replacement / `PatchPalette` 0x2e entry patches),
  bounded by new `Limits.max_palette_entries` (256) and `Limits.max_palettes`
  (4096);
* **per-instance palette bindings** (`0x08` palette-binding checkpoint
  variant so palette content renders from frame 0; `BindPalette` 0x2f in
  intervals; `pal = 0` unbinds; bindings die with their instances, palettes
  persist).

Materialization (normative): an index raster paints `entries[idx]` for every
stored index through the palette bound to the painting instance — a missing
binding/palette is the typed error `UnknownPalette`, an index at or beyond
the palette length is `OutOfBounds` (validated before any pixel is written).
No floating point, no per-frame index rewrites: `F_t = indices ∘ entries(t)`.

Old streams are unaffected: `0x03` checkpoints, raster/fill objects, and
every earlier tag parse exactly as before (v1 continues to extend per sealed
phase with tags 0x05/0x06/0x08/0x2d–0x2f).

## Courts

`tests/phase_j.rs` (12 tests) + `tests/malformed.rs` (9 new Phase-J hostile
courts), all byte-exact against an independent palette painter:

* accent cycling (`PatchPalette`): 41 frames of a window-UI index plane on
  640×360 — title bar / sidebar / separators / body stay put while the
  status bar toggles with the palette; exact vs the painter;
* whole-palette rotation (`SetPalette`): every pixel of every frame changes
  (color drift) — exact, and distant frames differ;
* flattening-tax court (§55 on UI content): the same visual frames encoded
  (a) as authored palette state and (b) rasterized then inverse-
  proceduralized — the palette stream is strictly smaller;
* typed semantics: empty palette / reserved id 0 non-canonical; bind to an
  undeclared palette `UnknownPalette`; unknown instance `UnknownInstance`;
  unsorted/duplicate patches non-canonical; patch index out of range
  `OutOfBounds`; index raster without binding / index ≥ palette length fail
  materialization typed (never panic, never wrap); duplicate palette decls
  `DuplicateId` at encoder and parser; palette stream accounting buckets sum;
  index-plane content identity is exact, distinct from a gray raster of the
  same bytes, and geometry/content-sensitive;
* multiple palettes + multiple bound instances share one checkpoint and each
  box renders through its own palette;
* static control: a never-changing palette yields identical frames;
* palette streams are `collapse_stream` fixpoints (Ok(None), never an error);
* hostile wire courts: index-object geometry bomb, checkpoint binding to an
  undeclared palette, interval bind to an undeclared palette, patch count
  bomb (> 256), duplicate patch index, out-of-range patch, empty and
  oversized `SetPalette` payloads — all typed errors before the integrity
  check.

## Measured (evidence/campaigns/phase-j-palette-<ts>, release)

| court | frames | representation | bytes | exact |
|---|---|---|---|---|
| accent flag 1920×1080 (UI index plane) | 101 | palette (`PatchPalette`/interval) | 2 076 110 B total; **24 B/interval** | ✓ |
| | | palette-less sparse floor | 204 773 B/interval (8 532×) | — |
| | | RAW | 2 073 600 B/frame (86 400×) | — |
| rotate flag 1920×1080 | 101 | palette (`SetPalette`/interval) | 2 076 510 B total; **28 B/interval** while every pixel changes | ✓ |
| flattening tax 240×160 | 13 | authored palette intervals | 288 B | ✓ |
| | | raster-origin inverse encode intervals | 50 016 B (**174×**) | ✓ |
| static palette 640×360 | 201 | established palette at rest | marginal **13 B/frame** == unchanged lane | ✓ |

The one-time index-plane declaration (≈ 2 073 600 B at 1920×1080) dominates
the total stream bytes on these whole-canvas courts — the same frame-0
declaration tax Phase G measures, targeted by Phase K (regions) and Phase Q
(native ingest). The *maintenance* is where palette state wins: 24 B to
re-map a 2M-sample region whose structure never changes.

## Adopted / rejected / recorded

Adopted: palette-index objects with exact content identity; a mutable,
bounded palette table; per-instance bindings (checkpoint `0x08` + interval
`0x2f`); `PatchPalette` (strictly ascending, in-range) and `SetPalette`
(whole replacement) mutations; materialization-time index validation with
typed errors; canonical record forms; new limits; accounting buckets
(`state_bytes` now carries pre-checkpoint palette-table records; new
informational `index_object_bytes` split).

Rejected: palette-as-immutable-object (would forbid palette mutation);
global single palette (would break per-object palettes); unbounded palette
tables/entries (hostile caps enforced at parser, encoder validator, and
writer).

Recorded, not hidden: the raster-origin inverse encoder has **no palette
family yet** — the 174× interval gap on identical visual frames is the
measured cost of flattening palette structure into rasters; palette
*discovery* in the raster encoder (color clustering, index-plane stability
detection) is Phase O/Q territory, exactly like trajectory collapse was for
motion. Whole-canvas index planes pay a one-time declaration equal to their
raster area (Phase K regions and Phase Q ingest attack that).

## Verdict

```
SEALED
```
