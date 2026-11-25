//! `nve pair` — Live `NVD1` binary delta device pairing server over BOTH UDP and TCP/WebSocket
//! with zero-JSON bidirectional `InputEvents` ingestion back into the live app instance.

use std::borrow::Cow;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use nve_abi::{
    decode_events, encode_events, EventPayload, NativeEvent, NativeEventKind, NveMapEntry,
    NveValue, ROOT_NODE_ID,
};
use nve_dioxus::DioxusSurface;
use nve_engine::loader::HotAppInstance;
use nve_interop::{fnv1a_64, PairingConfig, TcpDeltaServer, UdpDeltaServer};

use crate::dev::dylib_path_for_package;

static TOKEN_SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) struct PairOptions {
    pub dylib: Option<PathBuf>,
    pub package: Option<String>,
    pub host: String,
    pub udp_port: u16,
    pub ws_port: u16,
    pub token: Option<String>,
    pub width: f32,
    pub height: f32,
    pub fps: u32,
}

#[derive(Debug, Clone)]
pub(crate) enum PairControlMsg {
    /// Binary `NVP1` hot-patch frame, forwarded unchanged to `NveDevApi::apply_hot_patch`.
    Tier1Patch(Vec<u8>),
    DylibReload(PathBuf),
    Shutdown,
}

/// Runtime abstraction over a live NVE app hosted by [`PairSession`].
pub(crate) trait PairedApp {
    fn render(&mut self, frame_id: u64) -> Cow<'_, [u8]>;
    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool;
    fn set_props_direct(&mut self, _entries: &[NveMapEntry]) -> bool {
        false
    }
    fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool {
        false
    }
    fn reload(&mut self, _dylib_path: &Path) -> Result<u64, String> {
        Ok(1)
    }
}

impl PairedApp for HotAppInstance {
    fn render(&mut self, frame_id: u64) -> Cow<'_, [u8]> {
        Cow::Borrowed(HotAppInstance::render(self, frame_id))
    }
    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        HotAppInstance::dispatch_events(self, encoded_events)
    }
    fn set_props_direct(&mut self, entries: &[NveMapEntry]) -> bool {
        HotAppInstance::set_props_direct(self, entries)
    }
    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        HotAppInstance::apply_hot_patch(self, patch_nvp1)
    }
    fn reload(&mut self, dylib_path: &Path) -> Result<u64, String> {
        HotAppInstance::reload(self, dylib_path)
            .map(|o| o.generation)
            .map_err(|e| format!("cdylib hot-swap failed: {e}"))
    }
}

impl PairedApp for DioxusSurface {
    fn render(&mut self, frame_id: u64) -> Cow<'_, [u8]> {
        Cow::Owned(self.render_frame(frame_id).2)
    }
    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        decode_events(encoded_events)
            .map(|events| DioxusSurface::dispatch_events(self, &events))
            .unwrap_or(false)
    }
}

pub(crate) fn generate_pairing_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = u64::from(std::process::id());
    let mut seed = Vec::with_capacity(32);
    seed.extend_from_slice(&nanos.to_le_bytes());
    seed.extend_from_slice(&pid.to_le_bytes());
    seed.extend_from_slice(&seq.to_le_bytes());
    let h1 = fnv1a_64(&seed);
    seed.reverse();
    let h2 = fnv1a_64(&seed);
    format!("nve-{h1:08x}-{:08x}", (h2 ^ (h2 >> 32)) as u32)
}

pub(crate) fn resolve_or_generate_token(token: Option<String>) -> String {
    match token {
        Some(t) if !t.trim().is_empty() => t.trim().to_owned(),
        _ => generate_pairing_token(),
    }
}

