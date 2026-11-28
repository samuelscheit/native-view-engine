//! Embedded `nve-wire` thin-client pairing session behind default `feature = "wire"`.
//!
//! Runs on ONE dedicated `current_thread` Tokio runtime thread inside [`nve_wire::ReconnectingWireClient`]
//! and forwards inbound frames/state transitions directly into the surface's [`WakeBridge`].

/// Inbound pairing event delivered in order from the embedded `nve-wire` client.
#[cfg_attr(not(all(feature = "wire", not(target_arch = "wasm32"))), allow(dead_code))]
pub(crate) enum PairInboundEvent {
    SessionStarted { session_id: u64, reconnected: bool },
    Keyframe { session_id: u64, payload: Vec<u8> },
    Delta { session_id: u64, payload: Vec<u8> },
    Overflowed { session_id: u64 },
}

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
pub(crate) const WIRE_PAIRING_SUPPORTED: bool = true;

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
pub(crate) const WIRE_PAIRING_SUPPORTED: bool = false;

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
mod enabled {
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use nve_abi::{EventBatchBuf, NveRuntimeMetrics, ValidatedEvents};
    use nve_store::wire::WireDecoder;
    use nve_wire::{
        resolve_pair_uri, NveWireClientConfig, NveWireKind, NveWireMetrics, NveWireTransportError,
        NveWireTransportKind, NveWireUri, PairConnectConfig, ReconnectConfig,
        ReconnectingWireClient, WireInboundItem, WireResyncState, WireWakeCallback,
    };

    use super::PairInboundEvent;
    use crate::builder::RuntimeError;
    use crate::wake::WakeBridge;

    const MAX_PENDING_REMOTE_FRAMES: usize = 256;

    pub(crate) struct ValidatedPairUri {
        uri: NveWireUri,
    }

    pub(crate) struct PairSession {
        client: ReconnectingWireClient,
        wake_alive: Arc<AtomicBool>,
        last_rx: u64,
        last_tx: u64,
        resync: WireResyncState,
    }

    impl PairSession {
        /// Synchronously validates the URI syntax, host/port presence, TLS certificate pinning,
        /// and Bearer token BEFORE touching live surface state or spawning any worker thread.
        pub(crate) fn validate_and_parse(
            uri: &str,
            token: &str,
        ) -> Result<ValidatedPairUri, RuntimeError> {
            let parsed = resolve_pair_uri(uri, token, None, None, None, true).map_err(|err| {
                match err {
                    NveWireTransportError::Auth(_) => {
                        RuntimeError::InvalidArg("missing pairing token")
                    }
                    NveWireTransportError::Tls(_) => RuntimeError::InvalidArg(
                        "TLS-pinned pairing transport requires cert_sha256",
                    ),
                    _ => RuntimeError::InvalidArg("invalid pairing URI"),
                }
            })?;
            Ok(ValidatedPairUri { uri: parsed })
        }

        pub(crate) fn spawn_parsed(
            validated: ValidatedPairUri,
            wake: Arc<WakeBridge>,
            connected_flag: Arc<AtomicBool>,
        ) -> Result<Self, RuntimeError> {
            let mut wire_cfg = NveWireClientConfig::new(validated.uri.token.clone(), "nve-runtime");
            wire_cfg.cert_sha256 = validated.uri.cert_sha256;

            let wake_alive = Arc::new(AtomicBool::new(true));
            let wake_gate = Arc::clone(&wake_alive);
            let client = ReconnectingWireClient::spawn_with_connected_flag(
                validated.uri,
                wire_cfg,
                ReconnectConfig::default(),
                Some(Arc::new(move || {
                    if wake_gate.load(std::sync::atomic::Ordering::Acquire) {
                        wake.trigger();
                    }
                })),
                connected_flag,
            )
            .map_err(|e| RuntimeError::Io(e.to_string()))?;

            Ok(Self {
                client,
                wake_alive,
                last_rx: 0,
                last_tx: 0,
                resync: WireResyncState::new(),
            })
        }

