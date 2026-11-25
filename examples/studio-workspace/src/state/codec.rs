//! Compact zero-JSON little-endian binary codec (`NWS1`) for [`WorkspaceState`]
//! snapshot/restore across Tier-2 `cdylib` hot-swaps and mmap snapshots.

use crate::ui_islands::{IslandCardState, IslandHostKind, IslandsSnapshot};
use crate::ui_palette::CommandPaletteState;

use super::models::{
    AccentTheme, ActiveView, BoardFilter, ChatMessage, CornerRadiusMode, DensityMode,
    LayoutEngineMode, Priority, Stream100kState, StudioConfig, SyncTelemetryState,
    SyncTransportMode, TaskItem, TaskStatus, WorkspaceSnapshot,
};
use super::WorkspaceState;

/// 4-byte ASCII magic identifying a binary `WorkspaceState` snapshot (`NWS1`).
pub const WORKSPACE_STATE_BIN_MAGIC: [u8; 4] = *b"NWS1";

/// Current binary schema version for `NWS1`.
pub const WORKSPACE_STATE_BIN_VERSION: u16 = 1;

const MAX_ITEMS: usize = 16_384;

struct BinWriter {
    buf: Vec<u8>,
}

impl BinWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    fn push_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn push_bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }

    fn push_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn push_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn push_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn push_str(&mut self, s: &str) {
        let b = s.as_bytes();
        self.push_u32(b.len() as u32);
        self.buf.extend_from_slice(b);
    }
}

struct BinReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> BinReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.bytes.len().saturating_sub(self.pos) < n {
            return None;
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }

    fn read_u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn read_bool(&mut self) -> Option<bool> {
        match self.read_u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn read_u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn read_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn read_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn read_count(&mut self) -> Option<usize> {
        let c = self.read_u32()? as usize;
        (c <= MAX_ITEMS).then_some(c)
    }

    fn read_str(&mut self) -> Option<String> {
        let len = self.read_u32()? as usize;
        let slice = self.take(len)?;
        std::str::from_utf8(slice).ok().map(str::to_owned)
    }

    fn finish(self) -> Option<()> {
        (self.pos == self.bytes.len()).then_some(())
    }
}

