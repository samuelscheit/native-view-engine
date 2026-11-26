//! Deterministic in-memory multi-peer binary CRDT mesh transport (native + `wasm32`)
//! with configurable packet drop, reverse-order delivery, network partition/heal, raw frame
//! injection, and zero-copy `NveChannelBus` binary mirror.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use nve_interop::channel::{ChannelPayload, NveChannelBus};
use nve_interop::crdt::PeerId;

use super::transport::{CrdtPeerTransport, CrdtTransportError, TOPIC_CRDT_NVC1};

/// Deterministic network fault-injection knobs for [`InMemoryMeshHub`].
#[derive(Debug, Clone, Copy, Default)]
pub struct MeshFaultConfig {
    /// When `Some(n)` (`n > 0`), drops every `n`-th outbound frame deterministically.
    pub drop_every_n: Option<usize>,
    /// When `true`, delivers queued inbound frames in reverse (LIFO) order.
    pub reorder_reverse: bool,
}

struct MeshHubInner {
    inboxes: BTreeMap<PeerId, VecDeque<(PeerId, Vec<u8>)>>,
    partitioned_peers: BTreeSet<PeerId>,
    partition_groups: Option<(BTreeSet<PeerId>, BTreeSet<PeerId>)>,
    faults: MeshFaultConfig,
    tx_counter: usize,
    dropped_frames: u64,
    bus: NveChannelBus,
}

/// Shared deterministic in-memory network fabric connecting multiple [`InMemoryMeshTransport`] peers
/// with configurable packet drop, reverse-order delivery, partition/heal, and `NveChannelBus` mirror.
#[derive(Clone)]
pub struct InMemoryMeshHub {
    inner: Arc<Mutex<MeshHubInner>>,
}

impl Default for InMemoryMeshHub {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryMeshHub {
    pub fn new() -> Self {
        let mut bus = NveChannelBus::new();
        let _ = bus.ensure_channel(TOPIC_CRDT_NVC1);
        Self {
            inner: Arc::new(Mutex::new(MeshHubInner {
                inboxes: BTreeMap::new(),
                partitioned_peers: BTreeSet::new(),
                partition_groups: None,
                faults: MeshFaultConfig::default(),
                tx_counter: 0,
                dropped_frames: 0,
                bus,
            })),
        }
    }

    pub fn register_peer(&self, peer: PeerId) -> InMemoryMeshTransport {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .inboxes
            .entry(peer)
            .or_default();
        InMemoryMeshTransport {
            peer,
            hub: self.clone(),
        }
    }

    pub fn set_fault_config(&self, config: MeshFaultConfig) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .faults = config;
    }

    pub fn set_peer_partitioned(&self, peer: PeerId, partitioned: bool) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if partitioned {
            g.partitioned_peers.insert(peer);
        } else {
            g.partitioned_peers.remove(&peer);
        }
    }

    /// Splits the mesh into two isolated network partitions `group_a` and `group_b`.
    pub fn set_partition_groups(&self, group_a: &[PeerId], group_b: &[PeerId]) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.partition_groups = Some((
            group_a.iter().copied().collect(),
            group_b.iter().copied().collect(),
        ));
    }

    /// Heals all peer-level and group-level network partitions.
    pub fn heal_all(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.partitioned_peers.clear();
        g.partition_groups = None;
    }

    pub fn dropped_frames(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .dropped_frames
    }

    /// Directly injects arbitrary raw bytes into `to_peer`'s inbox (for corrupt/truncated wire tests).
    pub fn inject_raw_frame(&self, from_peer: PeerId, to_peer: PeerId, bytes: Vec<u8>) {
        if let Some(q) = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .inboxes
            .get_mut(&to_peer)
        {
            q.push_back((from_peer, bytes));
        }
    }

    fn can_communicate(g: &MeshHubInner, a: PeerId, b: PeerId) -> bool {
        if a == b || g.partitioned_peers.contains(&a) || g.partitioned_peers.contains(&b) {
            return false;
        }
        if let Some((ref ga, ref gb)) = g.partition_groups {
            let a_in_a = ga.contains(&a);
            let b_in_a = ga.contains(&b);
            let a_in_b = gb.contains(&a);
            let b_in_b = gb.contains(&b);
            if (a_in_a && b_in_b) || (a_in_b && b_in_a) {
                return false;
            }
        }
        true
    }
}

/// Deterministic in-memory peer transport backed by [`InMemoryMeshHub`].
#[derive(Clone)]
pub struct InMemoryMeshTransport {
    peer: PeerId,
    hub: InMemoryMeshHub,
}

impl InMemoryMeshTransport {
    pub fn hub(&self) -> &InMemoryMeshHub {
        &self.hub
    }
}

impl CrdtPeerTransport for InMemoryMeshTransport {
    fn local_peer(&self) -> PeerId {
        self.peer
    }

    fn send_frame(
        &mut self,
        target: Option<PeerId>,
        frame: Vec<u8>,
    ) -> Result<usize, CrdtTransportError> {
        let mut g = self.hub.inner.lock().unwrap_or_else(|e| e.into_inner());
        let _ = g.bus.publish_bytes(
            TOPIC_CRDT_NVC1,
            None,
            ChannelPayload::from_slice(&frame),
            false,
        );

        let recipients: Vec<PeerId> = match target {
            Some(dst) => {
                if !g.inboxes.contains_key(&dst) {
                    return Err(CrdtTransportError::UnknownPeer(dst));
                }
                vec![dst]
            }
            None => g
                .inboxes
                .keys()
                .copied()
                .filter(|&p| p != self.peer)
                .collect(),
        };

        let mut delivered = 0usize;
        for dst in recipients {
            if !InMemoryMeshHub::can_communicate(&g, self.peer, dst) {
                g.dropped_frames += 1;
                continue;
            }
            g.tx_counter = g.tx_counter.saturating_add(1);
            if let Some(n) = g.faults.drop_every_n
                && n > 0
                && g.tx_counter.is_multiple_of(n)
            {
                g.dropped_frames += 1;
                continue;
            }
            if let Some(inbox) = g.inboxes.get_mut(&dst) {
                inbox.push_back((self.peer, frame.clone()));
                delivered += 1;
            }
        }
        Ok(delivered)
    }

    fn poll_incoming(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        let mut g = self.hub.inner.lock().unwrap_or_else(|e| e.into_inner());
        let reverse = g.faults.reorder_reverse;
        let mut out: Vec<(PeerId, Vec<u8>)> = g
            .inboxes
            .get_mut(&self.peer)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default();
        if reverse {
            out.reverse();
        }
        out
    }

    fn connected_peer_count(&self) -> usize {
        let g = self.hub.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.inboxes
            .keys()
            .filter(|&&p| InMemoryMeshHub::can_communicate(&g, self.peer, p))
            .count()
    }

    fn is_partitioned(&self) -> bool {
        self.hub
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .partitioned_peers
            .contains(&self.peer)
    }

    fn set_partitioned(&mut self, partitioned: bool) {
        self.hub.set_peer_partitioned(self.peer, partitioned);
    }

    fn known_remote_peers(&self) -> Vec<PeerId> {
        let g = self.hub.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.inboxes
            .keys()
            .copied()
            .filter(|&p| p != self.peer)
            .collect()
    }
}