        /// Immediately suppresses future wakes from this pairing worker before shutdown joins it.
        #[inline]
        pub(crate) fn disable_wake(&self) {
            self.wake_alive.store(false, std::sync::atomic::Ordering::Release);
        }

        #[inline]
        pub(crate) fn has_pending_frame(&self) -> bool {
            self.client.has_pending_item()
        }

        #[inline]
        pub(crate) fn on_session_started(
            &mut self,
            session_id: u64,
            reconnected: bool,
            has_prior_state: bool,
        ) {
            self.resync
                .on_session_started(session_id, reconnected, has_prior_state);
        }

        #[inline]
        pub(crate) fn on_overflowed(&mut self, session_id: u64) {
            let client = &self.client;
            self.resync
                .on_overflowed(session_id, || client.request_keyframe().is_ok());
        }

        #[inline]
        pub(crate) fn on_decode_error(&mut self, session_id: u64) {
            let client = &self.client;
            self.resync
                .on_decode_error(session_id, || client.request_keyframe().is_ok());
        }

        #[inline]
        pub(crate) fn should_apply(&self, is_keyframe: bool, is_frame1: bool) -> bool {
            self.resync.should_apply(is_keyframe, is_frame1)
        }

        #[inline]
        pub(crate) fn mark_keyframe_applied(&mut self) {
            self.resync.mark_keyframe_applied();
        }

        pub(crate) fn try_recv_event(&self) -> Option<PairInboundEvent> {
            while let Some(item) = self.client.try_recv_item() {
                match item {
                    WireInboundItem::SessionStarted {
                        session_id,
                        reconnected,
                    } => {
                        return Some(PairInboundEvent::SessionStarted {
                            session_id,
                            reconnected,
                        });
                    }
                    WireInboundItem::Overflowed { session_id } => {
                        return Some(PairInboundEvent::Overflowed { session_id });
                    }
                    WireInboundItem::Frame { session_id, frame } => match frame.kind {
                        NveWireKind::SceneKeyframe => {
                            return Some(PairInboundEvent::Keyframe {
                                session_id,
                                payload: frame.payload,
                            });
                        }
                        NveWireKind::CommandDelta => {
                            return Some(PairInboundEvent::Delta {
                                session_id,
                                payload: frame.payload,
                            });
                        }
                        _ => continue,
                    },
                }
            }
            None
        }

        pub(crate) fn send_events(&self, events: ValidatedEvents<'_>) -> bool {
            if events.is_empty() || !self.client.is_connected() {
                return false;
            }
            let mut batch = EventBatchBuf::with_capacity(events.len(), events.bytes().len());
            batch.extend_from_validated(events);
            self.client.send_events(batch.to_wire_bytes()).is_ok()
        }

        pub(crate) fn sync_metrics(&mut self, dst: &mut NveRuntimeMetrics) {
            let m = self.client.metrics();
            let drx = m.wire_bytes_rx.saturating_sub(self.last_rx);
            let dtx = m.wire_bytes_tx.saturating_sub(self.last_tx);
            self.last_rx = m.wire_bytes_rx;
            self.last_tx = m.wire_bytes_tx;
            dst.wire_bytes_rx = dst.wire_bytes_rx.saturating_add(drx);
            dst.wire_bytes_tx = dst.wire_bytes_tx.saturating_add(dtx);
            dst.pair_connected = u32::from(self.client.is_connected());
        }

        pub(crate) fn overlay_live_metrics(&self, dst: &mut NveRuntimeMetrics) {
            let m = self.client.metrics();
            let drx = m.wire_bytes_rx.saturating_sub(self.last_rx);
            let dtx = m.wire_bytes_tx.saturating_sub(self.last_tx);
            dst.wire_bytes_rx = dst.wire_bytes_rx.saturating_add(drx);
            dst.wire_bytes_tx = dst.wire_bytes_tx.saturating_add(dtx);
            dst.pair_connected = u32::from(self.client.is_connected());
        }

