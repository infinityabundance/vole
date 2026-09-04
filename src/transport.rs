//! Phase R — procedural transport (master brief §34 / §35 / §36, §33
//! measurement).
//!
//! Transport organizes a standalone `.vole` stream into a sequence of five
//! packet classes — **OBJECT CHECKPOINT TRANSITION(INTERVAL) RESIDUAL
//! INTEGRITY** — so a receiver maintains state incrementally instead of
//! receiving an independent compressed raster per interval. This module is a
//! *framing and ordering* layer only: every packet payload is the byte-exact
//! v1 record the standalone writer emits (shared record helpers in
//! `format.rs`), the receiver rebuilds the canonical prefix and plays it
//! through the **normative parser/materializer** (`frames_so_far` decodes the
//! received prefix), and integrity is the standalone stream's own BLAKE3
//! trailer semantics. No normative decode path is duplicated, so the
//! materializer stays authoritative.
//!
//! ```text
//!   frame := [len:u32][kind:u8][seq:u64][body]      len = 9 + body.len()
//!   kinds:  0x00 HEADER | 0x01 OBJECT | 0x02 PALETTE | 0x03 CHECKPOINT
//!           | 0x04 INTERVAL | 0x7E INTEGRITY
//! ```
//!
//! Sequence numbers detect packet loss: a receiver fed frames out of order
//! fails with [`VoleError::TransportGap`] and recovers by re-feeding from the
//! first missing sequence (the transmitter can `encode_from(seq)`). A
//! receiver that fell too far behind rolls back to the checkpoint
//! (`reset_to_checkpoint`) and replays forward — replay is bounded by the
//! v1 envelope (`Limits.max_transition_replay` / `max_checkpoint_distance`)
//! exactly as the standalone decoder's replay is.
//!
//! Concatenating every packet payload reproduces the source stream's bytes
//! for canonical sources (identical semantics always): the reassembled stream
//! decodes to the identical raster sequence, which the courts assert.
//!
//! Streams carrying external object declarations (Phase P) are store-backed
//! and not standalone; transport rejects them in this phase (recorded
//! boundary).

use std::collections::HashSet;

use crate::{
    checked::{ByteReader, ByteSink},
    decoder,
    error::VoleError,
    format,
    object::ObjectId,
    pixel::Canvas,
    state::{Instance, PaletteId},
    transition::Transition,
};

/// Frame kind: stream header (canvas/universe/profile/features).
pub const KIND_HEADER: u8 = 0x00;
/// Frame kind: one immutable object declaration.
pub const KIND_OBJECT: u8 = 0x01;
/// Frame kind: one palette-table declaration.
pub const KIND_PALETTE: u8 = 0x02;
/// Frame kind: the checkpoint (interval-0 state).
pub const KIND_CHECKPOINT: u8 = 0x03;
/// Frame kind: one interval group of state transitions / canvas ops.
pub const KIND_INTERVAL: u8 = 0x04;
/// Frame kind: integrity trailer digest.
pub const KIND_INTEGRITY: u8 = 0x7E;

/// Frame header overhead per packet: `len:u32 + kind:u8 + seq:u64`.
pub const FRAME_OVERHEAD: u64 = 13;

/// One typed transport packet. Payload bytes are generated from these through
/// the shared `format::*_bytes` record helpers, so a packet body is always
/// byte-identical to the standalone writer's record.
#[derive(Debug, Clone)]
pub enum Packet {
    /// Stream header (v1 standalone only in this phase). `feature_bits` are
    /// preserved from the source stream (quantized-content declarations, Phase
    /// U, survive transport; external-object streams are refused).
    Header {
        width: u32,
        height: u32,
        feature_bits: u32,
    },
    /// Immutable object declaration.
    Object {
        id: u32,
        object: crate::object::Object,
    },
    /// Palette-table declaration (Phase J).
    Palette { id: u32, entries: Vec<u8> },
    /// Checkpoint instances (with palette bindings); `variant_08` records
    /// whether the source used the palette-binding checkpoint tag.
    Checkpoint {
        background: u8,
        instances: Vec<(Instance, Option<PaletteId>)>,
        variant_08: bool,
    },
    /// Interval group at absolute `t`.
    Interval {
        t: u64,
        transitions: Vec<Transition>,
    },
    /// Integrity digest: the standalone stream's BLAKE3 trailer value
    /// (digest of the prefix). Verified at the receiver.
    Integrity { digest: [u8; 32] },
}

