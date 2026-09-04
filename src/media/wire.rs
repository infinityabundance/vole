//! Format v2 core wire — Phase V.1.2 + V.1.4 family extension (normative
//! grammar documented in `docs/format-v2.md`; this module is its
//! implementation).
//!
//! The v2 core container is the multi-plane program of one epoch:
//!
//! ```text
//! File      := Header MediaDescriptor PlaneBlock* Integrity
//! ```
//!
//! * `Header` (24 B, same prefix shape as v1 so version/universe dispatch is
//!   a pure prefix read): magic `"VOLE"`, reserved 0, `format_version = 2`,
//!   `universe_id = 2`, `limit_profile = 1`, `feature_bits`, coded `width`,
//!   `height`;
//! * `MediaDescriptor` (tag `0x11`): the epoch's full media interpretation —
//!   layout code, per-plane component/depth table, chroma location, color
//!   description, SAR, orientation, and field structure;
//! * `PlaneBlock` (tag `0x10`, one per epoch plane, ascending index): the
//!   plane program — background, object declarations, initial instances,
//!   overlay, interval groups of [`PlaneOp`]s, and (under the V.1.4 family
//!   feature bit) the initial palette table + per-instance motion records;
//! * `Integrity`: the last 32 bytes are BLAKE3 over every preceding byte.
//!
//! Every count/length is bounded by the frozen v1 [`Limits`] envelope applied
//! per plane; unknown codes, tags, and non-canonical encodings fail closed
//! typed. Side data and the rational PTS schedule are reserved extensions
//! (V.1.2's wire core carries the program; timeline binding is in-memory
//! until the container layer lands).
//!
//! The v2 grammar was frozen at the end of V.1.2 and **deliberately extended
//! at the end of V.1.4** (`docs/format-v2.md` re-frozen together with the
//! goldens and the hostile corpus): feature bit `0x1` (family extension)
//! declares the V.1.4 surface — object kinds `0x03` (palette-index) and `0x04`
//! (generator), ops `0x29`–`0x31` (velocity/advance, trajectory/advance,
//! palette ops, binding, affine, transform residual), and the per-plane
//! initial palette/motion tail. Files without the bit keep their exact V.1.2
//! meaning and byte form; v1 files decode forever under v1 semantics and
//! never acquire v2 interpretation.

use std::collections::BTreeMap;

use crate::checked::{ByteReader, ByteSink};
use crate::error::VoleError;
use crate::integr;
use crate::limits::Limits;
use crate::media::color::{
    ChromaLocation, ColorDescription, ColorPrimaries, ColorRange, MatrixCoefficients,
    TransferCharacteristic,
};
use crate::media::core::{
    MultiPlaneProgram, PlaneContent, PlaneInstance, PlaneInstanceId, PlaneMotion, PlaneObject,
    PlaneObjectId, PlaneOp, PlanePaletteId, PlaneProgram,
};
use crate::media::epoch::{EpochId, VideoEpoch};
use crate::media::layout::{Component, PixelLayout};
use crate::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
use crate::media::plane::{BitDepth, PlaneData};
use crate::trajectory::TrajectorySegment;

/// V.1.4 family-extension feature bit: the stream may use object kinds `0x03`/
/// `0x04`, ops `0x29`–`0x31`, and the per-plane initial palette/motion tail.
/// Old files (bit clear) parse exactly as the frozen V.1.2 grammar.
pub const FEATURE_FAMILY: u32 = 0x1;

/// Known feature bits of the v2 core container (unknown bits fail closed).
pub const V2_FEATURES: u32 = FEATURE_FAMILY;

/// Canonical trajectory segment wire kinds (mirror the sealed v1 forms).
pub const SEG_LINEAR: u8 = 0x00;
pub const SEG_ACCEL: u8 = 0x01;

/// Canonical registry codes (frozen in `docs/format-v2.md`).
pub mod codes {
    pub const LAYOUT_GRAY: u16 = 1;
    pub const LAYOUT_YUV400: u16 = 2;
    pub const LAYOUT_YUV420: u16 = 3;
    pub const LAYOUT_YUV422: u16 = 4;
    pub const LAYOUT_YUV444: u16 = 5;
    pub const LAYOUT_YUVA420: u16 = 6;
    pub const LAYOUT_YUVA444: u16 = 7;
    pub const LAYOUT_GBR: u16 = 8;
    pub const LAYOUT_GBRA: u16 = 9;
    pub const LAYOUT_RGB: u16 = 10;
    pub const LAYOUT_BGR: u16 = 11;
    pub const LAYOUT_RGBA: u16 = 12;
    pub const LAYOUT_BGRA: u16 = 13;
    pub const LAYOUT_ARGB: u16 = 14;
    pub const LAYOUT_ABGR: u16 = 15;
    pub const LAYOUT_INDEXED: u16 = 16;

    pub const COMP_Y: u8 = 1;
    pub const COMP_CB: u8 = 2;
    pub const COMP_CR: u8 = 3;
    pub const COMP_R: u8 = 4;
    pub const COMP_G: u8 = 5;
    pub const COMP_B: u8 = 6;
    pub const COMP_A: u8 = 7;
    pub const COMP_GRAY: u8 = 8;
    pub const COMP_INDEX: u8 = 9;

    pub const OBJECT_FILL: u8 = 1;
    pub const OBJECT_RASTER: u8 = 2;
    pub const OBJECT_INDEX: u8 = 3; // V.1.4 family extension
    pub const OBJECT_GENERATOR: u8 = 4; // V.1.4 family extension

    pub const TAG_DESCRIPTOR: u8 = 0x11;
    pub const TAG_PLANE: u8 = 0x10;

    pub const OP_DECLARE_OBJECT: u8 = 0x21;
    pub const OP_CREATE_INSTANCE: u8 = 0x22;
    pub const OP_SET_POSITION: u8 = 0x23;
    pub const OP_CLEAR_INSTANCES: u8 = 0x24;
    pub const OP_CLEAR_OVERLAY: u8 = 0x25;
    pub const OP_PATCH_OVERLAY: u8 = 0x26;
    pub const OP_COPY_RECT: u8 = 0x27;
    pub const OP_RESIDUAL: u8 = 0x28;
    pub const OP_SET_VELOCITY: u8 = 0x29; // V.1.4 family extension
    pub const OP_ADVANCE_TRANSLATIONS: u8 = 0x2A; // V.1.4 family extension
    pub const OP_SET_TRAJECTORY: u8 = 0x2B; // V.1.4 family extension
    pub const OP_ADVANCE_TRAJECTORIES: u8 = 0x2C; // V.1.4 family extension
    pub const OP_SET_PALETTE: u8 = 0x2D; // V.1.4 family extension
    pub const OP_PATCH_PALETTE: u8 = 0x2E; // V.1.4 family extension
    pub const OP_BIND_PALETTE: u8 = 0x2F; // V.1.4 family extension
    pub const OP_SET_AFFINE: u8 = 0x30; // V.1.4 family extension
    pub const OP_TRANSFORM_RESIDUAL: u8 = 0x31; // V.1.4 family extension
}