        pub(crate) fn shutdown(&mut self) {
            self.disable_wake();
            self.client.shutdown();
        }
    }

    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    struct WireSessionEpoch(u64);

    struct QueuedRemoteFrame {
        session_id: u64,
        is_keyframe: bool,
        is_frame1: bool,
        payload: Vec<u8>,
    }

    /// Shared thin-client pairing replica queue & [`WireResyncState`] driver reusable by desktop
    /// shells (e.g. `nve-skia-shell`) without duplicating wire transport or resync state machines.
    pub struct PairReplicaClient {
        client: ReconnectingWireClient,
        endpoint_label: String,
        pending_frames: VecDeque<QueuedRemoteFrame>,
        has_seen_frame: bool,
        resync: WireResyncState,
        last_dequeued_session: WireSessionEpoch,
        last_dequeued_resets: bool,
    }

    impl PairReplicaClient {
        /// Connects synchronously to a remote `nve pair` server using [`PairConnectConfig`].
        pub fn connect(config: PairConnectConfig, client_name: &str) -> Result<Self, RuntimeError> {
            let uri = config
                .resolve_uri()
                .map_err(|e| RuntimeError::Io(format!("invalid pair endpoint '{}': {e}", config.addr)))?;
            let mut wire_cfg = NveWireClientConfig::new(uri.token.clone(), client_name);
            wire_cfg.cert_sha256 = uri.cert_sha256;
            let client =
                ReconnectingWireClient::connect_sync(uri, wire_cfg, ReconnectConfig::default(), None)
                    .map_err(|e| {
                        RuntimeError::Io(format!("wire pair connect to {} failed: {e}", config.addr))
                    })?;
            Ok(Self::from_client(client, config.addr))
        }

        /// Spawns a background reconnecting client (used when initial handshake should not block).
        pub fn spawn_with_reconnect(
            config: PairConnectConfig,
            client_name: &str,
            reconnect: ReconnectConfig,
        ) -> Result<Self, RuntimeError> {
            let uri = config
                .resolve_uri()
                .map_err(|e| RuntimeError::Io(format!("invalid pair endpoint '{}': {e}", config.addr)))?;
            let mut wire_cfg = NveWireClientConfig::new(uri.token.clone(), client_name);
            wire_cfg.cert_sha256 = uri.cert_sha256;
            let client = ReconnectingWireClient::spawn(uri, wire_cfg, reconnect, None)
                .map_err(|e| RuntimeError::Io(format!("wire spawn failed: {e}")))?;
            Ok(Self::from_client(client, config.addr))
        }

        fn from_client(client: ReconnectingWireClient, endpoint_label: String) -> Self {
            Self {
                client,
                endpoint_label,
                pending_frames: VecDeque::new(),
                has_seen_frame: false,
                resync: WireResyncState::new(),
                last_dequeued_session: WireSessionEpoch(0),
                last_dequeued_resets: false,
            }
        }

        #[inline]
        pub fn auth_rejections(&self) -> u64 {
            self.client.auth_rejections()
        }

        #[inline]
        pub fn transport(&self) -> NveWireTransportKind {
            self.client.transport_kind()
        }

        #[inline]
        pub fn session_id(&self) -> u32 {
            self.client.session_id()
        }

        #[inline]
        pub fn is_connected(&self) -> bool {
            self.client.is_connected()
        }

        #[inline]
        pub fn frames_received(&self) -> u64 {
            let m = self.client.metrics();
            m.command_deltas_received.saturating_add(m.keyframes_received)
        }

        #[inline]
        pub fn last_seq(&self) -> u32 {
            self.client.metrics().last_sequence
        }

        #[inline]
        pub fn metrics(&self) -> NveWireMetrics {
            self.client.metrics()
        }

