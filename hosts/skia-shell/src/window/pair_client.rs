//! Live `NVT1`/`NVD1` thin-client pairing adapter (`PairedRemoteApp`) connecting `nve-skia-shell`
//! directly to a remote `nve pair` host over UDP, raw framed TCP, or RFC 6455 binary WebSocket
//! with zero local `cdylib` required.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};
use nve_abi::{FrameId, SurfaceId};
use nve_interop::{PairingConfig, PairingMetrics, TcpDeltaClient, UdpDeltaClient};

use super::{DesktopApp, DesktopHostError};

/// Default WebSocket upgrade path served by `nve pair`.
pub const DEFAULT_PAIR_WS_PATH: &str = "/ws/nvd";

/// Transport protocol used by `--pair-connect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairTransport {
    /// Unreliable UDP with `NVT1` fragmentation + NACK/retransmit reliability.
    Udp,
    /// Raw 4-byte length-prefixed binary TCP stream.
    Tcp,
    /// RFC 6455 binary WebSocket frames carrying `NVT1`/`NVD1` packets.
    #[default]
    Ws,
}

impl PairTransport {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "udp" => Ok(Self::Udp),
            "tcp" => Ok(Self::Tcp),
            "ws" | "websocket" | "wss" => Ok(Self::Ws),
            other => Err(format!(
                "invalid --pair-transport '{other}' (expected one of: udp, tcp, ws)"
            )),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::Tcp => "tcp",
            Self::Ws => "ws",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::Tcp => "TCP",
            Self::Ws => "WS",
        }
    }
}

/// Connection parameters for live thin-client pairing mode (`--pair-connect`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairConnectConfig {
    /// Target host:port or scheme URL (`127.0.0.1:8092`, `ws://127.0.0.1:8092/ws/nvd`, etc.).
    pub addr: String,
    /// Shared pairing token printed by `nve pair`.
    pub token: String,
    /// Wire transport protocol (`udp`, `tcp`, or `ws`).
    pub transport: PairTransport,
    /// WebSocket upgrade path when using [`PairTransport::Ws`].
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
            ws_path: DEFAULT_PAIR_WS_PATH.to_string(),
            initial_frame_timeout: Duration::from_secs(5),
        }
    }

    /// Parses optional URL scheme/path and resolves the remote [`SocketAddr`].
    pub fn resolve_endpoint(&self) -> Result<(SocketAddr, String), DesktopHostError> {
        let trimmed = self.addr.trim();
        if trimmed.is_empty() {
            return Err(DesktopHostError::Pairing(
                "pair endpoint address cannot be empty".to_string(),
            ));
        }

        let without_scheme = if let Some(rest) = trimmed.strip_prefix("ws://") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("wss://") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("tcp://") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("udp://") {
            rest
        } else {
            trimmed
        };

        let (host_port, path) = match without_scheme.find('/') {
            Some(idx) => {
                let hp = &without_scheme[..idx];
                let p = &without_scheme[idx..];
                (hp, p.to_string())
            }
            None => (without_scheme, self.ws_path.clone()),
        };

        let mut addrs = host_port.to_socket_addrs().map_err(|e| {
            DesktopHostError::Pairing(format!("failed to resolve pair endpoint '{host_port}': {e}"))
        })?;
        let sock_addr = addrs.next().ok_or_else(|| {
            DesktopHostError::Pairing(format!("no socket addresses resolved for '{host_port}'"))
        })?;

        let normalized_path = if path.is_empty() {
            DEFAULT_PAIR_WS_PATH.to_string()
        } else if path.starts_with('/') {
            path
        } else {
            format!("/{path}")
        };
        Ok((sock_addr, normalized_path))
    }
}

enum LiveDeltaClient {
    Udp(UdpDeltaClient),
    Tcp(TcpDeltaClient),
}

impl LiveDeltaClient {
    fn session_id(&self) -> u32 {
        match self {
            Self::Udp(c) => c.session_id(),
            Self::Tcp(c) => c.session_id(),
        }
    }

    fn is_connected(&self) -> bool {
        match self {
            Self::Udp(c) => c.is_connected(),
            Self::Tcp(c) => c.is_connected(),
        }
    }

    fn recv_frame(
        &self,
        timeout: Duration,
    ) -> Result<Option<(u32, Vec<u8>)>, nve_interop::TransportError> {
        match self {
            Self::Udp(c) => c.recv_frame(timeout),
            Self::Tcp(c) => c.recv_frame(timeout),
        }
    }

    fn try_recv_frame(&self) -> Option<(u32, Vec<u8>)> {
        match self {
            Self::Udp(c) => c.try_recv_frame(),
            Self::Tcp(c) => c.try_recv_frame(),
        }
    }

    fn send_events(&self, encoded_event_ring: &[u8]) -> Result<(), nve_interop::TransportError> {
        match self {
            Self::Udp(c) => c.send_events(encoded_event_ring),
            Self::Tcp(c) => c.send_events(encoded_event_ring),
        }
    }

    fn metrics(&self) -> PairingMetrics {
        match self {
            Self::Udp(c) => c.metrics(),
            Self::Tcp(c) => c.metrics(),
        }
    }

    fn shutdown(&mut self) {
        match self {
            Self::Udp(c) => c.shutdown(),
            Self::Tcp(c) => c.shutdown(),
        }
    }
}

type WakeFn = Arc<dyn Fn() + Send + Sync>;