impl Packet {
    /// Payload (frame body) bytes of the packet.
    pub fn body(&self) -> Result<Vec<u8>, VoleError> {
        match self {
            Packet::Header {
                width,
                height,
                feature_bits,
            } => {
                let h = format::Header::v1(*width, *height, *feature_bits);
                format::header_bytes(&h)
            }
            Packet::Object { id, object } => format::object_decl_bytes(ObjectId(*id), object),
            Packet::Palette { id, entries } => format::palette_decl_bytes(PaletteId(*id), entries),
            Packet::Checkpoint {
                background,
                instances,
                variant_08,
            } => {
                if *variant_08 {
                    format::checkpoint_bindings_bytes(*background, instances)
                } else {
                    let plain: Vec<Instance> = instances.iter().map(|(i, _)| i.clone()).collect();
                    format::checkpoint_bytes(*background, &plain)
                }
            }
            Packet::Interval { t, transitions } => format::interval_bytes(*t, transitions),
            Packet::Integrity { digest } => Ok(digest.to_vec()),
        }
    }

    /// Frame kind tag.
    pub fn kind(&self) -> u8 {
        match self {
            Packet::Header { .. } => KIND_HEADER,
            Packet::Object { .. } => KIND_OBJECT,
            Packet::Palette { .. } => KIND_PALETTE,
            Packet::Checkpoint { .. } => KIND_CHECKPOINT,
            Packet::Interval { .. } => KIND_INTERVAL,
            Packet::Integrity { .. } => KIND_INTEGRITY,
        }
    }

    /// Body length in bytes.
    pub fn body_len(&self) -> Result<u64, VoleError> {
        Ok(self.body()?.len() as u64)
    }

    /// Framed length (frame header + body).
    pub fn framed_len(&self) -> Result<u64, VoleError> {
        Ok(FRAME_OVERHEAD + self.body_len()?)
    }
}

// ---------------------------------------------------------------------------
// Transmitter
// ---------------------------------------------------------------------------

/// The transmitting side: a standalone `.vole` stream packetized into the
/// canonical emission order
/// `HEADER OBJECT* PALETTE* CHECKPOINT INTERVAL* INTEGRITY`.
pub struct Transmitter {
    packets: Vec<Packet>,
}

impl Transmitter {
    /// Packetize a **standalone** `.vole` stream (store-backed streams with
    /// external object declarations are rejected: transport of a non-standalone
    /// stream is a recorded Phase-R boundary). Quantized-content declarations
    /// (Phase U feature bit 0x2) are standalone and are preserved.
    pub fn packetize(bytes: &[u8]) -> Result<Self, VoleError> {
        let parsed = decoder::decode_bytes(bytes)?;
        let header = parsed.header();
        if header.feature_bits() & crate::format::FEAT_EXTERNAL_OBJECTS != 0 {
            return Err(VoleError::ApiConstraint(
                "transport requires a standalone stream (no external objects)",
            ));
        }
        let initial = parsed.clone_initial();
        let mut packets: Vec<Packet> = Vec::new();
        packets.push(Packet::Header {
            width: parsed.width(),
            height: parsed.height(),
            feature_bits: header.feature_bits(),
        });
        for (id, obj) in initial.objects() {
            packets.push(Packet::Object {
                id: id.0,
                object: obj.clone(),
            });
        }
        let mut palettes: Vec<(u32, Vec<u8>)> = Vec::new();
        for (pid, entries) in initial.palettes() {
            palettes.push((pid.0, entries.to_vec()));
            packets.push(Packet::Palette {
                id: pid.0,
                entries: entries.to_vec(),
            });
        }
        let instances: Vec<(Instance, Option<PaletteId>)> = initial
            .instances()
            .map(|inst| (inst.clone(), initial.binding(inst.id)))
            .collect();
        let variant_08 = !palettes.is_empty() || initial.binding_count() > 0;
        packets.push(Packet::Checkpoint {
            background: initial.background(),
            instances,
            variant_08,
        });
        for (t, trs) in parsed.intervals() {
            packets.push(Packet::Interval {
                t: t.0,
                transitions: trs.clone(),
            });
        }
        // Integrity digest: the standalone trailer value (digest of the
        // concatenated prefix = header + decls + checkpoint + intervals).
        let mut prefix = Vec::new();
        for p in &packets {
            prefix.extend_from_slice(&p.body()?);
        }
        let digest = crate::integr::digest(&prefix);
        packets.push(Packet::Integrity { digest });
        Ok(Transmitter { packets })
    }

