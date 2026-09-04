//! Source-format canonicalizer — Phase V.1.3 (V.1 video programme, brief
//! §17–§18, §45–§46).
//!
//! Maps FFmpeg pixel formats to VOLE's canonical planar domain and
//! **reversibly unpacks** decoded frames into canonical planes:
//!
//! * planar formats whose payload is already tight canonical bytes map
//!   plane-for-plane (with declared reordering where FFmpeg's plane order
//!   differs, e.g. `gbrp` = G,B,R vs canonical R,G,B);
//! * semi-planar NV12/NV21/P010/P016 interleaved chroma is de-interleaved;
//! * packed 4:2:2 (YUYV422/UYVY422, even widths) and 8-bit packed RGB
//!   families are de-interleaved into canonical planes.
//!
//! Everything is reversible: [`repack_frame`] restores the exact source
//! payload bytes, which is how the framehash oracle proof runs for packed
//! formats (the oracle digests FFmpeg's own layout; VOLE proves byte-exact
//! re-packing, so the digest agreement is meaningful). Stride padding is
//! never preserved (§18); unsupported formats fail closed typed
//! (`UnsupportedPixelLayout`) — never a silent conversion.

use crate::error::VoleError;
use crate::media::layout::{Component, PixelLayout};
use crate::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};

/// The description of one supported source format.
#[derive(Debug, Clone, Copy)]
struct Desc {
    name: &'static str,
    layout: PixelLayout,
    depth: u8,
    kind: Kind,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    /// Payload plane order by component (a permutation of the canonical
    /// template components; tight rows).
    Planar,
    /// NV12/NV21-style semi-planar chroma (interleaved U/V sample pairs).
    SemiPlanar { uv_first: Component },
    /// 8-bit packed: one plane, `stride` bytes per pixel; canonical plane
    /// `p` holds byte `first + p` of each pixel.
    Packed { stride: u8, first: u8 },
    /// 8-bit packed 4:2:2 (even widths only).
    Yuyv,
    /// 8-bit packed 4:2:2 uyvy (even widths only).
    Uyvy,
}

const fn d(name: &'static str, layout: PixelLayout, depth: u8, kind: Kind) -> Desc {
    Desc {
        name,
        layout,
        depth,
        kind,
    }
}

use Component::*;

/// Supported source pixel formats (courted subset; unknown formats fail
/// closed typed — never silently converted).
pub static SUPPORTED: &[&str] = &[
    "gray",
    "gray16le",
    "yuv420p",
    "yuv420p10le",
    "yuv420p12le",
    "yuv420p14le",
    "yuv420p16le",
    "yuv422p",
    "yuv422p10le",
    "yuv422p12le",
    "yuv422p16le",
    "yuv444p",
    "yuv444p10le",
    "yuv444p12le",
    "yuv444p16le",
    "yuva420p",
    "yuva444p",
    "yuva444p10le",
    "yuva444p12le",
    "yuva444p16le",
    "gbrp",
    "gbrp10le",
    "gbrp12le",
    "gbrp16le",
    "gbrap",
    "gbrap10le",
    "nv12",
    "nv21",
    "p010le",
    "p016le",
    "rgb24",
    "bgr24",
    "rgb0",
    "bgr0",
    "0rgb",
    "0bgr",
    "rgba",
    "bgra",
    "argb",
    "abgr",
    "yuyv422",
    "uyvy422",
];

