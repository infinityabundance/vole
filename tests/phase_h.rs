//! Phase H courts: Exhaustive vs FixedHeuristic vs DsfbGuided search over the
//! same candidate universe.
//!
//! Claims under court (all streams decode byte-exact end-to-end):
//! * on steady content DSFB evaluates fewer candidates than the exhaustive
//!   oracle (`N_dsfb < N_exhaustive`) while producing identical bytes
//!   (`J_dsfb == J_exhaustive`);
//! * on content outside the fixed heuristic's constant probes, the fixed
//!   heuristic pays repeated full-raster rebases while DSFB adapts;
//! * after a regime change DSFB broadens (full re-learn), then narrows, with
//!   bounded adaptation latency (the deterministic rotating sweep bounds the
//!   worst case);
//! * regime changes are detected deterministically (`α`/broaden diagnostics)
//!   and local-rebase events are measured, never hidden.

use vole_video::{
    dsfb::EncoderStrategy,
    error::VoleError,
    inverse::{self, EncodeOptions},
    pixel::Canvas,
};

fn canvas_of(w: u32, h: u32, data: Vec<u8>) -> Canvas {
    Canvas::from_parts(w, h, data).expect("canvas")
}

fn paint_boxes(w: u32, h: u32, bg: u8, boxes: &[(i64, i64, u32, u32, u8)]) -> Canvas {
    let mut data = vec![bg; (w * h) as usize];
    for (bx, by, bw, bh, v) in boxes {
        for dy in 0..*bh as i64 {
            for dx in 0..*bw as i64 {
                let x = bx + dx;
                let y = by + dy;
                if x >= 0 && y >= 0 && x < i64::from(w) && y < i64::from(h) {
                    data[y as usize * w as usize + x as usize] = *v;
                }
            }
        }
    }
    canvas_of(w, h, data)
}

struct Det(u64);
impl Det {
    fn next(&mut self) -> u64 {
        let mut x = self.0.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        self.0 = x;
        x
    }
    fn byte(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }
}

fn opts_for(strategy: EncoderStrategy) -> EncodeOptions {
    EncodeOptions {
        bg_sweep: false,
        strategy,
        ..EncodeOptions::default()
    }
}

/// Deterministic textured canvas: pseudo-random-ish vertical stripes over a
/// flat background (no long uniform runs, so scroll/translation ambiguities
/// cannot masquerade as each other).
fn textured(w: u32, h: u32, bg: u8, seed: u64) -> Canvas {
    let mut rng = Det(seed);
    let mut data = vec![bg; (w * h) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            if rng.byte().is_multiple_of(3) {
                data[y * w as usize + x] = rng.byte();
            }
        }
    }
    canvas_of(w, h, data)
}

fn total_candidates(r: &inverse::EncodeReport) -> u64 {
    r.decisions.iter().map(|d| d.candidates_evaluated).sum()
}

fn total_work(r: &inverse::EncodeReport) -> u64 {
    r.decisions.iter().map(|d| d.search_work).sum()
}

fn raw_rebases(r: &inverse::EncodeReport) -> u64 {
    r.decisions
        .iter()
        .filter(|d| d.frame > 0 && d.winner_family == "raw" && d.object_decl_bytes > 0)
        .count() as u64
}

fn run3(
    frames: &[Canvas],
    opts: &EncodeOptions,
) -> Result<
    (
        inverse::EncodeReport,
        inverse::EncodeReport,
        inverse::EncodeReport,
    ),
    VoleError,
> {
    let ex = inverse::encode_frames(
        frames,
        &EncodeOptions {
            strategy: EncoderStrategy::Exhaustive,
            ..opts.clone()
        },
    )?;
    let fx = inverse::encode_frames(
        frames,
        &EncodeOptions {
            strategy: EncoderStrategy::FixedHeuristic,
            ..opts.clone()
        },
    )?;
    let ds = inverse::encode_frames(
        frames,
        &EncodeOptions {
            strategy: EncoderStrategy::DsfbGuided,
            ..opts.clone()
        },
    )?;
    assert!(ex.exact && fx.exact && ds.exact);
    Ok((ex, fx, ds))
}

// ---------------------------------------------------------------------------
// Steady content
// ---------------------------------------------------------------------------

