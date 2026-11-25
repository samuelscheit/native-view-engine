//! Framed binary TCP + auto-detected RFC 6455 binary WebSocket delta transport
//! (`TcpDeltaServer` & `TcpDeltaClient`) with mid-stream reconnect and `InputEvents` backchannel.

use std::collections::HashMap;
use std::io;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::sync_delta::{DeltaPacketRef, PacketKind, SyncDeltaError, SyncDeltaSession};

use super::metrics::{
    accept_server_hello, recv_frame_timeout, ClientSharedState, MetricsTracker, PairingMetrics,
};
use super::protocol::{
    build_hello_envelope, decode_keyframe_request, decode_reject, decode_timestamp_us,
    derive_token_proof, encode_envelope, encode_keyframe_request, encode_ping_or_pong,
    encode_reject, next_nonce, reject_to_error, validate_input_events_payload, verify_hello_ack,
    EventQueue, HelloPayload, PairingConfig, TransportEnvelopeRef, TransportError,
    TransportMsgKind,
};
use super::ws_codec::{
    client_perform_ws_upgrade, detect_stream_framing, read_framed_message_eof_aware,
    server_handle_ws_upgrade, write_framed_message, StreamFraming, StreamReadOutcome,
};

struct TcpPeerEntry {
    id: u64,
    session_id: u32,
    framing: StreamFraming,
    write_stream: Arc<Mutex<TcpStream>>,
    session: SyncDeltaSession,
}

struct TcpServerInner {
    config: PairingConfig,
    expected_proof: [u8; 16],
    started_at: Instant,
    next_peer_id: u64,
    seen_peers_total: u64,
    peers: HashMap<u64, TcpPeerEntry>,
    latest_frame: Option<Vec<u8>>,
    metrics: MetricsTracker,
}

impl TcpServerInner {
    fn now_ms(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_millis() as u64
    }
    fn now_us(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_micros() as u64
    }
}

/// Framed binary TCP + auto-detected RFC 6455 binary WebSocket delta server with `InputEvents` queue.
pub struct TcpDeltaServer {
    local_addr: SocketAddr,
    inner: Arc<(Mutex<TcpServerInner>, Condvar)>,
    events: Arc<EventQueue>,
    shutdown: Arc<AtomicBool>,
    accept_worker: Option<JoinHandle<()>>,
    peer_workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl TcpDeltaServer {
    pub fn bind(addr: impl ToSocketAddrs, config: PairingConfig) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;

        let expected_proof = derive_token_proof(&config.pairing_token, config.schema_version);
        let metrics = MetricsTracker::new(
            config.schema_version,
            Duration::from_millis(config.metrics_window_ms),
        );
        let inner = Arc::new((
            Mutex::new(TcpServerInner {
                config,
                expected_proof,
                started_at: Instant::now(),
                next_peer_id: 1,
                seen_peers_total: 0,
                peers: HashMap::new(),
                latest_frame: None,
                metrics,
            }),
            Condvar::new(),
        ));
        let events = Arc::new(EventQueue::new());
        let shutdown = Arc::new(AtomicBool::new(false));
        let peer_workers = Arc::new(Mutex::new(Vec::new()));
        let (ai, ae, asd, aw) = (
            Arc::clone(&inner), Arc::clone(&events), Arc::clone(&shutdown), Arc::clone(&peer_workers),
        );
        let accept_worker = thread::Builder::new()
            .name("nve-tcp-delta-accept".into())
            .spawn(move || tcp_accept_loop(listener, ai, ae, asd, aw))?;
        Ok(Self {
            local_addr, inner, events, shutdown, accept_worker: Some(accept_worker), peer_workers,
        })
    }

    pub fn local_addr(&self) -> SocketAddr { self.local_addr }
    pub fn peer_count(&self) -> usize { self.inner.0.lock().unwrap().peers.len() }

    pub fn wait_for_peers(&self, min_peers: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut guard = self.inner.0.lock().unwrap();
        while guard.peers.len() < min_peers {
            let rem = deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() { return false; }
            guard = self.inner.1.wait_timeout(guard, rem).unwrap().0;
        }
        true
    }

