//! Integration tests for `nve pair`: ephemeral OS port binding (`--ws-port 0 --udp-port 0`),
//! machine-parseable `NVE_PAIR_READY` stdout readiness output, and live RFC-6455 binary WebSocket
//! `NVT1`/`NVD1` frame streaming + `InputEvents` round-trips against `nve-example-kitchen-sink`.

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

use nve_abi::{encode_events, EventPayload, NativeEvent, NativeEventKind, NodeId, ROOT_NODE_ID};
use nve_backend_headless::HeadlessBackend;
use nve_engine::{RenderBackend, Viewport};
use nve_interop::{PairingConfig, TcpDeltaClient, TransportError};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Debug, Clone)]
struct ReadyInfo {
    ws_url: String,
    token: String,
    udp_addr: SocketAddr,
    tcp_addr: SocketAddr,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tools dir")
        .parent()
        .expect("packages dir")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn target_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(val) => {
            let p = PathBuf::from(val);
            if p.is_absolute() { p } else { root.join(p) }
        }
        None => root.join("target"),
    }
}

fn kitchen_sink_cdylib_path() -> PathBuf {
    static DYLIB: OnceLock<PathBuf> = OnceLock::new();
    DYLIB
        .get_or_init(|| {
            let root = workspace_root();
            let ext = if cfg!(target_os = "macos") {
                "dylib"
            } else if cfg!(target_os = "windows") {
                "dll"
            } else {
                "so"
            };
            let prefix = if cfg!(target_os = "windows") { "" } else { "lib" };
            let path = target_dir(&root)
                .join("debug")
                .join(format!("{prefix}nve_example_kitchen_sink.{ext}"));
            if !path.exists() {
                let status = Command::new("cargo")
                    .args(["build", "--locked", "-p", "nve-example-kitchen-sink"])
                    .current_dir(&root)
                    .status()
                    .expect("invoke cargo build -p nve-example-kitchen-sink");
                assert!(
                    status.success(),
                    "cargo build --locked -p nve-example-kitchen-sink failed with {status}"
                );
            }
            assert!(
                path.exists(),
                "expected kitchen-sink cdylib at {}",
                path.display()
            );
            path
        })
        .clone()
}

fn parse_ready_line(line: &str) -> Option<ReadyInfo> {
    let rest = line.trim().strip_prefix("NVE_PAIR_READY ")?;
    let mut ws_url = None;
    let mut token = None;
    let mut udp_addr = None;
    let mut tcp_addr = None;
    for part in rest.split_whitespace() {
        if let Some(v) = part.strip_prefix("ws=") {
            ws_url = Some(v.to_owned());
        } else if let Some(v) = part.strip_prefix("token=") {
            token = Some(v.to_owned());
        } else if let Some(v) = part.strip_prefix("udp=") {
            udp_addr = v.parse::<SocketAddr>().ok();
        } else if let Some(v) = part.strip_prefix("tcp=") {
            tcp_addr = v.parse::<SocketAddr>().ok();
        }
    }
    Some(ReadyInfo {
        ws_url: ws_url?,
        token: token?,
        udp_addr: udp_addr?,
        tcp_addr: tcp_addr?,
    })
}

