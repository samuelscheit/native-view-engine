//! Unified dual-ALPN QUIC (`nve-wire-2`) + HTTP/3 WebTransport (`h3`) + TLS WebSocket (`wss://`)
//! server running on ONE UDP socket + ONE TCP fallback socket with RFC 9221 datagrams.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use crossbeam_channel::{unbounded, Receiver, Sender};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;

use nve_wire_core::{
    NveWireChannel, NveWireFrame, NveWireKind, NveWireTransportKind, DEFAULT_MAX_WIRE_FRAME_BYTES,
    DEFAULT_WIRE_SCHEMA_VERSION, NVE_WIRE_FLAG_DATAGRAM,
};

use crate::server_peers::{handle_quic_or_wt_connection, handle_tcp_ws_connection};
use crate::tls::{build_dual_alpn_quic_server_config, build_wss_server_config, NveDevCertificate};
use crate::{
    NveWireInboundMessage, NveWireServerConfig, NveWireServerMetrics, NveWireTransportError,
    KEYFRAME_REQ_RECONNECT_INIT,
};

#[derive(Clone)]
pub(crate) enum OutboundCommand {
    Stream(Bytes),
    Datagram(Bytes),
}

#[derive(Clone)]
struct CachedKeyframe {
    seq: u64,
    bytes: Bytes,
}

struct PeerEntry {
    transport: NveWireTransportKind,
    tx: mpsc::UnboundedSender<OutboundCommand>,
    /// Keyframe sequence pushed specifically by `register_peer` on connect (NEVER set by
    /// `broadcast_frame`) so only duplicate reconnect-init requests are deduplicated.
    join_sent_kf_seq: u64,
    pending_kf_request: bool,
}

pub(crate) struct SharedServerState {
    pub(crate) bearer_token: String,
    pub(crate) schema_version: u16,
    pub(crate) max_frame_bytes: usize,
    next_session_id: AtomicU32,
    next_kf_seq: AtomicU64,
    had_scene_frame: AtomicBool,
    // Lock ordering invariant: ALWAYS acquire `latest_keyframe` BEFORE `peers`.
    latest_keyframe: Mutex<Option<CachedKeyframe>>,
    peers: Mutex<HashMap<u32, PeerEntry>>,
    inbound_tx: Sender<NveWireInboundMessage>,
    inbound_notify: Arc<tokio::sync::Notify>,
    frames_sent: AtomicU64,
    datagrams_sent: AtomicU64,
    frames_received: AtomicU64,
    wire_bytes_tx: AtomicU64,
    wire_bytes_rx: AtomicU64,
    pub(crate) rejected_handshakes: AtomicU64,
    shutdown: AtomicBool,
}

impl SharedServerState {
    pub(crate) fn register_peer(
        &self,
        transport: NveWireTransportKind,
    ) -> (u32, mpsc::UnboundedReceiver<OutboundCommand>) {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        let mut emit_join_kf_req = false;
        {
            let kf_guard = self
                .latest_keyframe
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
            let (join_sent_kf_seq, pending_kf_request) = if let Some(cached) = kf_guard.as_ref() {
                if tx.send(OutboundCommand::Stream(cached.bytes.clone())).is_ok() {
                    self.frames_sent.fetch_add(1, Ordering::Relaxed);
                    self.wire_bytes_tx
                        .fetch_add(cached.bytes.len() as u64, Ordering::Relaxed);
                    (cached.seq, false)
                } else {
                    (0, false)
                }
            } else if self.had_scene_frame.load(Ordering::Acquire) {
                // Deltas invalidated `latest_keyframe`; ask producer for ONE fresh full keyframe
                // targeted to this joining/reconnecting peer so it never rolls back to Frame 1.
                emit_join_kf_req = true;
                (0, true)
            } else {
                (0, false)
            };
            peers.insert(
                session_id,
                PeerEntry {
                    transport,
                    tx,
                    join_sent_kf_seq,
                    pending_kf_request,
                },
            );
        }
        if emit_join_kf_req {
            let _ = self.inbound_tx.send(NveWireInboundMessage {
                session_id,
                transport,
                frame: NveWireFrame::new(NveWireKind::KeyframeRequest, 0, Vec::new()),
            });
            self.inbound_notify.notify_waiters();
        }
        (session_id, rx)
    }

