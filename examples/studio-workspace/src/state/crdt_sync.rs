//! Multi-peer binary CRDT sync adapter for Studio Workspace (`NVC1` wire format).
//!
//! Maps Studio's collaborative entities onto convergent join-semilattice primitives:
//! - [`TaskItem`]s: [`OrSet`] membership + per-field [`LwwRegister`]s
//! - [`ChatMessage`]s (`Engineering` & `Design`): [`OrSet`] membership + [`LwwRegister`] fields + [`PnCounter`] reactions
//! - [`WorkspaceDoc`] / [`DocBlock`]: [`OrSet`] docs + [`RgaList`] block ordering + per-block [`LwwRegister`]s for kind/text/checked

use std::collections::{BTreeMap, BTreeSet};

use nve_interop::crdt::{
    CrdtDeltaPacket, CrdtRecord, CrdtValue, CrdtWireError, HlcClock, HlcTimestamp, LwwRegister,
    OpId, OrSet, PeerId, PnCounter, RgaList, VersionVector, fnv1a_64,
};

use super::{
    ChannelKind, ChatMessage, DocBlock, DocBlockKind, Priority, TaskItem, TaskStatus, WorkspaceDoc,
    WorkspaceSnapshot,
};

const SEED_TS: HlcTimestamp = HlcTimestamp::new(0, 1, 0);

const NS_TASK_SET: u16 = 1;
const NS_TASK_FIELDS: u16 = 2;
const NS_CHAT_ENG_SET: u16 = 3;
const NS_CHAT_DESIGN_SET: u16 = 4;
const NS_CHAT_FIELDS: u16 = 5;
const NS_CHAT_REACTIONS: u16 = 6;
const NS_DOC_SET: u16 = 7;
const NS_DOC_FIELDS: u16 = 8;
const NS_DOC_BLOCKS_RGA: u16 = 9;
const NS_DOC_BLOCK_FIELDS: u16 = 10;

const FIELD_TASK_KEY: u16 = 0;
const FIELD_TASK_TITLE: u16 = 1;
const FIELD_TASK_ASSIGNEE: u16 = 2;
const FIELD_TASK_TAG: u16 = 3;
const FIELD_TASK_PRIORITY: u16 = 4;
const FIELD_TASK_STATUS: u16 = 5;

const FIELD_CHAT_AUTHOR: u16 = 0;
const FIELD_CHAT_BADGE_BG: u16 = 1;
const FIELD_CHAT_BADGE_FG: u16 = 2;
const FIELD_CHAT_TIMESTAMP: u16 = 3;
const FIELD_CHAT_BODY: u16 = 4;

const FIELD_DOC_SLUG: u16 = 0;
const FIELD_DOC_TITLE: u16 = 1;
const FIELD_DOC_UPDATED_AT: u16 = 2;

const FIELD_BLOCK_KIND: u16 = 0;
const FIELD_BLOCK_CHECKED: u16 = 1;
const FIELD_BLOCK_TEXT: u16 = 2;

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
struct TrackedLww<T> {
    reg: LwwRegister<T>,
    op_id: OpId,
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
    fn new(value: T, ts: HlcTimestamp, op_id: OpId) -> Self {
        Self {
            reg: LwwRegister::new(value, ts),
            op_id,
        }
    }

