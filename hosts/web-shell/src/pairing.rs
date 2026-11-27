//! Binary `NveWireFrame` v2 thin-client pairing state machine and cross-target
//! [`PairedWebSession`] backed by `nve-wire-core` (works identically in browser `wasm32`
//! over `WebTransport` / `WebSocket` and in native tests).

use std::collections::VecDeque;

use nve_abi::{
    encode_events, pack_key_modifiers, EventPayload, FrameId, NativeEvent, NativeEventKind,
    SurfaceId, ROOT_NODE_ID,
};
use nve_backend_web::WebBackend;
use nve_engine::{BackendError, StoreBackend, StoreWireReplica, Viewport};
use nve_store::NodeStore;
use nve_wire_core::{
    NveWireClientSession, NveWireClientState, NveWireIngestOutcome, DEFAULT_MAX_WIRE_FRAME_BYTES,
};

#[cfg(feature = "scene-view")]
use crate::scene::SceneMirror;

#[cfg(all(test, not(target_arch = "wasm32"), feature = "app-kitchen-sink"))]
#[path = "pairing_tests.rs"]
mod tests;

/// Connection lifecycle state of [`PairClientMachine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairClientState {
    Disconnected,
    Connecting,
    Connected { session_id: u32 },
    Rejected(String),
}

/// Decoded result of feeding one inbound binary `NveWireFrame` v2 message into [`PairClientMachine`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PairInboundOutcome {
    /// True when this message completed the `Hello` -> `HelloAck` handshake.
    pub handshake_completed: bool,
    /// Incoming `CommandDelta` / `SceneKeyframe` payloads ready for `WebBackend::submit_frame`.
    pub frames: Vec<(u32, Vec<u8>)>,
    /// Outbound `NveWireFrame` v2 binary control messages to transmit back (`Pong`, `KeyframeRequest`).
    pub outbound: Vec<Vec<u8>>,
    /// True when the remote server sent `Bye`.
    pub peer_closed: bool,
}

/// Thin `wasm32`-clean wrapper around [`nve_wire_core::NveWireClientSession`].
pub struct PairClientMachine {
    session: NveWireClientSession,
    raw_bytes_rx: u64,
}

impl PairClientMachine {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            session: NveWireClientSession::new(token, "nve-web-shell")
                .with_max_frame_bytes(DEFAULT_MAX_WIRE_FRAME_BYTES),
            raw_bytes_rx: 0,
        }
    }

    pub fn state(&self) -> PairClientState {
        match self.session.state() {
            NveWireClientState::Disconnected | NveWireClientState::Closed => {
                PairClientState::Disconnected
            }
            NveWireClientState::Connecting => PairClientState::Connecting,
            NveWireClientState::Connected { session_id } => PairClientState::Connected {
                session_id: *session_id,
            },
            NveWireClientState::Rejected { reason, detail } => {
                PairClientState::Rejected(format!("pairing rejected ({reason:?}): {detail}"))
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.session.is_connected()
    }

    pub fn session_id(&self) -> u32 {
        self.session.session_id().unwrap_or(0)
    }

    pub fn last_applied_seq(&self) -> u32 {
        self.session.metrics().last_sequence
    }

    pub fn frames_received(&self) -> u64 {
        self.session.metrics().frames_received
    }

    pub fn wire_bytes_rx(&self) -> u64 {
        self.session.metrics().wire_bytes_rx
    }

    pub fn raw_bytes_rx(&self) -> u64 {
        self.raw_bytes_rx
    }

    pub fn events_tx(&self) -> u64 {
        self.session.metrics().events_sent
    }

    /// Generates a fresh `NveWireFrame` v2 `Hello` frame carrying the Bearer token.
    pub fn build_hello(&mut self) -> Vec<u8> {
        self.session.build_hello().unwrap_or_default()
    }

    /// Processes one incoming binary `NveWireFrame` v2 message.
    pub fn on_binary_message(&mut self, bytes: &[u8]) -> Result<PairInboundOutcome, String> {
        let mut out = PairInboundOutcome::default();
        let outcome = self
            .session
            .ingest_frame_bytes(bytes)
            .map_err(|e| e.to_string())?;
        let seq = self.session.metrics().last_sequence;

        match outcome {
            NveWireIngestOutcome::Connected { .. } => {
                out.handshake_completed = true;
            }
            NveWireIngestOutcome::Rejected { reason, detail } => {
                return Err(format!("pairing rejected ({reason:?}): {detail}"));
            }
            NveWireIngestOutcome::CommandDelta(payload)
            | NveWireIngestOutcome::SceneKeyframe(payload) => {
                self.raw_bytes_rx = self.raw_bytes_rx.saturating_add(payload.len() as u64);
                out.frames.push((seq, payload));
            }
            NveWireIngestOutcome::NodePatchDelta(_)
            | NveWireIngestOutcome::SlotChunk(_)
            | NveWireIngestOutcome::CrdtDelta(_)
            | NveWireIngestOutcome::Presence(_)
            | NveWireIngestOutcome::HotPatch(_)
            | NveWireIngestOutcome::Ping
            | NveWireIngestOutcome::Pong => {}
            NveWireIngestOutcome::Bye => {
                out.peer_closed = true;
            }
        }

        while let Some(reply) = self.session.pop_outbound() {
            out.outbound.push(reply);
        }
        Ok(out)
    }

    /// Encodes a batch of local DOM [`NativeEvent`]s into one `NveWireKind::EventBatch` frame.
    pub fn encode_input_events(&mut self, events: &[NativeEvent]) -> Result<Vec<u8>, String> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        if !self.is_connected() {
            return Err("cannot send EventBatch before handshake completes".to_string());
        }
        let encoded_ring = encode_events(events);
        self.session
            .encode_event_batch(&encoded_ring)
            .map_err(|e| e.to_string())
    }

    /// Builds a `KeyframeRequest` frame if connected.
    pub fn build_keyframe_request(&mut self) -> Option<Vec<u8>> {
        if !self.is_connected() {
            return None;
        }
        self.session.build_keyframe_request().ok()
    }

    /// Builds a graceful `Bye` frame and transitions to disconnected.
    pub fn build_bye(&mut self) -> Option<Vec<u8>> {
        if !self.is_connected() {
            return None;
        }
        self.session.build_bye().ok()
    }
}