    /// Every packet in emission order.
    pub fn packets(&self) -> &[Packet] {
        &self.packets
    }

    /// Number of packets (including the integrity packet).
    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    /// Number of interval packets.
    pub fn interval_count(&self) -> usize {
        self.packets
            .iter()
            .filter(|p| matches!(p, Packet::Interval { .. }))
            .count()
    }

    /// Sum of packet payload bytes (equals the source stream length for
    /// canonical sources: payloads are the exact stream records).
    pub fn payload_bytes(&self) -> Result<u64, VoleError> {
        let mut n = 0u64;
        for p in &self.packets {
            n = n
                .checked_add(p.body_len()?)
                .ok_or(VoleError::ArithmeticOverflow)?;
        }
        Ok(n)
    }

    /// Framed total (payloads + 13 bytes per packet).
    pub fn framed_bytes(&self) -> Result<u64, VoleError> {
        self.payload_bytes()?
            .checked_add(FRAME_OVERHEAD * self.packets.len() as u64)
            .ok_or(VoleError::ArithmeticOverflow)
    }

    /// The standalone integrity digest carried by the final packet.
    pub fn digest(&self) -> Result<[u8; 32], VoleError> {
        match self.packets.last() {
            Some(Packet::Integrity { digest }) => Ok(*digest),
            _ => Err(VoleError::TransportFormat("no integrity packet")),
        }
    }

    /// Encode every frame (length-prefixed, sequence-numbered) as a byte
    /// stream.
    pub fn encode(&self) -> Result<Vec<u8>, VoleError> {
        let mut out = ByteSink::new();
        for (seq, p) in self.packets.iter().enumerate() {
            out.extend(&Self::encode_one(seq as u64, p)?)?;
        }
        Ok(out.into_vec())
    }

    /// Encode the frames from `seq` onward (retransmission from a gap or from
    /// the checkpoint).
    pub fn encode_from(&self, seq: u64) -> Result<Vec<u8>, VoleError> {
        if seq as usize >= self.packets.len() {
            return Err(VoleError::ApiConstraint("retransmit sequence out of range"));
        }
        let mut out = ByteSink::new();
        for (i, p) in self.packets.iter().enumerate().skip(seq as usize) {
            out.extend(&Self::encode_one(i as u64, p)?)?;
        }
        Ok(out.into_vec())
    }

    /// Sequence of the first interval packet (the checkpoint-recovery resume
    /// point): header(0) + object/palette declarations + checkpoint.
    pub fn first_interval_seq(&self) -> u64 {
        let mut seq = 0u64;
        for p in &self.packets {
            if matches!(p, Packet::Interval { .. }) {
                break;
            }
            seq += 1;
        }
        seq
    }

    /// One framed packet.
    fn encode_one(seq: u64, p: &Packet) -> Result<Vec<u8>, VoleError> {
        let body = p.body()?;
        let mut f = ByteSink::new();
        let len = u32::try_from(9 + body.len()).map_err(|_| VoleError::ArithmeticOverflow)?;
        f.push(len)?;
        f.byte(p.kind())?;
        f.push(seq)?;
        f.extend(&body)?;
        Ok(f.into_vec())
    }
}

// ---------------------------------------------------------------------------
// Receiver
// ---------------------------------------------------------------------------

/// Receiving side: applies packets in sequence, maintaining the canonical
/// stream prefix, and materializes partial frames through the **normative
/// parser** (never a duplicated state machine).
pub struct Receiver {
    /// Canonical stream prefix rebuilt from applied packets (ends at a record
    /// boundary after every feed).
    prefix: Vec<u8>,
    /// Bytes of the prefix at the checkpoint (the recovery rollback point).
    checkpoint_end: usize,
    next_seq: u64,
    object_ids: HashSet<u32>,
    palette_ids: HashSet<u32>,
    saw_header: bool,
    saw_checkpoint: bool,
    last_interval_t: u64,
    /// Counters that survive `reset_to_checkpoint` (declaration packets are
    /// never rolled back).
    decl_packets: u64,
    applied_packets: u64,
    applied_bytes: u64,
    interval_packets: u64,
    integrity_digest: Option<[u8; 32]>,
}

impl Default for Receiver {
    fn default() -> Self {
        Self::new()
    }
}

