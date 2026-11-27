//! Single-thread Tokio reconnecting wire client with exponential backoff (`100ms..=5s`),
//! automatic `KeyframeRequest` recovery on reconnect, and synchronized wake callbacks.
//!
//! Embedded directly inside `packages/core/runtime` (and reused by `hosts/skia-shell`) so every
//! native host shares one client transport without multi-thread Tokio pools or duplicated loops.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use nve_wire_core::{NveWireFrame, NveWireMetrics, NveWireTransportKind, NveWireUri};
use tokio::sync::{mpsc, Notify};

use crate::{NveWireClient, NveWireClientConfig, NveWireTransportError};

/// Callback fired on inbound frames and connection state transitions.
pub type WireWakeCallback = Arc<dyn Fn() + Send + Sync>;

/// Exponential backoff configuration for [`ReconnectingWireClient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectConfig {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
        }
    }
}

enum OutboundCmd {
    Events(Vec<u8>),
    RequestKeyframe,
}

struct SharedState {
    connected: Arc<AtomicBool>,
    transport: AtomicU8,
    session_id: AtomicU32,
    reconnect_count: AtomicU64,
    metrics: Mutex<NveWireMetrics>,
    base_metrics: Mutex<NveWireMetrics>,
    wake: RwLock<Option<WireWakeCallback>>,
    stop: AtomicBool,
    stop_notify: Notify,
}

impl SharedState {
    fn new(wake: Option<WireWakeCallback>) -> Self {
        Self {
            connected: Arc::new(AtomicBool::new(false)),
            transport: AtomicU8::new(encode_transport(NveWireTransportKind::Auto)),
            session_id: AtomicU32::new(0),
            reconnect_count: AtomicU64::new(0),
            metrics: Mutex::new(NveWireMetrics::default()),
            base_metrics: Mutex::new(NveWireMetrics::default()),
            wake: RwLock::new(wake),
            stop: AtomicBool::new(false),
            stop_notify: Notify::new(),
        }
    }

    fn fire_wake(&self) {
        let guard = self.wake.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref cb) = *guard {
            cb();
        }
    }

    fn update_metrics_from(&self, client: &NveWireClient) {
        let cur = client.metrics();
        let base = *self.base_metrics.lock().unwrap_or_else(|e| e.into_inner());
        let combined = NveWireMetrics {
            session_id: cur.session_id,
            frames_received: base.frames_received.saturating_add(cur.frames_received),
            keyframes_received: base.keyframes_received.saturating_add(cur.keyframes_received),
            command_deltas_received: base
                .command_deltas_received
                .saturating_add(cur.command_deltas_received),
            node_patch_deltas_received: base
                .node_patch_deltas_received
                .saturating_add(cur.node_patch_deltas_received),
            slot_chunks_received: base
                .slot_chunks_received
                .saturating_add(cur.slot_chunks_received),
            crdt_deltas_received: base
                .crdt_deltas_received
                .saturating_add(cur.crdt_deltas_received),
            presence_received: base.presence_received.saturating_add(cur.presence_received),
            events_sent: base.events_sent.saturating_add(cur.events_sent),
            wire_bytes_rx: base.wire_bytes_rx.saturating_add(cur.wire_bytes_rx),
            wire_bytes_tx: base.wire_bytes_tx.saturating_add(cur.wire_bytes_tx),
            last_sequence: if cur.last_sequence != 0 {
                cur.last_sequence
            } else {
                base.last_sequence
            },
        };
        *self.metrics.lock().unwrap_or_else(|e| e.into_inner()) = combined;
    }

    fn checkpoint_metrics(&self) {
        let cur = *self.metrics.lock().unwrap_or_else(|e| e.into_inner());
        *self.base_metrics.lock().unwrap_or_else(|e| e.into_inner()) = cur;
    }
}

/// Auto-reconnecting `nve-wire` client driven by ONE dedicated Tokio `current_thread` runtime.
pub struct ReconnectingWireClient {
    shared: Arc<SharedState>,
    inbound_rx: Receiver<NveWireFrame>,
    outbound_tx: Option<mpsc::UnboundedSender<OutboundCmd>>,
    worker: Option<JoinHandle<()>>,
}

