//! Phase Q — the native procedural test format (master brief §53).
//!
//! A small, deterministic, **research-harness-only** textual input format for
//! authored procedural content. It is NOT a normative VOLE syntax and is never
//! part of the `.vole` wire format; it exists so the §55 native-preservation
//! court can author the *same* procedural state once and compare direct ingest
//! against rasterize-then-inverse-proceduralize.
//!
//! # Grammar
//!
//! ```text
//! # comments run to end of line
//! canvas W H                       canvas geometry (required, first)
//! background V                     checkpoint background (default 0)
//! object ID fill W H V             uniform fill object
//! object ID gradient W H B SX SY   generator: v = (B + SX·x + SY·y) mod 256
//! object ID checker W H A B CELL   generator checkerboard
//! object ID periodic W H B SX SY P generator sawtooth
//! object ID noise W H SEED         generator seeded noise (authored only)
//! object ID raster W H V*          literal raster, exactly W·H byte values
//! object ID index W H V*           palette-index plane, exactly W·H values
//! palette ID V*                    pre-checkpoint palette table (id ≥ 1)
//! instance IID OID X Y             checkpoint instance (paint order)
//! instance IID OID X Y palette PID checkpoint instance with palette binding
//! at T                             open the interval group at absolute frame T
//!   move IID X Y                     SetPosition
//!   velocity IID VX VY               SetVelocity
//!   advance                          AdvanceTranslations
//!   trajectory IID seg*              SetTrajectory (segments in sequence):
//!     lin VX VY STEPS                  constant-velocity segment
//!     accel VX0 VY0 AX AY STEPS        constant-acceleration segment
//!   advance_traj                     AdvanceTrajectories
//!   affine IID A B C D E F           SetAffine (Q8 placement)
//!   set_palette PID V*               replace the palette's entries
//!   patch_palette PID IDX=V ...      patch entries (strictly ascending IDX)
//!   bind IID PID                     BindPalette (PID 0 unbinds)
//!   copy SX SY W H DX DY             COPY_RECT from the previous frame
//!   move_rect SX SY W H DX DY        MOVE_RECT
//!   sparse X Y V ...                 overlay points (strictly ascending)
//!   clear_instances                  ClearInstances
//!   clear_overlay                    ClearOverlay
//! ```
//!
//! Values are whitespace separated and may span lines; numeric value runs end
//! at the next keyword. Geometry and sample-count validation happen in the
//! [`crate::ingest::Ingest`] layer and at finish through the normative
//! encoder. All integers are checked (`u8` samples in `0..=255`); malformed
//! input is a typed [`VoleError::ScriptParse`].

use crate::{
    affine::AffineParams, error::VoleError, ingest::Ingest, trajectory::TrajectorySegment,
};

/// Statement / op keywords. Numbers can never collide with them, so numeric
/// value runs terminate at the next keyword.
const KEYWORDS: &[&str] = &[
    "canvas",
    "background",
    "object",
    "palette",
    "instance",
    "at",
    "move",
    "velocity",
    "advance",
    "trajectory",
    "advance_traj",
    "affine",
    "set_palette",
    "patch_palette",
    "bind",
    "copy",
    "move_rect",
    "sparse",
    "clear_instances",
    "clear_overlay",
];

fn is_keyword(tok: &str) -> bool {
    KEYWORDS.contains(&tok)
}

/// Parse a procedural script into an [`Ingest`] session. Call
/// [`Ingest::finish`] to serialize.
pub fn parse_script(text: &str) -> Result<Ingest, VoleError> {
    // Strip comments, then tokenize on whitespace.
    let mut tokens: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = match line.find('#') {
            Some(i) => &line[..i],
            None => line,
        };
        tokens.extend(line.split_whitespace().map(str::to_string));
    }
    Parser { tokens, pos: 0 }.parse()
}

