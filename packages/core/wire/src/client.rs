//! Unified `NveWireClient` supporting raw QUIC (`nve-wire-2`), HTTP/3 WebTransport (`h3`),
//! TLS WebSocket (`wss://`), and Happy-Eyeballs racing (QUIC first, 250ms WSS fallback).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use nve_wire_core::{
    NveWireChannel, NveWireFrame, NveWireHello, NveWireHelloAck, NveWireKind, NveWireMetrics,
    NveWireRejectReason, NveWireTransportKind, NVE_WIRE_FLAG_DATAGRAM,
};

use crate::framing::{read_stream_frame, write_stream_raw};
use crate::tls::build_quic_client_config;
use crate::{NveWireClientConfig, NveWireTransportError};

/// Bounded capacity for inbound frames queued per transport client.
pub(crate) const CLIENT_INBOUND_CAP: usize = 256;
/// Bounded capacity for outbound frames queued per transport client.
pub(crate) const CLIENT_OUTBOUND_CAP: usize = 256;

pub(crate) enum ClientOut {
    Stream(Bytes),
    Datagram(Bytes),
    Close,
}

pub(crate) enum ClientInboundEvent {
    Frame(NveWireFrame),
    Overflowed,
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
    pub(crate) inbound_overflow: AtomicBool,
    pub(crate) dropped_inbound: AtomicU64,
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
            inbound_overflow: AtomicBool::new(false),
            dropped_inbound: AtomicU64::new(0),
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

/// Pushes an inbound frame into the bounded client queue; on overflow, drains stale deltas, sets
/// `inbound_overflow` so the consumer triggers keyframe resync, and preserves `SceneKeyframe`.
pub(crate) fn push_inbound(
    tx: &Sender<NveWireFrame>,
    drain_rx: &Receiver<NveWireFrame>,
    frame: NveWireFrame,
    stats: &ClientStats,
    notify: &tokio::sync::Notify,
) {
    stats.record_inbound(&frame);
    match tx.try_send(frame) {
        Ok(()) => {}
        Err(TrySendError::Full(f)) => {
            let mut dropped = 0u64;
            while drain_rx.try_recv().is_ok() {
                dropped += 1;
            }
            stats.inbound_overflow.store(true, Ordering::Release);
            if f.kind == NveWireKind::SceneKeyframe {
                if tx.try_send(f).is_err() {
                    dropped += 1;
                }
            } else {
                dropped += 1;
            }
            stats.dropped_inbound.fetch_add(dropped, Ordering::Relaxed);
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
    notify.notify_waiters();
}

/// Connected `nve-wire` client (QUIC, WebTransport, WSS, or Happy-Eyeballs).
pub struct NveWireClient {
    session_id: u32,
    transport_kind: NveWireTransportKind,
    outbound_tx: mpsc::Sender<ClientOut>,
    inbound_rx: Receiver<NveWireFrame>,
    inbound_notify: Arc<tokio::sync::Notify>,
    writer_done: Arc<tokio::sync::Notify>,
    writer_exited: Arc<AtomicBool>,
    stats: Arc<ClientStats>,
    next_sequence: AtomicU32,
    closed: AtomicBool,
    task_aborts: Mutex<Vec<AbortHandle>>,
    _runtime: Option<Arc<tokio::runtime::Runtime>>,
}

impl NveWireClient {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        session_id: u32,
        transport_kind: NveWireTransportKind,
        outbound_tx: mpsc::Sender<ClientOut>,
        inbound_rx: Receiver<NveWireFrame>,
        inbound_notify: Arc<tokio::sync::Notify>,
        writer_done: Arc<tokio::sync::Notify>,
        writer_exited: Arc<AtomicBool>,
        stats: Arc<ClientStats>,
        task_aborts: Vec<AbortHandle>,
    ) -> Self {
        Self {
            session_id,
            transport_kind,
            outbound_tx,
            inbound_rx,
            inbound_notify,
            writer_done,
            writer_exited,
            stats,
            next_sequence: AtomicU32::new(2),
            closed: AtomicBool::new(false),
            task_aborts: Mutex::new(task_aborts),
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
            SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0))
        } else {
            SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, 0))
        };
        let udp = std::net::UdpSocket::bind(bind_addr)
            .map_err(|e| NveWireTransportError::Io(e.to_string()))?;
        let ep_cfg = web_transport_quinn::crypto::endpoint_config()
            .map_err(|e| NveWireTransportError::Tls(e.to_string()))?;
        let mut endpoint = quinn::Endpoint::new(ep_cfg, None, udp, Arc::new(quinn::TokioRuntime))
            .map_err(|e| NveWireTransportError::Io(e.to_string()))?;
        endpoint.set_default_client_config(client_cfg);

        let handshake_fut = async {
            let conn = endpoint
                .connect(addr, &config.server_name)
                .map_err(|e| NveWireTransportError::Connect(e.to_string()))?
                .await
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
            Ok((conn, send, recv, ack))
        };
        let (conn, mut send, mut recv, ack) =
            tokio::time::timeout(config.connect_timeout, handshake_fut)
                .await
                .map_err(|_| NveWireTransportError::Connect("QUIC handshake timed out".to_owned()))??;

        let (inbound_tx, inbound_rx) = bounded(CLIENT_INBOUND_CAP);
        let inbound_notify = Arc::new(tokio::sync::Notify::new());
        let writer_done = Arc::new(tokio::sync::Notify::new());
        let writer_exited = Arc::new(AtomicBool::new(false));
        let (outbound_tx, mut out_rx) = mpsc::channel(CLIENT_OUTBOUND_CAP);
        let stats = Arc::new(ClientStats::new());

        let conn_write = conn.clone();
        let stats_w = stats.clone();
        let notify_w = inbound_notify.clone();
        let done_w = writer_done.clone();
        let exited_w = writer_exited.clone();
        let writer_jh = tokio::spawn(async move {
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
                        let _ = send.finish();
                        conn_write.close(0u32.into(), b"bye");
                        break;
                    }
                }
            }
            stats_w.connected.store(false, Ordering::Release);
            exited_w.store(true, Ordering::Release);
            done_w.notify_waiters();
            notify_w.notify_waiters();
        });

        let conn_dgram = conn.clone();
        let max_len = config.max_frame_bytes;
        let in_tx_d = inbound_tx.clone();
        let drain_d = inbound_rx.clone();
        let notify_d = inbound_notify.clone();
        let stats_d = stats.clone();
        let dgram_jh = tokio::spawn(async move {
            while let Ok(raw) = conn_dgram.read_datagram().await {
                if let Ok(frame) = NveWireFrame::decode(&raw, max_len) {
                    push_inbound(&in_tx_d, &drain_d, frame, &stats_d, &notify_d);
                }
            }
        });

        let drain_r = inbound_rx.clone();
        let stats_r = stats.clone();
        let notify_r = inbound_notify.clone();
        let reader_jh = tokio::spawn(async move {
            while let Ok(Some(frame)) = read_stream_frame(&mut recv, max_len).await {
                push_inbound(&inbound_tx, &drain_r, frame, &stats_r, &notify_r);
            }
            stats_r.connected.store(false, Ordering::Release);
            notify_r.notify_waiters();
        });

        Ok(Self::from_parts(
            ack.session_id,
            NveWireTransportKind::Quic,
            outbound_tx,
            inbound_rx,
            inbound_notify,
            writer_done,
            writer_exited,
            stats,
            vec![
                writer_jh.abort_handle(),
                dgram_jh.abort_handle(),
                reader_jh.abort_handle(),
            ],
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
                Err(e @ NveWireTransportError::Auth(_)) => Err(e),
                Err(_) => wss_fut.await,
            },
            w_res = &mut wss_fut => match w_res {
                Ok(c) => Ok(c),
                Err(e @ NveWireTransportError::Auth(_)) => Err(e),
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
        self.stats.connected.load(Ordering::Acquire)
    }

    #[inline]
    pub fn metrics(&self) -> NveWireMetrics {
        self.stats.snapshot(self.session_id)
    }

    fn alloc_seq(&self) -> u32 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed)
    }

    fn send_outbound(&self, cmd: ClientOut, byte_len: u64) -> Result<(), NveWireTransportError> {
        self.outbound_tx.try_send(cmd).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                NveWireTransportError::Io("wire client outbound queue full".to_owned())
            }
            mpsc::error::TrySendError::Closed(_) => {
                NveWireTransportError::Io("wire client outbound closed".to_owned())
            }
        })?;
        self.stats
            .wire_bytes_tx
            .fetch_add(byte_len, Ordering::Relaxed);
        Ok(())
    }

    pub fn send_frame(&self, frame: &NveWireFrame) -> Result<(), NveWireTransportError> {
        let bytes = Bytes::from(frame.encode()?);
        let len = bytes.len() as u64;
        self.send_outbound(ClientOut::Stream(bytes), len)
    }

    pub fn send_datagram(&self, frame: &NveWireFrame) -> Result<(), NveWireTransportError> {
        let mut df = frame.clone();
        df.flags |= NVE_WIRE_FLAG_DATAGRAM;
        let bytes = Bytes::from(df.encode()?);
        let len = bytes.len() as u64;
        self.send_outbound(ClientOut::Datagram(bytes), len)
    }

    pub fn send_events(
        &self,
        event_ring_bytes: impl Into<Vec<u8>>,
    ) -> Result<(), NveWireTransportError> {
        let frame = NveWireFrame::new(
            NveWireKind::EventBatch,
            self.alloc_seq(),
            event_ring_bytes.into(),
        )
        .with_channel(NveWireChannel::InputEvents);
        self.send_frame(&frame)?;
        self.stats.events_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
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

    pub(crate) async fn next_inbound_event(&self) -> Option<ClientInboundEvent> {
        loop {
            let notified = self.inbound_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.stats.inbound_overflow.swap(false, Ordering::AcqRel) {
                return Some(ClientInboundEvent::Overflowed);
            }
            if let Ok(frame) = self.inbound_rx.try_recv() {
                return Some(ClientInboundEvent::Frame(frame));
            }
            if !self.stats.connected.load(Ordering::Acquire) {
                if self.stats.inbound_overflow.swap(false, Ordering::AcqRel) {
                    return Some(ClientInboundEvent::Overflowed);
                }
                return self.inbound_rx.try_recv().ok().map(ClientInboundEvent::Frame);
            }
            notified.await;
        }
    }

    pub async fn next_frame(&self) -> Option<NveWireFrame> {
        loop {
            match self.next_inbound_event().await? {
                ClientInboundEvent::Frame(f) => return Some(f),
                ClientInboundEvent::Overflowed => continue,
            }
        }
    }

    pub async fn recv_frame_async(&self, timeout: Duration) -> Option<NveWireFrame> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.inbound_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Ok(frame) = self.inbound_rx.try_recv() {
                return Some(frame);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.inbound_rx.try_recv().ok();
            }
        }
    }

    fn abort_tasks(&self) {
        let mut aborts = self.task_aborts.lock().unwrap_or_else(|e| e.into_inner());
        for h in aborts.drain(..) {
            h.abort();
        }
    }

    /// Sends a single graceful `Bye` + `Close` and waits up to `flush_timeout` for the writer
    /// task to flush before aborting all remaining transport tasks.
    pub async fn close_graceful(&self, flush_timeout: Duration) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stats.connected.store(false, Ordering::Release);
        let done = self.writer_done.notified();
        tokio::pin!(done);
        done.as_mut().enable();
        let deadline = tokio::time::Instant::now() + flush_timeout;
        let bye = NveWireFrame::new(NveWireKind::Bye, self.alloc_seq(), Vec::new());
        if let Ok(encoded) = bye.encode() {
            let _ = tokio::time::timeout_at(
                deadline,
                self.outbound_tx.send(ClientOut::Stream(Bytes::from(encoded))),
            )
            .await;
        }
        let close_queued = matches!(
            tokio::time::timeout_at(deadline, self.outbound_tx.send(ClientOut::Close)).await,
            Ok(Ok(()))
        );
        if close_queued && !self.writer_exited.load(Ordering::Acquire) {
            let _ = tokio::time::timeout_at(deadline, done).await;
        }
        self.abort_tasks();
        self.inbound_notify.notify_waiters();
    }

    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stats.connected.store(false, Ordering::Release);
        let bye = NveWireFrame::new(NveWireKind::Bye, self.alloc_seq(), Vec::new());
        if let Ok(encoded) = bye.encode() {
            let _ = self
                .outbound_tx
                .try_send(ClientOut::Stream(Bytes::from(encoded)));
        }
        let _ = self.outbound_tx.try_send(ClientOut::Close);
        self.abort_tasks();
        self.inbound_notify.notify_waiters();
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
