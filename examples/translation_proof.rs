//! Phase-E evidence producer: persistent integer-translation accounting.
//!
//! `cargo run --example translation_proof` prints a deterministic report for
//! the evidence campaign: stream sizes for the persistent-translation court,
//! the equivalent per-frame `SetPosition` baseline, the raw-frame total, and
//! byte-exactness of the materialized frames against the independent painter.

use vole_video::{decoder, demo};

fn main() {
    let court = demo::TranslationCourt::default(); // 1920x1080, vx=2, vy=1, 100 intervals
    let translation = court.vole().expect("vole");
    let baseline = court.delta_baseline_bytes().expect("baseline");
    let raw_all = court.raw_bytes_all();
    let frames = court.materialize_and_verify().expect("exact vs reference");
    let parsed = decoder::decode_bytes(&translation).expect("parse");
    assert_eq!(parsed.frame_count(), court.frame_count());
    assert_eq!(frames.len() as u64, court.frame_count());
    println!(
        "translation: frames={} vx={} vy={} trans_stream={}B delta_stream={}B raw_all={}B \
         trans_vs_delta={:.2}x raw_over_trans={:.0}x exact={}",
        frames.len(),
        court.vx,
        court.vy,
        translation.len(),
        baseline.len(),
        raw_all,
        baseline.len() as f64 / translation.len() as f64,
        raw_all as f64 / translation.len() as f64,
        frames.iter().enumerate().all(|(k, c)| {
            c.as_slice()
                == &court.reference_raw()[k * (court.width as usize) * (court.height as usize)
                    ..(k + 1) * (court.width as usize) * (court.height as usize)]
        })
    );
    // Camera-like translation court (large region, 200 intervals).
    let camera = demo::TranslationCourt {
        width: 1280,
        height: 720,
        box_w: 640,
        box_h: 720,
        x0: 0,
        y0: 0,
        vx: 4,
        vy: 0,
        intervals: 200,
        ..demo::TranslationCourt::default()
    };
    let cbytes = camera.vole().expect("vole");
    let cframes = camera.materialize_and_verify().expect("exact");
    println!(
        "camera: frames={} stream={}B raw_all={}B exact={}",
        cframes.len(),
        cbytes.len(),
        camera.raw_bytes_all(),
        cframes
            .iter()
            .all(|c| c.sample_count() == u64::from(camera.width) * u64::from(camera.height))
    );
    // Static control: zero translation, all frames identical.
    let static_court = demo::TranslationCourt {
        vx: 0,
        vy: 0,
        ..demo::TranslationCourt::default()
    };
    let sframes = static_court.materialize_and_verify().expect("exact");
    let f0 = sframes.first().unwrap();
    println!(
        "static-control: frames={} all_identical={} stream={}B",
        sframes.len(),
        sframes.iter().all(|f| f.exactly_matches(f0)),
        static_court.vole().expect("vole").len()
    );
}
