//! Unified `NveWireClient` supporting raw QUIC (`nve-wire-2`), HTTP/3 WebTransport (`h3`),
//! TLS WebSocket (`wss://`), and Happy-Eyeballs racing (QUIC first, 250ms WSS fallback).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use crossbeam_channel::{unbounded, Receiver};
use tokio::sync::mpsc;

use nve_wire_core::{
    NveWireChannel, NveWireFrame, NveWireHello, NveWireHelloAck, NveWireKind, NveWireMetrics,
    NveWireRejectReason, NveWireTransportKind, NVE_WIRE_FLAG_DATAGRAM,
};

use crate::framing::{read_stream_frame, read_wt_frame, write_stream_raw, write_wt_raw};
use crate::tls::{
    build_quic_client_config, build_quic_transport_config, ensure_crypto_provider,
};
use crate::{NveWireClientConfig, NveWireTransportError};

pub(crate) enum ClientOut {
    Stream(Bytes),
    Datagram(Bytes),
    Close,
}

pub(crate) struct ClientStats {
    frames_received: AtomicU64,
    keyframes_received: AtomicU64,
    command_deltas_received: AtomicU64,
    node_patch_deltas_received: AtomicU64,
    slot_chunks_received: AtomicU64,
    crdt_deltas_received: AtomicU64,
    presence_received: AtomicU64,
    events_sent: AtomicU64,
    wire_bytes_rx: AtomicU64,
    wire_bytes_tx: AtomicU64,
    last_sequence: AtomicU32,
    pub(crate) connected: AtomicBool,
}

impl ClientStats {
    pub(crate) fn new() -> Self {
        Self {
            frames_received: AtomicU64::new(0),
            keyframes_received: AtomicU64::new(0),
            command_deltas_received: AtomicU64::new(0),
            node_patch_deltas_received: AtomicU64::new(0),
            slot_chunks_received: AtomicU64::new(0),
            crdt_deltas_received: AtomicU64::new(0),
            presence_received: AtomicU64::new(0),
            events_sent: AtomicU64::new(0),
            wire_bytes_rx: AtomicU64::new(0),
            wire_bytes_tx: AtomicU64::new(0),
            last_sequence: AtomicU32::new(0),
            connected: AtomicBool::new(true),
        }
    }

