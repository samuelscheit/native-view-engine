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

use nve_abi::{EventBatchBuf, NodeId, SurfaceId, TimestampUs};
use nve_backend_headless::HeadlessBackend;
use nve_components::tables::{
    core_realization_order, register_core_tables, Interaction, TextContent,
};
use nve_engine::{StoreBackend, StoreWireReplica, Viewport};
use nve_store::{NodeStore};
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
            let status = Command::new("cargo")
                .args(["build", "--locked", "-p", "nve-example-kitchen-sink"])
                .current_dir(&root)
                .status()
                .expect("invoke cargo build -p nve-example-kitchen-sink");
            assert!(
                status.success(),
                "cargo build --locked -p nve-example-kitchen-sink failed with {status}"
            );
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
    #[expect(clippy::panic, reason = "integration test timeout diagnostic")]
    {
        panic!(
            "nve pair did not emit NVE_PAIR_READY within timeout.\nstdout:\n{}\nstderr:\n{}",
            lines.join("\n"),
            stderr_lines.join("\n")
        );
    }
}

fn find_pressable_with_label(store: &NodeStore, label: &str) -> Option<NodeId> {
    fn walk(
        store: &NodeStore,
        id: NodeId,
        nearest_pressable: Option<NodeId>,
        label: &str,
    ) -> Option<NodeId> {
        if !store.is_live(id) {
            return None;
        }
        let active_pressable = if store
            .try_row::<Interaction>(id)
            .is_some_and(|i| !i.listeners().is_empty())
        {
            Some(id)
        } else {
            nearest_pressable
        };
        if store
            .try_row::<TextContent>(id)
            .is_some_and(|t| t.text().trim() == label)
        {
            return active_pressable;
        }
        for &child in store.children(id) {
            if let Some(found) = walk(store, child, active_pressable, label) {
                return Some(found);
            }
        }
        None
    }
    walk(store, NodeId::ROOT, None, label)
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

    register_core_tables();
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 1.0,
        },
    );
    let mut replica = StoreWireReplica::new(headless, core_realization_order());
    replica
        .submit_nve2_frame(&f0)
        .expect("submit initial Kitchen Sink frame");
    let tree0 = replica.backend().render_ascii_tree(replica.store());
    assert!(tree0.contains("NVE Kitchen Sink"), "tree0:\n{tree0}");
    assert!(tree0.contains("Count: 0"), "tree0:\n{tree0}");

    let inc_node = find_pressable_with_label(replica.store(), "Increment +")
        .expect("find Increment + pressable node");
    let reset_node =
        find_pressable_with_label(replica.store(), "Reset").expect("find Reset pressable node");

    let send_press = |client: &NveWireClient, node_id: NodeId, ts: u64| {
        let mut batch = EventBatchBuf::new();
        batch.push_press(SurfaceId::ROOT, node_id, TimestampUs(ts), 40.0, 20.0);
        client.send_events(&batch.to_wire_bytes()).expect("send EventBatch");
    };

    send_press(&ws, inc_node, 1_000);
    let f1 = ws.recv_frame_timeout(timeout).expect("frame 1").payload;
    replica.submit_nve2_frame(&f1).unwrap();
    assert!(replica
        .backend()
        .render_ascii_tree(replica.store())
        .contains("Count: 1"));

    send_press(&ws, inc_node, 2_000);
    let f2 = ws.recv_frame_timeout(timeout).expect("frame 2").payload;
    replica.submit_nve2_frame(&f2).unwrap();
    assert!(replica
        .backend()
        .render_ascii_tree(replica.store())
        .contains("Count: 2"));

    send_press(&ws, reset_node, 3_000);
    let f3 = ws.recv_frame_timeout(timeout).expect("frame 3").payload;
    replica.submit_nve2_frame(&f3).unwrap();
    assert!(replica
        .backend()
        .render_ascii_tree(replica.store())
        .contains("Count: 0"));

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
    register_core_tables();
    let mut replica = StoreWireReplica::new(HeadlessBackend::new(), core_realization_order());
    replica.submit_nve2_frame(&f0).expect("submit frame");
    assert!(replica
        .backend()
        .render_ascii_tree(replica.store())
        .contains("Count: 0"));
    quic.close();
}
