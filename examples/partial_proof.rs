//! Phase S evidence proof: partial materialization (§16/§37/§66).
//!
//! A 1920×1080 (Gray8, 41-frame) sprite-track stream is decoded three ways:
//!
//! * **whole-frame**: canonical sequential decode of every frame
//!   (`decoder::materialize_all`);
//! * **random access whole**: `Decoder::materialize(idx)` (replays the whole
//!   timeline, whole canvas);
//! * **random access partial**: `Decoder::materialize_view(idx, viewport)` —
//!   demand-planned partial decode that paints only the samples the viewport
//!   needs (and exactly the COPY_RECT history it reads).
//!
//! The governing property is asserted on every court: a partial view equals
//! the whole-frame crop sample-for-sample. Measured: painted sample writes,
//! peak per-level raster memory, distinct objects touched (the decode-time
//! analogue of "object fetches"), and wall latency.
//!
//! Courts: viewport-tracking decode (1/36th of the canvas), a COPY_RECT
//! scroll chain with cross-frame demand, a tile-grid partition of a frame,
//! and deep random access to the last frame. Synthetic authored content only:
//! the numbers measure decode work, never a claim about natural video.
//!
//! Run: `cargo run --release --example partial_proof`

use std::time::Instant;

use vole_video::{
    decoder, ingest::Ingest, partial, pixel::Canvas, transition::Transition, view::View, VoleError,
};

fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

fn crop(frame: &Canvas, x0: u32, y0: u32, w: u32, h: u32) -> Result<Canvas, VoleError> {
    let mut data = Vec::with_capacity((w as usize) * (h as usize));
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            data.push(frame.get(x, y));
        }
    }
    Canvas::from_parts(w, h, data)
}

fn texture(seed: u64, w: u32, h: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity((w * h) as usize);
    for _ in 0..w * h {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push((x % 256) as u8);
    }
    out
}

/// 1920×1080 × 41 frames: a huge static decoration plus a 200×100 sprite
/// gliding right at 4 px/frame (§76-style authored motion on the max canvas).
fn big_track() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (1920u32, 1080u32);
    let mut a = Ingest::new(w, h);
    a.background(5);
    a.declare_fill(1, 600, 900, 12)?;
    a.instance(1, 1, 100, 100)?;
    a.declare_raster(2, 200, 100, texture(11, 200, 100))?;
    a.instance(2, 2, 800, 500)?;
    for t in 1..=40u64 {
        a.at(t)?;
        a.set_position(2, 800 + 4 * t as i64, 500)?;
    }
    a.finish()
}

/// Scroll chain: a vertical band panned right one pixel per interval by
/// COPY_RECT (each level reads the previous frame) with a moving sprite.
fn scroll_chain() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (160u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(17);
    a.declare_raster(1, 12, 12, texture(3, 12, 12))?;
    a.instance(1, 1, 40, 40)?;
    for t in 1..=12u64 {
        a.at(t)?;
        a.set_position(1, 40 + 3 * t as i64, 40)?;
        a.push(Transition::CopyRect {
            src_x: 20 + t as i64,
            src_y: 0,
            width: 24,
            height: h,
            dst_x: 21 + t as i64,
            dst_y: 0,
        })?;
    }
    a.finish()
}

/// Palette + index + sparse residual content (other procedural families).
fn palette_residual() -> Result<Vec<u8>, VoleError> {
    let (w, h) = (96u32, 96u32);
    let mut a = Ingest::new(w, h);
    a.background(9);
    a.declare_palette(1, vec![10, 60, 120, 200, 250])?;
    let idx: Vec<u8> = (0..(48 * 48))
        .map(|i| (((i / 48) * 3 + (i % 48) * 5) % 5) as u8)
        .collect();
    a.declare_index(1, 48, 48, idx)?;
    a.instance_binding(1, 1, 10, 10, 1)?;
    for t in 1..=9u64 {
        a.at(t)?;
        if t % 3 == 0 {
            let changes: Vec<(u8, u8)> = (0..5)
                .map(|i| (i, (10 + t * 37 + u64::from(i) * 3) as u8))
                .collect();
            a.patch_palette(1, changes)?;
        }
    }
    a.finish()
}

