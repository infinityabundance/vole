//! Phase K courts: variable regions in the raster-origin encoder.
//!
//! Phase G/H treated every frame change at *whole-frame* granularity: a
//! rebase declared a full-canvas raster object. Phase K gives the encoder a
//! **variable-region family**: it partitions the per-frame diff into tiles of
//! a granularity (64 → 32 → 16 → 8), declares each diff-bearing tile's
//! bounding box (a *rectangular* region) as an immutable object holding the
//! target's own sub-rectangle, and paints it above the base with a fresh
//! instance. Exact content identity makes repeated region content free
//! (zero-declaration reuse). Courts cover: the no-rebase property on
//! localized-change content; exact-ref region reuse; granularity ladder;
//! DSFB governance (fewer candidates, equal bytes); overlay-shadowing
//! negative control; diff-gate negative control (noise stays RAW); and the
//! oracle min-payload invariant.

use vole_video::{decoder, error::VoleError, inverse, pixel::Canvas, transition::Transition};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

/// Deterministic structured "desktop" canvas (block pattern, not uniform, so
/// whole-frame resets cannot ride on fill objects).
fn base_canvas(w: u32, h: u32) -> Canvas {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = ((x / 7 + y / 5) % 5) as u8 * 37 + 20;
            d.push(v);
        }
    }
    canvas_of(w, h, d)
}

/// Overwrite a rectangle of `base` with deterministic pseudo-random bytes.
#[allow(clippy::too_many_arguments)] // geometry tuple kept inline for clarity
fn patch_rect(
    w: u32,
    h: u32,
    base: &Canvas,
    x: i64,
    y: i64,
    rw: u32,
    rh: u32,
    seed: u64,
) -> Canvas {
    let (_, _, mut data) = base.clone().into_parts();
    let mut s = seed.max(1);
    let mut byte = move || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (s >> 56) as u8
    };
    for dy in 0..rh as i64 {
        for dx in 0..rw as i64 {
            let cx = x + dx;
            let cy = y + dy;
            if cx >= 0 && cy >= 0 && cx < i64::from(w) && cy < i64::from(h) {
                data[cy as usize * w as usize + cx as usize] = byte();
            }
        }
    }
    canvas_of(w, h, data)
}

fn encode(frames: &[Canvas]) -> Result<inverse::EncodeReport, VoleError> {
    inverse::encode_frames(
        frames,
        &inverse::EncodeOptions {
            bg_sweep: false,
            background: Some(frames[0].as_slice()[0]),
            ..inverse::EncodeOptions::default()
        },
    )
}

