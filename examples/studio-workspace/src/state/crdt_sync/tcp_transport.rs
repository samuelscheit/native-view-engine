//! Live loopback QUIC/WSS multi-peer binary CRDT transport built directly on
//! `nve_wire::{NveWireServer, NveWireClient}` (`NveWireFrame` v2 `StateSync` / `CrdtDelta`).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::thread;
use std::time::{Duration, Instant};

use nve_interop::crdt::PeerId;
use nve_wire::{
    NveWireChannel, NveWireClient, NveWireClientConfig, NveWireFrame, NveWireKind, NveWireMetrics,
    NveWireServer, NveWireServerConfig, NveWireServerMetrics, NveWireTransportKind, NveWireUri,
};

use super::transport::{CrdtPeerNode, CrdtPeerTransport, CrdtTransportError};
use super::StudioCrdtDoc;

struct RemoteWirePeer {
    addr: SocketAddr,
    cert_sha256: [u8; 32],
    client: Option<NveWireClient>,
}

/// Real OS loopback QUIC/WSS CRDT peer transport backed by `nve-wire`.
pub struct LoopbackTcpCrdtTransport {
    peer: PeerId,
    bearer_token: String,
    server: NveWireServer,
    remotes: BTreeMap<PeerId, RemoteWirePeer>,
    partitioned: bool,
    seq: u32,
}

impl LoopbackTcpCrdtTransport {
    /// Binds a live dual-ALPN `NveWireServer` on `127.0.0.1:0` with `bearer_token`.
    pub fn bind(peer: PeerId, bearer_token: impl Into<String>) -> Result<Self, CrdtTransportError> {
        let token = bearer_token.into();
        let loopback = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0));
        let cfg = NveWireServerConfig::new(loopback, loopback, token.clone());
        let server = NveWireServer::bind_sync(cfg)?;
        Ok(Self {
            peer,
            bearer_token: token,
            server,
            remotes: BTreeMap::new(),
            partitioned: false,
            seq: 1,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.server.quic_addr()
    }

    pub fn cert_sha256(&self) -> [u8; 32] {
        self.server.cert_sha256()
    }

    fn connect_remote(
        &self,
        addr: SocketAddr,
        cert_sha256: [u8; 32],
    ) -> Result<NveWireClient, CrdtTransportError> {
        let uri = NveWireUri {
            host: addr.ip().to_string(),
            quic_port: Some(addr.port()),
            wss_port: None,
            path: "/nve-wire".to_owned(),
            transport: NveWireTransportKind::Quic,
            token: self.bearer_token.clone(),
            cert_sha256: Some(cert_sha256),
            width: None,
            height: None,
        };
        let cfg = NveWireClientConfig::new(self.bearer_token.clone(), format!("studio-peer-{}", self.peer.0))
            .with_cert_sha256(cert_sha256);
        Ok(NveWireClient::connect_uri_sync(&uri, &cfg)?)
    }

    pub fn connect_peer_pinned(
        &mut self,
        remote_peer: PeerId,
        addr: SocketAddr,
        cert_sha256: [u8; 32],
    ) -> Result<(), CrdtTransportError> {
        let client = self.connect_remote(addr, cert_sha256)?;
        self.remotes.insert(
            remote_peer,
            RemoteWirePeer {
                addr,
                cert_sha256,
                client: Some(client),
            },
        );
        Ok(())
    }

    pub fn disconnect_peer(&mut self, remote_peer: PeerId) {
        if let Some(link) = self.remotes.get_mut(&remote_peer)
            && let Some(client) = link.client.take()
        {
            client.close();
        }
    }

    pub fn reconnect_peer(&mut self, remote_peer: PeerId) -> Result<(), CrdtTransportError> {
        let (addr, cert_sha256) = {
            let Some(link) = self.remotes.get_mut(&remote_peer) else {
                return Err(CrdtTransportError::UnknownPeer(remote_peer));
            };
            if let Some(old) = link.client.take() {
                old.close();
            }
            (link.addr, link.cert_sha256)
        };
        let client = self.connect_remote(addr, cert_sha256)?;
        if let Some(link) = self.remotes.get_mut(&remote_peer) {
            link.client = Some(client);
        }
        Ok(())
    }

    pub fn partition(&mut self) {
        self.partitioned = true;
        for link in self.remotes.values_mut() {
            if let Some(client) = link.client.take() {
                client.close();
            }
        }
    }

    pub fn heal(&mut self) -> Result<(), CrdtTransportError> {
        self.partitioned = false;
        let peer_ids: Vec<PeerId> = self.remotes.keys().copied().collect();
        for remote_id in peer_ids {
            if self
                .remotes
                .get(&remote_id)
                .is_none_or(|l| l.client.as_ref().is_none_or(|c| !c.is_connected()))
            {
                self.reconnect_peer(remote_id)?;
            }
        }
        Ok(())
    }

    pub fn wait_for_inbound_peers(&self, min_peers: usize, timeout: Duration) -> bool {
        self.server.wait_for_peers(min_peers, timeout)
    }

    pub fn send_raw_wire_frame(&mut self, raw_bytes: &[u8]) -> Result<usize, CrdtTransportError> {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1).max(1);
        let frame = NveWireFrame::new(NveWireKind::CrdtDelta, seq, raw_bytes.to_vec())
            .with_channel(NveWireChannel::StateSync);
        Ok(self.server.broadcast_frame(&frame)?)
    }

    pub fn server_wire_metrics(&self) -> NveWireServerMetrics {
        self.server.metrics()
    }

    pub fn client_wire_metrics(&self, remote_peer: PeerId) -> Option<NveWireMetrics> {
        self.remotes
            .get(&remote_peer)
            .and_then(|l| l.client.as_ref())
            .map(NveWireClient::metrics)
    }

    pub fn shutdown(&mut self) {
        for link in self.remotes.values_mut() {
            if let Some(client) = link.client.take() {
                client.close();
            }
        }
        self.server.shutdown();
    }
}