impl ReconnectingWireClient {
    /// Spawns a non-blocking reconnecting client on a single dedicated `current_thread` Tokio
    /// runtime thread. Connects immediately and reconnects with exponential backoff on drops.
    pub fn spawn(
        uri: NveWireUri,
        client_cfg: NveWireClientConfig,
        reconnect_cfg: ReconnectConfig,
        wake: Option<WireWakeCallback>,
    ) -> Result<Self, NveWireTransportError> {
        let shared = Arc::new(SharedState::new(wake));
        let (inbound_tx, inbound_rx) = unbounded();
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();

        let w_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("nve-wire-client".to_owned())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                rt.block_on(reconnect_loop(
                    None,
                    uri,
                    client_cfg,
                    reconnect_cfg,
                    w_shared,
                    inbound_tx,
                    outbound_rx,
                ));
            })
            .map_err(|e| NveWireTransportError::Io(format!("spawn wire worker failed: {e}")))?;

        Ok(Self {
            shared,
            inbound_rx,
            outbound_tx: Some(outbound_tx),
            worker: Some(worker),
        })
    }

    /// Connects synchronously for the first handshake (propagating immediate auth/connect errors),
    /// then continues driving the live session and automatic exponential-backoff reconnects on one
    /// dedicated `current_thread` Tokio runtime thread.
    pub fn connect_sync(
        uri: NveWireUri,
        client_cfg: NveWireClientConfig,
        reconnect_cfg: ReconnectConfig,
        wake: Option<WireWakeCallback>,
    ) -> Result<Self, NveWireTransportError> {
        let shared = Arc::new(SharedState::new(wake));
        let (inbound_tx, inbound_rx) = unbounded();
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        let (init_tx, init_rx) = crossbeam_channel::bounded::<Result<(), NveWireTransportError>>(1);

        let w_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("nve-wire-client".to_owned())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = init_tx.send(Err(NveWireTransportError::Io(e.to_string())));
                        return;
                    }
                };
                rt.block_on(async move {
                    match NveWireClient::connect_uri_async(&uri, &client_cfg).await {
                        Ok(client) => {
                            w_shared
                                .transport
                                .store(encode_transport(client.transport_kind()), Ordering::Release);
                            w_shared
                                .session_id
                                .store(client.session_id(), Ordering::Release);
                            w_shared.connected.store(true, Ordering::Release);
                            w_shared.update_metrics_from(&client);
                            let _ = init_tx.send(Ok(()));
                            reconnect_loop(
                                Some(client),
                                uri,
                                client_cfg,
                                reconnect_cfg,
                                w_shared,
                                inbound_tx,
                                outbound_rx,
                            )
                            .await;
                        }
                        Err(e) => {
                            let _ = init_tx.send(Err(e));
                        }
                    }
                });
            })
            .map_err(|e| NveWireTransportError::Io(format!("spawn wire worker failed: {e}")))?;

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                shared,
                inbound_rx,
                outbound_tx: Some(outbound_tx),
                worker: Some(worker),
            }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                let _ = worker.join();
                Err(NveWireTransportError::Io(
                    "wire worker exited before handshake completed".to_owned(),
                ))
            }
        }
    }

    #[inline]
    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::Acquire)
    }

    /// Shared lock-free connection status flag (cross-thread safe for C ABI `pair_is_connected`).
    #[inline]
    pub fn connected_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shared.connected)
    }

    #[inline]
    pub fn transport_kind(&self) -> NveWireTransportKind {
        decode_transport(self.shared.transport.load(Ordering::Acquire))
    }

    #[inline]
    pub fn session_id(&self) -> u32 {
        self.shared.session_id.load(Ordering::Acquire)
    }

    #[inline]
    pub fn reconnect_count(&self) -> u64 {
        self.shared.reconnect_count.load(Ordering::Acquire)
    }

    #[inline]
    pub fn metrics(&self) -> NveWireMetrics {
        *self.shared.metrics.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[inline]
    pub fn has_pending_frame(&self) -> bool {
        !self.inbound_rx.is_empty()
    }

    #[inline]
    pub fn try_recv_frame(&self) -> Option<NveWireFrame> {
        self.inbound_rx.try_recv().ok()
    }

    #[inline]
    pub fn recv_frame_timeout(&self, timeout: Duration) -> Option<NveWireFrame> {
        self.inbound_rx.recv_timeout(timeout).ok()
    }

    pub fn send_events(&self, event_ring_bytes: &[u8]) -> Result<(), NveWireTransportError> {
        let Some(tx) = self.outbound_tx.as_ref() else {
            return Err(NveWireTransportError::Io("wire client shut down".to_owned()));
        };
        tx.send(OutboundCmd::Events(event_ring_bytes.to_vec()))
            .map_err(|_| NveWireTransportError::Io("wire worker channel closed".to_owned()))
    }

    pub fn request_keyframe(&self) -> Result<(), NveWireTransportError> {
        let Some(tx) = self.outbound_tx.as_ref() else {
            return Err(NveWireTransportError::Io("wire client shut down".to_owned()));
        };
        tx.send(OutboundCmd::RequestKeyframe)
            .map_err(|_| NveWireTransportError::Io("wire worker channel closed".to_owned()))
    }

    /// Installs or clears the wake callback, draining any in-flight invocation and dropping the
    /// replaced callback outside the lock.
    pub fn set_wake_callback(&self, cb: Option<WireWakeCallback>) {
        let old = {
            let mut guard = self.shared.wake.write().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *guard, cb)
        };
        drop(old);
    }

    /// Synchronously stops the worker thread and closes any active connection.
    pub fn shutdown(&mut self) {
        self.set_wake_callback(None);
        self.shared.stop.store(true, Ordering::Release);
        self.shared.stop_notify.notify_waiters();
        self.outbound_tx.take();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.shared.connected.store(false, Ordering::Release);
    }
}

