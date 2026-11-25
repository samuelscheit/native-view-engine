//! Low-level little-endian binary reader/writer primitives and sub-record encoders/decoders
//! for the `NWS1` v2 workspace codec.

use crate::ui_islands::{IslandCardState, IslandHostKind, IslandsSnapshot};

use super::docs::{DocBlock, DocBlockKind, WorkspaceDoc};
use super::models::{ChatMessage, Priority, TaskItem, TaskStatus};
use super::workbench::{WorkbenchPreset, WorkbenchState};

const MAX_ITEMS: usize = 16_384;

pub(super) struct BinWriter {
    pub(super) buf: Vec<u8>,
}

impl BinWriter {
    pub(super) fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    pub(super) fn push_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub(super) fn push_bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }

    pub(super) fn push_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(super) fn push_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(super) fn push_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(super) fn push_str(&mut self, s: &str) {
        let b = s.as_bytes();
        self.push_u32(b.len() as u32);
        self.buf.extend_from_slice(b);
    }
}

pub(super) struct BinReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> BinReader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(super) fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.bytes.len().saturating_sub(self.pos) < n {
            return None;
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }

    pub(super) fn read_u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    pub(super) fn read_bool(&mut self) -> Option<bool> {
        match self.read_u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    pub(super) fn read_u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    pub(super) fn read_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    pub(super) fn read_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    pub(super) fn read_count(&mut self) -> Option<usize> {
        let c = self.read_u32()? as usize;
        (c <= MAX_ITEMS).then_some(c)
    }

    pub(super) fn read_str(&mut self) -> Option<String> {
        let len = self.read_u32()? as usize;
        let slice = self.take(len)?;
        std::str::from_utf8(slice).ok().map(str::to_owned)
    }

    pub(super) fn finish(self) -> Option<()> {
        (self.pos == self.bytes.len()).then_some(())
    }
}

pub(super) fn encode_task(w: &mut BinWriter, t: &TaskItem) {
    w.push_u32(t.id);
    w.push_str(&t.key);
    w.push_str(&t.title);
    w.push_str(&t.assignee);
    w.push_str(&t.tag);
    w.push_u8(match t.priority {
        Priority::P0 => 0,
        Priority::P1 => 1,
        Priority::P2 => 2,
    });
    w.push_u8(match t.status {
        TaskStatus::Todo => 0,
        TaskStatus::InProgress => 1,
        TaskStatus::Done => 2,
    });
}

pub(super) fn decode_task(r: &mut BinReader<'_>) -> Option<TaskItem> {
    let id = r.read_u32()?;
    let key = r.read_str()?;
    let title = r.read_str()?;
    let assignee = r.read_str()?;
    let tag = r.read_str()?;
    let priority = match r.read_u8()? {
        0 => Priority::P0,
        1 => Priority::P1,
        2 => Priority::P2,
        _ => return None,
    };
    let status = match r.read_u8()? {
        0 => TaskStatus::Todo,
        1 => TaskStatus::InProgress,
        2 => TaskStatus::Done,
        _ => return None,
    };
    Some(TaskItem {
        id,
        key,
        title,
        assignee,
        tag,
        priority,
        status,
    })
}

pub(super) fn encode_chat_msg(w: &mut BinWriter, m: &ChatMessage) {
    w.push_u32(m.id);
    w.push_str(&m.author);
    w.push_str(&m.badge_bg);
    w.push_str(&m.badge_fg);
    w.push_str(&m.timestamp);
    w.push_str(&m.body);
    w.push_u32(m.reactions);
}

pub(super) fn decode_chat_msg(r: &mut BinReader<'_>) -> Option<ChatMessage> {
    Some(ChatMessage {
        id: r.read_u32()?,
        author: r.read_str()?,
        badge_bg: r.read_str()?,
        badge_fg: r.read_str()?,
        timestamp: r.read_str()?,
        body: r.read_str()?,
        reactions: r.read_u32()?,
    })
}

pub(super) fn encode_islands(w: &mut BinWriter, isl: &IslandsSnapshot) {
    w.push_u32(isl.root_surface_id);
    w.push_u32(isl.rail_surface_id);
    w.push_u32(isl.active_surface_count as u32);
    w.push_u32(isl.cards.len() as u32);
    for c in &isl.cards {
        w.push_str(&c.slug);
        w.push_u8(match c.host {
            IslandHostKind::ReactNative => 0,
            IslandHostKind::Flutter => 1,
            IslandHostKind::SwiftUI => 2,
            IslandHostKind::JetpackCompose => 3,
        });
        w.push_u32(c.surface_id);
        w.push_str(&c.component_name);
        w.push_str(&c.mount_policy);
        w.push_str(&c.lifecycle);
        w.push_u32(c.received_messages);
    }
    w.push_u32(isl.bus_broadcast_count);
    w.push_u32(isl.bus_total_deliveries);
    w.push_str(&isl.last_bus_summary);
    w.push_str(&isl.last_gesture_disposition);
    match isl.last_gesture_winner {
        Some(sid) => {
            w.push_u8(1);
            w.push_u32(sid);
        }
        None => w.push_u8(0),
    }
    w.push_str(&isl.last_gesture_reason);
    w.push_u32(isl.last_gesture_cancelled_count as u32);
}

