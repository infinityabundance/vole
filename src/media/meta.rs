//! Picture interpretation metadata — Phase V.1.1 (V.1 brief §26–§29,
//! contract §2.4).
//!
//! Orientation, sample aspect ratio, and field structure are **stored
//! interpretation, never baked into samples**: import does not rotate, scale,
//! or deinterlace (§26–§28). Presentation applies them (V.1.14+); export
//! restores them when the target container supports it. Visual side data is a
//! bounded registry of [`VisualSideDataKind`]s with explicit classification —
//! typed data is executable semantics, opaque data is preserved bounded and
//! inert, and anything unsupported fails closed rather than being guessed.

use crate::error::VoleError;

/// Display orientation of the coded samples (presentation applies it; stored
/// canonical samples are never rotated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Orientation {
    /// No rotation or flip.
    Normal,
    /// Rotate 90° clockwise for display.
    Rotate90,
    /// Rotate 180° for display.
    Rotate180,
    /// Rotate 270° clockwise for display.
    Rotate270,
    /// Mirror horizontally for display.
    FlipHorizontal,
    /// Mirror vertically for display.
    FlipVertical,
}

impl Orientation {
    /// Whether display differs from the coded orientation.
    pub fn is_identity(self) -> bool {
        self == Orientation::Normal
    }

    /// Stable label.
    pub fn label(self) -> &'static str {
        match self {
            Orientation::Normal => "normal",
            Orientation::Rotate90 => "rotate90",
            Orientation::Rotate180 => "rotate180",
            Orientation::Rotate270 => "rotate270",
            Orientation::FlipHorizontal => "flip-horizontal",
            Orientation::FlipVertical => "flip-vertical",
        }
    }
}

/// Sample aspect ratio: the intended shape of one coded sample, `width /
/// height` (nonzero). Stored video keeps coded dimensions; presentation may
/// scale to `display aspect = coded aspect × SAR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleAspectRatio {
    width: u32,
    height: u32,
}

impl SampleAspectRatio {
    /// A SAR of `width / height` (both nonzero). `1/1` is square samples.
    pub fn new(width: u32, height: u32) -> Result<Self, VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::InvalidTimeBase);
        }
        Ok(SampleAspectRatio { width, height })
    }

    /// Square samples.
    pub const fn square() -> Self {
        SampleAspectRatio {
            width: 1,
            height: 1,
        }
    }

    /// Width part.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height part.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whether samples are square.
    pub fn is_square(&self) -> bool {
        self.width == self.height
    }

    /// The exact display aspect ratio of a `w × h` coded picture as a
    /// reduced rational `(num, den)` (checked).
    pub fn display_aspect(&self, width: u32, height: u32) -> Result<(u32, u32), VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::GeometryMismatch);
        }
        let num = u128::from(width) * u128::from(self.width);
        let den = u128::from(height) * u128::from(self.height);
        let g = gcd128(num, den);
        let (num, den) = ((num / g) as u32, (den / g) as u32);
        if num == 0 || den == 0 {
            return Err(VoleError::ArithmeticOverflow);
        }
        Ok((num, den))
    }
}

fn gcd128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Field structure of the coded samples. Import never deinterlaces (§28);
/// optional bob/weave/deinterlace is presentation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldStructure {
    /// Not declared.
    Unknown,
    /// Progressive (no fields).
    Progressive,
    /// Interlaced, top field first.
    InterlacedTopFieldFirst,
    /// Interlaced, bottom field first.
    InterlacedBottomFieldFirst,
}

impl FieldStructure {
    /// Whether the content is interlaced.
    pub fn is_interlaced(self) -> bool {
        matches!(
            self,
            FieldStructure::InterlacedTopFieldFirst | FieldStructure::InterlacedBottomFieldFirst
        )
    }

    /// Whether the structure is declared at all.
    pub fn is_unknown(self) -> bool {
        self == FieldStructure::Unknown
    }

    /// Stable label.
    pub fn label(self) -> &'static str {
        match self {
            FieldStructure::Unknown => "unknown",
            FieldStructure::Progressive => "progressive",
            FieldStructure::InterlacedTopFieldFirst => "interlaced-tff",
            FieldStructure::InterlacedBottomFieldFirst => "interlaced-bff",
        }
    }
}

/// Classification of a visual side-data kind (brief §29): typed data is
/// executable semantics VOLE understands; opaque data is preserved bounded
/// and inert; unsupported data is refused when mandatory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualSideDataKind {
    /// SMPTE ST 2086 mastering-display colour volume (typed).
    MasteringDisplay,
    /// CEA-861.3 content light level (typed).
    ContentLightLevel,
    /// SMPTE 12M timecode (typed container; payload semantics preserved).
    Timecode,
    /// Reserved/unknown id space (never executed).
    Other(u32),
}

impl VisualSideDataKind {
    /// Registry classification.
    pub fn classification(self) -> SideDataClass {
        match self {
            VisualSideDataKind::MasteringDisplay
            | VisualSideDataKind::ContentLightLevel
            | VisualSideDataKind::Timecode => SideDataClass::KnownTyped,
            VisualSideDataKind::Other(_) => SideDataClass::Unsupported,
        }
    }

