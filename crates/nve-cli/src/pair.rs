//! `nve pair` — Live `NVD1` binary delta device pairing server over BOTH UDP and TCP/WebSocket
//! with zero-JSON bidirectional `InputEvents` ingestion back into the live app instance.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use nve_abi::{
    decode_events, encode_events, EventPayload, NativeEvent, NativeEventKind, ROOT_NODE_ID,
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
    Tier1Patch(String),
    DylibReload(PathBuf),
    Shutdown,
}

/// Runtime abstraction over a live NVE app hosted by [`PairSession`].
pub(crate) trait PairedApp {
    fn render(&mut self, frame_id: u64) -> Vec<u8>;
    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool;
    fn apply_hot_patch(&mut self, _patch_json: &str) -> bool {
        false
    }
    fn reload(&mut self, _dylib_path: &Path) -> Result<u64, String> {
        Ok(1)
    }
}

impl PairedApp for HotAppInstance {
    fn render(&mut self, frame_id: u64) -> Vec<u8> {
        HotAppInstance::render(self, frame_id)
    }
    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        HotAppInstance::dispatch_events(self, encoded_events)
    }
    fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
        HotAppInstance::apply_hot_patch(self, patch_json)
    }
    fn reload(&mut self, dylib_path: &Path) -> Result<u64, String> {
        HotAppInstance::reload(self, dylib_path)
            .map(|o| o.generation)
            .map_err(|e| format!("cdylib hot-swap failed: {e}"))
    }
}

