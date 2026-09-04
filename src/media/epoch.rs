//! Epochs and canonical video observations — Phase V.1.1 (V.1 brief §13,
//! §30, §40; contract §2.3, §2.5).
//!
//! A [`VideoEpoch`] declares the **full media interpretation** of the
//! observations bound to it: coded geometry, canonical layout, per-plane bit
//! depth, chroma location, color description, SAR, orientation, field
//! structure, and side data. Every [`CanonicalVideoObservation`] binds to one
//! epoch; a change of any declared property is a *new epoch*, never a silent
//! rescale of later observations (§13). A [`CanonicalVideo`] is the validated
//! presentation-ordered sequence of observations over its epochs.
//!
//! The two-clock separation (contract §2.2) applies: an observation's `PTS`
//! and `duration` live on the rational media clock here; the procedural state
//! machine (which V.1.2 binds to observations) keeps v1's explicit-interval
//! semantics. V.1.1 constructs **synthetic canonical vectors** only — nothing
//! reads a file.

use core::cmp::Ordering;

use crate::error::VoleError;
use crate::media::color::ColorDescription;
use crate::media::layout::{Component, PixelLayout};
use crate::media::meta::{FieldStructure, Orientation, SampleAspectRatio, VisualSideData};
use crate::media::plane::{BitDepth, Plane, PlaneData, PlaneStorage};
use crate::media::time::{Duration, Pts, TimeBase};

/// Identifies one epoch of a canonical video. Epoch ids are dense per
/// [`CanonicalVideo`] (`0..n` in declaration order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EpochId(pub u64);

/// One plane of an epoch: component, subsampling exponents (informational —
/// the authoritative geometry derives from the coded dimensions), and active
/// bit depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneTemplate {
    /// Component carried by the plane.
    pub component: Component,
    /// Horizontal subsampling exponent.
    pub subsample_x: u8,
    /// Vertical subsampling exponent.
    pub subsample_y: u8,
    /// Active bit depth of every sample of this plane.
    pub bit_depth: BitDepth,
}

/// A declared media interpretation epoch.
///
/// Immutable after construction; every field is validated so an epoch value
/// is canonical by type.
#[derive(Debug, Clone)]
pub struct VideoEpoch {
    id: EpochId,
    /// Coded (luma/reference) width of the epoch's pictures.
    width: u32,
    /// Coded (luma/reference) height of the epoch's pictures.
    height: u32,
    /// Canonical planar layout.
    layout: PixelLayout,
    /// Per-plane templates (derived from the layout and the declared depths).
    planes: Vec<PlaneTemplate>,
    /// Full color description (primaries, transfer, matrix, range, and
    /// chroma sample location — the single source of color semantics).
    color: ColorDescription,
    /// Sample aspect ratio.
    sar: SampleAspectRatio,
    /// Display orientation (interpretation only).
    orientation: Orientation,
    /// Field structure (interpretation only).
    field_structure: FieldStructure,
    /// Bounded visual side data.
    side_data: Vec<VisualSideData>,
}

impl VideoEpoch {
    /// Build an epoch with a **uniform** bit depth across every plane.
    #[allow(clippy::too_many_arguments)] // one epoch descriptor; grouped only when a MediaDescriptor struct lands with v2
    pub fn new_uniform(
        id: EpochId,
        width: u32,
        height: u32,
        layout: PixelLayout,
        bit_depth: BitDepth,
        color: ColorDescription,
        sar: SampleAspectRatio,
        orientation: Orientation,
        field_structure: FieldStructure,
    ) -> Result<Self, VoleError> {
        let templates = layout
            .planes()
            .iter()
            .map(|t| PlaneTemplate {
                component: t.component,
                subsample_x: t.subsample_x,
                subsample_y: t.subsample_y,
                bit_depth,
            })
            .collect();
        Self::from_templates(
            id,
            width,
            height,
            layout,
            templates,
            color,
            sar,
            orientation,
            field_structure,
        )
    }

