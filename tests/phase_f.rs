//! Phase F courts: the VOLE-native byte rANS entropy floor.
//!
//! 1. **Byte parity** against the independent `ryg-rans-rs` reconstruction of
//!    `ryg_rans` (a test-only oracle; never linked into normative decode):
//!    our encoder output must be byte-identical to the reference's single-state
//!    byte-rANS output for the same model, and each side must decode the
//!    other's stream.
//! 2. **Property round-trips** over an adversarial deterministic corpus
//!    (runs, skew, uniform, pathological lengths).
//! 3. **Accounting**: the declared RAW-fallback policy (RANS only when
//!    `model + encoded < raw`) holds on skewed and uniform content.
//! 4. **Hostile**: truncation, corruption, length-bomb declarations never
//!    panic and always resolve to a typed error or a bounded decode.

use ryg_rans_rs::byte::{
    rans_byte_dec_advance_symbol, rans_byte_dec_get, rans_byte_dec_init, rans_byte_enc_flush,
    rans_byte_enc_put_symbol, BackwardByteWriter, ByteReader, RansByteDecSymbol, RansByteEncSymbol,
    RansByteState,
};
use vole_video::{
    error::VoleError,
    rans::{self, ByteModel, MODEL_TOTAL, SCALE_BITS, STATE_L},
};

/// Deterministic xorshift64* byte stream.
fn det(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed.max(1);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        out.push(((s.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 56) as u8);
    }
    out
}

/// Build the corpus: (label, bytes).
fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    let mut c: Vec<(&'static str, Vec<u8>)> = Vec::new();

    c.push(("empty", Vec::new()));
    for n in [
        1usize, 2, 3, 7, 8, 16, 63, 64, 65, 255, 256, 257, 511, 512, 1024, 4096, 16384,
    ] {
        c.push(("uniform", det(n as u64 * 7 + 1, n)));
        // single-symbol runs force the degenerate all-mass symbol
        c.push(("single-symbol-run", vec![0x41; n]));
        c.push(("two-symbol-weighted", two_symbol(n, 13)));
        c.push(("runs", runs(n)));
    }
    // long mixed skew
    let mut skew = vec![b'a'; 1 << 16];
    for i in (0..skew.len()).step_by(97) {
        skew[i] = b'b';
    }
    for i in (3..skew.len()).step_by(251) {
        skew[i] = b'c';
    }
    c.push(("text-skew-65536", skew));
    // alternating long runs of two chars
    let mut alt = Vec::with_capacity(200_000);
    let mut on = true;
    let mut left = 3_997usize;
    while alt.len() < 200_000 {
        let b = if on { 0xEE } else { 0x11 };
        let take = left.min(200_000 - alt.len());
        alt.extend(std::iter::repeat_n(b, take));
        on = !on;
        left = (left * 7 + 13) % 5_000 + 1;
    }
    c.push(("alternating-runs-200k", alt));
    c
}

fn two_symbol(n: usize, seed: u64) -> Vec<u8> {
    let mut s = seed.max(1);
    (0..n)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            if (s.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 63 == 0 {
                b'0'
            } else {
                b'1'
            }
        })
        .collect()
}

fn runs(n: usize) -> Vec<u8> {
    // deterministic run-length-ish pattern of symbols
    let mut out = Vec::with_capacity(n);
    let mut v = 0u8;
    let mut run = 1usize;
    while out.len() < n {
        let take = run.min(n - out.len());
        out.extend(std::iter::repeat_n(v, take));
        v = v.wrapping_add(7);
        run = (run * 5 + 3) % 37 + 1;
    }
    out
}

fn cum2sym(model: &ByteModel) -> Vec<u8> {
    let mut t = vec![0u8; MODEL_TOTAL as usize];
    for s in 0..256usize {
        let start = model.start(s) as usize;
        let f = model.freq(s) as usize;
        for slot in t.iter_mut().take(start + f).skip(start) {
            *slot = s as u8;
        }
    }
    t
}

/// Reference decoder over a payload (mirror of the reference API usage).
fn reference_decode(payload: &[u8], model: &ByteModel, n: usize) -> Result<Vec<u8>, ()> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut reader = ByteReader::new(payload);
    let mut st = rans_byte_dec_init(&mut reader).map_err(|_| ())?;
    let mut out = vec![0u8; n];
    let c2s = cum2sym(model);
    for i in (0..n).rev() {
        let cf = rans_byte_dec_get(&st, SCALE_BITS);
        let s = c2s[cf as usize] as usize;
        out[i] = s as u8;
        // Decoded symbols always have freq >= 1 (they occur in the data).
        let d = RansByteDecSymbol::new(model.start(s), model.freq(s)).unwrap();
        rans_byte_dec_advance_symbol(&mut st, &mut reader, &d, SCALE_BITS).map_err(|_| ())?;
    }
    Ok(out)
}

