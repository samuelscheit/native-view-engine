//! Low-latency UDP delta transport (`UdpDeltaServer` & `UdpDeltaClient`) with MTU-safe
//! fragmentation, SACK/NACK/FragNack retransmission, RTT probing, and `InputEvents` backchannel.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::sync_delta::{
    DeltaPacketRef, PacketKind, SyncDeltaError, SyncDeltaSession, DELTA_HEADER_SIZE,
};

use super::frag::{packetize_udp_datagrams, FragReassembler, MAX_CACHED_FRAG_MESSAGES};
use super::metrics::{
    accept_server_hello, recv_frame_timeout, ClientSharedState, MetricsTracker, PairingMetrics,
};
use super::protocol::{
    build_hello_envelope, decode_frag_nack, decode_keyframe_request, decode_reject,
    decode_timestamp_us, derive_token_proof, encode_envelope, encode_frag_nack,
    encode_keyframe_request, encode_ping_or_pong, encode_reject, next_nonce, reject_to_error,
    validate_input_events_payload, verify_hello_ack, EventQueue, HelloPayload, PairingConfig,
    TransportEnvelopeRef, TransportError, TransportMsgKind, MAX_UDP_SINGLE_PAYLOAD,
};

struct UdpPeerState {
    session_id: u32,
    session: SyncDeltaSession,
    frag_cache: VecDeque<(u32, Vec<Vec<u8>>)>,
    last_activity: Instant,
    last_ping_sent: Instant,
}

impl UdpPeerState {
    fn cache_fragments(&mut self, frag_msg_id: u32, dgrams: Vec<Vec<u8>>) {
        if dgrams.len() <= 1 {
            return;
        }
        if let Some(pos) = self.frag_cache.iter().position(|(id, _)| *id == frag_msg_id) {
            self.frag_cache.remove(pos);
        }
        while self.frag_cache.len() >= MAX_CACHED_FRAG_MESSAGES {
            self.frag_cache.pop_front();
        }
        self.frag_cache.push_back((frag_msg_id, dgrams));
    }

    fn lookup_frag(&self, frag_msg_id: u32, frag_idx: u16) -> Option<&[u8]> {
        self.frag_cache
            .iter()
            .find(|(id, _)| *id == frag_msg_id)
            .and_then(|(_, dgrams)| dgrams.get(usize::from(frag_idx)))
            .map(Vec::as_slice)
    }
}

struct UdpServerInner {
    config: PairingConfig,
    expected_proof: [u8; 16],
    started_at: Instant,
    peers: HashMap<SocketAddr, UdpPeerState>,
    seen_peers_total: u64,
    latest_frame: Option<Vec<u8>>,
    metrics: MetricsTracker,
}

impl UdpServerInner {
    fn now_ms(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_millis() as u64
    }
    fn now_us(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_micros() as u64
    }

    fn send_nvd_to_peer(
        socket: &UdpSocket,
        addr: SocketAddr,
        peer: &mut UdpPeerState,
        metrics: &mut MetricsTracker,
        nvd_packet: &[u8],
        now: Instant,
    ) -> Result<(), TransportError> {
        let dgrams = packetize_udp_datagrams(peer.session_id, nvd_packet);
        for dgram in &dgrams {
            socket.send_to(dgram, addr)?;
            metrics.record_tx(now, dgram.len());
        }
        if dgrams.len() > 1 && nvd_packet.len() >= DELTA_HEADER_SIZE {
            peer.cache_fragments(u32::from_le_bytes(nvd_packet[8..12].try_into().unwrap()), dgrams);
        }
        Ok(())
    }

    fn send_keyframe_resync(&mut self, socket: &UdpSocket, addr: SocketAddr, now: Instant) {
        let Some(latest) = self.latest_frame.clone() else { return };
        let now_ms = self.now_ms(now);
        if let Some(peer) = self.peers.get_mut(&addr) {
            let kf = peer.session.encode_keyframe(&latest, now_ms);
            if Self::send_nvd_to_peer(socket, addr, peer, &mut self.metrics, &kf, now).is_ok() {
                self.metrics.record_frame_sent(PacketKind::Keyframe, latest.len(), true);
            }
        }
    }
}

