//! Phase-F evidence producer: VOLE native byte-rANS floor accounting.
//!
//! `cargo run --example rans_proof` prints a deterministic report consumed by
//! the evidence-campaign script: rANS-vs-RAW selection, measured sizes on
//! skewed / single-symbol / uniform corpora, and the pinned normative
//! constants. (Byte parity with the `ryg-rans-rs` oracle is asserted in
//! `tests/phase_f.rs`, not re-measured here.)

use vole_video::rans;

fn verify_roundtrip(data: &[u8], block: &[u8]) -> bool {
    rans::decode_block(block, (data.len() as u64).saturating_mul(2).max(1 << 22))
        .map(|back| back == data)
        .unwrap_or(false)
}

fn main() {
    // 1) Heavily skewed "screen-text-like" payload.
    let mut skew = vec![b' '; 262_144];
    for i in (0..skew.len()).step_by(97) {
        skew[i] = b'e';
    }
    for i in (3..skew.len()).step_by(251) {
        skew[i] = b'\n';
    }
    let sblock = rans::encode_block(&skew);
    let s_kind = if sblock[0] == rans::KIND_RANS {
        "RANS"
    } else {
        "RAW"
    };

    // 2) Single-symbol run.
    let ones = vec![0x41u8; 262_144];
    let oblock = rans::encode_block(&ones);
    let o_kind = if oblock[0] == rans::KIND_RANS {
        "RANS"
    } else {
        "RAW"
    };

    // 3) Uniform pseudo-random bytes.
    let mut seed = 0x1234_5678_9ABC_DEF0u64;
    let mut uniform = Vec::with_capacity(262_144);
    for _ in 0..262_144 {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        uniform.push(((seed.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 56) as u8);
    }
    let ublock = rans::encode_block(&uniform);
    let u_kind = if ublock[0] == rans::KIND_RANS {
        "RANS"
    } else {
        "RAW"
    };

    println!(
        "rans: scale_bits={} total={} state_l={} model_bytes={}",
        rans::SCALE_BITS,
        rans::MODEL_TOTAL,
        rans::STATE_L,
        rans::MODEL_SERIALIZED
    );
    println!(
        "skew: n={} kind={} block={}B ratio={:.2} verify={}",
        skew.len(),
        s_kind,
        sblock.len(),
        skew.len() as f64 / sblock.len() as f64,
        verify_roundtrip(&skew, &sblock)
    );
    println!(
        "single-symbol: n={} kind={} block={}B ratio={:.2} verify={}",
        ones.len(),
        o_kind,
        oblock.len(),
        ones.len() as f64 / oblock.len() as f64,
        verify_roundtrip(&ones, &oblock)
    );
    println!(
        "uniform: n={} kind={} block={}B verify={}",
        uniform.len(),
        u_kind,
        ublock.len(),
        verify_roundtrip(&uniform, &ublock)
    );
}
