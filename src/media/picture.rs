//! Multi-plane picture buffer — Phase V.1.2 (V.1 video programme, contract
//! §2.4, §2.6).
//!
//! A [`Picture`] is the materializable canvas of one epoch: an ordered set of
//! canonical planes whose geometry/component/depth table matches the epoch.
//! Sample access is in the **u32 sample domain** (the value space of the
//! plane's active bit depth); writes validate against the active depth, so a
//! picture is canonical by type. V.1.2 models **independent planes** (§46):
//! each plane is proceduralized and materialized separately; cross-plane
//! shared hypotheses are later-subphase work.

use crate::error::VoleError;
use crate::media::epoch::VideoEpoch;
use crate::media::plane::{Plane, PlaneData, PlaneStorage};

/// The materializable multi-plane canvas of one epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    planes: Vec<Plane>,
}

impl Picture {
    /// A picture whose planes exactly match `epoch`'s declared plane table,
    /// every sample set to the given per-plane background value.
    pub fn from_epoch(epoch: &VideoEpoch, backgrounds: &[u32]) -> Result<Self, VoleError> {
        if backgrounds.len() != epoch.plane_count() {
            return Err(VoleError::GeometryMismatch);
        }
        let mut planes = Vec::with_capacity(epoch.plane_count());
        for (i, tmpl) in epoch.planes().iter().enumerate() {
            let (pw, ph) = epoch.plane_dimensions(i)?;
            let n = usize::try_from(u64::from(pw) * u64::from(ph))
                .map_err(|_| VoleError::ArithmeticOverflow)?;
            let bg = backgrounds[i];
            if bg > tmpl.bit_depth.max_sample() {
                return Err(VoleError::InvalidSamples);
            }
            let data = match tmpl.bit_depth.storage() {
                PlaneStorage::U8 => PlaneData::U8(vec![bg as u8; n]),
                PlaneStorage::U16 => PlaneData::U16(vec![bg as u16; n]),
            };
            planes.push(Plane::new(
                tmpl.component,
                pw,
                ph,
                tmpl.bit_depth,
                tmpl.subsample_x,
                tmpl.subsample_y,
                data,
            )?);
        }
        Ok(Picture { planes })
    }

    /// A picture from already-built planes (the plane table must match
    /// `epoch` exactly — validated here against the epoch's own table).
    pub fn from_planes(epoch: &VideoEpoch, planes: Vec<Plane>) -> Result<Self, VoleError> {
        let pic = Picture { planes };
        pic.validate_against(epoch)?;
        Ok(pic)
    }

    /// Check the plane table against an epoch's declaration: count, per-plane
    /// component, geometry, bit depth, and payload lengths must all match.
    pub fn validate_against(&self, epoch: &VideoEpoch) -> Result<(), VoleError> {
        if self.planes.len() != epoch.plane_count() {
            return Err(VoleError::GeometryMismatch);
        }
        for (i, (tmpl, plane)) in epoch.planes().iter().zip(self.planes.iter()).enumerate() {
            let (pw, ph) = epoch.plane_dimensions(i)?;
            if plane.component() != tmpl.component
                || plane.width() != pw
                || plane.height() != ph
                || plane.bit_depth() != tmpl.bit_depth
                || plane.subsample_x() != tmpl.subsample_x
                || plane.subsample_y() != tmpl.subsample_y
            {
                return Err(VoleError::GeometryMismatch);
            }
            if plane.sample_count()
                != u64::from(pw)
                    .checked_mul(u64::from(ph))
                    .ok_or(VoleError::ArithmeticOverflow)?
            {
                return Err(VoleError::InvalidSamples);
            }
        }
        Ok(())
    }

    /// Plane count.
    pub fn plane_count(&self) -> usize {
        self.planes.len()
    }

    /// Plane `i`.
    pub fn plane(&self, i: usize) -> Option<&Plane> {
        self.planes.get(i)
    }

    /// Consume into the plane list.
    pub fn into_planes(self) -> Vec<Plane> {
        self.planes
    }

    /// The planes (borrowed).
    pub fn planes(&self) -> &[Plane] {
        &self.planes
    }

