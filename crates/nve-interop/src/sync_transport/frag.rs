//! UDP MTU-safe packetization (`<= 1200` bytes) and bounded fragment reassembly.

use nve_abi::hash::HashMap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::sync_delta::{fnv1a_64, DELTA_HEADER_SIZE};

use super::protocol::{
    encode_envelope, TransportError, TransportMsgKind, FRAG_SUBHEADER_SIZE, MAX_UDP_DATAGRAM_BYTES,
    MAX_UDP_FRAG_CHUNK, MAX_UDP_SINGLE_PAYLOAD, TRANSPORT_HEADER_SIZE, TRANSPORT_MAGIC,
    TRANSPORT_PROTOCOL_VERSION,
};

/// Maximum concurrent in-flight fragmented messages retained by a UDP receiver.
const MAX_INFLIGHT_FRAG_MESSAGES: usize = 16;

/// Maximum recently sent fragmented messages retained by a UDP sender for `FragNack` recovery.
pub(crate) const MAX_CACHED_FRAG_MESSAGES: usize = 16;

/// Splits an `NVD1` packet into one or more UDP datagrams guaranteed `<= MAX_UDP_DATAGRAM_BYTES`
/// (1200 bytes). Small packets take the zero-overhead `DeltaSingle` path; large packets (e.g.
/// initial keyframes) use `DeltaFrag`.
pub fn packetize_udp_datagrams(session_id: u32, nvd_packet: &[u8]) -> Vec<Vec<u8>> {
    if nvd_packet.len() <= MAX_UDP_SINGLE_PAYLOAD {
        return vec![encode_envelope(
            TransportMsgKind::DeltaSingle,
            session_id,
            nvd_packet,
        )];
    }

    let frag_msg_id = if nvd_packet.len() >= DELTA_HEADER_SIZE {
        let seq = u32::from_le_bytes(nvd_packet[8..12].try_into().unwrap());
        if seq != 0 {
            seq
        } else {
            fnv1a_64(nvd_packet) as u32
        }
    } else {
        fnv1a_64(nvd_packet) as u32
    };

    let total_len = nvd_packet.len() as u32;
    let frag_count = nvd_packet.len().div_ceil(MAX_UDP_FRAG_CHUNK);
    let frag_count_u16 = frag_count as u16;
    let mut out = Vec::with_capacity(frag_count);

    for frag_idx in 0..frag_count {
        let start = frag_idx * MAX_UDP_FRAG_CHUNK;
        let end = (start + MAX_UDP_FRAG_CHUNK).min(nvd_packet.len());
        let chunk = &nvd_packet[start..end];
        let mut dgram =
            Vec::with_capacity(TRANSPORT_HEADER_SIZE + FRAG_SUBHEADER_SIZE + chunk.len());
        dgram.extend_from_slice(&TRANSPORT_MAGIC);
        dgram.push(TRANSPORT_PROTOCOL_VERSION);
        dgram.push(TransportMsgKind::DeltaFrag as u8);
        dgram.extend_from_slice(&session_id.to_le_bytes());
        dgram.extend_from_slice(&frag_msg_id.to_le_bytes());
        dgram.extend_from_slice(&(frag_idx as u16).to_le_bytes());
        dgram.extend_from_slice(&frag_count_u16.to_le_bytes());
        dgram.extend_from_slice(&total_len.to_le_bytes());
        dgram.extend_from_slice(chunk);
        debug_assert!(dgram.len() <= MAX_UDP_DATAGRAM_BYTES);
        out.push(dgram);
    }
    out
}

#[derive(Debug)]
struct InFlightFrag {
    frag_count: u16,
    total_len: usize,
    buf: Vec<u8>,
    received: Vec<bool>,
    received_count: usize,
    highest_idx_seen: u16,
    first_seen: Instant,
    last_nack_at: Option<Instant>,
}

#[derive(Debug)]
pub(crate) struct FragReassembler {
    max_frame_bytes: usize,
    inflight: HashMap<u32, InFlightFrag>,
    order: VecDeque<u32>,
}

pub(crate) type FragNack = (u32, Vec<u16>);
pub(crate) type FragPushOutcome = (Option<Vec<u8>>, Option<FragNack>);