fn main() -> Result<(), VoleError> {
    // ------------------------------------------------------------------
    // Court 1 — viewport tracking on 1920×1080 (41 frames).
    // ------------------------------------------------------------------
    let bytes = big_track()?;
    let t = Instant::now();
    let full_frames = frames_of(&bytes)?;
    let full_seq_ms = t.elapsed().as_secs_f64() * 1e3;
    let n = full_frames.len() as u64;
    let area = 1920u64 * 1080;
    println!("court big-track 1920x1080 x{n}:");
    println!(
        "  whole-frame sequential decode: {full_seq_ms:.1} ms (writes >= {} samples = {} MiB raster)",
        n * area,
        n * area / (1024 * 1024)
    );

    // Random access: last frame, whole-frame API vs partial viewport API.
    let dec = decoder::Decoder::new(decoder::decode_bytes(&bytes)?);
    let t = Instant::now();
    let _whole40 = dec.materialize(40)?;
    let whole_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let vp = dec.materialize_view(
        40,
        View::Rect {
            x: 950,
            y: 480,
            width: 260,
            height: 140,
        },
    )?;
    let partial_ms = t.elapsed().as_secs_f64() * 1e3;
    assert!(vp
        .canvas
        .exactly_matches(&crop(&full_frames[40], 950, 480, 260, 140)?));
    let st = &vp.stats;
    println!(
        "  random access frame 40: whole {whole_ms:.1} ms; viewport (260x140) {partial_ms:.3} ms ({:.0}x)",
        whole_ms / partial_ms.max(1e-9)
    );
    println!(
        "    viewport painted {} samples (whole-frame lower bound {}), peak raster {} samples (of {})",
        st.painted_samples,
        n * area,
        st.peak_view_samples,
        area
    );
    println!(
        "    objects touched {} (the tracked sprite only; a 600x900 decoration never intersects the viewport)",
        st.objects_touched
    );
    assert_eq!(st.objects_touched, 1);
    assert_eq!(st.painted_samples, 260 * 140 + 200 * 100);
    assert!(st.painted_samples * 100 < n * area * 3);

    // Track the viewport across every frame (a viewport-playback client).
    let t = Instant::now();
    let mut painted_total = 0u64;
    for idx in 0..n {
        let pv = dec.materialize_view(
            idx,
            View::Rect {
                x: (790 + 4 * idx) as i64,
                y: 480,
                width: 260,
                height: 140,
            },
        )?;
        let x0 = 790u32 + 4 * idx as u32;
        assert!(pv
            .canvas
            .exactly_matches(&crop(&full_frames[idx as usize], x0, 480, 260, 140)?));
        painted_total += pv.stats.painted_samples;
    }
    let track_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "  all-41 viewport frames: {track_ms:.1} ms total, {painted_total} samples painted ({:.3}% of the {}-sample whole-frame lane)",
        painted_total as f64 * 100.0 / (n * area) as f64,
        n * area
    );

    // ------------------------------------------------------------------
    // Court 2 — COPY_RECT scroll chain with cross-frame demand.
    // ------------------------------------------------------------------
    let bytes = scroll_chain()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let full_frames = frames_of(&bytes)?;
    let mut chain_painted = 0u64;
    for idx in 0..full_frames.len() as u64 {
        let x0 = (24u32 + idx as u32).min(140);
        let pv = partial::materialize_view(
            &parsed,
            idx,
            View::Rect {
                x: x0 as i64,
                y: 20,
                width: 20,
                height: 56,
            },
        )?;
        let expect = crop(&full_frames[idx as usize], x0, 20, 20, 56)?;
        assert!(
            pv.canvas.exactly_matches(&expect),
            "scroll-chain view at idx {idx} x0 {x0} must equal the whole-frame crop"
        );
        chain_painted += pv.stats.painted_samples;
    }
    println!();
    println!("court scroll-chain (13 frames, copy rects every interval):");
    println!(
        "  viewport lane painted {chain_painted} samples total; every view equals the whole-frame crop byte-for-byte"
    );
    assert!(chain_painted < 13 * 160 * 96);

    // ------------------------------------------------------------------
    // Court 3 — tile-grid partition of a palette/index frame.
    // ------------------------------------------------------------------
    let bytes = palette_residual()?;
    let parsed = decoder::decode_bytes(&bytes)?;
    let full_frames = frames_of(&bytes)?;
    let (cw, ch) = (parsed.width(), parsed.height());
    let (tw, th) = (24u32, 24u32);
    let t = Instant::now();
    let mut covered = 0u64;
    let mut tiles = 0u64;
    let mut tx = 0u32;
    while tx * tw < cw {
        let mut ty = 0u32;
        while ty * th < ch {
            let pv = partial::materialize_view(
                &parsed,
                8,
                View::Tile {
                    tile_x: tx,
                    tile_y: ty,
                    tile_w: tw,
                    tile_h: th,
                },
            )?;
            let b = View::Tile {
                tile_x: tx,
                tile_y: ty,
                tile_w: tw,
                tile_h: th,
            }
            .clip(cw, ch)?
            .expect("hits");
            assert!(pv.canvas.exactly_matches(&crop(
                &full_frames[8],
                b.x,
                b.y,
                b.width,
                b.height
            )?));
            covered += u64::from(b.width) * u64::from(b.height);
            tiles += 1;
            ty += 1;
        }
        tx += 1;
    }
    let tiles_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(covered, u64::from(cw) * u64::from(ch));
    println!();
    println!("court tile grid {tw}x{th} of frame 8 ({cw}x{ch}):");
    println!(
        "  {tiles} tiles partition the frame exactly; {tiles_ms:.1} ms (each tile is a demand-planned partial decode)"
    );

    // ------------------------------------------------------------------
    // Court 4 — peak working raster memory across views.
    // ------------------------------------------------------------------
    let mut worst = 0u64;
    for idx in 0..n {
        let pv = dec.materialize_view(
            idx,
            View::Rect {
                x: (790 + 4 * idx) as i64,
                y: 480,
                width: 260,
                height: 140,
            },
        )?;
        worst = worst.max(pv.stats.peak_view_samples);
    }
    println!();
    println!(
        "peak working raster of a viewport decode: {worst} samples ({:.3}% of the {} sample full frame)",
        worst as f64 * 100.0 / area as f64,
        area
    );

    println!();
    println!("partial proof: OK");
    Ok(())
}
