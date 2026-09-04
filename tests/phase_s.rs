//! Phase S courts: partial materialization (master brief §16/§37/§66) —
//! `View::Rect` / `View::Tile` decode-work measurements against whole-frame
//! decode, with sample-for-sample parity as the governing property:
//!
//! > partial(idx, view) == crop(whole-frame decode of idx, view's in-canvas
//! > region), for every view and every stream shape (including COPY_RECT
//! > chains, residuals, palette content, affine placements, generators).
//!
//! Plus: FullFrame-view identity with the canonical decoder, tile-grid
//! partition exactness, decode-work reduction bounds, hostile geometry typed,
//! and the documented audit-scope boundary (content that never contributes to
//! a view is not audited; in-view content errors identically to whole-frame
//! decode).

use vole_video::{
    decoder, ingest::Ingest, materialize, partial, pixel::Canvas, rans::KIND_RAW,
    transition::Transition, view::View, VoleError,
};

// ---------------------------------------------------------------------------
// Deterministic helpers
// ---------------------------------------------------------------------------

/// Deterministic xorshift64 for court content (fixed seeds).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

/// Reference crop of a full frame (test-side; independent of the partial
/// decoder).
fn crop(frame: &Canvas, x0: u32, y0: u32, w: u32, h: u32) -> Result<Canvas, VoleError> {
    let mut data = Vec::with_capacity((w as usize) * (h as usize));
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            data.push(frame.get(x, y));
        }
    }
    Canvas::from_parts(w, h, data)
}

fn assert_view_matches_frame(pv: &partial::PartialView, frame: &Canvas, x0: u32, y0: u32) {
    let w = pv.canvas.width();
    let h = pv.canvas.height();
    let expect = crop(frame, x0, y0, w, h).expect("crop");
    assert!(
        pv.canvas.exactly_matches(&expect),
        "partial view at {x0},{y0}+{w}x{h} must equal the whole-frame crop"
    );
}

/// A deterministic in-canvas view rect (occasionally crossing edges).
fn random_view(rng: &mut Lcg, cw: u32, ch: u32) -> View {
    let w = 1 + rng.below(u64::from(cw)).min(24) as u32;
    let h = 1 + rng.below(u64::from(ch)).min(24) as u32;
    let cx = rng.below(u64::from(cw)) as u32;
    let cy = rng.below(u64::from(ch)) as u32;
    View::Rect {
        x: cx.saturating_sub(w / 2) as i64,
        y: cy.saturating_sub(h / 2) as i64,
        width: w,
        height: h,
    }
}

fn texture(seed: u64, w: u32, h: u32) -> Vec<u8> {
    let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    (0..w * h).map(|_| (rng.next() % 256) as u8).collect()
}

fn raw_block(pts: &[(i32, i32, u8)]) -> Vec<u8> {
    let mut body = Vec::with_capacity(9 * pts.len());
    for (x, y, v) in pts {
        body.extend_from_slice(&x.to_le_bytes());
        body.extend_from_slice(&y.to_le_bytes());
        body.push(*v);
    }
    let mut block = vec![KIND_RAW];
    block.extend_from_slice(&(body.len() as u64).to_le_bytes());
    block.extend_from_slice(&body);
    block
}

/// Strict-ascending point list of `(x, y)` for a canonical RAW residual.
fn sorted_pts(pts: &mut [(i32, i32, u8)]) {
    pts.sort_by_key(|&(x, y, _)| (x, y));
}

// ---------------------------------------------------------------------------
// Authored court streams
// ---------------------------------------------------------------------------

/// Scroll-like copy chain: a vertical band is CopyRect-panned right by one
/// pixel per interval (reading the previous frame) while a sprite moves inside
/// the band — genuine multi-level cross-frame demand for a viewport ROI.
fn copy_chain_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (160u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(17);
    a.declare_raster(1, 12, 12, texture(3, 12, 12))?;
    a.instance(1, 1, 40, 40)?;
    a.declare_fill(2, 4, 4, 200)?;
    for t in 1..=12u64 {
        a.at(t)?;
        a.set_position(1, 40 + 3 * t as i64, 40)?;
        // Pan a 24-wide band one pixel right each interval from the previous
        // frame (dst == src + 1): each level's copy reads the level before it.
        a.push(Transition::CopyRect {
            src_x: 20 + t as i64,
            src_y: 0,
            width: 24,
            height: h,
            dst_x: 21 + t as i64,
            dst_y: 0,
        })?;
        if t == 8 {
            a.create_instance(2, 2, 60, 20)?;
        }
    }
    a.finish()
}

