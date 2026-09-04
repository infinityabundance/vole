//! Color semantics — Phase V.1.1 (V.1 brief §21–§24, contract §2.4).
//!
//! Color interpretation is part of the image semantics, not incidental
//! metadata: primaries, transfer characteristic, matrix, range, and chroma
//! sample location are preserved exactly and **never inferred** ("HD means
//! BT.709" is forbidden). `Unspecified` means the source did not declare the
//! property — it is stored as unspecified, never guessed (§21). Canonical
//! storage preserves encoded component values; presentation projection
//! (V.1.14+) handles interpretation, and stored HDR media is never silently
//! tone-mapped (§23–§24).
//!
//! HDR static metadata units are declared here and validated: mastering
//! display chromaticity coordinates are integers in `0..=50_000` representing
//! `0.00002` units (SMPTE ST 2086 convention), and luminances are integers in
//! `0.0001 cd/m²` units. These unit conventions become normative with the v2
//! wire grammar (frozen at the end of V.1.2).

use crate::error::VoleError;

/// Mastering-display chromaticity coordinate bound (ST 2086: 0.00002 units).
pub const MAX_CHROMATICITY: u32 = 50_000;
/// Mastering-display luminance bound in 0.0001 cd/m² (100 000 = 10 cd/m² …
/// 100 000 000 = 10 000 cd/m²).
pub const MAX_LUMINANCE_UNITS: u32 = 100_000_000;

/// Color primaries of the encoded samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorPrimaries {
    /// Not declared by the source.
    Unspecified,
    /// ITU-R BT.709.
    Bt709,
    /// ITU-R BT.470 System M.
    Bt470M,
    /// ITU-R BT.470 System B/G.
    Bt470Bg,
    /// SMPTE 170 M (BT.601-family SD).
    Smpte170M,
    /// SMPTE 240 M.
    Smpte240M,
    /// Generic film.
    Film,
    /// ITU-R BT.2020.
    Bt2020,
}

impl ColorPrimaries {
    /// Whether the source declared nothing (never guessed).
    pub fn is_unspecified(self) -> bool {
        self == ColorPrimaries::Unspecified
    }

    /// Stable label.
    pub fn label(self) -> &'static str {
        match self {
            ColorPrimaries::Unspecified => "unspecified",
            ColorPrimaries::Bt709 => "bt709",
            ColorPrimaries::Bt470M => "bt470m",
            ColorPrimaries::Bt470Bg => "bt470bg",
            ColorPrimaries::Smpte170M => "smpte170m",
            ColorPrimaries::Smpte240M => "smpte240m",
            ColorPrimaries::Film => "film",
            ColorPrimaries::Bt2020 => "bt2020",
        }
    }
}

/// Transfer characteristic (opto-electronic transfer function of the encoded
/// samples).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferCharacteristic {
    /// Not declared by the source.
    Unspecified,
    /// ITU-R BT.709.
    Bt709,
    /// Gamma 2.2.
    Gamma22,
    /// Gamma 2.8.
    Gamma28,
    /// SMPTE 170 M.
    Smpte170M,
    /// SMPTE 240 M.
    Smpte240M,
    /// Linear.
    Linear,
    /// IEC 61966-2-1 (sRGB).
    Srgb,
    /// ITU-R BT.2020 (10-bit).
    Bt2020_10,
    /// ITU-R BT.2020 (12-bit).
    Bt2020_12,
    /// SMPTE ST 2084 (PQ).
    Smpte2084,
    /// ARIB STD-B67 (HLG).
    AribStdB67,
}

impl TransferCharacteristic {
    /// Whether the source declared nothing (never guessed).
    pub fn is_unspecified(self) -> bool {
        self == TransferCharacteristic::Unspecified
    }