/// Low-latency UDP delta server with MTU-safe packetization (`<= 1200` B), SACK/NACK/FragNack
/// retransmit pump, heartbeat RTT estimation, multi-peer reconnect/resync, and `InputEvents` queue.
pub struct UdpDeltaServer {
    local_addr: SocketAddr,
    socket: UdpSocket,
    inner: Arc<(Mutex<UdpServerInner>, Condvar)>,
    events: Arc<EventQueue>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl UdpDeltaServer {
    pub fn bind(addr: impl ToSocketAddrs, config: PairingConfig) -> Result<Self, TransportError> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(5)))?;
        let local_addr = socket.local_addr()?;
        let worker_sock = socket.try_clone()?;

        let expected_proof = derive_token_proof(&config.pairing_token, config.schema_version);
        let metrics = MetricsTracker::new(
            config.schema_version,
            Duration::from_millis(config.metrics_window_ms),
        );
        let inner = Arc::new((
            Mutex::new(UdpServerInner {
                config,
                expected_proof,
                started_at: Instant::now(),
                peers: HashMap::new(),
                seen_peers_total: 0,
                latest_frame: None,
                metrics,
            }),
            Condvar::new(),
        ));
        let events = Arc::new(EventQueue::new());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (wi, we, ws) = (Arc::clone(&inner), Arc::clone(&events), Arc::clone(&shutdown));
        let worker = thread::Builder::new()
            .name("nve-udp-delta-server".into())
            .spawn(move || udp_server_loop(worker_sock, wi, we, ws))?;
        Ok(Self { local_addr, socket, inner, events, shutdown, worker: Some(worker) })
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
        let addrs: Vec<SocketAddr> = guard.peers.keys().copied().collect();
        let mut sent = 0usize;
        for addr in addrs {
            let state = &mut *guard;
            if let Some(peer) = state.peers.get_mut(&addr) {
                let nvd_pkt = peer.session.encode_frame(frame_bytes, now_ms);
                let kind = DeltaPacketRef::parse(&nvd_pkt).map(|p| p.header().kind).unwrap_or(PacketKind::Keyframe);
                if UdpServerInner::send_nvd_to_peer(&self.socket, addr, peer, &mut state.metrics, &nvd_pkt, now).is_ok() {
                    state.metrics.record_frame_sent(kind, frame_bytes.len(), false);
                    sent += 1;
                }
            }
        }
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
        if let Some(handle) = self.worker.take() { let _ = handle.join(); }
    }
}

impl Drop for UdpDeltaServer {
    fn drop(&mut self) { self.shutdown(); }
}