fn layout_code(l: PixelLayout) -> u16 {
    use codes::*;
    match l {
        PixelLayout::Gray => LAYOUT_GRAY,
        PixelLayout::Yuv400 => LAYOUT_YUV400,
        PixelLayout::Yuv420 => LAYOUT_YUV420,
        PixelLayout::Yuv422 => LAYOUT_YUV422,
        PixelLayout::Yuv444 => LAYOUT_YUV444,
        PixelLayout::Yuva420 => LAYOUT_YUVA420,
        PixelLayout::Yuva444 => LAYOUT_YUVA444,
        PixelLayout::Gbr => LAYOUT_GBR,
        PixelLayout::Gbra => LAYOUT_GBRA,
        PixelLayout::Rgb => LAYOUT_RGB,
        PixelLayout::Bgr => LAYOUT_BGR,
        PixelLayout::Rgba => LAYOUT_RGBA,
        PixelLayout::Bgra => LAYOUT_BGRA,
        PixelLayout::Argb => LAYOUT_ARGB,
        PixelLayout::Abgr => LAYOUT_ABGR,
        PixelLayout::Indexed => LAYOUT_INDEXED,
    }
}

fn layout_of(code: u16) -> Result<PixelLayout, VoleError> {
    use codes::*;
    Ok(match code {
        LAYOUT_GRAY => PixelLayout::Gray,
        LAYOUT_YUV400 => PixelLayout::Yuv400,
        LAYOUT_YUV420 => PixelLayout::Yuv420,
        LAYOUT_YUV422 => PixelLayout::Yuv422,
        LAYOUT_YUV444 => PixelLayout::Yuv444,
        LAYOUT_YUVA420 => PixelLayout::Yuva420,
        LAYOUT_YUVA444 => PixelLayout::Yuva444,
        LAYOUT_GBR => PixelLayout::Gbr,
        LAYOUT_GBRA => PixelLayout::Gbra,
        LAYOUT_RGB => PixelLayout::Rgb,
        LAYOUT_BGR => PixelLayout::Bgr,
        LAYOUT_RGBA => PixelLayout::Rgba,
        LAYOUT_BGRA => PixelLayout::Bgra,
        LAYOUT_ARGB => PixelLayout::Argb,
        LAYOUT_ABGR => PixelLayout::Abgr,
        LAYOUT_INDEXED => PixelLayout::Indexed,
        _ => return Err(VoleError::UnsupportedPixelLayout),
    })
}

fn component_code(c: Component) -> u8 {
    use codes::*;
    match c {
        Component::Y => COMP_Y,
        Component::Cb => COMP_CB,
        Component::Cr => COMP_CR,
        Component::R => COMP_R,
        Component::G => COMP_G,
        Component::B => COMP_B,
        Component::A => COMP_A,
        Component::Gray => COMP_GRAY,
        Component::Index => COMP_INDEX,
        Component::Other(_) => 0,
    }
}

fn component_of(code: u8) -> Result<Component, VoleError> {
    use codes::*;
    Ok(match code {
        COMP_Y => Component::Y,
        COMP_CB => Component::Cb,
        COMP_CR => Component::Cr,
        COMP_R => Component::R,
        COMP_G => Component::G,
        COMP_B => Component::B,
        COMP_A => Component::A,
        COMP_GRAY => Component::Gray,
        COMP_INDEX => Component::Index,
        _ => return Err(VoleError::UnsupportedPixelLayout),
    })
}

fn chroma_code(c: ChromaLocation) -> u8 {
    match c {
        ChromaLocation::Unspecified => 0,
        ChromaLocation::Center => 1,
        ChromaLocation::Left => 2,
        ChromaLocation::TopLeft => 3,
        ChromaLocation::Top => 4,
        ChromaLocation::BottomLeft => 5,
        ChromaLocation::Bottom => 6,
    }
}
fn chroma_of(v: u8) -> Result<ChromaLocation, VoleError> {
    Ok(match v {
        0 => ChromaLocation::Unspecified,
        1 => ChromaLocation::Center,
        2 => ChromaLocation::Left,
        3 => ChromaLocation::TopLeft,
        4 => ChromaLocation::Top,
        5 => ChromaLocation::BottomLeft,
        6 => ChromaLocation::Bottom,
        _ => return Err(VoleError::NonCanonicalEncoding),
    })
}

fn primaries_code(p: ColorPrimaries) -> u8 {
    match p {
        ColorPrimaries::Unspecified => 0,
        ColorPrimaries::Bt709 => 1,
        ColorPrimaries::Bt470M => 2,
        ColorPrimaries::Bt470Bg => 3,
        ColorPrimaries::Smpte170M => 4,
        ColorPrimaries::Smpte240M => 5,
        ColorPrimaries::Film => 6,
        ColorPrimaries::Bt2020 => 7,
    }
}
fn primaries_of(v: u8) -> Result<ColorPrimaries, VoleError> {
    Ok(match v {
        0 => ColorPrimaries::Unspecified,
        1 => ColorPrimaries::Bt709,
        2 => ColorPrimaries::Bt470M,
        3 => ColorPrimaries::Bt470Bg,
        4 => ColorPrimaries::Smpte170M,
        5 => ColorPrimaries::Smpte240M,
        6 => ColorPrimaries::Film,
        7 => ColorPrimaries::Bt2020,
        _ => return Err(VoleError::NonCanonicalEncoding),
    })
}

fn transfer_code(t: TransferCharacteristic) -> u8 {
    match t {
        TransferCharacteristic::Unspecified => 0,
        TransferCharacteristic::Bt709 => 1,
        TransferCharacteristic::Gamma22 => 2,
        TransferCharacteristic::Gamma28 => 3,
        TransferCharacteristic::Smpte170M => 4,
        TransferCharacteristic::Smpte240M => 5,
        TransferCharacteristic::Linear => 6,
        TransferCharacteristic::Srgb => 7,
        TransferCharacteristic::Bt2020_10 => 8,
        TransferCharacteristic::Bt2020_12 => 9,
        TransferCharacteristic::Smpte2084 => 10,
        TransferCharacteristic::AribStdB67 => 11,
    }
}
fn transfer_of(v: u8) -> Result<TransferCharacteristic, VoleError> {
    Ok(match v {
        0 => TransferCharacteristic::Unspecified,
        1 => TransferCharacteristic::Bt709,
        2 => TransferCharacteristic::Gamma22,
        3 => TransferCharacteristic::Gamma28,
        4 => TransferCharacteristic::Smpte170M,
        5 => TransferCharacteristic::Smpte240M,
        6 => TransferCharacteristic::Linear,
        7 => TransferCharacteristic::Srgb,
        8 => TransferCharacteristic::Bt2020_10,
        9 => TransferCharacteristic::Bt2020_12,
        10 => TransferCharacteristic::Smpte2084,
        11 => TransferCharacteristic::AribStdB67,
        _ => return Err(VoleError::NonCanonicalEncoding),
    })
}

