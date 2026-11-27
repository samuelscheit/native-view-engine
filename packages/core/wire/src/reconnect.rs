//! Single-thread Tokio reconnecting wire client with bounded channels, monotonic session tagging,
//! terminal auth rejection, exponential backoff (`100ms..=5s`), and synchronized wake callbacks.
//!
//! Embedded directly inside `packages/core/runtime` (and reused by `hosts/skia-shell`) so every
//! native host shares one client transport without multi-thread Tokio pools or duplicated loops.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver};
use nve_wire_core::{NveWireFrame, NveWireMetrics, NveWireTransportKind, NveWireUri};
use tokio::sync::{mpsc, Notify};

use crate::reconnect_loop::{activate_session, reconnect_loop, OutboundCmd};
use crate::{NveWireClient, NveWireClientConfig, NveWireTransportError};

/// Callback fired on inbound frames and connection state transitions.
pub type WireWakeCallback = Arc<dyn Fn() + Send + Sync>;

/// Bounded capacity for inbound items exposed by [`ReconnectingWireClient`].
pub(crate) const RECONNECT_INBOUND_CAP: usize = 256;
/// Bounded capacity for outbound commands queued into [`ReconnectingWireClient`].
pub(crate) const RECONNECT_OUTBOUND_CAP: usize = 256;

/// Inbound event emitted by [`ReconnectingWireClient`] in delivery order.
#[derive(Debug, Clone)]
pub enum WireInboundItem {
    /// A transport connection was established under monotonic local `session_id` (`1, 2, ...`).
    SessionStarted { session_id: u64, reconnected: bool },
    /// An `NveWireFrame` received on `session_id`.
    Frame {
        session_id: u64,
        frame: NveWireFrame,
    },
    /// The bounded inbound queue overflowed on `session_id`; stale deltas were dropped and
    /// consumers must reset / wait for a fresh `SceneKeyframe` (or `frame == 1`).
    Overflowed { session_id: u64 },
}

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

pub(crate) struct SharedState {
    pub(crate) connected: Arc<AtomicBool>,
    pub(crate) transport: AtomicU8,
    pub(crate) server_session_id: AtomicU32,
    pub(crate) active_session_id: AtomicU64,
    pub(crate) next_session_seq: AtomicU64,
    pub(crate) reconnect_count: AtomicU64,
    pub(crate) auth_rejections: AtomicU64,
    pub(crate) dropped_outbound: AtomicU64,
    pub(crate) dropped_inbound: AtomicU64,
    base_client_dropped_inbound: AtomicU64,
    cur_client_dropped_inbound: AtomicU64,
    pub(crate) last_error: Mutex<Option<String>>,
    metrics: Mutex<NveWireMetrics>,
    base_metrics: Mutex<NveWireMetrics>,
    wake: RwLock<Option<WireWakeCallback>>,
    pub(crate) stop: AtomicBool,
    pub(crate) stop_notify: Notify,
}

impl SharedState {
    fn new(wake: Option<WireWakeCallback>, connected: Arc<AtomicBool>) -> Self {
        connected.store(false, Ordering::Release);
        Self {
            connected,
            transport: AtomicU8::new(NveWireTransportKind::Auto as u8),
            server_session_id: AtomicU32::new(0),
            active_session_id: AtomicU64::new(0),
            next_session_seq: AtomicU64::new(1),
            reconnect_count: AtomicU64::new(0),
            auth_rejections: AtomicU64::new(0),
            dropped_outbound: AtomicU64::new(0),
            dropped_inbound: AtomicU64::new(0),
            base_client_dropped_inbound: AtomicU64::new(0),
            cur_client_dropped_inbound: AtomicU64::new(0),
            last_error: Mutex::new(None),
            metrics: Mutex::new(NveWireMetrics::default()),
            base_metrics: Mutex::new(NveWireMetrics::default()),
            wake: RwLock::new(wake),
            stop: AtomicBool::new(false),
            stop_notify: Notify::new(),
        }
    }

    pub(crate) fn fire_wake(&self) {
        let guard = self.wake.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref cb) = *guard {
            cb();
        }
    }

    pub(crate) fn set_last_error(&self, err: Option<String>) {
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = err;
    }

    pub(crate) fn update_metrics_from(&self, client: &NveWireClient) {
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
        self.cur_client_dropped_inbound
            .store(client.dropped_inbound(), Ordering::Release);
    }

    pub(crate) fn checkpoint_metrics(&self) {
        let cur = *self.metrics.lock().unwrap_or_else(|e| e.into_inner());
        *self.base_metrics.lock().unwrap_or_else(|e| e.into_inner()) = cur;
        // Increment `base` BEFORE zeroing `cur` (paired with `cur` then `base` Acquire loads in
        // `dropped_inbound()`) so readers never observe a downward dip across reconnects.
        let cur_dropped = self.cur_client_dropped_inbound.load(Ordering::Acquire);
        self.base_client_dropped_inbound
            .fetch_add(cur_dropped, Ordering::Release);
        self.cur_client_dropped_inbound.store(0, Ordering::Release);
    }
}

