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
    NveWireChannel, NveWireFrame, NveWireHello, NveWireHelloAck, NveWireKind,
    NveWireRejectReason, NveWireTransportKind, DEFAULT_MAX_WIRE_FRAME_BYTES,
    DEFAULT_WIRE_SCHEMA_VERSION, NVE_WIRE_CAP_DATAGRAMS, NVE_WIRE_CAP_NODE_PATCHES,
    NVE_WIRE_CAP_SCENE_ARCHIVE, NVE_WIRE_CAP_SLOT_CHUNKS, NVE_WIRE_FLAG_DATAGRAM,
};

use crate::auth::verify_bearer_token;
use crate::server_peers::{handle_quic_or_wt_connection, handle_tcp_ws_connection};
use crate::tls::{build_dual_alpn_quic_server_config, build_wss_server_config, NveDevCertificate};
use crate::{NveWireInboundMessage, NveWireServerConfig, NveWireServerMetrics, NveWireTransportError};

#[derive(Clone)]
pub(crate) enum OutboundCommand {
    Stream(Bytes),
    Datagram(Bytes),
}

pub(crate) struct SharedServerState {
    bearer_token: String,
    schema_version: u16,
    pub(crate) max_frame_bytes: usize,
    next_session_id: AtomicU32,
    peers: Mutex<HashMap<u32, (NveWireTransportKind, mpsc::UnboundedSender<OutboundCommand>)>>,
    latest_keyframe: Mutex<Option<Bytes>>,
    inbound_tx: Sender<NveWireInboundMessage>,
    inbound_notify: Arc<tokio::sync::Notify>,
    frames_sent: AtomicU64,
    datagrams_sent: AtomicU64,
    frames_received: AtomicU64,
    wire_bytes_tx: AtomicU64,
    wire_bytes_rx: AtomicU64,
    rejected_handshakes: AtomicU64,
    shutdown: AtomicBool,
}

impl SharedServerState {
    pub(crate) fn register_peer(
        &self,
        transport: NveWireTransportKind,
    ) -> (u32, mpsc::UnboundedReceiver<OutboundCommand>) {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        if let Some(kf) = self.latest_keyframe.lock().unwrap().clone() {
            let _ = tx.send(OutboundCommand::Stream(kf));
        }
        self.peers.lock().unwrap().insert(session_id, (transport, tx));
        (session_id, rx)
    }

    pub(crate) fn peer_sender(
        &self,
        session_id: u32,
    ) -> Option<mpsc::UnboundedSender<OutboundCommand>> {
        self.peers
            .lock()
            .unwrap()
            .get(&session_id)
            .map(|(_, tx)| tx.clone())
    }

    pub(crate) fn unregister_peer(&self, session_id: u32) {
        self.peers.lock().unwrap().remove(&session_id);
    }

    pub(crate) fn verify_hello(
        &self,
        hello: &NveWireHello,
    ) -> Result<(), (NveWireRejectReason, &'static str)> {
        if !verify_bearer_token(&self.bearer_token, &hello.bearer_token) {
            self.rejected_handshakes.fetch_add(1, Ordering::Relaxed);
            return Err((NveWireRejectReason::Unauthorized, "invalid bearer token"));
        }
        if hello.schema_version != self.schema_version {
            self.rejected_handshakes.fetch_add(1, Ordering::Relaxed);
            return Err((
                NveWireRejectReason::SchemaVersionMismatch,
                "wire schema version mismatch",
            ));
        }
        Ok(())
    }

    pub(crate) fn make_hello_ack(&self, session_id: u32) -> Bytes {
        let ack = NveWireHelloAck {
            session_id,
            schema_version: self.schema_version,
            server_capabilities: NVE_WIRE_CAP_SCENE_ARCHIVE
                | NVE_WIRE_CAP_NODE_PATCHES
                | NVE_WIRE_CAP_DATAGRAMS
                | NVE_WIRE_CAP_SLOT_CHUNKS,
        };
        let bytes = NveWireFrame::new(NveWireKind::HelloAck, 1, ack.encode().to_vec())
            .encode()
            .expect("hello ack encode");
        Bytes::from(bytes)
    }

    pub(crate) fn make_reject(reason: NveWireRejectReason, detail: &str) -> Bytes {
        let bytes = NveWireFrame::new(NveWireKind::Reject, 1, reason.encode(detail))
            .encode()
            .expect("reject encode");
        Bytes::from(bytes)
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
            _ => {
                if frame.kind == NveWireKind::KeyframeRequest
                    && let Some(kf) = self.latest_keyframe.lock().unwrap().clone()
                {
                    let _ = reply_tx.send(OutboundCommand::Stream(kf));
                }
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
            peers: Mutex::new(HashMap::new()),
            latest_keyframe: Mutex::new(None),
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
        self.state.peers.lock().unwrap().len()
    }

    pub fn has_quic_peers(&self) -> bool {
        self.state
            .peers
            .lock()
            .unwrap()
            .values()
            .any(|(k, _)| matches!(k, NveWireTransportKind::Quic | NveWireTransportKind::WebTransport))
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
    pub fn broadcast_frame(&self, frame: &NveWireFrame) -> Result<usize, NveWireTransportError> {
        let encoded = Bytes::from(frame.encode()?);
        if frame.kind == NveWireKind::SceneKeyframe || frame.kind == NveWireKind::CommandDelta {
            *self.state.latest_keyframe.lock().unwrap() = Some(encoded.clone());
        }
        let peers = self.state.peers.lock().unwrap();
        let mut delivered = 0usize;
        for (_, tx) in peers.values() {
            if tx.send(OutboundCommand::Stream(encoded.clone())).is_ok() {
                delivered += 1;
            }
        }
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

    /// Broadcasts an unreliable RFC 9221 datagram (falls back to stream on WSS/WS peers).
    pub fn broadcast_datagram(&self, frame: &NveWireFrame) -> Result<usize, NveWireTransportError> {
        let mut dgram_frame = frame.clone();
        dgram_frame.flags |= NVE_WIRE_FLAG_DATAGRAM;
        let encoded = Bytes::from(dgram_frame.encode()?);
        let peers = self.state.peers.lock().unwrap();
        let mut delivered = 0usize;
        for (kind, tx) in peers.values() {
            let cmd = match kind {
                NveWireTransportKind::Quic | NveWireTransportKind::WebTransport => {
                    OutboundCommand::Datagram(encoded.clone())
                }
                _ => OutboundCommand::Stream(encoded.clone()),
            };
            if tx.send(cmd).is_ok() {
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