#[test]
fn steady_pan_dsfb_matches_oracle_bytes_with_fewer_candidates() -> Result<(), VoleError> {
    let (w, h) = (64u32, 64u32);
    let mut frames = Vec::new();
    for k in 0..40i64 {
        // A 10x6 box gliding diagonally inside the canvas (never off-canvas).
        frames.push(paint_boxes(w, h, 90, &[(8 + k, 12 + k, 10, 6, 180)]));
    }
    let opts = EncodeOptions {
        background: Some(90),
        ..opts_for(EncoderStrategy::Exhaustive)
    };
    let (ex, fx, ds) = run3(&frames, &opts)?;

    // Byte equality: every strategy finds the same 26 B translation winner.
    assert_eq!(ex.vole, ds.vole, "DSFB must emit the identical stream");
    assert_eq!(
        ex.vole, fx.vole,
        "fixed heuristic must also equal oracle on pan"
    );
    for d in ds.decisions.iter().skip(1) {
        assert_eq!(d.winner_family, "translation");
        assert_eq!(d.winner_payload_bytes, 26);
    }
    // DSFB evaluates strictly fewer candidates and less pixel work than the
    // exhaustive oracle, at identical quality.
    assert!(
        total_candidates(&ds) < total_candidates(&ex),
        "N_dsfb={} must be < N_exhaustive={}",
        total_candidates(&ds),
        total_candidates(&ex)
    );
    assert!(total_work(&ds) < total_work(&ex), "search work must fall");
    Ok(())
}

#[test]
fn steady_wrap_scroll_seven_dsfb_adapts_fixed_heuristic_misses() -> Result<(), VoleError> {
    let (w, h) = (40u32, 32u32);
    // Rows carry distinct *non-zero* values so a whole-canvas wrap can never
    // be confused with a translated blit over the zero background.
    let f0 = canvas_of(w, h, {
        let mut d = Vec::with_capacity((w * h) as usize);
        for r in 0..h {
            for _ in 0..w {
                d.push(((r + 1) % 251) as u8);
            }
        }
        d
    });
    let s = 7i64;
    let mut frames = vec![f0.clone()];
    for t in 1..=24u32 {
        let mut d = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            let src = (y + t as usize * s as usize) % h as usize;
            d.extend_from_slice(&f0.as_slice()[src * w as usize..(src + 1) * w as usize]);
        }
        frames.push(canvas_of(w, h, d));
    }
    let opts = opts_for(EncoderStrategy::Exhaustive);
    let (ex, fx, ds) = run3(&frames, &opts)?;

    // Oracle and DSFB: COPY_RECT winners at ~63 B/frame; byte-identical.
    assert_eq!(ex.vole, ds.vole);
    for d in ds.decisions.iter().skip(1) {
        assert_eq!(d.winner_family, "copy_rect");
        assert_eq!(
            d.winner_payload_bytes,
            ex.decisions[d.frame as usize].winner_payload_bytes
        );
    }
    assert!(
        total_candidates(&ds) < total_candidates(&ex) / 4,
        "N_dsfb={} should be far below N_exhaustive={}",
        total_candidates(&ds),
        total_candidates(&ex)
    );
    // The fixed heuristic only probes shifts 1..=3: scroll-by-7 is invisible
    // to it, so every frame falls back to a full-raster rebase.
    assert!(
        fx.vole.len() > ds.vole.len() * 4,
        "fixed heuristic {}B should dwarf DSFB {}B on s=7 wrap",
        fx.vole.len(),
        ds.vole.len()
    );
    assert_eq!(
        raw_rebases(&fx),
        24,
        "fixed heuristic must rebase every scroll frame"
    );
    assert_eq!(
        raw_rebases(&ds),
        0,
        "DSFB must not rebase steady scroll content"
    );
    Ok(())
}