pub(crate) fn format_bind_addr(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

pub(crate) fn display_host_for_uri(host: &str) -> &str {
    if host == "0.0.0.0" || host == "::" || host == "[::]" {
        "127.0.0.1"
    } else {
        host
    }
}

pub(crate) fn format_ws_url(host: &str, ws_port: u16) -> String {
    let disp = display_host_for_uri(host);
    if disp.contains(':') && !disp.starts_with('[') {
        format!("ws://[{disp}]:{ws_port}")
    } else {
        format!("ws://{disp}:{ws_port}")
    }
}

pub(crate) fn format_pairing_uri(
    host: &str,
    udp_port: u16,
    ws_port: u16,
    token: &str,
    width: f32,
    height: f32,
) -> String {
    let h = display_host_for_uri(host);
    format!(
        "nve://pair?host={h}&udp={udp_port}&ws={ws_port}&token={token}&w={}&h={}",
        width.round() as u32,
        height.round() as u32
    )
}

pub(crate) fn format_browser_pair_query(host: &str, ws_port: u16, token: &str) -> String {
    format!("?pair={}&token={token}", format_ws_url(host, ws_port))
}

pub(crate) fn format_pair_ready_line(
    host: &str,
    udp_addr: SocketAddr,
    ws_addr: SocketAddr,
    token: &str,
) -> String {
    let ws_url = format_ws_url(host, ws_addr.port());
    format!("NVE_PAIR_READY ws={ws_url} token={token} udp={udp_addr} tcp={ws_addr}")
}

pub(crate) fn format_pair_banner(
    dylib_path: &Path,
    host: &str,
    udp_addr: SocketAddr,
    ws_addr: SocketAddr,
    token: &str,
    width: f32,
    height: f32,
) -> String {
    let ws_url = format_ws_url(host, ws_addr.port());
    let uri = format_pairing_uri(host, udp_addr.port(), ws_addr.port(), token, width, height);
    let browser_query = format_browser_pair_query(host, ws_addr.port(), token);
    format!(
        "nve pair — Live Binary NVD1 Device Pairing\n  dylib:   {}\n  udp:     {udp_addr}\n  tcp/ws:  {ws_url}/ws/nvd (bound {ws_addr})\n  token:   {token}\n  uri:     {uri}\n  browser: {browser_query}\n",
        dylib_path.display(),
    )
}

pub(crate) fn resolve_dylib_path(
    dylib: Option<PathBuf>,
    package: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(path) = dylib {
        return if path.exists() {
            Ok(path)
        } else {
            Err(format!("specified dylib does not exist: {}", path.display()))
        };
    }
    let pkg = package.unwrap_or("nve-studio-workspace");
    let path = dylib_path_for_package(pkg);
    if !path.exists() {
        println!("[pair] building {pkg} -> {}...", path.display());
        let status = ProcessCommand::new("cargo")
            .args(["build", "-p", pkg])
            .status()
            .map_err(|e| format!("failed to invoke cargo build -p {pkg}: {e}"))?;
        if !status.success() {
            return Err(format!("cargo build -p {pkg} failed"));
        }
    }
    if path.exists() {
        Ok(path)
    } else {
        Err(format!("expected cdylib at {} after building {pkg}", path.display()))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PairStepOutcome {
    pub events_dispatched: usize,
    pub frames_broadcast: usize,
    pub shutdown_requested: bool,
}

/// Quantized snapshot of live `nve pair` transport metrics matching Studio's Live Sync card
/// (`syncTransport`, `syncPeers`, `syncKbps`, `syncCompression`, `syncRttMs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SyncTelemetrySnapshot {
    pub transport: &'static str,
    pub peers: u16,
    pub kbps_tenths: u32,
    pub compression_tenths: u16,
    pub rtt_ms_tenths: u32,
}

impl SyncTelemetrySnapshot {
    fn from_metrics(
        udp: &nve_interop::PairingMetrics,
        tcp: &nve_interop::PairingMetrics,
    ) -> Self {
        let peers = udp
            .connected_peers
            .saturating_add(tcp.connected_peers)
            .min(u16::MAX as usize) as u16;
        let transport = if udp.connected_peers > 0 {
            "lan-quic"
        } else if tcp.connected_peers > 0 {
            "relay-wss"
        } else {
            "offline"
        };

        let total_kbps = (udp.tx_kbps() + tcp.tx_kbps() + udp.rx_kbps() + tcp.rx_kbps()).max(0.0);
        let kbps_tenths = if total_kbps.is_finite() {
            (total_kbps * 10.0).round() as u32
        } else {
            0
        };

        let total_raw = udp.raw_bytes_sent.saturating_add(tcp.raw_bytes_sent);
        let total_wire = udp.wire_bytes_sent.saturating_add(tcp.wire_bytes_sent);
        let ratio = if total_wire == 0 {
            1.0
        } else {
            (total_raw as f64 / total_wire as f64).max(0.1)
        };
        let compression_tenths = if ratio.is_finite() {
            ((ratio * 10.0).round() as u32).min(u16::MAX as u32) as u16
        } else {
            10
        };

        let udp_rtt = udp.smoothed_rtt_us.or(udp.last_rtt_us);
        let tcp_rtt = tcp.smoothed_rtt_us.or(tcp.last_rtt_us);
        let avg_rtt_us = match (udp_rtt, tcp_rtt) {
            (Some(a), Some(b)) => (a + b) / 2,
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => 0,
        };
        let rtt_ms_tenths = ((avg_rtt_us as f64 / 1000.0) * 10.0).round() as u32;

        Self {
            transport,
            peers,
            kbps_tenths,
            compression_tenths,
            rtt_ms_tenths,
        }
    }

    fn as_prop_entries(&self) -> [NveMapEntry; 5] {
        [
            NveMapEntry::new("syncTransport", NveValue::from_str(self.transport)),
            NveMapEntry::new("syncPeers", NveValue::i64(i64::from(self.peers))),
            NveMapEntry::new("syncKbps", NveValue::f64(f64::from(self.kbps_tenths) / 10.0)),
            NveMapEntry::new(
                "syncCompression",
                NveValue::f64(f64::from(self.compression_tenths) / 10.0),
            ),
            NveMapEntry::new(
                "syncRttMs",
                NveValue::f64(f64::from(self.rtt_ms_tenths) / 10.0),
            ),
        ]
    }
}

/// Minimum interval between steady-state bandwidth/RTT prop refreshes (peer/transport changes
/// always push immediately).
const TELEMETRY_PUSH_INTERVAL: Duration = Duration::from_millis(250);

/// Live dual-transport (`UdpDeltaServer` + `TcpDeltaServer`) pairing session owning a [`PairedApp`].
pub(crate) struct PairSession<A: PairedApp> {
    app: A,
    udp: UdpDeltaServer,
    tcp: TcpDeltaServer,
    frame_id: u64,
    last_frame: Vec<u8>,
    token: String,
    last_telemetry: Option<SyncTelemetrySnapshot>,
    last_telemetry_push: Instant,
}

impl<A: PairedApp> PairSession<A> {
    pub(crate) fn start(
        mut app: A,
        host: &str,
        udp_port: u16,
        ws_port: u16,
        token: String,
        width: f32,
        height: f32,
    ) -> Result<Self, String> {
        let cfg = PairingConfig::new(token.clone());
        let udp_bind = format_bind_addr(host, udp_port);
        let ws_bind = format_bind_addr(host, ws_port);
        let udp = UdpDeltaServer::bind(&udp_bind, cfg.clone())
            .map_err(|e| format!("bind UDP pairing server on {udp_bind}: {e}"))?;
        let tcp = TcpDeltaServer::bind(&ws_bind, cfg)
            .map_err(|e| format!("bind TCP/WS pairing server on {ws_bind}: {e}"))?;

        if width > 0.0 && height > 0.0 {
            let layout_ev = encode_events(&[NativeEvent {
                surface_id: 1,
                node_id: ROOT_NODE_ID,
                timestamp_us: 0,
                kind: NativeEventKind::Layout,
                payload: EventPayload::Layout { x: 0.0, y: 0.0, width, height },
            }]);
            let _ = app.dispatch_events(&layout_ev);
        }

        let mut session = Self {
            app,
            udp,
            tcp,
            frame_id: 0,
            last_frame: Vec::new(),
            token,
            last_telemetry: None,
            last_telemetry_push: Instant::now(),
        };
        let _ = session.push_telemetry_if_needed(true);
        let _ = session.render_and_broadcast(true)?;
        Ok(session)
    }

    pub(crate) fn udp_addr(&self) -> SocketAddr { self.udp.local_addr() }
    pub(crate) fn ws_addr(&self) -> SocketAddr { self.tcp.local_addr() }
    #[cfg(test)]
    pub(crate) fn token(&self) -> &str { &self.token }

    /// Computes live combined UDP + TCP/WS pairing telemetry and pushes it via zero-copy
    /// `set_props_direct` into the hosted app whenever peers/transport change or metrics drift.
    pub(crate) fn push_telemetry_if_needed(&mut self, force: bool) -> bool {
        let udp_m = self.udp.metrics();
        let tcp_m = self.tcp.metrics();
        let snapshot = SyncTelemetrySnapshot::from_metrics(&udp_m, &tcp_m);

        let peer_or_transport_changed = self.last_telemetry.is_none_or(|prev| {
            prev.peers != snapshot.peers || prev.transport != snapshot.transport
        });
        let metric_changed = self.last_telemetry != Some(snapshot);

        if !force
            && !peer_or_transport_changed
            && (!metric_changed || self.last_telemetry_push.elapsed() < TELEMETRY_PUSH_INTERVAL)
        {
            return false;
        }

        self.last_telemetry = Some(snapshot);
        self.last_telemetry_push = Instant::now();
        let entries = snapshot.as_prop_entries();
        self.app.set_props_direct(&entries)
    }

    pub(crate) fn render_and_broadcast(&mut self, force: bool) -> Result<bool, String> {
        self.frame_id += 1;
        let frame = self.app.render(self.frame_id);
        if frame.is_empty() || (!force && frame.as_ref() == self.last_frame.as_slice()) {
            return Ok(false);
        }
        self.udp.send_frame(&frame).map_err(|e| format!("UDP send_frame failed: {e}"))?;
        self.tcp.send_frame(&frame).map_err(|e| format!("TCP/WS send_frame failed: {e}"))?;
        match frame {
            Cow::Borrowed(slice) => {
                self.last_frame.clear();
                self.last_frame.extend_from_slice(slice);
            }
            Cow::Owned(vec) => {
                self.last_frame = vec;
            }
        }
        Ok(true)
    }

    pub(crate) fn poll_once(
        &mut self,
        control_rx: Option<&Receiver<PairControlMsg>>,
    ) -> PairStepOutcome {
        let mut outcome = PairStepOutcome::default();
        if let Some(rx) = control_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    PairControlMsg::Tier1Patch(patch) => {
                        if self.app.apply_hot_patch(&patch)
                            && self.render_and_broadcast(true).unwrap_or(false)
                        {
                            outcome.frames_broadcast += 1;
                        }
                    }
                    PairControlMsg::DylibReload(path) => match self.app.reload(&path) {
                        Ok(_) if self.render_and_broadcast(true).unwrap_or(false) => {
                            outcome.frames_broadcast += 1;
                        }
                        Ok(_) => {}
                        Err(err) => eprintln!("[pair] reload error: {err}"),
                    },
                    PairControlMsg::Shutdown => {
                        outcome.shutdown_requested = true;
                        return outcome;
                    }
                }
            }
        }
        let mut incoming = Vec::new();
        while let Some(ev) = self.udp.try_recv_events() { incoming.push(ev); }
        while let Some(ev) = self.tcp.try_recv_events() { incoming.push(ev); }
        for (_peer_sid, encoded_ring) in incoming {
            let handled = self.app.dispatch_events(&encoded_ring);
            outcome.events_dispatched += 1;
            if self.render_and_broadcast(handled).unwrap_or(false) {
                outcome.frames_broadcast += 1;
            }
        }
        if self.push_telemetry_if_needed(false)
            && self.render_and_broadcast(true).unwrap_or(false)
        {
            outcome.frames_broadcast += 1;
        }
        outcome
    }

    pub(crate) fn run_until(
        &mut self,
        fps: u32,
        shutdown: &AtomicBool,
        control_rx: Option<&Receiver<PairControlMsg>>,
    ) {
        let idle_sleep = Duration::from_millis((1000 / fps.clamp(1, 240) as u64).clamp(1, 4));
        while !shutdown.load(Ordering::Relaxed) {
            let step = self.poll_once(control_rx);
            if step.shutdown_requested {
                break;
            }
            if step.events_dispatched == 0 && step.frames_broadcast == 0 {
                std::thread::sleep(idle_sleep);
            }
        }
        let _ = &self.token;
        self.udp.shutdown();
        self.tcp.shutdown();
    }
}