const TABLE: &[Desc] = &[
    d("gray", PixelLayout::Gray, 8, Kind::Planar),
    d("gray16le", PixelLayout::Gray, 16, Kind::Planar),
    d("yuv420p", PixelLayout::Yuv420, 8, Kind::Planar),
    d("yuv420p10le", PixelLayout::Yuv420, 10, Kind::Planar),
    d("yuv420p12le", PixelLayout::Yuv420, 12, Kind::Planar),
    d("yuv420p14le", PixelLayout::Yuv420, 14, Kind::Planar),
    d("yuv420p16le", PixelLayout::Yuv420, 16, Kind::Planar),
    d("yuv422p", PixelLayout::Yuv422, 8, Kind::Planar),
    d("yuv422p10le", PixelLayout::Yuv422, 10, Kind::Planar),
    d("yuv422p12le", PixelLayout::Yuv422, 12, Kind::Planar),
    d("yuv422p16le", PixelLayout::Yuv422, 16, Kind::Planar),
    d("yuv444p", PixelLayout::Yuv444, 8, Kind::Planar),
    d("yuv444p10le", PixelLayout::Yuv444, 10, Kind::Planar),
    d("yuv444p12le", PixelLayout::Yuv444, 12, Kind::Planar),
    d("yuv444p16le", PixelLayout::Yuv444, 16, Kind::Planar),
    d("yuva420p", PixelLayout::Yuva420, 8, Kind::Planar),
    d("yuva444p", PixelLayout::Yuva444, 8, Kind::Planar),
    d("yuva444p10le", PixelLayout::Yuva444, 10, Kind::Planar),
    d("yuva444p12le", PixelLayout::Yuva444, 12, Kind::Planar),
    d("yuva444p16le", PixelLayout::Yuva444, 16, Kind::Planar),
    // gbrp/gbrap payload plane order is G,B,R(,A); canonical is R,G,B(,A).
    d("gbrp", PixelLayout::Gbr, 8, Kind::Planar),
    d("gbrp10le", PixelLayout::Gbr, 10, Kind::Planar),
    d("gbrp12le", PixelLayout::Gbr, 12, Kind::Planar),
    d("gbrp16le", PixelLayout::Gbr, 16, Kind::Planar),
    d("gbrap", PixelLayout::Gbra, 8, Kind::Planar),
    d("gbrap10le", PixelLayout::Gbra, 10, Kind::Planar),
    d(
        "nv12",
        PixelLayout::Yuv420,
        8,
        Kind::SemiPlanar { uv_first: Cb },
    ),
    d(
        "nv21",
        PixelLayout::Yuv420,
        8,
        Kind::SemiPlanar { uv_first: Cr },
    ),
    d(
        "p010le",
        PixelLayout::Yuv420,
        10,
        Kind::SemiPlanar { uv_first: Cb },
    ),
    d(
        "p016le",
        PixelLayout::Yuv420,
        16,
        Kind::SemiPlanar { uv_first: Cb },
    ),
    d(
        "rgb24",
        PixelLayout::Rgb,
        8,
        Kind::Packed {
            stride: 3,
            first: 0,
        },
    ),
    d(
        "bgr24",
        PixelLayout::Bgr,
        8,
        Kind::Packed {
            stride: 3,
            first: 0,
        },
    ),
    d(
        "rgb0",
        PixelLayout::Rgb,
        8,
        Kind::Packed {
            stride: 4,
            first: 0,
        },
    ),
    d(
        "bgr0",
        PixelLayout::Bgr,
        8,
        Kind::Packed {
            stride: 4,
            first: 0,
        },
    ),
    d(
        "0rgb",
        PixelLayout::Rgb,
        8,
        Kind::Packed {
            stride: 4,
            first: 1,
        },
    ),
    d(
        "0bgr",
        PixelLayout::Bgr,
        8,
        Kind::Packed {
            stride: 4,
            first: 1,
        },
    ),
    d(
        "rgba",
        PixelLayout::Rgba,
        8,
        Kind::Packed {
            stride: 4,
            first: 0,
        },
    ),
    d(
        "bgra",
        PixelLayout::Bgra,
        8,
        Kind::Packed {
            stride: 4,
            first: 0,
        },
    ),
    d(
        "argb",
        PixelLayout::Argb,
        8,
        Kind::Packed {
            stride: 4,
            first: 0,
        },
    ),
    d(
        "abgr",
        PixelLayout::Abgr,
        8,
        Kind::Packed {
            stride: 4,
            first: 0,
        },
    ),
    d("yuyv422", PixelLayout::Yuv422, 8, Kind::Yuyv),
    d("uyvy422", PixelLayout::Yuv422, 8, Kind::Uyvy),
];

fn desc(name: &str) -> Result<&'static Desc, VoleError> {
    TABLE
        .iter()
        .find(|d| d.name == name)
        .ok_or(VoleError::UnsupportedPixelLayout)
}

/// The canonical layout + bit depth a source pixel format unpacks to.
pub fn layout_and_depth(name: &str) -> Result<(PixelLayout, u8), VoleError> {
    let d = desc(name)?;
    Ok((d.layout, d.depth))
}