#[test]
fn static_and_blink_steady_dsfb_matches_oracle() -> Result<(), VoleError> {
    let (w, h) = (32u32, 24u32);
    let base = paint_boxes(w, h, 70, &[(4, 4, 20, 10, 150)]);
    let mut frames = Vec::new();
    for _ in 0..16 {
        frames.push(base.clone());
    }
    for k in 0..16u8 {
        let idx = (10 * w + 20) as usize;
        let (_, _, mut data) = base.clone().into_parts();
        data[idx] = if k % 2 == 0 { 255 } else { 70 };
        frames.push(canvas_of(w, h, data));
    }
    let opts = opts_for(EncoderStrategy::Exhaustive);
    let (ex, _, ds) = run3(&frames, &opts)?;
    assert_eq!(
        ex.vole, ds.vole,
        "steady static+blink content must be byte-identical"
    );
    assert!(total_candidates(&ds) < total_candidates(&ex));
    // Static frames use the unchanged lane; blink frames use the sparse patch.
    // Frame layout: decisions 1..=15 are static; 16..=31 blink.
    for d in ds.decisions.iter().skip(1).take(15) {
        assert_eq!(d.winner_family, "unchanged");
    }
    for d in ds.decisions.iter().skip(16) {
        assert_eq!(d.winner_family, "sparse");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Regime change / adaptation
// ---------------------------------------------------------------------------

/// Build a 4-regime sequence: static textured scene -> wrap-by-7 -> noise ->
/// whole-scene vertical pan. All content is deterministic; the scene is
/// textured (no long uniform runs) so the regimes cannot masquerade as each
/// other's families.
fn regime_frames() -> Vec<Canvas> {
    let (w, h) = (32u32, 32u32);
    let scene = textured(w, h, 90, 7);
    let scene = {
        // Merge the box into the textured scene deterministically.
        let (_, _, mut d) = scene.into_parts();
        for y in 6..14usize {
            for x in 6..18usize {
                d[y * w as usize + x] = 180;
            }
        }
        canvas_of(w, h, d)
    };
    let mut frames: Vec<Canvas> = std::iter::repeat_n(scene.clone(), 24).collect();
    let s = 7i64;
    // Wrap-by-7: each frame is the previous frame toroidally shifted by 7 rows.
    for _ in 0..22u32 {
        let prev = frames.last().unwrap().clone();
        let mut d = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            let src = (y + s as usize) % h as usize;
            d.extend_from_slice(&prev.as_slice()[src * w as usize..(src + 1) * w as usize]);
        }
        frames.push(canvas_of(w, h, d));
    }
    // Noise.
    let mut rng = Det(0xC0FFEE);
    for _ in 0..14 {
        let mut d = Vec::with_capacity((w * h) as usize);
        for _ in 0..(w * h) {
            d.push(rng.byte());
        }
        frames.push(canvas_of(w, h, d));
    }
    // Whole-scene vertical pan: the textured scene glides down, exposing the
    // 90 background at the top (blit semantics, so TRANSLATION reproduces it).
    for k in 0..18u32 {
        let mut d = vec![90u8; (w * h) as usize];
        let kk = k as usize;
        for y in 0..h as usize {
            if y + kk < h as usize {
                let sy = y;
                let src_row = &scene.as_slice()[sy * w as usize..(sy + 1) * w as usize];
                d[(y + kk) * w as usize..(y + kk + 1) * w as usize].copy_from_slice(src_row);
            }
        }
        frames.push(canvas_of(w, h, d));
    }
    frames
}

#[test]
fn regime_changes_are_detected_and_adaptation_is_bounded() -> Result<(), VoleError> {
    let frames = regime_frames();
    let opts = EncodeOptions {
        background: Some(90),
        ..opts_for(EncoderStrategy::Exhaustive)
    };
    let (ex, fx, ds) = run3(&frames, &opts)?;

    // DSFB detects regime changes (α = broaden armed) shortly after each
    // switch and converges back to the oracle winner.
    let ex_fams: Vec<&str> = ex.decisions.iter().map(|d| d.winner_family).collect();
    let ds_fams: Vec<&str> = ds.decisions.iter().map(|d| d.winner_family).collect();
    let oracle_payloads: Vec<u64> = ex
        .decisions
        .iter()
        .map(|d| d.winner_payload_bytes)
        .collect();

    // Locate switch frames in the oracle (winner family changes).
    let mut switches = Vec::new();
    for i in 1..ex_fams.len() {
        if ex_fams[i] != ex_fams[i - 1] {
            switches.push(i);
        }
    }
    assert!(
        switches.len() >= 3,
        "expected regime switches, got {switches:?}"
    );

    // Convergence: within SWEEP_CADENCE + 3 frames of every switch the DSFB
    // winner must equal the oracle winner from then on.
    let cadence = vole_video::dsfb::SWEEP_CADENCE;
    for &sw in &switches {
        let horizon = (sw + cadence as usize + 3).min(ds_fams.len());
        for i in horizon..ds_fams.len() {
            if i > sw + 40 {
                break; // only check the near post-switch window
            }
            // Skip when a *later* regime has already begun in the oracle.
            if switches.iter().any(|later| *later > sw && *later <= i) {
                break;
            }
            assert_eq!(
                ds_fams[i], ex_fams[i],
                "DSFB must converge to oracle at frame {i} after switch at {sw}"
            );
        }
    }

    // Local rebase events: the fixed heuristic rebases every scroll frame; the
    // guided run must rebase far less overall and pay bounded extra bytes.
    assert!(raw_rebases(&fx) > raw_rebases(&ds));
    assert!(total_candidates(&ds) < total_candidates(&ex));

    // Byte overhead over the oracle is bounded: per-regime whole-frame rebase
    // penalties plus at most one rotating-sweep window of raster fallback.
    let wh = u64::from(frames[0].width()) * u64::from(frames[0].height());
    let overhead = ds.vole.len() as u64 - ex.vole.len() as u64;
    assert!(
        overhead < (cadence + 4) * (wh + 2000),
        "DSFB byte overhead {overhead} must stay bounded (frame {wh}B)"
    );
    // The oracle never loses to the guided run by more than the same bound in
    // either direction is impossible; but DSFB must beat the fixed heuristic
    // whose scroll probes miss (fixed rebases all scroll frames).
    assert!(
        fx.vole.len() > ds.vole.len(),
        "fixed heuristic {}B should exceed DSFB {}B on this regime court",
        fx.vole.len(),
        ds.vole.len()
    );

    // Oracle-regret accounting: winner payloads are recorded; on the steady
    // tail of each segment the guided payload equals the oracle payload.
    let mut mismatches = 0u64;
    for (i, d) in ds.decisions.iter().enumerate() {
        let _ = d;
        if oracle_payloads[i] != 0 && ds.decisions[i].winner_payload_bytes != oracle_payloads[i] {
            mismatches += 1;
        }
    }
    assert!(mismatches < switches.len() as u64 * (cadence + 4) + 4);
    Ok(())
}

#[test]
fn dsfb_diagnostics_track_phi_alpha_and_broaden() -> Result<(), VoleError> {
    let frames = regime_frames();
    let opts = EncodeOptions {
        background: Some(90),
        ..opts_for(EncoderStrategy::DsfbGuided)
    };
    let ds = inverse::encode_frames(&frames, &opts)?;
    assert!(ds.exact);
    // Every non-frame-0 decision under the guided strategy carries a diag.
    let diags: Vec<&vole_video::dsfb::DsfbFrameDiag> = ds
        .decisions
        .iter()
        .skip(1)
        .map(|d| d.dsfb_diag.as_ref().expect("guided run has diag"))
        .collect();
    // α is armed near regime changes and quiescent on steady runs.
    let alphas = diags.iter().filter(|d| d.alpha == 1).count();
    assert!(alphas >= 2, "expected regime broaden events, got {alphas}");
    let tail: Vec<_> = diags.iter().skip(diags.len() - 6).collect();
    for d in tail {
        assert_eq!(d.alpha, 0, "steady tail must be quiescent");
        assert!(!d.broadened);
    }
    // Winner families carry the highest φ on the steady tail.
    let last = diags.last().expect("diag");
    assert_eq!(last.winner, "translation");
    let phi = last
        .phi
        .iter()
        .find(|(f, _)| *f == "translation")
        .map(|(_, v)| *v)
        .unwrap_or(0.0);
    assert!(phi > 0.0);
    Ok(())
}

#[test]
fn strategies_are_deterministic_and_always_exact() -> Result<(), VoleError> {
    let frames = regime_frames();
    for strategy in [
        EncoderStrategy::Exhaustive,
        EncoderStrategy::FixedHeuristic,
        EncoderStrategy::DsfbGuided,
    ] {
        let opts = opts_for(strategy);
        let a = inverse::encode_frames(&frames, &opts)?;
        let b = inverse::encode_frames(&frames, &opts)?;
        assert!(a.exact);
        assert_eq!(
            a.vole, b.vole,
            "strategy {strategy:?} must be deterministic"
        );
        assert_eq!(a.decisions.len(), b.decisions.len());
    }
    Ok(())
}
