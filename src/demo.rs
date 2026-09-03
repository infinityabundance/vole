//! Deterministic synthetic procedural sources used for the Phase-A court and
//! the CLI demo. Each source yields canonical `.vole` bytes plus an
//! **independent** reference Gray8 raster (a separate painter loop, so byte
//! equality with the materializer is meaningful evidence).

use crate::{
    affine::{AffineParams, AFFINE_SCALE},
    decoder, encoder,
    error::VoleError,
    format::ParsedStream,
    object::{Object, ObjectId},
    pixel::Canvas,
    state::{Instance, InstanceId, PaletteId},
    trajectory::{self, TrajectorySegment},
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

/// Phase-D scroll court: a canvas whose *whole content* wraps vertically by `S`
/// rows each interval. This is expressed with **exactly two COPY_RECT** ops per
/// interval (recycle `[S..H)` to the top and wrap `[0..S)` to the bottom), so
/// COPY_RECT is genuinely load-bearing: intermediate frames are *not*
/// reproducible from immutable painter State (the State is unchanged across
/// intervals), yet every frame is reproduced byte-exactly by replaying the two
/// rectangles. Oracle: `row y of frame t == initRow[(y + t·S) mod H]`.
pub struct ScrollCourt {
    /// Canvas width.
    pub width: u32,
    /// Canvas height.
    pub height: u32,
    /// Rows scrolled (wrapped) each interval.
    pub scroll: u32,
    /// Number of intervals (frames == intervals + 1).
    pub intervals: u32,
    /// Declared object id framing a non-periodic initial raster.
    pub object_id: u32,
}

impl Default for ScrollCourt {
    fn default() -> Self {
        // A modest screen (96x96) so per-frame descriptor-vs-frame ratio is
        // honest: COPY_RECT per-interval cost is size-independent (~2 rects),
        // so larger screens amortize better; 96x96 keeps the byte count to
        // report representative without being trivially large.
        ScrollCourt {
            width: 96,
            height: 96,
            scroll: 3,
            intervals: 12,
            object_id: 7,
        }
    }
}

impl ScrollCourt {
    fn initial_raster(&self) -> Vec<u8> {
        // Row `r` is filled with its own index value (non-periodic vertically);
        // rows thus wrap-disambiguate every step.
        let w = self.width as usize;
        let h = self.height as usize;
        let mut data = Vec::with_capacity(w * h);
        for r in 0..h {
            let v = (r % 256) as u8;
            for _ in 0..w {
                data.push(v);
            }
        }
        data
    }

    /// VOLE bytes: an immutable ruler object + a one-instance checkpoint; each
    /// interval carries exactly two COPY_RECT ops that wrap-scroll the previous
    /// frame up by `self.scroll` rows.
    pub fn vole(&self) -> Result<Vec<u8>, VoleError> {
        let obj = Object::raster(self.width, self.height, self.initial_raster())?;
        let inst = Instance {
            id: InstanceId(1),
            object_id: ObjectId(self.object_id),
            x: 0,
            y: 0,
        };
        let h = self.height as i64;
        let s = self.scroll as i64;
        let mut tl: Vec<(u64, Vec<Transition>)> = Vec::with_capacity(self.intervals as usize);
        for k in 1u64..=u64::from(self.intervals) {
            // Shift up: recycle prior rows [s..h) into the destination top.
            let up = Transition::CopyRect {
                src_x: 0,
                src_y: s,
                width: self.width,
                height: (h - s) as u32,
                dst_x: 0,
                dst_y: 0,
            };
            // Wrap: recycle prior rows [0..s) to the bottom (vertical wrap).
            let wrap = Transition::CopyRect {
                src_x: 0,
                src_y: 0,
                width: self.width,
                height: s as u32,
                dst_x: 0,
                dst_y: h - s,
            };
            tl.push((k, vec![up, wrap]));
        }
        encoder::encode_stream(
            self.width,
            self.height,
            0,
            &[(self.object_id, obj)],
            &[inst],
            &tl,
        )
    }

    /// Independent oracle `.raw` bytes built strictly from the documented
    /// mapping (no reliance on the compositor internals).
    pub fn reference_raw(&self) -> Vec<u8> {
        let init = self.initial_raster();
        let w = self.width as usize;
        let h = self.height as usize;
        let s = self.scroll as usize;
        let mut raw = Vec::new();
        for t in 0..=self.intervals as usize {
            // row y of frame t <- init row (y + t*s) mod h.
            for y in 0..h {
                let src_row = (y + t * s) % h;
                raw.extend_from_slice(&init[src_row * w..src_row * w + w]);
            }
        }
        raw
    }

    /// Materialize and byte-exact compare to the independent oracle.
    pub fn materialize_and_verify(&self) -> Result<Vec<Canvas>, VoleError> {
        let bytes = self.vole()?;
        let parsed = decoder::decode_bytes(&bytes)?;
        let frames = decoder::materialize_all(&parsed)?;
        let mut got = Vec::new();
        for c in &frames {
            got.extend_from_slice(c.as_slice());
        }
        if got != self.reference_raw() {
            return Err(VoleError::ApiConstraint("scroll diverged from oracle"));
        }
        Ok(frames)
    }

    /// Number of materializable frames.
    pub fn frame_count(&self) -> u64 {
        1 + u64::from(self.intervals)
    }

    /// Total raw raster bytes of the canonical frame sequence.
    pub fn raw_bytes_all(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * self.frame_count()
    }
}

/// Phase-E integer-translation court: a persistent object whose instance
/// carries a persistent integer translation `(vx, vy)`, so the stream is
/// `position(t+1) = position(t) + (vx, vy)`. The representation is stored as
/// one `SetVelocity` plus one tiny `AdvanceTranslations` per interval — *not*
/// as per-frame absolute `SetPosition` coordinate payloads, and never as
/// repeated frame rasters.
pub struct TranslationCourt {
    /// Canvas width.
    pub width: u32,
    /// Canvas height.
    pub height: u32,
    /// Object box width.
    pub box_w: u32,
    /// Object box height.
    pub box_h: u32,
    /// Object fill value.
    pub value: u8,
    /// Object / instance ids.
    pub object_id: u32,
    pub instance_id: u32,
    /// Start position.
    pub x0: i64,
    pub y0: i64,
    /// Persistent integer translation (per interval).
    pub vx: i64,
    pub vy: i64,
    /// Number of intervals (frames == intervals + 1).
    pub intervals: u64,
}

impl Default for TranslationCourt {
    fn default() -> Self {
        TranslationCourt {
            width: 1920,
            height: 1080,
            box_w: 200,
            box_h: 100,
            value: 180,
            object_id: 7,
            instance_id: 1,
            x0: 100,
            y0: 60,
            vx: 2,
            vy: 1,
            intervals: 100,
        }
    }
}

impl TranslationCourt {
    fn object(&self) -> Object {
        Object::fill(self.box_w, self.box_h, self.value).expect("box fits")
    }

    fn instance_at(&self, k: u64) -> Instance {
        Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: self.x0 + self.vx * (k as i64),
            y: self.y0 + self.vy * (k as i64),
        }
    }

    /// Canonical `.vole` bytes using the persistent-translation representation:
    /// one `SetVelocity`, then `AdvanceTranslations` per interval.
    pub fn vole(&self) -> Result<Vec<u8>, VoleError> {
        let mut timeline: Vec<(u64, Vec<Transition>)> = Vec::new();
        for k in 1..=self.intervals {
            let group = if k == 1 {
                vec![
                    Transition::SetVelocity {
                        id: InstanceId(self.instance_id),
                        vx: self.vx,
                        vy: self.vy,
                    },
                    Transition::AdvanceTranslations,
                ]
            } else {
                vec![Transition::AdvanceTranslations]
            };
            timeline.push((k, group));
        }
        encoder::encode_stream(
            self.width,
            self.height,
            0,
            &[(self.object_id, self.object())],
            &[self.instance_at(0)],
            &timeline,
        )
    }

    /// Independent reference `.raw` frames (box painted at `(x0+vx*k, y0+vy*k)`).
    pub fn reference_raw(&self) -> Vec<u8> {
        let raster = self.object().expand();
        let mut raw = Vec::new();
        for k in 0..=self.intervals {
            let inst = self.instance_at(k);
            raw.extend(reference_painter(
                self.width,
                self.height,
                0,
                &[(inst.x, inst.y, &raster, self.box_w, self.box_h)],
            ));
        }
        raw
    }

    /// Materialize and byte-exact verify against the independent reference.
    pub fn materialize_and_verify(&self) -> Result<Vec<Canvas>, VoleError> {
        let parsed = decoder::decode_bytes(&self.vole()?)?;
        let frames = decoder::materialize_all(&parsed)?;
        let mut got = Vec::new();
        for c in &frames {
            got.extend_from_slice(c.as_slice());
        }
        if got != self.reference_raw() {
            return Err(VoleError::ApiConstraint(
                "translation diverged from reference",
            ));
        }
        Ok(frames)
    }

    /// The equivalent per-frame absolute `SetPosition` stream (baseline for
    /// the byte comparison; same frames, no persistent translation state).
    pub fn delta_baseline_bytes(&self) -> Result<Vec<u8>, VoleError> {
        let mut timeline = Vec::new();
        for k in 1..=self.intervals {
            let inst = self.instance_at(k);
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(self.instance_id),
                    x: inst.x,
                    y: inst.y,
                }],
            ));
        }
        encoder::encode_stream(
            self.width,
            self.height,
            0,
            &[(self.object_id, self.object())],
            &[self.instance_at(0)],
            &timeline,
        )
    }

    /// Number of materializable frames.
    pub fn frame_count(&self) -> u64 {
        1 + self.intervals
    }

    /// Total raw raster bytes of the canonical frame sequence.
    pub fn raw_bytes_all(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * self.frame_count()
    }
}