/// How a source format's payload relates to the canonical bytes: formats
/// whose payload *is* the canonical bytes (`Canonical`) versus formats whose
/// payload needs reversible packing (`SourceRepacked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarrierKind {
    /// The source payload is already the canonical tight planar bytes.
    Canonical,
    /// The source payload packs the canonical planes (packed / semi-planar /
    /// reordered formats); byte-exactness is proven through repacking.
    SourceRepacked,
}

/// The carrier class of a supported source pixel format.
pub fn layout_kind(name: &str) -> Result<CarrierKind, VoleError> {
    let d = desc(name)?;
    Ok(match d.kind {
        Kind::Planar => CarrierKind::Canonical,
        Kind::SemiPlanar { .. } | Kind::Packed { .. } | Kind::Yuyv | Kind::Uyvy => {
            CarrierKind::SourceRepacked
        }
    })
}

fn bps(depth: u8) -> u64 {
    if depth <= 8 {
        1
    } else {
        2
    }
}

fn ceil_div(n: u64, s: u8) -> u64 {
    (n + (1u64 << s) - 1) >> s
}

/// The canonical sample dims of template position `p` of the layout at coded
/// `(w, h)`.
pub fn plane_dims(layout: PixelLayout, w: u64, h: u64, p: usize) -> Result<(u64, u64), VoleError> {
    let tmpl = layout.planes().get(p).ok_or(VoleError::GeometryMismatch)?;
    Ok((ceil_div(w, tmpl.subsample_x), ceil_div(h, tmpl.subsample_y)))
}

/// Expected canonical payload bytes of one decoded frame at this format.
pub fn expected_canonical_bytes(name: &str, w: u64, h: u64) -> Result<u64, VoleError> {
    let d = desc(name)?;
    let sb = bps(d.depth);
    let mut total = 0u64;
    match d.kind {
        Kind::Planar => {
            for p in 0..d.layout.planes().len() {
                let (pw, ph) = plane_dims(d.layout, w, h, p)?;
                total += pw * ph * sb;
            }
        }
        Kind::SemiPlanar { .. } => {
            total += w * h * sb;
            let (cw, ch) = plane_dims(d.layout, w, h, 1)?;
            total += cw * ch * 2 * sb;
        }
        Kind::Packed { stride, .. } => {
            total += w * h * u64::from(stride) * sb;
        }
        Kind::Yuyv | Kind::Uyvy => {
            total += w * h * 2;
        }
    }
    Ok(total)
}

/// The payload-plane component order FFmpeg writes for this planar format.
fn payload_components(d: &Desc) -> &'static [Component] {
    match d.name {
        "gbrp" | "gbrp10le" | "gbrp12le" | "gbrp16le" => &[G, B, R],
        "gbrap" | "gbrap10le" => &[G, B, R, A],
        "gray" | "gray16le" => &[Gray],
        "yuv420p" | "yuv420p10le" | "yuv420p12le" | "yuv420p14le" | "yuv420p16le" | "yuv422p"
        | "yuv422p10le" | "yuv422p12le" | "yuv422p16le" | "yuv444p" | "yuv444p10le"
        | "yuv444p12le" | "yuv444p16le" => &[Y, Cb, Cr],
        "yuva420p" | "yuva444p" | "yuva444p10le" | "yuva444p12le" | "yuva444p16le" => {
            &[Y, Cb, Cr, A]
        }
        _ => &[],
    }
}

fn dims_for_comp(
    layout: PixelLayout,
    w: u64,
    h: u64,
    comp: Component,
) -> Result<(u64, u64), VoleError> {
    let tmpl = layout
        .planes()
        .iter()
        .find(|t| t.component == comp)
        .ok_or(VoleError::UnsupportedPixelLayout)?;
    Ok((ceil_div(w, tmpl.subsample_x), ceil_div(h, tmpl.subsample_y)))
}