impl Drop for ReconnectingWireClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn reconnect_loop(
    mut initial_client: Option<NveWireClient>,
    uri: NveWireUri,
    client_cfg: NveWireClientConfig,
    reconnect_cfg: ReconnectConfig,
    shared: Arc<SharedState>,
    inbound_tx: Sender<NveWireFrame>,
    mut outbound_rx: mpsc::UnboundedReceiver<OutboundCmd>,
) {
    let initial_backoff = reconnect_cfg.initial_backoff.max(Duration::from_millis(10));
    let max_backoff = reconnect_cfg.max_backoff.max(initial_backoff);
    let mut backoff = initial_backoff;
    let mut had_prior_session = initial_client.is_some();

    loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }

        let client = if let Some(c) = initial_client.take() {
            c
        } else {
            let stop_fut = shared.stop_notify.notified();
            tokio::pin!(stop_fut);
            stop_fut.as_mut().enable();
            if shared.stop.load(Ordering::Acquire) {
                break;
            }
            let conn_res = tokio::select! {
                biased;
                _ = &mut stop_fut => break,
                res = NveWireClient::connect_uri_async(&uri, &client_cfg) => res,
            };
            match conn_res {
                Ok(c) => {
                    backoff = initial_backoff;
                    shared
                        .transport
                        .store(encode_transport(c.transport_kind()), Ordering::Release);
                    shared.session_id.store(c.session_id(), Ordering::Release);
                    shared.connected.store(true, Ordering::Release);
                    if had_prior_session {
                        shared.reconnect_count.fetch_add(1, Ordering::Release);
                        let _ = c.request_keyframe();
                    }
                    had_prior_session = true;
                    shared.update_metrics_from(&c);
                    shared.fire_wake();
                    c
                }
                Err(_) => {
                    shared.connected.store(false, Ordering::Release);
                    let wait = backoff;
                    backoff = backoff.saturating_mul(2).min(max_backoff);
                    let stop_sleep = shared.stop_notify.notified();
                    tokio::pin!(stop_sleep);
                    stop_sleep.as_mut().enable();
                    if shared.stop.load(Ordering::Acquire) {
                        break;
                    }
                    tokio::select! {
                        biased;
                        _ = &mut stop_sleep => break,
                        _ = tokio::time::sleep(wait) => continue,
                    }
                }
            }
        };

        // Drive the active session purely event-driven until stopped or disconnected.
        let mut stopped = false;
        loop {
            let stop_fut = shared.stop_notify.notified();
            tokio::pin!(stop_fut);
            stop_fut.as_mut().enable();
            if shared.stop.load(Ordering::Acquire) {
                stopped = true;
                break;
            }
            tokio::select! {
                biased;
                _ = &mut stop_fut => {
                    stopped = true;
                    break;
                }
                cmd = outbound_rx.recv() => {
                    match cmd {
                        Some(OutboundCmd::Events(bytes)) => {
                            if client.send_events(&bytes).is_err() {
                                break;
                            }
                            shared.update_metrics_from(&client);
                        }
                        Some(OutboundCmd::RequestKeyframe) => {
                            if client.request_keyframe().is_err() {
                                break;
                            }
                            shared.update_metrics_from(&client);
                        }
                        None => {
                            stopped = true;
                            break;
                        }
                    }
                }
                maybe_frame = client.next_frame() => {
                    match maybe_frame {
                        Some(frame) => {
                            shared.update_metrics_from(&client);
                            if inbound_tx.send(frame).is_err() {
                                stopped = true;
                                break;
                            }
                            shared.fire_wake();
                        }
                        None => break,
                    }
                }
            }
        }

        shared.update_metrics_from(&client);
        shared.checkpoint_metrics();
        client.close();
        shared.connected.store(false, Ordering::Release);
        while outbound_rx.try_recv().is_ok() {}

        if stopped || shared.stop.load(Ordering::Acquire) {
            break;
        }
        shared.fire_wake();

        let wait = backoff;
        backoff = backoff.saturating_mul(2).min(max_backoff);
        let stop_sleep = shared.stop_notify.notified();
        tokio::pin!(stop_sleep);
        stop_sleep.as_mut().enable();
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        tokio::select! {
            biased;
            _ = &mut stop_sleep => break,
            _ = tokio::time::sleep(wait) => {}
        }
    }

    shared.connected.store(false, Ordering::Release);
}

const fn encode_transport(kind: NveWireTransportKind) -> u8 {
    match kind {
        NveWireTransportKind::Auto => 0,
        NveWireTransportKind::Quic => 1,
        NveWireTransportKind::WebTransport => 2,
        NveWireTransportKind::Wss => 3,
        NveWireTransportKind::Ws => 4,
    }
}

const fn decode_transport(raw: u8) -> NveWireTransportKind {
    match raw {
        1 => NveWireTransportKind::Quic,
        2 => NveWireTransportKind::WebTransport,
        3 => NveWireTransportKind::Wss,
        4 => NveWireTransportKind::Ws,
        _ => NveWireTransportKind::Auto,
    }
}