/// Exactness gate for a translation hypothesis: the hypothesis `(x0 + vx*k,
/// y0 + vy*k)` must reproduce every target position. If any frame disagrees,
/// a translation-only representation cannot be lossless and must be rejected
/// (this is the Phase-E negative-control gate; the full candidate court is
/// Phase G).
pub fn translation_hypothesis_exact(
    x0: i64,
    y0: i64,
    vx: i64,
    vy: i64,
    positions: &[(i64, i64)],
) -> bool {
    positions
        .iter()
        .enumerate()
        .all(|(k, (x, y))| *x == x0 + vx * (k as i64) && *y == y0 + vy * (k as i64))
}

/// Phase-I direct-procedural court: a parametric trajectory drives one
/// instance. The default content is the accelerating analogue of §76 — one
/// 200×100 box on a 1920×1080 Gray8 canvas whose velocity grows by `(1,0)`
/// every interval — stored as *one object, one instance, one checkpoint, one
/// trajectory program* and stepped by one-byte advances, never as per-frame
/// rasters or per-frame coordinate payloads.
///
/// The reference raster is an **independent** painter driven by a closed-form
/// position table (`trajectory::simulate_positions`), so a shared stepping bug
/// cannot mask a mismatch: `materialize_and_verify` proves the normative
/// materializer reproduces the same positions the closed form predicts.
pub struct TrajectoryCourt {
    /// Canvas width.
    pub width: u32,
    /// Canvas height.
    pub height: u32,
    /// Object box width.
    pub box_w: u32,
    /// Object box height.
    pub box_h: u32,
    /// Object fill value.
    pub value: u8,
    /// Object / instance ids.
    pub object_id: u32,
    pub instance_id: u32,
    /// Start position.
    pub x0: i64,
    pub y0: i64,
    /// Trajectory program; its total step count must equal `intervals`.
    pub segments: Vec<TrajectorySegment>,
    /// Number of intervals (frames == intervals + 1).
    pub intervals: u64,
}

