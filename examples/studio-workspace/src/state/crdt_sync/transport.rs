//! Binary multi-peer CRDT transport layer (`NVC1` deltas + `NVCD` vector-clock digests)
//! over `nve_interop::sync_transport` and `nve_interop::NveChannelBus`.
//!
//! Provides:
//! - [`CrdtPeerTransport`]: transport abstraction for peer-to-peer binary CRDT frame delivery.
//! - [`CrdtPeerNode`]: pairs a [`StudioCrdtDoc`] with any [`CrdtPeerTransport`] for automatic
//!   incremental delta publishing, idempotent merge, corrupt-frame rejection, and anti-entropy
//!   vector-clock catch-up on reconnect.
//! - [`InMemoryMeshTransport`]: deterministic multi-peer mesh (native + `wasm32`) with packet
//!   drop, reverse-order delivery, network partition/heal, and zero-copy `NveChannelBus` mirror.
//! - [`LoopbackTcpCrdtTransport`] (`#[cfg(not(target_arch = "wasm32"))]`): real OS loopback TCP
//!   (`127.0.0.1:0`) transport over `nve_interop::sync_transport::{TcpDeltaServer, TcpDeltaClient}`.

use std::collections::BTreeMap;
use std::fmt;

use nve_interop::crdt::{
    CrdtWireError, MAX_VV_ENTRIES, MAX_WIRE_BYTES, NVE_CRDT_DELTA_MAGIC, NveCrdtDelta, PeerId,
    VersionVector, crdt_xxh64,
};

use super::StudioCrdtDoc;

pub use super::mesh_transport::{InMemoryMeshHub, InMemoryMeshTransport, MeshFaultConfig};
#[cfg(not(target_arch = "wasm32"))]
pub use super::tcp_transport::{LoopbackTcpCrdtCluster, LoopbackTcpCrdtTransport};

/// 8-byte binary magic identifying an anti-entropy vector-clock digest frame (`NVE_DGST`).
pub const CRDT_DIGEST_MAGIC: [u8; 8] = *b"NVE_DGST";

/// Current wire version for vector-clock digest frames (`2`, `XXH64`-verified).
pub const CRDT_DIGEST_VERSION: u16 = 2;

/// Minimum encoded byte size of a digest frame:
/// `magic(8) + ver(2) + sender(8) + vv_len(4) + checksum(8)`.
pub const CRDT_DIGEST_MIN_LEN: usize = 30;

/// `NveChannelBus` binary topic mirrored by [`InMemoryMeshHub`].
pub const TOPIC_CRDT_NVC1: &str = "crdt.delta";

/// Typed error returned by binary CRDT peer transports.
#[derive(Debug)]
pub enum CrdtTransportError {
    Wire(CrdtWireError),
    Transport(String),
    UnknownPeer(PeerId),
    Disconnected,
    Timeout,
    Io(String),
}

impl fmt::Display for CrdtTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(err) => write!(f, "CRDT wire error: {err}"),
            Self::Transport(err) => write!(f, "wire transport error: {err}"),
            Self::UnknownPeer(p) => write!(f, "unknown CRDT peer: {}", p.0),
            Self::Disconnected => f.write_str("CRDT transport peer disconnected"),
            Self::Timeout => f.write_str("CRDT sync convergence timed out"),
            Self::Io(msg) => write!(f, "CRDT transport I/O error: {msg}"),
        }
    }
}

impl std::error::Error for CrdtTransportError {}

impl From<CrdtWireError> for CrdtTransportError {
    fn from(value: CrdtWireError) -> Self {
        Self::Wire(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<nve_wire::NveWireTransportError> for CrdtTransportError {
    fn from(value: nve_wire::NveWireTransportError) -> Self {
        Self::Transport(value.to_string())
    }
}

/// Live per-peer binary CRDT transport metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrdtSyncMetrics {
    pub connected_peers: u16,
    pub deltas_sent: u64,
    pub deltas_received: u64,
    pub records_applied: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub corrupt_frames_rejected: u64,
    pub anti_entropy_syncs: u64,
    pub partitioned: bool,
}

/// Summary produced by one [`CrdtPeerNode::pump_incoming`] pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrdtPumpReport {
    pub frames_processed: usize,
    pub deltas_merged: usize,
    pub records_applied: usize,
    pub digests_handled: usize,
    pub anti_entropy_deltas_sent: usize,
    pub corrupt_frames_rejected: usize,
    pub bytes_received: u64,
    pub bytes_sent: u64,
}

/// Binary frame exchanged between CRDT peers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrdtWireFrame {
    /// Raw `NVC1` binary delta packet (zero wrapper overhead).
    Delta(Vec<u8>),
    /// Compact checksummed `NVCD` vector-clock digest requesting anti-entropy catch-up.
    VectorClockDigest {
        sender: PeerId,
        version_vector: VersionVector,
    },
}

