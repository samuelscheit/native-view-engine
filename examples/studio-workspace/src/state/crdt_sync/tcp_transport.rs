//! Live loopback TCP (`127.0.0.1:0`) multi-peer binary CRDT transport built directly on
//! `nve_interop::sync_transport::{TcpDeltaServer, TcpDeltaClient, PairingConfig}`.
//!
//! Exchanges raw `NVC1` binary delta frames and `NVCD` anti-entropy vector-clock digests over
//! authenticated length-prefixed `NVT1` TCP streams with zero JSON, clean partition/disconnect,
//! and automatic anti-entropy catch-up upon reconnect.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::thread;
use std::time::{Duration, Instant};

use nve_interop::crdt::PeerId;
use nve_interop::sync_transport::{
    PairingConfig, PairingMetrics, TcpDeltaClient, TcpDeltaServer,
};

use super::StudioCrdtDoc;
use super::transport::{CrdtPeerNode, CrdtPeerTransport, CrdtTransportError};

struct RemoteTcpPeer {
    addr: SocketAddr,
    client: Option<TcpDeltaClient>,
}

/// Real OS loopback TCP CRDT peer transport backed by `nve_interop::sync_transport`.
///
/// Each peer binds a [`TcpDeltaServer`] on `127.0.0.1:0` to publish binary `NVC1`/`NVCD` frames
/// over length-prefixed `NVT1` TCP streams, and maintains authenticated [`TcpDeltaClient`]
/// connections to remote peers for full-duplex mesh delta exchange.
pub struct LoopbackTcpCrdtTransport {
    peer: PeerId,
    config: PairingConfig,
    server: TcpDeltaServer,
    remotes: BTreeMap<PeerId, RemoteTcpPeer>,
    partitioned: bool,
}

impl LoopbackTcpCrdtTransport {
    /// Binds a live `TcpDeltaServer` on `127.0.0.1:0` with `config` (pairing token + schema).
    pub fn bind(peer: PeerId, config: PairingConfig) -> Result<Self, CrdtTransportError> {
        let server = TcpDeltaServer::bind("127.0.0.1:0", config.clone())?;
        Ok(Self {
            peer,
            config,
            server,
            remotes: BTreeMap::new(),
            partitioned: false,
        })
    }

    /// Returns the OS-assigned `127.0.0.1:<port>` listening address of this peer's `TcpDeltaServer`.
    pub fn local_addr(&self) -> SocketAddr {
        self.server.local_addr()
    }

    /// Connects an authenticated length-prefixed `NVT1` `TcpDeltaClient` to `remote_peer` at `addr`.
    pub fn connect_peer(
        &mut self,
        remote_peer: PeerId,
        addr: SocketAddr,
    ) -> Result<(), CrdtTransportError> {
        let client = TcpDeltaClient::connect(addr, self.config.clone())?;
        self.remotes.insert(
            remote_peer,
            RemoteTcpPeer {
                addr,
                client: Some(client),
            },
        );
        Ok(())
    }

    /// Disconnects the outbound TCP socket to `remote_peer` (retaining its address for reconnect).
    pub fn disconnect_peer(&mut self, remote_peer: PeerId) {
        if let Some(link) = self.remotes.get_mut(&remote_peer)
            && let Some(mut client) = link.client.take()
        {
            client.disconnect();
        }
    }

    /// Re-establishes the TCP + `NVT1` pairing handshake to a previously connected `remote_peer`.
    pub fn reconnect_peer(&mut self, remote_peer: PeerId) -> Result<(), CrdtTransportError> {
        let Some(link) = self.remotes.get_mut(&remote_peer) else {
            return Err(CrdtTransportError::UnknownPeer(remote_peer));
        };
        if let Some(mut old) = link.client.take() {
            old.disconnect();
        }
        let client = TcpDeltaClient::connect(link.addr, self.config.clone())?;
        link.client = Some(client);
        Ok(())
    }

    /// Partitions this peer by disconnecting all outbound `TcpDeltaClient` sockets and suppressing
    /// outbound `TcpDeltaServer` broadcasts until [`Self::heal`] is invoked.
    pub fn partition(&mut self) {
        self.partitioned = true;
        for link in self.remotes.values_mut() {
            if let Some(mut client) = link.client.take() {
                client.disconnect();
            }
        }
    }