    /// Sample read in the u32 sample domain.
    pub fn get(&self, plane: usize, x: u32, y: u32) -> Option<u32> {
        let p = self.planes.get(plane)?;
        let w = p.width();
        if x >= w || y >= p.height() {
            return None;
        }
        let i = (y * w + x) as usize;
        Some(match p.data() {
            PlaneData::U8(v) => u32::from(v[i]),
            PlaneData::U16(v) => u32::from(v[i]),
        })
    }

    /// Sample write in the u32 sample domain. The value must fit the plane's
    /// active depth and the coordinate must be in bounds (typed otherwise).
    pub fn put(&mut self, plane: usize, x: u32, y: u32, value: u32) -> Result<(), VoleError> {
        let p = self.planes.get_mut(plane).ok_or(VoleError::OutOfBounds)?;
        let w = p.width();
        let depth = p.bit_depth();
        if x >= w || y >= p.height() {
            return Err(VoleError::OutOfBounds);
        }
        if value > depth.max_sample() {
            return Err(VoleError::InvalidSamples);
        }
        let i = (y * w + x) as usize;
        match p.data_mut() {
            PlaneData::U8(v) => v[i] = value as u8,
            PlaneData::U16(v) => v[i] = value as u16,
        }
        Ok(())
    }

    /// Fill a clipped axis-aligned rectangle of one plane with a sample value
    /// (canonical clip semantics: out-of-canvas regions are dropped).
    pub fn fill_rect_clipped(
        &mut self,
        plane: usize,
        value: u32,
        x0: i64,
        y0: i64,
        x1: i64,
        y1: i64,
    ) -> Result<(), VoleError> {
        let (cw, ch, depth) = {
            let p = self.planes.get(plane).ok_or(VoleError::OutOfBounds)?;
            (i64::from(p.width()), i64::from(p.height()), p.bit_depth())
        };
        if value > depth.max_sample() {
            return Err(VoleError::InvalidSamples);
        }
        let x0 = x0.clamp(0, cw);
        let y0 = y0.clamp(0, ch);
        let x1 = x1.clamp(x0, cw);
        let y1 = y1.clamp(y0, ch);
        let w = usize::try_from(cw).unwrap();
        let p = self.planes.get_mut(plane).expect("checked above");
        for y in y0..y1 {
            let s = usize::try_from(y).unwrap() * w + usize::try_from(x0).unwrap();
            let n = usize::try_from(x1 - x0).unwrap();
            match p.data_mut() {
                PlaneData::U8(v) => v[s..s + n].fill(value as u8),
                PlaneData::U16(v) => v[s..s + n].fill(value as u16),
            }
        }
        Ok(())
    }

    /// Canonical overwrite blit of a `sw × sh` source sample box (u32 domain,
    /// tight row-major) onto plane `p` with its top-left at `(dx, dy)`,
    /// clipped at the borders (mirrors the v1 `Canvas::blit` destination rule:
    /// iterate destination rows in canvas range, map to source rows).
    pub fn blit(
        &mut self,
        plane: usize,
        src: &[u32],
        sw: u32,
        sh: u32,
        dx: i64,
        dy: i64,
    ) -> Result<(), VoleError> {
        if src.len() as u64 != u64::from(sw) * u64::from(sh) {
            return Err(VoleError::InvalidSamples);
        }
        let (cw, ch, depth) = {
            let p = self.planes.get(plane).ok_or(VoleError::OutOfBounds)?;
            (i64::from(p.width()), i64::from(p.height()), p.bit_depth())
        };
        let max = depth.max_sample();
        if src.iter().any(|v| *v > max) {
            return Err(VoleError::InvalidSamples);
        }
        let y0 = dy.max(0);
        let y1 = (dy + i64::from(sh)).min(ch);
        let x0 = dx.max(0);
        let x1 = (dx + i64::from(sw)).min(cw);
        if y0 >= y1 || x0 >= x1 {
            return Ok(());
        }
        let w = usize::try_from(cw).unwrap();
        let p = self.planes.get_mut(plane).expect("checked above");
        for cty in y0..y1 {
            let sy = (cty - dy) as usize;
            let row_src = &src[sy * (sw as usize)..(sy + 1) * (sw as usize)];
            for ctox in x0..x1 {
                let sx = (ctox - dx) as usize;
                let v = row_src[sx];
                let i = usize::try_from(cty).unwrap() * w + usize::try_from(ctox).unwrap();
                match p.data_mut() {
                    PlaneData::U8(d) => d[i] = v as u8,
                    PlaneData::U16(d) => d[i] = v as u16,
                }
            }
        }
        Ok(())
    }

