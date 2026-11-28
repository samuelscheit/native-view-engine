//! Multi-peer binary CRDT sync adapter for Studio Workspace (`NveCrdtDelta` wire format).
//!
//! Maps Studio's collaborative entities onto convergent join-semilattice primitives:
//! - [`TaskItem`]s: [`OrSet`] membership + per-field [`LwwRegister`]s
//! - [`ChatMessage`]s (`Engineering` & `Design`): [`OrSet`] membership + [`LwwRegister`] fields + [`PnCounter`] reactions
//! - [`WorkspaceDoc`] / [`DocBlock`]: [`OrSet`] docs + [`RgaList`] block ordering + per-block [`LwwRegister`]s for kind/text/checked

mod live_ui;
pub mod mesh_transport;
mod mutations;
mod project;
#[cfg(not(target_arch = "wasm32"))]
pub mod tcp_transport;
pub mod transport;
mod validate;
mod wire;

pub use transport::{
    CRDT_DIGEST_MAGIC, CRDT_DIGEST_MIN_LEN, CRDT_DIGEST_VERSION, CrdtPeerNode, CrdtPeerTransport,
    CrdtPumpReport, CrdtSyncMetrics, CrdtTransportError, CrdtWireFrame, InMemoryMeshHub,
    InMemoryMeshTransport, MeshFaultConfig, TOPIC_CRDT_DELTA, version_vector_has_unseen_ops,
};
#[cfg(not(target_arch = "wasm32"))]
pub use transport::{LoopbackTcpCrdtCluster, LoopbackTcpCrdtTransport};

use std::collections::BTreeMap;

use nve_interop::crdt::{
    HlcClock, HlcTimestamp, LwwRegister, OpId, OrSet, PeerId, PnCounter, RgaList, VersionVector,
    crdt_xxh64,
};

use super::{ChannelKind, ChatMessage, DocBlockKind, Priority, TaskStatus, WorkspaceSnapshot};

pub(super) const SEED_TS: HlcTimestamp = HlcTimestamp::new(0, 1, 0);

pub(super) const NS_TASK_SET: u16 = 1;
pub(super) const NS_TASK_FIELDS: u16 = 2;
pub(super) const NS_CHAT_ENG_SET: u16 = 3;
pub(super) const NS_CHAT_DESIGN_SET: u16 = 4;
pub(super) const NS_CHAT_FIELDS: u16 = 5;
pub(super) const NS_CHAT_REACTIONS: u16 = 6;
pub(super) const NS_DOC_SET: u16 = 7;
pub(super) const NS_DOC_FIELDS: u16 = 8;
pub(super) const NS_DOC_BLOCKS_RGA: u16 = 9;
pub(super) const NS_DOC_BLOCK_FIELDS: u16 = 10;

pub(super) const FIELD_TASK_KEY: u16 = 0;
pub(super) const FIELD_TASK_TITLE: u16 = 1;
pub(super) const FIELD_TASK_ASSIGNEE: u16 = 2;
pub(super) const FIELD_TASK_TAG: u16 = 3;
pub(super) const FIELD_TASK_PRIORITY: u16 = 4;
pub(super) const FIELD_TASK_STATUS: u16 = 5;

pub(super) const FIELD_CHAT_AUTHOR: u16 = 0;
pub(super) const FIELD_CHAT_TIMESTAMP: u16 = 3;
pub(super) const FIELD_CHAT_BODY: u16 = 4;

pub(super) const FIELD_DOC_SLUG: u16 = 0;
pub(super) const FIELD_DOC_TITLE: u16 = 1;
pub(super) const FIELD_DOC_UPDATED_AT: u16 = 2;

pub(super) const FIELD_BLOCK_KIND: u16 = 0;
pub(super) const FIELD_BLOCK_CHECKED: u16 = 1;
pub(super) const FIELD_BLOCK_TEXT: u16 = 2;

/// Summary returned by [`StudioCrdtDoc::merge_delta`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CrdtMergeSummary {
    pub sender: PeerId,
    pub records_received: usize,
    pub records_applied: usize,
    pub tasks_changed: usize,
    pub messages_changed: usize,
    pub docs_changed: usize,
    pub blocks_changed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrackedLww<T> {
    pub(super) reg: LwwRegister<T>,
    pub(super) op_id: OpId,
}

impl<T: Default> Default for TrackedLww<T> {
    fn default() -> Self {
        Self {
            reg: LwwRegister::default(),
            op_id: OpId::ZERO,
        }
    }
}