/// Live thin-client [`DesktopApp`] backed by `UdpDeltaClient` or `TcpDeltaClient` (`Tcp`/`Ws`).
///
/// A dedicated background worker thread receives decoded `NVD1` `CommandBuffer` frames (waking
/// the `winit` event loop immediately via the installed wake callback) and flushes local binary
/// `EventRing` batches upstream over `TransportMsgKind::InputEvents = 11`.
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
    metrics: Arc<Mutex<PairingMetrics>>,
    wake_cb: Arc<Mutex<Option<WakeFn>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PairedRemoteApp {
    /// Connects to a remote `nve pair` server over the configured transport and spawns the live
    /// frame-reader / event-sender background thread.
    pub fn connect(config: PairConnectConfig) -> Result<Self, DesktopHostError> {
        if config.token.trim().is_empty() {
            return Err(DesktopHostError::Pairing(
                "missing --pair-token for --pair-connect".to_string(),
            ));
        }

        let (sock_addr, ws_path) = config.resolve_endpoint()?;
        let pairing_cfg = PairingConfig::new(config.token.clone());
        let client = match config.transport {
            PairTransport::Udp => {
                let c = UdpDeltaClient::connect(sock_addr, pairing_cfg).map_err(|e| {
                    DesktopHostError::Pairing(format!("UDP pair connect to {sock_addr} failed: {e}"))
                })?;
                LiveDeltaClient::Udp(c)
            }
            PairTransport::Tcp => {
                let c = TcpDeltaClient::connect(sock_addr, pairing_cfg).map_err(|e| {
                    DesktopHostError::Pairing(format!("TCP pair connect to {sock_addr} failed: {e}"))
                })?;
                LiveDeltaClient::Tcp(c)
            }
            PairTransport::Ws => {
                let c =
                    TcpDeltaClient::connect_websocket(sock_addr, &ws_path, pairing_cfg).map_err(
                        |e| {
                            DesktopHostError::Pairing(format!(
                                "WebSocket pair connect to ws://{sock_addr}{ws_path} failed: {e}"
                            ))
                        },
                    )?;
                LiveDeltaClient::Tcp(c)
            }
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
            .name("nve-skia-pair-client".to_string())
            .spawn(move || {
                pair_worker_loop(
                    client,
                    frame_tx,
                    event_rx,
                    w_frames,
                    w_seq,
                    w_conn,
                    w_metrics,
                    w_wake,
                    w_stop,
                );
            })
            .map_err(|e| {
                DesktopHostError::Pairing(format!("failed to spawn pairing worker thread: {e}"))
            })?;

        Ok(Self {
            transport: config.transport,
            endpoint_label: sock_addr.to_string(),
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

    pub fn metrics(&self) -> PairingMetrics {
        self.metrics.lock().unwrap().clone()
    }

    /// Blocks up to `timeout` for the next decoded `CommandBuffer` frame from the server,
    /// coalescing any newer already-buffered frames so callers always render the freshest state.
    pub fn wait_for_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<(u32, Vec<u8>), DesktopHostError> {
        let first = self.frame_rx.recv_timeout(timeout).map_err(|e| {
            DesktopHostError::Pairing(format!(
                "timed out waiting for paired NVD1 frame from {} ({e})",
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

    /// Drains any newly arrived frames into `latest_frame` without blocking.
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

    /// Explicitly shuts down the pairing client and joins its background thread.
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

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        if encoded_events.is_empty() || !self.connected.load(Ordering::Relaxed) {
            return false;
        }
        self.event_tx.send(encoded_events.to_vec()).is_ok()
    }

    fn apply_hot_patch(&mut self, _patch_json: &str) -> bool {
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
        let rtt_part = m
            .smoothed_rtt_us
            .or(m.last_rtt_us)
            .map(|us| format!(" · {:.1}ms", us as f64 / 1000.0))
            .unwrap_or_default();
        Some(format!(
            "Paired {} [{}] · seq #{} · {:.1} KB/s{rtt_part}",
            self.transport.label(),
            state,
            self.last_seq(),
            m.rx_kbps()
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn pair_worker_loop(
    mut client: LiveDeltaClient,
    frame_tx: Sender<(u32, Vec<u8>)>,
    event_rx: Receiver<Vec<u8>>,
    frames_received: Arc<AtomicU64>,
    last_seq: Arc<AtomicU64>,
    connected: Arc<AtomicBool>,
    metrics: Arc<Mutex<PairingMetrics>>,
    wake_cb: Arc<Mutex<Option<WakeFn>>>,
    stop: Arc<AtomicBool>,
) {
    let mut last_metrics_refresh = Instant::now();
    let poll_timeout = Duration::from_millis(2);

    while !stop.load(Ordering::Relaxed) {
        // 1. Flush all pending upstream InputEvents immediately.
        let mut flushed_event = false;
        while let Ok(encoded_ring) = event_rx.try_recv() {
            if client.send_events(&encoded_ring).is_ok() {
                flushed_event = true;
            }
        }

        // 2. Receive incoming decoded NVD1 CommandBuffer frames.
        let mut received_any = false;
        match client.recv_frame(poll_timeout) {
            Ok(Some((seq, frame))) => {
                frames_received.fetch_add(1, Ordering::Relaxed);
                last_seq.store(u64::from(seq), Ordering::Relaxed);
                if frame_tx.send((seq, frame)).is_err() {
                    break;
                }
                received_any = true;
                while let Some((seq2, frame2)) = client.try_recv_frame() {
                    frames_received.fetch_add(1, Ordering::Relaxed);
                    last_seq.store(u64::from(seq2), Ordering::Relaxed);
                    if frame_tx.send((seq2, frame2)).is_err() {
                        break;
                    }
                }
            }
            Ok(None) => {}
            Err(_) => {
                connected.store(false, Ordering::Relaxed);
                break;
            }
        }

        // 3. Refresh telemetry snapshot and wake the UI event loop when frames arrive.
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

    client.shutdown();
    connected.store(false, Ordering::Relaxed);
}