    /// Stable label for receipts.
    pub fn label(self) -> &'static str {
        match self {
            VisualSideDataKind::MasteringDisplay => "mastering-display",
            VisualSideDataKind::ContentLightLevel => "content-light-level",
            VisualSideDataKind::Timecode => "timecode",
            VisualSideDataKind::Other(_) => "other",
        }
    }
}

/// The classification of a side-data kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideDataClass {
    /// VOLE parses and preserves the semantics exactly.
    KnownTyped,
    /// Bounded, preserved verbatim, never executed.
    OpaquePreserved,
    /// Not executable: a stream declaring this as mandatory fails closed.
    Unsupported,
}

/// Bound on one opaque side-data payload (bounded registry, §29).
pub const MAX_OPAQUE_SIDE_DATA: usize = 1 << 20;

/// One piece of visual side data attached to an epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualSideData {
    /// Fully typed mastering-display volume.
    MasteringDisplay(crate::media::color::MasteringDisplay),
    /// Fully typed content light level.
    ContentLightLevel(crate::media::color::ContentLightLevel),
    /// Bounded opaque payload preserved verbatim under a declared kind id.
    Opaque {
        /// Declared kind id.
        kind: u32,
        /// Bounded payload (≤ [`MAX_OPAQUE_SIDE_DATA`] bytes).
        payload: Vec<u8>,
    },
}

impl VisualSideData {
    /// Wrap a bounded opaque payload. Oversized payloads are a typed error
    /// (the registry stays bounded).
    pub fn opaque(kind: u32, payload: Vec<u8>) -> Result<Self, VoleError> {
        if payload.len() > MAX_OPAQUE_SIDE_DATA {
            return Err(VoleError::DimensionTooLarge);
        }
        Ok(VisualSideData::Opaque { kind, payload })
    }

    /// The registry kind of this entry.
    pub fn kind(&self) -> VisualSideDataKind {
        match self {
            VisualSideData::MasteringDisplay(_) => VisualSideDataKind::MasteringDisplay,
            VisualSideData::ContentLightLevel(_) => VisualSideDataKind::ContentLightLevel,
            VisualSideData::Opaque { kind, .. } => VisualSideDataKind::Other(*kind),
        }
    }

    /// Registry classification.
    pub fn classification(&self) -> SideDataClass {
        self.kind().classification()
    }

    /// Whether this entry carries executable VOLE semantics.
    pub fn is_typed(&self) -> bool {
        matches!(
            self,
            VisualSideData::MasteringDisplay(_) | VisualSideData::ContentLightLevel(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::color::ContentLightLevel;

    #[test]
    fn sar_and_display_aspect_are_exact() {
        let sar = SampleAspectRatio::new(4, 3).unwrap();
        assert!(!sar.is_square());
        assert_eq!(sar.display_aspect(1920, 1080).unwrap(), (64, 27));
        assert!(SampleAspectRatio::new(0, 1).is_err());
        assert!(SampleAspectRatio::new(1, 0).is_err());
        assert!(SampleAspectRatio::square().is_square());
    }

    #[test]
    fn orientation_and_field_structure_are_interpretation_only() {
        assert!(Orientation::Normal.is_identity());
        assert!(!Orientation::Rotate90.is_identity());
        assert!(!FieldStructure::Progressive.is_interlaced());
        assert!(FieldStructure::InterlacedTopFieldFirst.is_interlaced());
        assert_eq!(
            FieldStructure::InterlacedTopFieldFirst.label(),
            "interlaced-tff"
        );
    }

    #[test]
    fn side_data_registry_is_bounded_and_classified() {
        let md = VisualSideData::MasteringDisplay(
            crate::media::color::MasteringDisplay::new(
                [(0, 0), (50000, 0), (0, 50000)],
                (15635, 16450),
                10_000_000,
                50,
            )
            .unwrap(),
        );
        let cll = VisualSideData::ContentLightLevel(ContentLightLevel {
            max_cll: 1000,
            max_fall: 400,
        });
        assert!(md.is_typed());
        assert_eq!(md.kind(), VisualSideDataKind::MasteringDisplay);
        assert_eq!(md.classification(), SideDataClass::KnownTyped);
        assert_eq!(cll.kind(), VisualSideDataKind::ContentLightLevel);
        // Opaque payloads are bounded.
        assert!(VisualSideData::opaque(0xABCD, vec![0; 5]).is_ok());
        assert!(VisualSideData::opaque(0xABCD, vec![0; MAX_OPAQUE_SIDE_DATA + 1]).is_err());
        let op = VisualSideData::opaque(0xABCD, vec![1, 2, 3]).unwrap();
        assert_eq!(op.kind(), VisualSideDataKind::Other(0xABCD));
        assert_eq!(op.classification(), SideDataClass::Unsupported);
        assert!(!op.is_typed());
    }
}
