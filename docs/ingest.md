# Native procedural ingest (Phase Q)

## The API (`vole_video::ingest::Ingest`)

Applications that already possess procedural state — UI hierarchies, vector
animation, game/simulation state, motion graphics, dashboards, deterministic
scene composition — emit that state **directly** (§39) instead of rendering to
rasters and letting the inverse proceduralizer infer the structure the source
just destroyed:

```rust
use vole_video::ingest::Ingest;
use vole_video::trajectory::TrajectorySegment;

let mut a = Ingest::new(1920, 1080);
a.background(30);
a.declare_raster(1, 200, 100, logo_bytes);     // immutable object
a.declare_generator(2, 200, 100, gradient);    // or a bounded program
a.declare_palette(1, entries);                 // palette table (Phase J)
a.instance(1, 1, 100, 40);                     // checkpoint instance
a.instance_binding(2, 1, 0, 0, 1);             // palette-bound instance

a.at(1)?; a.set_velocity(1, 2, 0)?; a.advance()?;          // translation
a.at(2)?; a.advance()?;
a.at(3)?; a.set_trajectory(1, vec![TrajectorySegment::Accel {
    vx0: 0, vy0: 0, ax: 1, ay: 1, steps: 30 }])?;
a.at(4)?; a.advance_trajectories()?;
a.at(5)?; a.patch_palette(1, vec![(1, 200)])?;

let bytes = a.finish()?;                        // canonical standalone .vole
```

Design rules:

* `Ingest` is a **thin, typed layer over the normative encoder**: `finish()`
  re-validates every descriptor through `encoder::encode_stream` /
  `encode_palette_stream` (geometry via `Limits.check_canvas`, duplicate ids,
  unknown object/instance/palette references, interval ordering, budgets), so
  an `Ingest` stream is **byte-canonical by construction** and misuse is a
  typed `VoleError`, never a malformed file.
* Time model: `at(t)` opens the interval group at absolute frame `t` (`t ≥ 1`,
  strictly increasing across different times; repeating `t` appends to the
  group). `finish()` yields `intervals + 1` materialized frames, exactly like
  the standalone decoder.
* Every v1 transition has a helper (`set_position`, `set_velocity`,
  `advance`, `set_trajectory`, `advance_trajectories`, `set_affine`,
  `set_palette`, `patch_palette`, `bind_palette`, `copy_rect`, `move_rect`,
  `patch_sparse`, `residual`, `clear_instances`, `clear_overlay`), plus the
  object/palette/instance declarations. `raw` byte payloads (raster samples,
  residual blocks) are produced by the caller; the rANS helpers in
  `vole_video::rans` build residual blocks.
* Nothing here re-implements wire semantics; there is **no wire-format change**
  in Phase Q.

## The §53 research-harness script format (`vole_video::script`)

A deterministic text format for authoring procedural content in courts and
examples. **Not a normative VOLE syntax** and never part of the `.vole` wire:

```text
# comment
canvas 192 108
background 40
palette 1 200 60 90 150 220
object 1 index 192 8 1 1 1 ...        # exactly W*H byte values (may span lines)
object 2 fill 64 32 90
object 3 gradient 64 32 10 2 1        # generator programs
object 4 checker 64 64 5 250 8
object 5 periodic 64 64 0 2 1 16
object 6 noise 64 64 7                # authored seeds only (never discovered)
instance 1 1 0 60 palette 1
instance 2 2 10 10
at 1
  move 2 12 10
  patch_palette 1 1=200
at 2
  velocity 2 2 0
  advance
at 3
  trajectory 2 lin 2 0 5
  advance_traj
```

`script::parse_script(&text)?.finish()` produces the same canonical bytes a
hand-built `Ingest` produces. Malformed scripts are typed
(`VoleError::ScriptParse` with a stable condition name).

## The §55 native-procedural preservation court

For the same authored content the court measures:

* **A** — direct ingest bytes;
* **B** — materialize the canonical raster sequence (normative materializer),
  then re-proceduralize with the exhaustive inverse encoder;
* **C** — raw raster bytes (external conventional-codec baselines belong to
  the §57 harness, outside this repo).

Both legs must reproduce the same raster sequence byte-for-byte, and the
**flattening tax** `B/A` (total and per-interval marginal) is measured.
Sealed numbers on the synthetic courts (release, `tests/phase_q.rs`,
`examples/ingest_proof.rs`, `docs/phase-q.md`): palette rotation 7.7× total /
180× per interval; palette accent strip 8.6× / 2.5× (B recovers reusable
region content but carries zero palette state); accelerating motion 37× /
28×; affine rotation of a noise tile 49× / 53×; authored seeded-noise region
33× (structural: the seed is unknowable to search). The interval marginal tax
is the quantity that compounds with stream length.