    /// Heals a partition by reconnecting all known remote peers over real loopback TCP sockets.
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

    /// Waits until at least `min_peers` inbound clients have completed the `NVT1` handshake.
    pub fn wait_for_inbound_peers(&self, min_peers: usize, timeout: Duration) -> bool {
        self.server.wait_for_peers(min_peers, timeout)
    }

    /// Sends raw wire bytes directly over this peer's `TcpDeltaServer` (e.g. to verify corrupt or
    /// truncated `NVC1` frame rejection over a real TCP socket).
    pub fn send_raw_wire_frame(&mut self, raw_bytes: &[u8]) -> Result<usize, CrdtTransportError> {
        Ok(self.server.send_frame(raw_bytes)?)
    }

    /// Returns live `nve_interop::sync_transport::PairingMetrics` from this peer's TCP server.
    pub fn server_pairing_metrics(&self) -> PairingMetrics {
        self.server.metrics()
    }

    /// Returns live `PairingMetrics` for the outbound TCP link to `remote_peer`, if connected.
    pub fn client_pairing_metrics(&self, remote_peer: PeerId) -> Option<PairingMetrics> {
        self.remotes
            .get(&remote_peer)
            .and_then(|l| l.client.as_ref())
            .map(TcpDeltaClient::metrics)
    }

    /// Gracefully shuts down all outbound clients and the listening `TcpDeltaServer`.
    pub fn shutdown(&mut self) {
        for link in self.remotes.values_mut() {
            if let Some(mut client) = link.client.take() {
                client.disconnect();
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
        Ok(self.server.send_frame(&frame)?)
    }

    fn poll_incoming(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        if self.partitioned {
            // Drain and drop any buffered bytes while partitioned.
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
                while let Some((_seq, bytes)) = client.try_recv_frame() {
                    out.push((remote_peer, bytes));
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
            .filter(|l| l.client.as_ref().is_some_and(TcpDeltaClient::is_connected))
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

/// Convenience multi-peer loopback TCP mesh orchestrator for N live [`StudioCrdtDoc`] peers
/// connected over real `127.0.0.1:0` sockets.
pub struct LoopbackTcpCrdtCluster {
    nodes: Vec<CrdtPeerNode<LoopbackTcpCrdtTransport>>,
}

impl LoopbackTcpCrdtCluster {
    /// Spawns `docs.len()` live TCP peers on `127.0.0.1:0` and connects a full mesh.
    pub fn spawn_full_mesh(
        docs: Vec<StudioCrdtDoc>,
        pairing_token: &str,
    ) -> Result<Self, CrdtTransportError> {
        let config = PairingConfig::new(pairing_token)
            .with_handshake_timeout_ms(2_000)
            .with_heartbeat_interval_ms(250);
        let mut nodes = Vec::with_capacity(docs.len());
        for doc in docs {
            let transport = LoopbackTcpCrdtTransport::bind(doc.peer_id(), config.clone())?;
            nodes.push(CrdtPeerNode::new(doc, transport));
        }

        let addrs: Vec<(PeerId, SocketAddr)> = nodes
            .iter()
            .map(|n| (n.peer_id(), n.transport().local_addr()))
            .collect();

        for (i, node) in nodes.iter_mut().enumerate() {
            for (j, &(remote_id, addr)) in addrs.iter().enumerate() {
                if i != j {
                    node.transport_mut().connect_peer(remote_id, addr)?;
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

    /// Isolates `peer_index` bidirectionally from every other peer in the cluster by disconnecting
    /// TCP links to and from that peer.
    pub fn partition_peer(&mut self, peer_index: usize) {
        let isolated_id = self.nodes[peer_index].peer_id();
        self.nodes[peer_index].transport_mut().partition();
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if i != peer_index {
                node.transport_mut().disconnect_peer(isolated_id);
            }
        }
    }

    /// Heals `peer_index`, re-establishing bidirectional TCP sockets and triggering immediate
    /// `NVCD` anti-entropy vector-clock catch-up across all peers.
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

    /// Broadcasts `NVC1` deltas and `NVCD` vector-clock digests from all non-partitioned peers,
    /// then pumps incoming sockets until every active peer holds an identical `VersionVector`.
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
