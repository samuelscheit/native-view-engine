//! Interactive multi-peer binary `NveCrdtDelta` CRDT sync actions for [`WorkspaceState`].
//!
//! Runs real [`StudioCrdtDoc`] peers over [`InMemoryMeshTransport`] (works identically on
//! native hosts and `wasm32-unknown-unknown`) so Studio's Live Sync panel exercises genuine
//! binary `NveCrdtDelta` encoding, partition/heal, and vector-clock anti-entropy catch-up.

use nve_interop::crdt::PeerId;

use super::transport::{CrdtPeerNode, CrdtPeerTransport, InMemoryMeshHub};
use super::StudioCrdtDoc;
use crate::state::{ChannelKind, WorkspaceState};

impl WorkspaceState {
    /// Executes a live 2-peer binary `NveCrdtDelta` CRDT sync round over [`InMemoryMeshTransport`]
    /// and updates `sync_telemetry` with exact wire delta & byte metrics.
    ///
    /// While `crdt_partitioned` is `true`, packets are dropped across the simulated partition
    /// and remote ops stay queued until [`Self::toggle_crdt_partition`] heals the mesh.
    pub fn sync_crdt_now(&mut self) {
        let hub = InMemoryMeshHub::new();
        let now_ms = 20_000u64.saturating_add(self.frame_count.saturating_mul(100));

        let local_doc = StudioCrdtDoc::from_snapshot(PeerId(1), now_ms, &self.current);
        let mut remote_doc = StudioCrdtDoc::from_snapshot(PeerId(2), now_ms, &self.current);

        // If the mesh was partitioned or has queued remote peer activity, synthesize a real
        // peer reaction/comment mutation on Peer 2 so convergence visibly merges real CRDT records.
        let pending = self.sync_telemetry.crdt_pending_remote_ops;
        let partitioned = self.sync_telemetry.crdt_partitioned;
        if pending > 0
            && let Some(first_msg) = self.current.eng_messages.first()
        {
            remote_doc.react_chat_message(
                now_ms + 10,
                ChannelKind::Engineering,
                first_msg.id,
                i64::from(pending),
            );
        }

        let t1 = hub.register_peer(PeerId(1));
        let t2 = hub.register_peer(PeerId(2));
        if partitioned {
            hub.set_peer_partitioned(PeerId(1), true);
        }

        let mut node1 = CrdtPeerNode::new(local_doc, t1);
        let mut node2 = CrdtPeerNode::new(remote_doc, t2);

        // Exchange NVCD anti-entropy digests first; if Peer 2 has unseen offline ops it replies
        // automatically with a minimal catch-up delta, otherwise both exchange live CRDT deltas.
        let _ = node1.broadcast_digest();
        let _ = node2.broadcast_digest();
        let rep2 = node2.pump_incoming();
        if rep2.anti_entropy_deltas_sent == 0 {
            let _ = node2.broadcast_delta();
        }
        let _ = node1.broadcast_delta();
        let rep1 = node1.pump_incoming();

        let m1 = node1.metrics();
        let bytes_round = m1.bytes_sent.saturating_add(m1.bytes_received);

        self.sync_telemetry.crdt_deltas_sent = self
            .sync_telemetry
            .crdt_deltas_sent
            .saturating_add(m1.deltas_sent);
        self.sync_telemetry.crdt_deltas_received = self
            .sync_telemetry
            .crdt_deltas_received
            .saturating_add(m1.deltas_received);
        self.sync_telemetry.crdt_bytes_synced = self
            .sync_telemetry
            .crdt_bytes_synced
            .saturating_add(bytes_round);

        if !partitioned && rep1.records_applied > 0 {
            let converged = node1.doc().project_snapshot(&self.current);
            self.sync_telemetry.crdt_pending_remote_ops = 0;
            self.commit_action(
                format!("CRDT Sync ({}B)", bytes_round),
                |s| {
                    s.tasks = converged.tasks;
                    s.next_task_id = converged.next_task_id;
                    s.eng_messages = converged.eng_messages;
                    s.design_messages = converged.design_messages;
                    s.next_msg_id = converged.next_msg_id;
                    s.docs = converged.docs;
                    s.next_doc_id = converged.next_doc_id;
                    s.next_block_id = converged.next_block_id;
                },
            );
        } else {
            self.frame_count = self.frame_count.saturating_add(1);
        }
    }

    /// Toggles simulated network partition / heal on the live Studio CRDT mesh.
    ///
    /// - Entering partition drops `connected_peers` to `0` and queues a remote peer edit while split.
    /// - Healing restores `connected_peers` and immediately runs binary `NveCrdtDelta` anti-entropy catch-up!
    pub fn toggle_crdt_partition(&mut self) {
        let next_partitioned = !self.sync_telemetry.crdt_partitioned;
        self.sync_telemetry.crdt_partitioned = next_partitioned;
        if next_partitioned {
            self.sync_telemetry.connected_peers = 0;
            self.sync_telemetry.crdt_pending_remote_ops = self
                .sync_telemetry
                .crdt_pending_remote_ops
                .saturating_add(1);
            self.frame_count = self.frame_count.saturating_add(1);
        } else {
            if self.sync_telemetry.connected_peers == 0 {
                self.sync_telemetry.connected_peers = 2;
            }
            self.sync_crdt_now();
        }
    }

    /// Helper returning whether the active CRDT mesh transport is currently partitioned.
    pub fn is_crdt_partitioned(&self) -> bool {
        self.sync_telemetry.crdt_partitioned
    }

    /// Convenience helper exposing peer connectivity count from a live transport snapshot.
    pub fn apply_crdt_transport_metrics<T: CrdtPeerTransport>(&mut self, node: &CrdtPeerNode<T>) {
        let m = node.metrics();
        self.sync_telemetry.connected_peers = m.connected_peers;
        self.sync_telemetry.crdt_deltas_sent = m.deltas_sent;
        self.sync_telemetry.crdt_deltas_received = m.deltas_received;
        self.sync_telemetry.crdt_bytes_synced = m.bytes_sent.saturating_add(m.bytes_received);
        self.sync_telemetry.crdt_partitioned = m.partitioned;
        self.frame_count = self.frame_count.saturating_add(1);
    }
}
