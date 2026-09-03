//! Phase-C evidence producer: sparse-mutation court accounting.
//!
//! `cargo run --example sparse_proof` prints a deterministic single-line
//! report that the evidence-campaign script consumes.

use vole::{decoder, demo, pixel::Canvas};

fn main() {
    let court = demo::BlinkCourt::default();
    let bytes = court.vole().expect("vole");
    let parsed = decoder::decode_bytes(&bytes).expect("parse");
    let frames = decoder::materialize_all(&parsed).expect("materialize");
    let raw_all = court.reference_raw().len() as u64;
    println!(
        "blink: frames={} stream={}B raw_all={}B raw_over_stream={:.0}x f0={} flast={}",
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