    /// Build an epoch with **per-plane** bit depths (count must equal the
    /// layout's plane count).
    #[allow(clippy::too_many_arguments)] // one epoch descriptor
    pub fn new_per_plane(
        id: EpochId,
        width: u32,
        height: u32,
        layout: PixelLayout,
        depths: &[BitDepth],
        color: ColorDescription,
        sar: SampleAspectRatio,
        orientation: Orientation,
        field_structure: FieldStructure,
    ) -> Result<Self, VoleError> {
        if depths.len() != layout.plane_count() {
            return Err(VoleError::GeometryMismatch);
        }
        let templates = layout
            .planes()
            .iter()
            .zip(depths.iter())
            .map(|(t, d)| PlaneTemplate {
                component: t.component,
                subsample_x: t.subsample_x,
                subsample_y: t.subsample_y,
                bit_depth: *d,
            })
            .collect();
        Self::from_templates(
            id,
            width,
            height,
            layout,
            templates,
            color,
            sar,
            orientation,
            field_structure,
        )
    }

    #[allow(clippy::too_many_arguments)] // one epoch descriptor
    fn from_templates(
        id: EpochId,
        width: u32,
        height: u32,
        layout: PixelLayout,
        planes: Vec<PlaneTemplate>,
        color: ColorDescription,
        sar: SampleAspectRatio,
        orientation: Orientation,
        field_structure: FieldStructure,
    ) -> Result<Self, VoleError> {
        if width == 0 || height == 0 {
            return Err(VoleError::GeometryMismatch);
        }
        // Total canonical sample count over every plane must be representable.
        let mut total = 0u64;
        for (i, _) in planes.iter().enumerate() {
            total = total
                .checked_add(layout.plane_sample_count(i, width, height)?)
                .ok_or(VoleError::ArithmeticOverflow)?;
        }
        if total == 0 {
            return Err(VoleError::GeometryMismatch);
        }
        Ok(VideoEpoch {
            id,
            width,
            height,
            layout,
            planes,
            color,
            sar,
            orientation,
            field_structure,
            side_data: Vec::new(),
        })
    }

    /// Append one piece of side data (validated/bounded by its type).
    pub fn with_side_data(mut self, sd: VisualSideData) -> Self {
        self.side_data.push(sd);
        self
    }

