//! Deterministic synthetic procedural sources used for the Phase-A court and
//! the CLI demo. Each source yields canonical `.vole` bytes plus an
//! **independent** reference Gray8 raster (a separate painter loop, so byte
//! equality with the materializer is meaningful evidence).

use crate::{
    decoder, encoder,
    error::VoleError,
    format::ParsedStream,
    object::{Object, ObjectId},
    pixel::Canvas,
    state::{Instance, InstanceId},
    transition::Transition,
};

/// The classic first proof (§76): a 200×100 object whose single instance moves
/// `+2` in x each interval from `x=100`, over 100 intervals on a 1920×1080
/// canvas (101 frames), stored as one object + one instance + one checkpoint +
/// 100 minimal transitions — never 101 stored rasters.
pub struct MovingRectCourt {
    /// Canvas width.
    pub width: u32,
    /// Canvas height.
    pub height: u32,
    /// Object box width.
    pub box_w: u32,
    /// Object box height.
    pub box_h: u32,
    /// Instance start x.
    pub x_start: i64,
    /// Per-interval x velocity.
    pub velocity: i64,
    /// Declared object id.
    pub object_id: u32,
    /// Declared instance id.
    pub instance_id: u32,
    /// Number of intervals (frames == intervals + 1).
    pub intervals: u64,
}

impl Default for MovingRectCourt {
    fn default() -> Self {
        MovingRectCourt {
            width: 1920,
            height: 1080,
            box_w: 200,
            box_h: 100,
            x_start: 100,
            velocity: 2,
            object_id: 7,
            instance_id: 1,
            intervals: 100,
        }
    }
}

/// A deliberately independent canonical painter: clear the canvas to the
/// background, then overwrite each object box (clipped) in paint order. The
/// loops differ structurally from the materializer's `Canvas::blit` so a shared
/// blit bug cannot mask a mismatch in this conformance court.
fn reference_painter(
    width: u32,
    height: u32,
    bg: u8,
    places: &[(i64, i64, &[u8], u32, u32)],
) -> Vec<u8> {
    let w = width as usize;
    let mut out = vec![bg; w * height as usize];
    for (px, py, obj, ow, oh) in places {
        let bw = *ow as usize;
        let bh = *oh as usize;
        for sy in 0..bh {
            let cy = *py + sy as i64;
            if cy < 0 || cy >= i64::from(height) {
                continue;
            }
            let src_row = sy * bw;
            let dst_row = cy as usize * w;
            for sx in 0..bw {
                let cx = *px + sx as i64;
                if cx < 0 || cx >= i64::from(width) {
                    continue;
                }
                out[dst_row + cx as usize] = obj[src_row + sx];
            }
        }
    }
    out
}

impl MovingRectCourt {
    /// Canonical `.vole` bytes of the court stream.
    pub fn vole(&self) -> Result<Vec<u8>, VoleError> {
        let obj = Object::fill(self.box_w, self.box_h, 180)?;
        let inst = Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: self.x_start,
            y: 0,
        };
        let mut timeline = Vec::with_capacity(self.intervals as usize);
        for k in 1..=self.intervals {
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(self.instance_id),
                    x: self.x_start + self.velocity * (k as i64),
                    y: 0,
                }],
            ));
        }
        encoder::encode_stream(
            self.width,
            self.height,
            0,
            &[(self.object_id, obj)],
            &[inst],
            &timeline,
        )
    }

    /// Independent reference `.raw` bytes: concatenated full frames.
    pub fn reference_raw(&self) -> Vec<u8> {
        let obj = Object::fill(self.box_w, self.box_h, 180).expect("fill fits");
        let raster = obj.expand();
        let mut raw = Vec::new();
        for f in 0..=self.intervals {
            let x = self.x_start + self.velocity * (f as i64);
            raw.extend(reference_painter(
                self.width,
                self.height,
                0,
                &[(x, 0, &raster, self.box_w, self.box_h)],
            ));
        }
        raw
    }

    /// Parse + materialize the `.vole`, then verify byte-for-byte against the
    /// independent reference. Returns the canvases on exact match.
    pub fn materialize_and_verify(&self) -> Result<Vec<Canvas>, VoleError> {
        let bytes = self.vole()?;
        let parsed: ParsedStream = decoder::decode_bytes(&bytes)?;
        let canvases = decoder::materialize_all(&parsed)?;
        let mut got = Vec::new();
        for c in &canvases {
            got.extend_from_slice(c.as_slice());
        }
        let expect = self.reference_raw();
        if got.len() != expect.len() || got != expect {
            return Err(VoleError::ApiConstraint(
                "materialized output diverges from reference",
            ));
        }
        Ok(canvases)
    }

    /// `.vole` byte length.
    pub fn vole_size(&self) -> Result<u64, VoleError> {
        Ok(self.vole()?.len() as u64)
    }

    /// Number of materialized frames.
    pub fn frame_count(&self) -> u64 {
        self.intervals + 1
    }

    /// Total raw raster bytes of the canonical frame sequence.
    pub fn raw_bytes_all(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * self.frame_count()
    }
}
