//! `nve pair` — Unified dual-ALPN QUIC (`nve-wire-2`) + HTTP/3 WebTransport (`h3`) + TLS
//! WebSocket (`wss://`) device pairing server with 13-day ECDSA P-256 cert pinning and
//! binary bidirectional `EventBatch` ingestion back into the live app instance.

use std::borrow::Cow;
use std::io::Write as _;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use nve_abi::{
    decode_events, native_events_to_direct, NveDirectEvent, NveMapEntry, NveValue, ROOT_NODE_ID,
};
use nve_dioxus::DioxusSurface;
use nve_engine::loader::HotAppInstance;
use nve_wire::{
    format_wire_uri, generate_bearer_token, NveWireKind, NveWireServer, NveWireServerConfig,
    NveWireServerMetrics,
};

use crate::dev::dylib_path_for_package;

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
    Tier1Patch(Vec<u8>),
    DylibReload(PathBuf),
    Shutdown,
}

/// Runtime abstraction over a live NVE app hosted by [`PairSession`].
pub(crate) trait PairedApp {
    fn render(&mut self, frame_id: u64) -> Cow<'_, [u8]>;
    fn dispatch_events(&mut self, events: &[NveDirectEvent]) -> bool;
    fn set_props(&mut self, _entries: &[NveMapEntry]) -> bool {
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
    fn dispatch_events(&mut self, events: &[NveDirectEvent]) -> bool {
        HotAppInstance::dispatch_events(self, events)
    }
    fn set_props(&mut self, entries: &[NveMapEntry]) -> bool {
        HotAppInstance::set_props(self, entries)
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
    fn dispatch_events(&mut self, events: &[NveDirectEvent]) -> bool {
        DioxusSurface::dispatch_direct_events(self, unsafe {
            nve_abi::NveEventBatchRef::from_slice(events)
        })
    }
}

pub(crate) fn generate_pairing_token() -> String {
    generate_bearer_token()
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
    quic_port: u16,
    wss_port: u16,
    token: &str,
    cert_sha256: Option<&[u8; 32]>,
    width: f32,
    height: f32,
) -> String {
    let h = display_host_for_uri(host);
    format_wire_uri(
        h,
        quic_port,
        wss_port,
        token,
        cert_sha256,
        width.round() as u32,
        height.round() as u32,
    )
}

pub(crate) fn format_browser_pair_query(
    host: &str,
    ws_port: u16,
    token: &str,
    cert_hash_hex: Option<&str>,
) -> String {
    match cert_hash_hex {
        Some(h) if !h.is_empty() => {
            format!("?pair={}&token={token}&cert_hash={h}", format_ws_url(host, ws_port))
        }
        _ => format!("?pair={}&token={token}", format_ws_url(host, ws_port)),
    }
}

pub(crate) fn format_pair_ready_line(
    host: &str,
    quic_addr: SocketAddr,
    wss_addr: SocketAddr,
    token: &str,
    cert_hash_hex: &str,
) -> String {
    let ws_url = format_ws_url(host, wss_addr.port());
    format!(
        "NVE_PAIR_READY ws={ws_url} token={token} udp={quic_addr} tcp={wss_addr} cert_hash={cert_hash_hex}"
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn format_pair_banner(
    dylib_path: &Path,
    host: &str,
    quic_addr: SocketAddr,
    wss_addr: SocketAddr,
    token: &str,
    cert_sha256: &[u8; 32],
    cert_hash_hex: &str,
    width: f32,
    height: f32,
) -> String {
    let disp = display_host_for_uri(host);
    let uri = format_pairing_uri(
        host,
        quic_addr.port(),
        wss_addr.port(),
        token,
        Some(cert_sha256),
        width,
        height,
    );
    let browser_query = format_browser_pair_query(host, wss_addr.port(), token, Some(cert_hash_hex));
    format!(
        "nve pair — Unified QUIC/WebTransport + WSS Device Pairing (NveWireFrame v2)\n  dylib:     {}\n  quic/h3:   https://{disp}:{}/nve-wire (ALPN: h3, nve-wire-2; bound {quic_addr})\n  wss/ws:    wss://{disp}:{}/nve-wire (bound {wss_addr})\n  cert_hash: {cert_hash_hex} (13-day ECDSA P-256)\n  token:     {token}\n  uri:       {uri}\n  browser:   {browser_query}\n",
        dylib_path.display(),
        quic_addr.port(),
        wss_addr.port(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SyncTelemetrySnapshot {
    pub transport: &'static str,
    pub peers: u16,
    pub kbps_tenths: u32,
    pub compression_tenths: u16,
    pub rtt_ms_tenths: u32,
}

impl SyncTelemetrySnapshot {
    fn from_server(has_quic: bool, m: &NveWireServerMetrics, elapsed: Duration) -> Self {
        let peers = m.connected_peers.min(u16::MAX as usize) as u16;
        let transport = if has_quic {
            "lan-quic"
        } else if peers > 0 {
            "relay-wss"
        } else {
            "offline"
        };
        let secs = elapsed.as_secs_f64().max(0.05);
        let kbps = ((m.wire_bytes_tx + m.wire_bytes_rx) as f64 / 1024.0) / secs;
        let kbps_tenths = if kbps.is_finite() {
            (kbps * 10.0).round() as u32
        } else {
            0
        };
        Self {
            transport,
            peers,
            kbps_tenths,
            compression_tenths: 10,
            rtt_ms_tenths: 0,
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

const TELEMETRY_PUSH_INTERVAL: Duration = Duration::from_millis(250);

/// Live unified dual-ALPN QUIC/WebTransport + WSS pairing session owning a [`PairedApp`].
pub(crate) struct PairSession<A: PairedApp> {
    app: A,
    server: NveWireServer,
    frame_id: u64,
    last_frame: Vec<u8>,
    token: String,
    started_at: Instant,
    last_telemetry: Option<SyncTelemetrySnapshot>,
    last_telemetry_push: Instant,
}

impl<A: PairedApp> PairSession<A> {
    pub(crate) fn start(
        mut app: A,
        host: &str,
        quic_port: u16,
        wss_port: u16,
        token: String,
        width: f32,
        height: f32,
    ) -> Result<Self, String> {
        let quic_bind: SocketAddr = format_bind_addr(host, quic_port)
            .to_socket_addrs()
            .map_err(|e| format!("resolve QUIC bind address: {e}"))?
            .next()
            .ok_or_else(|| "no QUIC bind address resolved".to_owned())?;
        let wss_bind: SocketAddr = format_bind_addr(host, wss_port)
            .to_socket_addrs()
            .map_err(|e| format!("resolve WSS bind address: {e}"))?
            .next()
            .ok_or_else(|| "no WSS bind address resolved".to_owned())?;

        let mut cfg = NveWireServerConfig::new(quic_bind, wss_bind, token.clone());
        cfg.cert_sans.push(host.to_owned());
        let server = NveWireServer::bind_sync(cfg)
            .map_err(|e| format!("bind NveWireServer on quic={quic_bind} wss={wss_bind}: {e}"))?;

        if width > 0.0 && height > 0.0 {
            let layout_ev = [NveDirectEvent::layout(1, ROOT_NODE_ID, 0, 0.0, 0.0, width, height)];
            let _ = app.dispatch_events(&layout_ev);
        }

        let now = Instant::now();
        let mut session = Self {
            app,
            server,
            frame_id: 0,
            last_frame: Vec::new(),
            token,
            started_at: now,
            last_telemetry: None,
            last_telemetry_push: now,
        };
        let _ = session.push_telemetry_if_needed(true);
        let _ = session.render_and_broadcast(true)?;
        Ok(session)
    }

    pub(crate) fn udp_addr(&self) -> SocketAddr {
        self.server.quic_addr()
    }
    pub(crate) fn ws_addr(&self) -> SocketAddr {
        self.server.wss_addr()
    }
    pub(crate) fn cert_sha256(&self) -> [u8; 32] {
        self.server.cert_sha256()
    }
    pub(crate) fn cert_sha256_hex(&self) -> String {
        self.server.cert_sha256_hex()
    }
    #[cfg(test)]
    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub(crate) fn push_telemetry_if_needed(&mut self, force: bool) -> bool {
        let m = self.server.metrics();
        let snapshot = SyncTelemetrySnapshot::from_server(
            self.server.has_quic_peers(),
            &m,
            self.started_at.elapsed(),
        );
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
        self.app.set_props(&entries)
    }

    pub(crate) fn render_and_broadcast(&mut self, force: bool) -> Result<bool, String> {
        self.frame_id += 1;
        let frame = self.app.render(self.frame_id);
        if frame.is_empty() || (!force && frame.as_ref() == self.last_frame.as_slice()) {
            return Ok(false);
        }
        self.server
            .broadcast_command_delta(self.frame_id as u32, &frame)
            .map_err(|e| format!("NveWireServer broadcast failed: {e}"))?;
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
        while let Some(msg) = self.server.try_recv_inbound() {
            if msg.frame.kind == NveWireKind::EventBatch {
                let handled = decode_events(&msg.frame.payload)
                    .map(|decoded| {
                        let mut direct = Vec::with_capacity(decoded.len());
                        native_events_to_direct(&decoded, &mut direct);
                        self.app.dispatch_events(&direct)
                    })
                    .unwrap_or(false);
                outcome.events_dispatched += 1;
                if self.render_and_broadcast(handled).unwrap_or(false) {
                    outcome.frames_broadcast += 1;
                }
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
        self.server.shutdown();
    }
}

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
            let mut session = match PairSession::start(
                app,
                &host,
                udp_port,
                ws_port,
                token.clone(),
                width,
                height,
            ) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let banner = format_pair_banner(
                &dylib_path,
                &host,
                session.udp_addr(),
                session.ws_addr(),
                &token,
                &session.cert_sha256(),
                &session.cert_sha256_hex(),
                width,
                height,
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
        app,
        &opts.host,
        opts.udp_port,
        opts.ws_port,
        token.clone(),
        opts.width,
        opts.height,
    )
    .unwrap_or_else(|e| {
        eprintln!("failed to start pairing server: {e}");
        std::process::exit(1);
    });

    let cert_hash = session.cert_sha256();
    let cert_hash_hex = session.cert_sha256_hex();
    print!(
        "{}",
        format_pair_banner(
            &dylib_path,
            &opts.host,
            session.udp_addr(),
            session.ws_addr(),
            &token,
            &cert_hash,
            &cert_hash_hex,
            opts.width,
            opts.height,
        )
    );
    println!(
        "{}",
        format_pair_ready_line(
            &opts.host,
            session.udp_addr(),
            session.ws_addr(),
            &token,
            &cert_hash_hex
        )
    );
    println!("Streaming live NveWireFrame v2 frames & listening for remote EventBatch (Ctrl-C to stop)...");
    let _ = std::io::stdout().flush();
    session.run_until(opts.fps, &AtomicBool::new(false), None);
}

#[cfg(test)]
#[path = "pair_tests.rs"]
mod tests;
