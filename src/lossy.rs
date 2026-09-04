//! Perceptual profile — Phase U (master brief §64 Phase-U block, §72 language,
//! "keep exact profiles intact").
//!
//! VOLE's normative decoder is, and remains, **exact**: for a given stream it
//! reproduces its content deterministically, sample for sample. A lossy system
//! therefore has to choose its reconstruction first:
//!
//! > `F̂ = Q(F)` — the chosen reconstruction — is what the decoder will
//! > reproduce. The stream that encodes `F̂` is still decoded *exactly*; the
//! > lossiness lives entirely in the deterministic, integer quantization
//! > `Q` applied at encode time, before residuals are formed.
//!
//! This module defines the Phase-U **perceptual profile** over raster-origin
//! input:
//!
//! * [`QuantProfile`] — a deterministic integer quantizer: a lattice
//!   `2^shift` (shift 0 with [`Filter::None`] = exact), a rounding rule
//!   ([`Rounding::HalfUp`] or the dead-zone [`Rounding::DeadZone`]), and an
//!   optional deterministic integer pre-filter ([`Filter::Box3`], a 3-tap
//!   `[1 2 1] ≫ 2` horizontal low-pass). No floating point anywhere in the
//!   normative path.
//! * [`quantize_frames`] applies `Q`; [`encode_lossy`] then runs the
//!   **exhaustive inverse encoder on `F̂`** and **proves** that the normative
//!   decoder reproduces `F̂` byte-for-byte (the materializer is authoritative —
//!   never an assumed hypothesis). Residual dropping is realized *through* the
//!   lattice: detail below the step never reaches a residual because the
//!   encoder's target is already on the lattice.
//! * [`rate_distortion`] / [`choose_rd`] — the deterministic
//!   rate–distortion search: bytes versus distortion (MAE, MSE, peak error)
//!   over the shift ladder under a byte budget.
//! * The resulting stream is marked with feature bit `0x2`
//!   ([`crate::format::FEAT_QUANTIZED_CONTENT`]) — a *declaration* that the
//!   stream's frames are the **chosen reconstruction `F̂`** (the encoder's
//!   deterministic integer `Q` applied at encode time), not the original
//!   capture. The bit never changes decoding; exact profiles are untouched
//!   (a stream whose frames are the source itself never sets it).
//!
//! Claims stay inside §72 language: VOLE does not "eliminate" entropy or
//! "recreate video from math"; the perceptual profile trades fidelity for
//! bytes through a declared, deterministic lattice, and every trade is
//! measured (bytes and distortion), never assumed.

use crate::{
    decoder, encoder, error::VoleError, format::FEAT_QUANTIZED_CONTENT, inverse, limits::Limits,
    pixel::Canvas,
};

/// Deterministic integer rounding rule of the lattice quantizer.
///
/// The quantized sample is `min(255, round_rule(v))`: values whose nearest
/// lattice point is `256` saturate at the Gray8 maximum `255` (the boundary
/// behavior is exact and documented — `255` is the one non-lattice output and
/// only appears in the top half-bin of a non-trivial shift).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// Round half up: `v' = ((v + 2^(shift-1)) >> shift) << shift` (shift 0 =
    /// identity). Maximum error per sample ≤ `2^(shift-1)`.
    HalfUp,
    /// Dead zone at zero: `v' = (v >> shift) << shift` (floor for the
    /// unsigned Gray8 domain). Values inside the dead zone collapse toward
    /// zero; maximum error per sample ≤ `2^shift − 1`.
    DeadZone,
}

/// Optional deterministic integer pre-filter (applied before the lattice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// No filter.
    None,
    /// 3-tap `[1 2 1] ≫ 2` horizontal low-pass with edge replication —
    /// `out[x] = (in[x-1] + 2·in[x] + in[x+1] + 2) >> 2` (deterministic
    /// integer; `in[-1]=in[0]`, `in[n]=in[n-1]`).
    Box3,
}

/// A deterministic integer quantization profile (the perceptual `Q`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantProfile {
    /// Lattice exponent: samples are quantized to multiples of `2^shift`.
    /// `0..=7` (Gray8); `0` with `Filter::None` is the exact profile.
    pub shift: u8,
    /// Rounding rule applied to the lattice.
    pub rounding: Rounding,
    /// Optional pre-filter.
    pub filter: Filter,
}

