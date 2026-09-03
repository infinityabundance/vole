//! Phase-L evidence producer: bounded fixed-point affine placement.
//!
//! `cargo run --release --example affine_proof` prints a deterministic
//! report: the rotating-tile flagship (affine state vs raw and vs
//! re-encoding the same visual frames through the raster encoder), integer
//! zoom / sub-pixel pan, and the residual closure of a Q8 30° rotation
//! approximation against a float-rendered target. Every stream is
//! byte-verified against an independent affine painter before it is counted.

use std::time::Instant;

use vole_video::{
    affine::AffineParams, decoder, demo, encoder, error::VoleError, inverse, transition::Transition,
};

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

/// Non-symmetric tile content with a distinctive mark at (7,3).
fn tile_content(w: u32, h: u32) -> Vec<u8> {
    let mut d = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = if x == 7 && y == 3 {
                250
            } else {
                ((x / 3 + y / 5) % 9) as u8 * 23 + 10
            };
            d.push(v);
        }
    }
    d
}

fn count_affine_ops(bytes: &[u8]) -> u64 {
    let parsed = decoder::decode_bytes(bytes).expect("stream parses");
    parsed
        .intervals()
        .iter()
        .flat_map(|(_, trs)| trs.iter())
        .filter(|t| matches!(t, Transition::SetAffine { .. }))
        .count() as u64
}