/// Palette-index content with palette patches, persistent sparse points, and
/// sparse residuals (RAW and rANS) on later frames.
fn palette_residual_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (96u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(9);
    a.declare_palette(1, vec![10, 60, 120, 200, 250])?;
    let idx: Vec<u8> = (0..(48 * 48))
        .map(|i| (((i / 48) * 3 + (i % 48) * 5) % 5) as u8)
        .collect();
    a.declare_index(1, 48, 48, idx)?;
    a.instance_binding(1, 1, 10, 10, 1)?;
    a.declare_fill(2, 6, 6, 250)?;
    for t in 1..=10u64 {
        a.at(t)?;
        if t % 3 == 0 {
            let changes: Vec<(u8, u8)> = (0..5)
                .map(|i| (i, (10 + t * 37 + u64::from(i) * 3) as u8))
                .collect();
            a.patch_palette(1, changes)?;
        }
        if t == 5 {
            a.patch_sparse(vec![(70, 70, 99)])?;
        }
        if t == 7 {
            let pts = [(4i32, 4i32, 77u8), (30, 12, 88), (60, 60, 99), (80, 80, 11)];
            a.push(Transition::Residual {
                block: raw_block(&pts),
            })?;
        }
        if t == 9 {
            let mut body = Vec::with_capacity(27);
            for (x, y, v) in [(2i32, 2i32, 200u8), (10, 40, 77), (90, 5, 5)] {
                body.extend_from_slice(&x.to_le_bytes());
                body.extend_from_slice(&y.to_le_bytes());
                body.push(v);
            }
            a.push(Transition::Residual {
                block: vole_video::rans::encode_block(&body),
            })?;
        }
    }
    a.finish()
}

/// One affine-rotating noise tile over a big static fill (Phase L).
fn affine_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (160u32, 120u32);
    let mut a = Ingest::new(w, h);
    a.background(40);
    a.declare_fill(1, 140, 100, 70)?;
    a.instance(1, 1, 10, 10)?;
    a.declare_generator(
        2,
        32,
        32,
        vole_video::generator::Generator::Noise { seed: 3 },
    )?;
    a.instance(2, 2, 60, 40)?;
    for t in 1..=8u64 {
        a.at(t)?;
        let params = vole_video::demo::quarter_turn_params(t as i64, 60, 40, 32, 32);
        a.set_affine(2, params)?;
    }
    a.finish()
}

/// Generator drift: a procedural gradient object moves across the canvas.
fn generator_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (128u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(0);
    a.declare_generator(
        1,
        96,
        64,
        vole_video::generator::Generator::Gradient {
            base: 3,
            sx: 3,
            sy: 1,
        },
    )?;
    a.instance(1, 1, 16, 16)?;
    for t in 1..=9u64 {
        a.at(t)?;
        a.set_position(1, 16 + t as i64, 16)?;
    }
    a.finish()
}

/// Big-canvas sprite track (1920×1080): a huge static decorative object plus a
/// moving textured 200×100 sprite — for work-reduction measurement.
fn big_track_stream(frames: u64) -> Result<Vec<u8>, VoleError> {
    let (w, h) = (1920u32, 1080u32);
    let mut a = Ingest::new(w, h);
    a.background(5);
    a.declare_fill(1, 600, 900, 12)?;
    a.instance(1, 1, 100, 100)?;
    a.declare_raster(2, 200, 100, texture(11, 200, 100))?;
    a.instance(2, 2, 800, 500)?;
    for t in 1..=frames {
        a.at(t)?;
        a.set_position(2, 800 + 4 * t as i64, 500)?;
    }
    a.finish()
}