/// Spawns a dedicated thread owning the `!Send` [`HotAppInstance`] and dual UDP + TCP/WS servers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_dylib_pair_thread(
    dylib_path: PathBuf,
    host: String,
    udp_port: u16,
    ws_port: u16,
    token: String,
    width: f32,
    height: f32,
    fps: u32,
) -> Result<(Sender<PairControlMsg>, String), String> {
    let (ctrl_tx, ctrl_rx) = unbounded();
    let (ready_tx, ready_rx) = bounded::<Result<String, String>>(1);

    std::thread::Builder::new()
        .name("nve-cli-pair-session".into())
        .spawn(move || {
            let app = match HotAppInstance::load(&dylib_path, 1, 1, None) {
                Ok(a) => a,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("load {}: {e}", dylib_path.display())));
                    return;
                }
            };
            let mut session = match PairSession::start(app, &host, udp_port, ws_port, token.clone(), width, height) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let banner = format_pair_banner(
                &dylib_path, &host, session.udp_addr(), session.ws_addr(), &token, width, height,
            );
            let _ = ready_tx.send(Ok(banner));
            session.run_until(fps, &AtomicBool::new(false), Some(&ctrl_rx));
        })
        .map_err(|e| format!("spawn pairing thread: {e}"))?;

    let banner = ready_rx
        .recv()
        .map_err(|_| "pairing thread exited before startup".to_owned())??;
    Ok((ctrl_tx, banner))
}