impl CrdtWireFrame {
    /// Encodes a compact binary `NVCD` anti-entropy vector-clock digest with trailing XXH64.
    pub fn encode_digest(sender: PeerId, vv: &VersionVector) -> Result<Vec<u8>, CrdtWireError> {
        if vv.len() > MAX_VV_ENTRIES {
            return Err(CrdtWireError::LimitExceeded {
                kind: "version_vector",
                count: vv.len(),
                max: MAX_VV_ENTRIES,
            });
        }
        let mut out = Vec::with_capacity(CRDT_DIGEST_MIN_LEN + vv.len() * 16);
        out.extend_from_slice(&CRDT_DIGEST_MAGIC);
        out.extend_from_slice(&CRDT_DIGEST_VERSION.to_le_bytes());
        out.extend_from_slice(&sender.as_u64().to_le_bytes());
        out.extend_from_slice(&(vv.len() as u32).to_le_bytes());
        for (peer, seq) in vv.iter() {
            out.extend_from_slice(&peer.as_u64().to_le_bytes());
            out.extend_from_slice(&seq.to_le_bytes());
        }
        let checksum = crdt_xxh64(&out);
        out.extend_from_slice(&checksum.to_le_bytes());
        Ok(out)
    }

    /// Decodes and `XXH64`-verifies a vector-clock digest frame.
    pub fn decode_digest(bytes: &[u8]) -> Result<(PeerId, VersionVector), CrdtWireError> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(CrdtWireError::LimitExceeded {
                kind: "wire_bytes",
                count: bytes.len(),
                max: MAX_WIRE_BYTES,
            });
        }
        if bytes.len() < CRDT_DIGEST_MIN_LEN {
            return Err(CrdtWireError::Truncated {
                expected: CRDT_DIGEST_MIN_LEN,
                actual: bytes.len(),
            });
        }
        let magic: [u8; 8] = bytes[0..8].try_into().expect("checked min len");
        if magic != CRDT_DIGEST_MAGIC {
            return Err(CrdtWireError::InvalidMagic(magic));
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != CRDT_DIGEST_VERSION {
            return Err(CrdtWireError::UnsupportedVersion(version));
        }
        let payload_end = bytes.len() - 8;
        let expected = u64::from_le_bytes(bytes[payload_end..].try_into().expect("8B checksum"));
        let actual = crdt_xxh64(&bytes[..payload_end]);
        if actual != expected {
            return Err(CrdtWireError::ChecksumMismatch { expected, actual });
        }

        let le64 = |o: usize| {
            u64::from_le_bytes([
                bytes[o],
                bytes[o + 1],
                bytes[o + 2],
                bytes[o + 3],
                bytes[o + 4],
                bytes[o + 5],
                bytes[o + 6],
                bytes[o + 7],
            ])
        };
        let sender = PeerId(le64(10));
        let vv_len = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]) as usize;
        if vv_len > MAX_VV_ENTRIES {
            return Err(CrdtWireError::LimitExceeded {
                kind: "version_vector",
                count: vv_len,
                max: MAX_VV_ENTRIES,
            });
        }
        let expected_len = CRDT_DIGEST_MIN_LEN.saturating_add(vv_len.saturating_mul(16));
        if bytes.len() < expected_len {
            return Err(CrdtWireError::Truncated {
                expected: expected_len,
                actual: bytes.len(),
            });
        }
        if bytes.len() > expected_len {
            return Err(CrdtWireError::TrailingBytes(bytes.len() - expected_len));
        }
        let mut vv = VersionVector::new();
        let mut off = 22usize;
        for _ in 0..vv_len {
            let p = PeerId(le64(off));
            let s = le64(off + 8);
            vv.set_max(p, s);
            off += 16;
        }
        Ok((sender, vv))
    }

    /// Classifies a raw binary wire frame as either a vector-clock digest or an `NveCrdtDelta`.
    pub fn parse(bytes: &[u8]) -> Result<Self, CrdtWireError> {
        if bytes.len() < 8 {
            return Err(CrdtWireError::Truncated {
                expected: 8,
                actual: bytes.len(),
            });
        }
        let magic: [u8; 8] = bytes[0..8].try_into().expect("checked 8B magic");
        if magic == CRDT_DIGEST_MAGIC {
            let (sender, version_vector) = Self::decode_digest(bytes)?;
            Ok(Self::VectorClockDigest {
                sender,
                version_vector,
            })
        } else if magic == NVE_CRDT_DELTA_MAGIC {
            Ok(Self::Delta(bytes.to_vec()))
        } else {
            Err(CrdtWireError::InvalidMagic(magic))
        }
    }
}