fn main() -> Result<(), VoleError> {
    // --- 1. Rotating-tile flagship ------------------------------------------
    {
        let (w, h) = (320u32, 180u32);
        let params: Vec<AffineParams> = (1..=80)
            .map(|k| demo::quarter_turn_params(k, 32, 32, 160, 90))
            .collect();
        let court = demo::AffineCourt {
            width: w,
            height: h,
            background: 90,
            tile_w: 64,
            tile_h: 64,
            content: tile_content(64, 64),
            plain_x: 128,
            plain_y: 58,
            object_id: 1,
            instance_id: 1,
            params,
            intervals: 80,
        };
        let t = Instant::now();
        let vole = court.vole()?;
        let frames = court.materialize_and_verify()?;
        assert_eq!(frames.len(), 81);
        let ops = count_affine_ops(&vole);
        println!(
            "rotate-flag-320x180: frames={} vole={}B raw_all={}B interval_bytes={}B \
             set_affine_ops={ops} exact=true verify_ms={:.1}",
            court.frame_count(),
            vole.len(),
            court.raw_bytes_all(),
            interval_bytes(&vole)?,
            t.elapsed().as_secs_f64() * 1000.0
        );
        assert!((vole.len() as u64) * 20 < court.raw_bytes_all());
    }

    // --- 2. Re-encoding the same rotation through the raster encoder --------
    {
        let (w, h) = (160u32, 160u32);
        let params: Vec<AffineParams> = (1..=40)
            .map(|k| demo::quarter_turn_params(k, 32, 32, 80, 80))
            .collect();
        let court = demo::AffineCourt {
            width: w,
            height: h,
            background: 90,
            tile_w: 64,
            tile_h: 64,
            content: tile_content(64, 64),
            plain_x: 48,
            plain_y: 48,
            object_id: 1,
            instance_id: 1,
            params,
            intervals: 40,
        };
        let vole = court.vole()?;
        let frames = court.materialize_and_verify()?;
        let t = Instant::now();
        let flattened = vole_video::inverse::encode_frames(
            &frames,
            &vole_video::inverse::EncodeOptions {
                bg_sweep: false,
                background: Some(court.background),
                ..vole_video::inverse::EncodeOptions::default()
            },
        )?;
        assert!(flattened.exact);
        println!(
            "rotation-flattening-160x160: frames={} affine={}B raster_encoded={}B \
             ratio={:.0}x exact=true encode_ms={:.0}",
            court.frame_count(),
            vole.len(),
            flattened.vole.len(),
            flattened.vole.len() as f64 / vole.len() as f64,
            t.elapsed().as_secs_f64() * 1000.0
        );
    }

    // --- 3. Zoom and sub-pixel pan ------------------------------------------
    {
        let params = vec![
            demo::zoom2_params(32, 32, 160, 90),
            demo::pan_params(128, 58, 1, 2),
            demo::pan_params(128, 58, 3, 2),
        ];
        let court = demo::AffineCourt {
            width: 320,
            height: 180,
            background: 90,
            tile_w: 64,
            tile_h: 64,
            content: tile_content(64, 64),
            plain_x: 128,
            plain_y: 58,
            object_id: 1,
            instance_id: 1,
            params,
            intervals: 3,
        };
        let vole = court.vole()?;
        let frames = court.materialize_and_verify()?;
        println!(
            "zoom-pan-320x180: frames={} vole={}B exact=true (2x zoom, 0.5px and 1.5px pans)",
            court.frame_count(),
            vole.len()
        );
        assert_eq!(frames.len(), 4);
    }

    // --- 4. Residual closure of a Q8 approximation --------------------------
    {
        let (w, h) = (160u32, 160u32);
        let content = tile_content(64, 64);
        let bg = 90u8;
        // Float-rendered 30-degree rotation (floor sampling, court-side).
        let (cos, sin) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        let mut float_frame = vec![bg; (w * h) as usize];
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let uf = 32.0 + cos * ((x - 80) as f64) + sin * ((y - 80) as f64);
                let vf = 32.0 - sin * ((x - 80) as f64) + cos * ((y - 80) as f64);
                let u = uf.floor() as i64;
                let v = vf.floor() as i64;
                if u < 0 || v < 0 || u >= 64 || v >= 64 {
                    continue;
                }
                float_frame[y as usize * w as usize + x as usize] = content[(v * 64 + u) as usize];
            }
        }
        let approx = AffineParams {
            a: (256.0 * cos).round() as i64,
            b: (256.0 * sin).round() as i64,
            c: 256 * 32 - (256.0 * cos).round() as i64 * 80 - (256.0 * sin).round() as i64 * 80,
            d: -(256.0 * sin).round() as i64,
            e: (256.0 * cos).round() as i64,
            f: 256 * 32 + (256.0 * sin).round() as i64 * 80 - (256.0 * cos).round() as i64 * 80,
        };
        let obj = vole_video::object::Object::raster(64, 64, content.clone())?;
        let inst = vole_video::state::Instance {
            id: vole_video::state::InstanceId(1),
            object_id: vole_video::object::ObjectId(1),
            x: 48,
            y: 48,
        };
        let approx_only = encoder::encode_stream(
            w,
            h,
            bg,
            &[(1, obj.clone())],
            std::slice::from_ref(&inst),
            &[(
                1,
                vec![Transition::SetAffine {
                    id: vole_video::state::InstanceId(1),
                    params: approx,
                }],
            )],
        )?;
        let base = decoder::materialize_all(&decoder::decode_bytes(&approx_only)?)?;
        let b = &base[1];
        let mut pts = Vec::new();
        for x in 0..w as i64 {
            for y in 0..h as i64 {
                let bv = b.get(x as u32, y as u32);
                let tv = float_frame[y as usize * w as usize + x as usize];
                if bv != tv {
                    pts.push((x, y, tv));
                }
            }
        }
        let mut groups = vec![(
            1u64,
            vec![
                Transition::SetAffine {
                    id: vole_video::state::InstanceId(1),
                    params: approx,
                },
                Transition::PatchSparse {
                    points: pts.clone(),
                },
            ],
        )];
        for k in 2..=40u64 {
            groups.push((k, Vec::new()));
        }
        let bytes =
            encoder::encode_stream(w, h, bg, &[(1, obj)], std::slice::from_ref(&inst), &groups)?;
        let frames = decoder::materialize_all(&decoder::decode_bytes(&bytes)?)?;
        for f in &frames[1..] {
            assert_eq!(f.as_slice(), &float_frame[..]);
        }
        println!(
            "residual-closure-30deg-160x160: frames={} vole={}B residual_points={} \
             residual_share_of_tile={:.3} tile_pixels={} exact=true \
             note=\"Q8 approx + sparse correction reproduces the float render exactly\"",
            frames.len(),
            bytes.len(),
            pts.len(),
            pts.len() as f64 / 4096.0,
            64 * 64
        );
    }
    Ok(())
}