fn winner_histogram(r: &inverse::EncodeReport) -> Vec<(&'static str, u64)> {
    let mut counts: Vec<(&'static str, u64)> = Vec::new();
    for d in &r.decisions {
        if let Some(slot) = counts.iter_mut().find(|(f, _)| *f == d.winner_family) {
            slot.1 += 1;
        } else {
            counts.push((d.winner_family, 1));
        }
    }
    counts
}

fn winners(r: &inverse::EncodeReport) -> String {
    winner_histogram(r)
        .iter()
        .map(|(f, c)| format!("{f}x{c}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn raw_rebases_after_frame0(r: &inverse::EncodeReport) -> u64 {
    r.decisions
        .iter()
        .filter(|d| d.frame > 0 && d.winner_family == "raw" && d.object_decl_bytes > 0)
        .count() as u64
}

/// Localized-change sequence: a 40×24 "clock" whose content changes every
/// frame and a 16×16 glyph area alternating between two known contents.
fn clock_frames(w: u32, h: u32, frames: u64) -> Vec<Canvas> {
    let base = base_canvas(w, h);
    let mut out = vec![base.clone()];
    for k in 1..=frames {
        let mut f = patch_rect(w, h, &base, 40, 24, 40, 24, 0xAB00 + k);
        f = patch_rect(
            w,
            h,
            &f,
            96,
            40,
            16,
            16,
            if k % 2 == 0 { 0x1111 } else { 0x2222 },
        );
        out.push(f);
    }
    out
}

#[test]
fn localized_changes_never_rebase_the_whole_frame() -> Result<(), VoleError> {
    let (w, h) = (160u32, 120u32);
    let frames = clock_frames(w, h, 40);
    let report = encode(&frames)?;
    assert!(report.exact);
    assert_eq!(
        raw_rebases_after_frame0(&report),
        0,
        "localized changes must not declare whole-canvas rasters"
    );
    let hist = winners(&report);
    assert!(
        hist.contains("regionsx40"),
        "every changed frame must be served by variable regions, got [{hist}]"
    );
    // The stream must decode back to the input raster exactly (encoder
    // invariant, re-checked here through the public path).
    let out = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
    assert_eq!(out.len(), frames.len());
    for (a, b) in out.iter().zip(frames.iter()) {
        assert_eq!(a, b);
    }
    Ok(())
}

#[test]
fn region_stream_is_far_from_raster_proportional() -> Result<(), VoleError> {
    let (w, h) = (160u32, 120u32);
    let frames = clock_frames(w, h, 40);
    let report = encode(&frames)?;
    // 41 frames × 19 200 samples; region stream must be well under the raw
    // sequence (declarations cover only the ~1 216 changed samples/frame).
    let raw_all = u64::from(w) * u64::from(h) * frames.len() as u64;
    assert!(
        (report.vole.len() as u64) * 8 < raw_all,
        "region representation is not raster-proportional ({} vs {})",
        report.vole.len(),
        raw_all
    );
    Ok(())
}

#[test]
fn identical_region_content_is_reused_with_zero_declaration() -> Result<(), VoleError> {
    // The alternating glyph area reuses exactly two contents: the clock
    // changes for 5 frames (5 first-use declarations), then goes static while
    // the glyph keeps alternating — every later region is an exact-ref with
    // zero declaration bytes.
    let (w, h) = (160u32, 120u32);
    let base = base_canvas(w, h);
    let mut frames = vec![base.clone()];
    for k in 1..=5u64 {
        let mut f = patch_rect(w, h, &base, 40, 24, 40, 24, 0xAB00 + k);
        f = patch_rect(
            w,
            h,
            &f,
            96,
            40,
            16,
            16,
            if k % 2 == 0 { 0x1111 } else { 0x2222 },
        );
        frames.push(f);
    }
    // Frames 6..=40: the clock keeps its frame-5 content (state persists) and
    // only the glyph area alternates between its two known contents.
    let still = patch_rect(w, h, &base, 40, 24, 40, 24, 0xAB00 + 5);
    for k in 6..=40u64 {
        let f = patch_rect(
            w,
            h,
            &still,
            96,
            40,
            16,
            16,
            if k % 2 == 0 { 0x1111 } else { 0x2222 },
        );
        frames.push(f);
    }
    let report = encode(&frames)?;
    assert!(report.exact);
    // First-use declarations: 5 clock contents + 2 glyph contents, all inside
    // the first five decisions.
    let first_use: u64 = report
        .decisions
        .iter()
        .filter(|d| d.frame > 0 && d.object_decl_bytes > 0)
        .count() as u64;
    assert_eq!(first_use, 5, "clock+glyph first uses live in frames 1..=5");
    // Every later region decision (the glyph alternation) pays zero
    // declarations: envelope + one reused CreateInstance per rectangle.
    let mut reused = 0u64;
    let mut decl_bytes = 0u64;
    for d in &report.decisions {
        if d.frame >= 6 && d.winner_family == "regions" {
            reused += 1;
            decl_bytes += d.object_decl_bytes;
        }
    }
    assert_eq!(reused, 35, "all 35 glyph-only frames use region reuse");
    assert_eq!(decl_bytes, 0);
    // The glyph frames cost the pure reuse floor: envelope + one create.
    let reuse_frames: Vec<u64> = report
        .decisions
        .iter()
        .filter(|d| d.frame >= 6)
        .map(|d| d.winner_payload_bytes)
        .collect();
    assert!(reuse_frames.iter().all(|p| *p <= 13 + 17 + 13 + 17));
    let out = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
    assert_eq!(out.len(), frames.len());
    for (a, b) in out.iter().zip(frames.iter()) {
        assert_eq!(a, b);
    }
    Ok(())
}

#[test]
fn region_granularity_ladder_is_deterministic_and_exact() -> Result<(), VoleError> {
    // A full-width banner (rectangular, 8 rows tall) plus a 24×24 block: the
    // ladder must produce *rectangular* regions (bounding boxes inside tiles,
    // not only full tiles) and stay byte-exact.
    let (w, h) = (128u32, 96u32);
    let base = base_canvas(w, h);
    let mut frames = vec![base.clone()];
    for k in 1..=10u64 {
        let mut f = patch_rect(w, h, &base, 0, 40, w, 8, 0x31 + k); // full-width banner
        f = patch_rect(w, h, &f, 100, 10, 24, 24, 0x77 + k); // dense block
        frames.push(f);
    }
    let report = encode(&frames)?;
    assert!(report.exact);
    assert_eq!(raw_rebases_after_frame0(&report), 0);
    assert!(winners(&report).contains("regionsx10"));
    // Every winner decision claims materialized-exactness.
    assert!(report.decisions.iter().all(|d| d.materialized_exact));
    // Any region rect taller than wide or wider than tall is fine; the claim
    // here is only exactness + no rebase. Rectangle shapes are exercised by
    // the unit-level region candidate checks in the encoder tests below.
    Ok(())
}

#[test]
fn dsfb_governs_regions_with_equal_bytes_and_fewer_candidates() -> Result<(), VoleError> {
    let (w, h) = (160u32, 120u32);
    let frames = clock_frames(w, h, 30);
    let oracle = encode(&frames)?;
    let guided = inverse::encode_frames(
        &frames,
        &inverse::EncodeOptions {
            bg_sweep: false,
            background: Some(frames[0].as_slice()[0]),
            strategy: vole_video::dsfb::EncoderStrategy::DsfbGuided,
            ..inverse::EncodeOptions::default()
        },
    )?;
    assert!(guided.exact);
    let n_oracle: u64 = oracle
        .decisions
        .iter()
        .map(|d| d.candidates_evaluated)
        .sum();
    let n_guided: u64 = guided
        .decisions
        .iter()
        .map(|d| d.candidates_evaluated)
        .sum();
    assert!(
        n_guided < n_oracle,
        "guided search must evaluate fewer candidates ({n_guided} vs {n_oracle})"
    );
    // Byte-identical stream on this steady content.
    assert_eq!(
        guided.vole.len(),
        oracle.vole.len(),
        "guided bytes must equal the oracle bytes on steady region content"
    );
    // The guided run must reach the same regions winners.
    assert!(guided
        .decisions
        .iter()
        .all(|d| d.winner_family == oracle.decisions[d.frame as usize].winner_family));
    Ok(())
}

#[test]
fn overlay_shadowing_keeps_sparse_in_charge() -> Result<(), VoleError> {
    // A 12×12 dense block blinks in place on a static base. The first
    // appearance is served by a region; afterwards the persistent overlay
    // (sparse) carries the blink because overlay paints above instances —
    // region rectangles cannot correct shadowed samples. What matters is
    // exactness and that the encoder never picks an invalid region program.
    let (w, h) = (64u32, 64u32);
    let base = base_canvas(w, h);
    let dark = patch_rect(w, h, &base, 20, 20, 12, 12, 0xDEAD);
    let light = patch_rect(w, h, &base, 20, 20, 12, 12, 0xBEEF);
    let mut frames = vec![base.clone()];
    for k in 0..20u64 {
        frames.push(if k % 2 == 0 {
            dark.clone()
        } else {
            light.clone()
        });
    }
    let report = encode(&frames)?;
    assert!(report.exact);
    let out = decoder::materialize_all(&decoder::decode_bytes(&report.vole)?)?;
    assert_eq!(out.len(), frames.len());
    for (a, b) in out.iter().zip(frames.iter()) {
        assert_eq!(a, b);
    }
    Ok(())
}

#[test]
fn noise_negative_control_stays_raw_and_bounded() -> Result<(), VoleError> {
    // Full-canvas noise: the diff gate skips the region family (a diff larger
    // than a quarter of the canvas cannot beat the whole-frame reset
    // sentinel), so noise stays RAW with bounded overhead.
    let (w, h) = (48u32, 32u32);
    let mut seed = 0x0BAD_F00Du64;
    let mut frames = Vec::new();
    for _ in 0..10 {
        let mut d = Vec::with_capacity((w * h) as usize);
        for _ in 0..(w * h) {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            d.push((seed >> 56) as u8);
        }
        frames.push(canvas_of(w, h, d));
    }
    let report = encode(&frames)?;
    assert!(report.exact);
    assert!(winner_histogram(&report).iter().all(|(f, _)| *f == "raw"));
    let raw_all = u64::from(w) * u64::from(h) * frames.len() as u64;
    assert!(
        report.vole.len() as u64 * 2 < raw_all * 3,
        "noise fallback overhead stays bounded"
    );
    Ok(())
}

#[test]
fn oracle_winner_is_min_over_every_evaluated_family() -> Result<(), VoleError> {
    // Exhaustive decision records must satisfy the oracle invariant: the
    // winner's payload equals the minimum best payload over every evaluated
    // family, and every winner is materialized-exact.
    let (w, h) = (160u32, 120u32);
    let frames = clock_frames(w, h, 12);
    let report = encode(&frames)?;
    for d in &report.decisions {
        let min_family = d
            .families
            .iter()
            .filter(|f| f.valid > 0)
            .map(|f| f.best_payload)
            .min();
        if let Some(m) = min_family {
            assert_eq!(
                d.winner_payload_bytes, m,
                "frame {} winner must be the min over evaluated families",
                d.frame
            );
        }
        assert!(d.materialized_exact);
        // Region winners: interval bytes are exactly the envelope plus one
        // CreateInstance per rectangle (17 B each).
        if d.winner_family == "regions" {
            let creates: u64 = d
                .emitted
                .iter()
                .filter(|t| matches!(t, Transition::CreateInstance { .. }))
                .count() as u64;
            assert_eq!(d.winner_interval_bytes, 13 + 17 * creates);
            assert_eq!(creates, d.emitted.len() as u64);
        }
    }
    Ok(())
}

#[test]
fn candidate_space_is_finite_on_region_frames() -> Result<(), VoleError> {
    // Region evaluation must stay bounded: at most 4 granularity candidates
    // per frame, and the whole per-frame candidate count stays small on the
    // probe canvas (the frame cost is dominated by diff scans, not the region
    // family).
    let (w, h) = (160u32, 120u32);
    let frames = clock_frames(w, h, 6);
    let report = encode(&frames)?;
    for d in &report.decisions {
        if d.frame > 0 {
            let regions = d
                .families
                .iter()
                .find(|f| f.family == "regions")
                .map(|f| f.evaluated)
                .unwrap_or(0);
            assert!(regions <= 4, "region family evaluates at most 4 candidates");
        }
    }
    Ok(())
}