struct Parser {
    tokens: Vec<String>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.pos).map(String::as_str)
    }

    fn next(&mut self) -> Option<&str> {
        let t = self.tokens.get(self.pos).map(String::as_str);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn i64(&mut self) -> Result<i64, VoleError> {
        let t = self
            .next()
            .ok_or(VoleError::ScriptParse("expected an integer"))?;
        t.parse::<i64>()
            .map_err(|_| VoleError::ScriptParse("expected an integer"))
    }

    fn u64(&mut self) -> Result<u64, VoleError> {
        let v = self.i64()?;
        u64::try_from(v).map_err(|_| VoleError::ScriptParse("integer out of range"))
    }

    fn u32(&mut self) -> Result<u32, VoleError> {
        let v = self.u64()?;
        u32::try_from(v).map_err(|_| VoleError::ScriptParse("integer out of range"))
    }

    fn byte(&mut self) -> Result<u8, VoleError> {
        let v = self.u64()?;
        u8::try_from(v).map_err(|_| VoleError::ScriptParse("byte out of range 0..=255"))
    }

    /// Consume byte values while the next token is numeric.
    fn byte_run(&mut self) -> Result<Vec<u8>, VoleError> {
        let mut out = Vec::new();
        while let Some(t) = self.peek() {
            if is_keyword(t) || !Self::is_number(t) {
                break;
            }
            let v = t
                .parse::<u64>()
                .map_err(|_| VoleError::ScriptParse("expected a byte value"))?;
            out.push(
                u8::try_from(v).map_err(|_| VoleError::ScriptParse("byte out of range 0..=255"))?,
            );
            self.pos += 1;
        }
        Ok(out)
    }

    /// Whether the token is a plain (optionally signed) integer literal.
    fn is_number(tok: &str) -> bool {
        if tok.is_empty() {
            return false;
        }
        let body = tok.strip_prefix('-').unwrap_or(tok);
        !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
    }

    fn parse(mut self) -> Result<Ingest, VoleError> {
        let mut ingest: Option<Ingest> = None;
        while self.peek().is_some() {
            let kw = self
                .next()
                .ok_or(VoleError::ScriptParse("unexpected end of script"))?
                .to_string();
            match kw.as_str() {
                "canvas" => {
                    if ingest.is_some() {
                        return Err(VoleError::ScriptParse("duplicate canvas"));
                    }
                    let w = self.u32()?;
                    let h = self.u32()?;
                    ingest = Some(Ingest::new(w, h));
                }
                "background" => {
                    let ing = ingest
                        .as_mut()
                        .ok_or(VoleError::ScriptParse("canvas first"))?;
                    ing.background(self.byte()?);
                }
                "object" => {
                    let ing = ingest
                        .as_mut()
                        .ok_or(VoleError::ScriptParse("canvas first"))?;
                    let id = self.u32()?;
                    let kind = self
                        .next()
                        .ok_or(VoleError::ScriptParse("expected object kind"))?
                        .to_string();
                    match kind.as_str() {
                        "fill" => {
                            let w = self.u32()?;
                            let h = self.u32()?;
                            let v = self.byte()?;
                            ing.declare_fill(id, w, h, v)?;
                        }
                        "gradient" => {
                            let w = self.u32()?;
                            let h = self.u32()?;
                            let b = self.byte()?;
                            let sx = self.i64()?;
                            let sy = self.i64()?;
                            ing.declare_gradient(id, w, h, b, sx, sy)?;
                        }
                        "checker" => {
                            let w = self.u32()?;
                            let h = self.u32()?;
                            let a = self.byte()?;
                            let b = self.byte()?;
                            let cell = self.u32()?;
                            let gen = crate::generator::Generator::Checker { a, b, cell };
                            gen.check()?;
                            ing.declare_generator(id, w, h, gen)?;
                        }
                        "periodic" => {
                            let w = self.u32()?;
                            let h = self.u32()?;
                            let b = self.byte()?;
                            let sx = self.i64()?;
                            let sy = self.i64()?;
                            let p = self.u32()?;
                            let gen = crate::generator::Generator::Periodic {
                                base: b,
                                sx,
                                sy,
                                period: p,
                            };
                            gen.check()?;
                            ing.declare_generator(id, w, h, gen)?;
                        }
                        "noise" => {
                            let w = self.u32()?;
                            let h = self.u32()?;
                            let seed = self.u64()?;
                            let gen = crate::generator::Generator::Noise { seed };
                            ing.declare_generator(id, w, h, gen)?;
                        }
                        "raster" | "index" => {
                            let w = self.u32()?;
                            let h = self.u32()?;
                            let n = u64::from(w)
                                .checked_mul(u64::from(h))
                                .ok_or(VoleError::ScriptParse("geometry overflow"))?;
                            let mut vals = Vec::with_capacity(n as usize);
                            for _ in 0..n {
                                vals.push(self.byte()?);
                            }
                            if kind == "raster" {
                                ing.declare_raster(id, w, h, vals)?;
                            } else {
                                ing.declare_index(id, w, h, vals)?;
                            }
                        }
                        _ => return Err(VoleError::ScriptParse("unknown object kind")),
                    }
                }
                "palette" => {
                    let ing = ingest
                        .as_mut()
                        .ok_or(VoleError::ScriptParse("canvas first"))?;
                    let id = self.u32()?;
                    let entries = self.byte_run()?;
                    ing.declare_palette(id, entries)?;
                }
                "instance" => {
                    let ing = ingest
                        .as_mut()
                        .ok_or(VoleError::ScriptParse("canvas first"))?;
                    let iid = self.u32()?;
                    let oid = self.u32()?;
                    let x = self.i64()?;
                    let y = self.i64()?;
                    if self.peek() == Some("palette") {
                        self.next();
                        let pid = self.u32()?;
                        ing.instance_binding(iid, oid, x, y, pid)?;
                    } else {
                        ing.instance(iid, oid, x, y)?;
                    }
                }
                "at" => {
                    let ing = ingest
                        .as_mut()
                        .ok_or(VoleError::ScriptParse("canvas first"))?;
                    let t = self.u64()?;
                    ing.at(t)?;
                }
                op => {
                    let ing = ingest
                        .as_mut()
                        .ok_or(VoleError::ScriptParse("canvas first"))?;
                    let op = op.to_string();
                    self.op(ing, &op)?;
                }
            }
        }
        ingest.ok_or(VoleError::ScriptParse("missing canvas"))
    }

    fn op(&mut self, ing: &mut Ingest, op: &str) -> Result<(), VoleError> {
        match op {
            "move" => {
                let iid = self.u32()?;
                let x = self.i64()?;
                let y = self.i64()?;
                ing.set_position(iid, x, y)?;
            }
            "velocity" => {
                let iid = self.u32()?;
                let vx = self.i64()?;
                let vy = self.i64()?;
                ing.set_velocity(iid, vx, vy)?;
            }
            "advance" => {
                ing.advance()?;
            }
            "trajectory" => {
                let iid = self.u32()?;
                let mut segments = Vec::new();
                loop {
                    match self.peek() {
                        Some("lin") => {
                            self.next();
                            let vx = self.i64()?;
                            let vy = self.i64()?;
                            let steps = self.u64()?;
                            let seg = TrajectorySegment::Linear { vx, vy, steps };
                            seg.check()?;
                            segments.push(seg);
                        }
                        Some("accel") => {
                            self.next();
                            let vx0 = self.i64()?;
                            let vy0 = self.i64()?;
                            let ax = self.i64()?;
                            let ay = self.i64()?;
                            let steps = self.u64()?;
                            let seg = TrajectorySegment::Accel {
                                vx0,
                                vy0,
                                ax,
                                ay,
                                steps,
                            };
                            seg.check()?;
                            segments.push(seg);
                        }
                        _ => break,
                    }
                }
                ing.set_trajectory(iid, segments)?;
            }
            "advance_traj" => {
                ing.advance_trajectories()?;
            }
            "affine" => {
                let iid = self.u32()?;
                let a = self.i64()?;
                let b = self.i64()?;
                let c = self.i64()?;
                let d = self.i64()?;
                let e = self.i64()?;
                let f = self.i64()?;
                let params = AffineParams { a, b, c, d, e, f };
                ing.set_affine(iid, params)?;
            }
            "set_palette" => {
                let id = self.u32()?;
                let entries = self.byte_run()?;
                ing.set_palette(id, entries)?;
            }
            "patch_palette" => {
                let id = self.u32()?;
                let mut changes = Vec::new();
                while let Some(t) = self.peek() {
                    if is_keyword(t) || !t.contains('=') {
                        break;
                    }
                    let pair = self
                        .next()
                        .ok_or(VoleError::ScriptParse("patch_palette expects idx=val"))?
                        .to_string();
                    let (idx, val) = pair
                        .split_once('=')
                        .ok_or(VoleError::ScriptParse("patch_palette expects idx=val"))?;
                    let idx = idx
                        .parse::<u8>()
                        .map_err(|_| VoleError::ScriptParse("bad palette index"))?;
                    let val = val
                        .parse::<u8>()
                        .map_err(|_| VoleError::ScriptParse("bad palette value"))?;
                    changes.push((idx, val));
                }
                ing.patch_palette(id, changes)?;
            }
            "bind" => {
                let iid = self.u32()?;
                let pid = self.u32()?;
                ing.bind_palette(iid, pid)?;
            }
            "copy" | "move_rect" => {
                let sx = self.i64()?;
                let sy = self.i64()?;
                let w = self.u32()?;
                let h = self.u32()?;
                let dx = self.i64()?;
                let dy = self.i64()?;
                if op == "copy" {
                    ing.copy_rect(sx, sy, w, h, dx, dy)?;
                } else {
                    ing.move_rect(sx, sy, w, h, dx, dy)?;
                }
            }
            "sparse" => {
                let mut points = Vec::new();
                while let Some(t) = self.peek() {
                    if is_keyword(t) || !Self::is_number(t) {
                        break;
                    }
                    let x = self.i64()?;
                    let y = self.i64()?;
                    let v = self.byte()?;
                    points.push((x, y, v));
                }
                ing.patch_sparse(points)?;
            }
            "clear_instances" => {
                ing.clear_instances()?;
            }
            "clear_overlay" => {
                ing.clear_overlay()?;
            }
            _ => return Err(VoleError::ScriptParse("unknown statement")),
        }
        Ok(())
    }
}