/// Returns `true` when `local` contains any operation not yet observed by `remote`.
pub fn version_vector_has_unseen_ops(local: &VersionVector, remote: &VersionVector) -> bool {
    local.iter().any(|(peer, seq)| seq > remote.get(peer))
}

/// Computes the component-wise minimum `VersionVector` across `peers` so one broadcast `NVC1`
/// delta covers every peer's unseen operations.
fn common_min_vv<'a>(
    known_peers: impl Iterator<Item = &'a VersionVector>,
    local_vv: &VersionVector,
) -> VersionVector {
    let list: Vec<&VersionVector> = known_peers.collect();
    if list.is_empty() {
        return VersionVector::new();
    }
    let mut out = VersionVector::new();
    for (peer, _) in local_vv.iter() {
        let min_seq = list.iter().map(|vv| vv.get(peer)).min().unwrap_or(0);
        if min_seq > 0 {
            out.set_max(peer, min_seq);
        }
    }
    out
}

/// Pluggable binary peer transport carrying `NVC1` deltas and `NVCD` vector-clock digests.
pub trait CrdtPeerTransport {
    fn local_peer(&self) -> PeerId;
    /// Sends a binary wire frame to `target` (`None` = broadcast to all reachable peers).
    /// Returns the number of peer deliveries attempted.
    fn send_frame(
        &mut self,
        target: Option<PeerId>,
        frame: Vec<u8>,
    ) -> Result<usize, CrdtTransportError>;
    /// Drains all currently queued inbound `(sender, raw_frame_bytes)` messages.
    fn poll_incoming(&mut self) -> Vec<(PeerId, Vec<u8>)>;
    fn connected_peer_count(&self) -> usize;
    fn is_partitioned(&self) -> bool;
    fn set_partitioned(&mut self, partitioned: bool);
    /// Returns the list of currently known remote peers, if tracked by the transport.
    fn known_remote_peers(&self) -> Vec<PeerId>;
}

/// Connects a [`StudioCrdtDoc`] to any [`CrdtPeerTransport`], handling incremental delta broadcast,
/// vector-clock anti-entropy catch-up on reconnect/heal, and atomic corrupt-frame rejection.
pub struct CrdtPeerNode<T: CrdtPeerTransport> {
    doc: StudioCrdtDoc,
    transport: T,
    peer_known_vv: BTreeMap<PeerId, VersionVector>,
    metrics: CrdtSyncMetrics,
}

impl<T: CrdtPeerTransport> CrdtPeerNode<T> {
    pub fn new(doc: StudioCrdtDoc, transport: T) -> Self {
        let metrics = CrdtSyncMetrics {
            connected_peers: transport.connected_peer_count() as u16,
            partitioned: transport.is_partitioned(),
            ..CrdtSyncMetrics::default()
        };
        Self {
            doc,
            transport,
            peer_known_vv: BTreeMap::new(),
            metrics,
        }
    }

    pub fn doc(&self) -> &StudioCrdtDoc {
        &self.doc
    }