/// Unpack one decoded frame payload into canonical planes (order matches the
/// canonical layout's plane templates). The payload length must equal
/// [`expected_canonical_bytes`].
pub fn unpack_frame(name: &str, w: u64, h: u64, payload: &[u8]) -> Result<Vec<Plane>, VoleError> {
    let d = desc(name)?;
    let want = expected_canonical_bytes(name, w, h)?;
    if payload.len() as u64 != want {
        return Err(VoleError::InvalidSamples);
    }
    let depth = BitDepth::new(d.depth)?;
    let sb = bps(d.depth) as usize;
    match d.kind {
        Kind::Planar => {
            // Payload regions in payload-plane order.
            let order = payload_components(d);
            let mut regions: Vec<(&Component, usize, usize)> = Vec::new();
            let mut off = 0usize;
            for comp in order {
                let (pw, ph) = dims_for_comp(d.layout, w, h, *comp)?;
                let bytes = (pw * ph) as usize * sb;
                regions.push((comp, off, bytes));
                off += bytes;
            }
            if off != payload.len() {
                return Err(VoleError::InvalidSamples);
            }
            let mut planes = Vec::with_capacity(d.layout.planes().len());
            for (p, tmpl) in d.layout.planes().iter().enumerate() {
                let (pw, ph) = plane_dims(d.layout, w, h, p)?;
                let (_, rstart, rlen) = regions
                    .iter()
                    .find(|(c, _, _)| **c == tmpl.component)
                    .ok_or(VoleError::UnsupportedPixelLayout)?;
                let slice = payload
                    .get(*rstart..rstart + rlen)
                    .ok_or(VoleError::Truncated)?;
                planes.push(build_plane(
                    tmpl.component,
                    pw,
                    ph,
                    depth,
                    tmpl.subsample_x,
                    tmpl.subsample_y,
                    slice,
                )?);
            }
            Ok(planes)
        }
        Kind::SemiPlanar { uv_first } => {
            let y_bytes = (w * h) as usize * sb;
            let y_slice = payload.get(..y_bytes).ok_or(VoleError::Truncated)?;
            let y = build_plane(Y, w, h, depth, 0, 0, y_slice)?;
            let (cw, ch) = plane_dims(d.layout, w, h, 1)?;
            let chroma_samples = (cw * ch) as usize;
            let uv_slice = payload
                .get(y_bytes..y_bytes + chroma_samples * 2 * sb)
                .ok_or(VoleError::Truncated)?;
            let mut planes = vec![y];
            for comp in [Cb, Cr] {
                let first = comp == uv_first;
                let mut values = Vec::with_capacity(chroma_samples);
                for i in 0..chroma_samples {
                    let base = i * 2 * sb + if first { 0 } else { sb };
                    values.push(sample_value(&uv_slice[base..base + sb], depth));
                }
                planes.push(build_plane_from_values(comp, cw, ch, depth, 1, 1, values)?);
            }
            Ok(planes)
        }
        Kind::Packed { stride, first } => {
            let stride = u64::from(stride) * bps(d.depth);
            let row_bytes = (w * stride) as usize;
            let mut planes = Vec::with_capacity(d.layout.planes().len());
            for (p, tmpl) in d.layout.planes().iter().enumerate() {
                let byte = u64::from(first) + p as u64 * bps(d.depth);
                let mut values = Vec::with_capacity((w * h) as usize);
                for y in 0..h as usize {
                    let row = payload
                        .get(y * row_bytes..(y + 1) * row_bytes)
                        .ok_or(VoleError::Truncated)?;
                    for x in 0..w as usize {
                        let at = x * stride as usize + byte as usize;
                        values.push(sample_value(&row[at..at + bps(d.depth) as usize], depth));
                    }
                }
                planes.push(build_plane_from_values(
                    tmpl.component,
                    w,
                    h,
                    depth,
                    tmpl.subsample_x,
                    tmpl.subsample_y,
                    values,
                )?);
            }
            Ok(planes)
        }
        Kind::Yuyv | Kind::Uyvy => {
            if !w.is_multiple_of(2) {
                // Packed 4:2:2 carries chroma per luma pair; odd widths have
                // no canonical representation — fail closed rather than guess.
                return Err(VoleError::UnsupportedPixelLayout);
            }
            let row_bytes = w * 2;
            let (cw, _) = plane_dims(d.layout, w, h, 1)?;
            let mut y_values = Vec::with_capacity((w * h) as usize);
            let mut cb_values = Vec::with_capacity((cw * h) as usize);
            let mut cr_values = Vec::with_capacity((cw * h) as usize);
            for y in 0..h as usize {
                let row = payload
                    .get(y * row_bytes as usize..(y + 1) * row_bytes as usize)
                    .ok_or(VoleError::Truncated)?;
                for pair in 0..(w / 2) as usize {
                    let base = pair * 4;
                    let (y0, u, y1, v) = match d.kind {
                        Kind::Yuyv => (row[base], row[base + 1], row[base + 2], row[base + 3]),
                        _ => (row[base + 1], row[base], row[base + 3], row[base + 2]),
                    };
                    y_values.push(u32::from(y0));
                    y_values.push(u32::from(y1));
                    cb_values.push(u32::from(u));
                    cr_values.push(u32::from(v));
                }
            }
            Ok(vec![
                build_plane_from_values(Y, w, h, depth, 0, 0, y_values)?,
                build_plane_from_values(Cb, cw, h, depth, 1, 0, cb_values)?,
                build_plane_from_values(Cr, cw, h, depth, 1, 0, cr_values)?,
            ])
        }
    }
}