    /// Epoch id.
    pub fn id(&self) -> EpochId {
        self.id
    }
    /// Coded width.
    pub fn width(&self) -> u32 {
        self.width
    }
    /// Coded height.
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Canonical layout.
    pub fn layout(&self) -> PixelLayout {
        self.layout
    }
    /// Per-plane templates.
    pub fn planes(&self) -> &[PlaneTemplate] {
        &self.planes
    }
    /// Plane count.
    pub fn plane_count(&self) -> usize {
        self.planes.len()
    }
    /// Chroma sample location (from the color description — the single source
    /// of color semantics).
    pub fn chroma_location(&self) -> crate::media::color::ChromaLocation {
        self.color.chroma_location()
    }
    /// Color description.
    pub fn color(&self) -> ColorDescription {
        self.color
    }
    /// Sample aspect ratio.
    pub fn sar(&self) -> SampleAspectRatio {
        self.sar
    }
    /// Orientation.
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }
    /// Field structure.
    pub fn field_structure(&self) -> FieldStructure {
        self.field_structure
    }
    /// Side data.
    pub fn side_data(&self) -> &[VisualSideData] {
        &self.side_data
    }

    /// The authoritative plane dimensions of plane `i` under the epoch's
    /// coded geometry (the normative ceil rule).
    pub fn plane_dimensions(&self, i: usize) -> Result<(u32, u32), VoleError> {
        self.layout.plane_dimensions(i, self.width, self.height)
    }

    /// The sample count of plane `i`.
    pub fn plane_sample_count(&self, i: usize) -> Result<u64, VoleError> {
        self.layout.plane_sample_count(i, self.width, self.height)
    }

    /// Total canonical sample count across every plane of one observation.
    pub fn total_sample_count(&self) -> Result<u64, VoleError> {
        self.layout.total_sample_count(self.width, self.height)
    }

    /// Total canonical storage bytes of one observation (sum of
    /// `sample_count × bytes_per_sample` per plane).
    pub fn observation_bytes(&self) -> Result<u64, VoleError> {
        let mut total = 0u64;
        for (i, tmpl) in self.planes.iter().enumerate() {
            let bytes = self
                .plane_sample_count(i)?
                .checked_mul(tmpl.bit_depth.storage().bytes_per_sample())
                .ok_or(VoleError::ArithmeticOverflow)?;
            total = total
                .checked_add(bytes)
                .ok_or(VoleError::ArithmeticOverflow)?;
        }
        Ok(total)
    }

    /// Check an observation's plane set against this epoch's declarations:
    /// plane count, per-plane component, geometry, bit depth, storage, and
    /// payload length must all match exactly (`GeometryMismatch` /
    /// `InvalidSamples` otherwise).
    pub fn check_observation(&self, obs: &CanonicalVideoObservation) -> Result<(), VoleError> {
        if obs.epoch != self.id {
            return Err(VoleError::EpochViolation);
        }
        if obs.planes.len() != self.planes.len() {
            return Err(VoleError::GeometryMismatch);
        }
        for (i, (tmpl, plane)) in self.planes.iter().zip(obs.planes.iter()).enumerate() {
            let (pw, ph) = self.plane_dimensions(i)?;
            if plane.component() != tmpl.component
                || plane.width() != pw
                || plane.height() != ph
                || plane.bit_depth() != tmpl.bit_depth
                || plane.subsample_x() != tmpl.subsample_x
                || plane.subsample_y() != tmpl.subsample_y
            {
                return Err(VoleError::GeometryMismatch);
            }
            if plane.storage() != tmpl.bit_depth.storage() {
                return Err(VoleError::InvalidSamples);
            }
            if plane.sample_count() != self.plane_sample_count(i)? {
                return Err(VoleError::InvalidSamples);
            }
        }
        Ok(())
    }

    /// A canonical sample payload for plane `i` with every sample at the
    /// declared active depth's midpoint (synthetic vectors; deterministic).
    pub fn synthetic_plane(&self, i: usize) -> Result<Plane, VoleError> {
        let tmpl = self.planes.get(i).ok_or(VoleError::GeometryMismatch)?;
        let (pw, ph) = self.plane_dimensions(i)?;
        let n = usize::try_from(u64::from(pw) * u64::from(ph))
            .map_err(|_| VoleError::ArithmeticOverflow)?;
        let data = match tmpl.bit_depth.storage() {
            PlaneStorage::U8 => PlaneData::U8(vec![0x80; n]),
            PlaneStorage::U16 => {
                let v = u16::try_from(tmpl.bit_depth.max_sample() / 2)
                    .map_err(|_| VoleError::ArithmeticOverflow)?;
                PlaneData::U16(vec![v; n])
            }
        };
        Plane::new(
            tmpl.component,
            pw,
            ph,
            tmpl.bit_depth,
            tmpl.subsample_x,
            tmpl.subsample_y,
            data,
        )
    }
}

/// One canonical observation: its epoch binding, its exact presentation time,
/// its optional exact duration, and the validated per-plane samples.
#[derive(Debug, Clone)]
pub struct CanonicalVideoObservation {
    epoch: EpochId,
    pts: Pts,
    duration: Option<Duration>,
    planes: Vec<Plane>,
}

impl CanonicalVideoObservation {
    /// Build and validate an observation against its epoch.
    pub fn new(
        epoch: &VideoEpoch,
        pts: Pts,
        duration: Option<Duration>,
        planes: Vec<Plane>,
    ) -> Result<Self, VoleError> {
        let obs = CanonicalVideoObservation {
            epoch: epoch.id(),
            pts,
            duration,
            planes,
        };
        epoch.check_observation(&obs)?;
        Ok(obs)
    }