impl QuantProfile {
    /// The exact (lossless) profile: identity on every sample.
    pub const EXACT: QuantProfile = QuantProfile {
        shift: 0,
        rounding: Rounding::HalfUp,
        filter: Filter::None,
    };

    /// Validate a profile (shift ≤ 7 for Gray8).
    pub fn check(&self) -> Result<(), VoleError> {
        if self.shift > 7 {
            return Err(VoleError::ApiConstraint("quantization shift must be <= 7"));
        }
        Ok(())
    }

    /// Whether this profile is the exact identity (lossless).
    pub fn is_exact(&self) -> bool {
        self.shift == 0 && self.filter == Filter::None
    }

    /// Stable label for receipts, e.g. `q2:halfup` / `q3:deadzone:box3`.
    pub fn label(&self) -> String {
        let mut s = format!("q{}", self.shift);
        s.push_str(match self.rounding {
            Rounding::HalfUp => ":halfup",
            Rounding::DeadZone => ":deadzone",
        });
        if self.filter == Filter::Box3 {
            s.push_str(":box3");
        }
        s
    }
}

/// Quantize one Gray8 sample onto the profile's lattice. Deterministic
/// integer arithmetic; shift 0 with no filter is the identity.
#[inline]
pub fn quantize_sample(profile: &QuantProfile, v: u8) -> u8 {
    let k = u32::from(profile.shift);
    if k == 0 {
        return v;
    }
    let v = u32::from(v);
    let q = match profile.rounding {
        Rounding::HalfUp => {
            let half = 1u32 << (k - 1);
            ((v + half) >> k) << k
        }
        Rounding::DeadZone => (v >> k) << k,
    };
    // Half-up can round 255 up onto 256 at shift 7; clamp back into Gray8.
    q.min(255) as u8
}

/// Apply the profile's pre-filter (integer `[1 2 1] ≫ 2` along each row).
/// A uniform row is preserved exactly (`(4v+2) ≫ 2 == v`).
fn prefilter_row(profile: &QuantProfile, row: &[u8], out: &mut [u8]) {
    match profile.filter {
        Filter::None => out.copy_from_slice(row),
        Filter::Box3 => {
            let n = row.len();
            if n == 0 {
                return;
            }
            for x in 0..n {
                let left = row[x.saturating_sub(1)];
                let center = row[x];
                let right = row[(x + 1).min(n - 1)];
                let acc = u32::from(left) + 2 * u32::from(center) + u32::from(right) + 2;
                out[x] = (acc >> 2) as u8;
            }
        }
    }
}

/// Quantize one frame: apply the pre-filter (per row), then the lattice
/// rounding on every sample. Deterministic; the result is `F̂`.
pub fn quantize_frame(frame: &Canvas, profile: &QuantProfile) -> Result<Canvas, VoleError> {
    profile.check()?;
    let limits = Limits::default();
    let w = frame.width();
    let h = frame.height();
    limits.check_canvas(w, h)?;
    let n = frame.as_slice().len();
    let mut filtered = vec![0u8; n];
    let mut q = vec![0u8; n];
    if profile.filter == Filter::None {
        filtered.copy_from_slice(frame.as_slice());
    } else {
        for y in 0..h as usize {
            let row = &frame.as_slice()[y * w as usize..(y + 1) * w as usize];
            let out = &mut filtered[y * w as usize..(y + 1) * w as usize];
            prefilter_row(profile, row, out);
        }
    }
    for (dst, src) in q.iter_mut().zip(filtered.iter()) {
        *dst = quantize_sample(profile, *src);
    }
    Canvas::from_parts(w, h, q)
}

/// Quantize every frame of a source sequence (the chosen reconstruction `F̂`).
pub fn quantize_frames(
    frames: &[Canvas],
    profile: &QuantProfile,
) -> Result<Vec<Canvas>, VoleError> {
    profile.check()?;
    frames.iter().map(|f| quantize_frame(f, profile)).collect()
}

/// Deterministic distortion of a reconstruction against its source: mean
/// absolute error (×1000, integer), mean squared error, peak absolute error,
/// and the sample count. All integer arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Distortion {
    /// `1000 × Σ|d| / samples` (floor).
    pub mae_x1000: u64,
    /// `Σd² / samples` (floor).
    pub mse: u64,
    /// `max |d|` over all samples.
    pub peak: u64,
    /// Samples compared.
    pub samples: u64,
}