/// Auto-reconnecting `nve-wire` client driven by ONE dedicated Tokio `current_thread` runtime.
pub struct ReconnectingWireClient {
    shared: Arc<SharedState>,
    inbound_rx: Receiver<WireInboundItem>,
    outbound_tx: Option<mpsc::Sender<OutboundCmd>>,
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
        Self::spawn_with_connected_flag(
            uri,
            client_cfg,
            reconnect_cfg,
            wake,
            Arc::new(AtomicBool::new(false)),
        )
    }

    /// Same as [`Self::spawn`], storing live connection state in a caller-owned `Arc<AtomicBool>`.
    pub fn spawn_with_connected_flag(
        uri: NveWireUri,
        client_cfg: NveWireClientConfig,
        reconnect_cfg: ReconnectConfig,
        wake: Option<WireWakeCallback>,
        connected_flag: Arc<AtomicBool>,
    ) -> Result<Self, NveWireTransportError> {
        Self::start_worker(uri, client_cfg, reconnect_cfg, wake, connected_flag, false)
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
        Self::start_worker(
            uri,
            client_cfg,
            reconnect_cfg,
            wake,
            Arc::new(AtomicBool::new(false)),
            true,
        )
    }

    fn start_worker(
        uri: NveWireUri,
        client_cfg: NveWireClientConfig,
        reconnect_cfg: ReconnectConfig,
        wake: Option<WireWakeCallback>,
        connected_flag: Arc<AtomicBool>,
        wait_initial: bool,
    ) -> Result<Self, NveWireTransportError> {
        let shared = Arc::new(SharedState::new(wake, connected_flag));
        let (inbound_tx, inbound_rx) = bounded(RECONNECT_INBOUND_CAP);
        let inbound_drain = inbound_rx.clone();
        let (outbound_tx, outbound_rx) = mpsc::channel(RECONNECT_OUTBOUND_CAP);
        let (init_tx, init_rx) = if wait_initial {
            let (tx, rx) = bounded::<Result<(), NveWireTransportError>>(1);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

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
                        if let Some(tx) = init_tx {
                            let _ = tx.send(Err(NveWireTransportError::Io(e.to_string())));
                        }
                        return;
                    }
                };
                rt.block_on(async move {
                    let initial = if let Some(tx) = init_tx {
                        match NveWireClient::connect_uri_async(&uri, &client_cfg).await {
                            Ok(client) => {
                                let sid = activate_session(
                                    &client,
                                    false,
                                    &w_shared,
                                    &inbound_tx,
                                    &inbound_drain,
                                );
                                let _ = tx.send(Ok(()));
                                Some((client, sid))
                            }
                            Err(e) => {
                                w_shared.set_last_error(Some(e.to_string()));
                                if matches!(e, NveWireTransportError::Auth(_)) {
                                    w_shared.auth_rejections.fetch_add(1, Ordering::Release);
                                }
                                let _ = tx.send(Err(e));
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    reconnect_loop(
                        initial,
                        uri,
                        client_cfg,
                        reconnect_cfg,
                        w_shared,
                        inbound_tx,
                        inbound_drain,
                        outbound_rx,
                    )
                    .await;
                });
                // Never stall `handle.join()` on in-flight blocking DNS lookup tasks.
                rt.shutdown_background();
            })
            .map_err(|e| NveWireTransportError::Io(format!("spawn wire worker failed: {e}")))?;

        if let Some(init_rx) = init_rx {
            match init_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let _ = worker.join();
                    return Err(e);
                }
                Err(_) => {
                    let _ = worker.join();
                    return Err(NveWireTransportError::Io(
                        "wire worker exited before handshake completed".to_owned(),
                    ));
                }
            }
        }

        Ok(Self {
            shared,
            inbound_rx,
            outbound_tx: Some(outbound_tx),
            worker: Some(worker),
        })
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
        NveWireTransportKind::from_u8(self.shared.transport.load(Ordering::Acquire))
    }

    /// Protocol `HelloAck.session_id` assigned by the server (`0` before first handshake).
    #[inline]
    pub fn session_id(&self) -> u32 {
        self.shared.server_session_id.load(Ordering::Acquire)
    }

    /// Monotonic client-local connection epoch (`0` when disconnected, `1, 2, ...` when connected).
    #[inline]
    pub fn active_session_id(&self) -> u64 {
        self.shared.active_session_id.load(Ordering::Acquire)
    }

    #[inline]
    pub fn reconnect_count(&self) -> u64 {
        self.shared.reconnect_count.load(Ordering::Acquire)
    }

    #[inline]
    pub fn auth_rejections(&self) -> u64 {
        self.shared.auth_rejections.load(Ordering::Acquire)
    }

    #[inline]
    pub fn dropped_outbound(&self) -> u64 {
        self.shared.dropped_outbound.load(Ordering::Acquire)
    }

    #[inline]
    pub fn dropped_inbound(&self) -> u64 {
        // Load `cur` BEFORE `base` to pair with `checkpoint_metrics` (`base += cur` then `cur = 0`).
        let cur = self
            .shared
            .cur_client_dropped_inbound
            .load(Ordering::Acquire);
        let base = self
            .shared
            .base_client_dropped_inbound
            .load(Ordering::Acquire);
        self.shared
            .dropped_inbound
            .load(Ordering::Acquire)
            .saturating_add(base)
            .saturating_add(cur)
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    #[inline]
    pub fn metrics(&self) -> NveWireMetrics {
        *self.shared.metrics.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[inline]
    pub fn has_pending_item(&self) -> bool {
        !self.inbound_rx.is_empty()
    }

    #[inline]
    pub fn try_recv_item(&self) -> Option<WireInboundItem> {
        self.inbound_rx.try_recv().ok()
    }

    #[inline]
    pub fn recv_item_timeout(&self, timeout: Duration) -> Option<WireInboundItem> {
        self.inbound_rx.recv_timeout(timeout).ok()
    }

    pub fn try_recv_frame(&self) -> Option<NveWireFrame> {
        while let Ok(item) = self.inbound_rx.try_recv() {
            if let WireInboundItem::Frame { frame, .. } = item {
                return Some(frame);
            }
        }
        None
    }

    pub fn recv_frame_timeout(&self, timeout: Duration) -> Option<NveWireFrame> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            match self.inbound_rx.recv_timeout(deadline - now) {
                Ok(WireInboundItem::Frame { frame, .. }) => return Some(frame),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    /// Queues an upstream `EventBatch` tagged with the active session ID.
    ///
    /// Immediately rejects and counts drops when disconnected or when the bounded outbound queue
    /// is full so stale pre-connect / offline events are never queued for a later session.
    pub fn send_events(
        &self,
        event_ring_bytes: impl Into<Vec<u8>>,
    ) -> Result<(), NveWireTransportError> {
        let sid = self.shared.active_session_id.load(Ordering::Acquire);
        if sid == 0 || !self.is_connected() {
            self.shared.dropped_outbound.fetch_add(1, Ordering::Relaxed);
            return Err(NveWireTransportError::Io(
                "wire client not connected".to_owned(),
            ));
        }
        let Some(tx) = self.outbound_tx.as_ref() else {
            self.shared.dropped_outbound.fetch_add(1, Ordering::Relaxed);
            return Err(NveWireTransportError::Io("wire client shut down".to_owned()));
        };
        tx.try_send(OutboundCmd::Events {
            session_id: sid,
            bytes: event_ring_bytes.into(),
        })
        .map_err(|e| {
            self.shared.dropped_outbound.fetch_add(1, Ordering::Relaxed);
            match e {
                mpsc::error::TrySendError::Full(_) => {
                    NveWireTransportError::Backpressure("wire client outbound queue full".to_owned())
                }
                mpsc::error::TrySendError::Closed(_) => {
                    NveWireTransportError::Io("wire worker channel closed".to_owned())
                }
            }
        })
    }

    pub fn request_keyframe(&self) -> Result<(), NveWireTransportError> {
        let sid = self.shared.active_session_id.load(Ordering::Acquire);
        if sid == 0 || !self.is_connected() {
            return Err(NveWireTransportError::Io(
                "wire client not connected".to_owned(),
            ));
        }
        let Some(tx) = self.outbound_tx.as_ref() else {
            return Err(NveWireTransportError::Io("wire client shut down".to_owned()));
        };
        tx.try_send(OutboundCmd::RequestKeyframe { session_id: sid })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => {
                    NveWireTransportError::Backpressure("wire client outbound queue full".to_owned())
                }
                mpsc::error::TrySendError::Closed(_) => {
                    NveWireTransportError::Io("wire worker channel unavailable".to_owned())
                }
            })
    }

    /// Installs or clears the wake callback, draining any in-flight invocation, dropping the
    /// replaced callback outside the lock, and firing under the shared read lock if inbound items
    /// or terminal auth errors were already pending when installed (mirroring `WakeBridge::set_sink`).
    pub fn set_wake_callback(&self, cb: Option<WireWakeCallback>) {
        let (old, should_fire) = {
            let mut guard = self.shared.wake.write().unwrap_or_else(|e| e.into_inner());
            let old = std::mem::replace(&mut *guard, cb);
            let pending = guard.is_some()
                && (!self.inbound_rx.is_empty()
                    || self.shared.auth_rejections.load(Ordering::Acquire) > 0);
            (old, pending)
        };
        drop(old);
        if should_fire {
            self.shared.fire_wake();
        }
    }

    /// Synchronously stops the worker thread and gracefully closes any active connection.
    pub fn shutdown(&mut self) {
        self.set_wake_callback(None);
        self.shared.stop.store(true, Ordering::Release);
        self.shared.stop_notify.notify_waiters();
        self.outbound_tx.take();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.shared.active_session_id.store(0, Ordering::Release);
        self.shared.connected.store(false, Ordering::Release);
    }
}

impl Drop for ReconnectingWireClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}
