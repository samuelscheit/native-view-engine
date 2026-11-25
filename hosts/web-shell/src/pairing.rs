//! Zero-JSON pure-Rust `NVT1`/`NVD1` binary WebSocket pairing client state machine and
//! cross-target [`PairedWebSession`] (works identically in browser `wasm32` and native tests).

use std::collections::VecDeque;

use nve_abi::{
    encode_events, EventPayload, FrameId, NativeEvent, NativeEventKind, NodeId, NveSceneRef,
    NveSceneTable, NveSceneView, SurfaceId, ROOT_NODE_ID,
};
use nve_backend_web::{encode_keydown_payload, WebBackend};
use nve_engine::{BackendError, RenderBackend, SurfaceRunner, Viewport};
use nve_interop::{
    derive_session_id, derive_token_proof, RejectReason, SyncDeltaError, SyncDeltaSession,
    TransportMsgKind, DEFAULT_SCHEMA_VERSION, DEFAULT_SYNC_WINDOW, MAX_INPUT_EVENT_BYTES,
    TRANSPORT_HEADER_SIZE, TRANSPORT_MAGIC, TRANSPORT_PROTOCOL_VERSION,
};

use crate::session::mirror_to_scene_input;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "pairing_tests.rs"]
mod tests;

const HELLO_PAYLOAD_SIZE: usize = 32;
const HELLO_ACK_PAYLOAD_SIZE: usize = 20;

/// Connection lifecycle state of [`PairClientMachine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairClientState {
    Disconnected,
    AwaitingHelloAck { client_nonce: u64 },
    Connected { session_id: u32, server_window: usize },
    Rejected(String),
}

/// Decoded result of feeding one inbound binary `NVT1` WebSocket frame into [`PairClientMachine`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PairInboundOutcome {
    /// True when this message completed the `Hello` -> `HelloAck` handshake.
    pub handshake_completed: bool,
    /// Reconstructed in-order `NVE1` `CommandBuffer` frames ready for `WebBackend::submit_frame`.
    pub frames: Vec<(u32, Vec<u8>)>,
    /// Outbound zero-JSON `NVT1` binary messages to send back (`Ack`, `Nack`, `Pong`, `KeyframeRequest`).
    pub outbound: Vec<Vec<u8>>,
    /// True when the remote server sent `Bye`.
    pub peer_closed: bool,
}