impl Receiver {
    /// A fresh receiver expecting sequence 0 (the header packet).
    pub fn new() -> Self {
        Receiver {
            prefix: Vec::new(),
            checkpoint_end: 0,
            next_seq: 0,
            object_ids: HashSet::new(),
            palette_ids: HashSet::new(),
            saw_header: false,
            saw_checkpoint: false,
            last_interval_t: 0,
            decl_packets: 0,
            applied_packets: 0,
            applied_bytes: 0,
            interval_packets: 0,
            integrity_digest: None,
        }
    }

    /// Feed one framed packet. Applies it when its sequence is the expected
    /// next one; a lost packet (out-of-sequence feed) is [`VoleError::TransportGap`]
    /// and nothing is applied. Non-canonical ordering or framing is a typed
    /// [`VoleError::TransportFormat`].
    pub fn feed(&mut self, frame: &[u8]) -> Result<(), VoleError> {
        let (kind, seq, body) = parse_frame(frame)?;
        if seq != self.next_seq {
            return Err(VoleError::TransportGap);
        }
        // Semantic ordering rules.
        match kind {
            KIND_HEADER => {
                if self.saw_header || !self.prefix.is_empty() {
                    return Err(VoleError::TransportFormat("header not first"));
                }
                self.saw_header = true;
            }
            KIND_OBJECT | KIND_PALETTE => {
                if self.saw_checkpoint {
                    return Err(VoleError::TransportFormat("declaration after checkpoint"));
                }
                if !self.saw_header {
                    return Err(VoleError::TransportFormat("header required first"));
                }
            }
            KIND_CHECKPOINT => {
                if self.saw_checkpoint {
                    return Err(VoleError::TransportFormat("duplicate checkpoint"));
                }
                if !self.saw_header {
                    return Err(VoleError::TransportFormat("header required first"));
                }
            }
            KIND_INTERVAL => {
                if !self.saw_checkpoint {
                    return Err(VoleError::TransportFormat("interval before checkpoint"));
                }
                let t = interval_time(body)?;
                if t == 0 || t <= self.last_interval_t {
                    return Err(VoleError::TransportFormat("non-consecutive interval"));
                }
                self.last_interval_t = t;
            }
            KIND_INTEGRITY => {
                if body.len() != 32 {
                    return Err(VoleError::TransportFormat(
                        "integrity body must be 32 bytes",
                    ));
                }
                if self.integrity_digest.is_some() {
                    return Err(VoleError::TransportFormat("duplicate integrity packet"));
                }
            }
            _ => return Err(VoleError::TransportFormat("unknown frame kind")),
        }
        // Duplicate declaration ids are caught here and again by the parser.
        if kind == KIND_OBJECT {
            let id = decl_id(body)?;
            if !self.object_ids.insert(id) {
                return Err(VoleError::DuplicateId);
            }
        }
        if kind == KIND_PALETTE {
            let id = decl_id(body)?;
            if id == 0 {
                return Err(VoleError::TransportFormat("palette id zero"));
            }
            if !self.palette_ids.insert(id) {
                return Err(VoleError::DuplicateId);
            }
        }
        // Apply.
        match kind {
            KIND_CHECKPOINT => {
                self.checkpoint_end = self.prefix.len() + body.len();
                self.saw_checkpoint = true;
                self.prefix.extend_from_slice(body);
            }
            KIND_INTEGRITY => {
                let mut d = [0u8; 32];
                d.copy_from_slice(body);
                self.integrity_digest = Some(d);
            }
            KIND_INTERVAL => {
                self.interval_packets += 1;
                self.prefix.extend_from_slice(body);
            }
            _ => {
                if kind == KIND_OBJECT || kind == KIND_PALETTE {
                    self.decl_packets += 1;
                }
                self.prefix.extend_from_slice(body);
            }
        }
        self.applied_packets += 1;
        self.applied_bytes = self.applied_bytes.saturating_add(body.len() as u64);
        self.next_seq += 1;
        Ok(())
    }

    /// Whether the checkpoint has been received (frames become available).
    pub fn has_checkpoint(&self) -> bool {
        self.saw_checkpoint
    }

    /// Whether the integrity packet has been received (stream complete).
    pub fn complete(&self) -> bool {
        self.integrity_digest.is_some()
    }

    /// Next expected sequence.
    pub fn expected_seq(&self) -> u64 {
        self.next_seq
    }