fn sample_value(src: &[u8], depth: BitDepth) -> u32 {
    match depth.storage() {
        PlaneStorage::U8 => u32::from(src[0]),
        PlaneStorage::U16 => u32::from(u16::from_le_bytes([src[0], src[1]])),
    }
}

fn build_plane(
    comp: Component,
    pw: u64,
    ph: u64,
    depth: BitDepth,
    sx: u8,
    sy: u8,
    bytes: &[u8],
) -> Result<Plane, VoleError> {
    let count = (pw * ph) as usize;
    let data = match depth.storage() {
        PlaneStorage::U8 => {
            if bytes.len() != count {
                return Err(VoleError::InvalidSamples);
            }
            if bytes.iter().any(|b| u32::from(*b) > depth.max_sample()) {
                return Err(VoleError::InvalidSamples);
            }
            PlaneData::U8(bytes.to_vec())
        }
        PlaneStorage::U16 => {
            if bytes.len() != count * 2 {
                return Err(VoleError::InvalidSamples);
            }
            let mut v = Vec::with_capacity(count);
            for pair in bytes.as_chunks::<2>().0 {
                let s = u16::from_le_bytes(*pair);
                if u32::from(s) > depth.max_sample() {
                    return Err(VoleError::InvalidSamples);
                }
                v.push(s);
            }
            PlaneData::U16(v)
        }
    };
    Plane::new(comp, pw as u32, ph as u32, depth, sx, sy, data)
}

fn build_plane_from_values(
    comp: Component,
    pw: u64,
    ph: u64,
    depth: BitDepth,
    sx: u8,
    sy: u8,
    values: Vec<u32>,
) -> Result<Plane, VoleError> {
    let max = depth.max_sample();
    let data = match depth.storage() {
        PlaneStorage::U8 => {
            if values.iter().any(|v| *v > max) {
                return Err(VoleError::InvalidSamples);
            }
            PlaneData::U8(values.iter().map(|v| *v as u8).collect())
        }
        PlaneStorage::U16 => {
            if values.iter().any(|v| *v > max) {
                return Err(VoleError::InvalidSamples);
            }
            PlaneData::U16(values.iter().map(|v| *v as u16).collect())
        }
    };
    Plane::new(comp, pw as u32, ph as u32, depth, sx, sy, data)
}