#[inline]
pub fn encode_nvt1_envelope(kind: TransportMsgKind, session_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(TRANSPORT_HEADER_SIZE + payload.len());
    out.extend_from_slice(&TRANSPORT_MAGIC);
    out.push(TRANSPORT_PROTOCOL_VERSION);
    out.push(kind as u8);
    out.extend_from_slice(&session_id.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn parse_nvt1_envelope(bytes: &[u8]) -> Result<(TransportMsgKind, u32, &[u8]), String> {
    if bytes.len() < TRANSPORT_HEADER_SIZE {
        return Err(format!(
            "truncated NVT1 envelope ({} bytes < {TRANSPORT_HEADER_SIZE})",
            bytes.len()
        ));
    }
    if bytes[0..2] != TRANSPORT_MAGIC {
        return Err(format!(
            "invalid NVT1 magic: [{:#x}, {:#x}]",
            bytes[0], bytes[1]
        ));
    }
    let version = bytes[2];
    if version != TRANSPORT_PROTOCOL_VERSION {
        return Err(format!("unsupported NVT1 protocol version: {version}"));
    }
    let kind = TransportMsgKind::from_u8(bytes[3])
        .map_err(|_| format!("invalid NVT1 message kind: {}", bytes[3]))?;
    let session_id = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    Ok((kind, session_id, &bytes[TRANSPORT_HEADER_SIZE..]))
}

fn mix_nonce(seed: u64) -> u64 {
    let mut x = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0xA076_1D64_78BD_642F);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Pure-Rust zero-JSON `NVT1` binary pairing client + `NVD1` `SyncDeltaSession` receiver.
pub struct PairClientMachine {
    token: String,
    schema_version: u16,
    max_window: usize,
    state: PairClientState,
    delta: SyncDeltaSession,
    nonce_counter: u64,
    frames_received: u64,
    wire_bytes_rx: u64,
    raw_bytes_rx: u64,
    events_tx: u64,
}

impl PairClientMachine {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            schema_version: DEFAULT_SCHEMA_VERSION,
            max_window: DEFAULT_SYNC_WINDOW,
            state: PairClientState::Disconnected,
            delta: SyncDeltaSession::new(DEFAULT_SYNC_WINDOW),
            nonce_counter: 1,
            frames_received: 0,
            wire_bytes_rx: 0,
            raw_bytes_rx: 0,
            events_tx: 0,
        }
    }

    pub fn with_schema_version(mut self, schema_version: u16) -> Self {
        self.schema_version = schema_version;
        self
    }

    pub fn state(&self) -> &PairClientState {
        &self.state
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.state, PairClientState::Connected { .. })
    }

    pub fn session_id(&self) -> u32 {
        match self.state {
            PairClientState::Connected { session_id, .. } => session_id,
            _ => 0,
        }
    }

    pub fn last_applied_seq(&self) -> u32 {
        self.delta.last_applied_seq()
    }

    pub fn frames_received(&self) -> u64 {
        self.frames_received
    }

    pub fn wire_bytes_rx(&self) -> u64 {
        self.wire_bytes_rx
    }

    pub fn raw_bytes_rx(&self) -> u64 {
        self.raw_bytes_rx
    }

    pub fn events_tx(&self) -> u64 {
        self.events_tx
    }

    /// Generates a fresh `NVT1` `Hello` binary envelope to send upon WebSocket open.
    pub fn build_hello(&mut self) -> Vec<u8> {
        let client_last_seq = self.delta.last_applied_seq();
        let seq = self.nonce_counter;
        self.nonce_counter = self.nonce_counter.wrapping_add(1).max(1);
        let client_nonce = mix_nonce(seq ^ u64::from(client_last_seq) ^ 0x5745_4253_4F43_4B01);
        let token_proof = derive_token_proof(&self.token, self.schema_version);

        let mut payload = [0u8; HELLO_PAYLOAD_SIZE];
        payload[0..2].copy_from_slice(&self.schema_version.to_le_bytes());
        payload[2..4].copy_from_slice(&(self.max_window.clamp(4, 1024) as u16).to_le_bytes());
        payload[4..8].copy_from_slice(&client_last_seq.to_le_bytes());
        payload[8..16].copy_from_slice(&client_nonce.to_le_bytes());
        payload[16..32].copy_from_slice(&token_proof);

        self.state = PairClientState::AwaitingHelloAck { client_nonce };
        encode_nvt1_envelope(TransportMsgKind::Hello, 0, &payload)
    }

    /// Processes one incoming binary `NVT1` WebSocket frame and returns decoded `NVE1` frames plus
    /// any outbound binary `NVT1` control responses (`Ack`, `Nack`, `Pong`, `KeyframeRequest`).
    pub fn on_binary_message(&mut self, bytes: &[u8]) -> Result<PairInboundOutcome, String> {
        self.wire_bytes_rx = self.wire_bytes_rx.saturating_add(bytes.len() as u64);
        let (kind, session_id, payload) = parse_nvt1_envelope(bytes)?;
        let mut out = PairInboundOutcome::default();

        match kind {
            TransportMsgKind::HelloAck => {
                let PairClientState::AwaitingHelloAck { client_nonce } = self.state else {
                    return Err("unexpected HelloAck outside handshake".to_string());
                };
                if payload.len() != HELLO_ACK_PAYLOAD_SIZE {
                    return Err(format!(
                        "invalid HelloAck length: expected {HELLO_ACK_PAYLOAD_SIZE}, got {}",
                        payload.len()
                    ));
                }
                let server_schema = u16::from_le_bytes(payload[0..2].try_into().unwrap());
                if server_schema != self.schema_version {
                    let msg = format!(
                        "schema version mismatch: expected {}, server sent {server_schema}",
                        self.schema_version
                    );
                    self.state = PairClientState::Rejected(msg.clone());
                    return Err(msg);
                }
                let server_window = usize::from(u16::from_le_bytes(payload[2..4].try_into().unwrap())).max(4);
                let server_nonce = u64::from_le_bytes(payload[8..16].try_into().unwrap());
                let expected_sid = derive_session_id(
                    &self.token,
                    self.schema_version,
                    client_nonce,
                    server_nonce,
                );
                if session_id != expected_sid {
                    let msg = format!(
                        "session_id verification failed: expected {expected_sid:#x}, got {session_id:#x}"
                    );
                    self.state = PairClientState::Rejected(msg.clone());
                    return Err(msg);
                }
                self.delta = SyncDeltaSession::new(server_window);
                self.state = PairClientState::Connected {
                    session_id,
                    server_window,
                };
                out.handshake_completed = true;
            }
            TransportMsgKind::Reject => {
                if payload.len() < 3 {
                    return Err("truncated Reject payload".to_string());
                }
                let reason = RejectReason::from_u8(payload[0])
                    .map(|r| format!("{r:?}"))
                    .unwrap_or_else(|| format!("code {}", payload[0]));
                let expected_schema = u16::from_le_bytes(payload[1..3].try_into().unwrap());
                let msg = format!("pairing rejected: {reason} (server schema v{expected_schema})");
                self.state = PairClientState::Rejected(msg.clone());
                return Err(msg);
            }
            TransportMsgKind::DeltaSingle => {
                let expected_sid = self.require_connected_sid(session_id)?;
                match self.delta.receive_packet(payload) {
                    Ok(recv) => {
                        for (seq, frame_bytes) in recv.applied_frames {
                            self.frames_received = self.frames_received.saturating_add(1);
                            self.raw_bytes_rx =
                                self.raw_bytes_rx.saturating_add(frame_bytes.len() as u64);
                            out.frames.push((seq, frame_bytes));
                        }
                        if let Some(ack_nvd) = recv.ack_packet {
                            out.outbound.push(encode_nvt1_envelope(
                                TransportMsgKind::DeltaSingle,
                                expected_sid,
                                &ack_nvd,
                            ));
                        }
                        for nack_nvd in recv.nack_packets {
                            out.outbound.push(encode_nvt1_envelope(
                                TransportMsgKind::DeltaSingle,
                                expected_sid,
                                &nack_nvd,
                            ));
                        }
                    }
                    Err(SyncDeltaError::BaseSequenceMismatch { .. }) => {
                        out.outbound.push(self.encode_keyframe_request(expected_sid, 3));
                    }
                    Err(err) => return Err(format!("NVD1 delta decode error: {err}")),
                }
            }
            TransportMsgKind::Ping => {
                let expected_sid = self.require_connected_sid(session_id)?;
                if payload.len() != 8 {
                    return Err("invalid Ping payload length".to_string());
                }
                out.outbound.push(encode_nvt1_envelope(
                    TransportMsgKind::Pong,
                    expected_sid,
                    payload,
                ));
            }
            TransportMsgKind::Pong => {
                let _ = self.require_connected_sid(session_id)?;
            }
            TransportMsgKind::Bye => {
                let _ = self.require_connected_sid(session_id)?;
                self.state = PairClientState::Disconnected;
                out.peer_closed = true;
            }
            other => {
                return Err(format!("unsupported inbound NVT1 message kind: {other:?}"));
            }
        }

        Ok(out)
    }

    /// Encodes a batch of local DOM [`NativeEvent`]s into one zero-JSON binary `NVT1`
    /// `InputEvents = 11` frame ready to transmit over the pairing WebSocket.
    pub fn encode_input_events(&mut self, events: &[NativeEvent]) -> Result<Vec<u8>, String> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let sid = match self.state {
            PairClientState::Connected { session_id, .. } => session_id,
            _ => return Err("cannot send InputEvents before pairing handshake completes".to_string()),
        };
        let encoded_ring = encode_events(events);
        if encoded_ring.len() > MAX_INPUT_EVENT_BYTES {
            return Err(format!(
                "encoded EventRing ({} bytes) exceeds MAX_INPUT_EVENT_BYTES ({MAX_INPUT_EVENT_BYTES})",
                encoded_ring.len()
            ));
        }
        self.events_tx = self.events_tx.saturating_add(1);
        Ok(encode_nvt1_envelope(
            TransportMsgKind::InputEvents,
            sid,
            &encoded_ring,
        ))
    }

    /// Builds a `KeyframeRequest` envelope if connected.
    pub fn build_keyframe_request(&self) -> Option<Vec<u8>> {
        match self.state {
            PairClientState::Connected { session_id, .. } => {
                Some(self.encode_keyframe_request(session_id, 1))
            }
            _ => None,
        }
    }

    /// Builds a graceful `Bye` envelope and transitions to `Disconnected`.
    pub fn build_bye(&mut self) -> Option<Vec<u8>> {
        match self.state {
            PairClientState::Connected { session_id, .. } => {
                self.state = PairClientState::Disconnected;
                Some(encode_nvt1_envelope(TransportMsgKind::Bye, session_id, &[]))
            }
            _ => None,
        }
    }

    fn require_connected_sid(&self, incoming_sid: u32) -> Result<u32, String> {
        let PairClientState::Connected { session_id, .. } = self.state else {
            return Err("received post-handshake NVT1 frame before HelloAck".to_string());
        };
        if incoming_sid != session_id {
            return Err(format!(
                "NVT1 session_id mismatch: expected {session_id:#x}, got {incoming_sid:#x}"
            ));
        }
        Ok(session_id)
    }

    fn encode_keyframe_request(&self, session_id: u32, reason: u8) -> Vec<u8> {
        let mut payload = [0u8; 8];
        payload[0..4].copy_from_slice(&self.delta.last_applied_seq().to_le_bytes());
        payload[4] = reason;
        encode_nvt1_envelope(TransportMsgKind::KeyframeRequest, session_id, &payload)
    }
}