impl<T> TrackedLww<T> {
    pub(super) fn new(value: T, ts: HlcTimestamp, op_id: OpId) -> Self {
        Self {
            reg: LwwRegister::new(value, ts),
            op_id,
        }
    }

    pub(super) fn set(&mut self, value: T, ts: HlcTimestamp, op_id: OpId) -> bool {
        if self.reg.set(value, ts) {
            self.op_id = op_id;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct TaskCrdt {
    pub(super) key: TrackedLww<String>,
    pub(super) title: TrackedLww<String>,
    pub(super) assignee: TrackedLww<String>,
    pub(super) tag: TrackedLww<String>,
    pub(super) priority: TrackedLww<Priority>,
    pub(super) status: TrackedLww<TaskStatus>,
}

impl Default for TaskCrdt {
    fn default() -> Self {
        Self {
            key: TrackedLww::default(),
            title: TrackedLww::default(),
            assignee: TrackedLww::default(),
            tag: TrackedLww::default(),
            priority: TrackedLww::new(Priority::P1, HlcTimestamp::ZERO, OpId::ZERO),
            status: TrackedLww::new(TaskStatus::Todo, HlcTimestamp::ZERO, OpId::ZERO),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ChatMessageCrdt {
    pub(super) author: TrackedLww<String>,
    pub(super) timestamp: TrackedLww<String>,
    pub(super) body: TrackedLww<String>,
    pub(super) reactions: PnCounter,
    pub(super) reaction_ops: BTreeMap<PeerId, OpId>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DocBlockCrdt {
    pub(super) kind_tag: TrackedLww<u32>,
    pub(super) checked: TrackedLww<bool>,
    pub(super) text: TrackedLww<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DocCrdt {
    pub(super) slug: TrackedLww<String>,
    pub(super) title: TrackedLww<String>,
    pub(super) updated_at: TrackedLww<String>,
    pub(super) block_order: RgaList<u32>,
    pub(super) blocks: BTreeMap<u32, DocBlockCrdt>,
}

/// Convergent multi-peer CRDT document backing Studio Workspace tasks, channels, and docs.
#[derive(Debug, Clone)]
pub struct StudioCrdtDoc {
    pub(super) peer: PeerId,
    pub(super) clock: HlcClock,
    pub(super) local_seq: u64,
    pub(super) local_entity_seq: u32,
    pub(super) vv: VersionVector,
    pub(super) task_set: OrSet<u64>,
    pub(super) tasks: BTreeMap<u32, TaskCrdt>,
    pub(super) eng_set: OrSet<u64>,
    pub(super) design_set: OrSet<u64>,
    pub(super) messages: BTreeMap<u64, ChatMessageCrdt>,
    pub(super) doc_set: OrSet<u64>,
    pub(super) docs: BTreeMap<u32, DocCrdt>,
}

impl StudioCrdtDoc {
    /// Creates an empty CRDT document for `peer`.
    pub fn new(peer: PeerId, now_ms: u64) -> Self {
        Self {
            peer,
            clock: HlcClock::with_initial(peer, now_ms.max(1)),
            local_seq: 0,
            local_entity_seq: 0,
            vv: VersionVector::new(),
            task_set: OrSet::new(),
            tasks: BTreeMap::new(),
            eng_set: OrSet::new(),
            design_set: OrSet::new(),
            messages: BTreeMap::new(),
            doc_set: OrSet::new(),
            docs: BTreeMap::new(),
        }
    }

    /// Deterministically seeds a peer's CRDT state from a shared [`WorkspaceSnapshot`].
    ///
    /// Seeded entries are tagged under [`PeerId::SEED`] so multiple peers seeded from the same
    /// snapshot share identical operation identifiers and never duplicate items or double-count
    /// initial reactions when they sync.
    pub fn from_snapshot(peer: PeerId, now_ms: u64, snapshot: &WorkspaceSnapshot) -> Self {
        let mut doc = Self::new(peer, now_ms);

        for task in &snapshot.tasks {
            let add_op = seed_op(1, u64::from(task.id));
            doc.task_set.add(u64::from(task.id), add_op);
            doc.vv.observe(add_op);

            let field_op = |f: u16| seed_op(2, (u64::from(task.id) << 4) | u64::from(f));
            for f in 0..=5 {
                doc.vv.observe(field_op(f));
            }
            doc.tasks.insert(
                task.id,
                TaskCrdt {
                    key: TrackedLww::new(task.key.clone(), SEED_TS, field_op(FIELD_TASK_KEY)),
                    title: TrackedLww::new(task.title.clone(), SEED_TS, field_op(FIELD_TASK_TITLE)),
                    assignee: TrackedLww::new(
                        task.assignee.clone(),
                        SEED_TS,
                        field_op(FIELD_TASK_ASSIGNEE),
                    ),
                    tag: TrackedLww::new(task.tag.clone(), SEED_TS, field_op(FIELD_TASK_TAG)),
                    priority: TrackedLww::new(
                        task.priority,
                        SEED_TS,
                        field_op(FIELD_TASK_PRIORITY),
                    ),
                    status: TrackedLww::new(task.status, SEED_TS, field_op(FIELD_TASK_STATUS)),
                },
            );
        }

        doc.seed_channel(ChannelKind::Engineering, &snapshot.eng_messages);
        doc.seed_channel(ChannelKind::Design, &snapshot.design_messages);

        for ws_doc in &snapshot.docs {
            let add_op = seed_op(6, u64::from(ws_doc.id));
            doc.doc_set.add(u64::from(ws_doc.id), add_op);
            doc.vv.observe(add_op);

            let doc_field_op = |f: u16| seed_op(7, (u64::from(ws_doc.id) << 4) | u64::from(f));
            for f in 0..=2 {
                doc.vv.observe(doc_field_op(f));
            }

            let mut crdt_doc = DocCrdt {
                slug: TrackedLww::new(ws_doc.slug.clone(), SEED_TS, doc_field_op(FIELD_DOC_SLUG)),
                title: TrackedLww::new(
                    ws_doc.title.clone(),
                    SEED_TS,
                    doc_field_op(FIELD_DOC_TITLE),
                ),
                updated_at: TrackedLww::new(
                    ws_doc.updated_at.clone(),
                    SEED_TS,
                    doc_field_op(FIELD_DOC_UPDATED_AT),
                ),
                block_order: RgaList::new(),
                blocks: BTreeMap::new(),
            };

            let mut prev_rga_id = None;
            for block in &ws_doc.blocks {
                let rga_id = seed_op(8, (u64::from(ws_doc.id) << 24) | u64::from(block.id));
                crdt_doc
                    .block_order
                    .insert_after(prev_rga_id, rga_id, block.id);
                doc.vv.observe(rga_id);
                prev_rga_id = Some(rga_id);

                let blk_field_op = |f: u16| {
                    seed_op(
                        9,
                        (u64::from(ws_doc.id) << 28) | (u64::from(block.id) << 4) | u64::from(f),
                    )
                };
                for f in 0..=2 {
                    doc.vv.observe(blk_field_op(f));
                }
                let (kind_tag, checked) = encode_block_kind(block.kind);
                crdt_doc.blocks.insert(
                    block.id,
                    DocBlockCrdt {
                        kind_tag: TrackedLww::new(
                            kind_tag,
                            SEED_TS,
                            blk_field_op(FIELD_BLOCK_KIND),
                        ),
                        checked: TrackedLww::new(
                            checked,
                            SEED_TS,
                            blk_field_op(FIELD_BLOCK_CHECKED),
                        ),
                        text: TrackedLww::new(
                            block.text.clone(),
                            SEED_TS,
                            blk_field_op(FIELD_BLOCK_TEXT),
                        ),
                    },
                );
            }

            doc.docs.insert(ws_doc.id, crdt_doc);
        }

        // Advance Lamport local_seq past every seeded OpId so causally-later local inserts
        // (especially head inserts at after_id=None) strictly outrank seeded RGA anchors.
        doc.local_seq = doc.vv.get(PeerId::SEED);
        doc
    }

    fn seed_channel(&mut self, channel: ChannelKind, messages: &[ChatMessage]) {
        let ch_bit = channel_bit(channel);
        let set = match channel {
            ChannelKind::Engineering => &mut self.eng_set,
            ChannelKind::Design => &mut self.design_set,
        };
        for msg in messages {
            let entity = chat_entity_id(channel, msg.id);
            let add_op = seed_op(3, (ch_bit << 32) | u64::from(msg.id));
            set.add(u64::from(msg.id), add_op);
            self.vv.observe(add_op);

            let field_op =
                |f: u16| seed_op(4, (ch_bit << 36) | (u64::from(msg.id) << 4) | u64::from(f));
            for f in 0..=4 {
                self.vv.observe(field_op(f));
            }

            let mut reactions = PnCounter::new();
            let mut reaction_ops = BTreeMap::new();
            if msg.reactions > 0 {
                let react_op = seed_op(5, (ch_bit << 32) | u64::from(msg.id));
                reactions.increment(PeerId::SEED, u64::from(msg.reactions));
                reaction_ops.insert(PeerId::SEED, react_op);
                self.vv.observe(react_op);
            }

            self.messages.insert(
                entity,
                ChatMessageCrdt {
                    author: TrackedLww::new(
                        msg.author.clone(),
                        SEED_TS,
                        field_op(FIELD_CHAT_AUTHOR),
                    ),
                    timestamp: TrackedLww::new(
                        msg.timestamp.clone(),
                        SEED_TS,
                        field_op(FIELD_CHAT_TIMESTAMP),
                    ),
                    body: TrackedLww::new(msg.body.clone(), SEED_TS, field_op(FIELD_CHAT_BODY)),
                    reactions,
                    reaction_ops,
                },
            );
        }
    }

    #[inline]
    pub const fn peer_id(&self) -> PeerId {
        self.peer
    }

    #[inline]
    pub fn version_vector(&self) -> &VersionVector {
        &self.vv
    }

    pub(super) fn observe_op(&mut self, op: OpId) {
        self.vv.observe(op);
        if op.seq > self.local_seq {
            self.local_seq = op.seq;
        }
    }

    pub(super) fn next_op(&mut self, now_ms: u64) -> (OpId, HlcTimestamp) {
        self.local_seq = self.local_seq.saturating_add(1);
        let op_id = OpId::new(self.peer, self.local_seq);
        self.vv.observe(op_id);
        let ts = self.clock.tick(now_ms);
        (op_id, ts)
    }

    pub(super) fn alloc_entity_id(&mut self) -> u32 {
        self.local_entity_seq = self.local_entity_seq.wrapping_add(1).max(1);
        let bucket = peer_bucket_u16(self.peer);
        (u32::from(bucket) << 16) | (self.local_entity_seq & 0xFFFF)
    }
}

#[inline]
pub(super) const fn seed_op(category: u8, payload: u64) -> OpId {
    OpId::new(
        PeerId::SEED,
        ((category as u64) << 40) | (payload & 0x00FF_FFFF_FFFF),
    )
}

#[inline]
pub(super) const fn channel_bit(channel: ChannelKind) -> u64 {
    match channel {
        ChannelKind::Engineering => 0,
        ChannelKind::Design => 1,
    }
}

#[inline]
pub(super) const fn chat_entity_id(channel: ChannelKind, msg_id: u32) -> u64 {
    (channel_bit(channel) << 32) | (msg_id as u64)
}

#[inline]
pub(super) const fn block_entity_id(doc_id: u32, block_id: u32) -> u64 {
    ((doc_id as u64) << 32) | (block_id as u64)
}

#[inline]
pub(super) const fn split_block_entity_id(entity_id: u64) -> (u32, u32) {
    ((entity_id >> 32) as u32, entity_id as u32)
}

fn peer_bucket_u16(peer: PeerId) -> u16 {
    if peer.0 >= 1 && peer.0 <= u64::from(u16::MAX) {
        peer.0 as u16
    } else {
        let h = crdt_xxh64(&peer.as_u64().to_le_bytes());
        ((h % u64::from(u16::MAX - 1)) as u16) + 1
    }
}

pub(super) const fn encode_priority(p: Priority) -> u32 {
    p as u32
}

pub(super) const fn decode_priority(v: u32) -> Option<Priority> {
    if v <= u8::MAX as u32 {
        Priority::try_from_u8(v as u8)
    } else {
        None
    }
}

pub(super) const fn encode_status(s: TaskStatus) -> u32 {
    s as u32
}

pub(super) const fn decode_status(v: u32) -> Option<TaskStatus> {
    if v <= u8::MAX as u32 {
        TaskStatus::try_from_u8(v as u8)
    } else {
        None
    }
}

pub(super) const fn encode_block_kind(kind: DocBlockKind) -> (u32, bool) {
    match kind {
        DocBlockKind::Heading1 => (0, false),
        DocBlockKind::Heading2 => (1, false),
        DocBlockKind::Paragraph => (2, false),
        DocBlockKind::Checklist { checked } => (3, checked),
        DocBlockKind::Code => (4, false),
        DocBlockKind::Callout => (5, false),
    }
}

pub(super) const fn decode_block_kind(kind_tag: u32, checked: bool) -> DocBlockKind {
    match kind_tag {
        0 => DocBlockKind::Heading1,
        1 => DocBlockKind::Heading2,
        2 => DocBlockKind::Paragraph,
        3 => DocBlockKind::Checklist { checked },
        4 => DocBlockKind::Code,
        5 => DocBlockKind::Callout,
        _ => DocBlockKind::Paragraph,
    }
}
