//! Phase-J evidence producer: palette state.
//!
//! `cargo run --release --example palette_proof` prints a deterministic
//! report for the evidence campaign:
//!
//! * the accent-cycling flagship (1920×1080 window-UI canvas, one index
//!   plane, one palette, one tiny `PatchPalette` per interval) — stream bytes
//!   vs RAW and vs the sparse/unchanged floors that a palette-less encoder
//!   would have to pay, all byte-exact against an independent palette
//!   painter;
//! * whole-palette rotation (`SetPalette` per interval) on the same content;
//! * the measured **flattening tax** (§55): the same visual frames encoded
//!   through the raster-origin inverse encoder at court scale;
//! * the static-palette floor (13 B/frame unchanged lane once palette content
//!   is established — an active-but-static palette is free at rest).

use std::time::Instant;

use vole_video::{decoder, demo, error::VoleError, inverse, transition::Transition};

/// Count palette ops present in a stream's intervals.
fn palette_op_counts(bytes: &[u8]) -> (u64, u64) {
    let parsed = decoder::decode_bytes(bytes).expect("stream parses");
    let (mut set, mut patch) = (0u64, 0u64);
    for (_t, trs) in parsed.intervals() {
        for tr in trs {
            match tr {
                Transition::SetPalette { .. } => set += 1,
                Transition::PatchPalette { .. } => patch += 1,
                _ => {}
            }
        }
    }
    (set, patch)
}

fn ui_court(w: u32, h: u32, mode: demo::PaletteMode, intervals: u64) -> demo::PaletteCourt {
    demo::PaletteCourt {
        width: w,
        height: h,
        background: 90,
        box_x: 0,
        box_y: 0,
        box_w: w,
        box_h: h,
        object_id: 1,
        instance_id: 1,
        palette_id: 1,
        indices: demo::window_ui_indices(w, h, 6, 24, 16, 12),
        base_entries: demo::window_ui_entries(),
        mode,
        accent_index: 4,
        cycle: vec![200, 60],
        intervals,
    }
}

