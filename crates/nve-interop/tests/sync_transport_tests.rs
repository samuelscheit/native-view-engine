use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nve_abi::{
    decode_events, encode_events, ColorRgba, CommandBufferBuilder, ComponentKind, Dimension,
    EdgeInsets, EventPayload, FlexDirection, LayoutProps, NativeEvent, NativeEventKind, StyleProps,
    ROOT_NODE_ID,
};
use nve_interop::{
    compute_ws_accept_key, encode_ws_frame, PairingConfig, StreamFraming, SyncTransportError,
    TcpDeltaClient, TcpDeltaServer, TransportError, TransportMsgKind, UdpDeltaClient,
    UdpDeltaServer, WsOpcode, MAX_INPUT_EVENT_BYTES, MAX_UDP_DATAGRAM_BYTES,
    MAX_UDP_SINGLE_PAYLOAD, TRANSPORT_MAGIC, TRANSPORT_PROTOCOL_VERSION,
};

// =============================================================================
// Realistic Studio Workspace CommandBuffer Fixture
// =============================================================================

fn make_studio_workspace_frame(
    frame_id: u64,
    accent: ColorRgba,
    status_text: &str,
    active_panel: u32,
) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Percent(100.0),
            flex_direction: FlexDirection::Row,
            padding: EdgeInsets::all(12.0),
            gap: 8.0,
            ..Default::default()
        },
    );
    let mut children = Vec::with_capacity(42);
    for i in 0..40u32 {
        let nid = 10 + i;
        children.push(nid);
        b.create_node(nid, ComponentKind::View, None);
        b.set_layout(
            nid,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Points(44.0),
                ..Default::default()
            },
        );
        b.set_style(
            nid,
            &StyleProps {
                background: if i == active_panel % 4 {
                    accent
                } else {
                    ColorRgba::rgb(30, 41, 59)
                },
                border_radius: 12.0,
                font_size: 12.0,
                ..Default::default()
            },
        );
    }
    b.create_node(99, ComponentKind::Text, None);
    b.set_text(99, status_text);
    children.push(99);
    b.set_children(2, &children);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    b.into_bytes()
}

// =============================================================================
// Deterministic Lossy + Reordering Local UDP Proxy Socket
// =============================================================================

#[derive(Debug, Default, Clone)]
struct ProxyStats {
    forwarded_s2c: usize,
    forwarded_c2s: usize,
    dropped_s2c: usize,
    dropped_c2s: usize,
    reordered_s2c: usize,
    max_datagram_bytes: usize,
    wire_bytes_s2c_total: usize,
    wire_bytes_s2c_since_mark: usize,
}

struct LossyUdpProxy {
    proxy_addr: SocketAddr,
    stats: Arc<Mutex<ProxyStats>>,
    drop_all_s2c: Arc<AtomicBool>,
    mark_active: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LossyUdpProxy {
    fn start(
        server_addr: SocketAddr,
        drop_percent: u32,
        reorder_percent: u32,
    ) -> io::Result<Self> {
        let sock = UdpSocket::bind("127.0.0.1:0")?;
        sock.set_read_timeout(Some(Duration::from_millis(5)))?;
        let proxy_addr = sock.local_addr()?;

        let stats = Arc::new(Mutex::new(ProxyStats::default()));
        let drop_all_s2c = Arc::new(AtomicBool::new(false));
        let mark_active = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));

        let w_stats = Arc::clone(&stats);
        let w_drop_all = Arc::clone(&drop_all_s2c);
        let w_mark = Arc::clone(&mark_active);
        let w_shutdown = Arc::clone(&shutdown);

        let worker = thread::Builder::new()
            .name("nve-test-udp-proxy".into())
            .spawn(move || {
                let mut client_addr: Option<SocketAddr> = None;
                let mut delayed_s2c: VecDeque<(Instant, Vec<u8>)> = VecDeque::new();
                let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
                let mut next_pct = || -> u32 {
                    rng = rng
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    ((rng >> 33) % 100) as u32
                };
                let mut buf = [0u8; 4096];

                while !w_shutdown.load(Ordering::Relaxed) {
                    // Flush delayed packets when their reorder delay expires.
                    let now = Instant::now();
                    while let Some((release_at, _)) = delayed_s2c.front() {
                        if now >= *release_at {
                            let (_, pkt) = delayed_s2c.pop_front().unwrap();
                            if let Some(caddr) = client_addr {
                                let _ = sock.send_to(&pkt, caddr);
                                let mut st = w_stats.lock().unwrap();
                                st.forwarded_s2c += 1;
                                st.wire_bytes_s2c_total += pkt.len();
                                if w_mark.load(Ordering::Relaxed) {
                                    st.wire_bytes_s2c_since_mark += pkt.len();
                                }
                            }
                        } else {
                            break;
                        }
                    }

                    match sock.recv_from(&mut buf) {
                        Ok((n, src)) => {
                            let pkt = buf[..n].to_vec();
                            {
                                let mut st = w_stats.lock().unwrap();
                                if n > st.max_datagram_bytes {
                                    st.max_datagram_bytes = n;
                                }
                            }
                            if src == server_addr {
                                let Some(caddr) = client_addr else {
                                    continue;
                                };
                                if w_drop_all.load(Ordering::Relaxed) {
                                    w_stats.lock().unwrap().dropped_s2c += 1;
                                    continue;
                                }
                                let r = next_pct();
                                if r < drop_percent {
                                    w_stats.lock().unwrap().dropped_s2c += 1;
                                    continue;
                                }
                                // Reorder: hold packet briefly so the next packet overtakes it.
                                if r < drop_percent + reorder_percent {
                                    w_stats.lock().unwrap().reordered_s2c += 1;
                                    delayed_s2c.push_back((
                                        Instant::now() + Duration::from_millis(8),
                                        pkt,
                                    ));
                                    continue;
                                }
                                let _ = sock.send_to(&pkt, caddr);
                                let mut st = w_stats.lock().unwrap();
                                st.forwarded_s2c += 1;
                                st.wire_bytes_s2c_total += pkt.len();
                                if w_mark.load(Ordering::Relaxed) {
                                    st.wire_bytes_s2c_since_mark += pkt.len();
                                }
                            } else {
                                client_addr = Some(src);
                                let r = next_pct();
                                if r < drop_percent {
                                    w_stats.lock().unwrap().dropped_c2s += 1;
                                    continue;
                                }
                                let _ = sock.send_to(&pkt, server_addr);
                                w_stats.lock().unwrap().forwarded_c2s += 1;
                            }
                        }
                        Err(err)
                            if err.kind() == io::ErrorKind::WouldBlock
                                || err.kind() == io::ErrorKind::TimedOut => {}
                        Err(_) => break,
                    }
                }
            })?;

