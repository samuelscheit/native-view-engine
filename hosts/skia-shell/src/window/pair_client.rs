//! Live `NveWireFrame` v2 thin-client pairing adapter (`PairedRemoteApp`) connecting
//! `nve-skia-shell` directly to a remote `nve pair` host over QUIC (`nve-wire-2`), HTTP/3
//! WebTransport (`h3`), TLS WebSocket (`wss://`), or Happy-Eyeballs auto-fallback.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};
use nve_abi::{direct_event_to_native, encode_events, FrameId, SurfaceId};
use nve_wire::{
    parse_wire_uri, NveWireClient, NveWireClientConfig, NveWireKind, NveWireMetrics,
    NveWireTransportKind, NveWireUri, DEFAULT_WIRE_WSS_PATH,
};

use super::{DesktopApp, DesktopHostError};

/// Default WebSocket / WebTransport path served by `nve pair`.
pub const DEFAULT_PAIR_WS_PATH: &str = DEFAULT_WIRE_WSS_PATH;

/// Transport protocol used by `--pair-connect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairTransport {
    /// Happy-Eyeballs: QUIC first, racing `wss://` after 250ms.
    #[default]
    Auto,
    /// Native raw QUIC (`nve-wire-2` ALPN).
    Quic,
    /// HTTP/3 WebTransport (`h3` ALPN).
    WebTransport,
    /// TLS WebSocket (`wss://`).
    Wss,
    /// Localhost plain WebSocket (`ws://`).
    Ws,
}

impl PairTransport {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "quic" | "udp" => Ok(Self::Quic),
            "wt" | "webtransport" | "h3" => Ok(Self::WebTransport),
            "wss" | "tls" => Ok(Self::Wss),
            "ws" | "tcp" | "websocket" => Ok(Self::Ws),
            other => Err(format!(
                "invalid --pair-transport '{other}' (expected one of: auto, quic, webtransport, wss, ws)"
            )),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Quic => "quic",
            Self::WebTransport => "webtransport",
            Self::Wss => "wss",
            Self::Ws => "ws",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Quic => "QUIC",
            Self::WebTransport => "WT/H3",
            Self::Wss => "WSS",
            Self::Ws => "WS",
        }
    }

    fn to_wire_kind(self) -> NveWireTransportKind {
        match self {
            Self::Auto => NveWireTransportKind::Auto,
            Self::Quic => NveWireTransportKind::Quic,
            Self::WebTransport => NveWireTransportKind::WebTransport,
            Self::Wss => NveWireTransportKind::Wss,
            Self::Ws => NveWireTransportKind::Ws,
        }
    }
}

/// Connection parameters for live thin-client pairing mode (`--pair-connect`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairConnectConfig {
    /// Target `nve://pair?...` URI, `quic://`, `https://`, `wss://`, `ws://`, or `host:port`.
    pub addr: String,
    /// Shared Bearer token printed by `nve pair`.
    pub token: String,
    /// Wire transport protocol (`auto`, `quic`, `webtransport`, `wss`, or `ws`).
    pub transport: PairTransport,
    /// Optional pinned SHA-256 leaf certificate digest (auto-parsed from `nve://pair?...`).
    pub cert_sha256: Option<[u8; 32]>,
    /// WebSocket / WebTransport path.
    pub ws_path: String,
    /// Maximum time to wait for Frame 1 on initial connect / `--screenshot`.
    pub initial_frame_timeout: Duration,
}

impl PairConnectConfig {
    pub fn new(
        addr: impl Into<String>,
        token: impl Into<String>,
        transport: PairTransport,
    ) -> Self {
        Self {
            addr: addr.into(),
            token: token.into(),
            transport,
            cert_sha256: None,
            ws_path: DEFAULT_PAIR_WS_PATH.to_string(),
            initial_frame_timeout: Duration::from_secs(5),
        }
    }

    pub fn with_cert_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.cert_sha256 = Some(sha256);
        self
    }

    pub fn resolve_uri(&self) -> Result<NveWireUri, DesktopHostError> {
        let mut uri = parse_wire_uri(&self.addr).map_err(|e| {
            DesktopHostError::Pairing(format!("invalid pair endpoint '{}': {e}", self.addr))
        })?;
        if self.transport != PairTransport::Auto {
            uri.transport = self.transport.to_wire_kind();
        }
        if !self.token.trim().is_empty() {
            uri.token = self.token.clone();
        }
        if let Some(hash) = self.cert_sha256 {
            uri.cert_sha256 = Some(hash);
        }
        if uri.path.is_empty() {
            uri.path = self.ws_path.clone();
        }
        Ok(uri)
    }
}

