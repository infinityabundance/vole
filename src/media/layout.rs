//! Component and pixel-layout registry — Phase V.1.1 (V.1 brief §14, §18–§20;
//! contract §2.4).
//!
//! Samples are named by [`Component`]; a [`PixelLayout`] is a canonical,
//! planar, tightly packed arrangement of components with a declared
//! subsampling factor per plane. **Subsampling is geometry**: a plane whose
//! axes are subsampled by `sx`/`sy` covers `ceil(w / 2^sx) × ceil(h / 2^sy)`
//! samples of a `w × h` coded picture (the ceil rule is normative and courted
//! on odd dimensions). Packed or interleaved *source* formats are not stored
//! as such — they are declared in [`PackedSourceLayout`] and reversibly
//! unpacked to a canonical [`PixelLayout`] at import (V.1.3).

use crate::error::VoleError;

/// A named sample component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Component {
    /// Luma of a YUV family picture (also the single plane of YUV 4:0:0).
    Y,
    /// Blue-difference chroma.
    Cb,
    /// Red-difference chroma.
    Cr,
    /// Red of an RGB/GBR family picture.
    R,
    /// Green of an RGB/GBR family picture.
    G,
    /// Blue of an RGB/GBR family picture.
    B,
    /// Alpha (an independent exact sample component; compositing is
    /// presentation policy).
    A,
    /// Monochrome (canonical Gray pictures).
    Gray,
    /// Palette index plane (PAL8-class content; rendered only through a
    /// declared palette — Phase J semantics generalized). Reserved id space
    /// for future components.
    Index,
    /// Reserved/other component id (fail-closed: unknown ids never get a
    /// guessed interpretation).
    Other(u16),
}

impl Component {
    /// Stable label for receipts and diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Component::Y => "Y",
            Component::Cb => "Cb",
            Component::Cr => "Cr",
            Component::R => "R",
            Component::G => "G",
            Component::B => "B",
            Component::A => "A",
            Component::Gray => "Gray",
            Component::Index => "Index",
            Component::Other(_) => "Other",
        }
    }

    /// Whether this component is one of the luma-like planes (Y or Gray).
    pub fn is_luma(self) -> bool {
        matches!(self, Component::Y | Component::Gray)
    }

    /// Whether this component is chroma (Cb/Cr).
    pub fn is_chroma(self) -> bool {
        matches!(self, Component::Cb | Component::Cr)
    }

    /// Whether this component is the alpha plane.
    pub fn is_alpha(self) -> bool {
        self == Component::A
    }
}

/// One plane of a canonical layout: its component and subsampling exponents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneTemplate {
    /// The component carried by this plane.
    pub component: Component,
    /// Horizontal subsampling exponent (plane width = `ceil(w / 2^sx)`).
    pub subsample_x: u8,
    /// Vertical subsampling exponent (plane height = `ceil(h / 2^sy)`).
    pub subsample_y: u8,
}

const fn t(c: Component, sx: u8, sy: u8) -> PlaneTemplate {
    PlaneTemplate {
        component: c,
        subsample_x: sx,
        subsample_y: sy,
    }
}

/// Canonical planar pixel layouts stored by VOLE (the V.1.1 registry).
///
/// Every layout is planar by construction; packed source forms are unpacked
/// at import. Depth is *not* part of the layout (a separate [`BitDepth`]):
/// `Yuv420` at 8 and at 10 bits are the same layout with different depths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelLayout {
    /// Single monochrome plane (`Gray`, full resolution).
    Gray,
    /// YUV 4:0:0 — single luma plane (`Y`, full resolution). Distinct from
    /// [`PixelLayout::Gray`] only in component naming and color semantics.
    Yuv400,
    /// YUV 4:2:0 — `Y` full, `Cb`/`Cr` subsampled by 2 on both axes.
    Yuv420,
    /// YUV 4:2:2 — `Y` full, `Cb`/`Cr` subsampled by 2 horizontally only.
    Yuv422,
    /// YUV 4:4:4 — `Y`, `Cb`, `Cr` all full resolution.
    Yuv444,
    /// YUVA 4:2:0 — `Yuv420` plus full-resolution alpha.
    Yuva420,
    /// YUVA 4:4:4 — `Yuv444` plus full-resolution alpha.
    Yuva444,
    /// Planar RGB (`R`, `G`, `B`, full resolution).
    Gbr,
    /// Planar RGB plus alpha (`R`, `G`, `B`, `A`).
    Gbra,
    /// Packed RGB unpacked to canonical `R`, `G`, `B` planes.
    Rgb,
    /// Packed BGR unpacked to canonical `B`, `G`, `R` planes (plane order
    /// preserves the source byte order of the packed form).
    Bgr,
    /// Packed RGBA unpacked to canonical `R`, `G`, `B`, `A` planes.
    Rgba,
    /// Packed BGRA unpacked to canonical `B`, `G`, `R`, `A` planes.
    Bgra,
    /// Packed ARGB unpacked to canonical `A`, `R`, `G`, `B` planes.
    Argb,
    /// Packed ABGR unpacked to canonical `A`, `B`, `G`, `R` planes.
    Abgr,
    /// A single palette-`Index` plane (PAL8-class content); rendered only
    /// through a declared palette.
    Indexed,
}

