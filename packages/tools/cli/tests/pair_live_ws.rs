//! Integration tests for `nve pair`: ephemeral OS port binding (`--ws-port 0 --udp-port 0`),
//! machine-parseable `NVE_PAIR_READY` stdout readiness output with 13-day ECDSA P-256 `cert_hash`,
//! and live QUIC + WSS `NveWireFrame` v2 frame streaming + `EventBatch` round-trips against
//! `nve-example-kitchen-sink`.

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

use nve_abi::{encode_events, EventPayload, NativeEvent, NativeEventKind, NodeId, ROOT_NODE_ID};
use nve_backend_headless::HeadlessBackend;
use nve_engine::{RenderBackend, Viewport};
use nve_wire::{
    parse_cert_hash_hex, parse_wire_uri, NveWireClient, NveWireClientConfig, NveWireTransportError,
};

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
    cert_hash_hex: String,
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
            if p.is_absolute() {
                p
            } else {
                root.join(p)
            }
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
    let mut cert_hash_hex = None;
    for part in rest.split_whitespace() {
        if let Some(v) = part.strip_prefix("ws=") {
            ws_url = Some(v.to_owned());
        } else if let Some(v) = part.strip_prefix("token=") {
            token = Some(v.to_owned());
        } else if let Some(v) = part.strip_prefix("udp=") {
            udp_addr = v.parse::<SocketAddr>().ok();
        } else if let Some(v) = part.strip_prefix("tcp=") {
            tcp_addr = v.parse::<SocketAddr>().ok();
        } else if let Some(v) = part.strip_prefix("cert_hash=") {
            cert_hash_hex = Some(v.to_owned());
        }
    }
    Some(ReadyInfo {
        ws_url: ws_url?,
        token: token?,
        udp_addr: udp_addr?,
        tcp_addr: tcp_addr?,
        cert_hash_hex: cert_hash_hex?,
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
fn nve_pair_ephemeral_ports_readiness_and_live_quic_wss_roundtrip() {
    let (_child, stdout_lines, ready) = spawn_pair_and_wait_ready(&[]);

    assert!(ready.udp_addr.port() > 0);
    assert!(ready.tcp_addr.port() > 0);
    assert_eq!(ready.ws_url, format!("ws://127.0.0.1:{}", ready.tcp_addr.port()));
    assert!(ready.token.starts_with("nve_"));
    let cert_hash = parse_cert_hash_hex(&ready.cert_hash_hex).expect("valid 64-hex SHA-256");

    let full_stdout = stdout_lines.join("\n");
    let expected_uri = format!(
        "nve://pair?host=127.0.0.1&quic={}&wss={}&token={}",
        ready.udp_addr.port(),
        ready.tcp_addr.port(),
        ready.token
    );
    assert!(
        full_stdout.contains(&expected_uri),
        "stdout missing pairing URI {expected_uri}:\n{full_stdout}"
    );

    // Connect over real TLS WSS with SHA-256 cert pinning!
    let cfg = NveWireClientConfig::new(ready.token.clone(), "cli-live-ws-test")
        .with_cert_sha256(cert_hash);
    let wss_uri = parse_wire_uri(&format!("wss://{}/nve-wire", ready.tcp_addr)).unwrap();
    let ws = NveWireClient::connect_uri_sync(&wss_uri, &cfg)
        .expect("TLS WSS NveWireFrame v2 handshake");

    let timeout = Duration::from_secs(3);
    let f0 = ws.recv_frame_timeout(timeout).expect("initial keyframe").payload;

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

    let send_press = |client: &NveWireClient, node_id: NodeId, ts: u64| {
        let ev_ring = encode_events(&[NativeEvent {
            surface_id: 1,
            node_id,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 40.0, y: 20.0 },
        }]);
        client.send_events(&ev_ring).expect("send EventBatch");
    };

    send_press(&ws, inc_node, 1_000);
    let f1 = ws.recv_frame_timeout(timeout).expect("frame 1").payload;
    headless.submit_frame(&f1).unwrap();
    assert!(headless.render_ascii_tree().contains("Count: 1"));

    send_press(&ws, inc_node, 2_000);
    let f2 = ws.recv_frame_timeout(timeout).expect("frame 2").payload;
    headless.submit_frame(&f2).unwrap();
    assert!(headless.render_ascii_tree().contains("Count: 2"));

    send_press(&ws, reset_node, 3_000);
    let f3 = ws.recv_frame_timeout(timeout).expect("frame 3").payload;
    headless.submit_frame(&f3).unwrap();
    assert!(headless.render_ascii_tree().contains("Count: 0"));

    ws.close();
}

#[test]
fn nve_pair_honors_explicit_token_and_rejects_invalid_bearer_token() {
    let (_child, _lines, ready) = spawn_pair_and_wait_ready(&["--token", "explicit-e2e-secret"]);

    assert_eq!(ready.token, "explicit-e2e-secret");
    let cert_hash = parse_cert_hash_hex(&ready.cert_hash_hex).unwrap();

    let wrong_cfg = NveWireClientConfig::new("wrong-secret", "intruder").with_cert_sha256(cert_hash);
    let quic_uri = parse_wire_uri(&format!("quic://{}", ready.udp_addr)).unwrap();
    let err = NveWireClient::connect_uri_sync(&quic_uri, &wrong_cfg)
        .err()
        .expect("invalid token must be rejected");
    assert!(matches!(err, NveWireTransportError::Auth(_)), "got {err:?}");

    let ok_cfg = NveWireClientConfig::new("explicit-e2e-secret", "cli-quic")
        .with_cert_sha256(cert_hash);
    let quic = NveWireClient::connect_uri_sync(&quic_uri, &ok_cfg)
        .expect("valid explicit token connects over QUIC nve-wire-2");
    let f0 = quic
        .recv_frame_timeout(Duration::from_secs(3))
        .expect("recv frame")
        .payload;
    let mut headless = HeadlessBackend::new();
    headless.submit_frame(&f0).expect("submit frame");
    assert!(headless.render_ascii_tree().contains("Count: 0"));
    quic.close();
}
