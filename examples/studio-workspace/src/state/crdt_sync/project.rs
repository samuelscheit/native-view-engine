//! Deterministic projection of converged [`StudioCrdtDoc`] state onto [`WorkspaceSnapshot`].

use std::collections::{BTreeMap, BTreeSet};

use nve_interop::crdt::{OpId, OrSet};

use super::{StudioCrdtDoc, chat_entity_id, decode_block_kind};
use crate::state::{ChannelKind, ChatMessage, DocBlock, TaskItem, WorkspaceDoc, WorkspaceSnapshot};

impl StudioCrdtDoc {
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
        out.tasks = tasks.into();
        out.next_task_id = max_task_id.saturating_add(1);

        let (eng_messages, max_eng) = self.project_channel(ChannelKind::Engineering, &self.eng_set);
        let (design_messages, max_des) =
            self.project_channel(ChannelKind::Design, &self.design_set);
        out.eng_messages = eng_messages.into();
        out.design_messages = design_messages.into();
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
                            kind: decode_block_kind(
                                *b.kind_tag.reg.value(),
                                *b.checked.reg.value(),
                            ),
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
        out.docs = docs.into();
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
                    timestamp: m.timestamp.reg.value().clone(),
                    body: m.body.reg.value().clone(),
                    reactions: m.reactions.clamped_u32(),
                });
            }
        }
        (list, max_id)
    }
}