/// Distortion of `recon` against `source` (both must share geometry; samples
/// compared pairwise). `recon` is the decoder-reproducible `F̂`.
pub fn distortion(source: &[Canvas], recon: &[Canvas]) -> Result<Distortion, VoleError> {
    if source.len() != recon.len() {
        return Err(VoleError::LengthMismatch);
    }
    let mut samples = 0u64;
    let mut sum_abs = 0u128;
    let mut sum_sq = 0u128;
    let mut peak = 0u64;
    for (a, b) in source.iter().zip(recon.iter()) {
        if a.width() != b.width() || a.height() != b.height() {
            return Err(VoleError::ObjectGeometryMismatch);
        }
        samples = samples
            .checked_add(a.sample_count())
            .ok_or(VoleError::ArithmeticOverflow)?;
        for (x, y) in a.as_slice().iter().zip(b.as_slice().iter()) {
            let d = u64::from((*x as i32 - *y as i32).unsigned_abs());
            sum_abs += u128::from(d);
            sum_sq += u128::from(d) * u128::from(d);
            peak = peak.max(d);
        }
    }
    if samples == 0 {
        return Err(VoleError::DimensionTooLarge);
    }
    let div = u128::from(samples);
    Ok(Distortion {
        mae_x1000: (sum_abs * 1000 / div) as u64,
        mse: (sum_sq / div) as u64,
        peak,
        samples,
    })
}

/// Options forwarded to the raster-origin encoder.
#[derive(Debug, Clone, Default)]
pub struct LossyOptions {
    /// Background hint for the inverse encoder (default: encoder sweep).
    pub background: Option<u8>,
}

fn encode_opts(opts: &LossyOptions) -> inverse::EncodeOptions {
    inverse::EncodeOptions {
        // Sweep the deterministic background set when no hint is given (the
        // Phase-G default); a fixed hint skips the sweep.
        bg_sweep: opts.background.is_none(),
        background: opts.background,
        ..inverse::EncodeOptions::default()
    }
}

/// A lossy (perceptual-profile) encode: `Q(source)` encoded exactly, decoder
/// output proven equal to `F̂`, distortion and bytes measured.
#[derive(Debug, Clone)]
pub struct LossyReport {
    /// The `.vole` stream (marked with feature bit `0x2` when lossy).
    pub stream: Vec<u8>,
    /// The quantization profile applied.
    pub profile: QuantProfile,
    /// The chosen reconstruction `F̂ = Q(source)` (byte-equal to normative
    /// decode of `stream`).
    pub reconstruction: Vec<Canvas>,
    /// Stream bytes.
    pub bytes: u64,
    /// Distortion of `F̂` against the source.
    pub distortion: Distortion,
    /// Whether the profile is exact (identity — the lossless path).
    pub exact: bool,
}

/// Quantize `source` with `profile`, encode the chosen reconstruction with the
/// exhaustive inverse encoder, mark the stream with the quantized-content
/// declaration (when lossy), and **prove** decoder output == `F̂` through the
/// normative materializer.
pub fn encode_lossy(
    source: &[Canvas],
    profile: &QuantProfile,
    opts: &LossyOptions,
) -> Result<LossyReport, VoleError> {
    profile.check()?;
    let reconstruction = quantize_frames(source, profile)?;
    let report = inverse::encode_frames(&reconstruction, &encode_opts(opts))?;
    let exact = profile.is_exact();
    let stream = if exact {
        report.vole
    } else {
        encoder::mark_quantized_content(&report.vole)?
    };
    // The normative materializer is authoritative: prove F̂, never assume it.
    let parsed = decoder::decode_bytes(&stream)?;
    let decoded = decoder::materialize_all(&parsed)?;
    if decoded.len() != reconstruction.len()
        || decoded
            .iter()
            .zip(reconstruction.iter())
            .any(|(a, b)| !a.exactly_matches(b))
    {
        return Err(VoleError::ApiConstraint(
            "lossy encode failed reconstruction proof",
        ));
    }
    let bytes = stream.len() as u64;
    let d = distortion(source, &reconstruction)?;
    Ok(LossyReport {
        stream,
        reconstruction,
        profile: *profile,
        bytes,
        distortion: d,
        exact,
    })
}

/// One row of the deterministic rate–distortion ladder.
#[derive(Debug, Clone)]
pub struct RdRow {
    /// The profile evaluated.
    pub profile: QuantProfile,
    /// Stream bytes for that profile.
    pub bytes: u64,
    /// Distortion against the source.
    pub distortion: Distortion,
}