    /// Stable label.
    pub fn label(self) -> &'static str {
        match self {
            TransferCharacteristic::Unspecified => "unspecified",
            TransferCharacteristic::Bt709 => "bt709",
            TransferCharacteristic::Gamma22 => "gamma22",
            TransferCharacteristic::Gamma28 => "gamma28",
            TransferCharacteristic::Smpte170M => "smpte170m",
            TransferCharacteristic::Smpte240M => "smpte240m",
            TransferCharacteristic::Linear => "linear",
            TransferCharacteristic::Srgb => "srgb",
            TransferCharacteristic::Bt2020_10 => "bt2020-10",
            TransferCharacteristic::Bt2020_12 => "bt2020-12",
            TransferCharacteristic::Smpte2084 => "pq",
            TransferCharacteristic::AribStdB67 => "hlg",
        }
    }
}

/// Matrix coefficients used to derive luma/chroma from RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatrixCoefficients {
    /// Not declared by the source.
    Unspecified,
    /// Identity (RGB content, no luma/chroma matrix).
    Identity,
    /// ITU-R BT.709.
    Bt709,
    /// SMPTE 170 M (BT.601-family SD).
    Smpte170M,
    /// SMPTE 240 M.
    Smpte240M,
    /// YCgCo.
    YcgCo,
    /// ITU-R BT.2020 non-constant luminance.
    Bt2020Ncl,
    /// ITU-R BT.2020 constant luminance.
    Bt2020Cl,
}

impl MatrixCoefficients {
    /// Whether the source declared nothing (never guessed).
    pub fn is_unspecified(self) -> bool {
        self == MatrixCoefficients::Unspecified
    }

    /// Whether this matrix means the samples are RGB (no luma/chroma
    /// derivation needed at presentation).
    pub fn is_identity(self) -> bool {
        self == MatrixCoefficients::Identity
    }

    /// Stable label.
    pub fn label(self) -> &'static str {
        match self {
            MatrixCoefficients::Unspecified => "unspecified",
            MatrixCoefficients::Identity => "identity",
            MatrixCoefficients::Bt709 => "bt709",
            MatrixCoefficients::Smpte170M => "smpte170m",
            MatrixCoefficients::Smpte240M => "smpte240m",
            MatrixCoefficients::YcgCo => "ycgco",
            MatrixCoefficients::Bt2020Ncl => "bt2020ncl",
            MatrixCoefficients::Bt2020Cl => "bt2020cl",
        }
    }
}

/// Luma/chroma sample range of the encoded samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorRange {
    /// Not declared by the source.
    Unspecified,
    /// Limited ("studio") range with headroom/footroom.
    Limited,
    /// Full range.
    Full,
}

impl ColorRange {
    /// Whether the source declared nothing (never guessed).
    pub fn is_unspecified(self) -> bool {
        self == ColorRange::Unspecified
    }

    /// Stable label.
    pub fn label(self) -> &'static str {
        match self {
            ColorRange::Unspecified => "unspecified",
            ColorRange::Limited => "limited",
            ColorRange::Full => "full",
        }
    }
}

/// Chroma sample location relative to luma samples (affects presentation
/// resampling and any motion-coordinate conversion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromaLocation {
    /// Not declared by the source.
    Unspecified,
    /// Horizontally centered (MPEG-2/4:2:0 default class).
    Center,
    /// Left-aligned (JPEG-class).
    Left,
    /// Top-left (DV-class 4:1:1).
    TopLeft,
    /// Top-centered.
    Top,
    /// Bottom-left.
    BottomLeft,
    /// Bottom-centered.
    Bottom,
}

impl ChromaLocation {
    /// Whether the source declared nothing.
    pub fn is_unspecified(self) -> bool {
        self == ChromaLocation::Unspecified
    }

    /// Stable label.
    pub fn label(self) -> &'static str {
        match self {
            ChromaLocation::Unspecified => "unspecified",
            ChromaLocation::Center => "center",
            ChromaLocation::Left => "left",
            ChromaLocation::TopLeft => "topleft",
            ChromaLocation::Top => "top",
            ChromaLocation::BottomLeft => "bottomleft",
            ChromaLocation::Bottom => "bottom",
        }
    }
}

/// The full color interpretation of an epoch's encoded samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorDescription {
    primaries: ColorPrimaries,
    transfer: TransferCharacteristic,
    matrix: MatrixCoefficients,
    range: ColorRange,
    chroma_location: ChromaLocation,
}