fn main() -> Result<(), VoleError> {
    /// Interval-only bytes of a stream: everything outside the one-time
    /// declarations (header, objects, checkpoint, palette state, trailer).
    fn interval_bytes(bytes: &[u8]) -> Result<u64, VoleError> {
        let c = inverse::account_stream(bytes)?;
        Ok(c.total_bytes
            - c.header_bytes
            - c.object_bytes
            - c.checkpoint_bytes
            - c.state_bytes
            - c.integrity_bytes)
    }

    // --- 1. Accent cycling flagship, 1920x1080 ------------------------------
    {
        let court = ui_court(1920, 1080, demo::PaletteMode::AccentCycle, 100);
        let t = Instant::now();
        let vole = court.vole()?;
        let frames = court.materialize_and_verify()?; // byte-exact vs painter
        let (set, patch) = palette_op_counts(&vole);
        let intervals = interval_bytes(&vole)?;
        let accent_pts = (1920u64 - 24) * 12; // status bar (x >= side_w)
        let sparse_floor = 5 + 9 * accent_pts; // per-interval sparse equivalent
        println!(
            "accent-flag-1920x1080: frames={} vole={}B raw_all={}B \
             interval_bytes={}B per_interval={}B sparse_floor_per_interval={}B \
             raw_per_frame={}B exact=true palette_ops=(set {set}, patch {patch}) verify_ms={:.1}",
            court.frame_count(),
            vole.len(),
            court.raw_bytes_all(),
            intervals,
            intervals / 100,
            sparse_floor,
            1920u64 * 1080,
            t.elapsed().as_secs_f64() * 1000.0
        );
        assert_eq!(frames.len(), 101);
        // Margins: palette interval (24 B) vs the palette-less floors.
        let per_interval = intervals / 100;
        assert!(per_interval * 200 < sparse_floor);
        assert!(
            per_interval * 50_000 < 1920u64 * 1080,
            "not raster-proportional"
        );
    }

    // --- 2. Whole-palette rotation flagship, 1920x1080 ----------------------
    {
        let court = ui_court(1920, 1080, demo::PaletteMode::RotateAll, 100);
        let vole = court.vole()?;
        let frames = court.materialize_and_verify()?;
        let (set, patch) = palette_op_counts(&vole);
        let intervals = interval_bytes(&vole)?;
        // Rotation re-maps every pixel each interval: no unchanged lane, no
        // sparse overlay can help a palette-less encoder short of RAW.
        println!(
            "rotate-flag-1920x1080: frames={} vole={}B raw_all={}B \
             interval_bytes={}B per_interval={}B raw_per_frame={}B exact=true \
             palette_ops=(set {set}, patch {patch})",
            court.frame_count(),
            vole.len(),
            court.raw_bytes_all(),
            intervals,
            intervals / 100,
            1920u64 * 1080
        );
        assert_eq!(frames.len(), 101);
        assert_ne!(frames[0], frames[1], "rotation changes every pixel");
    }

    // --- 3. Measured flattening tax (§55 court) -----------------------------
    {
        // The same visual frames rasterized and inverse-proceduralized
        // (Phase-G whole-frame encoder, which has no palette family yet).
        let court = ui_court(240, 160, demo::PaletteMode::AccentCycle, 12);
        let vole = court.vole()?;
        let frames = court.materialize_and_verify()?;
        let t = Instant::now();
        let flattened = inverse::encode_frames(
            &frames,
            &inverse::EncodeOptions {
                bg_sweep: false,
                background: Some(court.background),
                ..inverse::EncodeOptions::default()
            },
        )?;
        assert!(flattened.exact);
        let t_ms = t.elapsed().as_secs_f64() * 1000.0;
        let interval_palette = interval_bytes(&vole)?;
        let interval_flat = interval_bytes(&flattened.vole)?;
        println!(
            "flattening-tax-240x160: frames={} palette={}B flattened={}B total_ratio={:.2}x \
             interval_palette={}B interval_flattened={}B interval_ratio={:.0}x exact=true \
             encode_ms={:.0}",
            court.frame_count(),
            vole.len(),
            flattened.vole.len(),
            flattened.vole.len() as f64 / vole.len() as f64,
            interval_palette,
            interval_flat,
            interval_flat as f64 / interval_palette as f64,
            t_ms
        );
    }

    // --- 4. Static-palette floor (unchanged lane) ---------------------------
    {
        // Once palette content is established, a *static* palette costs the
        // ordinary unchanged lane (13 B/frame): zero-transition intervals.
        let mut wr = vole_video::format::StreamWriter::begin(640, 360);
        wr = wr.declare_object(
            vole_video::object::ObjectId(1),
            vole_video::object::Object::index_raster(
                640,
                360,
                demo::window_ui_indices(640, 360, 6, 24, 16, 12),
            )?,
        )?;
        wr = wr.palette(vole_video::state::PaletteId(1), demo::window_ui_entries())?;
        let inst = vole_video::state::Instance {
            id: vole_video::state::InstanceId(1),
            object_id: vole_video::object::ObjectId(1),
            x: 0,
            y: 0,
        };
        wr = wr.checkpoint_with_bindings(&[(inst, Some(vole_video::state::PaletteId(1)))])?;
        for k in 1..=200u64 {
            wr = wr.interval(vole_video::time::Interval(k), &[])?;
        }
        let bytes = wr.finish()?;
        let parsed = decoder::decode_bytes(&bytes)?;
        let frames = vole_video::decoder::materialize_all(&parsed)?;
        assert_eq!(frames.len(), 201);
        assert!(frames.iter().all(|f| f.exactly_matches(&frames[0])));
        let marginal = interval_bytes(&bytes)? / 200; // unchanged lane
        println!(
            "static-palette-640x360: frames={} vole={}B marginal_per_interval={}B \
             (unchanged lane 13.0B) exact=true note=\"palette content at rest is free\"",
            frames.len(),
            bytes.len(),
            marginal
        );
        assert_eq!(marginal, 13);
    }
    Ok(())
}