impl FragReassembler {
    pub(crate) fn new(max_frame_bytes: usize) -> Self {
        Self {
            max_frame_bytes,
            inflight: HashMap::default(),
            order: VecDeque::new(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.inflight.clear();
        self.order.clear();
    }

    /// Feeds one `DeltaFrag` payload. Returns `(completed_nvd_packet, optional_immediate_frag_nack)`.
    pub(crate) fn push(
        &mut self,
        payload: &[u8],
        now: Instant,
    ) -> Result<FragPushOutcome, TransportError> {
        if payload.len() < FRAG_SUBHEADER_SIZE {
            return Err(TransportError::TruncatedFrame);
        }
        let frag_msg_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
        let frag_idx = u16::from_le_bytes(payload[4..6].try_into().unwrap());
        let frag_count = u16::from_le_bytes(payload[6..8].try_into().unwrap());
        let total_len = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
        let chunk = &payload[FRAG_SUBHEADER_SIZE..];

        if frag_count == 0 || frag_idx >= frag_count {
            return Err(TransportError::MalformedFrame("invalid fragment index/count"));
        }
        if total_len == 0 || total_len > self.max_frame_bytes {
            return Err(TransportError::FrameTooLarge {
                declared: total_len,
                max: self.max_frame_bytes,
            });
        }
        let expected_count = total_len.div_ceil(MAX_UDP_FRAG_CHUNK);
        if usize::from(frag_count) != expected_count {
            return Err(TransportError::MalformedFrame(
                "fragment count mismatches total_len",
            ));
        }
        let start = usize::from(frag_idx) * MAX_UDP_FRAG_CHUNK;
        let expected_chunk_len = (total_len - start).min(MAX_UDP_FRAG_CHUNK);
        if chunk.len() != expected_chunk_len {
            return Err(TransportError::MalformedFrame("fragment chunk length mismatch"));
        }

        if !self.inflight.contains_key(&frag_msg_id) {
            while self.inflight.len() >= MAX_INFLIGHT_FRAG_MESSAGES {
                if let Some(oldest) = self.order.pop_front() {
                    self.inflight.remove(&oldest);
                } else {
                    break;
                }
            }
            self.order.push_back(frag_msg_id);
            self.inflight.insert(
                frag_msg_id,
                InFlightFrag {
                    frag_count,
                    total_len,
                    buf: vec![0u8; total_len],
                    received: vec![false; usize::from(frag_count)],
                    received_count: 0,
                    highest_idx_seen: 0,
                    first_seen: now,
                    last_nack_at: None,
                },
            );
        }

        let entry = self.inflight.get_mut(&frag_msg_id).unwrap();
        if entry.frag_count != frag_count || entry.total_len != total_len {
            return Err(TransportError::MalformedFrame(
                "conflicting fragment metadata for msg_id",
            ));
        }

        entry.buf[start..start + chunk.len()].copy_from_slice(chunk);
        let idx_u = usize::from(frag_idx);
        if !entry.received[idx_u] {
            entry.received[idx_u] = true;
            entry.received_count += 1;
        }
        if frag_idx > entry.highest_idx_seen {
            entry.highest_idx_seen = frag_idx;
        }

        if entry.received_count == usize::from(entry.frag_count) {
            let done = self.inflight.remove(&frag_msg_id).unwrap();
            self.order.retain(|&id| id != frag_msg_id);
            return Ok((Some(done.buf), None));
        }

        Ok((None, None))
    }

    pub(crate) fn poll_stale_nacks(&mut self, now: Instant, retry_ms: u64) -> Vec<(u32, Vec<u16>)> {
        let mut out = Vec::new();
        let retry = Duration::from_millis(retry_ms);
        let max_age = Duration::from_secs(5);
        let mut expired = Vec::new();

        for (&msg_id, entry) in &mut self.inflight {
            if now.saturating_duration_since(entry.first_seen) > max_age {
                expired.push(msg_id);
                continue;
            }
            let due = match entry.last_nack_at {
                Some(t) => now.saturating_duration_since(t) >= retry,
                None => now.saturating_duration_since(entry.first_seen) >= retry,
            };
            if due {
                let missing: Vec<u16> = entry
                    .received
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &got)| (!got).then_some(i as u16))
                    .collect();
                if !missing.is_empty() {
                    entry.last_nack_at = Some(now);
                    out.push((msg_id, missing));
                }
            }
        }
        for id in expired {
            self.inflight.remove(&id);
            self.order.retain(|&x| x != id);
        }
        out
    }
}