    pub(crate) fn peer_sender(
        &self,
        session_id: u32,
    ) -> Option<mpsc::UnboundedSender<OutboundCommand>> {
        self.peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&session_id)
            .map(|p| p.tx.clone())
    }

    pub(crate) fn unregister_peer(&self, session_id: u32) {
        self.peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&session_id);
    }

    pub(crate) fn handle_inbound_frame(
        &self,
        session_id: u32,
        transport: NveWireTransportKind,
        frame: NveWireFrame,
        reply_tx: &mpsc::UnboundedSender<OutboundCommand>,
    ) -> bool {
        self.frames_received.fetch_add(1, Ordering::Relaxed);
        self.wire_bytes_rx
            .fetch_add((frame.payload.len() + 16) as u64, Ordering::Relaxed);
        match frame.kind {
            NveWireKind::Ping => {
                if let Ok(pong) =
                    NveWireFrame::new(NveWireKind::Pong, frame.sequence, frame.payload).encode()
                {
                    let _ = reply_tx.send(OutboundCommand::Stream(Bytes::from(pong)));
                }
                true
            }
            NveWireKind::Bye => false,
            NveWireKind::KeyframeRequest => {
                let is_reconnect_init =
                    frame.payload.as_slice() == [KEYFRAME_REQ_RECONNECT_INIT];
                // Lock ordering: `latest_keyframe` -> `peers`.
                let mut forward_to_producer = false;
                {
                    let kf_guard = self
                        .latest_keyframe
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(cached) = kf_guard.as_ref() {
                        // Only deduplicate automatic reconnect-init requests against the exact
                        // cached keyframe `register_peer` already pushed; explicit recovery
                        // requests (empty payload) are ALWAYS served immediately!
                        let suppress_dup = is_reconnect_init
                            && peers.get_mut(&session_id).is_some_and(|p| {
                                if p.join_sent_kf_seq == cached.seq {
                                    p.join_sent_kf_seq = 0;
                                    true
                                } else {
                                    false
                                }
                            });
                        if !suppress_dup {
                            if let Some(p) = peers.get_mut(&session_id) {
                                p.join_sent_kf_seq = 0;
                                p.pending_kf_request = false;
                            }
                            if reply_tx
                                .send(OutboundCommand::Stream(cached.bytes.clone()))
                                .is_ok()
                            {
                                self.frames_sent.fetch_add(1, Ordering::Relaxed);
                                self.wire_bytes_tx
                                    .fetch_add(cached.bytes.len() as u64, Ordering::Relaxed);
                            }
                        }
                    } else if let Some(p) = peers.get_mut(&session_id) {
                        if !(is_reconnect_init && p.pending_kf_request) {
                            p.pending_kf_request = true;
                            forward_to_producer = true;
                        }
                    } else {
                        forward_to_producer = true;
                    }
                }
                if forward_to_producer {
                    let _ = self.inbound_tx.send(NveWireInboundMessage {
                        session_id,
                        transport,
                        frame,
                    });
                    self.inbound_notify.notify_waiters();
                }
                true
            }
            _ => {
                let _ = self.inbound_tx.send(NveWireInboundMessage {
                    session_id,
                    transport,
                    frame,
                });
                self.inbound_notify.notify_waiters();
                true
            }
        }
    }
}