    fn set(&mut self, value: T, ts: HlcTimestamp, op_id: OpId) -> bool {
        if self.reg.set(value, ts) {
            self.op_id = op_id;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
struct TaskCrdt {
    key: TrackedLww<String>,
    title: TrackedLww<String>,
    assignee: TrackedLww<String>,
    tag: TrackedLww<String>,
    priority: TrackedLww<Priority>,
    status: TrackedLww<TaskStatus>,
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
struct ChatMessageCrdt {
    author: TrackedLww<String>,
    badge_bg: TrackedLww<String>,
    badge_fg: TrackedLww<String>,
    timestamp: TrackedLww<String>,
    body: TrackedLww<String>,
    reactions: PnCounter,
    reaction_ops: BTreeMap<PeerId, OpId>,
}

#[derive(Debug, Clone, Default)]
struct DocBlockCrdt {
    kind_tag: TrackedLww<u32>,
    checked: TrackedLww<bool>,
    text: TrackedLww<String>,
}

#[derive(Debug, Clone, Default)]
struct DocCrdt {
    slug: TrackedLww<String>,
    title: TrackedLww<String>,
    updated_at: TrackedLww<String>,
    block_order: RgaList<u32>,
    blocks: BTreeMap<u32, DocBlockCrdt>,
}

/// Convergent multi-peer CRDT document backing Studio Workspace tasks, channels, and docs.
#[derive(Debug, Clone)]
pub struct StudioCrdtDoc {
    peer: PeerId,
    clock: HlcClock,
    local_seq: u64,
    local_entity_seq: u32,
    vv: VersionVector,
    task_set: OrSet<u64>,
    tasks: BTreeMap<u32, TaskCrdt>,
    eng_set: OrSet<u64>,
    design_set: OrSet<u64>,
    messages: BTreeMap<u64, ChatMessageCrdt>,
    doc_set: OrSet<u64>,
    docs: BTreeMap<u32, DocCrdt>,
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
                crdt_doc.block_order.insert_after(prev_rga_id, rga_id, block.id);
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
                        kind_tag: TrackedLww::new(kind_tag, SEED_TS, blk_field_op(FIELD_BLOCK_KIND)),
                        checked: TrackedLww::new(checked, SEED_TS, blk_field_op(FIELD_BLOCK_CHECKED)),
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

            let field_op = |f: u16| {
                seed_op(
                    4,
                    (ch_bit << 36) | (u64::from(msg.id) << 4) | u64::from(f),
                )
            };
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
                    badge_bg: TrackedLww::new(
                        msg.badge_bg.clone(),
                        SEED_TS,
                        field_op(FIELD_CHAT_BADGE_BG),
                    ),
                    badge_fg: TrackedLww::new(
                        msg.badge_fg.clone(),
                        SEED_TS,
                        field_op(FIELD_CHAT_BADGE_FG),
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

    fn observe_op(&mut self, op: OpId) {
        self.vv.observe(op);
        if op.seq > self.local_seq {
            self.local_seq = op.seq;
        }
    }

    fn next_op(&mut self, now_ms: u64) -> (OpId, HlcTimestamp) {
        self.local_seq = self.local_seq.saturating_add(1);
        let op_id = OpId::new(self.peer, self.local_seq);
        self.vv.observe(op_id);
        let ts = self.clock.tick(now_ms);
        (op_id, ts)
    }

    fn alloc_entity_id(&mut self) -> u32 {
        self.local_entity_seq = self.local_entity_seq.wrapping_add(1).max(1);
        let bucket = peer_bucket_u16(self.peer);
        (u32::from(bucket) << 16) | (self.local_entity_seq & 0xFFFF)
    }

    // -------------------------------------------------------------------------
    // Task mutations
    // -------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn add_task(
        &mut self,
        now_ms: u64,
        key: impl Into<String>,
        title: impl Into<String>,
        assignee: impl Into<String>,
        tag: impl Into<String>,
        priority: Priority,
        status: TaskStatus,
    ) -> u32 {
        let task_id = self.alloc_entity_id();
        let (add_op, _) = self.next_op(now_ms);
        self.task_set.add(u64::from(task_id), add_op);

        let (op_k, ts_k) = self.next_op(now_ms);
        let (op_t, ts_t) = self.next_op(now_ms);
        let (op_a, ts_a) = self.next_op(now_ms);
        let (op_g, ts_g) = self.next_op(now_ms);
        let (op_p, ts_p) = self.next_op(now_ms);
        let (op_s, ts_s) = self.next_op(now_ms);

        self.tasks.insert(
            task_id,
            TaskCrdt {
                key: TrackedLww::new(key.into(), ts_k, op_k),
                title: TrackedLww::new(title.into(), ts_t, op_t),
                assignee: TrackedLww::new(assignee.into(), ts_a, op_a),
                tag: TrackedLww::new(tag.into(), ts_g, op_g),
                priority: TrackedLww::new(priority, ts_p, op_p),
                status: TrackedLww::new(status, ts_s, op_s),
            },
        );
        task_id
    }

    pub fn edit_task_title(&mut self, now_ms: u64, task_id: u32, title: impl Into<String>) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        self.tasks.entry(task_id).or_default().title.set(title.into(), ts, op)
    }

    pub fn edit_task_status(&mut self, now_ms: u64, task_id: u32, status: TaskStatus) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        self.tasks.entry(task_id).or_default().status.set(status, ts, op)
    }

    pub fn edit_task_priority(&mut self, now_ms: u64, task_id: u32, priority: Priority) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        self.tasks.entry(task_id).or_default().priority.set(priority, ts, op)
    }

    pub fn edit_task_assignee(
        &mut self,
        now_ms: u64,
        task_id: u32,
        assignee: impl Into<String>,
    ) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        self.tasks
            .entry(task_id)
            .or_default()
            .assignee
            .set(assignee.into(), ts, op)
    }