pub(crate) fn cmd_pair(opts: PairOptions) {
    let dylib_path = resolve_dylib_path(opts.dylib, opts.package.as_deref()).unwrap_or_else(|e| {
        eprintln!("pair failed: {e}");
        std::process::exit(1);
    });
    let token = resolve_or_generate_token(opts.token);
    let app = HotAppInstance::load(&dylib_path, 1, 1, None).unwrap_or_else(|e| {
        eprintln!("failed to load {}: {e}", dylib_path.display());
        std::process::exit(1);
    });
    let mut session = PairSession::start(
        app, &opts.host, opts.udp_port, opts.ws_port, token.clone(), opts.width, opts.height,
    )
    .unwrap_or_else(|e| {
        eprintln!("failed to start pairing server: {e}");
        std::process::exit(1);
    });

    print!(
        "{}",
        format_pair_banner(
            &dylib_path,
            &opts.host,
            session.udp_addr(),
            session.ws_addr(),
            &token,
            opts.width,
            opts.height,
        )
    );
    println!(
        "{}",
        format_pair_ready_line(&opts.host, session.udp_addr(), session.ws_addr(), &token)
    );
    println!("Streaming live NVD1 frames & listening for remote InputEvents (Ctrl-C to stop)...");
    let _ = std::io::stdout().flush();
    session.run_until(opts.fps, &AtomicBool::new(false), None);
}

#[cfg(test)]
#[path = "pair_tests.rs"]
mod tests;