    pub fn doc_mut(&mut self) -> &mut StudioCrdtDoc {
        &mut self.doc
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn peer_id(&self) -> PeerId {
        self.doc.peer_id()
    }

    pub fn metrics(&self) -> CrdtSyncMetrics {
        let mut m = self.metrics;
        m.connected_peers = self.transport.connected_peer_count() as u16;
        m.partitioned = self.transport.is_partitioned();
        m
    }

    /// Broadcasts an incremental `NVC1` binary delta covering all operations not yet confirmed
    /// on all connected peers. Returns `(deliveries, wire_bytes)`.
    pub fn broadcast_delta(&mut self) -> Result<(usize, usize), CrdtTransportError> {
        let remotes = self.transport.known_remote_peers();
        let since = if remotes.is_empty()
            || remotes.iter().any(|p| !self.peer_known_vv.contains_key(p))
        {
            VersionVector::new()
        } else {
            common_min_vv(
                remotes.iter().filter_map(|p| self.peer_known_vv.get(p)),
                self.doc.version_vector(),
            )
        };
        let delta = self.doc.delta_since(&since);
        let len = delta.len();
        let delivered = self.transport.send_frame(None, delta)?;
        if delivered > 0 {
            self.metrics.deltas_sent += 1;
            self.metrics.bytes_sent += len as u64;
        }
        Ok((delivered, len))
    }

    /// Broadcasts an `NVCD` binary vector-clock digest so peers automatically respond with any
    /// operations missing from our local `VersionVector` (anti-entropy catch-up).
    pub fn broadcast_digest(&mut self) -> Result<(usize, usize), CrdtTransportError> {
        let digest = CrdtWireFrame::encode_digest(self.doc.peer_id(), self.doc.version_vector())?;
        let len = digest.len();
        let delivered = self.transport.send_frame(None, digest)?;
        if delivered > 0 {
            self.metrics.anti_entropy_syncs += 1;
            self.metrics.bytes_sent += len as u64;
        }
        Ok((delivered, len))
    }

    /// Drains all inbound frames, atomically rejecting corrupt/truncated frames without mutating
    /// [`StudioCrdtDoc`], merging valid `NVC1` deltas, and replying to `NVCD` digests with minimal
    /// `NVC1` catch-up deltas.
    pub fn pump_incoming(&mut self) -> CrdtPumpReport {
        let incoming = self.transport.poll_incoming();
        let mut report = CrdtPumpReport::default();

        for (from_peer, raw_bytes) in incoming {
            report.frames_processed += 1;
            let raw_len = raw_bytes.len() as u64;

            match CrdtWireFrame::parse(&raw_bytes) {
                Ok(CrdtWireFrame::Delta(delta_bytes)) => {
                    let pkt_vv = NveCrdtDelta::decode(&delta_bytes)
                        .ok()
                        .map(|p| (p.sender, p.version_vector));
                    match self.doc.merge_delta(&delta_bytes) {
                        Ok(summary) => {
                            report.deltas_merged += 1;
                            report.records_applied += summary.records_applied;
                            report.bytes_received += raw_len;
                            self.metrics.deltas_received += 1;
                            self.metrics.records_applied += summary.records_applied as u64;
                            self.metrics.bytes_received += raw_len;
                            if let Some((sender, vv)) = pkt_vv {
                                self.peer_known_vv
                                    .entry(sender)
                                    .or_default()
                                    .merge(&vv);
                            }
                        }
                        Err(_) => {
                            report.corrupt_frames_rejected += 1;
                            self.metrics.corrupt_frames_rejected += 1;
                        }
                    }
                }
                Ok(CrdtWireFrame::VectorClockDigest {
                    sender,
                    version_vector,
                }) => {
                    report.digests_handled += 1;
                    report.bytes_received += raw_len;
                    self.metrics.bytes_received += raw_len;
                    let peer_key = if sender.0 != 0 { sender } else { from_peer };
                    self.peer_known_vv
                        .entry(peer_key)
                        .or_default()
                        .merge(&version_vector);

                    if version_vector_has_unseen_ops(self.doc.version_vector(), &version_vector) {
                        let catchup = self.doc.delta_since(&version_vector);
                        let clen = catchup.len();
                        if let Ok(delivered) =
                            self.transport.send_frame(Some(peer_key), catchup)
                            && delivered > 0
                        {
                            report.anti_entropy_deltas_sent += 1;
                            report.bytes_sent += clen as u64;
                            self.metrics.deltas_sent += 1;
                            self.metrics.anti_entropy_syncs += 1;
                            self.metrics.bytes_sent += clen as u64;
                        }
                    }
                }
                Err(_) => {
                    report.corrupt_frames_rejected += 1;
                    self.metrics.corrupt_frames_rejected += 1;
                }
            }
        }
        report
    }
}