/// Evaluate the rate–distortion ladder for `shifts` (0..=`max_shift`, with the
/// profile's rounding/filter fixed): encode the source at every shift and
/// report bytes + distortion per step. Deterministic; every row is an
/// [`encode_lossy`] with its reconstruction proof.
pub fn rate_distortion(
    source: &[Canvas],
    rounding: Rounding,
    filter: Filter,
    max_shift: u8,
    opts: &LossyOptions,
) -> Result<Vec<RdRow>, VoleError> {
    let mut rows = Vec::new();
    for shift in 0..=max_shift.min(7) {
        let profile = QuantProfile {
            shift,
            rounding,
            filter,
        };
        let r = encode_lossy(source, &profile, opts)?;
        rows.push(RdRow {
            profile,
            bytes: r.bytes,
            distortion: r.distortion,
        });
    }
    Ok(rows)
}

/// Deterministic rate–distortion choice under a byte budget. The evaluated
/// ladder's *bytes are not assumed monotone in the lattice* (a courted
/// finding: an intermediate lattice can keep the residual dense while
/// destroying the exact residual's structure, so a lossy row can exceed the
/// lossless row). The choice is therefore the **least-distorted evaluated row
/// whose stream fits the budget** — the RD-optimal selection over the rows
/// actually measured (tie: smaller stream). When even the exact profile is
/// over budget, the exact row is returned with `budget_met == false` (an
/// honest report, never a silent violation). With no budget the **smallest
/// stream** is chosen (tie: least distorted) — the hardest-compression point
/// of the evaluated ladder.
#[derive(Debug, Clone)]
pub struct RdChoice {
    /// The row chosen.
    pub row: RdRow,
    /// Whether the chosen stream met the byte budget.
    pub budget_met: bool,
    /// Full ladder (deterministic, for reporting).
    pub rows: Vec<RdRow>,
}

/// Least-distorted row among `candidates` (tie: smaller stream).
fn least_distorted<'a>(candidates: impl Iterator<Item = &'a RdRow>) -> &'a RdRow {
    candidates
        .min_by(|a, b| {
            (
                a.distortion.mse,
                a.distortion.mae_x1000,
                a.distortion.peak,
                a.bytes,
            )
                .cmp(&(
                    b.distortion.mse,
                    b.distortion.mae_x1000,
                    b.distortion.peak,
                    b.bytes,
                ))
        })
        .expect("non-empty candidates")
}

/// Smallest row among `candidates` (tie: least distorted).
fn smallest<'a>(candidates: impl Iterator<Item = &'a RdRow>) -> &'a RdRow {
    candidates
        .min_by(|a, b| {
            (
                a.bytes,
                a.distortion.mse,
                a.distortion.mae_x1000,
                a.distortion.peak,
            )
                .cmp(&(
                    b.bytes,
                    b.distortion.mse,
                    b.distortion.mae_x1000,
                    b.distortion.peak,
                ))
        })
        .expect("non-empty candidates")
}

/// Choose a profile under `byte_budget` (None = no constraint, smallest
/// stream). See [`RdChoice`] for the exact semantics.
pub fn choose_rd(
    source: &[Canvas],
    rounding: Rounding,
    filter: Filter,
    max_shift: u8,
    byte_budget: Option<u64>,
    opts: &LossyOptions,
) -> Result<RdChoice, VoleError> {
    let rows = rate_distortion(source, rounding, filter, max_shift, opts)?;
    let (row, budget_met) = match byte_budget {
        Some(budget) => {
            let fits: Vec<&RdRow> = rows.iter().filter(|r| r.bytes <= budget).collect();
            if fits.is_empty() {
                // Even the exact profile is over budget: report it and an
                // unmet budget honestly — never a silent violation.
                (rows[0].clone(), false)
            } else {
                (least_distorted(fits.into_iter()).clone(), true)
            }
        }
        None => (smallest(rows.iter()).clone(), true),
    };
    Ok(RdChoice {
        row,
        budget_met,
        rows,
    })
}

/// Whether a stream declares quantized-lattice content (feature bit 0x2).
pub fn declares_quantized(bytes: &[u8]) -> Result<bool, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    Ok(parsed.header().feature_bits() & FEAT_QUANTIZED_CONTENT != 0)
}

/// The phase-U profile's feature-bit name (for receipts).
pub const QUANTIZED_FEATURE_NAME: &str = "FEAT_QUANTIZED_CONTENT 0x2";
