// Reliable Live Pairing Session (Sequence / ACK / SACK / NACK / Retransmit)

use std::collections::{BTreeMap, VecDeque};

use super::cmd_op::{decode_command_op_delta, encode_command_op_delta};
use super::wire::{
    encode_packet, mark_packet_retransmit, DeltaPacketRef, PacketKind, SyncDeltaError,
    DEFAULT_SYNC_WINDOW,
};
use super::xor_rle::{decode_xor_rle_delta, encode_xor_rle_delta};

/// Live bandwidth & compression telemetry for a [`SyncDeltaSession`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncMetrics {
    pub frames_sent: u64,
    pub keyframes_sent: u64,
    pub xor_rle_deltas_sent: u64,
    pub command_op_deltas_sent: u64,
    pub retransmits_sent: u64,
    pub wire_bytes_sent: u64,
    pub raw_bytes_sent: u64,
    pub last_packet_bytes: usize,
}

impl SyncMetrics {
    /// Average wire bytes per delta packet (excluding initial keyframes).
    pub fn avg_delta_bytes(&self) -> f64 {
        let delta_count = self.xor_rle_deltas_sent + self.command_op_deltas_sent;
        if delta_count == 0 {
            return self.last_packet_bytes as f64;
        }
        // Conservative average over all non-retransmit packets if needed, or total wire / frames.
        self.wire_bytes_sent as f64 / self.frames_sent.max(1) as f64
    }

    /// Estimated steady-state bandwidth in KB/s at `edits_per_sec` based on `bytes_per_frame`.
    pub fn estimated_kbps(bytes_per_frame: usize, edits_per_sec: f64) -> f64 {
        (bytes_per_frame as f64 * edits_per_sec) / 1024.0
    }
}

#[derive(Debug, Clone)]
struct SentEntry {
    seq: u32,
    packet: Vec<u8>,
    last_sent_ms: u64,
}

/// Result of feeding one incoming wire packet into [`SyncDeltaSession::receive_packet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveOutcome {
    /// Zero or more frames applied in strict sequence order `(seq, reconstructed_frame_bytes)`.
    pub applied_frames: Vec<(u32, Vec<u8>)>,
    /// Compact ACK packet to send back to the peer (carrying cumulative `ack_seq` + 64-bit SACK).
    pub ack_packet: Option<Vec<u8>>,
    /// Any NACK packets requesting missing sequence numbers detected by a gap.
    pub nack_packets: Vec<Vec<u8>>,
}

/// Stateful bidirectional binary delta sync session for live Studio pairing over QUIC,
/// WebRTC DataChannels, or WebSockets.
#[derive(Debug, Clone)]
pub struct SyncDeltaSession {
    max_window: usize,
    // Sender state
    next_seq: u32,
    last_sent_seq: u32,
    last_sent_frame: Vec<u8>,
    sent_window: VecDeque<SentEntry>,
    metrics: SyncMetrics,
    // Receiver state
    last_applied_seq: u32,
    last_applied_frame: Vec<u8>,
    reorder_buf: BTreeMap<u32, Vec<u8>>,
}

impl Default for SyncDeltaSession {
    fn default() -> Self {
        Self::new(DEFAULT_SYNC_WINDOW)
    }
}

impl SyncDeltaSession {
    pub fn new(max_window: usize) -> Self {
        Self {
            max_window: max_window.clamp(4, 1024),
            next_seq: 1,
            last_sent_seq: 0,
            last_sent_frame: Vec::new(),
            sent_window: VecDeque::new(),
            metrics: SyncMetrics::default(),
            last_applied_seq: 0,
            last_applied_frame: Vec::new(),
            reorder_buf: BTreeMap::new(),
        }
    }

    pub fn metrics(&self) -> SyncMetrics {
        self.metrics
    }

    pub fn in_flight_count(&self) -> usize {
        self.sent_window.len()
    }

    pub fn last_applied_seq(&self) -> u32 {
        self.last_applied_seq
    }

    pub fn last_applied_frame(&self) -> &[u8] {
        &self.last_applied_frame
    }

    pub fn max_window(&self) -> usize {
        self.max_window
    }

    pub fn last_sent_seq(&self) -> u32 {
        self.last_sent_seq
    }

    pub fn last_sent_frame(&self) -> &[u8] {
        &self.last_sent_frame
    }

    pub fn reorder_buffer_len(&self) -> usize {
        self.reorder_buf.len()
    }

