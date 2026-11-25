//! Local CRDT mutation operations on [`StudioCrdtDoc`] (tasks, chat messages, docs & blocks).

use std::collections::BTreeMap;

use nve_interop::crdt::{OpId, PnCounter, RgaList};

use super::{
    ChatMessageCrdt, DocBlockCrdt, DocCrdt, StudioCrdtDoc, TaskCrdt, TrackedLww, chat_entity_id,
    encode_block_kind,
};
use crate::state::{ChannelKind, DocBlockKind, Priority, TaskStatus};

impl StudioCrdtDoc {
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
        self.tasks
            .entry(task_id)
            .or_default()
            .title
            .set(title.into(), ts, op)
    }

    pub fn edit_task_status(&mut self, now_ms: u64, task_id: u32, status: TaskStatus) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        self.tasks
            .entry(task_id)
            .or_default()
            .status
            .set(status, ts, op)
    }

    pub fn edit_task_priority(&mut self, now_ms: u64, task_id: u32, priority: Priority) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, ts) = self.next_op(now_ms);
        self.tasks
            .entry(task_id)
            .or_default()
            .priority
            .set(priority, ts, op)
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
        self.tasks
            .entry(task_id)
            .or_default()
            .tag
            .set(tag.into(), ts, op)
    }

    pub fn delete_task(&mut self, now_ms: u64, task_id: u32) -> bool {
        if !self.task_set.contains(&u64::from(task_id)) {
            return false;
        }
        let (op, _) = self.next_op(now_ms);
        !self
            .task_set
            .remove_with_op(&u64::from(task_id), op)
            .is_empty()
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
        !self
            .doc_set
            .remove_with_op(&u64::from(doc_id), op)
            .is_empty()
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
        let current = *self
            .docs
            .get(&doc_id)?
            .blocks
            .get(&block_id)?
            .checked
            .reg
            .value();
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
}