/// The subsampling exponents must be small (a factor of `2^sx` per axis).
const MAX_SUBSAMPLE: u8 = 3;

// Static plane tables (const-initialized so `planes()` returns `&'static`).
const P_GRAY: [PlaneTemplate; 1] = [t(Component::Gray, 0, 0)];
const P_YUV400: [PlaneTemplate; 1] = [t(Component::Y, 0, 0)];
const P_YUV420: [PlaneTemplate; 3] = [
    t(Component::Y, 0, 0),
    t(Component::Cb, 1, 1),
    t(Component::Cr, 1, 1),
];
const P_YUV422: [PlaneTemplate; 3] = [
    t(Component::Y, 0, 0),
    t(Component::Cb, 1, 0),
    t(Component::Cr, 1, 0),
];
const P_YUV444: [PlaneTemplate; 3] = [
    t(Component::Y, 0, 0),
    t(Component::Cb, 0, 0),
    t(Component::Cr, 0, 0),
];
const P_YUVA420: [PlaneTemplate; 4] = [
    t(Component::Y, 0, 0),
    t(Component::Cb, 1, 1),
    t(Component::Cr, 1, 1),
    t(Component::A, 0, 0),
];
const P_YUVA444: [PlaneTemplate; 4] = [
    t(Component::Y, 0, 0),
    t(Component::Cb, 0, 0),
    t(Component::Cr, 0, 0),
    t(Component::A, 0, 0),
];
const P_GBR: [PlaneTemplate; 3] = [
    t(Component::R, 0, 0),
    t(Component::G, 0, 0),
    t(Component::B, 0, 0),
];
const P_GBRA: [PlaneTemplate; 4] = [
    t(Component::R, 0, 0),
    t(Component::G, 0, 0),
    t(Component::B, 0, 0),
    t(Component::A, 0, 0),
];
const P_RGB: [PlaneTemplate; 3] = [
    t(Component::R, 0, 0),
    t(Component::G, 0, 0),
    t(Component::B, 0, 0),
];
const P_BGR: [PlaneTemplate; 3] = [
    t(Component::B, 0, 0),
    t(Component::G, 0, 0),
    t(Component::R, 0, 0),
];
const P_RGBA: [PlaneTemplate; 4] = [
    t(Component::R, 0, 0),
    t(Component::G, 0, 0),
    t(Component::B, 0, 0),
    t(Component::A, 0, 0),
];
const P_BGRA: [PlaneTemplate; 4] = [
    t(Component::B, 0, 0),
    t(Component::G, 0, 0),
    t(Component::R, 0, 0),
    t(Component::A, 0, 0),
];
const P_ARGB: [PlaneTemplate; 4] = [
    t(Component::A, 0, 0),
    t(Component::R, 0, 0),
    t(Component::G, 0, 0),
    t(Component::B, 0, 0),
];
const P_ABGR: [PlaneTemplate; 4] = [
    t(Component::A, 0, 0),
    t(Component::B, 0, 0),
    t(Component::G, 0, 0),
    t(Component::R, 0, 0),
];
const P_INDEXED: [PlaneTemplate; 1] = [t(Component::Index, 0, 0)];