impl Drop for LoopbackTcpCrdtTransport {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl CrdtPeerTransport for LoopbackTcpCrdtTransport {
    fn local_peer(&self) -> PeerId {
        self.peer
    }

    fn send_frame(
        &mut self,
        _target: Option<PeerId>,
        frame: Vec<u8>,
    ) -> Result<usize, CrdtTransportError> {
        if self.partitioned {
            return Ok(0);
        }
        self.send_raw_wire_frame(&frame)
    }

    fn poll_incoming(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        if self.partitioned {
            for link in self.remotes.values_mut() {
                if let Some(client) = link.client.as_ref() {
                    while client.try_recv_frame().is_some() {}
                }
            }
            return Vec::new();
        }
        let mut out = Vec::new();
        for (&remote_peer, link) in &self.remotes {
            if let Some(client) = link.client.as_ref() {
                while let Some(wire_frame) = client.try_recv_frame() {
                    if wire_frame.kind == NveWireKind::CrdtDelta {
                        out.push((remote_peer, wire_frame.payload));
                    }
                }
            }
        }
        out
    }

    fn connected_peer_count(&self) -> usize {
        if self.partitioned {
            return 0;
        }
        self.remotes
            .values()
            .filter(|l| l.client.as_ref().is_some_and(NveWireClient::is_connected))
            .count()
    }

    fn is_partitioned(&self) -> bool {
        self.partitioned
    }

    fn set_partitioned(&mut self, partitioned: bool) {
        if partitioned {
            self.partition();
        } else {
            let _ = self.heal();
        }
    }

