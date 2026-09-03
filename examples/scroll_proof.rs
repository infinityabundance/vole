//! Phase-D evidence producer: COPY_RECT wrap-scroll court accounting.
//!
//! `cargo run --example scroll_proof` prints a deterministic one-line report
//! for the evidence campaign script.

use vole_video::{demo, pixel::Canvas};

fn main() {
    let court = demo::ScrollCourt::default(); // 96x96, scroll 3 rows/interval
    let bytes = court.vole().expect("vole");
    // The byte-exact independent-oracle proof is asserted by the court; rerun
    // it here for the evidence run.
    let frames = court.materialize_and_verify().expect("matches oracle");
    let raw_all = court.raw_bytes_all();
    println!(
        "scroll: frames={} stream={}B raw_all={}B raw_over_stream={:.1}x f0={} flast={}",
        frames.len(),
        bytes.len(),
        raw_all,
        raw_all as f64 / bytes.len() as f64,
        hex(&frames[0]),
        hex(frames.last().unwrap()),
    );
}

fn hex(c: &Canvas) -> String {
    blake3::hash(c.as_slice()).to_hex().to_string()
}