    /// Returns the earliest missing sequence number when out-of-order packets are buffered.
    pub fn next_missing_seq(&self) -> Option<u32> {
        if self.reorder_buf.is_empty() {
            None
        } else {
            Some(self.last_applied_seq.wrapping_add(1))
        }
    }

    /// Resets receiver sequence tracking (used when re-pairing to a fresh server epoch).
    pub fn reset_receiver(&mut self) {
        self.last_applied_seq = 0;
        self.last_applied_frame.clear();
        self.reorder_buf.clear();
    }

    /// Builds an explicit NACK packet requesting `missing_seq`.
    pub fn build_nack_packet(&self, missing_seq: u32) -> Vec<u8> {
        encode_packet(
            PacketKind::Nack,
            0,
            0,
            0,
            self.last_applied_seq,
            &missing_seq.to_le_bytes(),
        )
    }

    /// Builds an up-to-date cumulative ACK + 64-bit SACK packet.
    pub fn build_current_ack_packet(&self) -> Vec<u8> {
        self.build_ack_packet()
    }

    /// Encodes `frame_bytes`, automatically choosing the smallest of `Keyframe`, `XorRleDelta`,
    /// or `CommandOpDelta`, and inserts the packet into the bounded retransmit window.
    pub fn encode_frame(&mut self, frame_bytes: &[u8], now_ms: u64) -> Vec<u8> {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1).max(1);

        let (kind, base_seq, payload) = if self.last_sent_seq == 0 || self.last_sent_frame.is_empty()
        {
            (PacketKind::Keyframe, 0, frame_bytes.to_vec())
        } else {
            let xor_delta = encode_xor_rle_delta(&self.last_sent_frame, frame_bytes);
            let cmd_delta = encode_command_op_delta(&self.last_sent_frame, frame_bytes);

            let mut best_kind = PacketKind::XorRleDelta;
            let mut best_payload = xor_delta;

            if let Some(cd) = cmd_delta {
                if cd.len() < best_payload.len() {
                    best_kind = PacketKind::CommandOpDelta;
                    best_payload = cd;
                }
            }

            if frame_bytes.len() <= best_payload.len() {
                (PacketKind::Keyframe, 0, frame_bytes.to_vec())
            } else {
                (best_kind, self.last_sent_seq, best_payload)
            }
        };

        let packet = encode_packet(kind, 0, seq, base_seq, self.last_applied_seq, &payload);

        self.last_sent_seq = seq;
        self.last_sent_frame.clear();
        self.last_sent_frame.extend_from_slice(frame_bytes);

        match kind {
            PacketKind::Keyframe => self.metrics.keyframes_sent += 1,
            PacketKind::XorRleDelta => self.metrics.xor_rle_deltas_sent += 1,
            PacketKind::CommandOpDelta => self.metrics.command_op_deltas_sent += 1,
            _ => {}
        }
        self.metrics.frames_sent += 1;
        self.metrics.raw_bytes_sent += frame_bytes.len() as u64;
        self.metrics.wire_bytes_sent += packet.len() as u64;
        self.metrics.last_packet_bytes = packet.len();