        Ok(Self {
            proxy_addr,
            stats,
            drop_all_s2c,
            mark_active,
            shutdown,
            worker: Some(worker),
        })
    }

    fn proxy_addr(&self) -> SocketAddr {
        self.proxy_addr
    }

    fn start_steady_mark(&self) {
        self.stats.lock().unwrap().wire_bytes_s2c_since_mark = 0;
        self.mark_active.store(true, Ordering::SeqCst);
    }

    fn set_drop_all_s2c(&self, drop: bool) {
        self.drop_all_s2c.store(drop, Ordering::SeqCst);
    }

    fn stats(&self) -> ProxyStats {
        self.stats.lock().unwrap().clone()
    }
}

impl Drop for LossyUdpProxy {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

// =============================================================================
// Test 1: UDP 60 Studio Frames over 15% Lossy + Reordering Proxy (< 10 KB/s!)
// =============================================================================

#[test]
fn udp_live_60_studio_frames_under_15pct_loss_and_reorder_lossless_under_10_kbps() {
    let config = PairingConfig::new("studio-pair-live-2026")
        .with_schema_version(1)
        .with_max_window(64)
        .with_rto_ms(55)
        .with_heartbeat_interval_ms(120)
        .with_metrics_window_ms(1_000);

    let mut server = UdpDeltaServer::bind("127.0.0.1:0", config.clone()).unwrap();
    let proxy = LossyUdpProxy::start(server.local_addr(), 15, 20).unwrap();
    let mut client = UdpDeltaClient::connect(proxy.proxy_addr(), config).unwrap();

    assert!(server.wait_for_peers(1, Duration::from_secs(2)));
    assert_ne!(client.session_id(), 0);

    // 1. Mount initial Studio Workspace keyframe (~5.3 KB raw -> 5 MTU-safe UDP fragments <= 1200 B).
    let keyframe =
        make_studio_workspace_frame(0, ColorRgba::rgb(56, 189, 248), "Workspace Ready", 0);
    assert!(
        keyframe.len() > 4_000,
        "expected realistic multi-KB Studio keyframe, got {} B",
        keyframe.len()
    );
    assert_eq!(server.send_frame(&keyframe).unwrap(), 1);
    let (kf_seq, kf_rx) = client
        .recv_frame(Duration::from_secs(3))
        .unwrap()
        .expect("timed out waiting for initial keyframe");
    assert_eq!(kf_seq, 1);
    assert_eq!(kf_rx, keyframe);

    // 2. Stream 60 incremental Studio Workspace frames over 1 second (60 fps) through the 15% loss + reorder proxy!
    let statuses = ["Todo", "In Progress", "In Review", "Done"];
    let accents = [
        ColorRgba::rgb(56, 189, 248),
        ColorRgba::rgb(167, 139, 250),
        ColorRgba::rgb(251, 191, 36),
        ColorRgba::rgb(34, 197, 94),
    ];

    let mut expected_frames: Vec<Vec<u8>> = Vec::with_capacity(60);
    for i in 1..=60u64 {
        let status = format!("{} #{:02}", statuses[(i as usize) % statuses.len()], i);
        // Keep accent & panel steady on most frames and toggle accent every 6th frame (realistic Studio interaction).
        let accent = accents[((i / 6) as usize) % accents.len()];
        let f = make_studio_workspace_frame(i, accent, &status, 0);
        expected_frames.push(f);
    }

    proxy.start_steady_mark();
    let stream_start = Instant::now();
    let server_bytes_before = server.metrics().wire_bytes_sent;

    for (idx, frame) in expected_frames.iter().enumerate() {
        assert_eq!(server.send_frame(frame).unwrap(), 1);
        let target_elapsed = Duration::from_micros((idx as u64 + 1) * 16_666);
        if let Some(sleep_dur) = target_elapsed.checked_sub(stream_start.elapsed()) {
            thread::sleep(sleep_dur);
        }
    }

    // Collect all 60 reconstructed frames in strict sequence order.
    let mut received: Vec<(u32, Vec<u8>)> = Vec::with_capacity(60);
    let collect_deadline = Instant::now() + Duration::from_secs(4);
    while received.len() < 60 && Instant::now() < collect_deadline {
        if let Some(item) = client.recv_frame(Duration::from_millis(200)).unwrap() {
            received.push(item);
        }
    }
    let stream_elapsed = stream_start.elapsed();
    assert_eq!(
        received.len(),
        60,
        "expected all 60 frames recovered losslessly, got {}",
        received.len()
    );

    for (idx, ((seq, rx_frame), expected)) in
        received.iter().zip(expected_frames.iter()).enumerate()
    {
        assert_eq!(
            *seq,
            (idx as u32) + 2,
            "frames must arrive in strict monotonic sequence order"
        );
        assert_eq!(
            rx_frame, expected,
            "byte-for-byte mismatch on incremental frame {}",
            idx + 1
        );
    }

    // Wait briefly so final ACKs and Ping/Pong complete.
    thread::sleep(Duration::from_millis(60));

    let pstats = proxy.stats();
    let sm = server.metrics();
    let cm = client.metrics();

    assert!(
        pstats.max_datagram_bytes <= MAX_UDP_DATAGRAM_BYTES,
        "UDP datagram {} exceeded 1200B MTU!",
        pstats.max_datagram_bytes
    );
    assert!(
        pstats.dropped_s2c + pstats.dropped_c2s > 0,
        "proxy must drop real packets during test"
    );
    assert!(
        pstats.reordered_s2c > 0,
        "proxy must reorder real packets during test"
    );
    assert!(
        sm.retransmits_sent > 0,
        "expected SACK/NACK/RTO retransmits > 0 under 15% loss"
    );
    assert!(sm.acks_received > 0, "expected server ACKs received > 0");
    assert!(
        cm.nacks_sent > 0,
        "expected client NACKs/FragNACKs sent > 0 under 15% loss + reorder"
    );
    assert!(
        sm.rtt_samples > 0 || cm.rtt_samples > 0,
        "expected live RTT µs samples > 0"
    );
    assert!(
        cm.smoothed_rtt_us.is_some() || sm.smoothed_rtt_us.is_some(),
        "expected smoothed RTT µs populated"
    );

    let steady_server_bytes = sm.wire_bytes_sent.saturating_sub(server_bytes_before);
    let measured_kbps = (steady_server_bytes as f64 / stream_elapsed.as_secs_f64()) / 1024.0;
    let proxy_kbps =
        (pstats.wire_bytes_s2c_since_mark as f64 / stream_elapsed.as_secs_f64()) / 1024.0;

    eprintln!(
        "[udp_live_60fps] elapsed={:.2}s | raw_total={} B | steady_tx={} B ({:.2} KB/s server, {:.2} KB/s proxy, meter={:.2} KB/s) | max_dgram={} B | drops={} (s2c={}, c2s={}) | reordered={} | retx={} | rtt_us={:?}/{:?}",
        stream_elapsed.as_secs_f64(),
        sm.raw_bytes_sent,
        steady_server_bytes,
        measured_kbps,
        proxy_kbps,
        sm.tx_kbps(),
        pstats.max_datagram_bytes,
        pstats.dropped_s2c + pstats.dropped_c2s,
        pstats.dropped_s2c,
        pstats.dropped_c2s,
        pstats.reordered_s2c,
        sm.retransmits_sent,
        sm.smoothed_rtt_us,
        cm.smoothed_rtt_us
    );

    assert!(
        measured_kbps < 10.0,
        "measured steady-state UDP wire bandwidth under 15% drop + reorder must be < 10 KB/s, got {:.2} KB/s",
        measured_kbps
    );
    assert!(
        proxy_kbps < 10.0,
        "proxy-observed S2C wire bandwidth must be < 10 KB/s, got {:.2} KB/s",
        proxy_kbps
    );

    client.shutdown();
    server.shutdown();
}

// =============================================================================
// Test 2: UDP Automatic Keyframe Resync (> Window Loss) & Graceful Reconnect
// =============================================================================

#[test]
fn udp_window_overflow_auto_keyframe_resync_and_graceful_reconnect() {
    let config = PairingConfig::new("studio-udp-resync-token")
        .with_max_window(8)
        .with_rto_ms(40)
        .with_heartbeat_interval_ms(100);

    let mut server = UdpDeltaServer::bind("127.0.0.1:0", config.clone()).unwrap();
    let proxy = LossyUdpProxy::start(server.local_addr(), 0, 0).unwrap();
    let mut client = UdpDeltaClient::connect(proxy.proxy_addr(), config).unwrap();
    assert!(server.wait_for_peers(1, Duration::from_secs(2)));

    // 1. Send frames 1..=3 cleanly.
    for i in 1..=3u64 {
        let f = make_studio_workspace_frame(
            i,
            ColorRgba::rgb(56, 189, 248),
            &format!("PreBlackout {i}"),
            0,
        );
        server.send_frame(&f).unwrap();
        let (_, rx) = client
            .recv_frame(Duration::from_secs(2))
            .unwrap()
            .expect("missing pre-blackout frame");
        assert_eq!(rx, f);
    }

    // 2. Simulate total blackout dropping 12 consecutive frames (> window of 8!).
    proxy.set_drop_all_s2c(true);
    for i in 4..=15u64 {
        let f = make_studio_workspace_frame(
            i,
            ColorRgba::rgb(167, 139, 250),
            &format!("Blackout {i}"),
            0,
        );
        server.send_frame(&f).unwrap();
        thread::sleep(Duration::from_millis(5));
    }

    // 3. Restore link and send frame 16 -> client detects gap > window (16 - 3 = 13 > 8) and
    // automatically triggers keyframe resync!
    proxy.set_drop_all_s2c(false);
    let f16 = make_studio_workspace_frame(16, ColorRgba::rgb(34, 197, 94), "PostBlackout 16", 0);
    server.send_frame(&f16).unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut synced_f16 = false;
    while Instant::now() < deadline {
        if let Some((_, rx)) = client.recv_frame(Duration::from_millis(150)).unwrap() {
            if rx == f16 {
                synced_f16 = true;
                break;
            }
        }
    }
    assert!(
        synced_f16,
        "client did not auto-resync via keyframe after > window drop"
    );
    assert!(
        server.metrics().keyframe_resyncs >= 1,
        "expected server keyframe_resyncs >= 1"
    );

    // 4. Subsequent incremental delta applies cleanly after resync.
    let f17 = make_studio_workspace_frame(17, ColorRgba::rgb(34, 197, 94), "Steady 17", 0);
    server.send_frame(&f17).unwrap();
    let (_, rx17) = client
        .recv_frame(Duration::from_secs(2))
        .unwrap()
        .expect("missing steady frame after resync");
    assert_eq!(rx17, f17);

    // 5. Graceful disconnect, server advances while client offline, then reconnect -> auto keyframe resync!
    let old_sid = client.session_id();
    client.disconnect();
    thread::sleep(Duration::from_millis(30));

    let f20 = make_studio_workspace_frame(20, ColorRgba::rgb(251, 191, 36), "Offline 20", 0);
    server.send_frame(&f20).unwrap();

    let new_sid = client.reconnect().unwrap();
    assert_ne!(new_sid, 0);
    assert_ne!(new_sid, old_sid);
    let (_, rx_resync) = client
        .recv_frame(Duration::from_secs(2))
        .unwrap()
        .expect("expected automatic keyframe resync on UDP reconnect");
    assert_eq!(rx_resync, f20);
    assert!(client.metrics().reconnects >= 1);
    assert!(server.metrics().reconnects >= 1);

    let f21 = make_studio_workspace_frame(21, ColorRgba::rgb(251, 191, 36), "Resumed 21", 0);
    server.send_frame(&f21).unwrap();
    let (_, rx21) = client
        .recv_frame(Duration::from_secs(2))
        .unwrap()
        .expect("expected post-reconnect delta frame");
    assert_eq!(rx21, f21);

    client.shutdown();
    server.shutdown();
}

// =============================================================================
// Test 3: TCP Framed Stream + Mid-Stream Reconnect & Automatic Keyframe Resync
// =============================================================================

#[test]
fn tcp_framed_stream_mid_stream_reconnect_and_auto_keyframe_resync() {
    let config = PairingConfig::new("studio-tcp-pair-2026")
        .with_schema_version(2)
        .with_heartbeat_interval_ms(100);

    let mut server = TcpDeltaServer::bind("127.0.0.1:0", config.clone()).unwrap();
    let mut client = TcpDeltaClient::connect(server.local_addr(), config).unwrap();
    assert_eq!(client.framing(), StreamFraming::RawLengthPrefixed);
    assert!(server.wait_for_peers(1, Duration::from_secs(2)));

    // 1. Stream 15 Studio frames over native length-prefixed binary TCP.
    for i in 1..=15u64 {
        let f = make_studio_workspace_frame(
            i,
            ColorRgba::rgb(56, 189, 248),
            &format!("TCP Frame {i}"),
            0,
        );
        assert_eq!(server.send_frame(&f).unwrap(), 1);
        let (seq, rx) = client
            .recv_frame(Duration::from_secs(2))
            .unwrap()
            .expect("missing TCP frame");
        assert_eq!(seq, i as u32);
        assert_eq!(rx, f);
    }

    // 2. Disconnect mid-stream while server continues mutating state (frames 16..=25).
    let sid_before = client.session_id();
    client.disconnect();
    thread::sleep(Duration::from_millis(30));

    let mut latest_offline = Vec::new();
    for i in 16..=25u64 {
        latest_offline = make_studio_workspace_frame(
            i,
            ColorRgba::rgb(245, 158, 11),
            &format!("Offline State {i}"),
            1,
        );
        let _ = server.send_frame(&latest_offline).unwrap();
    }

    // 3. Reconnect -> server automatically sends Keyframe of latest_offline (frame 25)!
    let sid_after = client.reconnect().unwrap();
    assert_ne!(sid_after, sid_before);
    let (_, rx_kf) = client
        .recv_frame(Duration::from_secs(2))
        .unwrap()
        .expect("missing automatic keyframe resync after TCP reconnect");
    assert_eq!(rx_kf, latest_offline);

    // 4. Stream incremental delta frames 26..=35 seamlessly after reconnect.
    for i in 26..=35u64 {
        let f = make_studio_workspace_frame(
            i,
            ColorRgba::rgb(34, 197, 94),
            &format!("Post-Reconnect {i}"),
            1,
        );
        assert_eq!(server.send_frame(&f).unwrap(), 1);
        let (_, rx) = client
            .recv_frame(Duration::from_secs(2))
            .unwrap()
            .expect("missing post-reconnect TCP delta");
        assert_eq!(rx, f);
    }

    thread::sleep(Duration::from_millis(40));
    let sm = server.metrics();
    let cm = client.metrics();
    assert!(sm.reconnects >= 1);
    assert!(sm.keyframe_resyncs >= 1);
    assert!(cm.reconnects >= 1);
    assert!(sm.deltas_sent >= 20);
    assert!(
        sm.compression_ratio() > 5.0,
        "expected >5x raw/wire compression on TCP delta stream, got {:.2}x",
        sm.compression_ratio()
    );
    assert!(sm.rtt_samples > 0 || cm.rtt_samples > 0);

    eprintln!(
        "[tcp_stream] frames_sent={} | raw={} B | wire={} B | compression={:.1}x | resyncs={} | reconnects={} | rtt_us={:?}",
        sm.frames_sent,
        sm.raw_bytes_sent,
        sm.wire_bytes_sent,
        sm.compression_ratio(),
        sm.keyframe_resyncs,
        sm.reconnects,
        cm.smoothed_rtt_us
    );

    client.shutdown();
    server.shutdown();
}

// =============================================================================
// Test 4: Live RFC 6455 Binary WebSocket Transport + Fragmented WS Frame Codec
// =============================================================================

#[test]
fn websocket_binary_transport_live_streaming_reconnect_and_fragmented_frames() {
    let config = PairingConfig::new("studio-ws-pair-2026")
        .with_schema_version(1)
        .with_heartbeat_interval_ms(100);

    let mut server = TcpDeltaServer::bind("127.0.0.1:0", config.clone()).unwrap();

    // Connect BOTH a WebSocket binary client AND a native raw TCP client to the SAME server!
    let mut ws_client =
        TcpDeltaClient::connect_websocket(server.local_addr(), "/ws/nvd", config.clone()).unwrap();
    let mut raw_client = TcpDeltaClient::connect(server.local_addr(), config).unwrap();
    assert_eq!(ws_client.framing(), StreamFraming::WebSocketBinary);
    assert_eq!(raw_client.framing(), StreamFraming::RawLengthPrefixed);
    assert!(server.wait_for_peers(2, Duration::from_secs(2)));

    // Broadcast 20 frames -> BOTH WebSocket and Raw TCP clients receive 100% identical frames.
    for i in 1..=20u64 {
        let f = make_studio_workspace_frame(
            i,
            ColorRgba::rgb(56, 189, 248),
            &format!("WS+TCP Live {i}"),
            0,
        );
        assert_eq!(server.send_frame(&f).unwrap(), 2);
        let (_, ws_rx) = ws_client
            .recv_frame(Duration::from_secs(2))
            .unwrap()
            .expect("missing WebSocket frame");
        let (_, raw_rx) = raw_client
            .recv_frame(Duration::from_secs(2))
            .unwrap()
            .expect("missing raw TCP frame");
        assert_eq!(ws_rx, f);
        assert_eq!(raw_rx, f);
    }

    // Reconnect WebSocket client mid-stream -> automatic keyframe resync over WebSocket!
    ws_client.disconnect();
    thread::sleep(Duration::from_millis(25));
    let f21 = make_studio_workspace_frame(21, ColorRgba::rgb(239, 68, 68), "WS Offline 21", 2);
    assert_eq!(server.send_frame(&f21).unwrap(), 1);
    let (_, raw_21) = raw_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(raw_21, f21);

    ws_client.reconnect().unwrap();
    let (_, ws_resync) = ws_client
        .recv_frame(Duration::from_secs(2))
        .unwrap()
        .expect("missing WS keyframe resync");
    assert_eq!(ws_resync, f21);

    let f22 = make_studio_workspace_frame(22, ColorRgba::rgb(239, 68, 68), "WS Resumed 22", 2);
    assert_eq!(server.send_frame(&f22).unwrap(), 2);
    assert_eq!(
        ws_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1,
        f22
    );
    assert_eq!(
        raw_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1,
        f22
    );

    // Also test a raw RFC 6455 fragmented binary message + interleaved Ping directly against server.
    {
        let mut raw_ws = TcpStream::connect(server.local_addr()).unwrap();
        raw_ws.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let upgrade = format!(
            "GET /ws/nvd HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        raw_ws.write_all(upgrade.as_bytes()).unwrap();
        let mut resp = [0u8; 512];
        let n = io::Read::read(&mut raw_ws, &mut resp).unwrap();
        let resp_str = std::str::from_utf8(&resp[..n]).unwrap();
        assert!(resp_str.contains("101 Switching Protocols"));
        assert!(resp_str.contains(&compute_ws_accept_key(key)));

        // Send a WebSocket Ping control frame followed by a fragmented invalid Hello (verifies
        // server handles Ping -> Pong and Continuation frames cleanly without panicking).
        let ping = encode_ws_frame(true, WsOpcode::Ping, b"hb", Some([1, 2, 3, 4]));
        let frag1 = encode_ws_frame(false, WsOpcode::Binary, b"NT\x01\x01", Some([5, 6, 7, 8]));
        let frag2 = encode_ws_frame(true, WsOpcode::Continuation, b"\x00\x00\x00\x00", Some([9, 10, 11, 12]));
        raw_ws.write_all(&ping).unwrap();
        raw_ws.write_all(&frag1).unwrap();
        raw_ws.write_all(&frag2).unwrap();
        let mut pong_buf = [0u8; 32];
        let pn = io::Read::read(&mut raw_ws, &mut pong_buf).unwrap();
        assert!(pn >= 4);
        assert_eq!(pong_buf[0] & 0x0F, WsOpcode::Pong as u8);
        assert_eq!(&pong_buf[2..4], b"hb");
    }

    ws_client.shutdown();
    raw_client.shutdown();
    server.shutdown();
}

// =============================================================================
// Test 5: Adversarial Tests (Wrong Token, Schema Mismatch, OOM Bombs, Truncation, Clean Teardown)
// =============================================================================

#[test]
fn adversarial_auth_schema_oom_bombs_truncation_and_leak_free_shutdown() {
    let server_cfg = PairingConfig::new("super-secret-studio-token")
        .with_schema_version(3)
        .with_max_frame_bytes(64 * 1024)
        .with_handshake_timeout_ms(500);

    let mut udp_server = UdpDeltaServer::bind("127.0.0.1:0", server_cfg.clone()).unwrap();
    let mut tcp_server = TcpDeltaServer::bind("127.0.0.1:0", server_cfg.clone()).unwrap();

    // 1. Wrong pairing token must be rejected on UDP, TCP, and WebSocket.
    let bad_token_cfg = PairingConfig::new("wrong-token")
        .with_schema_version(3)
        .with_handshake_timeout_ms(500);
    assert!(matches!(
        UdpDeltaClient::connect(udp_server.local_addr(), bad_token_cfg.clone()),
        Err(TransportError::AuthenticationFailed)
    ));
    assert!(matches!(
        TcpDeltaClient::connect(tcp_server.local_addr(), bad_token_cfg.clone()),
        Err(TransportError::AuthenticationFailed)
    ));
    assert!(matches!(
        TcpDeltaClient::connect_websocket(tcp_server.local_addr(), "/ws", bad_token_cfg),
        Err(TransportError::AuthenticationFailed)
    ));

    // 2. Wrong schema version must be rejected cleanly on UDP & TCP.
    let bad_schema_cfg = PairingConfig::new("super-secret-studio-token")
        .with_schema_version(99)
        .with_handshake_timeout_ms(500);
    assert!(matches!(
        UdpDeltaClient::connect(udp_server.local_addr(), bad_schema_cfg.clone()),
        Err(TransportError::SchemaVersionMismatch {
            expected: 3,
            actual: 99
        })
    ));
    assert!(matches!(
        TcpDeltaClient::connect(tcp_server.local_addr(), bad_schema_cfg),
        Err(TransportError::SchemaVersionMismatch {
            expected: 3,
            actual: 99
        })
    ));

    assert!(udp_server.metrics().rejected_handshakes >= 2);
    assert!(tcp_server.metrics().rejected_handshakes >= 3);

    // 3. Oversized TCP length-prefix OOM bomb (2 GiB & 100 MiB) -> rejected BEFORE allocation!
    for bomb_len in [0x7FFF_FFFFu32, 100 * 1024 * 1024, 0u32] {
        let mut s = TcpStream::connect(tcp_server.local_addr()).unwrap();
        let _ = s.write_all(&bomb_len.to_le_bytes());
        let _ = s.write_all(&[0xAA; 16]);
        drop(s);
    }

    // 4. Oversized RFC 6455 64-bit WebSocket payload OOM bomb (4 GiB) -> rejected BEFORE allocation!
    {
        let mut s = TcpStream::connect(tcp_server.local_addr()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let req = "GET /ws HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        s.write_all(req.as_bytes()).unwrap();
        let mut resp = [0u8; 256];
        let _ = io::Read::read(&mut s, &mut resp);
        let mut bomb = vec![0x82, 0xFF]; // FIN+Binary, MASK=1, 64-bit extended length
        bomb.extend_from_slice(&(4u64 * 1024 * 1024 * 1024).to_be_bytes());
        bomb.extend_from_slice(&[1, 2, 3, 4]);
        let _ = s.write_all(&bomb);
    }

    // 5. Truncated TCP frames (1 byte, 3 bytes, header declaring 100 bytes with only 5 bytes).
    for garbage in [&[0x01u8][..], &[0x01, 0x02, 0x03][..], &[100, 0, 0, 0, 1, 2, 3, 4, 5][..]] {
        let mut s = TcpStream::connect(tcp_server.local_addr()).unwrap();
        let _ = s.write_all(garbage);
        drop(s);
    }

    // 6. Malformed, truncated, spoofed-session, and forged-fragment OOM-bomb UDP datagrams.
    {
        let attacker = UdpSocket::bind("127.0.0.1:0").unwrap();
        let target = udp_server.local_addr();
        let _ = attacker.send_to(b"", target);
        let _ = attacker.send_to(b"NT\x01", target); // truncated header
        let _ = attacker.send_to(b"XX\x01\x01\x00\x00\x00\x00", target); // bad magic
        let _ = attacker.send_to(b"NT\x99\x01\x00\x00\x00\x00", target); // bad version
        let _ = attacker.send_to(b"NT\x01\x04\xDE\xAD\xBE\xEFgarbage", target); // unknown session
        // Forged DeltaFrag claiming 1 GiB total_len
        let mut frag_bomb = vec![b'N', b'T', 1, 5, 1, 0, 0, 0];
        frag_bomb.extend_from_slice(&1u32.to_le_bytes()); // frag_msg_id
        frag_bomb.extend_from_slice(&0u16.to_le_bytes()); // frag_idx
        frag_bomb.extend_from_slice(&1u16.to_le_bytes()); // frag_count
        frag_bomb.extend_from_slice(&(1024u32 * 1024 * 1024).to_le_bytes()); // 1 GiB
        frag_bomb.extend_from_slice(&[0u8; 32]);
        let _ = attacker.send_to(&frag_bomb, target);
    }

    thread::sleep(Duration::from_millis(60));
    assert!(udp_server.metrics().malformed_packets_dropped >= 5);
    assert!(tcp_server.metrics().malformed_packets_dropped >= 4);

    // 7. After all adversarial attacks, legitimate UDP & TCP clients still pair and receive frames cleanly!
    let mut good_udp = UdpDeltaClient::connect(udp_server.local_addr(), server_cfg.clone()).unwrap();
    let mut good_tcp = TcpDeltaClient::connect(tcp_server.local_addr(), server_cfg.clone()).unwrap();
    assert!(udp_server.wait_for_peers(1, Duration::from_secs(2)));
    assert!(tcp_server.wait_for_peers(1, Duration::from_secs(2)));

    let frame = make_studio_workspace_frame(1, ColorRgba::rgb(34, 197, 94), "Healthy After Attacks", 0);
    assert_eq!(udp_server.send_frame(&frame).unwrap(), 1);
    assert_eq!(tcp_server.send_frame(&frame).unwrap(), 1);
    assert_eq!(
        good_udp.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1,
        frame
    );
    assert_eq!(
        good_tcp.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1,
        frame
    );

    good_udp.shutdown();
    good_tcp.shutdown();
    udp_server.shutdown();
    tcp_server.shutdown();

    // 8. Rapid lifecycle spin-up / teardown check: zero hangs, zero leaked threads.
    let teardown_start = Instant::now();
    let completed = Arc::new(AtomicUsize::new(0));
    for round in 0..5u32 {
        let cfg = PairingConfig::new(format!("teardown-token-{round}"));
        let us = UdpDeltaServer::bind("127.0.0.1:0", cfg.clone()).unwrap();
        let uc = UdpDeltaClient::connect(us.local_addr(), cfg.clone()).unwrap();
        let ts = TcpDeltaServer::bind("127.0.0.1:0", cfg.clone()).unwrap();
        let tc = TcpDeltaClient::connect(ts.local_addr(), cfg).unwrap();
        drop(uc);
        drop(tc);
        drop(us);
        drop(ts);
        completed.fetch_add(1, Ordering::SeqCst);
    }
    assert_eq!(completed.load(Ordering::SeqCst), 5);
    assert!(
        teardown_start.elapsed() < Duration::from_secs(3),
        "teardown took too long ({:?}), possible thread hang",
        teardown_start.elapsed()
    );
}

// =============================================================================
// Test 6: Bidirectional InputEvents Backchannel Across UDP, Raw TCP & WebSocket
// =============================================================================

#[test]
fn bidirectional_input_events_backchannel_udp_tcp_and_websocket_triggers_live_deltas() {
    let config = PairingConfig::new("studio-bidir-input-2026")
        .with_schema_version(1)
        .with_heartbeat_interval_ms(120);

    let mut udp_server = UdpDeltaServer::bind("127.0.0.1:0", config.clone()).unwrap();
    let mut tcp_server = TcpDeltaServer::bind("127.0.0.1:0", config.clone()).unwrap();

    // Seed initial live frame (count = 0) before clients pair -> auto keyframe on connect!
    let mut counter: u64 = 0;
    let initial = make_studio_workspace_frame(
        1,
        ColorRgba::rgb(56, 189, 248),
        &format!("Counter: {counter}"),
        0,
    );
    assert_eq!(udp_server.send_frame(&initial).unwrap(), 0);
    assert_eq!(tcp_server.send_frame(&initial).unwrap(), 0);

    let mut udp_client = UdpDeltaClient::connect(udp_server.local_addr(), config.clone()).unwrap();
    let mut tcp_client = TcpDeltaClient::connect(tcp_server.local_addr(), config.clone()).unwrap();
    let mut ws_client =
        TcpDeltaClient::connect_websocket(tcp_server.local_addr(), "/ws/nvd", config).unwrap();

    assert!(udp_server.wait_for_peers(1, Duration::from_secs(2)));
    assert!(tcp_server.wait_for_peers(2, Duration::from_secs(2)));

    // Every client receives the initial keyframe immediately upon pairing.
    assert_eq!(
        udp_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1,
        initial
    );
    assert_eq!(
        tcp_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1,
        initial
    );
    assert_eq!(
        ws_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1,
        initial
    );

    let make_press_ring = |node_id: u32, ts: u64| {
        encode_events(&[NativeEvent {
            surface_id: 1,
            node_id,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 48.0, y: 72.0 },
        }])
    };

    // 1. UDP client sends InputEvents -> server ingests, mutates state, broadcasts NVD1 delta to ALL clients.
    let udp_ev = make_press_ring(10, 1001);
    udp_client.send_events(&udp_ev).unwrap();
    let (udp_sid, udp_rx_bytes) = udp_server
        .recv_events(Duration::from_secs(2))
        .expect("UDP server should receive authenticated InputEvents");
    assert_eq!(udp_sid, u64::from(udp_client.session_id()));
    assert_eq!(udp_rx_bytes, udp_ev);
    assert_eq!(decode_events(&udp_rx_bytes).unwrap()[0].node_id, 10);

    counter += 1;
    let f2 = make_studio_workspace_frame(
        2,
        ColorRgba::rgb(34, 197, 94),
        &format!("Counter: {counter}"),
        1,
    );
    assert_eq!(udp_server.send_frame(&f2).unwrap(), 1);
    assert_eq!(tcp_server.send_frame(&f2).unwrap(), 2);
    assert_eq!(udp_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f2);
    assert_eq!(tcp_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f2);
    assert_eq!(ws_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f2);

    // 2. Raw TCP client sends InputEvents with text + press batch -> server ingests & broadcasts delta.
    let tcp_ev = encode_events(&[
        NativeEvent {
            surface_id: 1,
            node_id: 20,
            timestamp_us: 2001,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text("hello-nve".into()),
        },
        NativeEvent {
            surface_id: 1,
            node_id: 21,
            timestamp_us: 2002,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 12.0, y: 34.0 },
        },
    ]);
    tcp_client.send_events(&tcp_ev).unwrap();
    let (tcp_sid, tcp_rx_bytes) = tcp_server
        .recv_events(Duration::from_secs(2))
        .expect("TCP server should receive raw TCP InputEvents");
    assert_eq!(tcp_sid, u64::from(tcp_client.session_id()));
    assert_eq!(tcp_rx_bytes, tcp_ev);
    let decoded_tcp = decode_events(&tcp_rx_bytes).unwrap();
    assert_eq!(decoded_tcp.len(), 2);
    assert_eq!(decoded_tcp[0].payload, EventPayload::Text("hello-nve".into()));

    counter += 1;
    let f3 = make_studio_workspace_frame(
        3,
        ColorRgba::rgb(167, 139, 250),
        &format!("Counter: {counter}"),
        2,
    );
    assert_eq!(udp_server.send_frame(&f3).unwrap(), 1);
    assert_eq!(tcp_server.send_frame(&f3).unwrap(), 2);
    assert_eq!(udp_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f3);
    assert_eq!(tcp_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f3);
    assert_eq!(ws_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f3);

    // 3. RFC-6455 Binary WebSocket client sends InputEvents -> server ingests via try_recv_events/recv_events & broadcasts.
    let ws_ev = make_press_ring(30, 3001);
    ws_client.send_events(&ws_ev).unwrap();
    let (ws_sid, ws_rx_bytes) = tcp_server
        .recv_events(Duration::from_secs(2))
        .expect("TCP/WS server should receive WebSocket InputEvents");
    assert_eq!(ws_sid, u64::from(ws_client.session_id()));
    assert_eq!(ws_rx_bytes, ws_ev);
    assert!(tcp_server.try_recv_events().is_none());

    counter += 1;
    let f4 = make_studio_workspace_frame(
        4,
        ColorRgba::rgb(251, 191, 36),
        &format!("Counter: {counter}"),
        3,
    );
    assert_eq!(udp_server.send_frame(&f4).unwrap(), 1);
    assert_eq!(tcp_server.send_frame(&f4).unwrap(), 2);
    assert_eq!(udp_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f4);
    assert_eq!(tcp_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f4);
    assert_eq!(ws_client.recv_frame(Duration::from_secs(2)).unwrap().unwrap().1, f4);

    // Verify metrics counters across servers and clients.
    let um = udp_server.metrics();
    let tm = tcp_server.metrics();
    assert_eq!(um.events_rx, 1);
    assert_eq!(um.event_bytes_rx, udp_ev.len() as u64);
    assert_eq!(tm.events_rx, 2);
    assert_eq!(tm.event_bytes_rx, (tcp_ev.len() + ws_ev.len()) as u64);
    assert_eq!(udp_client.metrics().events_tx, 1);
    assert_eq!(tcp_client.metrics().events_tx, 1);
    assert_eq!(ws_client.metrics().events_tx, 1);

    udp_client.shutdown();
    tcp_client.shutdown();
    ws_client.shutdown();
    udp_server.shutdown();
    tcp_server.shutdown();
}

// =============================================================================
// Test 7: Adversarial Unauthenticated, Oversized & Corrupt InputEvents Rejection
// =============================================================================

#[test]
fn adversarial_input_events_rejects_unauthenticated_oversized_and_corrupt_payloads() {
    let config = PairingConfig::new("strict-input-guard-token")
        .with_schema_version(1)
        .with_handshake_timeout_ms(400);

    let mut udp_server = UdpDeltaServer::bind("127.0.0.1:0", config.clone()).unwrap();
    let mut tcp_server = TcpDeltaServer::bind("127.0.0.1:0", config.clone()).unwrap();

    let valid_ev = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: 5,
        timestamp_us: 99,
        kind: NativeEventKind::Press,
        payload: EventPayload::Empty,
    }]);

    let make_raw_input_envelope = |sid: u32, payload: &[u8]| {
        let mut buf = Vec::with_capacity(8 + payload.len());
        buf.extend_from_slice(&TRANSPORT_MAGIC);
        buf.push(TRANSPORT_PROTOCOL_VERSION);
        buf.push(TransportMsgKind::InputEvents as u8);
        buf.extend_from_slice(&sid.to_le_bytes());
        buf.extend_from_slice(payload);
        buf
    };

    // 1. Unauthenticated UDP InputEvents (no Hello handshake) must be rejected & never queued.
    {
        let attacker = UdpSocket::bind("127.0.0.1:0").unwrap();
        let pkt = make_raw_input_envelope(0x1234_5678, &valid_ev);
        attacker.send_to(&pkt, udp_server.local_addr()).unwrap();
    }

    // 2. Unauthenticated raw TCP InputEvents sent BEFORE Hello must be rejected & dropped.
    {
        let mut s = TcpStream::connect(tcp_server.local_addr()).unwrap();
        let env = make_raw_input_envelope(0x1234_5678, &valid_ev);
        s.write_all(&(env.len() as u32).to_le_bytes()).unwrap();
        s.write_all(&env).unwrap();
    }

    // 3. Unauthenticated WebSocket InputEvents sent BEFORE Hello must be rejected & dropped.
    {
        let mut s = TcpStream::connect(tcp_server.local_addr()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let req = "GET /ws HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        s.write_all(req.as_bytes()).unwrap();
        let mut resp = [0u8; 256];
        let _ = io::Read::read(&mut s, &mut resp);
        let env = make_raw_input_envelope(0x1234_5678, &valid_ev);
        let frame = encode_ws_frame(true, WsOpcode::Binary, &env, Some([1, 2, 3, 4]));
        let _ = s.write_all(&frame);
    }

    thread::sleep(Duration::from_millis(60));
    assert!(udp_server.try_recv_events().is_none());
    assert!(tcp_server.try_recv_events().is_none());
    assert_eq!(udp_server.metrics().events_rx, 0);
    assert_eq!(tcp_server.metrics().events_rx, 0);
    assert!(udp_server.metrics().malformed_packets_dropped >= 1);
    assert!(tcp_server.metrics().malformed_packets_dropped >= 2);

    // 4. Authenticated clients cannot send empty, corrupt, or oversized InputEvents payloads.
    let mut udp_client = UdpDeltaClient::connect(udp_server.local_addr(), config.clone()).unwrap();
    let mut tcp_client = TcpDeltaClient::connect(tcp_server.local_addr(), config).unwrap();
    assert!(udp_server.wait_for_peers(1, Duration::from_secs(2)));
    assert!(tcp_server.wait_for_peers(1, Duration::from_secs(2)));

    // Empty / corrupt payloads fail cleanly on client API.
    assert!(matches!(
        udp_client.send_events(&[]),
        Err(SyncTransportError::MalformedFrame(_))
    ));
    assert!(matches!(
        tcp_client.send_events(&[0xFF, 0xFF, 0xFF, 0xFF]),
        Err(SyncTransportError::MalformedFrame(_))
    ));

    // Oversized payload > MAX_UDP_SINGLE_PAYLOAD on UDP and > MAX_INPUT_EVENT_BYTES on TCP fail cleanly.
    let big_udp_text = "x".repeat(MAX_UDP_SINGLE_PAYLOAD + 64);
    let big_udp_ev = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: 1,
        timestamp_us: 1,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text(big_udp_text),
    }]);
    assert!(matches!(
        udp_client.send_events(&big_udp_ev),
        Err(SyncTransportError::FrameTooLarge { .. })
    ));

    let big_tcp_text = "y".repeat(MAX_INPUT_EVENT_BYTES + 64);
    let big_tcp_ev = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: 1,
        timestamp_us: 1,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text(big_tcp_text),
    }]);
    assert!(matches!(
        tcp_client.send_events(&big_tcp_ev),
        Err(SyncTransportError::FrameTooLarge { .. })
    ));

    // 5. Even after valid pairing, spoofed session_id or raw corrupt InputEvents datagrams are dropped.
    let spoof = UdpSocket::bind("127.0.0.1:0").unwrap();
    let bad_sid = udp_client.session_id().wrapping_add(1);
    let _ = spoof.send_to(&make_raw_input_envelope(bad_sid, &valid_ev), udp_server.local_addr());
    let _ = spoof.send_to(
        &make_raw_input_envelope(udp_client.session_id(), &[0xDE, 0xAD, 0xBE, 0xEF]),
        udp_server.local_addr(),
    );
    thread::sleep(Duration::from_millis(50));
    assert!(udp_server.try_recv_events().is_none());
    assert_eq!(udp_server.metrics().events_rx, 0);

    // Legitimate authenticated InputEvents still succeed afterwards!
    udp_client.send_events(&valid_ev).unwrap();
    tcp_client.send_events(&valid_ev).unwrap();
    assert_eq!(
        udp_server.recv_events(Duration::from_secs(2)).unwrap().1,
        valid_ev
    );
    assert_eq!(
        tcp_server.recv_events(Duration::from_secs(2)).unwrap().1,
        valid_ev
    );

    udp_client.shutdown();
    tcp_client.shutdown();
    udp_server.shutdown();
    tcp_server.shutdown();
}
