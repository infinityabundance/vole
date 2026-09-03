//! Phase-G evidence producer: exhaustive inverse proceduralization.
//!
//! `cargo run --release --example inverse_proof` prints a deterministic report
//! for the evidence campaign: per-court stream sizes, winner-family structure,
//! complete physical accounting (§31), procedural fraction (§32), and the
//! VOLE-raster-only baseline for the flagship §76-style court. All courts are
//! byte-exact (`decode(materialize(vole)) == input raster`); the encoder
//! refuses to return a stream for which that is false.

use std::time::Instant;

use vole_video::{
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

fn family_counts(report: &inverse::EncodeReport) -> Vec<(&'static str, u64)> {
    let mut counts: Vec<(&str, u64)> = Vec::new();
    for d in &report.decisions {
        if let Some(slot) = counts.iter_mut().find(|(f, _)| *f == d.winner_family) {
            slot.1 += 1;
        } else {
            counts.push((d.winner_family, 1));
        }
    }
    counts
}

fn families(report: &inverse::EncodeReport) -> String {
    family_counts(report)
        .iter()
        .map(|(f, c)| format!("{f}x{c}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_court(name: &str, frames: &[Canvas], opts: &EncodeOptions) {
    let t = Instant::now();
    let report = inverse::encode_frames(frames, opts).expect("encode");
    assert!(report.exact);
    let c = &report.cost;
    println!(
        "court={name} frames={} canvas={}x{} vole={}B raw={}B exact=true proc_fraction={:.4} \
         background={} winner_families=[{}] encode_ms={:.1}",
        report.frame_count,
        report.width,
        report.height,
        report.vole.len(),
        report.raw_raster_bytes,
        c.procedural_fraction(),
        report.background,
        families(&report),
        t.elapsed().as_secs_f64() * 1000.0
    );
    println!(
        "  accounting: header={}B objects={}B checkpoint={}B transitions={}B residual={}B \
         model={}B integrity={}B total={}B",
        c.header_bytes,
        c.object_bytes,
        c.checkpoint_bytes,
        c.transition_bytes,
        c.residual_bytes,
        c.model_bytes,
        c.integrity_bytes,
        c.total_bytes
    );
}

fn main() {
    // --- §76 flagship: 1920x1080, one 200x100 box gliding +2 x per interval.
    // Frame 0 holds the box; frames 1..100 continue the glide.
    {
        let (w, h, vx) = (1920u32, 1080u32, 2i64);
        let mut frames = Vec::new();
        for k in 0..101i64 {
            frames.push(paint_boxes(w, h, 90, &[(100 + vx * k, 60, 200, 100, 180)]));
        }
        let procedural = EncodeOptions {
            bg_sweep: false,
            background: Some(90),
            ..EncodeOptions::default()
        };
        let raster_only = EncodeOptions {
            raster_only: true,
            background: Some(90),
            ..EncodeOptions::default()
        };
        let t = Instant::now();
        let rep = inverse::encode_frames(&frames, &procedural).expect("encode");
        let t_proc = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let rep_raw = inverse::encode_frames(&frames, &raster_only).expect("encode");
        let t_raw = t.elapsed().as_secs_f64();
        assert!(rep.exact && rep_raw.exact);
        println!(
            "flag-76: frames={} vole_procedural={}B vole_raster_only={}B raw_all={}B \
             procedural_over_raw={:.3} raster_only_over_procedural={:.1} \
             proc_fraction={:.4} encode_ms_proc={:.0} encode_ms_raster_only={:.0}",
            rep.frame_count,
            rep.vole.len(),
            rep_raw.vole.len(),
            rep.raw_raster_bytes,
            rep.raw_raster_bytes as f64 / rep.vole.len() as f64,
            rep_raw.vole.len() as f64 / rep.vole.len() as f64,
            rep.cost.procedural_fraction(),
            t_proc * 1000.0,
            t_raw * 1000.0
        );
        let d1 = &rep.decisions[1];
        println!(
            "flag-76-frame1: family={} payload={}B interval={}B candidates={} valid={}",
            d1.winner_family,
            d1.winner_payload_bytes,
            d1.winner_interval_bytes,
            d1.candidates_evaluated,
            d1.candidates_valid
        );
    }

    // --- Static desktop-like scene (persistent unchanged lane at 1920x1080).
    {
        let (w, h) = (1920u32, 1080u32);
        let scene = paint_boxes(
            w,
            h,
            200,
            &[
                (60, 40, 360, 48, 40),
                (60, 100, 1200, 640, 255),
                (60, 760, 1800, 220, 240),
                (1700, 100, 140, 140, 20),
            ],
        );
        let frames = std::iter::repeat_n(scene, 240).collect::<Vec<_>>();
        run_court("static-desktop-1080p", &frames, &EncodeOptions::default());
    }

    // --- §34-style structural-innovation timeline at 640x360: mostly static
    // with one moving sprite interval and a palette-ish flash.
    {
        let (w, h) = (640u32, 360u32);
        let mut frames = Vec::new();
        let base = paint_boxes(w, h, 150, &[(40, 40, 200, 160, 90)]);
        for _ in 0..20 {
            frames.push(base.clone());
        }
        for k in 1..40i64 {
            frames.push(paint_boxes(w, h, 150, &[(40 + k * 2, 40, 200, 160, 90)]));
        }
        for _ in 0..20 {
            frames.push(base.clone());
        }
        for k in 0..10 {
            let v: u8 = if k % 2 == 0 { 255 } else { 150 };
            let f = base.clone();
            let (_, _, mut data) = f.into_parts();
            for i in 0..w as usize {
                data[200 * w as usize + i] = v;
            }
            frames.push(canvas_of(w, h, data));
        }
        run_court(
            "structural-timeline-640x360",
            &frames,
            &EncodeOptions::default(),
        );
    }

    // --- Screen-scroll with new text rows (copy + residual strip) 96x96.
    {
        let (w, h) = (96u32, 96u32);
        let f0 = canvas_of(w, h, {
            let mut d = Vec::with_capacity((w * h) as usize);
            for r in 0..h {
                for c in 0..w {
                    d.push(((c / 8 + r / 2) % 256) as u8);
                }
            }
            d
        });
        let mut rng = Det(0xF00D);
        let mut frames = vec![f0.clone()];
        let s = 3usize;
        for _ in 0..20 {
            let prev = frames.last().unwrap();
            let mut d = Vec::with_capacity((w * h) as usize);
            for y in s..h as usize {
                d.extend_from_slice(&prev.as_slice()[y * w as usize..(y + 1) * w as usize]);
            }
            for _ in 0..s {
                for x in 0..w as usize {
                    d.push(if x % 7 == 0 { 0u8 } else { 32 });
                }
                let _ = rng.byte();
            }
            frames.push(canvas_of(w, h, d));
        }
        run_court(
            "screen-scroll-96x96",
            &frames,
            &EncodeOptions {
                bg_sweep: false,
                ..EncodeOptions::default()
            },
        );
    }

    // --- Noise negative control (RAW fallback, bounded overhead) 64x64.
    {
        let (w, h) = (64u32, 64u32);
        let mut rng = Det(0xDEAD_BEEF);
        let mut frames = Vec::new();
        for _ in 0..12 {
            let mut d = Vec::with_capacity((w * h) as usize);
            for _ in 0..(w * h) {
                d.push(rng.byte());
            }
            frames.push(canvas_of(w, h, d));
        }
        run_court(
            "noise-64x64",
            &frames,
            &EncodeOptions {
                bg_sweep: false,
                ..EncodeOptions::default()
            },
        );
    }

    // --- Cycling uniform A/B panels (fill/object reuse) 192x108.
    {
        let (w, h) = (192u32, 108u32);
        let mut frames = Vec::new();
        for k in 0..60 {
            let v = if (k / 2) % 2 == 0 { 200 } else { 60 };
            let mut d = vec![v; (w * h) as usize];
            // A small persistent control strip that does not change.
            for y in 0..8usize {
                for x in 0..w as usize {
                    d[y * w as usize + x] = 128;
                }
            }
            frames.push(canvas_of(w, h, d));
        }
        run_court("cycling-panels-192x108", &frames, &EncodeOptions::default());
    }
}