impl Default for TrajectoryCourt {
    fn default() -> Self {
        TrajectoryCourt {
            width: 1920,
            height: 1080,
            box_w: 200,
            box_h: 100,
            value: 180,
            object_id: 7,
            instance_id: 1,
            x0: 100,
            y0: 60,
            // v(t) = (2 + t, 1): constant acceleration (1, 0) per interval.
            segments: vec![TrajectorySegment::Accel {
                vx0: 2,
                vy0: 1,
                ax: 1,
                ay: 0,
                steps: 40,
            }],
            intervals: 40,
        }
    }
}

impl TrajectoryCourt {
    fn object(&self) -> Object {
        Object::fill(self.box_w, self.box_h, self.value).expect("box fits")
    }

    fn check(&self) -> Result<(), VoleError> {
        let total: u64 = self.segments.iter().map(TrajectorySegment::steps).sum();
        if total != self.intervals {
            return Err(VoleError::ApiConstraint(
                "trajectory program steps must equal the court intervals",
            ));
        }
        crate::trajectory::check_program(&self.segments, &crate::limits::Limits::default())?;
        Ok(())
    }

    /// Exact per-frame positions of the moving instance (frame 0 is the start
    /// placement). Closed-form evaluation, independent of the state stepper.
    pub fn positions(&self) -> Result<Vec<(i64, i64)>, VoleError> {
        self.check()?;
        trajectory::simulate_positions(&self.segments, self.x0, self.y0, self.intervals).ok_or(
            VoleError::ApiConstraint("trajectory simulation overflowed or fell short"),
        )
    }