impl PixelLayout {
    /// The ordered plane templates of this canonical layout.
    pub fn planes(self) -> &'static [PlaneTemplate] {
        match self {
            PixelLayout::Gray => &P_GRAY,
            PixelLayout::Yuv400 => &P_YUV400,
            PixelLayout::Yuv420 => &P_YUV420,
            PixelLayout::Yuv422 => &P_YUV422,
            PixelLayout::Yuv444 => &P_YUV444,
            PixelLayout::Yuva420 => &P_YUVA420,
            PixelLayout::Yuva444 => &P_YUVA444,
            PixelLayout::Gbr => &P_GBR,
            PixelLayout::Gbra => &P_GBRA,
            PixelLayout::Rgb => &P_RGB,
            PixelLayout::Bgr => &P_BGR,
            PixelLayout::Rgba => &P_RGBA,
            PixelLayout::Bgra => &P_BGRA,
            PixelLayout::Argb => &P_ARGB,
            PixelLayout::Abgr => &P_ABGR,
            PixelLayout::Indexed => &P_INDEXED,
        }
    }

    /// Number of planes.
    pub fn plane_count(self) -> usize {
        self.planes().len()
    }

    /// The canonical subsampling geometry of plane `i` of a `width × height`
    /// coded picture: the **ceil rule** `ceil(n / 2^s)` per subsampled axis is
    /// normative (every coded sample is covered by a sample of each plane).
    /// `i` out of range is a typed geometry error.
    pub fn plane_dimensions(
        self,
        i: usize,
        width: u32,
        height: u32,
    ) -> Result<(u32, u32), VoleError> {
        let tmpl = self.planes().get(i).ok_or(VoleError::GeometryMismatch)?;
        let pw = subsample_len(width, tmpl.subsample_x)?;
        let ph = subsample_len(height, tmpl.subsample_y)?;
        if pw == 0 || ph == 0 {
            return Err(VoleError::GeometryMismatch);
        }
        Ok((pw, ph))
    }

    /// Sample count of plane `i` (`ceil` rule applied per axis).
    pub fn plane_sample_count(self, i: usize, width: u32, height: u32) -> Result<u64, VoleError> {
        let (pw, ph) = self.plane_dimensions(i, width, height)?;
        u64::from(pw)
            .checked_mul(u64::from(ph))
            .ok_or(VoleError::ArithmeticOverflow)
    }

    /// Total sample count over every plane of a `width × height` picture.
    pub fn total_sample_count(self, width: u32, height: u32) -> Result<u64, VoleError> {
        let mut total = 0u64;
        for i in 0..self.plane_count() {
            total = total
                .checked_add(self.plane_sample_count(i, width, height)?)
                .ok_or(VoleError::ArithmeticOverflow)?;
        }
        Ok(total)
    }

    /// Stable label for receipts and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            PixelLayout::Gray => "gray",
            PixelLayout::Yuv400 => "yuv400",
            PixelLayout::Yuv420 => "yuv420",
            PixelLayout::Yuv422 => "yuv422",
            PixelLayout::Yuv444 => "yuv444",
            PixelLayout::Yuva420 => "yuva420",
            PixelLayout::Yuva444 => "yuva444",
            PixelLayout::Gbr => "gbr",
            PixelLayout::Gbra => "gbra",
            PixelLayout::Rgb => "rgb",
            PixelLayout::Bgr => "bgr",
            PixelLayout::Rgba => "rgba",
            PixelLayout::Bgra => "bgra",
            PixelLayout::Argb => "argb",
            PixelLayout::Abgr => "abgr",
            PixelLayout::Indexed => "indexed",
        }
    }
}

/// `ceil(n / 2^s)` with `s ≤ MAX_SUBSAMPLE`; zero input is an error.
fn subsample_len(n: u32, s: u8) -> Result<u32, VoleError> {
    if n == 0 || s > MAX_SUBSAMPLE {
        return Err(VoleError::GeometryMismatch);
    }
    let shift = 1u32 << s;
    Ok(n.saturating_add(shift - 1) / shift)
}

/// Packed / interleaved **source** sample formats (foreign decoders and raw
/// captures). Never stored as-is: each maps to a canonical [`PixelLayout`]
/// that import (V.1.3) unpacks into. The mapping is total and deterministic;
/// depth is carried separately (e.g. `P010` is 10-bit `NV12`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackedSourceLayout {
    /// One luma plane + one interleaved `Cb, Cr` plane (4:2:0).
    Nv12,
    /// One luma plane + one interleaved `Cr, Cb` plane (4:2:0).
    Nv21,
    /// 10-bit `NV12` (samples in the high bits of `u16` words).
    P010,
    /// 12/16-bit `NV12` (samples in the high bits of `u16` words).
    P016,
    /// Interleaved `Y, Cb, Y, Cr` (4:2:2).
    Yuyv422,
    /// Interleaved `Cb, Y, Cr, Y` (4:2:2).
    Uyvy422,
    /// 8-bit palette-indexed.
    Pal8,
}