fn matrix_code(m: MatrixCoefficients) -> u8 {
    match m {
        MatrixCoefficients::Unspecified => 0,
        MatrixCoefficients::Identity => 1,
        MatrixCoefficients::Bt709 => 2,
        MatrixCoefficients::Smpte170M => 3,
        MatrixCoefficients::Smpte240M => 4,
        MatrixCoefficients::YcgCo => 5,
        MatrixCoefficients::Bt2020Ncl => 6,
        MatrixCoefficients::Bt2020Cl => 7,
    }
}
fn matrix_of(v: u8) -> Result<MatrixCoefficients, VoleError> {
    Ok(match v {
        0 => MatrixCoefficients::Unspecified,
        1 => MatrixCoefficients::Identity,
        2 => MatrixCoefficients::Bt709,
        3 => MatrixCoefficients::Smpte170M,
        4 => MatrixCoefficients::Smpte240M,
        5 => MatrixCoefficients::YcgCo,
        6 => MatrixCoefficients::Bt2020Ncl,
        7 => MatrixCoefficients::Bt2020Cl,
        _ => return Err(VoleError::NonCanonicalEncoding),
    })
}

fn range_code(r: ColorRange) -> u8 {
    match r {
        ColorRange::Unspecified => 0,
        ColorRange::Limited => 1,
        ColorRange::Full => 2,
    }
}
fn range_of(v: u8) -> Result<ColorRange, VoleError> {
    Ok(match v {
        0 => ColorRange::Unspecified,
        1 => ColorRange::Limited,
        2 => ColorRange::Full,
        _ => return Err(VoleError::NonCanonicalEncoding),
    })
}

fn orientation_code(o: Orientation) -> u8 {
    match o {
        Orientation::Normal => 0,
        Orientation::Rotate90 => 1,
        Orientation::Rotate180 => 2,
        Orientation::Rotate270 => 3,
        Orientation::FlipHorizontal => 4,
        Orientation::FlipVertical => 5,
    }
}
fn orientation_of(v: u8) -> Result<Orientation, VoleError> {
    Ok(match v {
        0 => Orientation::Normal,
        1 => Orientation::Rotate90,
        2 => Orientation::Rotate180,
        3 => Orientation::Rotate270,
        4 => Orientation::FlipHorizontal,
        5 => Orientation::FlipVertical,
        _ => return Err(VoleError::NonCanonicalEncoding),
    })
}

fn field_code(f: FieldStructure) -> u8 {
    match f {
        FieldStructure::Unknown => 0,
        FieldStructure::Progressive => 1,
        FieldStructure::InterlacedTopFieldFirst => 2,
        FieldStructure::InterlacedBottomFieldFirst => 3,
    }
}
fn field_of(v: u8) -> Result<FieldStructure, VoleError> {
    Ok(match v {
        0 => FieldStructure::Unknown,
        1 => FieldStructure::Progressive,
        2 => FieldStructure::InterlacedTopFieldFirst,
        3 => FieldStructure::InterlacedBottomFieldFirst,
        _ => return Err(VoleError::NonCanonicalEncoding),
    })
}

/// Canonical coordinate bound (v1 mirror): `|x|, |y| ≤ 2^24`.
pub const MAX_COORD: i64 = 1 << 24;