    /// Total canonical storage bytes over every plane.
    pub fn total_bytes(&self) -> u64 {
        self.planes
            .iter()
            .map(|p| p.sample_count() * p.storage().bytes_per_sample())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::color::ColorDescription;
    use crate::media::epoch::EpochId;
    use crate::media::meta::{FieldStructure, Orientation, SampleAspectRatio};
    use crate::media::plane::BitDepth;
    use crate::media::PixelLayout;

    fn epoch(layout: PixelLayout, depth: u8) -> VideoEpoch {
        VideoEpoch::new_uniform(
            EpochId(0),
            6,
            4,
            layout,
            BitDepth::new(depth).unwrap(),
            ColorDescription::unspecified(),
            SampleAspectRatio::square(),
            Orientation::Normal,
            FieldStructure::Progressive,
        )
        .unwrap()
    }

    #[test]
    fn picture_sample_domain_is_depth_exact() {
        let e = epoch(PixelLayout::Yuv420, 10);
        let mut pic = Picture::from_epoch(&e, &[0, 512, 512]).unwrap();
        assert_eq!(pic.plane_count(), 3);
        assert_eq!(pic.plane(0).unwrap().width(), 6);
        assert_eq!(pic.plane(1).unwrap().width(), 3);
        pic.put(0, 1, 1, 1023).unwrap();
        assert_eq!(pic.get(0, 1, 1), Some(1023));
        // Above the active depth is refused.
        assert_eq!(
            pic.put(0, 0, 0, 1024).unwrap_err(),
            VoleError::InvalidSamples
        );
        // Out of bounds is refused.
        assert_eq!(pic.put(0, 6, 0, 0).unwrap_err(), VoleError::OutOfBounds);
        // Fill respects clipping.
        pic.fill_rect_clipped(0, 100, -2, -2, 100, 100).unwrap();
        for y in 0..4u32 {
            for x in 0..6u32 {
                assert_eq!(pic.get(0, x, y), Some(100));
            }
        }
        // Blit with clipping; the out-of-canvas part is dropped.
        let src: Vec<u32> = (0..24u32).collect();
        pic.blit(0, &src, 6, 4, -1, 0).unwrap();
        for x in 0..6u32 {
            // Source row 0 shifted left by 1: dest x gets src x+1, x=5 clipped.
            let expect = if x + 1 < 6 { x + 1 } else { 100 };
            assert_eq!(pic.get(0, x, 0), Some(expect));
        }
        // U8 picture rejects u16-style values.
        let e8 = epoch(PixelLayout::Gray, 8);
        let mut p8 = Picture::from_epoch(&e8, &[7]).unwrap();
        assert_eq!(p8.put(0, 0, 0, 255).unwrap(), ());
        assert_eq!(p8.put(0, 0, 0, 256).unwrap_err(), VoleError::InvalidSamples);
        assert_eq!(p8.get(0, 0, 0), Some(255));
        // A Blit with an invalid source value is refused before any write.
        let mut p = Picture::from_epoch(&e, &[0, 0, 0]).unwrap();
        assert_eq!(
            p.blit(0, &[1024], 1, 1, 0, 0).unwrap_err(),
            VoleError::InvalidSamples
        );
    }

    #[test]
    fn picture_validates_against_epoch() {
        let e = epoch(PixelLayout::Yuv444, 8);
        let pic = Picture::from_epoch(&e, &[0, 0, 0]).unwrap();
        let bad = epoch(PixelLayout::Yuv420, 8);
        assert!(pic.validate_against(&bad).is_err());
        assert!(pic.validate_against(&e).is_ok());
        // Background values validate per plane.
        let e10 = epoch(PixelLayout::Yuv420, 10);
        assert!(Picture::from_epoch(&e10, &[0, 1023, 0]).is_ok());
        assert!(Picture::from_epoch(&e10, &[0, 1024, 0]).is_err());
        // from_planes validates the count against the epoch.
        let planes = pic.planes().to_vec();
        assert!(Picture::from_planes(&e, planes).is_ok());
    }
}