        self.push_sent_window(seq, packet.clone(), now_ms);
        packet
    }

    /// Forces a standalone keyframe packet (useful after prolonged packet loss or new peer join).
    pub fn encode_keyframe(&mut self, frame_bytes: &[u8], now_ms: u64) -> Vec<u8> {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1).max(1);
        let packet = encode_packet(
            PacketKind::Keyframe,
            0,
            seq,
            0,
            self.last_applied_seq,
            frame_bytes,
        );
        self.last_sent_seq = seq;
        self.last_sent_frame.clear();
        self.last_sent_frame.extend_from_slice(frame_bytes);
        self.metrics.frames_sent += 1;
        self.metrics.keyframes_sent += 1;
        self.metrics.raw_bytes_sent += frame_bytes.len() as u64;
        self.metrics.wire_bytes_sent += packet.len() as u64;
        self.metrics.last_packet_bytes = packet.len();
        self.push_sent_window(seq, packet.clone(), now_ms);
        packet
    }

    fn push_sent_window(&mut self, seq: u32, packet: Vec<u8>, now_ms: u64) {
        if self.sent_window.len() >= self.max_window {
            self.sent_window.pop_front();
        }
        self.sent_window.push_back(SentEntry {
            seq,
            packet,
            last_sent_ms: now_ms,
        });
    }

    /// Processes an incoming ACK packet (cumulative `ack_seq` + 64-bit SACK bitmap), evicting
    /// acknowledged packets from the retransmit window. Returns the number of evicted packets.
    pub fn handle_ack(&mut self, packet_bytes: &[u8]) -> Result<usize, SyncDeltaError> {
        let parsed = DeltaPacketRef::parse(packet_bytes)?;
        let hdr = parsed.header();
        if hdr.kind != PacketKind::Ack {
            return Err(SyncDeltaError::InvalidPacketKind(hdr.kind as u8));
        }
        if parsed.payload().len() != 8 {
            return Err(SyncDeltaError::CorruptPayload("ACK payload must be 8-byte SACK bitmap"));
        }
        let sack_bitmap = u64::from_le_bytes(parsed.payload().try_into().unwrap());
        let cum_ack = hdr.ack_seq;

        let before = self.sent_window.len();
        self.sent_window.retain(|entry| {
            if entry.seq <= cum_ack {
                return false;
            }
            let diff = entry.seq - cum_ack;
            if (1..=64).contains(&diff) && (sack_bitmap & (1u64 << (diff - 1))) != 0 {
                return false;
            }
            true
        });
        Ok(before - self.sent_window.len())
    }

    /// Processes an incoming NACK packet requesting `missing_seq`.
    ///
    /// Returns the retransmitted packet (with `FLAG_RETRANSMIT`) if still in the window, or a
    /// fresh `Keyframe` of the latest state if the missing packet aged out of the window.
    pub fn handle_nack(
        &mut self,
        packet_bytes: &[u8],
        now_ms: u64,
    ) -> Result<Option<Vec<u8>>, SyncDeltaError> {
        let parsed = DeltaPacketRef::parse(packet_bytes)?;
        let hdr = parsed.header();
        if hdr.kind != PacketKind::Nack {
            return Err(SyncDeltaError::InvalidPacketKind(hdr.kind as u8));
        }
        if parsed.payload().len() != 4 {
            return Err(SyncDeltaError::CorruptPayload("NACK payload must be 4-byte missing_seq"));
        }
        let missing_seq = u32::from_le_bytes(parsed.payload().try_into().unwrap());

        if let Some(entry) = self.sent_window.iter_mut().find(|e| e.seq == missing_seq) {
            entry.last_sent_ms = now_ms;
            self.metrics.retransmits_sent += 1;
            let retx = mark_packet_retransmit(&entry.packet);
            self.metrics.wire_bytes_sent += retx.len() as u64;
            return Ok(Some(retx));
        }

        // Missing packet fell out of the retransmit window -> recover cleanly with a fresh Keyframe.
        if !self.last_sent_frame.is_empty() {
            let latest = self.last_sent_frame.clone();
            let kf = self.encode_keyframe(&latest, now_ms);
            self.metrics.retransmits_sent += 1;
            return Ok(Some(kf));
        }
        Ok(None)
    }

    /// Returns any unacknowledged packets whose retransmission timer (`rto_ms`) has elapsed.
    pub fn poll_retransmits(&mut self, now_ms: u64, rto_ms: u64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for entry in &mut self.sent_window {
            if now_ms.saturating_sub(entry.last_sent_ms) >= rto_ms {
                entry.last_sent_ms = now_ms;
                self.metrics.retransmits_sent += 1;
                let retx = mark_packet_retransmit(&entry.packet);
                self.metrics.wire_bytes_sent += retx.len() as u64;
                out.push(retx);
            }
        }
        out
    }

    /// Receives and validates one wire packet on the peer side, applying it immediately if in
    /// order or buffering it in the bounded reorder window if out of order.
    pub fn receive_packet(&mut self, packet_bytes: &[u8]) -> Result<ReceiveOutcome, SyncDeltaError> {
        let parsed = DeltaPacketRef::parse(packet_bytes)?;
        let hdr = parsed.header();

        match hdr.kind {
            PacketKind::Ack => {
                let _ = self.handle_ack(packet_bytes)?;
                return Ok(ReceiveOutcome {
                    applied_frames: Vec::new(),
                    ack_packet: None,
                    nack_packets: Vec::new(),
                });
            }
            PacketKind::Nack => {
                return Ok(ReceiveOutcome {
                    applied_frames: Vec::new(),
                    ack_packet: None,
                    nack_packets: Vec::new(),
                });
            }
            PacketKind::Keyframe | PacketKind::XorRleDelta | PacketKind::CommandOpDelta => {}
        }

        // Ignore duplicate / already-applied sequence numbers, still returning an up-to-date ACK.
        if hdr.seq <= self.last_applied_seq {
            return Ok(ReceiveOutcome {
                applied_frames: Vec::new(),
                ack_packet: Some(self.build_ack_packet()),
                nack_packets: Vec::new(),
            });
        }

        let mut applied_frames = Vec::new();
        let mut nack_packets = Vec::new();

        let expected_next = self.last_applied_seq + 1;
        if hdr.kind == PacketKind::Keyframe && hdr.seq >= expected_next {
            // A fresh keyframe resets the baseline immediately even after a gap.
            let frame = parsed.payload().to_vec();
            self.last_applied_seq = hdr.seq;
            self.last_applied_frame = frame.clone();
            applied_frames.push((hdr.seq, frame));
            // Drop stale buffered packets older than the keyframe.
            self.reorder_buf.retain(|&s, _| s > self.last_applied_seq);
            self.drain_reorder_buffer(&mut applied_frames)?;
        } else if hdr.seq == expected_next {
            let frame = self.apply_single_packet(parsed)?;
            self.last_applied_seq = hdr.seq;
            self.last_applied_frame = frame.clone();
            applied_frames.push((hdr.seq, frame));
            self.drain_reorder_buffer(&mut applied_frames)?;
        } else {
            // Out-of-order delta packet: buffer up to `max_window` and emit NACKs for the gap.
            if self.reorder_buf.len() < self.max_window {
                self.reorder_buf.insert(hdr.seq, packet_bytes.to_vec());
            }
            let gap_end = hdr.seq.min(expected_next.saturating_add(8));
            for missing in expected_next..gap_end {
                if !self.reorder_buf.contains_key(&missing) {
                    nack_packets.push(encode_packet(
                        PacketKind::Nack,
                        0,
                        0,
                        0,
                        self.last_applied_seq,
                        &missing.to_le_bytes(),
                    ));
                }
            }
        }

        Ok(ReceiveOutcome {
            applied_frames,
            ack_packet: Some(self.build_ack_packet()),
            nack_packets,
        })
    }

    fn apply_single_packet(&self, pkt: DeltaPacketRef<'_>) -> Result<Vec<u8>, SyncDeltaError> {
        let hdr = pkt.header();
        match hdr.kind {
            PacketKind::Keyframe => Ok(pkt.payload().to_vec()),
            PacketKind::XorRleDelta => {
                if hdr.base_seq != self.last_applied_seq {
                    return Err(SyncDeltaError::BaseSequenceMismatch {
                        expected: self.last_applied_seq,
                        actual: hdr.base_seq,
                    });
                }
                decode_xor_rle_delta(&self.last_applied_frame, pkt.payload())
            }
            PacketKind::CommandOpDelta => {
                if hdr.base_seq != self.last_applied_seq {
                    return Err(SyncDeltaError::BaseSequenceMismatch {
                        expected: self.last_applied_seq,
                        actual: hdr.base_seq,
                    });
                }
                decode_command_op_delta(&self.last_applied_frame, pkt.payload())
            }
            PacketKind::Ack | PacketKind::Nack => Err(SyncDeltaError::InvalidPacketKind(hdr.kind as u8)),
        }
    }

    fn drain_reorder_buffer(
        &mut self,
        applied: &mut Vec<(u32, Vec<u8>)>,
    ) -> Result<(), SyncDeltaError> {
        while let Some(raw) = self.reorder_buf.remove(&(self.last_applied_seq + 1)) {
            let pkt = DeltaPacketRef::parse(&raw)?;
            let frame = self.apply_single_packet(pkt)?;
            let seq = pkt.header().seq;
            self.last_applied_seq = seq;
            self.last_applied_frame = frame.clone();
            applied.push((seq, frame));
        }
        Ok(())
    }

    fn build_ack_packet(&self) -> Vec<u8> {
        let mut sack = 0u64;
        for &buffered_seq in self.reorder_buf.keys() {
            if buffered_seq > self.last_applied_seq {
                let diff = buffered_seq - self.last_applied_seq;
                if (1..=64).contains(&diff) {
                    sack |= 1u64 << (diff - 1);
                }
            }
        }
        encode_packet(
            PacketKind::Ack,
            0,
            0,
            0,
            self.last_applied_seq,
            &sack.to_le_bytes(),
        )
    }
}