type WakeFn = Arc<dyn Fn() + Send + Sync>;

/// Live thin-client [`DesktopApp`] backed by `nve_wire::NveWireClient` (QUIC / WebTransport / WSS).
pub struct PairedRemoteApp {
    transport: PairTransport,
    endpoint_label: String,
    session_id: u32,
    frame_rx: Receiver<(u32, Vec<u8>)>,
    event_tx: Sender<Vec<u8>>,
    latest_frame: Vec<u8>,
    has_unconsumed_frame: bool,
    frames_received: Arc<AtomicU64>,
    last_seq: Arc<AtomicU64>,
    connected: Arc<AtomicBool>,
    metrics: Arc<Mutex<NveWireMetrics>>,
    wake_cb: Arc<Mutex<Option<WakeFn>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PairedRemoteApp {
    /// Connects to a remote `nve pair` server over QUIC/WebTransport/WSS and spawns the reader loop.
    pub fn connect(config: PairConnectConfig) -> Result<Self, DesktopHostError> {
        let uri = config.resolve_uri()?;
        if uri.token.trim().is_empty() {
            return Err(DesktopHostError::Pairing(
                "missing --pair-token for --pair-connect".to_string(),
            ));
        }

        let mut wire_cfg = NveWireClientConfig::new(uri.token.clone(), "nve-skia-shell");
        wire_cfg.cert_sha256 = uri.cert_sha256;

        let client = NveWireClient::connect_uri_sync(&uri, &wire_cfg).map_err(|e| {
            DesktopHostError::Pairing(format!("wire pair connect to {} failed: {e}", config.addr))
        })?;

        let active_transport = match client.transport_kind() {
            NveWireTransportKind::Auto => PairTransport::Auto,
            NveWireTransportKind::Quic => PairTransport::Quic,
            NveWireTransportKind::WebTransport => PairTransport::WebTransport,
            NveWireTransportKind::Wss => PairTransport::Wss,
            NveWireTransportKind::Ws => PairTransport::Ws,
        };
        let session_id = client.session_id();
        let initial_metrics = client.metrics();
        let (frame_tx, frame_rx) = unbounded::<(u32, Vec<u8>)>();
        let (event_tx, event_rx) = unbounded::<Vec<u8>>();
        let frames_received = Arc::new(AtomicU64::new(0));
        let last_seq = Arc::new(AtomicU64::new(0));
        let connected = Arc::new(AtomicBool::new(client.is_connected()));
        let metrics = Arc::new(Mutex::new(initial_metrics));
        let wake_cb: Arc<Mutex<Option<WakeFn>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));

        let w_frames = Arc::clone(&frames_received);
        let w_seq = Arc::clone(&last_seq);
        let w_conn = Arc::clone(&connected);
        let w_metrics = Arc::clone(&metrics);
        let w_wake = Arc::clone(&wake_cb);
        let w_stop = Arc::clone(&stop);

        let worker = thread::Builder::new()
            .name("nve-skia-wire-client".to_string())
            .spawn(move || {
                pair_worker_loop(
                    client, frame_tx, event_rx, w_frames, w_seq, w_conn, w_metrics, w_wake, w_stop,
                );
            })
            .map_err(|e| {
                DesktopHostError::Pairing(format!("failed to spawn pairing worker thread: {e}"))
            })?;

        Ok(Self {
            transport: active_transport,
            endpoint_label: config.addr,
            session_id,
            frame_rx,
            event_tx,
            latest_frame: Vec::new(),
            has_unconsumed_frame: false,
            frames_received,
            last_seq,
            connected,
            metrics,
            wake_cb,
            stop,
            worker: Some(worker),
        })
    }

    pub fn transport(&self) -> PairTransport {
        self.transport
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn frames_received(&self) -> u64 {
        self.frames_received.load(Ordering::Relaxed)
    }

    pub fn last_seq(&self) -> u32 {
        self.last_seq.load(Ordering::Relaxed) as u32
    }

    pub fn metrics(&self) -> NveWireMetrics {
        *self.metrics.lock().unwrap()
    }

    pub fn wait_for_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<(u32, Vec<u8>), DesktopHostError> {
        let first = self.frame_rx.recv_timeout(timeout).map_err(|e| {
            DesktopHostError::Pairing(format!(
                "timed out waiting for paired wire frame from {} ({e})",
                self.endpoint_label
            ))
        })?;
        let mut latest = first;
        while let Ok(newer) = self.frame_rx.try_recv() {
            latest = newer;
        }
        self.latest_frame = latest.1.clone();
        self.has_unconsumed_frame = true;
        Ok(latest)
    }

    fn drain_incoming_frames(&mut self) -> bool {
        let mut got_new = false;
        while let Ok((_seq, bytes)) = self.frame_rx.try_recv() {
            if !bytes.is_empty() {
                self.latest_frame = bytes;
                self.has_unconsumed_frame = true;
                got_new = true;
            }
        }
        got_new
    }

    pub fn disconnect(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.connected.store(false, Ordering::Relaxed);
    }
}

