//! Phase Q evidence proof: the §55 native-procedural preservation court.
//!
//! The *same* authored procedural state is carried two ways:
//!
//! * **A — native ingest** (`ingest` / `script`): the procedural state is
//!   emitted directly (palette state, generator programs, trajectory
//!   programs, affine placements, objects, transitions);
//! * **B — raster-origin** (`inverse::encode_frames`): the canonical raster
//!   sequence of A is materialized, then re-proceduralized by the exhaustive
//!   inverse encoder, which must infer structure from flattened pixels.
//!
//! Both A and B must reproduce the same canonical raster sequence
//! byte-for-byte (verified by decoding each stream). The **flattening tax**
//! `B/A` (total and per-interval marginal) is measured, never assumed — the
//! cost of flattening structured media into rasters before storage.
//!
//! Run: `cargo run --release --example ingest_proof`

use std::time::Instant;

use vole_video::{
    decoder,
    ingest::Ingest,
    inverse::{self, EncodeOptions},
    pixel::Canvas,
    script,
    trajectory::TrajectorySegment,
    VoleError,
};

fn frames_of(bytes: &[u8]) -> Result<Vec<Canvas>, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    decoder::materialize_all(&parsed)
}

fn inverse_leg(frames: &[Canvas], bg: u8) -> Result<inverse::EncodeReport, VoleError> {
    let opts = EncodeOptions {
        bg_sweep: false,
        background: Some(bg),
        ..EncodeOptions::default()
    };
    inverse::encode_frames(frames, &opts)
}

/// Marginal interval bytes of an ingest stream: total minus the frame-0-only
/// stream over the same declarations.
fn ingest_interval_bytes(full: &[u8], decls_only: &[u8]) -> u64 {
    (full.len() - decls_only.len()) as u64
}

fn frames_raw(frames: &[Canvas]) -> u64 {
    frames.iter().map(|f| f.sample_count()).sum()
}

/// One flattening-court row: content label + leg A/B byte measures.
struct Row {
    content: &'static str,
    a_total: u64,
    b_total: u64,
    a_interval: u64,
    b_interval: u64,
    frames: usize,
    raw: u64,
    exact: bool,
}

/// Encode leg A (direct ingest), materialize the canonical raster sequence,
/// encode leg B (raster-origin inverse), and report the flattening court row.
fn court(
    rows: &mut Vec<Row>,
    content: &'static str,
    a_bytes: &[u8],
    a0_bytes: &[u8],
    bg: u8,
    label: &str,
) -> Result<(), VoleError> {
    let fa = frames_of(a_bytes)?;
    let a_interval = ingest_interval_bytes(a_bytes, a0_bytes);
    let t = Instant::now();
    let b = inverse_leg(&fa, bg)?;
    let b_ms = t.elapsed().as_millis();
    assert!(
        b.exact,
        "inverse leg verified byte-exact by its own contract"
    );
    let fb = frames_of(&b.vole)?;
    let exact = fb.len() == fa.len() && fa.iter().zip(&fb).all(|(x, y)| x.exactly_matches(y));
    let b_interval: u64 = b.decisions[1..]
        .iter()
        .map(|d| d.winner_payload_bytes)
        .sum();
    let b_winners: Vec<&str> = b.decisions.iter().map(|d| d.winner_family).collect();
    println!(
        "{label}: A={}B (interval {a_interval}B) B={}B (interval {b_interval}B, {b_ms}ms) exact={exact} winners={b_winners:?}",
        a_bytes.len(),
        b.vole.len(),
    );
    rows.push(Row {
        content,
        a_total: a_bytes.len() as u64,
        b_total: b.vole.len() as u64,
        a_interval,
        b_interval,
        frames: fa.len(),
        raw: frames_raw(&fa),
        exact,
    });
    Ok(())
}