        #[inline]
        pub fn has_pending_frame(&self) -> bool {
            !self.pending_frames.is_empty() || self.client.has_pending_item()
        }

        pub fn set_wake_callback(&mut self, wake: Option<WireWakeCallback>) {
            self.client.set_wake_callback(wake);
        }

        pub fn send_events(&self, events: ValidatedEvents<'_>) -> bool {
            if events.is_empty() || !self.client.is_connected() {
                return false;
            }
            let mut buf = EventBatchBuf::with_capacity(events.len(), events.bytes().len());
            buf.extend_from_validated(events);
            self.client.send_events(buf.to_wire_bytes()).is_ok()
        }

        fn ingest_item(&mut self, item: WireInboundItem) -> Option<u32> {
            match item {
                WireInboundItem::SessionStarted {
                    session_id,
                    reconnected,
                } => {
                    if reconnected {
                        self.pending_frames.clear();
                    }
                    self.resync
                        .on_session_started(session_id, reconnected, self.has_seen_frame);
                    None
                }
                WireInboundItem::Overflowed { session_id } => {
                    self.pending_frames.clear();
                    let client = &self.client;
                    self.resync
                        .on_overflowed(session_id, || client.request_keyframe().is_ok());
                    None
                }
                WireInboundItem::Frame { session_id, frame } => {
                    if !matches!(
                        frame.kind,
                        NveWireKind::CommandDelta | NveWireKind::SceneKeyframe
                    ) || frame.payload.is_empty()
                    {
                        return None;
                    }
                    let is_keyframe = frame.kind == NveWireKind::SceneKeyframe;
                    let is_frame1 =
                        WireDecoder::validate_header(&frame.payload).is_ok_and(|(num, _)| num == 1);
                    let has_queued_resync_base = self.pending_frames.front().is_some_and(|f| {
                        (f.is_keyframe || f.is_frame1) && f.session_id == session_id
                    });
                    if !self.resync.should_apply(is_keyframe, is_frame1) && !has_queued_resync_base
                    {
                        return None;
                    }
                    if is_keyframe || is_frame1 {
                        self.pending_frames.clear();
                    } else if self.pending_frames.len() >= MAX_PENDING_REMOTE_FRAMES {
                        self.pending_frames.clear();
                        let client = &self.client;
                        self.resync
                            .on_overflowed(session_id, || client.request_keyframe().is_ok());
                        return None;
                    }
                    let seq = frame.sequence;
                    self.pending_frames.push_back(QueuedRemoteFrame {
                        session_id,
                        is_keyframe,
                        is_frame1,
                        payload: frame.payload,
                    });
                    self.has_seen_frame = true;
                    Some(seq)
                }
            }
        }

        pub fn wait_for_frame(&mut self, timeout: Duration) -> Result<(u32, Vec<u8>), RuntimeError> {
            let deadline = Instant::now() + timeout;
            let mut last_seq = self.client.metrics().last_sequence;
            while self.pending_frames.is_empty() {
                let now = Instant::now();
                if now >= deadline {
                    return Err(RuntimeError::Io(format!(
                        "timed out waiting for paired wire frame from {}",
                        self.endpoint_label
                    )));
                }
                let Some(item) = self.client.recv_item_timeout(deadline - now) else {
                    continue;
                };
                if let Some(seq) = self.ingest_item(item) {
                    last_seq = seq;
                }
            }
            while let Some(item) = self.client.try_recv_item() {
                if let Some(seq) = self.ingest_item(item) {
                    last_seq = seq;
                }
            }
            let latest = self
                .pending_frames
                .back()
                .map(|f| f.payload.clone())
                .unwrap_or_default();
            Ok((last_seq, latest))
        }

        pub fn take_queued_wire_frame(&mut self) -> Option<(bool, Vec<u8>)> {
            while let Some(item) = self.client.try_recv_item() {
                let _ = self.ingest_item(item);
            }
            let q = self.pending_frames.pop_front()?;
            self.last_dequeued_session = WireSessionEpoch(q.session_id);
            self.last_dequeued_resets = q.is_keyframe || q.is_frame1;
            Some((q.is_keyframe, q.payload))
        }