/// Cross-target paired thin-client web session binding [`PairClientMachine`] directly to
/// `SurfaceRunner<WebBackend>` and a zero-copy [`NveSceneTable`] mirror (zero local app VDOM).
pub struct PairedWebSession {
    client: PairClientMachine,
    runner: SurfaceRunner<WebBackend>,
    viewport: Viewport,
    outbound_queue: VecDeque<Vec<u8>>,
    scene_table: NveSceneTable,
    scene_view_box: Box<NveSceneView>,
    scene_sort_scratch: Vec<NodeId>,
    scene_frame_id: FrameId,
}

impl PairedWebSession {
    pub fn new(
        surface_id: SurfaceId,
        backend: WebBackend,
        viewport: Viewport,
        token: impl Into<String>,
    ) -> Self {
        let scene_table = NveSceneTable::new();
        let scene_view_box = Box::new(scene_table.as_view());
        Self {
            client: PairClientMachine::new(token),
            runner: SurfaceRunner::new(surface_id, viewport, backend),
            viewport,
            outbound_queue: VecDeque::new(),
            scene_table,
            scene_view_box,
            scene_sort_scratch: Vec::new(),
            scene_frame_id: 0,
        }
    }

    pub fn backend(&self) -> &WebBackend {
        self.runner.backend()
    }

