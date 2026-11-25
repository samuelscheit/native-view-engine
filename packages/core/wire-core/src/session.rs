//! Sans-I/O `NveWireFrame` v2 client state machine shared by `hosts/web-shell` (WASM),
//! `nve-wire` C ABI, and protocol tests.

use std::collections::VecDeque;

use crate::frame::{
    NveWireChannel, NveWireError, NveWireFrame, NveWireFrameRef, NveWireKind,
    DEFAULT_MAX_WIRE_FRAME_BYTES,
};
use crate::handshake::{
    NveWireHello, NveWireHelloAck, NveWireRejectReason, DEFAULT_WIRE_SCHEMA_VERSION,
};

/// Lifecycle state of a sans-I/O `NveWireClientSession`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NveWireClientState {
    Disconnected,
    Connecting,
    Connected { session_id: u32 },
    Rejected { reason: NveWireRejectReason, detail: String },
    Closed,
}

/// Telemetry counters tracked by [`NveWireClientSession`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NveWireMetrics {
    pub session_id: u32,
    pub frames_received: u64,
    pub keyframes_received: u64,
    pub command_deltas_received: u64,
    pub node_patch_deltas_received: u64,
    pub slot_chunks_received: u64,
    pub crdt_deltas_received: u64,
    pub presence_received: u64,
    pub events_sent: u64,
    pub wire_bytes_rx: u64,
    pub wire_bytes_tx: u64,
    pub last_sequence: u32,
}

/// Decoded event returned from [`NveWireClientSession::ingest_frame_bytes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NveWireIngestOutcome {
    Connected { session_id: u32, server_capabilities: u32 },
    Rejected { reason: NveWireRejectReason, detail: String },
    SceneKeyframe(Vec<u8>),
    CommandDelta(Vec<u8>),
    NodePatchDelta(Vec<u8>),
    SlotChunk(Vec<u8>),
    CrdtDelta(Vec<u8>),
    Presence(Vec<u8>),
    HotPatch(Vec<u8>),
    Ping,
    Pong,
    Bye,
}

/// Sans-I/O client state machine for `NveWireFrame` v2 sessions.
#[derive(Debug)]
pub struct NveWireClientSession {
    bearer_token: String,
    client_name: String,
    schema_version: u16,
    max_frame_bytes: usize,
    state: NveWireClientState,
    metrics: NveWireMetrics,
    next_sequence: u32,
    outbound: VecDeque<Vec<u8>>,
    latest_frame_payload: Vec<u8>,
}

impl NveWireClientSession {
    pub fn new(bearer_token: impl Into<String>, client_name: impl Into<String>) -> Self {
        Self {
            bearer_token: bearer_token.into(),
            client_name: client_name.into(),
            schema_version: DEFAULT_WIRE_SCHEMA_VERSION,
            max_frame_bytes: DEFAULT_MAX_WIRE_FRAME_BYTES,
            state: NveWireClientState::Disconnected,
            metrics: NveWireMetrics::default(),
            next_sequence: 1,
            outbound: VecDeque::new(),
            latest_frame_payload: Vec::new(),
        }
    }

    pub fn with_max_frame_bytes(mut self, max_frame_bytes: usize) -> Self {
        self.max_frame_bytes = max_frame_bytes;
        self
    }

    #[inline]
    pub fn state(&self) -> &NveWireClientState {
        &self.state
    }

    #[inline]
    pub fn is_connected(&self) -> bool {
        matches!(self.state, NveWireClientState::Connected { .. })
    }

    #[inline]
    pub fn session_id(&self) -> Option<u32> {
        match self.state {
            NveWireClientState::Connected { session_id } => Some(session_id),
            _ => None,
        }
    }

    #[inline]
    pub fn metrics(&self) -> NveWireMetrics {
        self.metrics
    }

    #[inline]
    pub fn latest_frame_payload(&self) -> &[u8] {
        &self.latest_frame_payload
    }

    #[inline]
    pub fn pop_outbound(&mut self) -> Option<Vec<u8>> {
        self.outbound.pop_front()
    }

    fn alloc_seq(&mut self) -> u32 {
        let seq = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        seq
    }

    /// Builds and enqueues the initial Bearer-token `Hello` frame.
    pub fn build_hello(&mut self) -> Result<Vec<u8>, NveWireError> {
        self.state = NveWireClientState::Connecting;
        let mut hello = NveWireHello::new(self.bearer_token.clone(), self.client_name.clone());
        hello.schema_version = self.schema_version;
        let payload = hello.encode()?;
        let seq = self.alloc_seq();
        let bytes = NveWireFrame::new(NveWireKind::Hello, seq, payload).encode()?;
        self.metrics.wire_bytes_tx += bytes.len() as u64;
        Ok(bytes)
    }