impl Drop for PairedRemoteApp {
    fn drop(&mut self) {
        self.disconnect();
    }
}

impl DesktopApp for PairedRemoteApp {
    fn generation(&self) -> u64 {
        self.frames_received.load(Ordering::Relaxed).max(1)
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        self.drain_incoming_frames();
        if self.has_unconsumed_frame {
            self.has_unconsumed_frame = false;
            &self.latest_frame
        } else {
            &[]
        }
    }

    fn dispatch_events(&mut self, events: nve_abi::NveEventBatchRef<'_>) -> bool {
        if events.is_empty() || !self.connected.load(Ordering::Relaxed) {
            return false;
        }
        let owned: Vec<_> = events.iter().filter_map(direct_event_to_native).collect();
        if owned.is_empty() {
            return false;
        }
        self.event_tx.send(encode_events(&owned)).is_ok()
    }

    fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool {
        false
    }

    fn reload(
        &mut self,
        _new_dylib_path: &Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        Ok(self.generation())
    }

    fn install_wake_callback(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake_cb.lock().unwrap() = Some(wake);
    }

    fn has_pending_remote_frame(&self) -> bool {
        self.has_unconsumed_frame || !self.frame_rx.is_empty()
    }

    fn remote_status_banner(&self) -> Option<String> {
        let m = self.metrics();
        let state = if self.is_connected() {
            "LIVE"
        } else {
            "DISCONNECTED"
        };
        Some(format!(
            "Paired {} [{}] · seq #{} · {} B rx",
            self.transport.label(),
            state,
            self.last_seq(),
            m.wire_bytes_rx
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn pair_worker_loop(
    client: NveWireClient,
    frame_tx: Sender<(u32, Vec<u8>)>,
    event_rx: Receiver<Vec<u8>>,
    frames_received: Arc<AtomicU64>,
    last_seq: Arc<AtomicU64>,
    connected: Arc<AtomicBool>,
    metrics: Arc<Mutex<NveWireMetrics>>,
    wake_cb: Arc<Mutex<Option<WakeFn>>>,
    stop: Arc<AtomicBool>,
) {
    let mut last_metrics_refresh = Instant::now();
    let poll_timeout = Duration::from_millis(2);

    while !stop.load(Ordering::Relaxed) {
        let mut flushed_event = false;
        while let Ok(encoded_ring) = event_rx.try_recv() {
            if client.send_events(&encoded_ring).is_ok() {
                flushed_event = true;
            }
        }

        let mut received_any = false;
        if let Some(frame) = client.recv_frame_timeout(poll_timeout) {
            if matches!(
                frame.kind,
                NveWireKind::CommandDelta | NveWireKind::SceneKeyframe
            ) {
                frames_received.fetch_add(1, Ordering::Relaxed);
                last_seq.store(u64::from(frame.sequence), Ordering::Relaxed);
                if frame_tx.send((frame.sequence, frame.payload)).is_err() {
                    break;
                }
                received_any = true;
            }
            while let Some(f2) = client.try_recv_frame() {
                if matches!(f2.kind, NveWireKind::CommandDelta | NveWireKind::SceneKeyframe) {
                    frames_received.fetch_add(1, Ordering::Relaxed);
                    last_seq.store(u64::from(f2.sequence), Ordering::Relaxed);
                    if frame_tx.send((f2.sequence, f2.payload)).is_err() {
                        break;
                    }
                    received_any = true;
                }
            }
        } else if !client.is_connected() {
            connected.store(false, Ordering::Relaxed);
            break;
        }

        let now = Instant::now();
        if received_any
            || flushed_event
            || now.saturating_duration_since(last_metrics_refresh) >= Duration::from_millis(100)
        {
            last_metrics_refresh = now;
            connected.store(client.is_connected(), Ordering::Relaxed);
            *metrics.lock().unwrap() = client.metrics();
        }

        if received_any
            && let Some(cb) = wake_cb.lock().unwrap().as_ref().cloned()
        {
            cb();
        }
    }

    client.close();
    connected.store(false, Ordering::Relaxed);
}