    pub fn backend_mut(&mut self) -> &mut WebBackend {
        self.runner.backend_mut()
    }

    pub fn client(&self) -> &PairClientMachine {
        &self.client
    }

    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    pub fn session_id(&self) -> u32 {
        self.client.session_id()
    }

    pub fn frames_received(&self) -> u64 {
        self.client.frames_received()
    }

    pub fn last_applied_seq(&self) -> u32 {
        self.client.last_applied_seq()
    }

    pub fn scene_table(&self) -> &NveSceneTable {
        &self.scene_table
    }

    pub fn scene_view(&self) -> NveSceneView {
        *self.scene_view_box
    }

    pub fn scene_view_ptr(&self) -> *const NveSceneView {
        &*self.scene_view_box as *const NveSceneView
    }

    pub fn scene_ref(&self) -> NveSceneRef<'_> {
        self.scene_table.as_ref()
    }

    /// Generates the initial `NVT1` `Hello` binary message to send upon WebSocket open.
    pub fn build_hello(&mut self) -> Vec<u8> {
        self.client.build_hello()
    }

    /// Decodes one incoming `NVT1` WebSocket binary frame, applies any reconstructed `NVD1`
    /// `CommandBuffer` frames directly onto `WebBackend::submit_frame()`, and returns the next
    /// outbound `NVT1` response bytes (with any additional queued replies accessible via
    /// [`Self::take_outbound_message`]).
    pub fn on_pair_binary_message(&mut self, raw: &[u8]) -> Result<Vec<u8>, String> {
        let outcome = self.client.on_binary_message(raw)?;
        for (seq, cmd_bytes) in outcome.frames {
            self.apply_remote_frame(u64::from(seq), &cmd_bytes)
                .map_err(|e| format!("WebBackend submit_frame failed for seq {seq}: {e}"))?;
        }
        for msg in outcome.outbound {
            if !msg.is_empty() {
                self.outbound_queue.push_back(msg);
            }
        }
        if outcome.handshake_completed
            && self.viewport.width.is_finite()
            && self.viewport.height.is_finite()
            && self.viewport.width > 0.0
            && self.viewport.height > 0.0
        {
            if let Ok(Some(layout_pkt)) = self.encode_viewport_resize(self.viewport) {
                self.outbound_queue.push_back(layout_pkt);
            }
        }
        Ok(self.outbound_queue.pop_front().unwrap_or_default())
    }

    /// Pops any additional queued outbound `NVT1` binary frames (`Nack`, `Layout`, `InputEvents`).
    pub fn take_outbound_message(&mut self) -> Option<Vec<u8>> {
        self.outbound_queue.pop_front()
    }

    /// Drains pending browser DOM events from `WebBackend` and encodes them into one binary
    /// `NVT1` `InputEvents = 11` frame.
    pub fn drain_dom_events_as_nvt1(&mut self) -> Result<Option<Vec<u8>>, String> {
        let events = self.runner.backend_mut().drain_events();
        if events.is_empty() || !self.client.is_connected() {
            return Ok(None);
        }
        let pkt = self.client.encode_input_events(&events)?;
        Ok((!pkt.is_empty()).then_some(pkt))
    }

    /// Updates `WebBackend`'s viewport and returns an outbound `NVT1` `InputEvents = 11` `Layout`
    /// packet when connected.
    pub fn encode_viewport_resize(
        &mut self,
        viewport: Viewport,
    ) -> Result<Option<Vec<u8>>, String> {
        let surface_id = self.runner.surface_id();
        self.viewport = viewport;
        self.runner.backend_mut().set_viewport(surface_id, viewport);
        #[cfg(target_arch = "wasm32")]
        self.runner.backend().repaint_all_canvases();

        if !self.client.is_connected()
            || !viewport.width.is_finite()
            || !viewport.height.is_finite()
            || viewport.width <= 0.0
            || viewport.height <= 0.0
        {
            return Ok(None);
        }

        let ev = NativeEvent {
            surface_id,
            node_id: ROOT_NODE_ID,
            timestamp_us: 0,
            kind: NativeEventKind::Layout,
            payload: EventPayload::Layout {
                x: 0.0,
                y: 0.0,
                width: viewport.width,
                height: viewport.height,
            },
        };
        let pkt = self.client.encode_input_events(&[ev])?;
        Ok((!pkt.is_empty()).then_some(pkt))
    }

    /// Encodes a synthetic `keydown` (`NK1` zero-JSON binary payload) as an outbound `NVT1`
    /// `InputEvents = 11` frame.
    pub fn encode_keydown(
        &mut self,
        key: &str,
        meta: bool,
        ctrl: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        if !self.client.is_connected() || key.is_empty() {
            return Ok(None);
        }
        let surface_id = self.runner.surface_id();
        let data = encode_keydown_payload(key, meta, ctrl, false, false);
        let ev = NativeEvent {
            surface_id,
            node_id: ROOT_NODE_ID,
            timestamp_us: 0,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "keydown".into(),
                data,
            },
        };
        let pkt = self.client.encode_input_events(&[ev])?;
        Ok((!pkt.is_empty()).then_some(pkt))
    }

    /// Builds a graceful `Bye` frame if connected.
    pub fn build_bye(&mut self) -> Option<Vec<u8>> {
        self.client.build_bye()
    }

    fn apply_remote_frame(
        &mut self,
        frame_id: FrameId,
        command_bytes: &[u8],
    ) -> Result<(), BackendError> {
        if command_bytes.is_empty() {
            return Ok(());
        }
        let _ = self
            .runner
            .submit_recorded_frame(frame_id.max(1), 0, 0, command_bytes)?;
        self.sync_scene_table(frame_id.max(1));
        Ok(())
    }

    fn sync_scene_table(&mut self, frame_id: FrameId) {
        self.scene_frame_id = frame_id;
        let backend = self.runner.backend();
        self.scene_sort_scratch.clear();
        self.scene_sort_scratch
            .extend(backend.iter_nodes().map(|(id, _)| id));
        self.scene_sort_scratch.sort_unstable();

        let sort_ids = &self.scene_sort_scratch;
        let _ = self.scene_table.sync_frame(
            frame_id,
            sort_ids.iter().filter_map(|&node_id| {
                let el = backend.node(node_id)?;
                Some(mirror_to_scene_input(backend, node_id, el))
            }),
        );
        *self.scene_view_box = self.scene_table.as_view();
    }
}