impl PairedApp for DioxusSurface {
    fn render(&mut self, frame_id: u64) -> Vec<u8> {
        self.render_frame(frame_id).2
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

pub(crate) fn display_host_for_uri(host: &str) -> &str {
    if host == "0.0.0.0" || host == "::" { "127.0.0.1" } else { host }
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

pub(crate) fn format_pair_banner(
    dylib_path: &Path,
    host: &str,
    udp_addr: SocketAddr,
    ws_addr: SocketAddr,
    token: &str,
    width: f32,
    height: f32,
) -> String {
    let disp = display_host_for_uri(host);
    let uri = format_pairing_uri(host, udp_addr.port(), ws_addr.port(), token, width, height);
    format!(
        "nve pair — Live Binary NVD1 Device Pairing\n  dylib:   {}\n  udp:     {udp_addr}\n  tcp/ws:  ws://{disp}:{}/ws/nvd (bound {ws_addr})\n  token:   {token}\n  uri:     {uri}\n",
        dylib_path.display(),
        ws_addr.port()
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

/// Live dual-transport (`UdpDeltaServer` + `TcpDeltaServer`) pairing session owning a [`PairedApp`].
pub(crate) struct PairSession<A: PairedApp> {
    app: A,
    udp: UdpDeltaServer,
    tcp: TcpDeltaServer,
    frame_id: u64,
    last_frame: Vec<u8>,
    token: String,
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
        let udp = UdpDeltaServer::bind(format!("{host}:{udp_port}"), cfg.clone())
            .map_err(|e| format!("bind UDP pairing server on {host}:{udp_port}: {e}"))?;
        let tcp = TcpDeltaServer::bind(format!("{host}:{ws_port}"), cfg)
            .map_err(|e| format!("bind TCP/WS pairing server on {host}:{ws_port}: {e}"))?;

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
        };
        let _ = session.render_and_broadcast(true)?;
        Ok(session)
    }

    pub(crate) fn udp_addr(&self) -> SocketAddr { self.udp.local_addr() }
    pub(crate) fn ws_addr(&self) -> SocketAddr { self.tcp.local_addr() }
    #[cfg(test)]
    pub(crate) fn token(&self) -> &str { &self.token }

    pub(crate) fn render_and_broadcast(&mut self, force: bool) -> Result<bool, String> {
        self.frame_id += 1;
        let frame = self.app.render(self.frame_id);
        if frame.is_empty() || (!force && frame == self.last_frame) {
            return Ok(false);
        }
        self.udp.send_frame(&frame).map_err(|e| format!("UDP send_frame failed: {e}"))?;
        self.tcp.send_frame(&frame).map_err(|e| format!("TCP/WS send_frame failed: {e}"))?;
        self.last_frame = frame;
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
                    PairControlMsg::Tier1Patch(json) => {
                        if self.app.apply_hot_patch(&json)
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
            &dylib_path, &opts.host, session.udp_addr(), session.ws_addr(), &token, opts.width, opts.height,
        )
    );
    println!("Streaming live NVD1 frames & listening for remote InputEvents (Ctrl-C to stop)...");
    session.run_until(opts.fps, &AtomicBool::new(false), None);
}

#[cfg(test)]
mod tests {
    use nve_backend_headless::HeadlessBackend;
    use nve_engine::{RenderBackend, Viewport};
    use nve_interop::{PairingConfig, TcpDeltaClient, UdpDeltaClient};

    use super::*;
    use crate::doctor::InspectDemo;

    #[test]
    fn pair_token_generation_and_uri_formatting() {
        let (t1, t2) = (generate_pairing_token(), generate_pairing_token());
        assert!(t1.starts_with("nve-") && t1 != t2);
        assert_eq!(resolve_or_generate_token(Some("  custom-tok  ".into())), "custom-tok");
        assert!(resolve_or_generate_token(Some("   ".into())).starts_with("nve-"));
        assert!(resolve_or_generate_token(None).starts_with("nve-"));
        assert_eq!(
            format_pairing_uri("0.0.0.0", 8091, 8092, "tok123", 393.0, 852.0),
            "nve://pair?host=127.0.0.1&udp=8091&ws=8092&token=tok123&w=393&h=852"
        );
        assert!(format_pairing_uri("192.168.1.42", 9001, 9002, "lan", 430.0, 932.0)
            .contains("host=192.168.1.42"));
    }

    #[test]
    fn pair_session_dispatches_remote_input_events_over_udp_tcp_and_ws_and_streams_deltas() {
        let (ctrl_tx, ctrl_rx) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let server_thread = std::thread::spawn(move || {
            let surface = DioxusSurface::new(1, InspectDemo);
            let mut session = PairSession::start(
                surface, "127.0.0.1", 0, 0, "cli-pair-e2e-token".into(), 393.0, 852.0,
            )
            .expect("start PairSession");
            ready_tx
                .send((session.udp_addr(), session.ws_addr(), session.token().to_owned()))
                .unwrap();
            session.run_until(120, &AtomicBool::new(false), Some(&ctrl_rx));
        });

        let (udp_addr, ws_addr, token) = ready_rx.recv().unwrap();
        let cfg = PairingConfig::new(token);
        let mut udp = UdpDeltaClient::connect(udp_addr, cfg.clone()).unwrap();
        let mut tcp = TcpDeltaClient::connect(ws_addr, cfg.clone()).unwrap();
        let mut ws = TcpDeltaClient::connect_websocket(ws_addr, "/ws/nvd", cfg).unwrap();

        let timeout = Duration::from_secs(2);
        let f0 = udp.recv_frame(timeout).unwrap().unwrap().1;
        assert_eq!(f0, tcp.recv_frame(timeout).unwrap().unwrap().1);
        assert_eq!(f0, ws.recv_frame(timeout).unwrap().unwrap().1);

        let mut headless = HeadlessBackend::new();
        headless.set_viewport(1, Viewport { width: 393.0, height: 852.0, scale_factor: 1.0 });
        headless.submit_frame(&f0).unwrap();
        assert!(headless.render_ascii_tree().contains("Counter: 0"));
        let btn_node = headless.hit_test(60.0, 75.0, true).expect("pressable button");

        let make_tap = |ts: u64| {
            encode_events(&[NativeEvent {
                surface_id: 1,
                node_id: btn_node,
                timestamp_us: ts,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point { x: 60.0, y: 75.0 },
            }])
        };

        // 1. UDP tap -> Counter: 1 across UDP, TCP, and WS.
        udp.send_events(&make_tap(1000)).unwrap();
        let f1 = udp.recv_frame(timeout).unwrap().unwrap().1;
        assert_eq!(f1, tcp.recv_frame(timeout).unwrap().unwrap().1);
        assert_eq!(f1, ws.recv_frame(timeout).unwrap().unwrap().1);
        headless.submit_frame(&f1).unwrap();
        assert!(headless.render_ascii_tree().contains("Counter: 1"));

        // 2. RFC-6455 WebSocket tap -> Counter: 2 across UDP, TCP, and WS.
        ws.send_events(&make_tap(2000)).unwrap();
        let f2 = ws.recv_frame(timeout).unwrap().unwrap().1;
        assert_eq!(f2, udp.recv_frame(timeout).unwrap().unwrap().1);
        assert_eq!(f2, tcp.recv_frame(timeout).unwrap().unwrap().1);
        headless.submit_frame(&f2).unwrap();
        assert!(headless.render_ascii_tree().contains("Counter: 2"));

        // 3. Raw TCP tap -> Counter: 3 across UDP, TCP, and WS.
        tcp.send_events(&make_tap(3000)).unwrap();
        let f3 = tcp.recv_frame(timeout).unwrap().unwrap().1;
        assert_eq!(f3, udp.recv_frame(timeout).unwrap().unwrap().1);
        assert_eq!(f3, ws.recv_frame(timeout).unwrap().unwrap().1);
        headless.submit_frame(&f3).unwrap();
        assert!(headless.render_ascii_tree().contains("Counter: 3"));

        ctrl_tx.send(PairControlMsg::Shutdown).unwrap();
        udp.shutdown();
        tcp.shutdown();
        ws.shutdown();
        server_thread.join().expect("pair server thread exited cleanly");
    }
}