fn spawn_pair_and_wait_ready(extra_args: &[&str]) -> (ChildGuard, Vec<String>, ReadyInfo) {
    let dylib = kitchen_sink_cdylib_path();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nve"));
    cmd.arg("pair")
        .arg("--dylib")
        .arg(&dylib)
        .args(["--host", "127.0.0.1", "--udp-port", "0", "--ws-port", "0"])
        .args(extra_args)
        .current_dir(workspace_root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn nve pair");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let (err_tx, err_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if err_tx.send(line).is_err() {
                break;
            }
        }
    });

    let guard = ChildGuard(child);
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(line) => {
                let maybe_ready = parse_ready_line(&line);
                lines.push(line);
                if let Some(info) = maybe_ready {
                    return (guard, lines, info);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let stderr_lines: Vec<String> = err_rx.try_iter().collect();
    panic!(
        "nve pair did not emit NVE_PAIR_READY within timeout.\nstdout:\n{}\nstderr:\n{}",
        lines.join("\n"),
        stderr_lines.join("\n")
    );
}

fn find_pressable_with_label(backend: &HeadlessBackend, label: &str) -> Option<NodeId> {
    fn walk(
        backend: &HeadlessBackend,
        id: NodeId,
        nearest_pressable: Option<NodeId>,
        label: &str,
    ) -> Option<NodeId> {
        let node = backend.node(id)?;
        let active_pressable = if node.event_mask != 0 {
            Some(id)
        } else {
            nearest_pressable
        };
        if node.text.as_deref().is_some_and(|t| t.trim() == label) {
            return active_pressable;
        }
        for &child in &node.children {
            if let Some(found) = walk(backend, child, active_pressable, label) {
                return Some(found);
            }
        }
        None
    }
    walk(backend, ROOT_NODE_ID, None, label)
}

#[test]
fn nve_pair_ephemeral_ports_readiness_and_live_ws_roundtrip() {
    let (_child, stdout_lines, ready) = spawn_pair_and_wait_ready(&[]);

    assert!(ready.udp_addr.port() > 0, "UDP port must be resolved non-zero");
    assert!(ready.tcp_addr.port() > 0, "TCP/WS port must be resolved non-zero");
    assert_eq!(
        ready.ws_url,
        format!("ws://127.0.0.1:{}", ready.tcp_addr.port())
    );
    assert!(
        ready.token.starts_with("nve-"),
        "expected auto-generated token, got {}",
        ready.token
    );

    let full_stdout = stdout_lines.join("\n");
    let expected_uri = format!(
        "nve://pair?host=127.0.0.1&udp={}&ws={}&token={}",
        ready.udp_addr.port(),
        ready.tcp_addr.port(),
        ready.token
    );
    let expected_browser_query = format!("?pair={}&token={}", ready.ws_url, ready.token);
    assert!(
        full_stdout.contains(&expected_uri),
        "stdout missing pairing URI {expected_uri}:\n{full_stdout}"
    );
    assert!(
        full_stdout.contains(&expected_browser_query),
        "stdout missing browser query {expected_browser_query}:\n{full_stdout}"
    );

    // Connect using `/` (matching browser `new WebSocket("ws://127.0.0.1:<port>")`).
    let cfg = PairingConfig::new(ready.token.clone());
    let mut ws = TcpDeltaClient::connect_websocket(ready.tcp_addr, "/", cfg)
        .expect("RFC-6455 binary WebSocket NVT1 handshake on /");

    let timeout = Duration::from_secs(3);
    let f0 = ws
        .recv_frame(timeout)
        .expect("recv initial keyframe")
        .expect("initial keyframe present")
        .1;

    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        1,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 1.0,
        },
    );
    headless.submit_frame(&f0).expect("submit initial Kitchen Sink frame");
    let tree0 = headless.render_ascii_tree();
    assert!(tree0.contains("NVE Kitchen Sink"), "tree0:\n{tree0}");
    assert!(tree0.contains("Count: 0"), "tree0:\n{tree0}");

    let inc_node = find_pressable_with_label(&headless, "Increment +")
        .expect("find Increment + pressable node");
    let reset_node =
        find_pressable_with_label(&headless, "Reset").expect("find Reset pressable node");

    let send_press = |ws: &mut TcpDeltaClient, node_id: NodeId, ts: u64| {
        let ev_ring = encode_events(&[NativeEvent {
            surface_id: 1,
            node_id,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 40.0, y: 20.0 },
        }]);
        ws.send_events(&ev_ring).expect("send NVT1 InputEvents");
    };

    // Click Increment + twice -> Count: 1 -> Count: 2.
    send_press(&mut ws, inc_node, 1_000);
    let f1 = ws
        .recv_frame(timeout)
        .expect("recv frame 1")
        .expect("frame 1 present")
        .1;
    headless.submit_frame(&f1).expect("submit frame 1");
    assert!(
        headless.render_ascii_tree().contains("Count: 1"),
        "expected Count: 1 after 1st tap"
    );

    send_press(&mut ws, inc_node, 2_000);
    let f2 = ws
        .recv_frame(timeout)
        .expect("recv frame 2")
        .expect("frame 2 present")
        .1;
    headless.submit_frame(&f2).expect("submit frame 2");
    assert!(
        headless.render_ascii_tree().contains("Count: 2"),
        "expected Count: 2 after 2nd tap"
    );

    // Click Reset -> Count: 0.
    send_press(&mut ws, reset_node, 3_000);
    let f3 = ws
        .recv_frame(timeout)
        .expect("recv frame 3")
        .expect("frame 3 present")
        .1;
    headless.submit_frame(&f3).expect("submit frame 3");
    assert!(
        headless.render_ascii_tree().contains("Count: 0"),
        "expected Count: 0 after Reset tap"
    );

    ws.shutdown();
}

#[test]
fn nve_pair_honors_explicit_token_and_rejects_invalid_ws_token() {
    let (_child, _lines, ready) = spawn_pair_and_wait_ready(&["--token", "explicit-e2e-secret"]);

    assert_eq!(ready.token, "explicit-e2e-secret");
    assert!(ready.udp_addr.port() > 0);
    assert!(ready.tcp_addr.port() > 0);

    let wrong_cfg = PairingConfig::new("wrong-secret");
    let err = TcpDeltaClient::connect_websocket(ready.tcp_addr, "/ws/nvd", wrong_cfg)
        .err()
        .expect("invalid token must be rejected");
    assert!(
        matches!(
            err,
            TransportError::AuthenticationFailed | TransportError::HandshakeRejected(_)
        ),
        "expected AuthenticationFailed/HandshakeRejected error, got {err:?}"
    );

    let ok_cfg = PairingConfig::new("explicit-e2e-secret");
    let mut ws = TcpDeltaClient::connect_websocket(ready.tcp_addr, "/ws/nvd", ok_cfg)
        .expect("valid explicit token connects on /ws/nvd");
    let f0 = ws
        .recv_frame(Duration::from_secs(3))
        .expect("recv frame")
        .expect("frame present")
        .1;
    let mut headless = HeadlessBackend::new();
    headless.submit_frame(&f0).expect("submit frame");
    assert!(headless.render_ascii_tree().contains("Count: 0"));
    ws.shutdown();
}