    pub fn edit_task_tag(&mut self, now_ms: u64, task_id: u32, tag: impl Into<String>) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        self.tasks.entry(task_id).or_default().tag.set(tag.into(), ts, op)
    }

    pub fn delete_task(&mut self, now_ms: u64, task_id: u32) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, _) = self.next_op(now_ms);
        !self.task_set.remove_with_op(&u64::from(task_id), op).is_empty()
    }

    // -------------------------------------------------------------------------
    // Chat mutations
    // -------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn post_chat_message(
        &mut self,
        now_ms: u64,
        channel: ChannelKind,
        author: impl Into<String>,
        badge_bg: impl Into<String>,
        badge_fg: impl Into<String>,
        timestamp: impl Into<String>,
        body: impl Into<String>,
    ) -> u32 {
        let msg_id = self.alloc_entity_id();
        let (add_op, _) = self.next_op(now_ms);
        match channel {
            ChannelKind::Engineering => {
                self.eng_set.add(u64::from(msg_id), add_op);
            }
            ChannelKind::Design => {
                self.design_set.add(u64::from(msg_id), add_op);
            }
        }

        let (op_a, ts_a) = self.next_op(now_ms);
        let (op_bg, ts_bg) = self.next_op(now_ms);
        let (op_fg, ts_fg) = self.next_op(now_ms);
        let (op_ts, ts_ts) = self.next_op(now_ms);
        let (op_b, ts_b) = self.next_op(now_ms);

        let entity = chat_entity_id(channel, msg_id);
        self.messages.insert(
            entity,
            ChatMessageCrdt {
                author: TrackedLww::new(author.into(), ts_a, op_a),
                badge_bg: TrackedLww::new(badge_bg.into(), ts_bg, op_bg),
                badge_fg: TrackedLww::new(badge_fg.into(), ts_fg, op_fg),
                timestamp: TrackedLww::new(timestamp.into(), ts_ts, op_ts),
                body: TrackedLww::new(body.into(), ts_b, op_b),
                reactions: PnCounter::new(),
                reaction_ops: BTreeMap::new(),
            },
        );
        msg_id
    }

    pub fn edit_chat_message(
        &mut self,
        now_ms: u64,
        channel: ChannelKind,
        msg_id: u32,
        body: impl Into<String>,
    ) -> bool {
        if !self.channel_contains(channel, msg_id) {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        let entity = chat_entity_id(channel, msg_id);
        self.messages
            .entry(entity)
            .or_default()
            .body
            .set(body.into(), ts, op)
    }

    pub fn react_chat_message(
        &mut self,
        now_ms: u64,
        channel: ChannelKind,
        msg_id: u32,
        delta: i64,
    ) -> bool {
        if delta == 0 || !self.channel_contains(channel, msg_id) {
            return false;
        }
        let (op, _) = self.next_op(now_ms);
        let entity = chat_entity_id(channel, msg_id);
        let entry = self.messages.entry(entity).or_default();
        entry.reactions.apply_delta(self.peer, delta);
        entry.reaction_ops.insert(self.peer, op);
        true
    }

    pub fn delete_chat_message(&mut self, now_ms: u64, channel: ChannelKind, msg_id: u32) -> bool {
        if !self.channel_contains(channel, msg_id) {
            return false;
        }
        let (op, _) = self.next_op(now_ms);
        let set = match channel {
            ChannelKind::Engineering => &mut self.eng_set,
            ChannelKind::Design => &mut self.design_set,
        };
        !set.remove_with_op(&u64::from(msg_id), op).is_empty()
    }

    fn channel_contains(&self, channel: ChannelKind, msg_id: u32) -> bool {
        match channel {
            ChannelKind::Engineering => self.eng_set.contains(&u64::from(msg_id)),
            ChannelKind::Design => self.design_set.contains(&u64::from(msg_id)),
        }
    }

    // -------------------------------------------------------------------------
    // Document & block mutations
    // -------------------------------------------------------------------------

    pub fn add_doc(
        &mut self,
        now_ms: u64,
        slug: impl Into<String>,
        title: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> u32 {
        let doc_id = self.alloc_entity_id();
        let (add_op, _) = self.next_op(now_ms);
        self.doc_set.add(u64::from(doc_id), add_op);

        let (op_s, ts_s) = self.next_op(now_ms);
        let (op_t, ts_t) = self.next_op(now_ms);
        let (op_u, ts_u) = self.next_op(now_ms);

        self.docs.insert(
            doc_id,
            DocCrdt {
                slug: TrackedLww::new(slug.into(), ts_s, op_s),
                title: TrackedLww::new(title.into(), ts_t, op_t),
                updated_at: TrackedLww::new(updated_at.into(), ts_u, op_u),
                block_order: RgaList::new(),
                blocks: BTreeMap::new(),
            },
        );
        doc_id
    }

    pub fn delete_doc(&mut self, now_ms: u64, doc_id: u32) -> bool {
        if !self.doc_set.contains(&u64::from(doc_id)) {
            return false;
        }
        let (op, _) = self.next_op(now_ms);
        !self.doc_set.remove_with_op(&u64::from(doc_id), op).is_empty()
    }

    pub fn add_doc_block(
        &mut self,
        now_ms: u64,
        doc_id: u32,
        kind: DocBlockKind,
        text: impl Into<String>,
    ) -> Option<u32> {
        let tail_block = self
            .docs
            .get(&doc_id)?
            .block_order
            .visible_nodes()
            .last()
            .map(|n| n.value);
        self.insert_doc_block_after(now_ms, doc_id, tail_block, kind, text)
    }

    pub fn insert_doc_block_after(
        &mut self,
        now_ms: u64,
        doc_id: u32,
        after_block_id: Option<u32>,
        kind: DocBlockKind,
        text: impl Into<String>,
    ) -> Option<u32> {
        if !self.doc_set.contains(&u64::from(doc_id)) {
            return None;
        }
        let anchor_rga_id = match after_block_id {
            None => None,
            Some(target_blk) => Some(self.latest_rga_id_for_block(doc_id, target_blk)?),
        };
        let block_id = self.alloc_entity_id();
        let (rga_op, _) = self.next_op(now_ms);
        let (op_k, ts_k) = self.next_op(now_ms);
        let (op_c, ts_c) = self.next_op(now_ms);
        let (op_t, ts_t) = self.next_op(now_ms);

        let (kind_tag, checked) = encode_block_kind(kind);
        let doc = self.docs.entry(doc_id).or_default();
        doc.block_order.insert_after(anchor_rga_id, rga_op, block_id);
        doc.blocks.insert(
            block_id,
            DocBlockCrdt {
                kind_tag: TrackedLww::new(kind_tag, ts_k, op_k),
                checked: TrackedLww::new(checked, ts_c, op_c),
                text: TrackedLww::new(text.into(), ts_t, op_t),
            },
        );
        Some(block_id)
    }

    pub fn edit_doc_block_text(
        &mut self,
        now_ms: u64,
        doc_id: u32,
        block_id: u32,
        text: impl Into<String>,
    ) -> bool {
        if self.latest_rga_id_for_block(doc_id, block_id).is_none() {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        self.docs
            .entry(doc_id)
            .or_default()
            .blocks
            .entry(block_id)
            .or_default()
            .text
            .set(text.into(), ts, op)
    }

    pub fn edit_doc_block_kind(
        &mut self,
        now_ms: u64,
        doc_id: u32,
        block_id: u32,
        kind: DocBlockKind,
    ) -> bool {
        if self.latest_rga_id_for_block(doc_id, block_id).is_none() {
            return false;
        }
        let (kind_tag, checked) = encode_block_kind(kind);
        let (op_k, ts_k) = self.next_op(now_ms);
        let (op_c, ts_c) = self.next_op(now_ms);
        let blk = self
            .docs
            .entry(doc_id)
            .or_default()
            .blocks
            .entry(block_id)
            .or_default();
        let a = blk.kind_tag.set(kind_tag, ts_k, op_k);
        let b = blk.checked.set(checked, ts_c, op_c);
        a || b
    }

    pub fn toggle_doc_block_checklist(
        &mut self,
        now_ms: u64,
        doc_id: u32,
        block_id: u32,
    ) -> Option<bool> {
        self.latest_rga_id_for_block(doc_id, block_id)?;
        let current = *self.docs.get(&doc_id)?.blocks.get(&block_id)?.checked.reg.value();
        let next = !current;
        let (op, ts) = self.next_op(now_ms);
        self.docs
            .entry(doc_id)
            .or_default()
            .blocks
            .entry(block_id)
            .or_default()
            .checked
            .set(next, ts, op);
        Some(next)
    }

    pub fn reorder_doc_block(
        &mut self,
        now_ms: u64,
        doc_id: u32,
        block_id: u32,
        after_block_id: Option<u32>,
    ) -> bool {
        if after_block_id == Some(block_id) {
            return false;
        }
        let old_ids = self.all_visible_rga_ids_for_block(doc_id, block_id);
        if old_ids.is_empty() {
            return false;
        }
        let anchor_rga_id = match after_block_id {
            None => None,
            Some(target) => match self.latest_rga_id_for_block(doc_id, target) {
                Some(id) => Some(id),
                None => return false,
            },
        };
        let (del_op, _) = self.next_op(now_ms);
        let (ins_op, _) = self.next_op(now_ms);
        let doc = self.docs.entry(doc_id).or_default();
        for old_id in old_ids {
            doc.block_order.delete_with_op(old_id, del_op);
        }
        doc.block_order.insert_after(anchor_rga_id, ins_op, block_id);
        true
    }

    pub fn delete_doc_block(&mut self, now_ms: u64, doc_id: u32, block_id: u32) -> bool {
        let ids = self.all_visible_rga_ids_for_block(doc_id, block_id);
        if ids.is_empty() {
            return false;
        }
        let (del_op, _) = self.next_op(now_ms);
        let doc = self.docs.entry(doc_id).or_default();
        let mut changed = false;
        for id in ids {
            if doc.block_order.delete_with_op(id, del_op) {
                changed = true;
            }
        }
        changed
    }

    fn all_visible_rga_ids_for_block(&self, doc_id: u32, block_id: u32) -> Vec<OpId> {
        let Some(doc) = self.docs.get(&doc_id) else {
            return Vec::new();
        };
        doc.block_order
            .visible_nodes()
            .into_iter()
            .filter(|n| n.value == block_id)
            .map(|n| n.id)
            .collect()
    }

    fn latest_rga_id_for_block(&self, doc_id: u32, block_id: u32) -> Option<OpId> {
        self.all_visible_rga_ids_for_block(doc_id, block_id)
            .into_iter()
            .max()
    }

    // -------------------------------------------------------------------------
    // Binary delta encode / merge (`NVC1`)
    // -------------------------------------------------------------------------

    /// Encodes all CRDT operations not yet observed by `since` into an `NVC1` binary frame.
    pub fn delta_since(&self, since: &VersionVector) -> Vec<u8> {
        let mut records = Vec::new();

        push_orset_delta(NS_TASK_SET, &self.task_set, since, &mut records);
        for (&task_id, t) in &self.tasks {
            let eid = u64::from(task_id);
            push_lww_str(NS_TASK_FIELDS, eid, FIELD_TASK_KEY, &t.key, since, &mut records);
            push_lww_str(NS_TASK_FIELDS, eid, FIELD_TASK_TITLE, &t.title, since, &mut records);
            push_lww_str(NS_TASK_FIELDS, eid, FIELD_TASK_ASSIGNEE, &t.assignee, since, &mut records);
            push_lww_str(NS_TASK_FIELDS, eid, FIELD_TASK_TAG, &t.tag, since, &mut records);
            push_lww_u32(
                NS_TASK_FIELDS,
                eid,
                FIELD_TASK_PRIORITY,
                encode_priority(*t.priority.reg.value()),
                t.priority.reg.timestamp(),
                t.priority.op_id,
                since,
                &mut records,
            );
            push_lww_u32(
                NS_TASK_FIELDS,
                eid,
                FIELD_TASK_STATUS,
                encode_status(*t.status.reg.value()),
                t.status.reg.timestamp(),
                t.status.op_id,
                since,
                &mut records,
            );
        }

        push_orset_delta(NS_CHAT_ENG_SET, &self.eng_set, since, &mut records);
        push_orset_delta(NS_CHAT_DESIGN_SET, &self.design_set, since, &mut records);
        for (&entity_id, m) in &self.messages {
            push_lww_str(NS_CHAT_FIELDS, entity_id, FIELD_CHAT_AUTHOR, &m.author, since, &mut records);
            push_lww_str(NS_CHAT_FIELDS, entity_id, FIELD_CHAT_BADGE_BG, &m.badge_bg, since, &mut records);
            push_lww_str(NS_CHAT_FIELDS, entity_id, FIELD_CHAT_BADGE_FG, &m.badge_fg, since, &mut records);
            push_lww_str(NS_CHAT_FIELDS, entity_id, FIELD_CHAT_TIMESTAMP, &m.timestamp, since, &mut records);
            push_lww_str(NS_CHAT_FIELDS, entity_id, FIELD_CHAT_BODY, &m.body, since, &mut records);
            for (peer, pos, neg) in m.reactions.peer_entries() {
                let op_id = m.reaction_ops.get(&peer).copied().unwrap_or(OpId::ZERO);
                if !since.contains(op_id) {
                    records.push(CrdtRecord::PnCounterSet {
                        namespace: NS_CHAT_REACTIONS,
                        entity_id,
                        peer,
                        op_id,
                        pos,
                        neg,
                    });
                }
            }
        }

        push_orset_delta(NS_DOC_SET, &self.doc_set, since, &mut records);
        for (&doc_id, d) in &self.docs {
            let did = u64::from(doc_id);
            push_lww_str(NS_DOC_FIELDS, did, FIELD_DOC_SLUG, &d.slug, since, &mut records);
            push_lww_str(NS_DOC_FIELDS, did, FIELD_DOC_TITLE, &d.title, since, &mut records);
            push_lww_str(NS_DOC_FIELDS, did, FIELD_DOC_UPDATED_AT, &d.updated_at, since, &mut records);

            let rga_delta = d.block_order.delta_since(since);
            for (id, after_id, block_id) in rga_delta.inserts {
                records.push(CrdtRecord::RgaInsert {
                    namespace: NS_DOC_BLOCKS_RGA,
                    list_id: did,
                    id,
                    after_id,
                    value: CrdtValue::U32(block_id),
                });
            }
            for (target_id, remove_op) in rga_delta.deletes {
                records.push(CrdtRecord::RgaDelete {
                    namespace: NS_DOC_BLOCKS_RGA,
                    list_id: did,
                    target_id,
                    remove_op,
                });
            }

            for (&block_id, b) in &d.blocks {
                let beid = block_entity_id(doc_id, block_id);
                push_lww_u32(
                    NS_DOC_BLOCK_FIELDS,
                    beid,
                    FIELD_BLOCK_KIND,
                    *b.kind_tag.reg.value(),
                    b.kind_tag.reg.timestamp(),
                    b.kind_tag.op_id,
                    since,
                    &mut records,
                );
                if b.checked.op_id.seq > 0 && !since.contains(b.checked.op_id) {
                    records.push(CrdtRecord::LwwSet {
                        namespace: NS_DOC_BLOCK_FIELDS,
                        entity_id: beid,
                        field_id: FIELD_BLOCK_CHECKED,
                        op_id: b.checked.op_id,
                        timestamp: b.checked.reg.timestamp(),
                        value: CrdtValue::Bool(*b.checked.reg.value()),
                    });
                }
                push_lww_str(NS_DOC_BLOCK_FIELDS, beid, FIELD_BLOCK_TEXT, &b.text, since, &mut records);
            }
        }

        let packet = CrdtDeltaPacket::new(self.peer, self.vv.clone(), records);
        packet
            .encode()
            .expect("StudioCrdtDoc packet stays within NVC1 limits")
    }

    /// Validates and merges an `NVC1` binary delta packet idempotently.
    ///
    /// Guarantees atomicity against malformed/corrupt inputs: if validation fails, `self`
    /// is left completely untouched.
    pub fn merge_delta(&mut self, bytes: &[u8]) -> Result<CrdtMergeSummary, CrdtWireError> {
        let packet = CrdtDeltaPacket::decode(bytes)?;
        validate_studio_records(&packet.records)?;

        let mut summary = CrdtMergeSummary {
            sender: packet.sender,
            records_received: packet.records.len(),
            ..Default::default()
        };

        let local_now = self.clock.last_timestamp().physical_ms;
        for rec in packet.records {
            match rec {
                CrdtRecord::OrSetAddU64 {
                    namespace,
                    key,
                    op_id,
                } => {
                    let changed = match namespace {
                        NS_TASK_SET => {
                            let c = self.task_set.add(key, op_id);
                            if c {
                                summary.tasks_changed += 1;
                            }
                            c
                        }
                        NS_CHAT_ENG_SET => {
                            let c = self.eng_set.add(key, op_id);
                            if c {
                                summary.messages_changed += 1;
                            }
                            c
                        }
                        NS_CHAT_DESIGN_SET => {
                            let c = self.design_set.add(key, op_id);
                            if c {
                                summary.messages_changed += 1;
                            }
                            c
                        }
                        NS_DOC_SET => {
                            let c = self.doc_set.add(key, op_id);
                            if c {
                                summary.docs_changed += 1;
                            }
                            c
                        }
                        _ => false,
                    };
                    if changed {
                        summary.records_applied += 1;
                    }
                    self.observe_op(op_id);
                }
                CrdtRecord::OrSetRemove {
                    namespace,
                    removed_tag,
                    remove_op,
                } => {
                    let changed = match namespace {
                        NS_TASK_SET => {
                            let c = self.task_set.remove_tag(removed_tag, remove_op);
                            if c {
                                summary.tasks_changed += 1;
                            }
                            c
                        }
                        NS_CHAT_ENG_SET => {
                            let c = self.eng_set.remove_tag(removed_tag, remove_op);
                            if c {
                                summary.messages_changed += 1;
                            }
                            c
                        }
                        NS_CHAT_DESIGN_SET => {
                            let c = self.design_set.remove_tag(removed_tag, remove_op);
                            if c {
                                summary.messages_changed += 1;
                            }
                            c
                        }
                        NS_DOC_SET => {
                            let c = self.doc_set.remove_tag(removed_tag, remove_op);
                            if c {
                                summary.docs_changed += 1;
                            }
                            c
                        }
                        _ => false,
                    };
                    if changed {
                        summary.records_applied += 1;
                    }
                    self.observe_op(remove_op);
                }
                CrdtRecord::LwwSet {
                    namespace,
                    entity_id,
                    field_id,
                    op_id,
                    timestamp,
                    value,
                } => {
                    self.clock.receive(local_now, timestamp);
                    self.observe_op(op_id);
                    let changed = self.apply_lww(namespace, entity_id, field_id, op_id, timestamp, value);
                    if changed {
                        summary.records_applied += 1;
                        match namespace {
                            NS_TASK_FIELDS => summary.tasks_changed += 1,
                            NS_CHAT_FIELDS => summary.messages_changed += 1,
                            NS_DOC_FIELDS => summary.docs_changed += 1,
                            NS_DOC_BLOCK_FIELDS => summary.blocks_changed += 1,
                            _ => {}
                        }
                    }
                }
                CrdtRecord::PnCounterSet {
                    namespace: NS_CHAT_REACTIONS,
                    entity_id,
                    peer,
                    op_id,
                    pos,
                    neg,
                } => {
                    self.observe_op(op_id);
                    let entry = self.messages.entry(entity_id).or_default();
                    let changed = entry.reactions.merge_peer(peer, pos, neg);
                    if op_id.seq > 0 {
                        entry
                            .reaction_ops
                            .entry(peer)
                            .and_modify(|existing| {
                                if op_id > *existing {
                                    *existing = op_id;
                                }
                            })
                            .or_insert(op_id);
                    }
                    if changed {
                        summary.records_applied += 1;
                        summary.messages_changed += 1;
                    }
                }
                CrdtRecord::RgaInsert {
                    namespace: NS_DOC_BLOCKS_RGA,
                    list_id,
                    id,
                    after_id,
                    value: CrdtValue::U32(block_id),
                } => {
                    self.observe_op(id);
                    let doc_id = list_id as u32;
                    let changed = self
                        .docs
                        .entry(doc_id)
                        .or_default()
                        .block_order
                        .insert_after(after_id, id, block_id);
                    if changed {
                        summary.records_applied += 1;
                        summary.blocks_changed += 1;
                    }
                }
                CrdtRecord::RgaDelete {
                    namespace: NS_DOC_BLOCKS_RGA,
                    list_id,
                    target_id,
                    remove_op,
                } => {
                    self.observe_op(remove_op);
                    let doc_id = list_id as u32;
                    let changed = self
                        .docs
                        .entry(doc_id)
                        .or_default()
                        .block_order
                        .delete_with_op(target_id, remove_op);
                    if changed {
                        summary.records_applied += 1;
                        summary.blocks_changed += 1;
                    }
                }
                _ => {}
            }
        }

        for (_, seq) in packet.version_vector.iter() {
            if seq > self.local_seq {
                self.local_seq = seq;
            }
        }
        self.vv.merge(&packet.version_vector);
        Ok(summary)
    }

    fn apply_lww(
        &mut self,
        namespace: u16,
        entity_id: u64,
        field_id: u16,
        op_id: OpId,
        ts: HlcTimestamp,
        value: CrdtValue,
    ) -> bool {
        match (namespace, field_id, value) {
            (NS_TASK_FIELDS, FIELD_TASK_KEY, CrdtValue::Utf8(s)) => {
                self.tasks.entry(entity_id as u32).or_default().key.set(s, ts, op_id)
            }
            (NS_TASK_FIELDS, FIELD_TASK_TITLE, CrdtValue::Utf8(s)) => {
                self.tasks.entry(entity_id as u32).or_default().title.set(s, ts, op_id)
            }
            (NS_TASK_FIELDS, FIELD_TASK_ASSIGNEE, CrdtValue::Utf8(s)) => {
                self.tasks.entry(entity_id as u32).or_default().assignee.set(s, ts, op_id)
            }
            (NS_TASK_FIELDS, FIELD_TASK_TAG, CrdtValue::Utf8(s)) => {
                self.tasks.entry(entity_id as u32).or_default().tag.set(s, ts, op_id)
            }
            (NS_TASK_FIELDS, FIELD_TASK_PRIORITY, CrdtValue::U32(p)) => {
                if let Some(priority) = decode_priority(p) {
                    self.tasks
                        .entry(entity_id as u32)
                        .or_default()
                        .priority
                        .set(priority, ts, op_id)
                } else {
                    false
                }
            }
            (NS_TASK_FIELDS, FIELD_TASK_STATUS, CrdtValue::U32(s)) => {
                if let Some(status) = decode_status(s) {
                    self.tasks
                        .entry(entity_id as u32)
                        .or_default()
                        .status
                        .set(status, ts, op_id)
                } else {
                    false
                }
            }
            (NS_CHAT_FIELDS, FIELD_CHAT_AUTHOR, CrdtValue::Utf8(s)) => {
                self.messages.entry(entity_id).or_default().author.set(s, ts, op_id)
            }
            (NS_CHAT_FIELDS, FIELD_CHAT_BADGE_BG, CrdtValue::Utf8(s)) => {
                self.messages.entry(entity_id).or_default().badge_bg.set(s, ts, op_id)
            }
            (NS_CHAT_FIELDS, FIELD_CHAT_BADGE_FG, CrdtValue::Utf8(s)) => {
                self.messages.entry(entity_id).or_default().badge_fg.set(s, ts, op_id)
            }
            (NS_CHAT_FIELDS, FIELD_CHAT_TIMESTAMP, CrdtValue::Utf8(s)) => {
                self.messages.entry(entity_id).or_default().timestamp.set(s, ts, op_id)
            }
            (NS_CHAT_FIELDS, FIELD_CHAT_BODY, CrdtValue::Utf8(s)) => {
                self.messages.entry(entity_id).or_default().body.set(s, ts, op_id)
            }
            (NS_DOC_FIELDS, FIELD_DOC_SLUG, CrdtValue::Utf8(s)) => {
                self.docs.entry(entity_id as u32).or_default().slug.set(s, ts, op_id)
            }
            (NS_DOC_FIELDS, FIELD_DOC_TITLE, CrdtValue::Utf8(s)) => {
                self.docs.entry(entity_id as u32).or_default().title.set(s, ts, op_id)
            }
            (NS_DOC_FIELDS, FIELD_DOC_UPDATED_AT, CrdtValue::Utf8(s)) => {
                self.docs.entry(entity_id as u32).or_default().updated_at.set(s, ts, op_id)
            }
            (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_KIND, CrdtValue::U32(k)) => {
                let (doc_id, block_id) = split_block_entity_id(entity_id);
                self.docs
                    .entry(doc_id)
                    .or_default()
                    .blocks
                    .entry(block_id)
                    .or_default()
                    .kind_tag
                    .set(k, ts, op_id)
            }
            (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_CHECKED, CrdtValue::Bool(c)) => {
                let (doc_id, block_id) = split_block_entity_id(entity_id);
                self.docs
                    .entry(doc_id)
                    .or_default()
                    .blocks
                    .entry(block_id)
                    .or_default()
                    .checked
                    .set(c, ts, op_id)
            }
            (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_TEXT, CrdtValue::Utf8(s)) => {
                let (doc_id, block_id) = split_block_entity_id(entity_id);
                self.docs
                    .entry(doc_id)
                    .or_default()
                    .blocks
                    .entry(block_id)
                    .or_default()
                    .text
                    .set(s, ts, op_id)
            }
            _ => false,
        }
    }

    // -------------------------------------------------------------------------
    // Deterministic snapshot projection
    // -------------------------------------------------------------------------

    /// Projects the converged CRDT state deterministically onto `base`, preserving local UI
    /// view/layout state while replacing tasks, channels, and docs with canonical CRDT state.
    pub fn project_snapshot(&self, base: &WorkspaceSnapshot) -> WorkspaceSnapshot {
        let mut out = base.clone();

        let mut tasks = Vec::with_capacity(self.task_set.len());
        let mut max_task_id = 0u32;
        for &raw_id in self.task_set.iter() {
            let id = raw_id as u32;
            max_task_id = max_task_id.max(id);
            if let Some(t) = self.tasks.get(&id) {
                tasks.push(TaskItem {
                    id,
                    key: t.key.reg.value().clone(),
                    title: t.title.reg.value().clone(),
                    assignee: t.assignee.reg.value().clone(),
                    tag: t.tag.reg.value().clone(),
                    priority: *t.priority.reg.value(),
                    status: *t.status.reg.value(),
                });
            }
        }
        out.tasks = tasks;
        out.next_task_id = max_task_id.saturating_add(1);

        let (eng_messages, max_eng) = self.project_channel(ChannelKind::Engineering, &self.eng_set);
        let (design_messages, max_des) = self.project_channel(ChannelKind::Design, &self.design_set);
        out.eng_messages = eng_messages;
        out.design_messages = design_messages;
        out.next_msg_id = max_eng.max(max_des).saturating_add(1);

        let mut docs = Vec::with_capacity(self.doc_set.len());
        let mut max_doc_id = 0u32;
        let mut max_block_id = 0u32;
        for &raw_doc_id in self.doc_set.iter() {
            let doc_id = raw_doc_id as u32;
            max_doc_id = max_doc_id.max(doc_id);
            if let Some(d) = self.docs.get(&doc_id) {
                let visible = d.block_order.visible_nodes();
                // If concurrent moves created duplicate RGA nodes for one block_id, keep the
                // node with the highest OpId deterministically.
                let mut winner_op = BTreeMap::<u32, OpId>::new();
                for node in &visible {
                    winner_op
                        .entry(node.value)
                        .and_modify(|cur| {
                            if node.id > *cur {
                                *cur = node.id;
                            }
                        })
                        .or_insert(node.id);
                }

                let mut seen = BTreeSet::new();
                let mut blocks = Vec::with_capacity(visible.len());
                for node in visible {
                    if winner_op.get(&node.value) != Some(&node.id) || !seen.insert(node.value) {
                        continue;
                    }
                    let block_id = node.value;
                    max_block_id = max_block_id.max(block_id);
                    if let Some(b) = d.blocks.get(&block_id) {
                        blocks.push(DocBlock {
                            id: block_id,
                            kind: decode_block_kind(*b.kind_tag.reg.value(), *b.checked.reg.value()),
                            text: b.text.reg.value().clone(),
                        });
                    }
                }
                docs.push(WorkspaceDoc {
                    id: doc_id,
                    slug: d.slug.reg.value().clone(),
                    title: d.title.reg.value().clone(),
                    updated_at: d.updated_at.reg.value().clone(),
                    blocks,
                });
            }
        }
        if !docs.iter().any(|d| d.id == out.active_doc_id) {
            out.active_doc_id = docs.first().map_or(0, |d| d.id);
        }
        out.docs = docs;
        out.next_doc_id = max_doc_id.saturating_add(1);
        out.next_block_id = max_block_id.saturating_add(1);

        out
    }

    fn project_channel(&self, channel: ChannelKind, set: &OrSet<u64>) -> (Vec<ChatMessage>, u32) {
        let mut list = Vec::with_capacity(set.len());
        let mut max_id = 0u32;
        for &raw_id in set.iter() {
            let id = raw_id as u32;
            max_id = max_id.max(id);
            let entity = chat_entity_id(channel, id);
            if let Some(m) = self.messages.get(&entity) {
                list.push(ChatMessage {
                    id,
                    author: m.author.reg.value().clone(),
                    badge_bg: m.badge_bg.reg.value().clone(),
                    badge_fg: m.badge_fg.reg.value().clone(),
                    timestamp: m.timestamp.reg.value().clone(),
                    body: m.body.reg.value().clone(),
                    reactions: m.reactions.clamped_u32(),
                });
            }
        }
        (list, max_id)
    }
}

// -----------------------------------------------------------------------------
// Encoding / validation helpers
// -----------------------------------------------------------------------------

#[inline]
const fn seed_op(category: u8, payload: u64) -> OpId {
    OpId::new(PeerId::SEED, ((category as u64) << 40) | (payload & 0x00FF_FFFF_FFFF))
}

#[inline]
const fn channel_bit(channel: ChannelKind) -> u64 {
    match channel {
        ChannelKind::Engineering => 0,
        ChannelKind::Design => 1,
    }
}

#[inline]
const fn chat_entity_id(channel: ChannelKind, msg_id: u32) -> u64 {
    (channel_bit(channel) << 32) | (msg_id as u64)
}

#[inline]
const fn block_entity_id(doc_id: u32, block_id: u32) -> u64 {
    ((doc_id as u64) << 32) | (block_id as u64)
}

#[inline]
const fn split_block_entity_id(entity_id: u64) -> (u32, u32) {
    ((entity_id >> 32) as u32, entity_id as u32)
}

fn peer_bucket_u16(peer: PeerId) -> u16 {
    if peer.0 >= 1 && peer.0 <= u64::from(u16::MAX) {
        peer.0 as u16
    } else {
        let h = fnv1a_64(&peer.to_le_bytes());
        ((h % u64::from(u16::MAX - 1)) as u16) + 1
    }
}

const fn encode_priority(p: Priority) -> u32 {
    match p {
        Priority::P0 => 0,
        Priority::P1 => 1,
        Priority::P2 => 2,
    }
}

const fn decode_priority(v: u32) -> Option<Priority> {
    match v {
        0 => Some(Priority::P0),
        1 => Some(Priority::P1),
        2 => Some(Priority::P2),
        _ => None,
    }
}

const fn encode_status(s: TaskStatus) -> u32 {
    match s {
        TaskStatus::Todo => 0,
        TaskStatus::InProgress => 1,
        TaskStatus::Done => 2,
    }
}

const fn decode_status(v: u32) -> Option<TaskStatus> {
    match v {
        0 => Some(TaskStatus::Todo),
        1 => Some(TaskStatus::InProgress),
        2 => Some(TaskStatus::Done),
        _ => None,
    }
}

const fn encode_block_kind(kind: DocBlockKind) -> (u32, bool) {
    match kind {
        DocBlockKind::Heading1 => (0, false),
        DocBlockKind::Heading2 => (1, false),
        DocBlockKind::Paragraph => (2, false),
        DocBlockKind::Checklist { checked } => (3, checked),
        DocBlockKind::Code => (4, false),
        DocBlockKind::Callout => (5, false),
    }
}

const fn decode_block_kind(kind_tag: u32, checked: bool) -> DocBlockKind {
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

fn push_orset_delta(
    namespace: u16,
    set: &OrSet<u64>,
    since: &VersionVector,
    out: &mut Vec<CrdtRecord>,
) {
    let delta = set.delta_since(since);
    for (key, op_id) in delta.adds {
        out.push(CrdtRecord::OrSetAddU64 {
            namespace,
            key,
            op_id,
        });
    }
    for (removed_tag, remove_op) in delta.removes {
        out.push(CrdtRecord::OrSetRemove {
            namespace,
            removed_tag,
            remove_op,
        });
    }
}

fn push_lww_str(
    namespace: u16,
    entity_id: u64,
    field_id: u16,
    entry: &TrackedLww<String>,
    since: &VersionVector,
    out: &mut Vec<CrdtRecord>,
) {
    if entry.op_id.seq > 0 && !since.contains(entry.op_id) {
        out.push(CrdtRecord::LwwSet {
            namespace,
            entity_id,
            field_id,
            op_id: entry.op_id,
            timestamp: entry.reg.timestamp(),
            value: CrdtValue::Utf8(entry.reg.value().clone()),
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_lww_u32(
    namespace: u16,
    entity_id: u64,
    field_id: u16,
    val: u32,
    timestamp: HlcTimestamp,
    op_id: OpId,
    since: &VersionVector,
    out: &mut Vec<CrdtRecord>,
) {
    if op_id.seq > 0 && !since.contains(op_id) {
        out.push(CrdtRecord::LwwSet {
            namespace,
            entity_id,
            field_id,
            op_id,
            timestamp,
            value: CrdtValue::U32(val),
        });
    }
}

fn validate_studio_records(records: &[CrdtRecord]) -> Result<(), CrdtWireError> {
    for rec in records {
        match rec {
            CrdtRecord::OrSetAddU64 { namespace, .. }
            | CrdtRecord::OrSetRemove { namespace, .. } => {
                if !matches!(
                    *namespace,
                    NS_TASK_SET | NS_CHAT_ENG_SET | NS_CHAT_DESIGN_SET | NS_DOC_SET
                ) {
                    return Err(CrdtWireError::InvalidPayload("unknown Studio OR-Set namespace"));
                }
            }
            CrdtRecord::LwwSet {
                namespace,
                field_id,
                value,
                ..
            } => match (*namespace, *field_id, value) {
                (
                    NS_TASK_FIELDS,
                    FIELD_TASK_KEY | FIELD_TASK_TITLE | FIELD_TASK_ASSIGNEE | FIELD_TASK_TAG,
                    CrdtValue::Utf8(_),
                )
                | (
                    NS_CHAT_FIELDS,
                    FIELD_CHAT_AUTHOR
                    | FIELD_CHAT_BADGE_BG
                    | FIELD_CHAT_BADGE_FG
                    | FIELD_CHAT_TIMESTAMP
                    | FIELD_CHAT_BODY,
                    CrdtValue::Utf8(_),
                )
                | (
                    NS_DOC_FIELDS,
                    FIELD_DOC_SLUG | FIELD_DOC_TITLE | FIELD_DOC_UPDATED_AT,
                    CrdtValue::Utf8(_),
                )
                | (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_TEXT, CrdtValue::Utf8(_))
                | (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_CHECKED, CrdtValue::Bool(_)) => {}
                (NS_TASK_FIELDS, FIELD_TASK_PRIORITY, CrdtValue::U32(v))
                    if decode_priority(*v).is_some() => {}
                (NS_TASK_FIELDS, FIELD_TASK_STATUS, CrdtValue::U32(v))
                    if decode_status(*v).is_some() => {}
                (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_KIND, CrdtValue::U32(v)) if *v <= 5 => {}
                _ => return Err(CrdtWireError::InvalidPayload("invalid Studio LWW field/value")),
            },
            CrdtRecord::PnCounterSet { namespace, .. } => {
                if *namespace != NS_CHAT_REACTIONS {
                    return Err(CrdtWireError::InvalidPayload(
                        "unknown Studio PnCounter namespace",
                    ));
                }
            }
            CrdtRecord::RgaInsert {
                namespace, value, ..
            } => {
                if *namespace != NS_DOC_BLOCKS_RGA || !matches!(value, CrdtValue::U32(_)) {
                    return Err(CrdtWireError::InvalidPayload("invalid Studio RGA insert"));
                }
            }
            CrdtRecord::RgaDelete { namespace, .. } => {
                if *namespace != NS_DOC_BLOCKS_RGA {
                    return Err(CrdtWireError::InvalidPayload("invalid Studio RGA delete"));
                }
            }
            CrdtRecord::GCounterSet { .. } | CrdtRecord::OrSetAddString { .. } => {
                return Err(CrdtWireError::InvalidPayload(
                    "unsupported record kind for StudioCrdtDoc",
                ));
            }
        }
    }
    Ok(())
}