fn check_coord(v: i32) -> Result<(), VoleError> {
    if i64::from(v).abs() > MAX_COORD {
        return Err(VoleError::NonCanonicalEncoding);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

fn write_object(
    s: &mut ByteSink,
    id: u32,
    obj: &PlaneObject,
    depth: BitDepth,
) -> Result<(), VoleError> {
    if obj.width == 0 || obj.height == 0 {
        return Err(VoleError::InvalidSamples);
    }
    let max = depth.max_sample();
    s.push(id)?;
    s.push(obj.width)?;
    s.push(obj.height)?;
    match &obj.content {
        PlaneContent::Fill(v) => {
            if *v > max {
                return Err(VoleError::InvalidSamples);
            }
            s.byte(codes::OBJECT_FILL)?;
            s.push(*v)?;
        }
        PlaneContent::Raster(data) => {
            if data.len() as u64 != u64::from(obj.width) * u64::from(obj.height) {
                return Err(VoleError::InvalidSamples);
            }
            match data {
                PlaneData::U8(v) => {
                    if !depth.is_byte_depth() {
                        return Err(VoleError::InvalidSamples);
                    }
                    if v.iter().any(|x| u32::from(*x) > max) {
                        return Err(VoleError::InvalidSamples);
                    }
                    s.byte(codes::OBJECT_RASTER)?;
                    s.push(v.len() as u64)?;
                    s.extend(v)?;
                }
                PlaneData::U16(v) => {
                    if depth.is_byte_depth() {
                        return Err(VoleError::InvalidSamples);
                    }
                    if v.iter().any(|x| u32::from(*x) > max) {
                        return Err(VoleError::InvalidSamples);
                    }
                    s.byte(codes::OBJECT_RASTER)?;
                    // Canonical payload length is in bytes: two per u16 sample.
                    s.push(
                        u64::try_from(v.len())
                            .expect("usize <= u64")
                            .checked_mul(2)
                            .ok_or(VoleError::ArithmeticOverflow)?,
                    )?;
                    for w in v {
                        s.extend(&w.to_le_bytes())?;
                    }
                }
            }
        }
        // Palette-index content (V.1.4): one byte per index, tight.
        PlaneContent::Index(indices) => {
            if indices.len() as u64 != u64::from(obj.width) * u64::from(obj.height) {
                return Err(VoleError::InvalidSamples);
            }
            s.byte(codes::OBJECT_INDEX)?;
            s.push(indices.len() as u64)?;
            s.extend(indices)?;
        }
        // Depth-aware generator content (V.1.4).
        PlaneContent::Generator(gen) => {
            gen.check(max)?;
            s.byte(codes::OBJECT_GENERATOR)?;
            s.extend(&gen.program_bytes())?;
        }
    }
    Ok(())
}

/// Write the trajectory-segment list of one payload (canonical wire: kind
/// byte + signed i32 fields + u64 steps, mirroring the sealed v1 form).
fn write_segments(s: &mut ByteSink, segments: &[TrajectorySegment]) -> Result<(), VoleError> {
    for seg in segments {
        match seg {
            TrajectorySegment::Linear { vx, vy, steps } => {
                check_coord(*vx as i32)?;
                check_coord(*vy as i32)?;
                s.byte(SEG_LINEAR)?;
                s.push(*vx as i32)?;
                s.push(*vy as i32)?;
                s.push(*steps)?;
            }
            TrajectorySegment::Accel {
                vx0,
                vy0,
                ax,
                ay,
                steps,
            } => {
                check_coord(*vx0 as i32)?;
                check_coord(*vy0 as i32)?;
                check_coord(*ax as i32)?;
                check_coord(*ay as i32)?;
                s.byte(SEG_ACCEL)?;
                s.push(*vx0 as i32)?;
                s.push(*vy0 as i32)?;
                s.push(*ax as i32)?;
                s.push(*ay as i32)?;
                s.push(*steps)?;
            }
        }
    }
    Ok(())
}

/// Read the trajectory-segment list of one payload (canonical checks: kind
/// byte, signed coordinate domain, canonical segment form enforced by
/// `TrajectorySegment::check`).
fn read_segments(
    r: &mut ByteReader<'_>,
    count: u32,
    limits: &Limits,
) -> Result<Vec<TrajectorySegment>, VoleError> {
    if u64::from(count) > u64::from(limits.max_trajectory_segments) {
        return Err(VoleError::DimensionTooLarge);
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let kind = r.u8()?;
        let seg = match kind {
            SEG_LINEAR => {
                let vx = i64::from(r.pull::<i32>()?);
                let vy = i64::from(r.pull::<i32>()?);
                let steps = r.pull::<u64>()?;
                TrajectorySegment::Linear { vx, vy, steps }
            }
            SEG_ACCEL => {
                let vx0 = i64::from(r.pull::<i32>()?);
                let vy0 = i64::from(r.pull::<i32>()?);
                let ax = i64::from(r.pull::<i32>()?);
                let ay = i64::from(r.pull::<i32>()?);
                let steps = r.pull::<u64>()?;
                TrajectorySegment::Accel {
                    vx0,
                    vy0,
                    ax,
                    ay,
                    steps,
                }
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        };
        seg.check()?;
        out.push(seg);
    }
    // Adjacent-equal-velocity linear segments are non-canonical (one canonical
    // encoding only).
    for pair in out.windows(2) {
        if let [TrajectorySegment::Linear { vx: a, vy: b, .. }, TrajectorySegment::Linear { vx: c, vy: d, .. }] =
            pair
        {
            if a == c && b == d {
                return Err(VoleError::NonCanonicalEncoding);
            }
        }
    }
    Ok(out)
}

/// Serialize the multi-plane core container of one epoch (header + media
/// descriptor + plane blocks + BLAKE3 trailer).
pub fn write_multiplane(program: &MultiPlaneProgram) -> Result<Vec<u8>, VoleError> {
    let limits = Limits::default();
    let epoch = &program.epoch;
    if program.planes.len() != epoch.plane_count() {
        return Err(VoleError::GeometryMismatch);
    }
    limits.check_canvas(epoch.width(), epoch.height())?;
    if !epoch.side_data().is_empty() {
        return Err(VoleError::ApiConstraint(
            "v2 core wire does not serialize side data yet (reserved extension)",
        ));
    }
    // The writer sets the minimal feature bits the content needs: a stream
    // without V.1.4 family-extension surface keeps the exact V.1.2 byte form.
    let features = if program.planes.iter().any(|p| p.uses_family_extension()) {
        FEATURE_FAMILY
    } else {
        0
    };
    let mut s = ByteSink::new();
    // Header (24 bytes; same prefix shape as v1).
    s.extend(b"VOLE")?;
    s.byte(0)?;
    s.push(2u16)?; // format_version = 2
    s.push(2u32)?; // universe_id = 2
    s.byte(1)?; // limit_profile = 1 (v1 envelope, per plane)
    s.push(features)?;
    s.push(epoch.width())?;
    s.push(epoch.height())?;
    // Media descriptor.
    s.byte(codes::TAG_DESCRIPTOR)?;
    s.push(layout_code(epoch.layout()))?;
    let planes_tmpl = epoch.layout().planes();
    s.byte(planes_tmpl.len() as u8)?;
    for (i, tmpl) in planes_tmpl.iter().enumerate() {
        s.byte(component_code(tmpl.component))?;
        s.byte(epoch.planes()[i].bit_depth.bits())?;
    }
    let color = epoch.color();
    s.byte(chroma_code(color.chroma_location()))?;
    s.byte(primaries_code(color.primaries()))?;
    s.byte(transfer_code(color.transfer()))?;
    s.byte(matrix_code(color.matrix()))?;
    s.byte(range_code(color.range()))?;
    s.push(epoch.sar().width())?;
    s.push(epoch.sar().height())?;
    s.byte(orientation_code(epoch.orientation()))?;
    s.byte(field_code(epoch.field_structure()))?;
    // Plane blocks.
    for (i, prog) in program.planes.iter().enumerate() {
        let depth = epoch.planes()[i].bit_depth;
        let max = depth.max_sample();
        s.byte(codes::TAG_PLANE)?;
        s.byte(i as u8)?;
        s.push(prog.background)?;
        // Objects.
        s.push(prog.objects.len() as u32)?;
        let mut ordered: Vec<(&PlaneObjectId, &PlaneObject)> = prog.objects.iter().collect();
        ordered.sort_by_key(|(id, _)| id.0);
        for (id, obj) in ordered {
            write_object(&mut s, id.0, obj, depth)?;
        }
        // Instances.
        s.push(prog.instances.len() as u32)?;
        for inst in &prog.instances {
            check_coord(inst.x as i32)?;
            check_coord(inst.y as i32)?;
            s.push(inst.id.0)?;
            s.push(inst.object.0)?;
            s.push(inst.x as i32)?;
            s.push(inst.y as i32)?;
        }
        // Overlay.
        s.push(prog.overlay.len() as u32)?;
        for &(x, y, v) in &prog.overlay {
            check_coord(x as i32)?;
            check_coord(y as i32)?;
            if v > depth.max_sample() {
                return Err(VoleError::InvalidSamples);
            }
            s.push(x as i32)?;
            s.push(y as i32)?;
            s.push(v)?;
        }
        // Intervals.
        s.push(prog.intervals.len() as u32)?;
        for (t, ops) in &prog.intervals {
            s.push(*t)?;
            s.push(ops.len() as u32)?;
            for op in ops {
                match op {
                    PlaneOp::DeclareObject { id, object } => {
                        s.byte(codes::OP_DECLARE_OBJECT)?;
                        write_object(&mut s, id.0, object, depth)?;
                    }
                    PlaneOp::CreateInstance { id, object, x, y } => {
                        check_coord(*x as i32)?;
                        check_coord(*y as i32)?;
                        s.byte(codes::OP_CREATE_INSTANCE)?;
                        s.push(id.0)?;
                        s.push(object.0)?;
                        s.push(*x as i32)?;
                        s.push(*y as i32)?;
                    }
                    PlaneOp::SetPosition { id, x, y } => {
                        check_coord(*x as i32)?;
                        check_coord(*y as i32)?;
                        s.byte(codes::OP_SET_POSITION)?;
                        s.push(id.0)?;
                        s.push(*x as i32)?;
                        s.push(*y as i32)?;
                    }
                    PlaneOp::ClearInstances => s.byte(codes::OP_CLEAR_INSTANCES)?,
                    PlaneOp::ClearOverlay => s.byte(codes::OP_CLEAR_OVERLAY)?,
                    PlaneOp::PatchOverlay { points } => {
                        s.byte(codes::OP_PATCH_OVERLAY)?;
                        s.push(points.len() as u32)?;
                        for &(x, y, v) in points {
                            check_coord(x as i32)?;
                            check_coord(y as i32)?;
                            if v > depth.max_sample() {
                                return Err(VoleError::InvalidSamples);
                            }
                            s.push(x as i32)?;
                            s.push(y as i32)?;
                            s.push(v)?;
                        }
                    }
                    PlaneOp::CopyRect {
                        src_x,
                        src_y,
                        width: cw,
                        height: ch,
                        dst_x,
                        dst_y,
                    } => {
                        check_coord(*src_x as i32)?;
                        check_coord(*src_y as i32)?;
                        check_coord(*dst_x as i32)?;
                        check_coord(*dst_y as i32)?;
                        s.byte(codes::OP_COPY_RECT)?;
                        s.push(*src_x as i32)?;
                        s.push(*src_y as i32)?;
                        s.push(*cw)?;
                        s.push(*ch)?;
                        s.push(*dst_x as i32)?;
                        s.push(*dst_y as i32)?;
                    }
                    PlaneOp::Residual { block } => {
                        if block.len() as u64 > limits.max_residual_bytes {
                            return Err(VoleError::DimensionTooLarge);
                        }
                        s.byte(codes::OP_RESIDUAL)?;
                        s.push(block.len() as u64)?;
                        s.extend(block)?;
                    }
                    // --- V.1.4 family-extension ops ---
                    PlaneOp::SetVelocity { id, vx, vy } => {
                        check_coord(*vx as i32)?;
                        check_coord(*vy as i32)?;
                        s.byte(codes::OP_SET_VELOCITY)?;
                        s.push(id.0)?;
                        s.push(*vx as i32)?;
                        s.push(*vy as i32)?;
                    }
                    PlaneOp::AdvanceTranslations => s.byte(codes::OP_ADVANCE_TRANSLATIONS)?,
                    PlaneOp::SetTrajectory { id, segments } => {
                        s.byte(codes::OP_SET_TRAJECTORY)?;
                        s.push(id.0)?;
                        s.push(segments.len() as u32)?;
                        write_segments(&mut s, segments)?;
                    }
                    PlaneOp::AdvanceTrajectories => s.byte(codes::OP_ADVANCE_TRAJECTORIES)?,
                    PlaneOp::SetPalette { id, entries } => {
                        s.byte(codes::OP_SET_PALETTE)?;
                        s.push(id.0)?;
                        s.push(entries.len() as u32)?;
                        for v in entries {
                            if *v > max {
                                return Err(VoleError::InvalidSamples);
                            }
                            s.push(*v)?;
                        }
                    }
                    PlaneOp::PatchPalette { id, changes } => {
                        s.byte(codes::OP_PATCH_PALETTE)?;
                        s.push(id.0)?;
                        s.push(changes.len() as u32)?;
                        for (idx, v) in changes {
                            if *v > max {
                                return Err(VoleError::InvalidSamples);
                            }
                            s.push(*idx)?;
                            s.push(*v)?;
                        }
                    }
                    PlaneOp::BindPalette { instance, palette } => {
                        s.byte(codes::OP_BIND_PALETTE)?;
                        s.push(instance.0)?;
                        s.push(palette.0)?;
                    }
                    PlaneOp::SetAffine { id, params } => {
                        s.byte(codes::OP_SET_AFFINE)?;
                        s.push(id.0)?;
                        for c in [params.a, params.b, params.c, params.d, params.e, params.f] {
                            check_coord(c as i32)?;
                            s.push(c as i32)?;
                        }
                    }
                    PlaneOp::TransformResidual { block } => {
                        if block.len() as u64 > limits.max_residual_bytes {
                            return Err(VoleError::DimensionTooLarge);
                        }
                        s.byte(codes::OP_TRANSFORM_RESIDUAL)?;
                        s.push(block.len() as u64)?;
                        s.extend(block)?;
                    }
                }
            }
        }
        // V.1.4 family-extension tail: the initial palette table (ascending
        // palette id) and per-instance motion records (ascending instance id).
        if features & FEATURE_FAMILY != 0 {
            write_initial_tail(&mut s, prog, max)?;
        }
    }
    integr::append_trailer(&mut s)?;
    Ok(s.into_vec())
}

/// Write the per-plane family-extension tail (palette table then motion
/// records, each in canonical ascending order).
fn write_initial_tail(s: &mut ByteSink, prog: &PlaneProgram, max: u32) -> Result<(), VoleError> {
    s.push(prog.palettes.len() as u32)?;
    for (id, entries) in &prog.palettes {
        if *id == PlanePaletteId::NONE || entries.is_empty() {
            return Err(VoleError::NonCanonicalEncoding);
        }
        s.push(id.0)?;
        s.push(entries.len() as u32)?;
        for v in entries {
            if *v > max {
                return Err(VoleError::InvalidSamples);
            }
            s.push(*v)?;
        }
    }
    let mut motion = prog.initial_motion.clone();
    motion.sort_by_key(PlaneMotion::instance);
    s.push(motion.len() as u32)?;
    for rec in motion {
        s.push(rec.instance().0)?;
        match rec {
            PlaneMotion::Velocity { vx, vy, .. } => {
                check_coord(vx as i32)?;
                check_coord(vy as i32)?;
                s.byte(0x01)?;
                s.push(vx as i32)?;
                s.push(vy as i32)?;
            }
            PlaneMotion::Trajectory { segments, .. } => {
                s.byte(0x02)?;
                s.push(segments.len() as u32)?;
                write_segments(s, &segments)?;
            }
            PlaneMotion::Affine { params, .. } => {
                s.byte(0x03)?;
                for c in [params.a, params.b, params.c, params.d, params.e, params.f] {
                    check_coord(c as i32)?;
                    s.push(c as i32)?;
                }
            }
            PlaneMotion::Binding { palette, .. } => {
                s.byte(0x04)?;
                s.push(palette.0)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn read_object(
    r: &mut ByteReader<'_>,
    depth: BitDepth,
    limits: &Limits,
    ext: bool,
) -> Result<(u32, PlaneObject), VoleError> {
    let id = r.pull::<u32>()?;
    let w = r.pull::<u32>()?;
    let h = r.pull::<u32>()?;
    if w == 0 || h == 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let count = u64::from(w)
        .checked_mul(u64::from(h))
        .ok_or(VoleError::ArithmeticOverflow)?;
    if count > limits.max_object_bytes {
        return Err(VoleError::DimensionTooLarge);
    }
    let kind = r.u8()?;
    let max = depth.max_sample();
    let content = match kind {
        codes::OBJECT_FILL => {
            let v = r.pull::<u32>()?;
            if v > max {
                return Err(VoleError::InvalidSamples);
            }
            PlaneContent::Fill(v)
        }
        codes::OBJECT_RASTER => {
            let len = r.pull::<u64>()?;
            let spb = depth.storage().bytes_per_sample();
            // The declared length is canonical payload bytes: exactly
            // `w * h` samples at `bytes_per_sample` bytes each.
            let want = count
                .checked_mul(spb)
                .ok_or(VoleError::ArithmeticOverflow)?;
            if len != want {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let bytes = r.take_vec(len as usize)?;
            let data = match depth.storage() {
                crate::media::plane::PlaneStorage::U8 => {
                    if !depth.is_byte_depth() {
                        return Err(VoleError::InvalidSamples);
                    }
                    if bytes.iter().any(|v| u32::from(*v) > max) {
                        return Err(VoleError::InvalidSamples);
                    }
                    PlaneData::U8(bytes)
                }
                crate::media::plane::PlaneStorage::U16 => {
                    if depth.is_byte_depth() {
                        return Err(VoleError::InvalidSamples);
                    }
                    let mut v = Vec::with_capacity(bytes.len() / 2);
                    for c in bytes.as_chunks::<2>().0 {
                        let w = u16::from_le_bytes([c[0], c[1]]);
                        if u32::from(w) > max {
                            return Err(VoleError::InvalidSamples);
                        }
                        v.push(w);
                    }
                    PlaneData::U16(v)
                }
            };
            PlaneContent::Raster(data)
        }
        // Palette-index content (V.1.4): one byte per index, tight (indices
        // are bounded by max_palette_entries = 256 at render time).
        codes::OBJECT_INDEX => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let len = r.pull::<u64>()?;
            if len != count {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let indices = r.take_vec(len as usize)?;
            PlaneContent::Index(indices)
        }
        // Depth-aware generator content (V.1.4).
        codes::OBJECT_GENERATOR => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            PlaneContent::Generator(crate::media::gen::Gen::parse_program(r, max)?)
        }
        _ => return Err(VoleError::NonCanonicalEncoding),
    };
    Ok((
        id,
        PlaneObject {
            width: w,
            height: h,
            content,
        },
    ))
}

/// Parse a v2 core container into its epoch and plane programs (validated,
/// bounded, canonical). Structural errors (magic/version/universe/tags)
/// surface precisely before the integrity check; the trailing digest is
/// verified once the structure has parsed — unknown mandatory structure fails
/// closed typed, and a flipped content byte is `IntegrityMismatch`.
pub fn parse_multiplane(bytes: &[u8]) -> Result<MultiPlaneProgram, VoleError> {
    let limits = Limits::default();
    if bytes.len() as u64 > limits.max_stream_bytes {
        return Err(VoleError::DimensionTooLarge);
    }
    if bytes.len() < 32 {
        return Err(VoleError::Truncated);
    }
    let (payload, trailer) = bytes.split_at(bytes.len() - 32);
    let mut r = ByteReader::new(payload);

    // Header.
    if r.take(4)? != b"VOLE" {
        return Err(VoleError::BadMagic);
    }
    if r.u8()? != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    if r.pull::<u16>()? != 2 {
        return Err(VoleError::UnsupportedFeature);
    }
    if r.pull::<u32>()? != 2 {
        return Err(VoleError::UnsupportedUniverse);
    }
    if r.u8()? != 1 {
        return Err(VoleError::UnsupportedLimitProfile);
    }
    let features = r.pull::<u32>()?;
    if features & !V2_FEATURES != 0 {
        return Err(VoleError::UnsupportedFeature);
    }
    let ext = features & FEATURE_FAMILY != 0;
    let width = r.pull::<u32>()?;
    let height = r.pull::<u32>()?;
    limits.check_canvas(width, height)?;

    // Media descriptor.
    if r.u8()? != codes::TAG_DESCRIPTOR {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let layout = layout_of(r.pull::<u16>()?)?;
    let tmpls = layout.planes();
    let declared_planes = r.u8()?;
    if declared_planes as usize != tmpls.len() {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let mut components = Vec::with_capacity(tmpls.len());
    let mut depths = Vec::with_capacity(tmpls.len());
    for tmpl in tmpls {
        let code = r.u8()?;
        let comp = component_of(code)?;
        let bits = r.u8()?;
        let depth = BitDepth::new(bits)?;
        if comp != tmpl.component {
            return Err(VoleError::NonCanonicalEncoding);
        }
        components.push(comp);
        depths.push(depth);
    }
    let chroma = chroma_of(r.u8()?)?;
    let primaries = primaries_of(r.u8()?)?;
    let transfer = transfer_of(r.u8()?)?;
    let matrix = matrix_of(r.u8()?)?;
    let range = range_of(r.u8()?)?;
    let sar_w = r.pull::<u32>()?;
    let sar_h = r.pull::<u32>()?;
    let orientation = orientation_of(r.u8()?)?;
    let field = field_of(r.u8()?)?;
    let sar = SampleAspectRatio::new(sar_w, sar_h)?;
    let color = ColorDescription::new(primaries, transfer, matrix, range, chroma);
    let epoch = VideoEpoch::new_per_plane(
        EpochId(0),
        width,
        height,
        layout,
        &depths,
        color,
        sar,
        orientation,
        field,
    )?;
    let _ = components;

    // Plane blocks.
    let mut planes: Vec<Option<PlaneProgram>> = vec![None; tmpls.len()];
    let mut seen_planes = 0u32;
    while r.remaining() > 0 {
        let tag = r.u8()?;
        match tag {
            codes::TAG_PLANE => {
                let idx = r.u8()? as usize;
                if idx >= planes.len() {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                if planes[idx].is_some() {
                    return Err(VoleError::DuplicateId);
                }
                seen_planes = seen_planes.saturating_add(1);
                let depth = depths[idx];
                let bg = r.pull::<u32>()?;
                if bg > depth.max_sample() {
                    return Err(VoleError::InvalidSamples);
                }
                let mut objects: BTreeMap<PlaneObjectId, PlaneObject> = BTreeMap::new();
                let n_objects = r.pull::<u32>()?;
                if u64::from(n_objects) > u64::from(limits.max_objects) {
                    return Err(VoleError::DimensionTooLarge);
                }
                for _ in 0..n_objects {
                    let (id, obj) = read_object(&mut r, depth, &limits, ext)?;
                    if objects.insert(PlaneObjectId(id), obj).is_some() {
                        return Err(VoleError::DuplicateId);
                    }
                }
                let n_inst = r.pull::<u32>()?;
                if u64::from(n_inst) > u64::from(limits.max_instances) {
                    return Err(VoleError::DimensionTooLarge);
                }
                let mut instances = Vec::with_capacity(n_inst as usize);
                let mut seen_iid = std::collections::HashSet::new();
                for _ in 0..n_inst {
                    let iid = r.pull::<u32>()?;
                    let oid = r.pull::<u32>()?;
                    let x = r.pull::<i32>()?;
                    let y = r.pull::<i32>()?;
                    check_coord(x)?;
                    check_coord(y)?;
                    if !seen_iid.insert(iid) {
                        return Err(VoleError::DuplicateId);
                    }
                    if !objects.contains_key(&PlaneObjectId(oid)) {
                        return Err(VoleError::UnknownObject);
                    }
                    instances.push(PlaneInstance {
                        id: PlaneInstanceId(iid),
                        object: PlaneObjectId(oid),
                        x: i64::from(x),
                        y: i64::from(y),
                    });
                }
                let n_overlay = r.pull::<u32>()?;
                if u64::from(n_overlay) > limits.max_overlay_points {
                    return Err(VoleError::DimensionTooLarge);
                }
                let mut overlay = Vec::with_capacity(n_overlay as usize);
                let mut prev_key: Option<(i64, i64)> = None;
                for _ in 0..n_overlay {
                    let x = i64::from(r.pull::<i32>()?);
                    let y = i64::from(r.pull::<i32>()?);
                    let v = r.pull::<u32>()?;
                    if v > depth.max_sample() {
                        return Err(VoleError::InvalidSamples);
                    }
                    check_coord(x as i32)?;
                    check_coord(y as i32)?;
                    let key = (x, y);
                    if prev_key.is_some_and(|p| key <= p) {
                        return Err(VoleError::NonCanonicalEncoding);
                    }
                    prev_key = Some(key);
                    overlay.push((x, y, v));
                }
                let n_intervals = r.pull::<u32>()?;
                if u64::from(n_intervals) > limits.max_checkpoint_distance {
                    return Err(VoleError::CheckpointOutOfEnvelope);
                }
                let mut intervals = Vec::with_capacity(n_intervals as usize);
                let mut prev_t: Option<u64> = None;
                for _ in 0..n_intervals {
                    let t = r.pull::<u64>()?;
                    if t == 0 {
                        return Err(VoleError::NonConsecutiveInterval);
                    }
                    if prev_t.is_some_and(|p| t <= p) {
                        return Err(VoleError::NonConsecutiveInterval);
                    }
                    prev_t = Some(t);
                    let n_ops = r.pull::<u32>()?;
                    if n_ops > limits.max_transitions_per_interval {
                        return Err(VoleError::DimensionTooLarge);
                    }
                    let mut ops = Vec::with_capacity(n_ops as usize);
                    for _ in 0..n_ops {
                        ops.push(read_op(&mut r, depth, &limits, ext)?);
                    }
                    intervals.push((t, ops));
                }
                // V.1.4 family-extension tail (only under the feature bit).
                let (palettes, initial_motion) = if ext {
                    read_initial_tail(&mut r, depth, &limits)?
                } else {
                    (BTreeMap::new(), Vec::new())
                };
                planes[idx] = Some(PlaneProgram {
                    background: bg,
                    objects,
                    instances,
                    overlay,
                    intervals,
                    palettes,
                    initial_motion,
                });
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        }
    }
    if seen_planes as usize != planes.len() {
        return Err(VoleError::NonCanonicalEncoding);
    }
    if r.remaining() != 0 {
        return Err(VoleError::NonCanonicalEncoding);
    }
    let plane_programs: Vec<PlaneProgram> =
        planes.into_iter().map(|p| p.expect("present")).collect();
    let program = MultiPlaneProgram::new(epoch, plane_programs)?;
    // Integrity: the trailing 32 bytes must equal BLAKE3 of the payload.
    if integr::digest(payload) != trailer {
        return Err(VoleError::IntegrityMismatch);
    }
    Ok(program)
}

/// Read the per-plane family-extension tail: the initial palette table
/// (strictly ascending ids; entries inside the plane depth, count
/// `1..=max_palette_entries`) and the per-instance motion records (strictly
/// ascending instance ids, at most one per initial instance; canonical per
/// kind).
#[allow(clippy::type_complexity)] // (palette table, motion records)
fn read_initial_tail(
    r: &mut ByteReader<'_>,
    depth: BitDepth,
    limits: &Limits,
) -> Result<(BTreeMap<PlanePaletteId, Vec<u32>>, Vec<PlaneMotion>), VoleError> {
    let max = depth.max_sample();
    let mut palettes: BTreeMap<PlanePaletteId, Vec<u32>> = BTreeMap::new();
    let n_pal = r.pull::<u32>()?;
    if u64::from(n_pal) > u64::from(limits.max_palettes) {
        return Err(VoleError::DimensionTooLarge);
    }
    let mut prev_id: Option<u32> = None;
    for _ in 0..n_pal {
        let id = r.pull::<u32>()?;
        if id == 0 {
            return Err(VoleError::NonCanonicalEncoding);
        }
        if prev_id.is_some_and(|p| id <= p) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        prev_id = Some(id);
        let n_entries = r.pull::<u32>()?;
        if n_entries == 0 || u64::from(n_entries) > u64::from(limits.max_palette_entries) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        let mut entries = Vec::with_capacity(n_entries as usize);
        for _ in 0..n_entries {
            let v = r.pull::<u32>()?;
            if v > max {
                return Err(VoleError::InvalidSamples);
            }
            entries.push(v);
        }
        palettes.insert(PlanePaletteId(id), entries);
    }
    let mut motion: Vec<PlaneMotion> = Vec::new();
    let n_motion = r.pull::<u32>()?;
    let mut prev_inst: Option<u32> = None;
    for _ in 0..n_motion {
        let instance = PlaneInstanceId(r.pull::<u32>()?);
        if prev_inst.is_some_and(|p| instance.0 <= p) {
            return Err(VoleError::NonCanonicalEncoding);
        }
        prev_inst = Some(instance.0);
        let kind = r.u8()?;
        let rec = match kind {
            0x01 => {
                let vx = i64::from(r.pull::<i32>()?);
                let vy = i64::from(r.pull::<i32>()?);
                check_coord(vx as i32)?;
                check_coord(vy as i32)?;
                if vx == 0 && vy == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                PlaneMotion::Velocity { instance, vx, vy }
            }
            0x02 => {
                let n_seg = r.pull::<u32>()?;
                if n_seg == 0 {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                let segments = read_segments(r, n_seg, limits)?;
                PlaneMotion::Trajectory { instance, segments }
            }
            0x03 => {
                let mut c = [0i64; 6];
                for v in &mut c {
                    let x = i64::from(r.pull::<i32>()?);
                    check_coord(x as i32)?;
                    *v = x;
                }
                let params = crate::affine::AffineParams {
                    a: c[0],
                    b: c[1],
                    c: c[2],
                    d: c[3],
                    e: c[4],
                    f: c[5],
                };
                params.check()?;
                if params.is_identity() {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                PlaneMotion::Affine { instance, params }
            }
            0x04 => {
                let palette = PlanePaletteId(r.pull::<u32>()?);
                if palette == PlanePaletteId::NONE {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                PlaneMotion::Binding { instance, palette }
            }
            _ => return Err(VoleError::NonCanonicalEncoding),
        };
        motion.push(rec);
    }
    Ok((palettes, motion))
}

fn read_op(
    r: &mut ByteReader<'_>,
    depth: BitDepth,
    limits: &Limits,
    ext: bool,
) -> Result<PlaneOp, VoleError> {
    let tag = r.u8()?;
    Ok(match tag {
        codes::OP_DECLARE_OBJECT => {
            let (id, object) = read_object(r, depth, limits, ext)?;
            PlaneOp::DeclareObject {
                id: PlaneObjectId(id),
                object,
            }
        }
        codes::OP_CREATE_INSTANCE => {
            let id = r.pull::<u32>()?;
            let object = r.pull::<u32>()?;
            let x = i64::from(r.pull::<i32>()?);
            let y = i64::from(r.pull::<i32>()?);
            check_coord(x as i32)?;
            check_coord(y as i32)?;
            PlaneOp::CreateInstance {
                id: PlaneInstanceId(id),
                object: PlaneObjectId(object),
                x,
                y,
            }
        }
        codes::OP_SET_POSITION => {
            let id = r.pull::<u32>()?;
            let x = i64::from(r.pull::<i32>()?);
            let y = i64::from(r.pull::<i32>()?);
            check_coord(x as i32)?;
            check_coord(y as i32)?;
            PlaneOp::SetPosition {
                id: PlaneInstanceId(id),
                x,
                y,
            }
        }
        codes::OP_CLEAR_INSTANCES => PlaneOp::ClearInstances,
        codes::OP_CLEAR_OVERLAY => PlaneOp::ClearOverlay,
        codes::OP_PATCH_OVERLAY => {
            let n = r.pull::<u32>()?;
            if u64::from(n) > u64::from(limits.max_transitions_per_interval) {
                return Err(VoleError::DimensionTooLarge);
            }
            let mut points = Vec::with_capacity(n as usize);
            let mut prev_key: Option<(i64, i64)> = None;
            for _ in 0..n {
                let x = i64::from(r.pull::<i32>()?);
                let y = i64::from(r.pull::<i32>()?);
                let v = r.pull::<u32>()?;
                check_coord(x as i32)?;
                check_coord(y as i32)?;
                if v > depth.max_sample() {
                    return Err(VoleError::InvalidSamples);
                }
                let key = (x, y);
                if prev_key.is_some_and(|p| key <= p) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                prev_key = Some(key);
                points.push((x, y, v));
            }
            PlaneOp::PatchOverlay { points }
        }
        codes::OP_COPY_RECT => {
            let src_x = i64::from(r.pull::<i32>()?);
            let src_y = i64::from(r.pull::<i32>()?);
            let width = r.pull::<u32>()?;
            let height = r.pull::<u32>()?;
            let dst_x = i64::from(r.pull::<i32>()?);
            let dst_y = i64::from(r.pull::<i32>()?);
            check_coord(src_x as i32)?;
            check_coord(src_y as i32)?;
            check_coord(dst_x as i32)?;
            check_coord(dst_y as i32)?;
            if width == 0 || height == 0 {
                return Err(VoleError::NonCanonicalEncoding);
            }
            if u64::from(width) * u64::from(height) > limits.max_copy_area {
                return Err(VoleError::DimensionTooLarge);
            }
            PlaneOp::CopyRect {
                src_x,
                src_y,
                width,
                height,
                dst_x,
                dst_y,
            }
        }
        codes::OP_RESIDUAL => {
            let len = r.pull::<u64>()?;
            if len > limits.max_residual_bytes {
                return Err(VoleError::DimensionTooLarge);
            }
            let block = r.take_vec(len as usize)?;
            PlaneOp::Residual { block }
        }
        // --- V.1.4 family-extension ops (tag requires the feature bit) ---
        codes::OP_SET_VELOCITY => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let id = PlaneInstanceId(r.pull::<u32>()?);
            let vx = i64::from(r.pull::<i32>()?);
            let vy = i64::from(r.pull::<i32>()?);
            check_coord(vx as i32)?;
            check_coord(vy as i32)?;
            PlaneOp::SetVelocity { id, vx, vy }
        }
        codes::OP_ADVANCE_TRANSLATIONS => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            PlaneOp::AdvanceTranslations
        }
        codes::OP_SET_TRAJECTORY => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let id = PlaneInstanceId(r.pull::<u32>()?);
            let n_seg = r.pull::<u32>()?;
            let segments = read_segments(r, n_seg, limits)?;
            PlaneOp::SetTrajectory { id, segments }
        }
        codes::OP_ADVANCE_TRAJECTORIES => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            PlaneOp::AdvanceTrajectories
        }
        codes::OP_SET_PALETTE => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let id = PlanePaletteId(r.pull::<u32>()?);
            if id == PlanePaletteId::NONE {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let n = r.pull::<u32>()?;
            if n == 0 || u64::from(n) > u64::from(limits.max_palette_entries) {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let mut entries = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let v = r.pull::<u32>()?;
                if v > depth.max_sample() {
                    return Err(VoleError::InvalidSamples);
                }
                entries.push(v);
            }
            PlaneOp::SetPalette { id, entries }
        }
        codes::OP_PATCH_PALETTE => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let id = PlanePaletteId(r.pull::<u32>()?);
            let n = r.pull::<u32>()?;
            if u64::from(n) > u64::from(limits.max_transitions_per_interval) {
                return Err(VoleError::DimensionTooLarge);
            }
            let mut changes = Vec::with_capacity(n as usize);
            let mut prev_idx: Option<u32> = None;
            for _ in 0..n {
                let idx = r.pull::<u32>()?;
                let v = r.pull::<u32>()?;
                if v > depth.max_sample() {
                    return Err(VoleError::InvalidSamples);
                }
                if prev_idx.is_some_and(|p| idx <= p) {
                    return Err(VoleError::NonCanonicalEncoding);
                }
                prev_idx = Some(idx);
                changes.push((idx, v));
            }
            PlaneOp::PatchPalette { id, changes }
        }
        codes::OP_BIND_PALETTE => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let instance = PlaneInstanceId(r.pull::<u32>()?);
            let palette = PlanePaletteId(r.pull::<u32>()?);
            PlaneOp::BindPalette { instance, palette }
        }
        codes::OP_SET_AFFINE => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let id = PlaneInstanceId(r.pull::<u32>()?);
            let mut c = [0i64; 6];
            for v in &mut c {
                let x = i64::from(r.pull::<i32>()?);
                check_coord(x as i32)?;
                *v = x;
            }
            let params = crate::affine::AffineParams {
                a: c[0],
                b: c[1],
                c: c[2],
                d: c[3],
                e: c[4],
                f: c[5],
            };
            params.check()?;
            PlaneOp::SetAffine { id, params }
        }
        codes::OP_TRANSFORM_RESIDUAL => {
            if !ext {
                return Err(VoleError::NonCanonicalEncoding);
            }
            let len = r.pull::<u64>()?;
            if len > limits.max_residual_bytes {
                return Err(VoleError::DimensionTooLarge);
            }
            let block = r.take_vec(len as usize)?;
            PlaneOp::TransformResidual { block }
        }
        _ => return Err(VoleError::NonCanonicalEncoding),
    })
}