fn encode_task(w: &mut BinWriter, t: &TaskItem) {
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

fn decode_task(r: &mut BinReader<'_>) -> Option<TaskItem> {
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

fn encode_chat_msg(w: &mut BinWriter, m: &ChatMessage) {
    w.push_u32(m.id);
    w.push_str(&m.author);
    w.push_str(&m.badge_bg);
    w.push_str(&m.badge_fg);
    w.push_str(&m.timestamp);
    w.push_str(&m.body);
    w.push_u32(m.reactions);
}

fn decode_chat_msg(r: &mut BinReader<'_>) -> Option<ChatMessage> {
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

fn encode_islands(w: &mut BinWriter, isl: &IslandsSnapshot) {
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

fn decode_islands(r: &mut BinReader<'_>) -> Option<IslandsSnapshot> {
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
        let surface_id = r.read_u32()?;
        let component_name = r.read_str()?;
        let mount_policy = r.read_str()?;
        let lifecycle = r.read_str()?;
        let received_messages = r.read_u32()?;
        cards.push(IslandCardState {
            slug,
            host,
            surface_id,
            component_name,
            mount_policy,
            lifecycle,
            received_messages,
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

fn encode_snapshot(w: &mut BinWriter, s: &WorkspaceSnapshot) {
    w.push_u8(match s.active_view {
        ActiveView::Board => 0,
        ActiveView::ChannelEngineering => 1,
        ActiveView::ChannelDesign => 2,
        ActiveView::Stream100k => 3,
        ActiveView::Islands => 4,
        ActiveView::Metrics => 5,
    });
    w.push_u8(match s.board_filter {
        BoardFilter::All => 0,
        BoardFilter::Active => 1,
        BoardFilter::Done => 2,
    });
    w.push_str(&s.board_search_query);
    w.push_str(&s.new_task_draft);

    w.push_u32(s.tasks.len() as u32);
    for t in &s.tasks {
        encode_task(w, t);
    }
    w.push_u32(s.next_task_id);

    w.push_u32(s.eng_messages.len() as u32);
    for m in &s.eng_messages {
        encode_chat_msg(w, m);
    }
    w.push_u32(s.design_messages.len() as u32);
    for m in &s.design_messages {
        encode_chat_msg(w, m);
    }
    w.push_str(&s.eng_draft);
    w.push_str(&s.design_draft);
    w.push_u32(s.next_msg_id);
    w.push_u32(s.stress_rows as u32);

    w.push_u32(s.telemetry_samples_us.len() as u32);
    for &sample in &s.telemetry_samples_us {
        w.push_u32(sample);
    }

    w.push_u32(s.stream.scroll_offset_y);
    w.push_u32(s.stream.viewport_height);
    w.push_u32(s.stream.overscan as u32);
    w.push_bool(s.stream.variable_heights);
    w.push_u32(s.stream.expanded_rows.len() as u32);
    for &row in &s.stream.expanded_rows {
        w.push_u32(row as u32);
    }

    encode_islands(w, &s.islands);

    w.push_bool(s.studio.panel_open);
    w.push_u8(match s.studio.accent {
        AccentTheme::Cyan => 0,
        AccentTheme::Violet => 1,
        AccentTheme::Green => 2,
        AccentTheme::Amber => 3,
    });
    w.push_u8(match s.studio.radius {
        CornerRadiusMode::Sharp => 0,
        CornerRadiusMode::Soft => 1,
        CornerRadiusMode::Pill => 2,
    });
    w.push_u8(match s.studio.density {
        DensityMode::Normal => 0,
        DensityMode::Compact => 1,
    });
    w.push_bool(s.studio.xray);
    w.push_u8(match s.studio.layout_engine {
        LayoutEngineMode::Taffy => 0,
        LayoutEngineMode::Yoga => 1,
    });

    w.push_str(&s.action_label);
    w.push_u32(s.viewport_width);
    w.push_u32(s.viewport_height);
    w.push_bool(s.mobile_nav_open);
    w.push_bool(s.mobile_studio_open);
}

fn decode_snapshot(r: &mut BinReader<'_>) -> Option<WorkspaceSnapshot> {
    let active_view = match r.read_u8()? {
        0 => ActiveView::Board,
        1 => ActiveView::ChannelEngineering,
        2 => ActiveView::ChannelDesign,
        3 => ActiveView::Stream100k,
        4 => ActiveView::Islands,
        5 => ActiveView::Metrics,
        _ => return None,
    };
    let board_filter = match r.read_u8()? {
        0 => BoardFilter::All,
        1 => BoardFilter::Active,
        2 => BoardFilter::Done,
        _ => return None,
    };
    let board_search_query = r.read_str()?;
    let new_task_draft = r.read_str()?;

    let task_len = r.read_count()?;
    let mut tasks = Vec::with_capacity(task_len.min(256));
    for _ in 0..task_len {
        tasks.push(decode_task(r)?);
    }
    let next_task_id = r.read_u32()?;

    let eng_len = r.read_count()?;
    let mut eng_messages = Vec::with_capacity(eng_len.min(256));
    for _ in 0..eng_len {
        eng_messages.push(decode_chat_msg(r)?);
    }
    let des_len = r.read_count()?;
    let mut design_messages = Vec::with_capacity(des_len.min(256));
    for _ in 0..des_len {
        design_messages.push(decode_chat_msg(r)?);
    }
    let eng_draft = r.read_str()?;
    let design_draft = r.read_str()?;
    let next_msg_id = r.read_u32()?;
    let stress_rows = r.read_u32()? as usize;

    let sample_len = r.read_count()?;
    let mut telemetry_samples_us = Vec::with_capacity(sample_len.min(256));
    for _ in 0..sample_len {
        telemetry_samples_us.push(r.read_u32()?);
    }

    let scroll_offset_y = r.read_u32()?;
    let stream_vp_h = r.read_u32()?;
    let overscan = r.read_u32()? as usize;
    let variable_heights = r.read_bool()?;
    let exp_len = r.read_count()?;
    let mut expanded_rows = Vec::with_capacity(exp_len.min(256));
    for _ in 0..exp_len {
        expanded_rows.push(r.read_u32()? as usize);
    }
    let stream = Stream100kState {
        scroll_offset_y,
        viewport_height: stream_vp_h,
        overscan,
        variable_heights,
        expanded_rows,
    };

    let islands = decode_islands(r)?;

    let panel_open = r.read_bool()?;
    let accent = match r.read_u8()? {
        0 => AccentTheme::Cyan,
        1 => AccentTheme::Violet,
        2 => AccentTheme::Green,
        3 => AccentTheme::Amber,
        _ => return None,
    };
    let radius = match r.read_u8()? {
        0 => CornerRadiusMode::Sharp,
        1 => CornerRadiusMode::Soft,
        2 => CornerRadiusMode::Pill,
        _ => return None,
    };
    let density = match r.read_u8()? {
        0 => DensityMode::Normal,
        1 => DensityMode::Compact,
        _ => return None,
    };
    let xray = r.read_bool()?;
    let layout_engine = match r.read_u8()? {
        0 => LayoutEngineMode::Taffy,
        1 => LayoutEngineMode::Yoga,
        _ => return None,
    };
    let studio = StudioConfig {
        panel_open,
        accent,
        radius,
        density,
        xray,
        layout_engine,
    };

    let action_label = r.read_str()?;
    let viewport_width = r.read_u32()?;
    let viewport_height = r.read_u32()?;
    let mobile_nav_open = r.read_bool()?;
    let mobile_studio_open = r.read_bool()?;

    Some(WorkspaceSnapshot {
        active_view,
        board_filter,
        board_search_query,
        new_task_draft,
        tasks,
        next_task_id,
        eng_messages,
        design_messages,
        eng_draft,
        design_draft,
        next_msg_id,
        stress_rows,
        telemetry_samples_us,
        stream,
        islands,
        studio,
        action_label,
        viewport_width,
        viewport_height,
        mobile_nav_open,
        mobile_studio_open,
    })
}

impl WorkspaceState {
    /// Encodes the full workspace state into compact zero-JSON `NWS1` binary format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = BinWriter::with_capacity(1024 + self.history.len() * 512);
        w.buf.extend_from_slice(&WORKSPACE_STATE_BIN_MAGIC);
        w.push_u16(WORKSPACE_STATE_BIN_VERSION);
        encode_snapshot(&mut w, &self.current);

        w.push_u32(self.history.len() as u32);
        for snap in &self.history {
            encode_snapshot(&mut w, snap);
        }
        w.push_u32(self.cursor as u32);
        w.push_u64(self.frame_count);

        w.push_bool(self.palette.open);
        w.push_str(&self.palette.query);
        w.push_u32(self.palette.selected as u32);

        w.push_u8(match self.sync_telemetry.transport {
            SyncTransportMode::LanQuic => 0,
            SyncTransportMode::UsbMux => 1,
            SyncTransportMode::RelayWss => 2,
            SyncTransportMode::Offline => 3,
        });
        w.push_u16(self.sync_telemetry.connected_peers);
        w.push_u32(self.sync_telemetry.kbps_tenths);
        w.push_u16(self.sync_telemetry.compression_tenths);
        w.push_u32(self.sync_telemetry.rtt_ms_tenths);

        w.buf
    }

    /// Decodes a compact binary `NWS1` snapshot back into [`WorkspaceState`].
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut r = BinReader::new(bytes);
        if r.take(4)? != WORKSPACE_STATE_BIN_MAGIC {
            return None;
        }
        if r.read_u16()? != WORKSPACE_STATE_BIN_VERSION {
            return None;
        }

        let current = decode_snapshot(&mut r)?;
        let hist_len = r.read_count()?;
        let mut history = Vec::with_capacity(hist_len.min(128));
        for _ in 0..hist_len {
            history.push(decode_snapshot(&mut r)?);
        }
        let mut cursor = r.read_u32()? as usize;
        let frame_count = r.read_u64()?;

        let palette = CommandPaletteState {
            open: r.read_bool()?,
            query: r.read_str()?,
            selected: r.read_u32()? as usize,
        };

        let transport = match r.read_u8()? {
            0 => SyncTransportMode::LanQuic,
            1 => SyncTransportMode::UsbMux,
            2 => SyncTransportMode::RelayWss,
            3 => SyncTransportMode::Offline,
            _ => return None,
        };
        let sync_telemetry = SyncTelemetryState {
            transport,
            connected_peers: r.read_u16()?,
            kbps_tenths: r.read_u32()?,
            compression_tenths: r.read_u16()?,
            rtt_ms_tenths: r.read_u32()?,
        };

        r.finish()?;

        if history.is_empty() {
            history.push(current.clone());
            cursor = 0;
        } else if cursor >= history.len() {
            cursor = history.len() - 1;
        }

        Some(Self {
            current,
            history,
            cursor,
            frame_count,
            palette,
            sync_telemetry,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ChannelKind, CornerRadiusMode, DensityMode};

    #[test]
    fn binary_nws1_roundtrip_and_corrupt_buffer_rejection() {
        let mut state = WorkspaceState::default();
        state.set_accent(AccentTheme::Violet);
        state.set_radius(CornerRadiusMode::Pill);
        state.set_density(DensityMode::Compact);
        state.toggle_xray();
        state.send_quick_reply(ChannelKind::Engineering, "Zero-JSON NWS1 binary snapshot");
        state.open_palette();
        state.set_palette_query("Yoga");
        state.cycle_sync_transport();

        let bytes = state.to_bytes();
        assert!(bytes.starts_with(&WORKSPACE_STATE_BIN_MAGIC));
        let restored = WorkspaceState::from_bytes(&bytes).expect("valid NWS1 roundtrip");
        assert_eq!(restored, state);

        // Truncated prefixes fail cleanly without panicking.
        for cut in [0, 1, 4, 6, 32, bytes.len() - 1] {
            assert!(WorkspaceState::from_bytes(&bytes[..cut]).is_none());
        }
        // Trailing bytes rejected.
        let mut with_tail = bytes.clone();
        with_tail.push(0);
        assert!(WorkspaceState::from_bytes(&with_tail).is_none());
        // JSON or wrong magic rejected.
        assert!(WorkspaceState::from_bytes(b"{}").is_none());
        assert!(WorkspaceState::from_bytes(b"BAD!\x01\x00").is_none());
    }
}