fn main() -> Result<(), VoleError> {
    let mut rows: Vec<Row> = Vec::new();

    // ------------------------------------------------------------------
    // Court 1 — palette rotation over a full-canvas index plane (96x96, 13
    // frames). Every pixel's color changes each interval (entries rotate);
    // A stores one 8-byte palette-table replacement per interval.
    // ------------------------------------------------------------------
    {
        let (w, h) = (96u32, 96u32);
        let bg = 40u8;
        let base = [10u8, 40, 90, 150, 220, 30, 60, 200];
        // XOR class lattice: rotating the palette recolorizes every pixel with
        // no spatial-translation symmetry, so no raster family can abstract it.
        let indices: Vec<u8> = (0..(w * h))
            .map(|i| (((i / w * 7) ^ (i % w * 13)) % 8) as u8)
            .collect();
        let mut a = Ingest::new(w, h);
        a.background(bg);
        a.declare_palette(1, base.to_vec())?;
        a.declare_index(1, w, h, indices.clone())?;
        a.instance_binding(1, 1, 0, 0, 1)?;
        for k in 1..=12u64 {
            a.at(k)?;
            let shift = (k % 8) as usize;
            let entries: Vec<u8> = (0..8).map(|i| base[(i + shift) % 8]).collect();
            a.set_palette(1, entries)?;
        }
        let mut a0 = Ingest::new(w, h);
        a0.background(bg);
        a0.declare_palette(1, base.to_vec())?;
        a0.declare_index(1, w, h, indices)?;
        a0.instance_binding(1, 1, 0, 0, 1)?;
        let a_bytes = a.finish()?;
        let a0_bytes = a0.finish()?;
        let fa = frames_of(&a_bytes)?;
        assert_eq!(fa.len(), 13);
        for k in 0..7 {
            assert_ne!(
                fa[k].get(0, 0),
                fa[k + 1].get(0, 0),
                "palette rotation recolors every pixel each interval"
            );
        }
        court(
            &mut rows,
            "palette rotation, every pixel changes",
            &a_bytes,
            &a0_bytes,
            bg,
            "palette-rotate-96x96",
        )?;
    }

    // ------------------------------------------------------------------
    // Court 2 — accent-strip palette toggle (96x96, 13 frames): the visual
    // change is a uniform 96x8 strip alternating between two colors, which the
    // raster encoder serves with reusable region objects.
    // ------------------------------------------------------------------
    {
        let (w, h) = (96u32, 96u32);
        let bg = 40u8;
        let mut indices = vec![0u8; (8 * w) as usize];
        indices.iter_mut().for_each(|v| *v = 1);
        let mut a = Ingest::new(w, h);
        a.background(bg);
        a.declare_palette(1, vec![200, 60, 90, 150, 220])?;
        a.declare_index(1, w, 8, indices.clone())?;
        a.instance_binding(1, 1, 0, 60, 1)?;
        for k in 1..=12u64 {
            a.at(k)?;
            let v = if k % 2 == 1 { 200 } else { 60 };
            a.patch_palette(1, vec![(1, v)])?;
        }
        let mut a0 = Ingest::new(w, h);
        a0.background(bg);
        a0.declare_palette(1, vec![200, 60, 90, 150, 220])?;
        a0.declare_index(1, w, 8, indices)?;
        a0.instance_binding(1, 1, 0, 60, 1)?;
        let a_bytes = a.finish()?;
        let a0_bytes = a0.finish()?;
        let fa = frames_of(&a_bytes)?;
        assert_eq!(fa.len(), 13);
        assert_eq!(fa[0].get(0, 60), 60, "strip renders palette entry 1 = 60");
        assert_eq!(fa[1].get(0, 60), 200, "interval 1 patches entry 1 to 200");
        assert_eq!(fa[2].get(0, 60), 60, "interval 2 patches it back");
        court(
            &mut rows,
            "palette accent strip (uniform color change)",
            &a_bytes,
            &a0_bytes,
            bg,
            "palette-accent-96x96",
        )?;
    }

    // ------------------------------------------------------------------
    // Court 3 — constant-acceleration object (160x120, 11 frames). A stores
    // one trajectory program; the raster encoder sees accelerating pixels.
    // ------------------------------------------------------------------
    {
        let (w, h) = (160u32, 120u32);
        let bg = 20u8;
        let samples: Vec<u8> = (0..(24 * 16))
            .map(|i| ((i / 24) * 7 + (i % 24) * 3) as u8)
            .collect();
        let mut a = Ingest::new(w, h);
        a.background(bg);
        a.declare_raster(1, 24, 16, samples.clone())?;
        a.instance(1, 1, 20, 20)?;
        for k in 1..=10u64 {
            a.at(k)?;
            if k == 1 {
                a.set_trajectory(
                    1,
                    vec![TrajectorySegment::Accel {
                        vx0: 2,
                        vy0: 0,
                        ax: 1,
                        ay: 0,
                        steps: 10,
                    }],
                )?;
            }
            a.advance_trajectories()?;
        }
        let mut a0 = Ingest::new(w, h);
        a0.background(bg);
        a0.declare_raster(1, 24, 16, samples)?;
        a0.instance(1, 1, 20, 20)?;
        let a_bytes = a.finish()?;
        let a0_bytes = a0.finish()?;
        let fa = frames_of(&a_bytes)?;
        assert_eq!(fa.len(), 11);
        // Closed form: after k advances the tile's origin is at
        // x = 20 + k·2 + k(k−1)/2 (discrete pos += v; v += a).
        assert_eq!(fa[0].get(20, 20), 0);
        assert_eq!(fa[5].get(40, 20), 0, "k=5 => +20 px");
        assert_eq!(fa[10].get(85, 20), 0, "k=10 => +65 px");
        assert_eq!(fa[10].get(84, 20), bg, "left of the tile is background");
        court(
            &mut rows,
            "constant-acceleration object (trajectory)",
            &a_bytes,
            &a0_bytes,
            bg,
            "accel-motion-160x120",
        )?;
    }

    // ------------------------------------------------------------------
    // Court 4 — affine rotation of a noise-textured tile (64x64, 6 frames).
    // A stores a Q8 placement per interval; the flattened sequence is a full
    // 32x32 pixel permutation per frame that no raster family can abstract.
    // ------------------------------------------------------------------
    {
        let (w, h) = (64u32, 64u32);
        let bg = 90u8;
        let mut a = Ingest::new(w, h);
        a.background(bg);
        a.declare_generator(
            1,
            32,
            32,
            vole_video::generator::Generator::Noise { seed: 3 },
        )?;
        a.instance(1, 1, 16, 16)?;
        for k in 1..=5u64 {
            a.at(k)?;
            let params = vole_video::demo::quarter_turn_params(k as i64, 16, 16, 32, 32);
            a.set_affine(1, params)?;
        }
        let mut a0 = Ingest::new(w, h);
        a0.background(bg);
        a0.declare_generator(
            1,
            32,
            32,
            vole_video::generator::Generator::Noise { seed: 3 },
        )?;
        a0.instance(1, 1, 16, 16)?;
        let a_bytes = a.finish()?;
        let a0_bytes = a0.finish()?;
        let fa = frames_of(&a_bytes)?;
        assert_eq!(fa.len(), 6);
        assert_ne!(
            fa[0].get(16, 16),
            fa[1].get(16, 16),
            "rotation permutes pixels"
        );
        assert_eq!(fa[4].get(16, 16), fa[0].get(16, 16), "k=4 is a full turn");
        court(
            &mut rows,
            "affine rotation of a noise tile (Q8)",
            &a_bytes,
            &a0_bytes,
            bg,
            "affine-rotate-noise-64x64",
        )?;
    }

    // ------------------------------------------------------------------
    // Court 5 — authored seeded-noise region, static afterwards (64x64, 3
    // frames). A stores a 9-byte noise program; no raster search can recover
    // an unknown seed (§21), so the flattening tax is structural.
    // ------------------------------------------------------------------
    {
        let (w, h) = (64u32, 64u32);
        let bg = 30u8;
        let mut a = Ingest::new(w, h);
        a.background(bg);
        a.declare_generator(
            1,
            48,
            48,
            vole_video::generator::Generator::Noise { seed: 7 },
        )?;
        a.instance(1, 1, 8, 8)?;
        for k in 1..=2u64 {
            a.at(k)?;
        }
        let mut a0 = Ingest::new(w, h);
        a0.background(bg);
        a0.declare_generator(
            1,
            48,
            48,
            vole_video::generator::Generator::Noise { seed: 7 },
        )?;
        a0.instance(1, 1, 8, 8)?;
        let a_bytes = a.finish()?;
        let a0_bytes = a0.finish()?;
        let fa = frames_of(&a_bytes)?;
        assert_eq!(fa.len(), 3);
        assert_eq!(fa[0], fa[1], "static after the declared content");
        assert_eq!(fa[1], fa[2]);
        court(
            &mut rows,
            "authored seeded-noise region (static)",
            &a_bytes,
            &a0_bytes,
            bg,
            "noise-static-64x64",
        )?;
    }

    // ------------------------------------------------------------------
    // Scripted variant of court 2: the identical content authored in the §53
    // script format parses to the identical stream.
    // ------------------------------------------------------------------
    {
        let mut body = String::from("canvas 96 96\nbackground 40\npalette 1 200 60 90 150 220\n");
        body.push_str("object 1 index 96 8 ");
        for _ in 0..(96 * 8) {
            body.push_str("1 ");
        }
        body.push_str("\ninstance 1 1 0 60 palette 1\n");
        for k in 1..=12u64 {
            let v = if k % 2 == 1 { 200 } else { 60 };
            body.push_str(&format!("at {k}\npatch_palette 1 1={v}\n"));
        }
        let ing = script::parse_script(&body)?;
        let bytes = ing.finish()?;
        let fa = frames_of(&bytes)?;
        assert_eq!(fa.len(), 13);
        assert_eq!(fa[0].get(0, 60), 60, "strip renders palette entry 1 = 60");
        assert_eq!(fa[1].get(0, 60), 200, "interval 1 patches entry 1 to 200");
        assert_eq!(fa[12].get(0, 60), 60);
        println!(
            "script-accent-96x96: parsed script stream {}B (13 frames)",
            bytes.len()
        );
    }

    // ------------------------------------------------------------------
    // Summary table.
    // ------------------------------------------------------------------
    println!();
    println!("| content | frames | raw B | A total B | B total B | total tax | A interval B | B interval B | exact |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for r in &rows {
        let tax = r.b_total as f64 / (r.a_total).max(1) as f64;
        let itax = if r.a_interval > 0 {
            r.b_interval as f64 / (r.a_interval).max(1) as f64
        } else {
            0.0
        };
        println!(
            "| {} | {} | {} | {} | {} | {tax:.2}x | {} | {} ({itax:.1}x) | {} |",
            r.content, r.frames, r.raw, r.a_total, r.b_total, r.a_interval, r.b_interval, r.exact
        );
        assert!(r.exact, "both legs reproduce the canonical raster sequence");
    }
    println!();
    println!("ingest proof: OK ({} courts)", rows.len());
    Ok(())
}