impl ColorDescription {
    /// A complete color description. All components may be
    /// `Unspecified`-independent: each property is preserved exactly as
    /// declared, never inferred.
    pub fn new(
        primaries: ColorPrimaries,
        transfer: TransferCharacteristic,
        matrix: MatrixCoefficients,
        range: ColorRange,
        chroma_location: ChromaLocation,
    ) -> Self {
        ColorDescription {
            primaries,
            transfer,
            matrix,
            range,
            chroma_location,
        }
    }

    /// The common H.273-style `unspecified` description (every property
    /// unspecified). Used only when the source genuinely declares nothing.
    pub fn unspecified() -> Self {
        Self::new(
            ColorPrimaries::Unspecified,
            TransferCharacteristic::Unspecified,
            MatrixCoefficients::Unspecified,
            ColorRange::Unspecified,
            ChromaLocation::Unspecified,
        )
    }

    /// BT.709 SDR (limited range, centered chroma): the "HD SDR" signaling
    /// set, only meaningful when the source declares it.
    pub fn bt709_sdr() -> Self {
        Self::new(
            ColorPrimaries::Bt709,
            TransferCharacteristic::Bt709,
            MatrixCoefficients::Bt709,
            ColorRange::Limited,
            ChromaLocation::Center,
        )
    }

    /// BT.601-family SD signaling (SMPTE 170 M).
    pub fn bt601_sd() -> Self {
        Self::new(
            ColorPrimaries::Smpte170M,
            TransferCharacteristic::Smpte170M,
            MatrixCoefficients::Smpte170M,
            ColorRange::Limited,
            ChromaLocation::Center,
        )
    }

    /// BT.2020 + SMPTE ST 2084 (PQ) 10-bit HDR signaling.
    pub fn bt2020_pq() -> Self {
        Self::new(
            ColorPrimaries::Bt2020,
            TransferCharacteristic::Smpte2084,
            MatrixCoefficients::Bt2020Ncl,
            ColorRange::Limited,
            ChromaLocation::Center,
        )
    }

    /// BT.2020 + ARIB STD-B67 (HLG) signaling.
    pub fn bt2020_hlg() -> Self {
        Self::new(
            ColorPrimaries::Bt2020,
            TransferCharacteristic::AribStdB67,
            MatrixCoefficients::Bt2020Ncl,
            ColorRange::Limited,
            ChromaLocation::Center,
        )
    }

    /// Full-range identity (RGB-family pictures).
    pub fn rgb_full() -> Self {
        Self::new(
            ColorPrimaries::Bt709,
            TransferCharacteristic::Srgb,
            MatrixCoefficients::Identity,
            ColorRange::Full,
            ChromaLocation::Unspecified,
        )
    }

    /// Accessors.
    pub fn primaries(&self) -> ColorPrimaries {
        self.primaries
    }
    /// Accessors.
    pub fn transfer(&self) -> TransferCharacteristic {
        self.transfer
    }
    /// Accessors.
    pub fn matrix(&self) -> MatrixCoefficients {
        self.matrix
    }
    /// Accessors.
    pub fn range(&self) -> ColorRange {
        self.range
    }
    /// Accessors.
    pub fn chroma_location(&self) -> ChromaLocation {
        self.chroma_location
    }

    /// Whether any property is unspecified (never silently completed).
    pub fn has_unspecified(&self) -> bool {
        self.primaries.is_unspecified()
            || self.transfer.is_unspecified()
            || self.matrix.is_unspecified()
            || self.range.is_unspecified()
            || self.chroma_location.is_unspecified()
    }

    /// Compact stable descriptor, e.g. `bt709/bt709/bt709/limited/center`.
    pub fn describe(&self) -> String {
        format!(
            "{}/{}/{}/{}/{}",
            self.primaries.label(),
            self.transfer.label(),
            self.matrix.label(),
            self.range.label(),
            self.chroma_location.label()
        )
    }
}