pub(super) fn decode_islands(r: &mut BinReader<'_>) -> Option<IslandsSnapshot> {
    let root_surface_id = r.read_u32()?;
    let rail_surface_id = r.read_u32()?;
    let active_surface_count = r.read_u32()? as usize;
    let card_len = r.read_count()?;
    let mut cards = Vec::with_capacity(card_len.min(64));
    for _ in 0..card_len {
        let slug = r.read_str()?;
        let host = match r.read_u8()? {
            0 => IslandHostKind::ReactNative,
            1 => IslandHostKind::Flutter,
            2 => IslandHostKind::SwiftUI,
            3 => IslandHostKind::JetpackCompose,
            _ => return None,
        };
        cards.push(IslandCardState {
            slug,
            host,
            surface_id: r.read_u32()?,
            component_name: r.read_str()?,
            mount_policy: r.read_str()?,
            lifecycle: r.read_str()?,
            received_messages: r.read_u32()?,
        });
    }
    let bus_broadcast_count = r.read_u32()?;
    let bus_total_deliveries = r.read_u32()?;
    let last_bus_summary = r.read_str()?;
    let last_gesture_disposition = r.read_str()?;
    let last_gesture_winner = match r.read_u8()? {
        0 => None,
        1 => Some(r.read_u32()?),
        _ => return None,
    };
    let last_gesture_reason = r.read_str()?;
    let last_gesture_cancelled_count = r.read_u32()? as usize;
    Some(IslandsSnapshot {
        root_surface_id,
        rail_surface_id,
        active_surface_count,
        cards,
        bus_broadcast_count,
        bus_total_deliveries,
        last_bus_summary,
        last_gesture_disposition,
        last_gesture_winner,
        last_gesture_reason,
        last_gesture_cancelled_count,
    })
}

pub(super) fn encode_doc_block_kind(w: &mut BinWriter, kind: DocBlockKind) {
    match kind {
        DocBlockKind::Heading1 => w.push_u8(0),
        DocBlockKind::Heading2 => w.push_u8(1),
        DocBlockKind::Paragraph => w.push_u8(2),
        DocBlockKind::Checklist { checked: false } => w.push_u8(3),
        DocBlockKind::Checklist { checked: true } => w.push_u8(4),
        DocBlockKind::Code => w.push_u8(5),
        DocBlockKind::Callout => w.push_u8(6),
    }
}

pub(super) fn decode_doc_block_kind(r: &mut BinReader<'_>) -> Option<DocBlockKind> {
    match r.read_u8()? {
        0 => Some(DocBlockKind::Heading1),
        1 => Some(DocBlockKind::Heading2),
        2 => Some(DocBlockKind::Paragraph),
        3 => Some(DocBlockKind::Checklist { checked: false }),
        4 => Some(DocBlockKind::Checklist { checked: true }),
        5 => Some(DocBlockKind::Code),
        6 => Some(DocBlockKind::Callout),
        _ => None,
    }
}

pub(super) fn encode_doc(w: &mut BinWriter, doc: &WorkspaceDoc) {
    w.push_u32(doc.id);
    w.push_str(&doc.slug);
    w.push_str(&doc.title);
    w.push_str(&doc.updated_at);
    w.push_u32(doc.blocks.len() as u32);
    for b in &doc.blocks {
        w.push_u32(b.id);
        encode_doc_block_kind(w, b.kind);
        w.push_str(&b.text);
    }
}

pub(super) fn decode_doc(r: &mut BinReader<'_>) -> Option<WorkspaceDoc> {
    let id = r.read_u32()?;
    let slug = r.read_str()?;
    let title = r.read_str()?;
    let updated_at = r.read_str()?;
    let block_len = r.read_count()?;
    let mut blocks = Vec::with_capacity(block_len.min(128));
    for _ in 0..block_len {
        blocks.push(DocBlock {
            id: r.read_u32()?,
            kind: decode_doc_block_kind(r)?,
            text: r.read_str()?,
        });
    }
    Some(WorkspaceDoc {
        id,
        slug,
        title,
        updated_at,
        blocks,
    })
}

pub(super) fn encode_workbench(w: &mut BinWriter, wb: &WorkbenchState) {
    w.push_u8(match wb.preset {
        WorkbenchPreset::All => 0,
        WorkbenchPreset::Desktop => 1,
        WorkbenchPreset::Tablet => 2,
        WorkbenchPreset::Mobile => 3,
    });
    w.push_bool(wb.show_device_chrome);
    w.push_str(&wb.selected_host_slug);
}

pub(super) fn decode_workbench(r: &mut BinReader<'_>) -> Option<WorkbenchState> {
    let preset = match r.read_u8()? {
        0 => WorkbenchPreset::All,
        1 => WorkbenchPreset::Desktop,
        2 => WorkbenchPreset::Tablet,
        3 => WorkbenchPreset::Mobile,
        _ => return None,
    };
    let show_device_chrome = r.read_bool()?;
    let selected_host_slug = r.read_str()?;
    Some(WorkbenchState {
        preset,
        show_device_chrome,
        selected_host_slug,
    })
}