#[test]
fn byte_parity_and_cross_decode_with_reference_oracle() {
    for (name, data) in corpus() {
        let model = ByteModel::from_data(&data);

        // Our encoder.
        let ours = rans::encode_with(&data, &model).unwrap_or_else(|e| {
            panic!("our encode failed for {name}: {e:?}");
        });

        // Reference single-state byte-rANS encode (put each symbol forward, flush).
        let mut buf = vec![0u8; 4 + 3 * data.len() + 64];
        let mut w = BackwardByteWriter::new(&mut buf);
        let mut st = RansByteState::new();
        for &b in data.iter() {
            let s = b as usize;
            let es = RansByteEncSymbol::new(model.start(s), model.freq(s), SCALE_BITS)
                .unwrap_or_else(|e| panic!("ref esym for {name}: {e:?}"));
            rans_byte_enc_put_symbol(&mut st, &mut w, &es)
                .unwrap_or_else(|_| panic!("ref encode put failed for {name}"));
        }
        rans_byte_enc_flush(&st, &mut w).unwrap();
        let theirs = w.encoded().to_vec();

        assert_eq!(ours, theirs, "byte parity failed for corpus {name}");

        // Cross-decode: our decoder reads the reference stream.
        let back_ours = rans::decode_with(&theirs, &model, data.len())
            .unwrap_or_else(|e| panic!("our decode of ref stream failed for {name}: {e:?}"));
        assert_eq!(back_ours, data, "our-decode(ref) mismatch for {name}");

        // Cross-decode: reference decoder reads our stream.
        let back_theirs = reference_decode(&ours, &model, data.len())
            .unwrap_or_else(|_| panic!("ref decode of our stream failed for {name}"));
        assert_eq!(back_theirs, data, "ref-decode(ours) mismatch for {name}");

        // Self round trip through the block container.
        let block = rans::encode_block(&data);
        let back = rans::decode_block(&block, 1 << 22)
            .unwrap_or_else(|e| panic!("block decode failed for {name}: {e:?}"));
        assert_eq!(back, data, "block roundtrip mismatch for {name}");
    }
}

#[test]
fn skewed_runs_choose_rans_and_gain() {
    // Single-symbol run: model mass collapses to one symbol; rANS payload is
    // just the 4-byte state, so RANS must be chosen and tiny.
    let data = vec![0x41u8; 100_000];
    let block = rans::encode_block(&data);
    assert_eq!(block[0], rans::KIND_RANS);
    assert!(
        block.len() < 600,
        "single-symbol payload must be near model size"
    );
    assert_eq!(rans::decode_block(&block, 1 << 22).unwrap(), data);
}

#[test]
fn uniform_falls_back_to_raw() {
    let data = det(0xDEAD_BEEF, 20_000);
    let block = rans::encode_block(&data);
    assert_eq!(block[0], rans::KIND_RAW);
    assert_eq!(rans::decode_block(&block, 1 << 22).unwrap(), data);
}

#[test]
fn declared_length_bomb_is_bounded() {
    // A hostile RANS block declaring out_len = 2^40 with a tiny model body must
    // fail *before allocating* whenever the caller supplies a sane bound.
    let data = vec![7u8; 5000];
    let block = rans::encode_block(&data);
    let mut bomb = block.clone();
    bomb[1..9].copy_from_slice(&(1u64 << 40).to_le_bytes());
    assert!(matches!(
        rans::decode_block(&bomb, 1 << 20),
        Err(VoleError::DimensionTooLarge)
    ));

    // A *small* lie on skewed (non-degenerate, rANS-coded) data must fail
    // structurally with a typed error (renorm overread), never silently return
    // short output and never panic.
    let skew = two_symbol(5000, 2024);
    let sblock = rans::encode_block(&skew);
    assert_eq!(sblock[0], rans::KIND_RANS, "weighted binary should be rANS");
    let mut lie = sblock.clone();
    lie[1..9].copy_from_slice(&(skew.len() as u64 + 500).to_le_bytes());
    assert!(rans::decode_block(&lie, 1 << 20).is_err());
}

#[test]
fn every_truncation_is_a_typed_error() {
    let data = det(99, 4096);
    let block = rans::encode_block(&data);
    for cut in 0..block.len() {
        let r = rans::decode_block(&block[..cut], 1 << 22);
        assert!(r.is_err(), "truncation at byte {cut} must error");
    }
}

#[test]
fn corruption_is_deterministic_and_bounded() {
    let data = det(42, 4096);
    let block = rans::encode_block(&data);
    for i in 0..block.len() {
        let mut a = block.clone();
        a[i] ^= 0xA5;
        let ra = rans::decode_block(&a, 1 << 22);
        // deterministic: identical input -> identical outcome
        let mut b = block.clone();
        b[i] ^= 0xA5;
        let rb = rans::decode_block(&b, 1 << 22);
        assert_eq!(ra.is_ok(), rb.is_ok());
        if let (Ok(x), Ok(y)) = (&ra, &rb) {
            assert_eq!(x, y);
        }
        // valid decode OR typed error; never panic (reaching here proves it)
        let _ = ra;
    }
}

#[test]
fn model_serialization_roundtrips() {
    let data = det(1234, 2048);
    let m = ByteModel::from_data(&data);
    let bytes = m.to_bytes();
    assert_eq!(bytes.len(), rans::MODEL_SERIALIZED);
    let m2 = ByteModel::from_bytes(&bytes).unwrap();
    assert_eq!(m, m2);
    // The re-encoded payload is byte-identical (canonical model bytes).
    let b1 = rans::encode_with(&data, &m).unwrap();
    let b2 = rans::encode_with(&data, &m2).unwrap();
    assert_eq!(b1, b2);
}

#[test]
fn normative_constants_are_pinned() {
    // These are frozen by the Phase-F receipt; changing them silently would
    // break every golden stream and parity court.
    assert_eq!(SCALE_BITS, 14);
    assert_eq!(MODEL_TOTAL, 1 << 14);
    assert_eq!(STATE_L, 1 << 23);
    assert_eq!(rans::MODEL_SERIALIZED, 512);
}
