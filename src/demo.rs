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

/// Phase-B static-scene court: a persistent object that never moves across many
/// intervals. It demonstrates the **unchanged lane**: each interval advances
/// time with zero transitions (nothing happened is a first-class, cheap
/// representation), and it measures the *amortized* per-frame overhead of
/// keeping that unchanged state persistent.
pub struct StaticSceneCourt {
    /// Canvas width.
    pub width: u32,
    /// Canvas height.
    pub height: u32,
    /// Declared object id.
    pub object_id: u32,
    /// Declared instance id.
    pub instance_id: u32,
    /// Number of static intervals.
    pub intervals: u64,
}

impl Default for StaticSceneCourt {
    fn default() -> Self {
        StaticSceneCourt {
            width: 1920,
            height: 1080,
            object_id: 1,
            instance_id: 1,
            intervals: 10_000,
        }
    }
}

impl StaticSceneCourt {
    /// VOLE bytes: one persistent object + one instance at a fixed position and
    /// only *empty* interval groups (unchanged state).
    pub fn vole(&self) -> Result<Vec<u8>, VoleError> {
        let obj = Object::fill(320, 40, 90)?; // a persistent UI strip, say
        let inst = Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: 0,
            y: 20,
        };
        let mut timeline = Vec::with_capacity(self.intervals as usize);
        for k in 1..=self.intervals {
            timeline.push((k, Vec::new())); // empty: unchanged state lane
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

    /// Materialize; every frame must equal the checkpoint view (static).
    pub fn frames(&self) -> Result<Vec<Canvas>, VoleError> {
        let parsed = decoder::decode_bytes(&self.vole()?)?;
        decoder::materialize_all(&parsed)
    }

    /// All materialized frames must be identical (an unchanged frame is a
    /// materializable view of persistent state, not a repeated raster store).
    pub fn verify_static(&self) -> Result<u64, VoleError> {
        let parsed = decoder::decode_bytes(&self.vole()?)?;
        let frames = decoder::materialize_all(&parsed)?;
        let f0 = frames.first().expect("checkpoint frame");
        let mut unchanged = 0u64;
        for f in &frames {
            if f.exactly_matches(f0) {
                unchanged += 1;
            }
        }
        assert_eq!(unchanged, frames.len() as u64);
        if unchanged != frames.len() as u64 {
            return Err(VoleError::ApiConstraint("static scene diverged"));
        }
        Ok(parsed.frame_count())
    }

    /// Report (stream_bytes, frame_count, raw_all_bytes).
    pub fn account(&self) -> Result<(u64, u64, u64), VoleError> {
        let bytes = self.vole()?;
        let flows = self.verify_static()?;
        let raw = u64::from(self.width) * u64::from(self.height) * flows;
        Ok((bytes.len() as u64, flows, raw))
    }
}

/// Phase-C sparse-mutation story: a persistent object with a small blinking
/// overlay pixel whose value flips each interval. Verifies sparse overlay
/// points materialize exactly and are cheap to represent (no full frame re-
/// store).
pub struct BlinkCourt {
    /// Canvas dimensions.
    pub width: u32,
    pub height: u32,
    /// Object id/instance id.
    pub object_id: u32,
    pub instance_id: u32,
    /// Overlay coordinate that blinks every interval.
    pub px: i64,
    pub py: i64,
    /// Number of intervals (frames == intervals + 1).
    pub intervals: u64,
}

impl Default for BlinkCourt {
    fn default() -> Self {
        BlinkCourt {
            width: 640,
            height: 360,
            object_id: 1,
            instance_id: 1,
            px: 50,
            py: 20,
            intervals: 64,
        }
    }
}

impl BlinkCourt {
    /// VOLE bytes (sparse overlay toggles each interval).
    pub fn vole(&self) -> Result<Vec<u8>, VoleError> {
        let obj = Object::fill(400, 300, 128)?;
        let inst = Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: 0,
            y: 0,
        };
        let mut timeline = Vec::with_capacity(self.intervals as usize);
        for k in 1..=self.intervals {
            let value: u8 = if (k % 2) == 1 { 0 } else { 255 };
            timeline.push((
                k,
                vec![Transition::PatchSparse {
                    points: vec![(self.px, self.py, value)],
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

    /// Independent reference frames (patches turn the pixel on/off).
    pub fn reference_raw(&self) -> Vec<u8> {
        let obj = Object::fill(400, 300, 128).expect("fill");
        let raster = obj.expand();
        let w = self.width as usize;
        let mut raw = Vec::new();
        for f in 0..=self.intervals {
            // frame f: object drawn; then (if f>0) overlay value at parity of f.
            let mut fram = vec![0u8; w * self.height as usize];
            self.blit(&mut fram, &raster, 400, 300, 0, 0);
            if f > 0 {
                let v: u8 = if (f % 2) == 0 { 255 } else { 0 };
                self.setpix(&mut fram, self.px, self.py, v, w);
            }
            raw.extend(fram);
        }
        raw
    }

    fn blit(&self, fram: &mut [u8], src: &[u8], sw: u32, sh: u32, dx: i64, dy: i64) {
        let w = self.width as i64;
        let h = self.height as i64;
        for sy in 0..sh as i64 {
            for sx in 0..sw as i64 {
                let cx = dx + sx;
                let cy = dy + sy;
                if cx >= 0 && cx < w && cy >= 0 && cy < h {
                    fram[cy as usize * (self.width as usize) + cx as usize] =
                        src[(sy as usize) * (sw as usize) + sx as usize];
                }
            }
        }
    }

    fn setpix(&self, fram: &mut [u8], x: i64, y: i64, v: u8, w: usize) {
        if x >= 0 && y >= 0 && x < i64::from(self.width) && y < i64::from(self.height) {
            fram[y as usize * w + x as usize] = v;
        }
    }

    /// Materialize and byte-exact verify against the reference.
    pub fn materialize_and_verify(&self) -> Result<Vec<Canvas>, VoleError> {
        let parsed = decoder::decode_bytes(&self.vole()?)?;
        let canvas = decoder::materialize_all(&parsed)?;
        let mut got = Vec::new();
        for c in &canvas {
            got.extend_from_slice(c.as_slice());
        }
        if got != self.reference_raw() {
            return Err(VoleError::ApiConstraint("blink diverged"));
        }
        Ok(canvas)
    }
}