/// Cross-target paired thin-client web session binding [`PairClientMachine`] directly to
/// `StoreWireReplica<WebBackend>` (plus, with `scene-view`, a zero-copy [`SceneMirror`]; zero local
/// app VDOM).
pub struct PairedWebSession {
    surface_id: SurfaceId,
    client: PairClientMachine,
    replica: StoreWireReplica<WebBackend>,
    viewport: Viewport,
    outbound_queue: VecDeque<Vec<u8>>,
    #[cfg(feature = "scene-view")]
    scene: SceneMirror,
}

impl PairedWebSession {
    pub fn new(
        surface_id: SurfaceId,
        mut backend: WebBackend,
        viewport: Viewport,
        token: impl Into<String>,
    ) -> Self {
        crate::register_shell_web_systems(&mut backend);
        backend.set_viewport(surface_id, viewport);
        nve_components::tables::register_core_tables();
        nve_web_html::register_web_html_tables();
        let replica = StoreWireReplica::new(
            backend,
            nve_components::tables::core_realization_order(),
        );
        Self {
            surface_id,
            client: PairClientMachine::new(token),
            replica,
            viewport,
            outbound_queue: VecDeque::new(),
            #[cfg(feature = "scene-view")]
            scene: SceneMirror::new(),
        }
    }

    pub fn backend(&self) -> &WebBackend {
        self.replica.backend()
    }

    pub fn backend_mut(&mut self) -> &mut WebBackend {
        self.replica.backend_mut()
    }

    pub fn store(&self) -> &NodeStore {
        self.replica.store()
    }

    pub fn node_count(&self) -> usize {
        self.replica.store().live_nodes().count()
    }

    pub fn render_html(&self) -> String {
        self.replica.backend().render_html(self.replica.store())
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

    #[cfg(feature = "scene-view")]
    pub fn scene(&self) -> &SceneMirror {
        &self.scene
    }

    pub fn build_hello(&mut self) -> Vec<u8> {
        self.client.build_hello()
    }

    pub fn on_pair_binary_message(&mut self, raw: &[u8]) -> Result<Vec<u8>, String> {
        let outcome = self.client.on_binary_message(raw)?;
        for (seq, cmd_bytes) in outcome.frames {
            self.apply_remote_frame(u64::from(seq), &cmd_bytes)
                .map_err(|e| format!("WebBackend submit_nve2_frame failed for seq {seq}: {e}"))?;
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
            && let Ok(Some(layout_pkt)) = self.encode_viewport_resize(self.viewport)
        {
            self.outbound_queue.push_back(layout_pkt);
        }
        Ok(self.outbound_queue.pop_front().unwrap_or_default())
    }

    pub fn take_outbound_message(&mut self) -> Option<Vec<u8>> {
        self.outbound_queue.pop_front()
    }

    /// Drains pending browser DOM events from `WebBackend` into one `NveWireKind::EventBatch` frame.
    pub fn drain_dom_events_as_wire_frame(&mut self) -> Result<Option<Vec<u8>>, String> {
        let events = self.replica.backend_mut().drain_events();
        if events.is_empty() || !self.client.is_connected() {
            return Ok(None);
        }
        let pkt = self.client.encode_input_events(&events)?;
        Ok((!pkt.is_empty()).then_some(pkt))
    }

    pub fn encode_viewport_resize(
        &mut self,
        viewport: Viewport,
    ) -> Result<Option<Vec<u8>>, String> {
        let surface_id = self.surface_id;
        self.viewport = viewport;
        self.replica.backend_mut().set_viewport(surface_id, viewport);

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

    pub fn encode_keydown(
        &mut self,
        key: &str,
        meta: bool,
        ctrl: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        if !self.client.is_connected() || key.is_empty() {
            return Ok(None);
        }
        let surface_id = self.surface_id;
        let ev = NativeEvent {
            surface_id,
            node_id: ROOT_NODE_ID,
            timestamp_us: 0,
            kind: NativeEventKind::KeyDown,
            payload: EventPayload::Key {
                key: key.to_owned(),
                code: String::new(),
                key_code: 0,
                modifiers: pack_key_modifiers(meta, ctrl, false, false, false),
                is_composing: false,
            },
        };
        let pkt = self.client.encode_input_events(&[ev])?;
        Ok((!pkt.is_empty()).then_some(pkt))
    }

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
        let _ = self.replica.submit_nve2_frame(command_bytes)?;
        #[cfg(feature = "scene-view")]
        self.scene
            .sync(self.replica.store(), self.replica.last_changes(), frame_id.max(1));
        #[cfg(not(feature = "scene-view"))]
        let _ = frame_id;
        Ok(())
    }
}
