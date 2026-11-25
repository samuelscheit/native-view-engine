//! Binary `NVC1` delta encoding, validation, and idempotent merge for [`StudioCrdtDoc`].

use nve_interop::crdt::{
    NveCrdtDelta, CrdtRecord, CrdtValue, CrdtWireError, HlcTimestamp, OpId, OrSet,
    VersionVector,
};

use super::{
    CrdtMergeSummary, FIELD_BLOCK_CHECKED, FIELD_BLOCK_KIND, FIELD_BLOCK_TEXT, FIELD_CHAT_AUTHOR,
    FIELD_CHAT_BADGE_BG, FIELD_CHAT_BADGE_FG, FIELD_CHAT_BODY, FIELD_CHAT_TIMESTAMP,
    FIELD_DOC_SLUG, FIELD_DOC_TITLE, FIELD_DOC_UPDATED_AT, FIELD_TASK_ASSIGNEE, FIELD_TASK_KEY,
    FIELD_TASK_PRIORITY, FIELD_TASK_STATUS, FIELD_TASK_TAG, FIELD_TASK_TITLE, NS_CHAT_DESIGN_SET,
    NS_CHAT_ENG_SET, NS_CHAT_FIELDS, NS_CHAT_REACTIONS, NS_DOC_BLOCK_FIELDS, NS_DOC_BLOCKS_RGA,
    NS_DOC_FIELDS, NS_DOC_SET, NS_TASK_FIELDS, NS_TASK_SET, StudioCrdtDoc, TrackedLww,
    block_entity_id, decode_priority, decode_status, encode_priority, encode_status,
    split_block_entity_id,
};

impl StudioCrdtDoc {
    /// Encodes all CRDT operations not yet observed by `since` into an `NVC1` binary frame.
    pub fn delta_since(&self, since: &VersionVector) -> Vec<u8> {
        let mut records = Vec::new();

        push_orset_delta(NS_TASK_SET, &self.task_set, since, &mut records);
        for (&task_id, t) in &self.tasks {
            let eid = u64::from(task_id);
            push_lww_str(NS_TASK_FIELDS, eid, FIELD_TASK_KEY, &t.key, since, &mut records);
            push_lww_str(NS_TASK_FIELDS, eid, FIELD_TASK_TITLE, &t.title, since, &mut records);
            push_lww_str(
                NS_TASK_FIELDS,
                eid,
                FIELD_TASK_ASSIGNEE,
                &t.assignee,
                since,
                &mut records,
            );
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
            push_lww_str(
                NS_CHAT_FIELDS,
                entity_id,
                FIELD_CHAT_AUTHOR,
                &m.author,
                since,
                &mut records,
            );
            push_lww_str(
                NS_CHAT_FIELDS,
                entity_id,
                FIELD_CHAT_BADGE_BG,
                &m.badge_bg,
                since,
                &mut records,
            );
            push_lww_str(
                NS_CHAT_FIELDS,
                entity_id,
                FIELD_CHAT_BADGE_FG,
                &m.badge_fg,
                since,
                &mut records,
            );
            push_lww_str(
                NS_CHAT_FIELDS,
                entity_id,
                FIELD_CHAT_TIMESTAMP,
                &m.timestamp,
                since,
                &mut records,
            );
            push_lww_str(
                NS_CHAT_FIELDS,
                entity_id,
                FIELD_CHAT_BODY,
                &m.body,
                since,
                &mut records,
            );
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
            push_lww_str(
                NS_DOC_FIELDS,
                did,
                FIELD_DOC_UPDATED_AT,
                &d.updated_at,
                since,
                &mut records,
            );

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
                push_lww_str(
                    NS_DOC_BLOCK_FIELDS,
                    beid,
                    FIELD_BLOCK_TEXT,
                    &b.text,
                    since,
                    &mut records,
                );
            }
        }

        let packet = NveCrdtDelta::new(self.peer, self.vv.clone(), records);
        packet
            .encode()
            .expect("StudioCrdtDoc packet stays within NVC1 limits")
    }

    /// Validates and merges an `NVC1` binary delta packet idempotently.
    ///
    /// Guarantees atomicity against malformed/corrupt inputs: if validation fails, `self`
    /// is left completely untouched.
    pub fn merge_delta(&mut self, bytes: &[u8]) -> Result<CrdtMergeSummary, CrdtWireError> {
        let packet = NveCrdtDelta::decode(bytes)?;
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
                    let changed =
                        self.apply_lww(namespace, entity_id, field_id, op_id, timestamp, value);
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