    /// Encodes one upstream `EventBatch` (`EventRing` bytes) frame.
    pub fn encode_event_batch(&mut self, event_ring_bytes: &[u8]) -> Result<Vec<u8>, NveWireError> {
        let seq = self.alloc_seq();
        let bytes = NveWireFrame::new(NveWireKind::EventBatch, seq, event_ring_bytes.to_vec())
            .with_channel(NveWireChannel::InputEvents)
            .encode()?;
        self.metrics.events_sent += 1;
        self.metrics.wire_bytes_tx += bytes.len() as u64;
        Ok(bytes)
    }

    /// Encodes a `KeyframeRequest` control frame.
    pub fn build_keyframe_request(&mut self) -> Result<Vec<u8>, NveWireError> {
        let seq = self.alloc_seq();
        let bytes = NveWireFrame::new(NveWireKind::KeyframeRequest, seq, Vec::new()).encode()?;
        self.metrics.wire_bytes_tx += bytes.len() as u64;
        Ok(bytes)
    }

    /// Encodes a graceful `Bye` control frame and transitions to `Closed`.
    pub fn build_bye(&mut self) -> Result<Vec<u8>, NveWireError> {
        self.state = NveWireClientState::Closed;
        let seq = self.alloc_seq();
        let bytes = NveWireFrame::new(NveWireKind::Bye, seq, Vec::new()).encode()?;
        self.metrics.wire_bytes_tx += bytes.len() as u64;
        Ok(bytes)
    }

    /// Processes one incoming `NveWireFrame` v2 binary message.
    pub fn ingest_frame_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<NveWireIngestOutcome, NveWireError> {
        let frame = NveWireFrameRef::decode(bytes, self.max_frame_bytes)?;
        self.metrics.wire_bytes_rx += bytes.len() as u64;
        self.metrics.last_sequence = frame.header.sequence;

        match frame.header.kind {
            NveWireKind::HelloAck => {
                let ack = NveWireHelloAck::decode(frame.payload)?;
                self.state = NveWireClientState::Connected {
                    session_id: ack.session_id,
                };
                self.metrics.session_id = ack.session_id;
                Ok(NveWireIngestOutcome::Connected {
                    session_id: ack.session_id,
                    server_capabilities: ack.server_capabilities,
                })
            }
            NveWireKind::Reject => {
                let (reason, detail) = NveWireRejectReason::decode(frame.payload)?;
                self.state = NveWireClientState::Rejected {
                    reason,
                    detail: detail.clone(),
                };
                Ok(NveWireIngestOutcome::Rejected { reason, detail })
            }
            NveWireKind::SceneKeyframe => {
                self.metrics.frames_received += 1;
                self.metrics.keyframes_received += 1;
                self.latest_frame_payload = frame.payload.to_vec();
                Ok(NveWireIngestOutcome::SceneKeyframe(
                    self.latest_frame_payload.clone(),
                ))
            }
            NveWireKind::CommandDelta => {
                self.metrics.frames_received += 1;
                self.metrics.command_deltas_received += 1;
                self.latest_frame_payload = frame.payload.to_vec();
                Ok(NveWireIngestOutcome::CommandDelta(
                    self.latest_frame_payload.clone(),
                ))
            }
            NveWireKind::NodePatchDelta => {
                self.metrics.frames_received += 1;
                self.metrics.node_patch_deltas_received += 1;
                Ok(NveWireIngestOutcome::NodePatchDelta(frame.payload.to_vec()))
            }
            NveWireKind::SlotChunk => {
                self.metrics.slot_chunks_received += 1;
                Ok(NveWireIngestOutcome::SlotChunk(frame.payload.to_vec()))
            }
            NveWireKind::CrdtDelta => {
                self.metrics.crdt_deltas_received += 1;
                Ok(NveWireIngestOutcome::CrdtDelta(frame.payload.to_vec()))
            }
            NveWireKind::Presence => {
                self.metrics.presence_received += 1;
                Ok(NveWireIngestOutcome::Presence(frame.payload.to_vec()))
            }
            NveWireKind::HotPatch => Ok(NveWireIngestOutcome::HotPatch(frame.payload.to_vec())),
            NveWireKind::Ping => {
                let seq = self.alloc_seq();
                let pong =
                    NveWireFrame::new(NveWireKind::Pong, seq, frame.payload.to_vec()).encode()?;
                self.metrics.wire_bytes_tx += pong.len() as u64;
                self.outbound.push_back(pong);
                Ok(NveWireIngestOutcome::Ping)
            }
            NveWireKind::Pong => Ok(NveWireIngestOutcome::Pong),
            NveWireKind::Bye => {
                self.state = NveWireClientState::Closed;
                Ok(NveWireIngestOutcome::Bye)
            }
            NveWireKind::Hello | NveWireKind::EventBatch | NveWireKind::KeyframeRequest => {
                Err(NveWireError::InvalidPayload(
                    "client received server-bound frame kind",
                ))
            }
        }
    }
}