impl PackedSourceLayout {
    /// The canonical planar layout this packed form unpacks to (V.1.3).
    pub fn canonical_target(self) -> PixelLayout {
        match self {
            PackedSourceLayout::Nv12 | PackedSourceLayout::Nv21 => PixelLayout::Yuv420,
            PackedSourceLayout::P010 | PackedSourceLayout::P016 => PixelLayout::Yuv420,
            PackedSourceLayout::Yuyv422 | PackedSourceLayout::Uyvy422 => PixelLayout::Yuv422,
            PackedSourceLayout::Pal8 => PixelLayout::Indexed,
        }
    }

    /// Stable label for receipts and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            PackedSourceLayout::Nv12 => "nv12",
            PackedSourceLayout::Nv21 => "nv21",
            PackedSourceLayout::P010 => "p010",
            PackedSourceLayout::P016 => "p016",
            PackedSourceLayout::Yuyv422 => "yuyv422",
            PackedSourceLayout::Uyvy422 => "uyvy422",
            PackedSourceLayout::Pal8 => "pal8",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_geometry_is_the_normative_ceil_rule() {
        // Even dimensions halve exactly for 4:2:0.
        let l = PixelLayout::Yuv420;
        assert_eq!(l.plane_count(), 3);
        assert_eq!(l.plane_dimensions(0, 1920, 1080).unwrap(), (1920, 1080));
        assert_eq!(l.plane_dimensions(1, 1920, 1080).unwrap(), (960, 540));
        assert_eq!(l.plane_dimensions(2, 1920, 1080).unwrap(), (960, 540));
        // Odd dimensions use ceil — court values from the contract §2.4.
        for (w, h) in [(1u32, 1u32), (3, 3), (1919, 1079), (1921, 1081)] {
            let (cw, ch) = l.plane_dimensions(1, w, h).unwrap();
            assert_eq!(cw, w.div_ceil(2), "chroma width ceil at {w}x{h}");
            assert_eq!(ch, h.div_ceil(2), "chroma height ceil at {w}x{h}");
        }
        // 4:2:2 subsamples horizontally only.
        let l = PixelLayout::Yuv422;
        assert_eq!(l.plane_dimensions(1, 1919, 1079).unwrap(), (960, 1079));
        // 4:4:4 and RGB/Gray families are full resolution.
        for l in [
            PixelLayout::Yuv444,
            PixelLayout::Gbr,
            PixelLayout::Rgb,
            PixelLayout::Gray,
        ] {
            for i in 0..l.plane_count() {
                assert_eq!(l.plane_dimensions(i, 1919, 1079).unwrap(), (1919, 1079));
            }
        }
        // Total sample counts are checked and exact.
        assert_eq!(
            PixelLayout::Yuva420.total_sample_count(3, 3).unwrap(),
            9 + 4 + 4 + 9
        );
    }

    #[test]
    fn packed_layouts_have_total_canonical_targets() {
        for p in [
            PackedSourceLayout::Nv12,
            PackedSourceLayout::Nv21,
            PackedSourceLayout::P010,
            PackedSourceLayout::P016,
        ] {
            assert_eq!(p.canonical_target(), PixelLayout::Yuv420);
        }
        assert_eq!(
            PackedSourceLayout::Yuyv422.canonical_target(),
            PixelLayout::Yuv422
        );
        assert_eq!(
            PackedSourceLayout::Uyvy422.canonical_target(),
            PixelLayout::Yuv422
        );
        assert_eq!(
            PackedSourceLayout::Pal8.canonical_target(),
            PixelLayout::Indexed
        );
        assert_eq!(PixelLayout::Bgr.planes()[0].component, Component::B);
        assert_eq!(PixelLayout::Argb.planes()[0].component, Component::A);
        assert_eq!(PixelLayout::Indexed.planes()[0].component, Component::Index);
    }

    #[test]
    fn component_labels_are_stable() {
        assert_eq!(Component::Y.label(), "Y");
        assert!(Component::Y.is_luma());
        assert!(Component::Gray.is_luma());
        assert!(Component::Cb.is_chroma());
        assert!(Component::A.is_alpha());
        assert_eq!(PixelLayout::Yuv420.label(), "yuv420");
    }
}