    /// Packets applied.
    pub fn applied_packets(&self) -> u64 {
        self.applied_packets
    }

    /// Payload bytes applied.
    pub fn applied_bytes(&self) -> u64 {
        self.applied_bytes
    }

    /// Interval packets applied.
    pub fn interval_count_applied(&self) -> u64 {
        self.interval_packets
    }

    /// Number of object declarations received.
    pub fn object_count(&self) -> usize {
        self.object_ids.len()
    }

    /// Length of the rebuilt canonical prefix.
    pub fn prefix_len(&self) -> usize {
        self.prefix.len()
    }

    /// Materialize the frames available from the received prefix (frame 0 once
    /// the checkpoint has arrived; one further frame per applied interval).
    /// Playback runs through the **normative parser** on the prefix plus its
    /// digest trailer — the authoritative decoder, never a transport-side
    /// state machine.
    pub fn frames_so_far(&self) -> Result<Vec<Canvas>, VoleError> {
        if !self.saw_checkpoint {
            return Err(VoleError::InvalidStatePhase);
        }
        let mut full = self.prefix.clone();
        let d = crate::integr::digest(&self.prefix);
        full.extend_from_slice(&d);
        let parsed = decoder::decode_bytes(&full)?;
        decoder::materialize_all(&parsed)
    }

    /// Verify the integrity digest against the rebuilt prefix (the standalone
    /// stream's trailer semantics). Requires the integrity packet.
    pub fn verify(&self) -> Result<bool, VoleError> {
        match self.integrity_digest {
            Some(d) => Ok(crate::integr::digest(&self.prefix) == d),
            None => Err(VoleError::TransportFormat("integrity packet not received")),
        }
    }

    /// Reassemble the complete standalone stream (prefix + digest trailer).
    /// For canonical sources the reassembled bytes equal the transmitted
    /// stream byte-for-byte; for every source they decode identically.
    pub fn reassemble(&self) -> Result<Vec<u8>, VoleError> {
        if !self.complete() {
            return Err(VoleError::TransportFormat("stream incomplete"));
        }
        let mut full = self.prefix.clone();
        let d = crate::integr::digest(&self.prefix);
        full.extend_from_slice(&d);
        Ok(full)
    }

    /// Checkpoint recovery: roll the applied state back to the checkpoint,
    /// discarding applied intervals. Declaration packets are kept, so the
    /// receiver resumes at the first interval packet (`first_interval_seq`)
    /// and replays forward — replay work is bounded by the v1 decode envelope.
    pub fn reset_to_checkpoint(&mut self) {
        self.prefix.truncate(self.checkpoint_end);
        self.saw_checkpoint = true;
        self.integrity_digest = None;
        // next sequence: header(1) + object/palette packets + checkpoint(1).
        self.next_seq = 2 + self.decl_packets;
        self.last_interval_t = 0;
        self.interval_packets = 0;
        self.applied_bytes = self.prefix.len() as u64;
        self.applied_packets = self.next_seq;
    }

    /// Number of intervals that will be replayed after a checkpoint reset
    /// (measured recovery work; bounded by the decode envelope).
    pub fn checkpoint_end_len(&self) -> usize {
        self.checkpoint_end
    }
}

// ---------------------------------------------------------------------------
// Framing parse helpers
// ---------------------------------------------------------------------------

/// Parse one frame: returns `(kind, seq, body)`.
pub fn parse_frame(frame: &[u8]) -> Result<(u8, u64, &[u8]), VoleError> {
    if frame.len() < FRAME_OVERHEAD as usize {
        return Err(VoleError::Truncated);
    }
    let mut r = ByteReader::new(frame);
    let len = r.pull::<u32>()? as usize;
    if len < 9 || len != frame.len() - 4 {
        return Err(VoleError::TransportFormat("bad frame length"));
    }
    let kind = r.u8()?;
    let seq = r.pull::<u64>()?;
    let body = r.take(len - 9)?;
    Ok((kind, seq, body))
}