    /// Canonical `.vole` bytes: one `SetTrajectory` at interval 1, then one
    /// `AdvanceTrajectories` per interval — never a stored frame, never a
    /// per-frame coordinate payload.
    pub fn vole(&self) -> Result<Vec<u8>, VoleError> {
        self.check()?;
        let obj = self.object();
        let inst = Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: self.x0,
            y: self.y0,
        };
        let mut timeline = Vec::with_capacity(self.intervals as usize);
        for k in 1..=self.intervals {
            let group = if k == 1 {
                vec![
                    Transition::SetTrajectory {
                        id: InstanceId(self.instance_id),
                        segments: self.segments.clone(),
                    },
                    Transition::AdvanceTrajectories,
                ]
            } else {
                vec![Transition::AdvanceTrajectories]
            };
            timeline.push((k, group));
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

    /// Independent reference `.raw` frames: each frame paints the box at the
    /// closed-form position table.
    pub fn reference_raw(&self) -> Result<Vec<u8>, VoleError> {
        let raster = self.object().expand();
        let mut raw = Vec::new();
        for (x, y) in self.positions()? {
            raw.extend(reference_painter(
                self.width,
                self.height,
                0,
                &[(x, y, &raster, self.box_w, self.box_h)],
            ));
        }
        Ok(raw)
    }

    /// Materialize and byte-exact verify against the independent reference.
    pub fn materialize_and_verify(&self) -> Result<Vec<Canvas>, VoleError> {
        let parsed = decoder::decode_bytes(&self.vole()?)?;
        let frames = decoder::materialize_all(&parsed)?;
        let mut got = Vec::new();
        for c in &frames {
            got.extend_from_slice(c.as_slice());
        }
        if got != self.reference_raw()? {
            return Err(VoleError::ApiConstraint(
                "trajectory diverged from reference",
            ));
        }
        Ok(frames)
    }

    /// The equivalent per-frame absolute `SetPosition` stream (baseline for
    /// the byte comparison; same frames, no parametric state).
    pub fn set_position_baseline_bytes(&self) -> Result<Vec<u8>, VoleError> {
        let obj = self.object();
        let inst = Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: self.x0,
            y: self.y0,
        };
        let positions = self.positions()?;
        let mut timeline = Vec::with_capacity(self.intervals as usize);
        for k in 1..=self.intervals {
            let (x, y) = positions[k as usize];
            timeline.push((
                k,
                vec![Transition::SetPosition {
                    id: InstanceId(self.instance_id),
                    x,
                    y,
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

    /// The equivalent per-frame `SetVelocity + AdvanceTranslations` stream
    /// (the Phase-E baseline for motion whose velocity is *not* constant: the
    /// velocity must be rewritten every interval).
    pub fn velocity_baseline_bytes(&self) -> Result<Vec<u8>, VoleError> {
        let obj = self.object();
        let inst = Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: self.x0,
            y: self.y0,
        };
        let positions = self.positions()?;
        let mut timeline = Vec::with_capacity(self.intervals as usize);
        for k in 1..=self.intervals {
            let p_prev = positions[k as usize - 1];
            let p = positions[k as usize];
            let vx = p.0 - p_prev.0;
            let vy = p.1 - p_prev.1;
            timeline.push((
                k,
                vec![
                    Transition::SetVelocity {
                        id: InstanceId(self.instance_id),
                        vx,
                        vy,
                    },
                    Transition::AdvanceTranslations,
                ],
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

    /// Number of materializable frames.
    pub fn frame_count(&self) -> u64 {
        1 + self.intervals
    }

    /// Total raw raster bytes of the canonical frame sequence.
    pub fn raw_bytes_all(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * self.frame_count()
    }
}

// ---------------------------------------------------------------------------
// Phase J — palette state courts
// ---------------------------------------------------------------------------

/// How a [`PaletteCourt`] mutates its palette over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    /// The accent entry (and only that entry) takes the next cycle value each
    /// interval — one `PatchPalette` per frame. Frame `k` shows `cycle[k %
    /// cycle.len()]` (frame 0 shows `cycle[0]`, laid down by the checkpoint
    /// palette).
    AccentCycle,
    /// The whole palette's values rotate by one position each interval (the
    /// classic color-drift animation): frame `k` shows
    /// `entries[i] = base[(i + k) % base.len()]` — one `SetPalette` per frame.
    RotateAll,
}

/// Deterministic window-UI index plane (Phase J content): a palette-index
/// box with a title bar, a sidebar, a body, horizontal separators, an accent
/// status bar, and a small cursor cell. Row-major `w*h` indices:
/// `0` body · `1` title bar · `2` sidebar · `3` separators · `4` accent bar ·
/// `5` cursor.
pub fn window_ui_indices(
    w: u32,
    h: u32,
    title_h: u32,
    side_w: u32,
    sep_every: u32,
    status_h: u32,
) -> Vec<u8> {
    assert!(w >= side_w && h >= title_h + status_h + 2);
    let mut out = Vec::with_capacity((w * h) as usize);
    let (cw, ch) = (w as i64, h as i64);
    // A deterministic cursor cell in the lower body.
    let (cur_x, cur_y) = (cw / 2, ch - status_h as i64 - 8);
    for y in 0..ch {
        for x in 0..cw {
            let idx = if y < title_h as i64 {
                1
            } else if x < side_w as i64 {
                2
            } else if y >= ch - status_h as i64 {
                4
            } else if x >= cur_x && x < cur_x + 2 && y >= cur_y && y < cur_y + 2 {
                5
            } else if sep_every > 0 && (y as u32 - title_h).is_multiple_of(sep_every) {
                3
            } else {
                0
            };
            out.push(idx);
        }
    }
    out
}

/// Base entries for the window-UI plane (indices 0..=5):
/// `[body, title, sidebar, separator, accent(init), cursor]`.
pub fn window_ui_entries() -> Vec<u8> {
    vec![30, 255, 200, 128, 200, 0]
}

/// Rotate `entries` left by `shift` (used by the `RotateAll` mode on both the
/// encode and the reference side).
pub fn rotate_palette(entries: &[u8], shift: u64) -> Vec<u8> {
    let n = entries.len() as u64;
    (0..entries.len())
        .map(|i| entries[((i as u64 + shift) % n) as usize])
        .collect()
}

/// Phase-J direct-procedural court: one palette-index object (a whole-box
/// index plane) bound to one palette; the palette mutates every interval
/// while the *index plane never changes*. Frames are materialized views of
/// `indices ∘ entries(t)` — never stored rasters and never per-frame index
/// rewrites.
///
/// The reference raster is an **independent** painter that maps the index
/// plane through the mode's analytic per-frame entry table, so a shared
/// mapping bug cannot mask a mismatch.
pub struct PaletteCourt {
    /// Canvas width / height.
    pub width: u32,
    pub height: u32,
    /// Canvas background sample.
    pub background: u8,
    /// Index-plane box geometry and placement (painted clipped at this
    /// top-left).
    pub box_x: i64,
    pub box_y: i64,
    pub box_w: u32,
    pub box_h: u32,
    /// Object / instance / palette ids.
    pub object_id: u32,
    pub instance_id: u32,
    pub palette_id: u32,
    /// Row-major index plane (box width × box height).
    pub indices: Vec<u8>,
    /// Palette entries at frame 0.
    pub base_entries: Vec<u8>,
    /// Mutation mode.
    pub mode: PaletteMode,
    /// Accent index and cycle values (`AccentCycle` mode).
    pub accent_index: u8,
    pub cycle: Vec<u8>,
    /// Number of intervals (frames == intervals + 1).
    pub intervals: u64,
}

impl PaletteCourt {
    fn entries_at(&self, frame: u64) -> Vec<u8> {
        match self.mode {
            PaletteMode::AccentCycle => {
                let mut e = self.base_entries.clone();
                let v = self.cycle[(frame as usize) % self.cycle.len()];
                e[self.accent_index as usize] = v;
                e
            }
            PaletteMode::RotateAll => rotate_palette(&self.base_entries, frame),
        }
    }

    fn check(&self) -> Result<(), VoleError> {
        if self.base_entries.is_empty()
            || self.base_entries.len() > 256
            || self.cycle.is_empty()
            || self.indices.len() as u64 != u64::from(self.box_w) * u64::from(self.box_h)
        {
            return Err(VoleError::ApiConstraint("bad palette court parameters"));
        }
        if self.mode == PaletteMode::AccentCycle
            && usize::from(self.accent_index) >= self.base_entries.len()
        {
            return Err(VoleError::ApiConstraint("accent index outside the palette"));
        }
        // Every index in the plane must be mappable at every frame.
        let max_idx = *self.indices.iter().max().unwrap_or(&0);
        if usize::from(max_idx) >= self.base_entries.len() {
            return Err(VoleError::ApiConstraint(
                "index plane exceeds the palette length",
            ));
        }
        Ok(())
    }

    /// Canonical `.vole` bytes: one palette declaration + one index object +
    /// one bound instance at the checkpoint, then one tiny palette mutation
    /// per interval — never stored rasters, never per-frame index rewrites.
    pub fn vole(&self) -> Result<Vec<u8>, VoleError> {
        self.check()?;
        let obj = Object::index_raster(self.box_w, self.box_h, self.indices.clone())?;
        let inst = Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: self.box_x,
            y: self.box_y,
        };
        let mut timeline = Vec::with_capacity(self.intervals as usize);
        for k in 1..=self.intervals {
            let tr = match self.mode {
                PaletteMode::AccentCycle => Transition::PatchPalette {
                    id: PaletteId(self.palette_id),
                    changes: vec![(
                        self.accent_index,
                        self.cycle[(k as usize) % self.cycle.len()],
                    )],
                },
                PaletteMode::RotateAll => Transition::SetPalette {
                    id: PaletteId(self.palette_id),
                    entries: rotate_palette(&self.base_entries, k),
                },
            };
            timeline.push((k, vec![tr]));
        }
        encoder::encode_palette_stream(
            self.width,
            self.height,
            self.background,
            &[(self.object_id, obj)],
            &[(self.palette_id, self.base_entries.clone())],
            &[(inst, Some(PaletteId(self.palette_id)))],
            &timeline,
        )
    }

    /// Independent reference `.raw` frames: a separate painter loop maps the
    /// index plane through the mode's analytic per-frame entries.
    pub fn reference_raw(&self) -> Result<Vec<u8>, VoleError> {
        let mut raw = Vec::new();
        for f in 0..=self.intervals {
            let entries = self.entries_at(f);
            raw.extend(palette_reference_painter(
                self.width,
                self.height,
                self.background,
                self.box_x,
                self.box_y,
                &self.indices,
                self.box_w,
                self.box_h,
                &entries,
            ));
        }
        Ok(raw)
    }

    /// Materialize and byte-exact verify against the independent reference.
    pub fn materialize_and_verify(&self) -> Result<Vec<Canvas>, VoleError> {
        let parsed = decoder::decode_bytes(&self.vole()?)?;
        let frames = decoder::materialize_all(&parsed)?;
        let mut got = Vec::new();
        for c in &frames {
            got.extend_from_slice(c.as_slice());
        }
        if got != self.reference_raw()? {
            return Err(VoleError::ApiConstraint(
                "palette court diverged from reference",
            ));
        }
        Ok(frames)
    }

    /// Number of materializable frames.
    pub fn frame_count(&self) -> u64 {
        1 + self.intervals
    }

    /// Total raw raster bytes of the canonical frame sequence.
    pub fn raw_bytes_all(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * self.frame_count()
    }
}

/// Independent palette painter: fill the canvas with `bg`, then overwrite the
/// index box (clipped) mapping every index through `entries`. Structurally
/// distinct from the materializer's `Canvas` path.
#[allow(clippy::too_many_arguments)] // 9 ordered painter params (geometry + entries)
fn palette_reference_painter(
    width: u32,
    height: u32,
    bg: u8,
    bx: i64,
    by: i64,
    indices: &[u8],
    bw: u32,
    bh: u32,
    entries: &[u8],
) -> Vec<u8> {
    let w = width as usize;
    let mut out = vec![bg; w * height as usize];
    for sy in 0..bh as i64 {
        let cy = by + sy;
        if cy < 0 || cy >= i64::from(height) {
            continue;
        }
        let src_row = sy as usize * bw as usize;
        let dst_row = cy as usize * w;
        for sx in 0..bw as i64 {
            let cx = bx + sx;
            if cx < 0 || cx >= i64::from(width) {
                continue;
            }
            let idx = indices[src_row + sx as usize];
            out[dst_row + cx as usize] = entries[idx as usize];
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Phase L — bounded fixed-point affine placement courts
// ---------------------------------------------------------------------------

/// Affine parameters rotating content of local center `(u0, v0)` about the
/// destination center `(cx, cy)` by `k` quarter turns (multiples of 90°, k
/// mod 4). Quarter turns are exact in Q8 (integer coefficients).
pub fn quarter_turn_params(k: i64, u0: i64, v0: i64, cx: i64, cy: i64) -> AffineParams {
    let k = k.rem_euclid(4);
    let (m00, m01, m10, m11) = match k {
        0 => (1i64, 0i64, 0i64, 1i64),
        1 => (0, 1, -1, 0),
        2 => (-1, 0, 0, -1),
        _ => (0, -1, 1, 0),
    };
    AffineParams {
        a: AFFINE_SCALE * m00,
        b: AFFINE_SCALE * m01,
        c: AFFINE_SCALE * (u0 - m00 * cx - m01 * cy),
        d: AFFINE_SCALE * m10,
        e: AFFINE_SCALE * m11,
        f: AFFINE_SCALE * (v0 - m10 * cx - m11 * cy),
    }
}

/// Affine parameters for an integer `2×` zoom about `(cx, cy)` of content
/// with local center `(u0, v0)`. Exact in Q8 (coefficient 128 = 1/2 px per
/// dest px).
pub fn zoom2_params(u0: i64, v0: i64, cx: i64, cy: i64) -> AffineParams {
    let a = AFFINE_SCALE / 2;
    let e = AFFINE_SCALE / 2;
    AffineParams {
        a,
        b: 0,
        c: AFFINE_SCALE * u0 - a * cx,
        d: 0,
        e,
        f: AFFINE_SCALE * v0 - e * cy,
    }
}

/// Affine parameters for a plain placement at content top-left `(px, py)`
/// panned by `pan_num`/`pan_den` destination pixels (Q8 translation).
pub fn pan_params(px: i64, py: i64, pan_num: i64, pan_den: i64) -> AffineParams {
    let p = AFFINE_SCALE * pan_num / pan_den;
    AffineParams {
        a: AFFINE_SCALE,
        b: 0,
        c: -AFFINE_SCALE * px + p,
        d: 0,
        e: AFFINE_SCALE,
        f: -AFFINE_SCALE * py,
    }
}

/// Phase-L direct-procedural court: one tile object whose placement is
/// affine-mapped per interval (rotation / zoom / sub-pixel pan). Frame 0 is
/// the plain placement; every later frame applies one `SetAffine`. The
/// reference raster is an **independent** painter with a structurally
/// different loop (per-row incremental accumulation vs per-pixel products),
/// so a shared sampling bug cannot mask a mismatch.
pub struct AffineCourt {
    /// Canvas width / height.
    pub width: u32,
    pub height: u32,
    /// Canvas background sample.
    pub background: u8,
    /// Tile object geometry and content (row-major, non-uniform).
    pub tile_w: u32,
    pub tile_h: u32,
    pub content: Vec<u8>,
    /// Plain placement of the tile at frame 0.
    pub plain_x: i64,
    pub plain_y: i64,
    /// Object / instance ids.
    pub object_id: u32,
    pub instance_id: u32,
    /// Affine placement applied at frames 1..=intervals (length ==
    /// intervals). The identity affine is allowed (returns to plain mode).
    pub params: Vec<AffineParams>,
    /// Number of intervals (frames == intervals + 1).
    pub intervals: u64,
}

impl AffineCourt {
    /// Validate the court parameters (geometry, count, coefficients).
    pub fn check(&self) -> Result<(), VoleError> {
        if self.content.len() as u64 != u64::from(self.tile_w) * u64::from(self.tile_h) {
            return Err(VoleError::ApiConstraint("tile content/geometry mismatch"));
        }
        if self.params.len() as u64 != self.intervals {
            return Err(VoleError::ApiConstraint(
                "one affine parameter set per interval",
            ));
        }
        for p in &self.params {
            p.check()?;
        }
        Ok(())
    }

    /// Canonical `.vole` bytes: one tile object, one instance at the plain
    /// placement, then one `SetAffine` per interval.
    pub fn vole(&self) -> Result<Vec<u8>, VoleError> {
        self.check()?;
        let obj = Object::raster(self.tile_w, self.tile_h, self.content.clone())?;
        let inst = Instance {
            id: InstanceId(self.instance_id),
            object_id: ObjectId(self.object_id),
            x: self.plain_x,
            y: self.plain_y,
        };
        let mut timeline = Vec::with_capacity(self.intervals as usize);
        for (k, p) in self.params.iter().enumerate() {
            timeline.push((
                (k + 1) as u64,
                vec![Transition::SetAffine {
                    id: InstanceId(self.instance_id),
                    params: *p,
                }],
            ));
        }
        encoder::encode_stream(
            self.width,
            self.height,
            self.background,
            &[(self.object_id, obj)],
            &[inst],
            &timeline,
        )
    }

    /// Independent reference `.raw` frames.
    pub fn reference_raw(&self) -> Result<Vec<u8>, VoleError> {
        self.check()?;
        let mut raw = Vec::new();
        // Frame 0: plain placement.
        raw.extend(affine_reference_painter(
            self.width,
            self.height,
            self.background,
            &self.content,
            self.tile_w,
            self.tile_h,
            self.plain_x,
            self.plain_y,
            None,
        ));
        for p in &self.params {
            raw.extend(affine_reference_painter(
                self.width,
                self.height,
                self.background,
                &self.content,
                self.tile_w,
                self.tile_h,
                self.plain_x,
                self.plain_y,
                Some(*p),
            ));
        }
        Ok(raw)
    }

    /// Materialize and byte-exact verify against the independent reference.
    pub fn materialize_and_verify(&self) -> Result<Vec<Canvas>, VoleError> {
        let parsed = decoder::decode_bytes(&self.vole()?)?;
        let frames = decoder::materialize_all(&parsed)?;
        let mut got = Vec::new();
        for c in &frames {
            got.extend_from_slice(c.as_slice());
        }
        if got != self.reference_raw()? {
            return Err(VoleError::ApiConstraint(
                "affine court diverged from reference",
            ));
        }
        Ok(frames)
    }

    /// Number of materializable frames.
    pub fn frame_count(&self) -> u64 {
        1 + self.intervals
    }

    /// Total raw raster bytes of the canonical frame sequence.
    pub fn raw_bytes_all(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * self.frame_count()
    }
}

/// Independent affine painter: fill `bg`, then either blit the tile at its
/// plain placement or, when `mapped` is given, sample every destination pixel
/// through the canonical Q8 source map with per-row incremental accumulation.
#[allow(clippy::too_many_arguments)] // 9 ordered painter params (geometry + content + placement)
fn affine_reference_painter(
    width: u32,
    height: u32,
    bg: u8,
    content: &[u8],
    tile_w: u32,
    tile_h: u32,
    plain_x: i64,
    plain_y: i64,
    mapped: Option<AffineParams>,
) -> Vec<u8> {
    let cw = width as usize;
    let mut out = vec![bg; cw * height as usize];
    match mapped {
        None => {
            for sy in 0..tile_h as i64 {
                let cy = plain_y + sy;
                if cy < 0 || cy >= i64::from(height) {
                    continue;
                }
                let src_row = sy as usize * tile_w as usize;
                let dst_row = cy as usize * cw;
                for sx in 0..tile_w as i64 {
                    let cx = plain_x + sx;
                    if cx < 0 || cx >= i64::from(width) {
                        continue;
                    }
                    out[dst_row + cx as usize] = content[src_row + sx as usize];
                }
            }
        }
        Some(p) => {
            let tw = i64::from(tile_w);
            let th = i64::from(tile_h);
            for dy in 0..height as i64 {
                // Per-row base then per-pixel increments (different arithmetic
                // shape from the materializer's direct products, same value).
                let mut nux = p.b * dy + p.c;
                let mut nvx = p.e * dy + p.f;
                for dx in 0..width as i64 {
                    let su = nux >> crate::affine::AFFINE_SHIFT;
                    let sv = nvx >> crate::affine::AFFINE_SHIFT;
                    nux += p.a;
                    nvx += p.d;
                    if su < 0 || sv < 0 || su >= tw || sv >= th {
                        continue;
                    }
                    out[dy as usize * cw + dx as usize] = content[(sv * tw + su) as usize];
                }
            }
        }
    }
    out
}