/// Repack canonical planes back to the source pixel format's exact payload
/// bytes (the inverse of [`unpack_frame`]; used by the oracle proof and the
/// round-trip courts).
pub fn repack_frame(name: &str, w: u64, h: u64, planes: &[Plane]) -> Result<Vec<u8>, VoleError> {
    let d = desc(name)?;
    let sb = bps(d.depth) as usize;
    let out_len = expected_canonical_bytes(name, w, h)? as usize;
    let mut out = vec![0u8; out_len];
    match d.kind {
        Kind::Planar => {
            let order = payload_components(d);
            let mut off = 0usize;
            for comp in order {
                let (pw, ph) = dims_for_comp(d.layout, w, h, *comp)?;
                let bytes = (pw * ph) as usize * sb;
                let plane = plane_of(planes, *comp)?;
                let canonical = plane.canonical_bytes();
                out[off..off + bytes].copy_from_slice(&canonical);
                off += bytes;
            }
            if off != out_len {
                return Err(VoleError::InvalidSamples);
            }
        }
        Kind::SemiPlanar { uv_first } => {
            let y = plane_of(planes, Y)?;
            let y_bytes = (w * h) as usize * sb;
            out[..y_bytes].copy_from_slice(&y.canonical_bytes());
            let (cw, _) = plane_dims(d.layout, w, h, 1)?;
            let cb = plane_of(planes, Cb)?.canonical_bytes();
            let cr = plane_of(planes, Cr)?.canonical_bytes();
            let mut off = y_bytes;
            for row in 0..h.div_ceil(2) {
                for col in 0..cw {
                    let i = (row * cw + col) as usize * sb;
                    let (a, b) = if uv_first == Cb {
                        (&cb[i..i + sb], &cr[i..i + sb])
                    } else {
                        (&cr[i..i + sb], &cb[i..i + sb])
                    };
                    out[off..off + sb].copy_from_slice(a);
                    off += sb;
                    out[off..off + sb].copy_from_slice(b);
                    off += sb;
                }
            }
        }
        Kind::Packed { stride, first } => {
            let stride = u64::from(stride) * bps(d.depth);
            let row_bytes = (w * stride) as usize;
            for (p, tmpl) in d.layout.planes().iter().enumerate() {
                let byte = u64::from(first) + p as u64 * bps(d.depth);
                let plane = plane_of(planes, tmpl.component)?;
                let canonical = plane.canonical_bytes();
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let src =
                            &canonical[(y * w as usize + x) * sb..(y * w as usize + x + 1) * sb];
                        let dst = y * row_bytes + x * stride as usize + byte as usize;
                        out[dst..dst + sb].copy_from_slice(src);
                    }
                }
            }
        }
        Kind::Yuyv | Kind::Uyvy => {
            if !w.is_multiple_of(2) {
                return Err(VoleError::UnsupportedPixelLayout);
            }
            let y = plane_of(planes, Y)?.canonical_bytes();
            let cb = plane_of(planes, Cb)?.canonical_bytes();
            let cr = plane_of(planes, Cr)?.canonical_bytes();
            let row_bytes = w * 2;
            let (cw, _) = plane_dims(d.layout, w, h, 1)?;
            for row in 0..h as usize {
                for pair in 0..(w / 2) as usize {
                    let base = (row as u64 * row_bytes + pair as u64 * 4) as usize;
                    let li = (row as u64 * w + pair as u64 * 2) as usize;
                    let ci = (row as u64 * cw + pair as u64) as usize;
                    let (y0, y1, u, v) = (y[li], y[li + 1], cb[ci], cr[ci]);
                    match d.kind {
                        Kind::Yuyv => {
                            out[base] = y0;
                            out[base + 1] = u;
                            out[base + 2] = y1;
                            out[base + 3] = v;
                        }
                        _ => {
                            out[base] = u;
                            out[base + 1] = y0;
                            out[base + 2] = v;
                            out[base + 3] = y1;
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn plane_of(planes: &[Plane], comp: Component) -> Result<&Plane, VoleError> {
    planes
        .iter()
        .find(|p| p.component() == comp)
        .ok_or(VoleError::GeometryMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::epoch::VideoEpoch;
    use crate::media::picture::Picture;

    fn epoch_for(layout: PixelLayout, depth: u8, w: u64, h: u64) -> VideoEpoch {
        use crate::media::color::ColorDescription;
        use crate::media::epoch::EpochId;
        use crate::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
        VideoEpoch::new_uniform(
            EpochId(0),
            w as u32,
            h as u32,
            layout,
            BitDepth::new(depth).unwrap(),
            ColorDescription::unspecified(),
            SampleAspectRatio::square(),
            Orientation::Normal,
            FieldStructure::Progressive,
        )
        .unwrap()
    }

    fn check_roundtrip(name: &str, w: u64, h: u64) {
        let (layout, depth) = layout_and_depth(name).unwrap();
        let epoch = epoch_for(layout, depth, w, h);
        let want = expected_canonical_bytes(name, w, h).unwrap();
        // Canonical picture size must equal the source payload size (tight)
        // except for the zero-channel packed families (rgb0/bgr0/0rgb/0bgr),
        // whose dropped zero byte is deliberately not canonical content.
        let zero_packed = ["rgb0", "bgr0", "0rgb", "0bgr"];
        if !zero_packed.contains(&name) {
            assert_eq!(
                epoch.observation_bytes().unwrap(),
                want,
                "{name}: canonical size matches the source payload size"
            );
        }
        // Build deterministic depth-valid canonical planes, then prove
        // pack/unpack round-trips byte-exactly.
        let max = BitDepth::new(depth).unwrap().max_sample();
        let mut seed = 0x243F_6A88_85A3_08D3u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 33) as u32 % (max + 1)
        };
        let mut planes = Vec::new();
        for p in 0..epoch.plane_count() {
            let (pw, ph) = epoch.plane_dimensions(p).unwrap();
            let tmpl = &epoch.planes()[p];
            let n = (pw * ph) as usize;
            let values: Vec<u32> = (0..n).map(|_| next()).collect();
            let data = match tmpl.bit_depth.storage() {
                PlaneStorage::U8 => PlaneData::U8(values.iter().map(|v| *v as u8).collect()),
                PlaneStorage::U16 => PlaneData::U16(values.iter().map(|v| *v as u16).collect()),
            };
            planes.push(
                Plane::new(
                    tmpl.component,
                    pw,
                    ph,
                    tmpl.bit_depth,
                    tmpl.subsample_x,
                    tmpl.subsample_y,
                    data,
                )
                .unwrap(),
            );
        }
        let pic = Picture::from_planes(&epoch, planes.clone()).unwrap();
        let zero_packed = ["rgb0", "bgr0", "0rgb", "0bgr"];
        if !zero_packed.contains(&name) {
            assert_eq!(pic.total_bytes(), want);
        }
        let payload = repack_frame(name, w, h, &planes).unwrap();
        assert_eq!(payload.len() as u64, want);
        let round = unpack_frame(name, w, h, &payload).unwrap();
        for (a, b) in planes.iter().zip(round.iter()) {
            assert_eq!(a.canonical_bytes(), b.canonical_bytes());
        }
        // Re-pack the re-unpacked planes: byte-identical source payload.
        let again = repack_frame(name, w, h, &round).unwrap();
        assert_eq!(again, payload, "{name}: pack ∘ unpack == id at {w}x{h}");
    }

    #[test]
    fn unpack_repack_roundtrips_odd_and_even() {
        for &name in SUPPORTED {
            for (w, h) in [(16u64, 16u64), (18, 12), (3, 3)] {
                let even_only = name == "yuyv422" || name == "uyvy422";
                if even_only && (w % 2 != 0) {
                    continue;
                }
                check_roundtrip(name, w, h);
            }
        }
    }

    #[test]
    fn padding_bits_beyond_active_depth_are_refused() {
        // A 10-bit planar payload whose u16 words carry nonzero padding bits
        // must be refused typed (never silently truncated).
        let (layout, depth) = layout_and_depth("yuv420p10le").unwrap();
        let epoch = epoch_for(layout, depth, 16, 16);
        let want = expected_canonical_bytes("yuv420p10le", 16, 16).unwrap();
        assert_eq!(epoch.observation_bytes().unwrap(), want);
        let mut payload = vec![0xFFu8; want as usize]; // every u16 word = 0xFFFF
        payload[0] = 0x00;
        payload[1] = 0x04; // word 0x0400 > 0x03FF: padding bit set
        assert_eq!(
            unpack_frame("yuv420p10le", 16, 16, &payload).unwrap_err(),
            VoleError::InvalidSamples
        );
        // A wrong payload length is refused before any sample work.
        assert_eq!(
            unpack_frame("yuv420p10le", 16, 16, &payload[..want as usize - 1]).unwrap_err(),
            VoleError::InvalidSamples
        );
    }

    #[test]
    fn unsupported_formats_fail_closed() {
        assert_eq!(
            layout_and_depth("yuv420p10be").unwrap_err(),
            VoleError::UnsupportedPixelLayout
        );
        assert_eq!(
            layout_and_depth("pal8").unwrap_err(),
            VoleError::UnsupportedPixelLayout
        );
        assert_eq!(
            layout_and_depth("not-a-format").unwrap_err(),
            VoleError::UnsupportedPixelLayout
        );
    }
}