    /// Epoch id.
    pub fn epoch(&self) -> EpochId {
        self.epoch
    }
    /// Presentation timestamp.
    pub fn pts(&self) -> Pts {
        self.pts
    }
    /// Presentation duration (VFR is a per-observation duration).
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }
    /// Planes.
    pub fn planes(&self) -> &[Plane] {
        &self.planes
    }
}

/// A validated canonical video: one or more epochs (dense ids `0..n`) and a
/// presentation-ordered observation sequence over them.
#[derive(Debug, Clone)]
pub struct CanonicalVideo {
    epochs: Vec<VideoEpoch>,
    observations: Vec<CanonicalVideoObservation>,
}

impl CanonicalVideo {
    /// Validate and build a canonical video:
    ///
    /// * at least one epoch; epoch ids dense `0..n` in declaration order
    ///   (else `EpochViolation`);
    /// * every observation binds to a declared epoch and matches its epoch's
    ///   plane table (else `EpochViolation` / `GeometryMismatch`);
    /// * PTS are strictly increasing across the sequence in presentation
    ///   order (else `EpochViolation`).
    pub fn new(
        mut epochs: Vec<VideoEpoch>,
        observations: Vec<CanonicalVideoObservation>,
    ) -> Result<Self, VoleError> {
        if epochs.is_empty() {
            return Err(VoleError::EpochViolation);
        }
        for (i, e) in epochs.iter_mut().enumerate() {
            if e.id != EpochId(i as u64) {
                return Err(VoleError::EpochViolation);
            }
        }
        let mut prev: Option<Pts> = None;
        for obs in &observations {
            let epoch = epochs
                .iter()
                .find(|e| e.id == obs.epoch)
                .ok_or(VoleError::EpochViolation)?;
            epoch.check_observation(obs)?;
            if let Some(p) = prev {
                // Presentation order: strictly increasing PTS.
                if obs.pts.cmp_pts(&p)? != Ordering::Greater {
                    return Err(VoleError::EpochViolation);
                }
            }
            prev = Some(obs.pts);
        }
        Ok(CanonicalVideo {
            epochs,
            observations,
        })
    }

    /// Epochs (declaration order; ids are dense `0..n`).
    pub fn epochs(&self) -> &[VideoEpoch] {
        &self.epochs
    }

    /// Epoch by id.
    pub fn epoch(&self, id: EpochId) -> Option<&VideoEpoch> {
        self.epochs.iter().find(|e| e.id == id)
    }

    /// Observations in presentation order.
    pub fn observations(&self) -> &[CanonicalVideoObservation] {
        &self.observations
    }

    /// Observation count.
    pub fn observation_count(&self) -> u64 {
        self.observations.len() as u64
    }

    /// The epoch of an observation by its index.
    pub fn epoch_of(&self, index: usize) -> Option<&VideoEpoch> {
        let obs = self.observations.get(index)?;
        self.epoch(obs.epoch)
    }

    /// The first observation's PTS.
    pub fn start_pts(&self) -> Option<Pts> {
        self.observations.first().map(|o| o.pts)
    }

    /// The exact presentation end time of the last observation — the last
    /// observation's PTS plus its duration — when every observation carries a
    /// duration. A single missing duration makes the span undefined (`None`),
    /// never a guess.
    pub fn end_pts(&self) -> Option<Pts> {
        let mut end = None;
        for obs in &self.observations {
            end = Some(obs.pts.checked_add(obs.duration?).ok()?);
        }
        end
    }