    pub(crate) fn record_inbound(&self, frame: &NveWireFrame) {
        self.wire_bytes_rx
            .fetch_add((frame.payload.len() + 16) as u64, Ordering::Relaxed);
        self.last_sequence.store(frame.sequence, Ordering::Relaxed);
        match frame.kind {
            NveWireKind::SceneKeyframe => {
                self.frames_received.fetch_add(1, Ordering::Relaxed);
                self.keyframes_received.fetch_add(1, Ordering::Relaxed);
            }
            NveWireKind::CommandDelta => {
                self.frames_received.fetch_add(1, Ordering::Relaxed);
                self.command_deltas_received.fetch_add(1, Ordering::Relaxed);
            }
            NveWireKind::NodePatchDelta => {
                self.frames_received.fetch_add(1, Ordering::Relaxed);
                self.node_patch_deltas_received.fetch_add(1, Ordering::Relaxed);
            }
            NveWireKind::SlotChunk => {
                self.slot_chunks_received.fetch_add(1, Ordering::Relaxed);
            }
            NveWireKind::CrdtDelta => {
                self.crdt_deltas_received.fetch_add(1, Ordering::Relaxed);
            }
            NveWireKind::Presence => {
                self.presence_received.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    fn snapshot(&self, session_id: u32) -> NveWireMetrics {
        NveWireMetrics {
            session_id,
            frames_received: self.frames_received.load(Ordering::Relaxed),
            keyframes_received: self.keyframes_received.load(Ordering::Relaxed),
            command_deltas_received: self.command_deltas_received.load(Ordering::Relaxed),
            node_patch_deltas_received: self.node_patch_deltas_received.load(Ordering::Relaxed),
            slot_chunks_received: self.slot_chunks_received.load(Ordering::Relaxed),
            crdt_deltas_received: self.crdt_deltas_received.load(Ordering::Relaxed),
            presence_received: self.presence_received.load(Ordering::Relaxed),
            events_sent: self.events_sent.load(Ordering::Relaxed),
            wire_bytes_rx: self.wire_bytes_rx.load(Ordering::Relaxed),
            wire_bytes_tx: self.wire_bytes_tx.load(Ordering::Relaxed),
            last_sequence: self.last_sequence.load(Ordering::Relaxed),
        }
    }
}

/// Connected `nve-wire` client (QUIC, WebTransport, WSS, or Happy-Eyeballs).
pub struct NveWireClient {
    session_id: u32,
    transport_kind: NveWireTransportKind,
    outbound_tx: mpsc::UnboundedSender<ClientOut>,
    inbound_rx: Receiver<NveWireFrame>,
    inbound_notify: Arc<tokio::sync::Notify>,
    stats: Arc<ClientStats>,
    next_sequence: AtomicU32,
    _runtime: Option<Arc<tokio::runtime::Runtime>>,
}

impl NveWireClient {
    pub(crate) fn from_parts(
        session_id: u32,
        transport_kind: NveWireTransportKind,
        outbound_tx: mpsc::UnboundedSender<ClientOut>,
        inbound_rx: Receiver<NveWireFrame>,
        inbound_notify: Arc<tokio::sync::Notify>,
        stats: Arc<ClientStats>,
    ) -> Self {
        Self {
            session_id,
            transport_kind,
            outbound_tx,
            inbound_rx,
            inbound_notify,
            stats,
            next_sequence: AtomicU32::new(2),
            _runtime: None,
        }
    }

    pub(crate) fn set_runtime(&mut self, rt: Arc<tokio::runtime::Runtime>) {
        self._runtime = Some(rt);
    }

    /// Connects over raw QUIC (`nve-wire-2` ALPN) with SHA-256 certificate pinning.
    pub async fn connect_quic(
        addr: SocketAddr,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError> {
        let cert_hash = config.cert_sha256.ok_or_else(|| {
            NveWireTransportError::Tls("QUIC connect requires pinned cert_sha256".to_owned())
        })?;
        let client_cfg = build_quic_client_config(cert_hash)?;
        let bind_addr: SocketAddr = if addr.is_ipv6() {
            "[::]:0".parse().unwrap()
        } else {
            "0.0.0.0:0".parse().unwrap()
        };
        let udp = std::net::UdpSocket::bind(bind_addr)
            .map_err(|e| NveWireTransportError::Io(e.to_string()))?;
        let ep_cfg = web_transport_quinn::crypto::endpoint_config()
            .map_err(|e| NveWireTransportError::Tls(e.to_string()))?;
        let mut endpoint = quinn::Endpoint::new(ep_cfg, None, udp, Arc::new(quinn::TokioRuntime))
            .map_err(|e| NveWireTransportError::Io(e.to_string()))?;
        endpoint.set_default_client_config(client_cfg);

        let conn = tokio::time::timeout(
            config.connect_timeout,
            endpoint
                .connect(addr, &config.server_name)
                .map_err(|e| NveWireTransportError::Connect(e.to_string()))?,
        )
        .await
        .map_err(|_| NveWireTransportError::Connect("QUIC handshake timed out".to_owned()))?
        .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;

        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;

        let hello_bytes = encode_hello(config)?;
        write_stream_raw(&mut send, &hello_bytes).await?;

        let ack = match read_stream_frame(&mut recv, config.max_frame_bytes).await? {
            Some(f) => decode_hello_response(f)?,
            None => {
                return Err(NveWireTransportError::Auth(
                    "server closed QUIC stream before HelloAck".to_owned(),
                ))
            }
        };

        let (inbound_tx, inbound_rx) = unbounded();
        let inbound_notify = Arc::new(tokio::sync::Notify::new());
        let (outbound_tx, mut out_rx) = mpsc::unbounded_channel();
        let stats = Arc::new(ClientStats::new());

        let conn_write = conn.clone();
        let stats_w = stats.clone();
        tokio::spawn(async move {
            let _ep = endpoint;
            while let Some(cmd) = out_rx.recv().await {
                match cmd {
                    ClientOut::Stream(bytes) => {
                        if write_stream_raw(&mut send, &bytes).await.is_err() {
                            break;
                        }
                    }
                    ClientOut::Datagram(bytes) => {
                        let _ = conn_write.send_datagram(bytes);
                    }
                    ClientOut::Close => {
                        conn_write.close(0u32.into(), b"bye");
                        break;
                    }
                }
            }
            stats_w.connected.store(false, Ordering::Relaxed);
        });

        let conn_dgram = conn.clone();
        let max_len = config.max_frame_bytes;
        let in_tx_d = inbound_tx.clone();
        let notify_d = inbound_notify.clone();
        let stats_d = stats.clone();
        tokio::spawn(async move {
            while let Ok(raw) = conn_dgram.read_datagram().await {
                if let Ok(frame) = NveWireFrame::decode(&raw, max_len) {
                    stats_d.record_inbound(&frame);
                    let _ = in_tx_d.send(frame);
                    notify_d.notify_waiters();
                }
            }
        });

        let stats_r = stats.clone();
        let notify_r = inbound_notify.clone();
        tokio::spawn(async move {
            while let Ok(Some(frame)) = read_stream_frame(&mut recv, max_len).await {
                stats_r.record_inbound(&frame);
                let _ = inbound_tx.send(frame);
                notify_r.notify_waiters();
            }
            stats_r.connected.store(false, Ordering::Relaxed);
        });

        Ok(Self::from_parts(
            ack.session_id,
            NveWireTransportKind::Quic,
            outbound_tx,
            inbound_rx,
            inbound_notify,
            stats,
        ))
    }

    /// Connects over real HTTP/3 WebTransport (`h3` ALPN) with 13-day ECDSA SHA-256 cert pinning.
    pub async fn connect_webtransport(
        url_str: &str,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError> {
        let _ = ensure_crypto_provider();
        let cert_hash = config.cert_sha256.ok_or_else(|| {
            NveWireTransportError::Tls("WebTransport connect requires pinned cert_sha256".to_owned())
        })?;
        let parsed_url = url::Url::parse(url_str)
            .map_err(|e| NveWireTransportError::Connect(format!("invalid WT url: {e}")))?;

        let mut transport = quinn::TransportConfig::default();
        transport.datagram_receive_buffer_size(Some(65_536));
        transport.datagram_send_buffer_size(65_536);
        let _ = build_quic_transport_config();

        let client = web_transport_quinn::ClientBuilder::new()
            .with_transport_config(transport)
            .with_server_certificate_hashes(vec![cert_hash.to_vec()])
            .map_err(|e| NveWireTransportError::Tls(format!("WT client builder failed: {e}")))?;

        let session = tokio::time::timeout(config.connect_timeout, client.connect(parsed_url))
            .await
            .map_err(|_| {
                NveWireTransportError::Connect("WebTransport handshake timed out".to_owned())
            })?
            .map_err(|e| {
                NveWireTransportError::Connect(format!("WebTransport connect failed: {e}"))
            })?;

        let (mut send, mut recv) = session
            .open_bi()
            .await
            .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;

        let hello_bytes = encode_hello(config)?;
        write_wt_raw(&mut send, &hello_bytes).await?;

        let ack = match read_wt_frame(&mut recv, config.max_frame_bytes).await? {
            Some(f) => decode_hello_response(f)?,
            None => {
                return Err(NveWireTransportError::Auth(
                    "server closed WebTransport stream before HelloAck".to_owned(),
                ))
            }
        };

        let (inbound_tx, inbound_rx) = unbounded();
        let inbound_notify = Arc::new(tokio::sync::Notify::new());
        let (outbound_tx, mut out_rx) = mpsc::unbounded_channel();
        let stats = Arc::new(ClientStats::new());

        let sess_write = session.clone();
        let stats_w = stats.clone();
        tokio::spawn(async move {
            while let Some(cmd) = out_rx.recv().await {
                match cmd {
                    ClientOut::Stream(bytes) => {
                        if write_wt_raw(&mut send, &bytes).await.is_err() {
                            break;
                        }
                    }
                    ClientOut::Datagram(bytes) => {
                        let _ = sess_write.send_datagram(bytes);
                    }
                    ClientOut::Close => {
                        sess_write.close(0, b"bye");
                        break;
                    }
                }
            }
            stats_w.connected.store(false, Ordering::Relaxed);
        });

        let sess_dgram = session.clone();
        let max_len = config.max_frame_bytes;
        let in_tx_d = inbound_tx.clone();
        let notify_d = inbound_notify.clone();
        let stats_d = stats.clone();
        tokio::spawn(async move {
            while let Ok(raw) = sess_dgram.read_datagram().await {
                if let Ok(frame) = NveWireFrame::decode(&raw, max_len) {
                    stats_d.record_inbound(&frame);
                    let _ = in_tx_d.send(frame);
                    notify_d.notify_waiters();
                }
            }
        });

        let stats_r = stats.clone();
        let notify_r = inbound_notify.clone();
        tokio::spawn(async move {
            while let Ok(Some(frame)) = read_wt_frame(&mut recv, max_len).await {
                stats_r.record_inbound(&frame);
                let _ = inbound_tx.send(frame);
                notify_r.notify_waiters();
            }
            stats_r.connected.store(false, Ordering::Relaxed);
        });

        Ok(Self::from_parts(
            ack.session_id,
            NveWireTransportKind::WebTransport,
            outbound_tx,
            inbound_rx,
            inbound_notify,
            stats,
        ))
    }

    /// Happy-Eyeballs race: attempts QUIC (`nve-wire-2`) immediately, and if QUIC fails early or
    /// exceeds `config.fallback_delay` (default `250ms`), races `wss_url` in parallel. First wins.
    pub async fn connect_happy_eyeballs(
        quic_addr: SocketAddr,
        wss_url: &str,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError> {
        let quic_fut = Self::connect_quic(quic_addr, config);
        tokio::pin!(quic_fut);

        let quic_early = tokio::select! {
            res = &mut quic_fut => Some(res),
            _ = tokio::time::sleep(config.fallback_delay) => None,
        };

        if let Some(Ok(client)) = quic_early {
            return Ok(client);
        }
        if let Some(Err(NveWireTransportError::Auth(msg))) = quic_early {
            return Err(NveWireTransportError::Auth(msg));
        }

        let wss_fut = Self::connect_ws(wss_url, config);
        tokio::pin!(wss_fut);

        if quic_early.is_some() {
            return wss_fut.await;
        }

        tokio::select! {
            q_res = &mut quic_fut => match q_res {
                Ok(c) => Ok(c),
                Err(_) => wss_fut.await,
            },
            w_res = &mut wss_fut => match w_res {
                Ok(c) => Ok(c),
                Err(_) => quic_fut.await,
            },
        }
    }

    #[inline]
    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    #[inline]
    pub fn transport_kind(&self) -> NveWireTransportKind {
        self.transport_kind
    }

    #[inline]
    pub fn is_connected(&self) -> bool {
        self.stats.connected.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn metrics(&self) -> NveWireMetrics {
        self.stats.snapshot(self.session_id)
    }

    fn alloc_seq(&self) -> u32 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed)
    }

    pub fn send_frame(&self, frame: &NveWireFrame) -> Result<(), NveWireTransportError> {
        let bytes = Bytes::from(frame.encode()?);
        self.stats
            .wire_bytes_tx
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.outbound_tx
            .send(ClientOut::Stream(bytes))
            .map_err(|_| NveWireTransportError::Io("wire client outbound closed".to_owned()))
    }

    pub fn send_datagram(&self, frame: &NveWireFrame) -> Result<(), NveWireTransportError> {
        let mut df = frame.clone();
        df.flags |= NVE_WIRE_FLAG_DATAGRAM;
        let bytes = Bytes::from(df.encode()?);
        self.stats
            .wire_bytes_tx
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.outbound_tx
            .send(ClientOut::Datagram(bytes))
            .map_err(|_| NveWireTransportError::Io("wire client outbound closed".to_owned()))
    }

    pub fn send_events(&self, event_ring_bytes: &[u8]) -> Result<(), NveWireTransportError> {
        let frame = NveWireFrame::new(
            NveWireKind::EventBatch,
            self.alloc_seq(),
            event_ring_bytes.to_vec(),
        )
        .with_channel(NveWireChannel::InputEvents);
        self.stats.events_sent.fetch_add(1, Ordering::Relaxed);
        self.send_frame(&frame)
    }

    pub fn request_keyframe(&self) -> Result<(), NveWireTransportError> {
        let frame = NveWireFrame::new(NveWireKind::KeyframeRequest, self.alloc_seq(), Vec::new());
        self.send_frame(&frame)
    }

    pub fn try_recv_frame(&self) -> Option<NveWireFrame> {
        self.inbound_rx.try_recv().ok()
    }

    pub fn recv_frame_timeout(&self, timeout: Duration) -> Option<NveWireFrame> {
        self.inbound_rx.recv_timeout(timeout).ok()
    }

    pub async fn recv_frame_async(&self, timeout: Duration) -> Option<NveWireFrame> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.inbound_notify.notified();
            if let Ok(frame) = self.inbound_rx.try_recv() {
                return Some(frame);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.inbound_rx.try_recv().ok();
            }
        }
    }

    pub fn close(&self) {
        self.stats.connected.store(false, Ordering::Relaxed);
        let bye = NveWireFrame::new(NveWireKind::Bye, self.alloc_seq(), Vec::new());
        if let Ok(encoded) = bye.encode() {
            let _ = self.outbound_tx.send(ClientOut::Stream(Bytes::from(encoded)));
        }
        let _ = self.outbound_tx.send(ClientOut::Close);
    }
}

impl Drop for NveWireClient {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) fn encode_hello(config: &NveWireClientConfig) -> Result<Vec<u8>, NveWireTransportError> {
    let hello = NveWireHello::new(config.bearer_token.clone(), config.client_name.clone());
    let payload = hello.encode()?;
    Ok(NveWireFrame::new(NveWireKind::Hello, 1, payload).encode()?)
}

pub(crate) fn decode_hello_response(
    frame: NveWireFrame,
) -> Result<NveWireHelloAck, NveWireTransportError> {
    match frame.kind {
        NveWireKind::HelloAck => Ok(NveWireHelloAck::decode(&frame.payload)?),
        NveWireKind::Reject => {
            let (reason, detail) = NveWireRejectReason::decode(&frame.payload)?;
            Err(NveWireTransportError::Auth(format!(
                "rejected ({reason:?}): {detail}"
            )))
        }
        other => Err(NveWireTransportError::Auth(format!(
            "unexpected handshake reply kind: {other:?}"
        ))),
    }
}