    pub fn send_frame(&self, frame_bytes: &[u8]) -> Result<usize, TransportError> {
        let now = Instant::now();
        let mut guard = self.inner.0.lock().unwrap();
        let max_bytes = guard.config.clamped_max_frame_bytes();
        if frame_bytes.len() > max_bytes {
            return Err(TransportError::FrameTooLarge { declared: frame_bytes.len(), max: max_bytes });
        }
        guard.latest_frame = Some(frame_bytes.to_vec());
        let now_ms = guard.now_ms(now);
        let peer_ids: Vec<u64> = guard.peers.keys().copied().collect();
        let (mut dead, mut sent) = (Vec::new(), 0usize);

        for pid in peer_ids {
            let state = &mut *guard;
            if let Some(peer) = state.peers.get_mut(&pid) {
                let nvd_pkt = peer.session.encode_frame(frame_bytes, now_ms);
                let kind = DeltaPacketRef::parse(&nvd_pkt).map(|p| p.header().kind).unwrap_or(PacketKind::Keyframe);
                let env = encode_envelope(TransportMsgKind::DeltaSingle, peer.session_id, &nvd_pkt);
                let written = peer.write_stream.lock().ok()
                    .and_then(|mut ws| write_framed_message(&mut ws, peer.framing, false, &env).ok());
                if let Some(wire_len) = written {
                    state.metrics.record_tx(now, wire_len);
                    state.metrics.record_frame_sent(kind, frame_bytes.len(), false);
                    sent += 1;
                } else {
                    dead.push(pid);
                }
            }
        }
        for pid in dead { guard.peers.remove(&pid); }
        guard.metrics.snapshot.connected_peers = guard.peers.len();
        Ok(sent)
    }

    pub fn try_recv_events(&self) -> Option<(u64, Vec<u8>)> { self.events.try_recv() }
    pub fn recv_events(&self, timeout: Duration) -> Option<(u64, Vec<u8>)> {
        self.events.recv_timeout(timeout, &self.shutdown)
    }
    pub fn metrics(&self) -> PairingMetrics {
        let mut guard = self.inner.0.lock().unwrap();
        guard.metrics.snapshot.connected_peers = guard.peers.len();
        guard.metrics.snapshot(Instant::now())
    }

    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.events.notify_all();
        for peer in self.inner.0.lock().unwrap().peers.values() {
            if let Ok(ws) = peer.write_stream.lock() { let _ = ws.shutdown(Shutdown::Both); }
        }
        if let Some(handle) = self.accept_worker.take() { let _ = handle.join(); }
        for handle in std::mem::take(&mut *self.peer_workers.lock().unwrap()) { let _ = handle.join(); }
    }
}

impl Drop for TcpDeltaServer {
    fn drop(&mut self) { self.shutdown(); }
}

fn tcp_accept_loop(
    listener: TcpListener,
    inner: Arc<(Mutex<TcpServerInner>, Condvar)>,
    events: Arc<EventQueue>,
    shutdown: Arc<AtomicBool>,
    peer_workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let (pi, pe, ps) = (Arc::clone(&inner), Arc::clone(&events), Arc::clone(&shutdown));
                if let Ok(handle) = thread::Builder::new()
                    .name("nve-tcp-delta-peer".into())
                    .spawn(move || tcp_server_peer_worker(stream, pi, pe, ps))
                {
                    let mut workers = peer_workers.lock().unwrap();
                    let mut i = 0;
                    while i < workers.len() {
                        if workers[i].is_finished() { let _ = workers.swap_remove(i).join(); } else { i += 1; }
                    }
                    workers.push(handle);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
            Err(_) => break,
        }
    }
}