/// SMPTE ST 2086 mastering-display colour volume (static HDR metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasteringDisplay {
    /// Display primaries as `(x, y)` chromaticity coordinates in `0.00002`
    /// units: `[red, green, blue]`.
    pub display_primaries: [(u16, u16); 3],
    /// White point `(x, y)` in the same units.
    pub white_point: (u16, u16),
    /// Maximum luminance in `0.0001 cd/m²` units.
    pub max_luminance: u32,
    /// Minimum luminance in `0.0001 cd/m²` units.
    pub min_luminance: u32,
}

impl MasteringDisplay {
    /// Validate every field against the ST 2086-style bounds.
    pub fn new(
        display_primaries: [(u16, u16); 3],
        white_point: (u16, u16),
        max_luminance: u32,
        min_luminance: u32,
    ) -> Result<Self, VoleError> {
        let check = |v: (u16, u16)| {
            u32::from(v.0) <= MAX_CHROMATICITY && u32::from(v.1) <= MAX_CHROMATICITY
        };
        if !display_primaries.iter().copied().all(check) || !check(white_point) {
            return Err(VoleError::InvalidSamples);
        }
        if max_luminance > MAX_LUMINANCE_UNITS || min_luminance > MAX_LUMINANCE_UNITS {
            return Err(VoleError::InvalidSamples);
        }
        if max_luminance < min_luminance {
            return Err(VoleError::InvalidSamples);
        }
        Ok(MasteringDisplay {
            display_primaries,
            white_point,
            max_luminance,
            min_luminance,
        })
    }
}

/// CEA-861.3 content light level (static HDR metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentLightLevel {
    /// Maximum content light level in cd/m².
    pub max_cll: u16,
    /// Maximum frame-average light level in cd/m².
    pub max_fall: u16,
}

/// Static HDR metadata attached to an epoch (present when the source signals
/// it; stored exactly, never derived).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HdrMetadata {
    /// Mastering-display colour volume.
    pub mastering_display: Option<MasteringDisplay>,
    /// Content light level.
    pub content_light_level: Option<ContentLightLevel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_descriptions_are_exact_signaling_sets() {
        let pq = ColorDescription::bt2020_pq();
        assert_eq!(pq.primaries(), ColorPrimaries::Bt2020);
        assert_eq!(pq.transfer(), TransferCharacteristic::Smpte2084);
        assert_eq!(pq.matrix(), MatrixCoefficients::Bt2020Ncl);
        assert_eq!(pq.range(), ColorRange::Limited);
        assert!(!pq.has_unspecified());
        assert_eq!(pq.describe(), "bt2020/pq/bt2020ncl/limited/center");
        assert!(ColorDescription::unspecified().has_unspecified());
        assert!(ColorDescription::bt601_sd().primaries() == ColorPrimaries::Smpte170M);
        assert!(ColorDescription::rgb_full().matrix().is_identity());
        assert_eq!(
            ColorDescription::bt2020_hlg().transfer(),
            TransferCharacteristic::AribStdB67
        );
    }

    #[test]
    fn hdr_metadata_is_bounded_and_typed() {
        let ok = MasteringDisplay::new(
            [(0, 0), (50000, 0), (0, 50000)],
            (15635, 16450),
            10_000_000,
            50,
        )
        .unwrap();
        assert_eq!(ok.max_luminance, 10_000_000);
        // Chromaticity beyond 50 000 is refused.
        assert!(MasteringDisplay::new([(50001, 0), (0, 0), (0, 0)], (0, 0), 1, 0).is_err());
        // max < min luminance is refused.
        assert!(MasteringDisplay::new([(0, 0), (0, 0), (0, 0)], (0, 0), 1, 2).is_err());
        // Luminance beyond the declared cap is refused.
        assert!(MasteringDisplay::new(
            [(0, 0), (0, 0), (0, 0)],
            (0, 0),
            MAX_LUMINANCE_UNITS + 1,
            0
        )
        .is_err());
        let m = HdrMetadata {
            mastering_display: Some(ok),
            content_light_level: Some(ContentLightLevel {
                max_cll: 1000,
                max_fall: 400,
            }),
        };
        assert_eq!(m.mastering_display.unwrap().white_point, (15635, 16450));
        assert_eq!(m.content_light_level.unwrap().max_cll, 1000);
    }
}