/// Index-trap content: an index instance whose top-left pixel carries an
/// out-of-range index (5 with a 2-entry palette).
fn index_trap_stream() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (96u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(0);
    a.declare_palette(1, vec![200, 60])?;
    let mut idx = vec![0u8; 24 * 24];
    idx[0] = 5;
    a.declare_index(1, 24, 24, idx)?;
    a.instance_binding(1, 1, 10, 10, 1)?;
    a.finish()
}

/// One deterministic random-content movie (valid by construction): random
/// object kinds (fill/raster/index/generator), absolute motion, palette
/// patches, sparse overlay, copy/move rects, and strict-sorted RAW residuals
/// at random intervals. Uses one palette for index instances.
fn random_movie(seed: u64) -> Result<Vec<u8>, VoleError> {
    let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1));
    let w = 24 + rng.below(56) as u32;
    let h = 24 + rng.below(56) as u32;
    let frames = 3 + rng.below(6);
    let mut a = Ingest::new(w, h);
    a.background(rng.below(256) as u8);
    let pal_len = 2 + rng.below(4) as usize;
    let entries: Vec<u8> = (0..pal_len).map(|_| rng.below(256) as u8).collect();
    a.declare_palette(1, entries)?;

    // Objects: (id, w, h, is_index).
    let n_objs = 1 + rng.below(3) as u32;
    let mut kinds: Vec<(u32, u32, u32, bool)> = Vec::new();
    for oid in 1..=n_objs {
        let ow = 4 + rng.below(12) as u32;
        let oh = 4 + rng.below(12) as u32;
        match rng.below(4) {
            0 => {
                a.declare_fill(oid, ow, oh, rng.below(256) as u8)?;
                kinds.push((oid, ow, oh, false));
            }
            1 => {
                let data = texture(rng.next(), ow, oh);
                a.declare_raster(oid, ow, oh, data)?;
                kinds.push((oid, ow, oh, false));
            }
            2 => {
                let data: Vec<u8> = (0..ow * oh)
                    .map(|_| rng.below(pal_len as u64) as u8)
                    .collect();
                a.declare_index(oid, ow, oh, data)?;
                kinds.push((oid, ow, oh, true));
            }
            _ => {
                a.declare_generator(
                    oid,
                    ow,
                    oh,
                    vole_video::generator::Generator::Checker {
                        a: rng.below(256) as u8,
                        b: rng.below(256) as u8,
                        cell: 1 + rng.below(3) as u32,
                    },
                )?;
                kinds.push((oid, ow, oh, false));
            }
        }
    }
    let n_inst = 1 + rng.below(3) as u32;
    for iid in 1..=n_inst {
        let &(oid, ow, oh, is_index) = &kinds[rng.below(kinds.len() as u64) as usize];
        let x = rng.below(u64::from(w).saturating_sub(u64::from(ow))) as i64;
        let y = rng.below(u64::from(h).saturating_sub(u64::from(oh))) as i64;
        if is_index {
            a.instance_binding(iid, oid, x, y, 1)?;
        } else {
            a.instance(iid, oid, x, y)?;
        }
    }

    for t in 1..=frames {
        a.at(t)?;
        // Move instance 1 around the canvas (its object box comes from the
        // first `kinds` entry by construction of the instance loop above).
        let (ow, oh) = (kinds[0].1, kinds[0].2);
        let maxx = w.saturating_sub(ow);
        let maxy = h.saturating_sub(oh);
        a.set_position(
            1,
            rng.below(u64::from(maxx.max(1))) as i64,
            rng.below(u64::from(maxy.max(1))) as i64,
        )?;
        // Random palette patch.
        if rng.below(3) == 0 {
            let changes: Vec<(u8, u8)> = (0..pal_len.min(4) as u8)
                .map(|i| (i, rng.below(256) as u8))
                .collect();
            a.patch_palette(1, changes)?;
        }
        // Occasional sparse overlay point.
        if rng.below(4) == 0 {
            a.patch_sparse(vec![(
                rng.below(u64::from(w)) as i64,
                rng.below(u64::from(h)) as i64,
                rng.below(256) as u8,
            )])?;
        }
        // Occasional small copy or move rect (previous-frame reads).
        if rng.below(4) == 0 {
            let rw = 2 + rng.below(10) as u32;
            let rh = 2 + rng.below(10) as u32;
            let sx = rng.below(u64::from(w).saturating_sub(u64::from(rw))) as i64;
            let sy = rng.below(u64::from(h).saturating_sub(u64::from(rh))) as i64;
            let dx = rng.below(u64::from(w).saturating_sub(u64::from(rw))) as i64;
            let dy = rng.below(u64::from(h).saturating_sub(u64::from(rh))) as i64;
            let tr = if rng.below(2) == 0 {
                Transition::CopyRect {
                    src_x: sx,
                    src_y: sy,
                    width: rw,
                    height: rh,
                    dst_x: dx,
                    dst_y: dy,
                }
            } else {
                Transition::MoveRect {
                    src_x: sx,
                    src_y: sy,
                    width: rw,
                    height: rh,
                    dst_x: dx,
                    dst_y: dy,
                }
            };
            a.push(tr)?;
        }
        // Occasional strict-sorted RAW residual.
        if rng.below(5) == 0 {
            let mut pts = Vec::new();
            let mut x = 0i32;
            let mut y = 0i32;
            for _ in 0..(1 + rng.below(5)) {
                x += rng.below(24) as i32 + 1;
                if x >= i32::try_from(w).unwrap_or(i32::MAX) {
                    x = rng.below(24) as i32;
                    y += 1;
                }
                if y >= i32::try_from(h).unwrap_or(i32::MAX) {
                    break;
                }
                pts.push((x, y, rng.below(256) as u8));
            }
            sorted_pts(&mut pts);
            if !pts.is_empty() {
                a.push(Transition::Residual {
                    block: raw_block(&pts),
                })?;
            }
        }
        let _ = t;
    }
    a.finish()
}