    fn known_remote_peers(&self) -> Vec<PeerId> {
        self.remotes.keys().copied().collect()
    }
}

/// Multi-peer loopback QUIC CRDT mesh orchestrator for N live [`StudioCrdtDoc`] peers.
pub struct LoopbackTcpCrdtCluster {
    nodes: Vec<CrdtPeerNode<LoopbackTcpCrdtTransport>>,
}

impl LoopbackTcpCrdtCluster {
    pub fn spawn_full_mesh(
        docs: Vec<StudioCrdtDoc>,
        pairing_token: &str,
    ) -> Result<Self, CrdtTransportError> {
        let mut nodes = Vec::with_capacity(docs.len());
        for doc in docs {
            let transport = LoopbackTcpCrdtTransport::bind(doc.peer_id(), pairing_token)?;
            nodes.push(CrdtPeerNode::new(doc, transport));
        }

        let addrs: Vec<(PeerId, SocketAddr, [u8; 32])> = nodes
            .iter()
            .map(|n| {
                (
                    n.peer_id(),
                    n.transport().local_addr(),
                    n.transport().cert_sha256(),
                )
            })
            .collect();

        for (i, node) in nodes.iter_mut().enumerate() {
            for (j, &(remote_id, addr, sha256)) in addrs.iter().enumerate() {
                if i != j {
                    node.transport_mut()
                        .connect_peer_pinned(remote_id, addr, sha256)?;
                }
            }
        }

        let expected_inbound = nodes.len().saturating_sub(1);
        for node in &nodes {
            if !node
                .transport()
                .wait_for_inbound_peers(expected_inbound, Duration::from_secs(2))
            {
                return Err(CrdtTransportError::Timeout);
            }
        }

        Ok(Self { nodes })
    }

    pub fn nodes(&self) -> &[CrdtPeerNode<LoopbackTcpCrdtTransport>] {
        &self.nodes
    }

    pub fn nodes_mut(&mut self) -> &mut [CrdtPeerNode<LoopbackTcpCrdtTransport>] {
        &mut self.nodes
    }

    pub fn node(&self, index: usize) -> &CrdtPeerNode<LoopbackTcpCrdtTransport> {
        &self.nodes[index]
    }

    pub fn node_mut(&mut self, index: usize) -> &mut CrdtPeerNode<LoopbackTcpCrdtTransport> {
        &mut self.nodes[index]
    }

    pub fn partition_peer(&mut self, peer_index: usize) {
        let isolated_id = self.nodes[peer_index].peer_id();
        self.nodes[peer_index].transport_mut().partition();
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if i != peer_index {
                node.transport_mut().disconnect_peer(isolated_id);
            }
        }
    }

    pub fn heal_peer(&mut self, peer_index: usize) -> Result<(), CrdtTransportError> {
        let healed_id = self.nodes[peer_index].peer_id();
        self.nodes[peer_index].transport_mut().heal()?;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if i != peer_index {
                node.transport_mut().reconnect_peer(healed_id)?;
            }
        }
        let expected_inbound = self.nodes.len().saturating_sub(1);
        for node in &self.nodes {
            if !node
                .transport()
                .wait_for_inbound_peers(expected_inbound, Duration::from_secs(2))
            {
                return Err(CrdtTransportError::Timeout);
            }
        }
        for node in &mut self.nodes {
            let _ = node.broadcast_digest()?;
        }
        Ok(())
    }

    pub fn sync_active_until_converged(
        &mut self,
        timeout: Duration,
    ) -> Result<usize, CrdtTransportError> {
        for node in &mut self.nodes {
            if !node.transport().is_partitioned() {
                let _ = node.broadcast_digest()?;
                let _ = node.broadcast_delta()?;
            }
        }

        let deadline = Instant::now() + timeout;
        let mut total_frames = 0usize;
        let mut last_reannounce = Instant::now();

        while Instant::now() < deadline {
            for node in &mut self.nodes {
                let rep = node.pump_incoming();
                total_frames += rep.frames_processed;
            }

            let active_vvs: Vec<_> = self
                .nodes
                .iter()
                .filter(|n| !n.transport().is_partitioned())
                .map(|n| n.doc().version_vector())
                .collect();
            if active_vvs.len() <= 1 || active_vvs.windows(2).all(|w| w[0] == w[1]) {
                return Ok(total_frames);
            }

            if last_reannounce.elapsed() >= Duration::from_millis(25) {
                for node in &mut self.nodes {
                    if !node.transport().is_partitioned() {
                        let _ = node.broadcast_digest()?;
                    }
                }
                last_reannounce = Instant::now();
            }
            thread::sleep(Duration::from_millis(5));
        }

        Err(CrdtTransportError::Timeout)
    }
}