fn tcp_server_peer_worker(
    mut read_stream: TcpStream,
    inner: Arc<(Mutex<TcpServerInner>, Condvar)>,
    events: Arc<EventQueue>,
    shutdown: Arc<AtomicBool>,
) {
    let _ = read_stream.set_nonblocking(false);
    let _ = read_stream.set_nodelay(true);
    let _ = read_stream.set_read_timeout(Some(Duration::from_millis(10)));

    let (config, expected_proof) = {
        let g = inner.0.lock().unwrap();
        (g.config.clone(), g.expected_proof)
    };
    let handshake_timeout = Duration::from_millis(config.handshake_timeout_ms);
    let (max_frame_bytes, max_event_bytes) = (config.clamped_max_frame_bytes(), config.clamped_max_event_bytes());

    let Ok(framing) = detect_stream_framing(&read_stream, handshake_timeout, &shutdown) else {
        inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
        return;
    };
    if framing == StreamFraming::WebSocketBinary
        && server_handle_ws_upgrade(&mut read_stream, handshake_timeout, &shutdown).is_err()
    {
        inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
        return;
    }
    let Ok(write_clone) = read_stream.try_clone() else { return };
    let write_stream = Arc::new(Mutex::new(write_clone));

    // Only Hello is accepted prior to authentication (unauthenticated InputEvents are dropped).
    let deadline = Instant::now() + handshake_timeout;
    let hello_bytes = loop {
        if shutdown.load(Ordering::Relaxed) || Instant::now() >= deadline { return; }
        match read_framed_message_eof_aware(
            &mut read_stream, &write_stream, framing, false, max_frame_bytes, &shutdown,
        ) {
            Ok((StreamReadOutcome::Frame(bytes), wire_len)) => {
                inner.0.lock().unwrap().metrics.record_rx(Instant::now(), wire_len);
                break bytes;
            }
            Ok((StreamReadOutcome::Idle, _)) => continue,
            Ok((StreamReadOutcome::Closed, _)) | Err(_) => {
                inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
                return;
            }
        }
    };

    let Ok(env) = TransportEnvelopeRef::parse(&hello_bytes) else {
        inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
        return;
    };
    let Ok(hello) = (if env.kind == TransportMsgKind::Hello {
        HelloPayload::decode(env.payload)
    } else {
        Err(TransportError::MalformedFrame("expected Hello"))
    }) else {
        inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
        return;
    };

    let now = Instant::now();
    let (peer_id, session_id) = {
        let mut guard = inner.0.lock().unwrap();
        let acc = match accept_server_hello(
            hello, &config, &expected_proof, 0xCAFE_BABE, guard.latest_frame.is_some(),
        ) {
            Ok(acc) => acc,
            Err(reason) => {
                let rej = encode_reject(reason, config.schema_version);
                if let Ok(mut ws) = write_stream.lock() { let _ = write_framed_message(&mut ws, framing, false, &rej); }
                guard.metrics.snapshot.rejected_handshakes += 1;
                guard.metrics.record_tx(Instant::now(), rej.len());
                return;
            }
        };
        let pid = guard.next_peer_id;
        guard.next_peer_id += 1;
        if guard.seen_peers_total > 0 { guard.metrics.snapshot.reconnects += 1; }
        guard.seen_peers_total += 1;

        let mut peer_session = SyncDeltaSession::new(acc.window);
        if let Ok(mut ws) = write_stream.lock() {
            if let Ok(n) = write_framed_message(&mut ws, framing, false, &acc.ack_envelope) {
                guard.metrics.record_tx(now, n);
            }
            let ping = encode_ping_or_pong(TransportMsgKind::Ping, acc.session_id, guard.now_us(now));
            if let Ok(n) = write_framed_message(&mut ws, framing, false, &ping) {
                guard.metrics.record_tx(now, n);
            }
            if let Some(latest) = guard.latest_frame.clone() {
                let kf = peer_session.encode_keyframe(&latest, guard.now_ms(now));
                let kf_env = encode_envelope(TransportMsgKind::DeltaSingle, acc.session_id, &kf);
                if let Ok(n) = write_framed_message(&mut ws, framing, false, &kf_env) {
                    guard.metrics.record_tx(now, n);
                    guard.metrics.record_frame_sent(PacketKind::Keyframe, latest.len(), true);
                }
            }
        }
        guard.peers.insert(pid, TcpPeerEntry {
            id: pid, session_id: acc.session_id, framing, write_stream: Arc::clone(&write_stream), session: peer_session,
        });
        guard.metrics.snapshot.session_id = acc.session_id;
        guard.metrics.snapshot.connected_peers = guard.peers.len();
        inner.1.notify_all();
        (pid, acc.session_id)
    };

    let hb_interval = Duration::from_millis(config.heartbeat_interval_ms);
    let mut last_ping_sent = Instant::now();

    while !shutdown.load(Ordering::Relaxed) {
        match read_framed_message_eof_aware(
            &mut read_stream, &write_stream, framing, false, max_frame_bytes, &shutdown,
        ) {
            Ok((StreamReadOutcome::Frame(bytes), wire_len)) => {
                let now = Instant::now();
                let mut guard = inner.0.lock().unwrap();
                guard.metrics.record_rx(now, wire_len);
                let Ok(env) = TransportEnvelopeRef::parse(&bytes) else {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                };
                if env.session_id != session_id {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                }
                match env.kind {
                    TransportMsgKind::DeltaSingle => {
                        let now_ms = guard.now_ms(now);
                        let state = &mut *guard;
                        if let Some(peer) = state.peers.get_mut(&peer_id) {
                            if let Ok(parsed) = DeltaPacketRef::parse(env.payload) {
                                match parsed.header().kind {
                                    PacketKind::Ack if peer.session.handle_ack(env.payload).is_ok() => {
                                        state.metrics.snapshot.acks_received += 1;
                                    }
                                    PacketKind::Nack => {
                                        state.metrics.snapshot.nacks_received += 1;
                                        if let Ok(Some(retx)) = peer.session.handle_nack(env.payload, now_ms) {
                                            let retx_env = encode_envelope(TransportMsgKind::DeltaSingle, session_id, &retx);
                                            if let Ok(mut ws) = peer.write_stream.lock()
                                                && let Ok(n) = write_framed_message(&mut ws, peer.framing, false, &retx_env)
                                            {
                                                state.metrics.record_tx(now, n);
                                                state.metrics.snapshot.retransmits_sent += 1;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            } else {
                                state.metrics.snapshot.malformed_packets_dropped += 1;
                            }
                        }
                    }
                    TransportMsgKind::KeyframeRequest => {
                        if decode_keyframe_request(env.payload).is_err() {
                            guard.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        let (now_ms, latest_opt) = (guard.now_ms(now), guard.latest_frame.clone());
                        let state = &mut *guard;
                        if let (Some(peer), Some(latest)) = (state.peers.get_mut(&peer_id), latest_opt) {
                            let kf = peer.session.encode_keyframe(&latest, now_ms);
                            let kf_env = encode_envelope(TransportMsgKind::DeltaSingle, session_id, &kf);
                            if let Ok(mut ws) = peer.write_stream.lock()
                                && let Ok(n) = write_framed_message(&mut ws, peer.framing, false, &kf_env)
                            {
                                state.metrics.record_tx(now, n);
                                state.metrics.record_frame_sent(PacketKind::Keyframe, latest.len(), true);
                            }
                        }
                    }
                    TransportMsgKind::InputEvents => {
                        if validate_input_events_payload(env.payload, max_event_bytes).is_err() {
                            guard.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        guard.metrics.record_event_rx(env.payload.len());
                        events.push(u64::from(session_id), env.payload.to_vec());
                    }
                    TransportMsgKind::Ping => {
                        if let Ok(ts) = decode_timestamp_us(env.payload) {
                            let pong = encode_ping_or_pong(TransportMsgKind::Pong, session_id, ts);
                            if let Ok(mut ws) = write_stream.lock()
                                && let Ok(n) = write_framed_message(&mut ws, framing, false, &pong)
                            {
                                guard.metrics.record_tx(now, n);
                            }
                        }
                    }
                    TransportMsgKind::Pong => {
                        if let Ok(sent_us) = decode_timestamp_us(env.payload) {
                            let rtt = guard.now_us(now).saturating_sub(sent_us);
                            guard.metrics.record_rtt_us(rtt);
                        }
                    }
                    TransportMsgKind::Bye => break,
                    _ => guard.metrics.snapshot.malformed_packets_dropped += 1,
                }
            }
            Ok((StreamReadOutcome::Idle, _)) => {
                let now = Instant::now();
                if now.saturating_duration_since(last_ping_sent) >= hb_interval {
                    last_ping_sent = now;
                    let mut guard = inner.0.lock().unwrap();
                    let ping = encode_ping_or_pong(TransportMsgKind::Ping, session_id, guard.now_us(now));
                    if let Ok(mut ws) = write_stream.lock() {
                        if let Ok(n) = write_framed_message(&mut ws, framing, false, &ping) {
                            guard.metrics.record_tx(now, n);
                        } else {
                            break;
                        }
                    }
                }
            }
            Ok((StreamReadOutcome::Closed, _)) => break,
            Err(_) => { inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1; break; }
        }
    }

    let mut guard = inner.0.lock().unwrap();
    if let Some(removed) = guard.peers.remove(&peer_id) { let _ = removed.id; }
    guard.metrics.snapshot.connected_peers = guard.peers.len();
}

/// Framed binary TCP / RFC 6455 binary WebSocket delta client with reconnect and `InputEvents`.
pub struct TcpDeltaClient {
    server_addr: SocketAddr,
    framing: StreamFraming,
    ws_path: String,
    write_stream: Option<Arc<Mutex<TcpStream>>>,
    shared: Arc<Mutex<ClientSharedState>>,
    frame_rx: mpsc::Receiver<(u32, Vec<u8>)>,
    frame_tx: mpsc::Sender<(u32, Vec<u8>)>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TcpDeltaClient {
    pub fn connect(server_addr: SocketAddr, config: PairingConfig) -> Result<Self, TransportError> {
        Self::connect_with_framing(server_addr, StreamFraming::RawLengthPrefixed, "/", config)
    }

    pub fn connect_websocket(
        server_addr: SocketAddr,
        path: &str,
        config: PairingConfig,
    ) -> Result<Self, TransportError> {
        Self::connect_with_framing(server_addr, StreamFraming::WebSocketBinary, path, config)
    }

    fn connect_with_framing(
        server_addr: SocketAddr,
        framing: StreamFraming,
        ws_path: &str,
        config: PairingConfig,
    ) -> Result<Self, TransportError> {
        let (frame_tx, frame_rx) = mpsc::channel();
        let mut client = Self {
            server_addr,
            framing,
            ws_path: ws_path.to_string(),
            write_stream: None,
            shared: Arc::new(Mutex::new(ClientSharedState::new(config))),
            frame_rx,
            frame_tx,
            shutdown: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        client.establish_connection(false)?;
        Ok(client)
    }

    fn establish_connection(&mut self, is_reconnect: bool) -> Result<u32, TransportError> {
        let (config, client_last_seq) = {
            let g = self.shared.lock().unwrap();
            (g.config.clone(), g.session.last_applied_seq())
        };
        let handshake_timeout = Duration::from_millis(config.handshake_timeout_ms);
        let max_frame_bytes = config.clamped_max_frame_bytes();

        let mut read_stream = TcpStream::connect_timeout(&self.server_addr, handshake_timeout)?;
        read_stream.set_nodelay(true)?;
        read_stream.set_read_timeout(Some(Duration::from_millis(10)))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        if self.framing == StreamFraming::WebSocketBinary {
            let host = self.server_addr.to_string();
            client_perform_ws_upgrade(&mut read_stream, &host, &self.ws_path, handshake_timeout, &shutdown)?;
        }
        let write_stream = Arc::new(Mutex::new(read_stream.try_clone()?));
        let client_nonce = next_nonce(u64::from(client_last_seq) ^ 0x2468_ACE0);
        let hello_env = build_hello_envelope(&config, client_last_seq, client_nonce);
        {
            let n = write_framed_message(&mut write_stream.lock().unwrap(), self.framing, true, &hello_env)?;
            self.shared.lock().unwrap().metrics.record_tx(Instant::now(), n);
        }

        let deadline = Instant::now() + handshake_timeout;
        let (sid, win) = loop {
            if Instant::now() >= deadline { return Err(TransportError::HandshakeTimeout); }
            match read_framed_message_eof_aware(
                &mut read_stream, &write_stream, self.framing, true, max_frame_bytes, &shutdown,
            )? {
                (StreamReadOutcome::Frame(bytes), wire_len) => {
                    self.shared.lock().unwrap().metrics.record_rx(Instant::now(), wire_len);
                    let env = TransportEnvelopeRef::parse(&bytes)?;
                    match env.kind {
                        TransportMsgKind::HelloAck => break verify_hello_ack(env, &config, client_nonce)?,
                        TransportMsgKind::Reject => {
                            let (reason, exp) = decode_reject(env.payload)?;
                            return Err(reject_to_error(reason, exp, config.schema_version));
                        }
                        _ => return Err(TransportError::MalformedFrame("unexpected handshake reply")),
                    }
                }
                (StreamReadOutcome::Idle, _) => continue,
                (StreamReadOutcome::Closed, _) => return Err(TransportError::Disconnected),
            }
        };

        {
            let mut guard = self.shared.lock().unwrap();
            guard.on_connected(sid, win, is_reconnect);
            let now = Instant::now();
            let ping = encode_ping_or_pong(TransportMsgKind::Ping, sid, guard.now_us(now));
            if let Ok(mut ws) = write_stream.lock()
                && let Ok(n) = write_framed_message(&mut ws, self.framing, true, &ping)
            {
                guard.metrics.record_tx(now, n);
            }
        }

        self.write_stream = Some(Arc::clone(&write_stream));
        self.shutdown = Arc::clone(&shutdown);
        let (ws, wt, wf) = (Arc::clone(&self.shared), self.frame_tx.clone(), self.framing);
        self.worker = Some(
            thread::Builder::new()
                .name("nve-tcp-delta-client".into())
                .spawn(move || tcp_client_loop(read_stream, write_stream, wf, ws, wt, shutdown))?,
        );
        Ok(sid)
    }

    pub fn framing(&self) -> StreamFraming { self.framing }
    pub fn session_id(&self) -> u32 { self.shared.lock().unwrap().session_id }
    pub fn is_connected(&self) -> bool { self.shared.lock().unwrap().connected }
    pub fn recv_frame(&self, timeout: Duration) -> Result<Option<(u32, Vec<u8>)>, TransportError> {
        recv_frame_timeout(&self.frame_rx, timeout)
    }
    pub fn try_recv_frame(&self) -> Option<(u32, Vec<u8>)> { self.frame_rx.try_recv().ok() }

    /// Sends a raw binary encoded `EventRing` batch back to the paired TCP/WS server (`InputEvents = 11`).
    pub fn send_events(&self, encoded_event_ring: &[u8]) -> Result<(), TransportError> {
        let Some(ws_arc) = self.write_stream.as_ref() else { return Err(TransportError::Disconnected) };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        if !guard.connected { return Err(TransportError::Disconnected); }
        validate_input_events_payload(encoded_event_ring, guard.config.clamped_max_event_bytes())?;
        let pkt = encode_envelope(TransportMsgKind::InputEvents, guard.session_id, encoded_event_ring);
        let n = write_framed_message(&mut ws_arc.lock().unwrap(), self.framing, true, &pkt)?;
        guard.metrics.record_tx(now, n);
        guard.metrics.record_event_tx();
        Ok(())
    }

    pub fn request_keyframe(&self) -> Result<(), TransportError> {
        let Some(ws_arc) = self.write_stream.as_ref() else { return Err(TransportError::Disconnected) };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        let pkt = encode_keyframe_request(guard.session_id, guard.session.last_applied_seq(), 1);
        let n = write_framed_message(&mut ws_arc.lock().unwrap(), self.framing, true, &pkt)?;
        guard.metrics.record_tx(now, n);
        guard.metrics.snapshot.keyframe_resyncs += 1;
        Ok(())
    }

    pub fn send_ping(&self) -> Result<(), TransportError> {
        let Some(ws_arc) = self.write_stream.as_ref() else { return Err(TransportError::Disconnected) };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        let ping = encode_ping_or_pong(TransportMsgKind::Ping, guard.session_id, guard.now_us(now));
        let n = write_framed_message(&mut ws_arc.lock().unwrap(), self.framing, true, &ping)?;
        guard.metrics.record_tx(now, n);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Some(ws_arc) = self.write_stream.take() {
            let sid = self.shared.lock().unwrap().session_id;
            let bye = encode_envelope(TransportMsgKind::Bye, sid, &[]);
            if let Ok(mut ws) = ws_arc.lock() {
                let _ = write_framed_message(&mut ws, self.framing, true, &bye);
                let _ = ws.shutdown(Shutdown::Both);
            }
        }
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() { let _ = handle.join(); }
        self.shared.lock().unwrap().on_disconnected();
    }

    pub fn reconnect(&mut self) -> Result<u32, TransportError> {
        self.disconnect();
        self.establish_connection(true)
    }

    pub fn metrics(&self) -> PairingMetrics { self.shared.lock().unwrap().metrics.snapshot(Instant::now()) }
    pub fn shutdown(&mut self) { self.disconnect(); }
}

impl Drop for TcpDeltaClient {
    fn drop(&mut self) { self.disconnect(); }
}

fn tcp_client_loop(
    mut read_stream: TcpStream,
    write_stream: Arc<Mutex<TcpStream>>,
    framing: StreamFraming,
    shared: Arc<Mutex<ClientSharedState>>,
    frame_tx: mpsc::Sender<(u32, Vec<u8>)>,
    shutdown: Arc<AtomicBool>,
) {
    let (max_frame_bytes, hb_interval) = {
        let g = shared.lock().unwrap();
        (g.config.clamped_max_frame_bytes(), Duration::from_millis(g.config.heartbeat_interval_ms))
    };
    let mut last_ping_sent = Instant::now();

    while !shutdown.load(Ordering::Relaxed) {
        match read_framed_message_eof_aware(
            &mut read_stream, &write_stream, framing, true, max_frame_bytes, &shutdown,
        ) {
            Ok((StreamReadOutcome::Frame(bytes), wire_len)) => {
                let now = Instant::now();
                let mut guard = shared.lock().unwrap();
                guard.metrics.record_rx(now, wire_len);
                let sid = guard.session_id;
                let Ok(env) = TransportEnvelopeRef::parse(&bytes) else {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                };
                if env.session_id != sid {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                }
                match env.kind {
                    TransportMsgKind::DeltaSingle => match guard.session.receive_packet(env.payload) {
                        Ok(outcome) => {
                            for (seq, frame) in outcome.applied_frames {
                                guard.metrics.snapshot.frames_received += 1;
                                guard.metrics.snapshot.raw_bytes_received += frame.len() as u64;
                                let _ = frame_tx.send((seq, frame));
                            }
                            if let Some(ack_nvd) = outcome.ack_packet {
                                let ack_env = encode_envelope(TransportMsgKind::DeltaSingle, sid, &ack_nvd);
                                if let Ok(mut ws) = write_stream.lock()
                                    && let Ok(n) = write_framed_message(&mut ws, framing, true, &ack_env)
                                {
                                    guard.metrics.record_tx(now, n);
                                    guard.metrics.snapshot.acks_sent += 1;
                                }
                            }
                        }
                        Err(SyncDeltaError::BaseSequenceMismatch { .. }) => {
                            let req = encode_keyframe_request(sid, guard.session.last_applied_seq(), 3);
                            if let Ok(mut ws) = write_stream.lock()
                                && let Ok(n) = write_framed_message(&mut ws, framing, true, &req)
                            {
                                guard.metrics.record_tx(now, n);
                                guard.metrics.snapshot.keyframe_resyncs += 1;
                            }
                        }
                        Err(_) => guard.metrics.snapshot.malformed_packets_dropped += 1,
                    },
                    TransportMsgKind::Ping => {
                        if let Ok(ts) = decode_timestamp_us(env.payload) {
                            let pong = encode_ping_or_pong(TransportMsgKind::Pong, sid, ts);
                            if let Ok(mut ws) = write_stream.lock()
                                && let Ok(n) = write_framed_message(&mut ws, framing, true, &pong)
                            {
                                guard.metrics.record_tx(now, n);
                            }
                        }
                    }
                    TransportMsgKind::Pong => {
                        if let Ok(sent_us) = decode_timestamp_us(env.payload) {
                            let rtt = guard.now_us(now).saturating_sub(sent_us);
                            guard.metrics.record_rtt_us(rtt);
                        }
                    }
                    TransportMsgKind::Bye => break,
                    _ => guard.metrics.snapshot.malformed_packets_dropped += 1,
                }
            }
            Ok((StreamReadOutcome::Idle, _)) => {
                let now = Instant::now();
                if now.saturating_duration_since(last_ping_sent) >= hb_interval {
                    last_ping_sent = now;
                    let mut guard = shared.lock().unwrap();
                    let ping = encode_ping_or_pong(TransportMsgKind::Ping, guard.session_id, guard.now_us(now));
                    if let Ok(mut ws) = write_stream.lock() {
                        if let Ok(n) = write_framed_message(&mut ws, framing, true, &ping) {
                            guard.metrics.record_tx(now, n);
                        } else {
                            break;
                        }
                    }
                }
            }
            Ok((StreamReadOutcome::Closed, _)) => break,
            Err(_) => { shared.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1; break; }
        }
    }
    shared.lock().unwrap().on_disconnected();
}