/// Live dual-ALPN QUIC/WebTransport + TLS WebSocket (`wss://`) server.
pub struct NveWireServer {
    quic_addr: SocketAddr,
    wss_addr: SocketAddr,
    cert: NveDevCertificate,
    state: Arc<SharedServerState>,
    inbound_rx: Receiver<NveWireInboundMessage>,
    quic_endpoint: quinn::Endpoint,
    runtime: Option<Arc<tokio::runtime::Runtime>>,
    _shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl NveWireServer {
    /// Starts a server using an internal multi-threaded Tokio runtime.
    pub fn bind_sync(config: NveWireServerConfig) -> Result<Self, NveWireTransportError> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| NveWireTransportError::Io(e.to_string()))?,
        );
        let mut server = rt.block_on(Self::bind_async(config))?;
        server.runtime = Some(rt);
        Ok(server)
    }

    /// Starts the dual-ALPN QUIC + WSS server on the current Tokio runtime.
    pub async fn bind_async(config: NveWireServerConfig) -> Result<Self, NveWireTransportError> {
        if config.bearer_token.trim().is_empty() {
            return Err(NveWireTransportError::Auth(
                "server bearer_token must not be empty".to_owned(),
            ));
        }
        let cert = NveDevCertificate::generate(&config.cert_sans)?;
        let quic_server_cfg = build_dual_alpn_quic_server_config(&cert)?;
        let udp_socket = std::net::UdpSocket::bind(config.quic_bind_addr)
            .map_err(|e| NveWireTransportError::Io(format!("UDP bind failed: {e}")))?;
        let ep_cfg = web_transport_quinn::crypto::endpoint_config()
            .map_err(|e| NveWireTransportError::Tls(format!("QUIC endpoint config failed: {e}")))?;
        let quic_endpoint = quinn::Endpoint::new(
            ep_cfg,
            Some(quic_server_cfg),
            udp_socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|e| NveWireTransportError::Io(format!("QUIC endpoint failed: {e}")))?;
        let quic_addr = quic_endpoint
            .local_addr()
            .map_err(|e| NveWireTransportError::Io(e.to_string()))?;

        let tcp_listener = TcpListener::bind(config.wss_bind_addr)
            .await
            .map_err(|e| NveWireTransportError::Io(format!("TCP/WSS bind failed: {e}")))?;
        let wss_addr = tcp_listener
            .local_addr()
            .map_err(|e| NveWireTransportError::Io(e.to_string()))?;

        let tls_acceptor = TlsAcceptor::from(build_wss_server_config(&cert)?);
        let (inbound_tx, inbound_rx) = unbounded();
        let state = Arc::new(SharedServerState {
            bearer_token: config.bearer_token,
            schema_version: DEFAULT_WIRE_SCHEMA_VERSION,
            max_frame_bytes: config.max_frame_bytes.max(DEFAULT_MAX_WIRE_FRAME_BYTES),
            next_session_id: AtomicU32::new(1),
            next_kf_seq: AtomicU64::new(1),
            had_scene_frame: AtomicBool::new(false),
            latest_keyframe: Mutex::new(None),
            peers: Mutex::new(HashMap::new()),
            inbound_tx,
            inbound_notify: Arc::new(tokio::sync::Notify::new()),
            frames_sent: AtomicU64::new(0),
            datagrams_sent: AtomicU64::new(0),
            frames_received: AtomicU64::new(0),
            wire_bytes_tx: AtomicU64::new(0),
            wire_bytes_rx: AtomicU64::new(0),
            rejected_handshakes: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
        });

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let ep_clone = quic_endpoint.clone();
        let state_quic = state.clone();
        let mut shutdown_quic = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_quic.changed() => break,
                    incoming = ep_clone.accept() => {
                        let Some(incoming) = incoming else { break };
                        let st = state_quic.clone();
                        tokio::spawn(async move {
                            if let Ok(conn) = incoming.await {
                                handle_quic_or_wt_connection(conn, st).await;
                            }
                        });
                    }
                }
            }
        });

        let state_wss = state.clone();
        let mut shutdown_wss = shutdown_rx;
        let allow_plain_loopback_ws = config.allow_plain_loopback_ws;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_wss.changed() => break,
                    accepted = tcp_listener.accept() => {
                        let Ok((stream, peer_addr)) = accepted else { break };
                        let st = state_wss.clone();
                        let acc = tls_acceptor.clone();
                        tokio::spawn(async move {
                            handle_tcp_ws_connection(
                                stream,
                                peer_addr,
                                acc,
                                allow_plain_loopback_ws,
                                st,
                            )
                            .await;
                        });
                    }
                }
            }
        });

        Ok(Self {
            quic_addr,
            wss_addr,
            cert,
            state,
            inbound_rx,
            quic_endpoint,
            runtime: None,
            _shutdown_tx: shutdown_tx,
        })
    }

    pub fn quic_addr(&self) -> SocketAddr {
        self.quic_addr
    }

    pub fn wss_addr(&self) -> SocketAddr {
        self.wss_addr
    }

    pub fn cert_sha256(&self) -> [u8; 32] {
        self.cert.sha256
    }

    pub fn cert_sha256_hex(&self) -> String {
        self.cert.sha256_hex()
    }

    pub fn cert_sha256_base64(&self) -> String {
        self.cert.sha256_base64()
    }

    pub fn peer_count(&self) -> usize {
        self.state
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn has_quic_peers(&self) -> bool {
        self.state
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|p| {
                matches!(
                    p.transport,
                    NveWireTransportKind::Quic | NveWireTransportKind::WebTransport
                )
            })
    }

    pub fn wait_for_peers(&self, min_peers: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.peer_count() >= min_peers {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.peer_count() >= min_peers
    }

    pub fn metrics(&self) -> NveWireServerMetrics {
        NveWireServerMetrics {
            connected_peers: self.peer_count(),
            frames_sent: self.state.frames_sent.load(Ordering::Relaxed),
            datagrams_sent: self.state.datagrams_sent.load(Ordering::Relaxed),
            frames_received: self.state.frames_received.load(Ordering::Relaxed),
            wire_bytes_tx: self.state.wire_bytes_tx.load(Ordering::Relaxed),
            wire_bytes_rx: self.state.wire_bytes_rx.load(Ordering::Relaxed),
            rejected_handshakes: self.state.rejected_handshakes.load(Ordering::Relaxed),
        }
    }

    /// Broadcasts one `NveWireFrame` to all connected peers (QUIC, WebTransport, and WSS/WS).
    ///
    /// Locks `latest_keyframe` before `peers` consistently with `register_peer`. Genuine
    /// [`NveWireKind::SceneKeyframe`] frames refresh `latest_keyframe`; [`NveWireKind::CommandDelta`]
    /// frames invalidate `latest_keyframe` (`None`) so late-joining or reconnecting peers never
    /// roll back to a stale Frame-1 snapshot.
    pub fn broadcast_frame(&self, frame: &NveWireFrame) -> Result<usize, NveWireTransportError> {
        let encoded = Bytes::from(frame.encode()?);
        let mut kf_guard = self
            .state
            .latest_keyframe
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut peers = self.state.peers.lock().unwrap_or_else(|e| e.into_inner());
        let is_keyframe = match frame.kind {
            NveWireKind::SceneKeyframe => {
                let seq = self.state.next_kf_seq.fetch_add(1, Ordering::Relaxed);
                *kf_guard = Some(CachedKeyframe {
                    seq,
                    bytes: encoded.clone(),
                });
                self.state.had_scene_frame.store(true, Ordering::Release);
                true
            }
            NveWireKind::CommandDelta | NveWireKind::NodePatchDelta => {
                *kf_guard = None;
                self.state.had_scene_frame.store(true, Ordering::Release);
                false
            }
            _ => false,
        };
        let mut delivered = 0usize;
        for peer in peers.values_mut() {
            if peer.tx.send(OutboundCommand::Stream(encoded.clone())).is_ok() {
                delivered += 1;
                if is_keyframe {
                    peer.join_sent_kf_seq = 0;
                    peer.pending_kf_request = false;
                } else if matches!(
                    frame.kind,
                    NveWireKind::CommandDelta | NveWireKind::NodePatchDelta
                ) {
                    peer.join_sent_kf_seq = 0;
                }
            }
        }
        drop(peers);
        drop(kf_guard);
        if delivered > 0 {
            self.state
                .frames_sent
                .fetch_add(delivered as u64, Ordering::Relaxed);
            self.state
                .wire_bytes_tx
                .fetch_add((encoded.len() * delivered) as u64, Ordering::Relaxed);
        }
        Ok(delivered)
    }

    /// Sends one `NveWireFrame` directly to a single connected peer identified by `session_id`
    /// without broadcasting to other peers or modifying `latest_keyframe`.
    pub fn send_frame_to_peer(
        &self,
        session_id: u32,
        frame: &NveWireFrame,
    ) -> Result<bool, NveWireTransportError> {
        let encoded = Bytes::from(frame.encode()?);
        let mut peers = self.state.peers.lock().unwrap_or_else(|e| e.into_inner());
        let Some(peer) = peers.get_mut(&session_id) else {
            return Ok(false);
        };
        if peer.tx.send(OutboundCommand::Stream(encoded.clone())).is_ok() {
            if frame.kind == NveWireKind::SceneKeyframe {
                peer.pending_kf_request = false;
            }
            drop(peers);
            self.state.frames_sent.fetch_add(1, Ordering::Relaxed);
            self.state
                .wire_bytes_tx
                .fetch_add(encoded.len() as u64, Ordering::Relaxed);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Sends a targeted [`NveWireKind::SceneKeyframe`] to a single peer (e.g. in response to a
    /// `KeyframeRequest`) without resetting other healthy peers.
    pub fn send_scene_keyframe_to_peer(
        &self,
        session_id: u32,
        sequence: u32,
        scene_archive: &[u8],
    ) -> Result<bool, NveWireTransportError> {
        let frame = NveWireFrame::new(NveWireKind::SceneKeyframe, sequence, scene_archive.to_vec())
            .with_channel(NveWireChannel::UiScene);
        self.send_frame_to_peer(session_id, &frame)
    }

    /// Broadcasts an unreliable RFC 9221 datagram (falls back to stream on WSS/WS peers).
    pub fn broadcast_datagram(&self, frame: &NveWireFrame) -> Result<usize, NveWireTransportError> {
        let mut dgram_frame = frame.clone();
        dgram_frame.flags |= NVE_WIRE_FLAG_DATAGRAM;
        let encoded = Bytes::from(dgram_frame.encode()?);
        let peers = self.state.peers.lock().unwrap_or_else(|e| e.into_inner());
        let mut delivered = 0usize;
        for peer in peers.values() {
            let cmd = match peer.transport {
                NveWireTransportKind::Quic | NveWireTransportKind::WebTransport => {
                    OutboundCommand::Datagram(encoded.clone())
                }
                _ => OutboundCommand::Stream(encoded.clone()),
            };
            if peer.tx.send(cmd).is_ok() {
                delivered += 1;
            }
        }
        if delivered > 0 {
            self.state
                .datagrams_sent
                .fetch_add(delivered as u64, Ordering::Relaxed);
            self.state
                .wire_bytes_tx
                .fetch_add((encoded.len() * delivered) as u64, Ordering::Relaxed);
        }
        Ok(delivered)
    }

    pub fn broadcast_scene_keyframe(
        &self,
        sequence: u32,
        scene_archive: &[u8],
    ) -> Result<usize, NveWireTransportError> {
        let frame = NveWireFrame::new(NveWireKind::SceneKeyframe, sequence, scene_archive.to_vec())
            .with_channel(NveWireChannel::UiScene);
        self.broadcast_frame(&frame)
    }

    pub fn broadcast_command_delta(
        &self,
        sequence: u32,
        command_bytes: &[u8],
    ) -> Result<usize, NveWireTransportError> {
        let frame = NveWireFrame::new(NveWireKind::CommandDelta, sequence, command_bytes.to_vec())
            .with_channel(NveWireChannel::UiScene);
        self.broadcast_frame(&frame)
    }

    pub fn try_recv_inbound(&self) -> Option<NveWireInboundMessage> {
        self.inbound_rx.try_recv().ok()
    }

    pub fn recv_inbound_timeout(&self, timeout: Duration) -> Option<NveWireInboundMessage> {
        self.inbound_rx.recv_timeout(timeout).ok()
    }

    pub async fn recv_inbound_async(&self, timeout: Duration) -> Option<NveWireInboundMessage> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.state.inbound_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Ok(msg) = self.inbound_rx.try_recv() {
                return Some(msg);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.inbound_rx.try_recv().ok();
            }
        }
    }

    pub fn shutdown(&self) {
        self.state.shutdown.store(true, Ordering::Relaxed);
        let _ = self._shutdown_tx.send(true);
        self.quic_endpoint.close(0u32.into(), b"server shutdown");
    }
}

impl Drop for NveWireServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}