// ---------------------------------------------------------------------------
// Geometry courts
// ---------------------------------------------------------------------------

#[test]
fn view_clip_geometry_is_typed_and_correct() -> Result<(), VoleError> {
    let (cw, ch) = (64u32, 48u32);
    let b = View::FullFrame.clip(cw, ch)?.expect("full");
    assert_eq!((b.x, b.y, b.width, b.height), (0, 0, 64, 48));
    assert!(b.is_full(cw, ch));
    let b = View::Rect {
        x: 10,
        y: 5,
        width: 8,
        height: 4,
    }
    .clip(cw, ch)?
    .expect("inside");
    assert_eq!((b.x, b.y, b.width, b.height), (10, 5, 8, 4));
    // Partial overhang clips to the canvas.
    let b = View::Rect {
        x: -3,
        y: 46,
        width: 10,
        height: 10,
    }
    .clip(cw, ch)?
    .expect("clipped");
    assert_eq!((b.x, b.y, b.width, b.height), (0, 46, 7, 2));
    // Fully outside => no intersection.
    assert!(View::Rect {
        x: 100,
        y: 0,
        width: 8,
        height: 8
    }
    .clip(cw, ch)?
    .is_none());
    assert!(View::Rect {
        x: -20,
        y: -20,
        width: 4,
        height: 4
    }
    .clip(cw, ch)?
    .is_none());
    // Tile grid: tile (3,2) of 16×16 tiles on 64×48 is the box at (48,32);
    // tile (4,2) starts at x=64 and does not intersect the canvas at all.
    let b = View::Tile {
        tile_x: 3,
        tile_y: 2,
        tile_w: 16,
        tile_h: 16,
    }
    .clip(cw, ch)?
    .expect("inside");
    assert_eq!((b.x, b.y, b.width, b.height), (48, 32, 16, 16));
    assert!(View::Tile {
        tile_x: 4,
        tile_y: 2,
        tile_w: 16,
        tile_h: 16
    }
    .clip(cw, ch)?
    .is_none());
    // Zero-size requests are typed.
    assert_eq!(
        View::Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 4
        }
        .clip(cw, ch)
        .unwrap_err(),
        VoleError::DimensionTooLarge
    );
    assert_eq!(
        View::Tile {
            tile_x: 0,
            tile_y: 0,
            tile_w: 0,
            tile_h: 4
        }
        .clip(cw, ch)
        .unwrap_err(),
        VoleError::DimensionTooLarge
    );
    assert_eq!(
        View::FullFrame.kind(),
        vole_video::view::ViewKind::FullFrame
    );
    assert_eq!(
        View::Rect {
            x: 0,
            y: 0,
            width: 4,
            height: 4
        }
        .kind(),
        vole_video::view::ViewKind::Rect
    );
    assert_eq!(
        View::Tile {
            tile_x: 0,
            tile_y: 0,
            tile_w: 4,
            tile_h: 4
        }
        .kind(),
        vole_video::view::ViewKind::Tile
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Parity courts
// ---------------------------------------------------------------------------

/// For every frame of every authored/random stream: materialize views and
/// assert sample-for-sample equality with the whole-frame crop.
#[test]
fn partial_views_equal_whole_frame_crops_on_all_streams() -> Result<(), VoleError> {
    let mut streams: Vec<(String, Vec<u8>)> = vec![
        ("copy-chain".into(), copy_chain_stream()?),
        ("palette-residual".into(), palette_residual_stream()?),
        ("affine".into(), affine_stream()?),
        ("generator".into(), generator_stream()?),
    ];
    for seed in 0..12u64 {
        streams.push((format!("random-{seed}"), random_movie(seed)?));
    }
    let mut rng = Lcg(0xD1CE);
    for (name, bytes) in &streams {
        let parsed = decoder::decode_bytes(bytes)?;
        let frames = decoder::materialize_all(&parsed)?;
        let (cw, ch) = (parsed.width(), parsed.height());
        assert!(frames.len() >= 3, "{name}: expected several frames");
        for idx in 0..frames.len() as u64 {
            for _ in 0..4 {
                let view = random_view(&mut rng, cw, ch);
                let pv = partial::materialize_view(&parsed, idx, view)?;
                let b = view.clip(cw, ch)?.expect("random view hits");
                assert_view_matches_frame(&pv, &frames[idx as usize], b.x, b.y);
            }
            // Full-frame view equals the canonical whole-frame decode.
            let full_view = partial::materialize_view(&parsed, idx, View::FullFrame)?;
            assert!(
                full_view.canvas.exactly_matches(&frames[idx as usize]),
                "{name}: FullFrame view must equal whole-frame decode at {idx}"
            );
        }
    }
    Ok(())
}

/// Decoder-level API parity: `Decoder::materialize_view` agrees with
/// `Decoder::materialize` on every frame and a spread of views.
#[test]
fn decoder_materialize_view_parity() -> Result<(), VoleError> {
    let bytes = copy_chain_stream()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let (cw, ch) = (parsed.width(), parsed.height());
    let dec = decoder::Decoder::new(parsed);
    for idx in 0..dec.frame_count() {
        let full = dec.materialize(idx)?;
        for view in [
            View::FullFrame,
            View::Rect {
                x: 0,
                y: 0,
                width: cw,
                height: ch,
            },
            View::Rect {
                x: 30,
                y: 20,
                width: 24,
                height: 24,
            },
            View::Tile {
                tile_x: 1,
                tile_y: 1,
                tile_w: 32,
                tile_h: 32,
            },
        ] {
            let pv = dec.materialize_view(idx, view)?;
            let b = view.clip(cw, ch)?.expect("view hits");
            if b.is_full(cw, ch) {
                assert!(
                    pv.canvas.exactly_matches(&full),
                    "full-box view equals Decoder::materialize"
                );
            } else {
                assert_view_matches_frame(&pv, &full, b.x, b.y);
            }
        }
    }
    Ok(())
}

/// A tile grid partitions a frame exactly: every tile of a grid agrees with
/// the whole-frame crop and together the tiles cover the canvas.
#[test]
fn tile_grid_partitions_the_frame_exactly() -> Result<(), VoleError> {
    let bytes = palette_residual_stream()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    let (cw, ch) = (parsed.width(), parsed.height());
    let (tw, th) = (32u32, 32u32);
    for (idx, frame) in frames.iter().enumerate() {
        let mut covered = 0u64;
        let mut tx = 0u32;
        while tx * tw < cw {
            let mut ty = 0u32;
            while ty * th < ch {
                let view = View::Tile {
                    tile_x: tx,
                    tile_y: ty,
                    tile_w: tw,
                    tile_h: th,
                };
                let pv = partial::materialize_view(&parsed, idx as u64, view)?;
                let b = view.clip(cw, ch)?.expect("grid tile hits");
                assert_view_matches_frame(&pv, frame, b.x, b.y);
                covered += u64::from(b.width) * u64::from(b.height);
                ty += 1;
            }
            tx += 1;
        }
        assert_eq!(
            covered,
            u64::from(cw) * u64::from(ch),
            "tiles partition frame {idx}"
        );
    }
    Ok(())
}

/// State-level views (`materialize::materialize` with `View::Rect`/`Tile`)
/// equal the whole-frame state crop (checkpoint frame, no timeline ops).
#[test]
fn state_level_rect_and_tile_views_are_exact_crops() -> Result<(), VoleError> {
    let bytes = affine_stream()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let state = parsed.clone_initial();
    let (cw, ch) = (parsed.width(), parsed.height());
    let limits = *parsed.limits();
    let full = materialize::materialize(&state, View::FullFrame, cw, ch, &limits)?.canvas;
    for view in [
        View::Rect {
            x: 55,
            y: 35,
            width: 42,
            height: 42,
        },
        View::Rect {
            x: -5,
            y: 0,
            width: 20,
            height: 20,
        },
        View::Tile {
            tile_x: 2,
            tile_y: 1,
            tile_w: 64,
            tile_h: 64,
        },
    ] {
        let b = view.clip(cw, ch)?.expect("hits");
        let m = materialize::materialize(&state, view, cw, ch, &limits)?.canvas;
        let expect = crop(&full, b.x, b.y, m.width(), m.height())?;
        assert!(m.exactly_matches(&expect));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Work-reduction courts
// ---------------------------------------------------------------------------

#[test]
fn partial_decode_work_tracks_the_region_of_interest() -> Result<(), VoleError> {
    // 1920×1080, 41 frames, sprite moving 4 px/frame. A 260×140 viewport that
    // tracks the sprite (it always fully covers the sprite box and nothing
    // else). Per level the partial decoder paints the viewport background
    // (260×140) plus the sprite overpaint (200×100) = 56 400 writes, versus a
    // whole-frame level's ≥ 2 073 600 samples.
    let bytes = big_track_stream(40)?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    let (cw, ch) = (1920u32, 1080u32);
    let area = u64::from(cw) * u64::from(ch);
    let per_level = 260u64 * 140 + 200 * 100;
    let mut partial_last = 0u64;
    for (idx, frame) in frames.iter().enumerate() {
        let x0 = 790u32 + (idx as u32) * 4;
        let view = View::Rect {
            x: x0 as i64,
            y: 480,
            width: 260,
            height: 140,
        };
        let pv = partial::materialize_view(&parsed, idx as u64, view)?;
        let b = view.clip(cw, ch)?.expect("in canvas");
        assert_view_matches_frame(&pv, frame, b.x, b.y);
        assert_eq!(pv.stats.frames_replayed, idx as u64 + 1);
        // No interval in this stream carries a canvas op, so only the target
        // level is ever demanded: exactly one level paints, once, whatever
        // the frame index (state replay is transition work, not raster work).
        assert_eq!(pv.stats.levels_materialized, 1);
        assert_eq!(
            pv.stats.painted_samples, per_level,
            "viewport decode paints exactly the viewport-sized lane"
        );
        assert_eq!(pv.stats.objects_touched, 1, "only the tracked sprite");
        assert_eq!(pv.stats.copy_samples_written, 0);
        partial_last = pv.stats.painted_samples;
    }
    // Decoding to the last frame paints one viewport-sized level (partial)
    // versus ≥ one whole canvas per replayed level for whole-frame decode.
    let frames_n = frames.len() as u64;
    assert!(
        partial_last * 100 < frames_n * area * 3,
        "partial {partial_last} must be < 3% of the whole-frame lower bound {}",
        frames_n * area
    );
    Ok(())
}

#[test]
fn copy_chain_region_work_is_bounded_and_exact() -> Result<(), VoleError> {
    let bytes = copy_chain_stream()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    let (cw, ch) = (parsed.width(), parsed.height());
    let full_area = u64::from(cw) * u64::from(ch);
    for (idx, frame) in frames.iter().enumerate() {
        let x0 = (24u32 + idx as u32).min(cw.saturating_sub(20));
        let view = View::Rect {
            x: x0 as i64,
            y: 20,
            width: 20,
            height: 56,
        };
        let pv = partial::materialize_view(&parsed, idx as u64, view)?;
        let b = view.clip(cw, ch)?.expect("hits");
        assert_view_matches_frame(&pv, frame, b.x, b.y);
        assert!(
            pv.stats.painted_samples <= (idx as u64 + 1) * 20 * 56 * 2,
            "copy-chain demand must stay near the region (frame {idx}: {} painted)",
            pv.stats.painted_samples
        );
        assert!(
            pv.stats.painted_samples < (idx as u64 + 1) * full_area / 10,
            "far below whole-frame per level"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Audit-scope and hostile courts
// ---------------------------------------------------------------------------

#[test]
fn out_of_range_index_inside_the_view_errors_like_whole_frame() -> Result<(), VoleError> {
    let bytes = index_trap_stream()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    assert_eq!(
        decoder::materialize_all(&parsed).unwrap_err(),
        VoleError::OutOfBounds
    );
    // A view covering the poisoned pixel (the poisoned box sits at 10,10).
    assert_eq!(
        partial::materialize_view(
            &parsed,
            0,
            View::Rect {
                x: 0,
                y: 0,
                width: 48,
                height: 48
            }
        )
        .unwrap_err(),
        VoleError::OutOfBounds
    );
    Ok(())
}

#[test]
fn out_of_view_poison_is_not_audited_documented_boundary() -> Result<(), VoleError> {
    let bytes = index_trap_stream()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    // A view far from the poisoned instance (at 10,10) samples clean
    // background: the sampling contract — the poison never contributes.
    let pv = partial::materialize_view(
        &parsed,
        0,
        View::Rect {
            x: 70,
            y: 70,
            width: 10,
            height: 10,
        },
    )?;
    assert_eq!((pv.canvas.width(), pv.canvas.height()), (10, 10));
    for y in 0..10 {
        for x in 0..10 {
            assert_eq!(pv.canvas.get(x, y), 0, "background only");
        }
    }
    Ok(())
}

#[test]
fn unsorted_residual_inside_the_timeline_errors_like_whole_frame() -> Result<(), VoleError> {
    let mut a = Ingest::new(32, 32);
    a.background(0);
    a.declare_fill(1, 32, 32, 0)?;
    a.instance(1, 1, 0, 0)?;
    a.at(1)?;
    a.push(Transition::Residual {
        block: raw_block(&[(5, 5, 9), (2, 2, 8)]),
    })?;
    let bytes = a.finish()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    assert_eq!(
        decoder::materialize_all(&parsed).unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Any view of frame 1 must surface the residual error (the container is
    // fully validated even in partial decode).
    assert_eq!(
        partial::materialize_view(
            &parsed,
            1,
            View::Rect {
                x: 0,
                y: 0,
                width: 32,
                height: 32
            }
        )
        .unwrap_err(),
        VoleError::NonCanonicalEncoding
    );
    // Frame 0 (never carrying the residual) is fine either way.
    let _ = partial::materialize_view(&parsed, 0, View::FullFrame)?;
    Ok(())
}

#[test]
fn out_of_range_views_and_indexes_are_typed() -> Result<(), VoleError> {
    let bytes = copy_chain_stream()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = parsed.frame_count();
    assert_eq!(
        partial::materialize_view(&parsed, frames, View::FullFrame).unwrap_err(),
        VoleError::OutOfBounds
    );
    assert_eq!(
        partial::materialize_view(
            &parsed,
            0,
            View::Rect {
                x: 10_000,
                y: 0,
                width: 8,
                height: 8
            }
        )
        .unwrap_err(),
        VoleError::ApiConstraint("view does not intersect the canvas")
    );
    assert_eq!(
        partial::materialize_view(
            &parsed,
            0,
            View::Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 8
            }
        )
        .unwrap_err(),
        VoleError::DimensionTooLarge
    );
    Ok(())
}

/// Measured stats on a residual-bearing frame: residual writes are counted
/// only where they fall inside the region, and totals equal their subcounts.
#[test]
fn partial_stats_are_internally_consistent() -> Result<(), VoleError> {
    let bytes = palette_residual_stream()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    let frame = &frames[9];
    // A near-full view (not the exact full box, so the partial path runs):
    // frame 9's residual points are (2,2), (10,40), (90,5).
    let pv = partial::materialize_view(
        &parsed,
        9,
        View::Rect {
            x: 0,
            y: 0,
            width: 95,
            height: 95,
        },
    )?;
    let expect = crop(frame, 0, 0, 95, 95)?;
    assert!(pv.canvas.exactly_matches(&expect));
    assert_eq!(pv.stats.residual_samples_written, 3);
    assert_eq!(
        pv.stats.painted_samples,
        pv.stats.base_samples_written
            + pv.stats.copy_samples_written
            + pv.stats.residual_samples_written
    );
    assert_eq!(pv.stats.frames_replayed, 10);
    assert!(pv.stats.levels_materialized >= 1);
    assert!(pv.stats.peak_view_samples <= 95u64 * 95);
    // A tiny view that contains no residual point touches none.
    let pv2 = partial::materialize_view(
        &parsed,
        9,
        View::Rect {
            x: 50,
            y: 50,
            width: 8,
            height: 8,
        },
    )?;
    assert_eq!(pv2.stats.residual_samples_written, 0);
    Ok(())
}

/// Demand-plan bookkeeping stays bounded on a hostile arrangement: many tiny
/// copies with chained destinations cannot blow the planner up (the span
/// budget saturates to the full canvas; exactness is preserved).
#[test]
fn pathological_copy_overlap_stays_exact_and_bounded() -> Result<(), VoleError> {
    let (w, h) = (64u32, 64u32);
    let mut a = Ingest::new(w, h);
    a.background(3);
    a.declare_raster(1, 8, 8, texture(9, 8, 8))?;
    a.instance(1, 1, 20, 20)?;
    for t in 1..=40u64 {
        a.at(t)?;
        for k in 0..6u64 {
            let s = (t * 3 + k) % 50;
            a.push(Transition::CopyRect {
                src_x: s as i64,
                src_y: (k * 5) as i64,
                width: 4,
                height: 4,
                dst_x: (s + 1) as i64,
                dst_y: (k * 5 + 1) as i64,
            })?;
        }
    }
    let bytes = a.finish()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let frames = decoder::materialize_all(&parsed)?;
    let mut rng = Lcg(77);
    for idx in 0..frames.len() as u64 {
        for _ in 0..3 {
            let view = random_view(&mut rng, w, h);
            let pv = partial::materialize_view(&parsed, idx, view)?;
            let b = view.clip(w, h)?.expect("hits");
            assert_view_matches_frame(&pv, &frames[idx as usize], b.x, b.y);
        }
    }
    Ok(())
}

/// Decode of a whole-frame-box view routes through the canonical step
/// machinery (identical bytes), including content with canvas ops.
#[test]
fn full_canvas_box_view_equals_whole_frame_decode_with_ops() -> Result<(), VoleError> {
    let bytes = frames_of(&copy_chain_stream()?)?;
    assert!(!bytes.is_empty());
    // Re-decode via partial FullFrame on the copy chain: exact identity.
    let src = copy_chain_stream()?;
    let parsed = decoder::decode_bytes(&src)?;
    let frames = decoder::materialize_all(&parsed)?;
    let pv = partial::materialize_view(&parsed, 7, View::FullFrame)?;
    assert!(pv.canvas.exactly_matches(&frames[7]));
    Ok(())
}