        pub fn on_queued_wire_frame_applied(&mut self, is_keyframe: bool) {
            if is_keyframe || self.last_dequeued_resets {
                self.resync.mark_keyframe_applied();
            }
        }

        pub fn on_queued_wire_frame_error(&mut self) {
            self.pending_frames.clear();
            let sid = if self.last_dequeued_session.0 != 0 {
                self.last_dequeued_session.0
            } else {
                self.client.active_session_id()
            };
            let client = &self.client;
            self.resync
                .on_decode_error(sid, || client.request_keyframe().is_ok());
        }

        pub fn remote_status_banner(&self) -> String {
            let m = self.metrics();
            let transport = self.client.transport_kind();
            if self.client.auth_rejections() > 0 {
                let reason = self
                    .client
                    .last_error()
                    .unwrap_or_else(|| "unauthorized".to_owned());
                return format!("Paired {} [AUTH REJECTED] · {reason}", transport.as_str());
            }
            let state = if self.is_connected() {
                "LIVE"
            } else {
                "RECONNECTING"
            };
            format!(
                "Paired {} [{}] · seq #{} · {} B rx",
                transport.as_str(),
                state,
                m.last_sequence,
                m.wire_bytes_rx
            )
        }

        pub fn disconnect(&mut self) {
            self.client.shutdown();
        }
    }

    impl Drop for PairReplicaClient {
        fn drop(&mut self) {
            self.disconnect();
        }
    }
}

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
mod disabled {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use nve_abi::{NveRuntimeMetrics, ValidatedEvents};

    use super::PairInboundEvent;
    use crate::builder::RuntimeError;
    use crate::wake::WakeBridge;

    pub(crate) struct ValidatedPairUri;

    pub(crate) struct PairSession;

    impl PairSession {
        pub(crate) fn validate_and_parse(
            _uri: &str,
            _token: &str,
        ) -> Result<ValidatedPairUri, RuntimeError> {
            Err(RuntimeError::Unsupported(
                "nve-runtime built without wire pairing support",
            ))
        }

        pub(crate) fn spawn_parsed(
            _validated: ValidatedPairUri,
            _wake: Arc<WakeBridge>,
            _connected_flag: Arc<AtomicBool>,
        ) -> Result<Self, RuntimeError> {
            Err(RuntimeError::Unsupported(
                "nve-runtime built without wire pairing support",
            ))
        }

        pub(crate) fn has_pending_frame(&self) -> bool {
            false
        }

        pub(crate) fn on_session_started(
            &mut self,
            _session_id: u64,
            _reconnected: bool,
            _has_prior_state: bool,
        ) {
        }

        pub(crate) fn on_overflowed(&mut self, _session_id: u64) {}

        pub(crate) fn on_decode_error(&mut self, _session_id: u64) {}

        pub(crate) fn should_apply(&self, _is_keyframe: bool, _is_frame1: bool) -> bool {
            true
        }

        pub(crate) fn mark_keyframe_applied(&mut self) {}

        pub(crate) fn try_recv_event(&self) -> Option<PairInboundEvent> {
            None
        }

        pub(crate) fn send_events(&self, _events: ValidatedEvents<'_>) -> bool {
            false
        }

        pub(crate) fn sync_metrics(&mut self, dst: &mut NveRuntimeMetrics) {
            dst.pair_connected = 0;
        }

        pub(crate) fn overlay_live_metrics(&self, dst: &mut NveRuntimeMetrics) {
            dst.pair_connected = 0;
        }

        pub(crate) fn disable_wake(&self) {}

        pub(crate) fn shutdown(&mut self) {}
    }
}

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
pub(crate) use enabled::PairSession;
#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
pub use enabled::PairReplicaClient;

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
pub(crate) use disabled::PairSession;