/// Interval time from an interval record body (`TAG_INTERVAL t:u64 count...`).
fn interval_time(body: &[u8]) -> Result<u64, VoleError> {
    if body.len() < 13 || body[0] != format::TAG_INTERVAL {
        return Err(VoleError::TransportFormat("malformed interval body"));
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(&body[1..9]);
    Ok(u64::from_le_bytes(b))
}

/// Object/palette declaration id from a decl record body (`tag id:u32 ...`).
fn decl_id(body: &[u8]) -> Result<u32, VoleError> {
    if body.len() < 5 {
        return Err(VoleError::TransportFormat("malformed declaration body"));
    }
    let mut b = [0u8; 4];
    b.copy_from_slice(&body[1..5]);
    Ok(u32::from_le_bytes(b))
}

// ---------------------------------------------------------------------------
// Structural-innovation measurement (§33)
// ---------------------------------------------------------------------------

/// Per-interval transport statistics: bytes sent per interval and the
/// structural events that interval carried (the §33 "bandwidth responds to
/// structural innovation, not raster resolution" measurement).
#[derive(Debug, Clone)]
pub struct IntervalStat {
    /// Absolute interval time.
    pub t: u64,
    /// Frame index this interval yields (interval ordinal + 1).
    pub frame: u64,
    /// Interval record payload bytes.
    pub packet_bytes: u64,
    /// Framed bytes (payload + frame header).
    pub framed_bytes: u64,
    /// Number of state transitions / canvas ops in the interval.
    pub transitions: usize,
    /// Structural event classes carried by the interval.
    pub events: Vec<&'static str>,
    /// Residual payload bytes carried by the interval (0 when the procedural
    /// state explained the frame exactly).
    pub residual_bytes: u64,
}

/// A transport profile of a standalone stream: object/checkpoint/interval
/// sizing plus the §33 per-interval structural-innovation series.
#[derive(Debug, Clone)]
pub struct TransportProfile {
    /// Canvas geometry.
    pub width: u32,
    pub height: u32,
    /// Number of frames (checkpoint + intervals).
    pub frames: u64,
    /// Number of object declarations.
    pub objects: usize,
    /// Packet count and byte totals.
    pub packet_count: usize,
    pub payload_bytes: u64,
    pub framed_bytes: u64,
    /// Per-interval rows.
    pub intervals: Vec<IntervalStat>,
}

/// Profile a standalone stream for transport (bytes over time vs structural
/// events over time).
pub fn profile_stream(bytes: &[u8]) -> Result<TransportProfile, VoleError> {
    let parsed = decoder::decode_bytes(bytes)?;
    let tx = Transmitter::packetize(bytes)?;
    let mut intervals = Vec::new();
    for p in tx.packets() {
        if let Packet::Interval { t, transitions } = p {
            let body = p.body()?;
            let mut events = Vec::new();
            let mut residual = 0u64;
            for tr in transitions {
                events.push(event_label(tr));
                if let Transition::Residual { block } = tr {
                    residual = residual.saturating_add(block.len() as u64);
                }
            }
            intervals.push(IntervalStat {
                t: *t,
                frame: intervals.len() as u64 + 1,
                packet_bytes: body.len() as u64,
                framed_bytes: FRAME_OVERHEAD + body.len() as u64,
                transitions: transitions.len(),
                events,
                residual_bytes: residual,
            });
        }
    }
    Ok(TransportProfile {
        width: parsed.width(),
        height: parsed.height(),
        frames: 1 + intervals.len() as u64,
        objects: parsed.clone_initial().object_count(),
        packet_count: tx.packet_count(),
        payload_bytes: tx.payload_bytes()?,
        framed_bytes: tx.framed_bytes()?,
        intervals,
    })
}

/// Structural event label of one transition (§33 event class).
pub fn event_label(tr: &Transition) -> &'static str {
    match tr {
        Transition::CreateInstance { .. } => "create_instance",
        Transition::SetPosition { .. } => "position",
        Transition::SetVelocity { .. } => "velocity",
        Transition::AdvanceTranslations => "advance",
        Transition::SetTrajectory { .. } => "trajectory",
        Transition::AdvanceTrajectories => "advance_trajectory",
        Transition::SetPalette { .. } => "set_palette",
        Transition::PatchPalette { .. } => "patch_palette",
        Transition::BindPalette { .. } => "bind_palette",
        Transition::SetAffine { .. } => "affine",
        Transition::ClearInstances => "clear_instances",
        Transition::ClearOverlay => "clear_overlay",
        Transition::Residual { .. } => "residual",
        Transition::PatchSparse { .. } => "sparse",
        Transition::CopyRect { .. } => "copy",
        Transition::MoveRect { .. } => "move",
        Transition::DeclareObject(..) | Transition::DeclareFill { .. } => "declare",
    }
}