    /// Total timeline span (last end minus first start) on `at`'s base —
    /// exact, or `None` when any duration is missing; a typed error when the
    /// span is not exactly representable on the requested base.
    pub fn total_span(&self, at: TimeBase) -> Result<Option<Duration>, VoleError> {
        let start = match self.start_pts() {
            Some(p) => p,
            None => return Ok(None),
        };
        let end = match self.end_pts() {
            Some(e) => e,
            None => return Ok(None),
        };
        let span = end.checked_span_from(&start)?;
        Ok(Some(span.rescale(at)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::color::{
        ColorDescription, ColorPrimaries, ColorRange, MatrixCoefficients, TransferCharacteristic,
    };

    fn hdr_epoch(id: u64, w: u32, h: u32, layout: PixelLayout, depth: u8) -> VideoEpoch {
        VideoEpoch::new_uniform(
            EpochId(id),
            w,
            h,
            layout,
            BitDepth::new(depth).unwrap(),
            ColorDescription::bt2020_pq(),
            SampleAspectRatio::square(),
            Orientation::Normal,
            FieldStructure::Progressive,
        )
        .unwrap()
    }

    fn obs_of(epoch: &VideoEpoch, pts: Pts) -> CanonicalVideoObservation {
        let planes = (0..epoch.plane_count())
            .map(|i| epoch.synthetic_plane(i).unwrap())
            .collect();
        CanonicalVideoObservation::new(
            epoch,
            pts,
            Some(Duration::new(1, pts.time_base()).unwrap()),
            planes,
        )
        .unwrap()
    }

    #[test]
    fn epoch_derives_exact_plane_tables() {
        let e = hdr_epoch(0, 1920, 1080, PixelLayout::Yuv420, 10);
        assert_eq!(e.plane_count(), 3);
        assert_eq!(e.plane_dimensions(0).unwrap(), (1920, 1080));
        assert_eq!(e.plane_dimensions(1).unwrap(), (960, 540));
        assert_eq!(e.plane_sample_count(0).unwrap(), 1920 * 1080);
        assert_eq!(e.total_sample_count().unwrap(), 1920 * 1080 + 2 * 960 * 540);
        assert_eq!(
            e.observation_bytes().unwrap(),
            (1920 * 1080 + 2 * 960 * 540) * 2,
            "10-bit storage is u16"
        );
        assert_eq!(e.planes()[0].component, Component::Y);
        assert_eq!(e.color(), ColorDescription::bt2020_pq());
        assert_eq!(e.color().primaries(), ColorPrimaries::Bt2020);
        assert_eq!(e.color().transfer(), TransferCharacteristic::Smpte2084);
        assert_eq!(e.color().matrix(), MatrixCoefficients::Bt2020Ncl);
        assert_eq!(e.color().range(), ColorRange::Limited);
    }

    #[test]
    fn observation_validation_is_exact_and_typed() {
        let e = hdr_epoch(0, 4, 4, PixelLayout::Yuv420, 10);
        let tb = TimeBase::for_frame_rate(24000, 1001).unwrap();
        let pts = Pts::new(0, tb);
        // Correct observation from the epoch's synthetic planes.
        let good = obs_of(&e, pts);
        assert_eq!(good.epoch(), EpochId(0));
        assert_eq!(good.planes().len(), 3);
        // A wrong epoch binding is an epoch violation.
        let mut forged = good.clone();
        forged.epoch = EpochId(7);
        assert_eq!(
            e.check_observation(&forged).unwrap_err(),
            VoleError::EpochViolation
        );
        // Too few planes is a geometry mismatch.
        let mut short = good.clone();
        short.planes.pop();
        assert!(matches!(
            CanonicalVideoObservation::new(&e, pts, None, short.planes).unwrap_err(),
            VoleError::GeometryMismatch
        ));
        // A plane with the wrong geometry (luma size in the chroma slot) is a
        // geometry mismatch.
        let mut wrong_geo = good.clone();
        let luma = wrong_geo.planes[0].clone(); // 4x4 Y plane at depth 10
        let bad = Plane::new(
            Component::Cb,
            4,
            4,
            luma.bit_depth(),
            0,
            0,
            PlaneData::U16(vec![0; 16]),
        )
        .unwrap();
        wrong_geo.planes[1] = bad;
        assert!(matches!(
            e.check_observation(&wrong_geo).unwrap_err(),
            VoleError::GeometryMismatch
        ));
        // A depth-8 plane in a 10-bit slot is invalid samples.
        let mut wrong_depth = good.clone();
        let d8 = BitDepth::new(8).unwrap();
        let (pw, ph) = e.plane_dimensions(2).unwrap();
        wrong_depth.planes[2] = Plane::new(
            Component::Cr,
            pw,
            ph,
            d8,
            1,
            1,
            PlaneData::U8(vec![0; (pw * ph) as usize]),
        )
        .unwrap();
        assert!(matches!(
            e.check_observation(&wrong_depth).unwrap_err(),
            VoleError::GeometryMismatch | VoleError::InvalidSamples
        ));
    }

    #[test]
    fn sequence_validates_presentation_order_and_epoch_changes() {
        let a = hdr_epoch(0, 4, 4, PixelLayout::Yuv420, 10);
        // Epoch change mid-stream: geometry and depth both change (12-bit 4:4:4).
        let b = hdr_epoch(1, 6, 6, PixelLayout::Yuv444, 12);
        let tb = TimeBase::for_frame_rate(30000, 1001).unwrap();
        let mut observations = Vec::new();
        let mut pts = Pts::new(-1001, tb); // nonzero, negative origin
        for k in 0..4u64 {
            let e = if k < 2 { &a } else { &b };
            observations.push(obs_of(e, pts));
            pts = pts.checked_add(Duration::new(1, tb).unwrap()).unwrap();
        }
        let v = CanonicalVideo::new(vec![a.clone(), b.clone()], observations).unwrap();
        assert_eq!(v.epochs().len(), 2);
        assert_eq!(v.observation_count(), 4);
        assert_eq!(v.epoch_of(2).unwrap().layout(), PixelLayout::Yuv444);
        assert_eq!(v.epoch_of(2).unwrap().plane_count(), 3);
        // A reordered (non-increasing PTS) sequence fails typed.
        let mut obs2 = v.observations().to_vec();
        obs2.swap(1, 2);
        assert!(matches!(
            CanonicalVideo::new(vec![a.clone(), b.clone()], obs2).unwrap_err(),
            VoleError::EpochViolation
        ));
        // Unknown epoch id in the observation list fails typed.
        let mut obs3 = v.observations().to_vec();
        obs3[0].epoch = EpochId(9);
        assert!(matches!(
            CanonicalVideo::new(vec![a.clone(), b.clone()], obs3).unwrap_err(),
            VoleError::EpochViolation
        ));
        // Span is exact: 4 observations of 1 tick at 30000/1001 -> 4 ticks.
        let span = v.total_span(tb).unwrap().unwrap();
        assert_eq!(span.value(), 4);
        assert_eq!(span.time_base(), tb);
        // End PTS: start −1001 + 4 ticks = −997.
        assert_eq!(v.end_pts().unwrap().value(), -997);
    }

    #[test]
    fn epoch_ids_must_be_dense() {
        let a = hdr_epoch(5, 4, 4, PixelLayout::Yuv420, 10); // wrong id
        let tb = TimeBase::ticks_per_second(25).unwrap();
        let obs = obs_of(&a, Pts::new(0, tb));
        assert!(matches!(
            CanonicalVideo::new(vec![a], vec![obs]).unwrap_err(),
            VoleError::EpochViolation
        ));
        // Missing durations make the span undefined, never guessed.
        let e = hdr_epoch(0, 4, 4, PixelLayout::Gray, 8);
        let p = Pts::new(0, tb);
        let planes = (0..e.plane_count())
            .map(|i| e.synthetic_plane(i).unwrap())
            .collect();
        let o = CanonicalVideoObservation::new(&e, p, None, planes).unwrap();
        let v = CanonicalVideo::new(vec![e], vec![o]).unwrap();
        assert_eq!(v.total_span(tb).unwrap(), None);
        assert_eq!(v.end_pts(), None);
    }
}