fn udp_server_loop(
    socket: UdpSocket,
    inner: Arc<(Mutex<UdpServerInner>, Condvar)>,
    events: Arc<EventQueue>,
    shutdown: Arc<AtomicBool>,
) {
    let mut buf = [0u8; 2048];
    let mut last_tick = Instant::now();

    while !shutdown.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buf) {
            Ok((n, src)) => {
                let now = Instant::now();
                let mut guard = inner.0.lock().unwrap();
                guard.metrics.record_rx(now, n);
                let Ok(env) = TransportEnvelopeRef::parse(&buf[..n]) else {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                };
                match env.kind {
                    TransportMsgKind::Hello => {
                        let Ok(hello) = HelloPayload::decode(env.payload) else {
                            guard.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        match accept_server_hello(
                            hello, &guard.config, &guard.expected_proof, 0x55AA_55AA, guard.latest_frame.is_some(),
                        ) {
                            Err(reason) => {
                                guard.metrics.snapshot.rejected_handshakes += 1;
                                let rej = encode_reject(reason, guard.config.schema_version);
                                if socket.send_to(&rej, src).is_ok() { guard.metrics.record_tx(now, rej.len()); }
                            }
                            Ok(acc) => {
                                if guard.peers.contains_key(&src) || guard.seen_peers_total > 0 {
                                    guard.metrics.snapshot.reconnects += 1;
                                }
                                guard.seen_peers_total += 1;
                                guard.peers.insert(src, UdpPeerState {
                                    session_id: acc.session_id,
                                    session: SyncDeltaSession::new(acc.window),
                                    frag_cache: VecDeque::new(),
                                    last_activity: now,
                                    last_ping_sent: now,
                                });
                                guard.metrics.snapshot.session_id = acc.session_id;
                                guard.metrics.snapshot.connected_peers = guard.peers.len();
                                if socket.send_to(&acc.ack_envelope, src).is_ok() {
                                    guard.metrics.record_tx(now, acc.ack_envelope.len());
                                }
                                let ping = encode_ping_or_pong(TransportMsgKind::Ping, acc.session_id, guard.now_us(now));
                                if socket.send_to(&ping, src).is_ok() { guard.metrics.record_tx(now, ping.len()); }
                                guard.send_keyframe_resync(&socket, src, now);
                                inner.1.notify_all();
                            }
                        }
                    }
                    TransportMsgKind::DeltaSingle => {
                        let now_ms = guard.now_ms(now);
                        let state = &mut *guard;
                        let Some(peer) = state.peers.get_mut(&src).filter(|p| p.session_id == env.session_id) else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        peer.last_activity = now;
                        let Ok(parsed) = DeltaPacketRef::parse(env.payload) else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        match parsed.header().kind {
                            PacketKind::Ack if peer.session.handle_ack(env.payload).is_ok() => {
                                state.metrics.snapshot.acks_received += 1;
                            }
                            PacketKind::Nack => {
                                state.metrics.snapshot.nacks_received += 1;
                                if let Ok(Some(retx)) = peer.session.handle_nack(env.payload, now_ms) {
                                    let is_kf = DeltaPacketRef::parse(&retx).is_ok_and(|p| p.header().kind == PacketKind::Keyframe);
                                    if UdpServerInner::send_nvd_to_peer(&socket, src, peer, &mut state.metrics, &retx, now).is_ok() {
                                        state.metrics.snapshot.retransmits_sent += 1;
                                        if is_kf { state.metrics.snapshot.keyframe_resyncs += 1; }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    TransportMsgKind::FragNack => {
                        let state = &mut *guard;
                        let Some(peer) = state.peers.get_mut(&src).filter(|p| p.session_id == env.session_id) else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        peer.last_activity = now;
                        if let Ok((fid, missing)) = decode_frag_nack(env.payload) {
                            state.metrics.snapshot.nacks_received += 1;
                            for idx in missing {
                                if let Some(dg) = peer.lookup_frag(fid, idx) {
                                    if socket.send_to(dg, src).is_ok() {
                                        state.metrics.record_tx(now, dg.len());
                                        state.metrics.snapshot.retransmits_sent += 1;
                                    }
                                }
                            }
                        } else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                        }
                    }
                    TransportMsgKind::KeyframeRequest => {
                        let authed = guard.peers.get_mut(&src)
                            .is_some_and(|p| (p.session_id == env.session_id).then(|| p.last_activity = now).is_some());
                        if !authed || decode_keyframe_request(env.payload).is_err() {
                            guard.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        guard.send_keyframe_resync(&socket, src, now);
                    }
                    TransportMsgKind::InputEvents => {
                        let max_ev = guard.config.clamped_max_event_bytes().min(MAX_UDP_SINGLE_PAYLOAD);
                        let authed = guard.peers.get_mut(&src)
                            .is_some_and(|p| (p.session_id == env.session_id).then(|| p.last_activity = now).is_some());
                        if !authed || validate_input_events_payload(env.payload, max_ev).is_err() {
                            guard.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        guard.metrics.record_event_rx(env.payload.len());
                        events.push(u64::from(env.session_id), env.payload.to_vec());
                    }
                    TransportMsgKind::Ping => {
                        if let Some(peer) = guard.peers.get_mut(&src).filter(|p| p.session_id == env.session_id) {
                            peer.last_activity = now;
                            if let Ok(ts) = decode_timestamp_us(env.payload) {
                                let pong = encode_ping_or_pong(TransportMsgKind::Pong, env.session_id, ts);
                                if socket.send_to(&pong, src).is_ok() { guard.metrics.record_tx(now, pong.len()); }
                            }
                        }
                    }
                    TransportMsgKind::Pong => {
                        let now_us = guard.now_us(now);
                        if let Some(peer) = guard.peers.get_mut(&src).filter(|p| p.session_id == env.session_id) {
                            peer.last_activity = now;
                            if let Ok(sent_us) = decode_timestamp_us(env.payload) {
                                guard.metrics.record_rtt_us(now_us.saturating_sub(sent_us));
                            }
                        }
                    }
                    TransportMsgKind::Bye => {
                        if guard.peers.get(&src).is_some_and(|p| p.session_id == env.session_id) {
                            guard.peers.remove(&src);
                            guard.metrics.snapshot.connected_peers = guard.peers.len();
                        }
                    }
                    TransportMsgKind::HelloAck | TransportMsgKind::Reject | TransportMsgKind::DeltaFrag => {
                        guard.metrics.snapshot.malformed_packets_dropped += 1;
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }

        let now = Instant::now();
        if now.saturating_duration_since(last_tick) >= Duration::from_millis(8) {
            last_tick = now;
            let mut guard = inner.0.lock().unwrap();
            let base_rto = guard.config.rto_ms;
            let rto = guard.metrics.snapshot.smoothed_rtt_us
                .map(|us| base_rto.max((us / 1000).saturating_mul(3).saturating_add(25)))
                .unwrap_or(base_rto);
            let hb = Duration::from_millis(guard.config.heartbeat_interval_ms);
            let (now_ms, now_us) = (guard.now_ms(now), guard.now_us(now));
            let addrs: Vec<SocketAddr> = guard.peers.keys().copied().collect();
            for addr in addrs {
                let state = &mut *guard;
                if let Some(peer) = state.peers.get_mut(&addr) {
                    for retx in peer.session.poll_retransmits(now_ms, rto) {
                        if UdpServerInner::send_nvd_to_peer(&socket, addr, peer, &mut state.metrics, &retx, now).is_ok() {
                            state.metrics.snapshot.retransmits_sent += 1;
                        }
                    }
                    if now.saturating_duration_since(peer.last_ping_sent) >= hb {
                        peer.last_ping_sent = now;
                        let ping = encode_ping_or_pong(TransportMsgKind::Ping, peer.session_id, now_us);
                        if socket.send_to(&ping, addr).is_ok() { state.metrics.record_tx(now, ping.len()); }
                    }
                }
            }
        }
    }
}

/// Low-latency UDP delta client with handshake retry, fragment reassembly, SACK/NACK, RTT, and `InputEvents`.
pub struct UdpDeltaClient {
    server_addr: SocketAddr,
    socket: Option<UdpSocket>,
    shared: Arc<Mutex<ClientSharedState>>,
    frame_rx: mpsc::Receiver<(u32, Vec<u8>)>,
    frame_tx: mpsc::Sender<(u32, Vec<u8>)>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl UdpDeltaClient {
    pub fn connect(server_addr: SocketAddr, config: PairingConfig) -> Result<Self, TransportError> {
        let (frame_tx, frame_rx) = mpsc::channel();
        let mut client = Self {
            server_addr,
            socket: None,
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
        let bind_addr = if self.server_addr.is_ipv6() { "[::1]:0" } else { "127.0.0.1:0" };
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(20)))?;

        let (config, client_last_seq) = {
            let g = self.shared.lock().unwrap();
            (g.config.clone(), g.session.last_applied_seq())
        };
        let client_nonce = next_nonce(u64::from(client_last_seq) ^ 0x1357_9BDF);
        let hello_pkt = build_hello_envelope(&config, client_last_seq, client_nonce);
        let deadline = Instant::now() + Duration::from_millis(config.handshake_timeout_ms);
        let (mut last_send, mut buf) = (None::<Instant>, [0u8; 2048]);

        let (sid, win) = loop {
            let now = Instant::now();
            if now >= deadline { return Err(TransportError::HandshakeTimeout); }
            if last_send.map_or(true, |t| now.saturating_duration_since(t) >= Duration::from_millis(25)) {
                socket.send_to(&hello_pkt, self.server_addr)?;
                self.shared.lock().unwrap().metrics.record_tx(now, hello_pkt.len());
                last_send = Some(now);
            }
            match socket.recv_from(&mut buf) {
                Ok((n, _)) => {
                    self.shared.lock().unwrap().metrics.record_rx(Instant::now(), n);
                    let Ok(env) = TransportEnvelopeRef::parse(&buf[..n]) else { continue };
                    match env.kind {
                        TransportMsgKind::HelloAck => break verify_hello_ack(env, &config, client_nonce)?,
                        TransportMsgKind::Reject => {
                            let (reason, exp) = decode_reject(env.payload)?;
                            return Err(reject_to_error(reason, exp, config.schema_version));
                        }
                        _ => {}
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut => {}
                Err(err) => return Err(TransportError::Io(err)),
            }
        };

        {
            let mut guard = self.shared.lock().unwrap();
            guard.on_connected(sid, win, is_reconnect);
            let now = Instant::now();
            let ping = encode_ping_or_pong(TransportMsgKind::Ping, sid, guard.now_us(now));
            let _ = socket.send_to(&ping, self.server_addr);
            guard.metrics.record_tx(now, ping.len());
        }

        socket.set_read_timeout(Some(Duration::from_millis(5)))?;
        let worker_sock = socket.try_clone()?;
        self.socket = Some(socket);
        self.shutdown = Arc::new(AtomicBool::new(false));
        let (ws, wt, wsd, sa) = (Arc::clone(&self.shared), self.frame_tx.clone(), Arc::clone(&self.shutdown), self.server_addr);
        self.worker = Some(
            thread::Builder::new()
                .name("nve-udp-delta-client".into())
                .spawn(move || udp_client_loop(worker_sock, sa, ws, wt, wsd))?,
        );
        Ok(sid)
    }

    pub fn local_addr(&self) -> Option<SocketAddr> { self.socket.as_ref().and_then(|s| s.local_addr().ok()) }
    pub fn server_addr(&self) -> SocketAddr { self.server_addr }
    pub fn session_id(&self) -> u32 { self.shared.lock().unwrap().session_id }
    pub fn is_connected(&self) -> bool { self.shared.lock().unwrap().connected }
    pub fn recv_frame(&self, timeout: Duration) -> Result<Option<(u32, Vec<u8>)>, TransportError> {
        recv_frame_timeout(&self.frame_rx, timeout)
    }
    pub fn try_recv_frame(&self) -> Option<(u32, Vec<u8>)> { self.frame_rx.try_recv().ok() }

    /// Sends a raw binary encoded `EventRing` batch back to the paired UDP server (`InputEvents = 11`).
    pub fn send_events(&self, encoded_event_ring: &[u8]) -> Result<(), TransportError> {
        let Some(socket) = self.socket.as_ref() else { return Err(TransportError::Disconnected) };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        if !guard.connected { return Err(TransportError::Disconnected); }
        let max_bytes = guard.config.clamped_max_event_bytes().min(MAX_UDP_SINGLE_PAYLOAD);
        validate_input_events_payload(encoded_event_ring, max_bytes)?;
        let pkt = encode_envelope(TransportMsgKind::InputEvents, guard.session_id, encoded_event_ring);
        socket.send_to(&pkt, self.server_addr)?;
        guard.metrics.record_tx(now, pkt.len());
        guard.metrics.record_event_tx();
        Ok(())
    }

    pub fn request_keyframe(&self) -> Result<(), TransportError> {
        let Some(socket) = self.socket.as_ref() else { return Err(TransportError::Disconnected) };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        let pkt = encode_keyframe_request(guard.session_id, guard.session.last_applied_seq(), 1);
        socket.send_to(&pkt, self.server_addr)?;
        guard.metrics.record_tx(now, pkt.len());
        guard.metrics.snapshot.keyframe_resyncs += 1;
        Ok(())
    }

    pub fn send_ping(&self) -> Result<(), TransportError> {
        let Some(socket) = self.socket.as_ref() else { return Err(TransportError::Disconnected) };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        let ping = encode_ping_or_pong(TransportMsgKind::Ping, guard.session_id, guard.now_us(now));
        socket.send_to(&ping, self.server_addr)?;
        guard.metrics.record_tx(now, ping.len());
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Some(socket) = self.socket.as_ref() {
            let sid = self.shared.lock().unwrap().session_id;
            let _ = socket.send_to(&encode_envelope(TransportMsgKind::Bye, sid, &[]), self.server_addr);
        }
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() { let _ = handle.join(); }
        self.socket = None;
        self.shared.lock().unwrap().on_disconnected();
    }

    pub fn reconnect(&mut self) -> Result<u32, TransportError> {
        self.disconnect();
        self.establish_connection(true)
    }

    pub fn metrics(&self) -> PairingMetrics { self.shared.lock().unwrap().metrics.snapshot(Instant::now()) }
    pub fn shutdown(&mut self) { self.disconnect(); }
}

impl Drop for UdpDeltaClient {
    fn drop(&mut self) { self.disconnect(); }
}

fn udp_client_loop(
    socket: UdpSocket,
    server_addr: SocketAddr,
    shared: Arc<Mutex<ClientSharedState>>,
    frame_tx: mpsc::Sender<(u32, Vec<u8>)>,
    shutdown: Arc<AtomicBool>,
) {
    let mut reassembler = FragReassembler::new(shared.lock().unwrap().config.clamped_max_frame_bytes());
    let mut nack_cooldowns: HashMap<u32, Instant> = HashMap::new();
    let mut last_kf_req: Option<Instant> = None;
    let (mut last_tick, mut last_ping_sent) = (Instant::now(), Instant::now());
    let mut buf = [0u8; 2048];

    while !shutdown.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buf) {
            Ok((n, _)) => {
                let now = Instant::now();
                let mut guard = shared.lock().unwrap();
                guard.metrics.record_rx(now, n);
                let sid = guard.session_id;
                let Ok(env) = TransportEnvelopeRef::parse(&buf[..n]) else {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                };
                if env.session_id != sid {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                }
                match env.kind {
                    TransportMsgKind::DeltaSingle => handle_udp_nvd_packet(
                        &socket, server_addr, &mut guard, env.payload, now, &frame_tx, &mut nack_cooldowns, &mut last_kf_req,
                    ),
                    TransportMsgKind::DeltaFrag => match reassembler.push(env.payload, now) {
                        Ok((Some(full), _)) => handle_udp_nvd_packet(
                            &socket, server_addr, &mut guard, &full, now, &frame_tx, &mut nack_cooldowns, &mut last_kf_req,
                        ),
                        Ok((None, Some((mid, miss)))) => {
                            let fnack = encode_frag_nack(sid, mid, &miss);
                            if socket.send_to(&fnack, server_addr).is_ok() {
                                guard.metrics.record_tx(now, fnack.len());
                                guard.metrics.snapshot.nacks_sent += 1;
                            }
                        }
                        Ok((None, None)) => {}
                        Err(_) => guard.metrics.snapshot.malformed_packets_dropped += 1,
                    },
                    TransportMsgKind::Ping => {
                        if let Ok(ts) = decode_timestamp_us(env.payload) {
                            let pong = encode_ping_or_pong(TransportMsgKind::Pong, sid, ts);
                            if socket.send_to(&pong, server_addr).is_ok() { guard.metrics.record_tx(now, pong.len()); }
                        }
                    }
                    TransportMsgKind::Pong => {
                        if let Ok(sent_us) = decode_timestamp_us(env.payload) {
                            let rtt = guard.now_us(now).saturating_sub(sent_us);
                            guard.metrics.record_rtt_us(rtt);
                        }
                    }
                    TransportMsgKind::Bye => { guard.on_disconnected(); break; }
                    _ => guard.metrics.snapshot.malformed_packets_dropped += 1,
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }

        let now = Instant::now();
        if now.saturating_duration_since(last_tick) >= Duration::from_millis(10) {
            last_tick = now;
            let mut guard = shared.lock().unwrap();
            let sid = guard.session_id;
            let hb = Duration::from_millis(guard.config.heartbeat_interval_ms);
            for (mid, miss) in reassembler.poll_stale_nacks(now, 15) {
                let fnack = encode_frag_nack(sid, mid, &miss);
                if socket.send_to(&fnack, server_addr).is_ok() {
                    guard.metrics.record_tx(now, fnack.len());
                    guard.metrics.snapshot.nacks_sent += 1;
                }
            }
            if let Some(mseq) = guard.session.next_missing_seq() {
                if nack_cooldowns.get(&mseq).map_or(true, |t| now.saturating_duration_since(*t) >= Duration::from_millis(18)) {
                    nack_cooldowns.insert(mseq, now);
                    let env = encode_envelope(TransportMsgKind::DeltaSingle, sid, &guard.session.build_nack_packet(mseq));
                    if socket.send_to(&env, server_addr).is_ok() {
                        guard.metrics.record_tx(now, env.len());
                        guard.metrics.snapshot.nacks_sent += 1;
                    }
                }
            }
            if now.saturating_duration_since(last_ping_sent) >= hb {
                last_ping_sent = now;
                let ping = encode_ping_or_pong(TransportMsgKind::Ping, sid, guard.now_us(now));
                if socket.send_to(&ping, server_addr).is_ok() { guard.metrics.record_tx(now, ping.len()); }
            }
            if nack_cooldowns.len() > 128 {
                nack_cooldowns.retain(|_, ts| now.saturating_duration_since(*ts) < Duration::from_secs(2));
            }
        }
    }
    reassembler.clear();
}

#[allow(clippy::too_many_arguments)]
fn handle_udp_nvd_packet(
    socket: &UdpSocket,
    server_addr: SocketAddr,
    guard: &mut ClientSharedState,
    nvd_bytes: &[u8],
    now: Instant,
    frame_tx: &mpsc::Sender<(u32, Vec<u8>)>,
    nack_cooldowns: &mut HashMap<u32, Instant>,
    last_kf_req: &mut Option<Instant>,
) {
    let sid = guard.session_id;
    let Ok(parsed) = DeltaPacketRef::parse(nvd_bytes) else {
        guard.metrics.snapshot.malformed_packets_dropped += 1;
        return;
    };
    let hdr = parsed.header();
    let (last_applied, max_win) = (guard.session.last_applied_seq(), guard.session.max_window() as u32);
    let mut send_kf_req = |guard: &mut ClientSharedState, reason: u8| {
        if last_kf_req.map_or(true, |t| now.saturating_duration_since(t) >= Duration::from_millis(25)) {
            *last_kf_req = Some(now);
            let req = encode_keyframe_request(sid, guard.session.last_applied_seq(), reason);
            if socket.send_to(&req, server_addr).is_ok() {
                guard.metrics.record_tx(now, req.len());
                guard.metrics.snapshot.keyframe_resyncs += 1;
            }
        }
    };

    if hdr.kind != PacketKind::Keyframe && hdr.seq > last_applied && hdr.seq.saturating_sub(last_applied) > max_win {
        send_kf_req(guard, 2);
        return;
    }
    match guard.session.receive_packet(nvd_bytes) {
        Ok(outcome) => {
            for (seq, frame) in outcome.applied_frames {
                nack_cooldowns.remove(&seq);
                guard.metrics.snapshot.frames_received += 1;
                guard.metrics.snapshot.raw_bytes_received += frame.len() as u64;
                let _ = frame_tx.send((seq, frame));
            }
            if let Some(ack_nvd) = outcome.ack_packet {
                let ack_env = encode_envelope(TransportMsgKind::DeltaSingle, sid, &ack_nvd);
                if socket.send_to(&ack_env, server_addr).is_ok() {
                    guard.metrics.record_tx(now, ack_env.len());
                    guard.metrics.snapshot.acks_sent += 1;
                }
            }
            for nack_nvd in outcome.nack_packets {
                let missing_seq = DeltaPacketRef::parse(&nack_nvd)
                    .ok()
                    .and_then(|p| (p.payload().len() == 4).then(|| u32::from_le_bytes(p.payload().try_into().unwrap())))
                    .unwrap_or(0);
                if hdr.seq.saturating_sub(missing_seq) < 2 { continue; }
                if nack_cooldowns.get(&missing_seq).map_or(true, |t| now.saturating_duration_since(*t) >= Duration::from_millis(25)) {
                    nack_cooldowns.insert(missing_seq, now);
                    let nack_env = encode_envelope(TransportMsgKind::DeltaSingle, sid, &nack_nvd);
                    if socket.send_to(&nack_env, server_addr).is_ok() {
                        guard.metrics.record_tx(now, nack_env.len());
                        guard.metrics.snapshot.nacks_sent += 1;
                    }
                }
            }
        }
        Err(SyncDeltaError::BaseSequenceMismatch { .. }) => send_kf_req(guard, 3),
        Err(_) => guard.metrics.snapshot.malformed_packets_dropped += 1,
    }
}
