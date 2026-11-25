//! Compact zero-JSON little-endian binary codec (`NWS1` v2) for [`WorkspaceState`]
//! with trailing XXH64 integrity checksum for Tier-2 hot-swaps, `MmapSnapshot`, and `.nvewal`.

use nve_engine::xxh64;
use nve_router::Routable;
use nve_system_env::{ColorScheme, ContrastLevel, LocaleSnapshot, TextDirection, ThemeSnapshot};

use crate::routes::{StudioRoute, StudioRouter};
use crate::sdk::StudioSystemEnv;
use crate::ui_palette::CommandPaletteState;

use super::codec_io::{
    decode_chat_msg, decode_doc, decode_doc_block_kind, decode_islands, decode_task,
    decode_workbench, encode_chat_msg, encode_doc, encode_doc_block_kind, encode_islands,
    encode_task, encode_workbench, BinReader, BinWriter,
};
use super::models::{
    AccentTheme, ActiveView, BoardFilter, CornerRadiusMode, DensityMode, LayoutEngineMode,
    Stream100kState, StudioConfig, SyncTelemetryState, SyncTransportMode, WorkspaceSnapshot,
};
use super::WorkspaceState;

/// 4-byte ASCII magic identifying a binary `WorkspaceState` snapshot (`NWS1`).
pub const WORKSPACE_STATE_BIN_MAGIC: [u8; 4] = *b"NWS1";

/// Current binary schema version for `NWS1` (v2: Docs + Workbench + trailing FNV-1a64 checksum).
pub const WORKSPACE_STATE_BIN_VERSION: u16 = 2;

const HEADER_LEN: usize = 4 + 2;
const CHECKSUM_LEN: usize = 8;

fn encode_snapshot(w: &mut BinWriter, s: &WorkspaceSnapshot) {
    w.push_u8(match s.active_view {
        ActiveView::Board => 0,
        ActiveView::ChannelEngineering => 1,
        ActiveView::ChannelDesign => 2,
        ActiveView::Docs => 3,
        ActiveView::Workbench => 4,
        ActiveView::Stream100k => 5,
        ActiveView::Islands => 6,
        ActiveView::Metrics => 7,
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

    w.push_u32(s.docs.len() as u32);
    for doc in &s.docs {
        encode_doc(w, doc);
    }
    w.push_u32(s.active_doc_id);
    w.push_u32(s.next_doc_id);
    w.push_u32(s.next_block_id);
    w.push_str(&s.doc_new_block_draft);
    encode_doc_block_kind(w, s.doc_new_block_kind);
    match s.doc_editing_block_id {
        Some(bid) => {
            w.push_u8(1);
            w.push_u32(bid);
        }
        None => w.push_u8(0),
    }
    w.push_str(&s.doc_edit_draft);
    w.push_bool(s.doc_markdown_preview_open);

    encode_workbench(w, &s.workbench);

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
        3 => ActiveView::Docs,
        4 => ActiveView::Workbench,
        5 => ActiveView::Stream100k,
        6 => ActiveView::Islands,
        7 => ActiveView::Metrics,
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

    let doc_len = r.read_count()?;
    let mut docs = Vec::with_capacity(doc_len.min(64));
    for _ in 0..doc_len {
        docs.push(decode_doc(r)?);
    }
    let active_doc_id = r.read_u32()?;
    let next_doc_id = r.read_u32()?;
    let next_block_id = r.read_u32()?;
    let doc_new_block_draft = r.read_str()?;
    let doc_new_block_kind = decode_doc_block_kind(r)?;
    let doc_editing_block_id = match r.read_u8()? {
        0 => None,
        1 => Some(r.read_u32()?),
        _ => return None,
    };
    let doc_edit_draft = r.read_str()?;
    let doc_markdown_preview_open = r.read_bool()?;

    let workbench = decode_workbench(r)?;

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
        docs,
        active_doc_id,
        next_doc_id,
        next_block_id,
        doc_new_block_draft,
        doc_new_block_kind,
        doc_editing_block_id,
        doc_edit_draft,
        doc_markdown_preview_open,
        workbench,
        studio,
        action_label,
        viewport_width,
        viewport_height,
        mobile_nav_open,
        mobile_studio_open,
    })
}

impl WorkspaceState {
    /// Encodes the full workspace state into compact zero-JSON `NWS1` v2 binary format
    /// terminated by an 8-byte little-endian FNV-1a64 integrity checksum.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = BinWriter::with_capacity(2048 + self.history.len() * 768);
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
        w.push_u64(self.sync_telemetry.crdt_deltas_sent);
        w.push_u64(self.sync_telemetry.crdt_deltas_received);
        w.push_u64(self.sync_telemetry.crdt_bytes_synced);
        w.push_bool(self.sync_telemetry.crdt_partitioned);
        w.push_u32(self.sync_telemetry.crdt_pending_remote_ops);

        let route_entries = self.router.history_routes();
        w.push_u32(route_entries.len() as u32);
        for route in &route_entries {
            w.push_str(&route.to_path());
        }
        w.push_u32(self.router.cursor() as u32);

        match self.focused_task_id {
            Some(tid) => {
                w.push_u8(1);
                w.push_u32(tid);
            }
            None => w.push_u8(0),
        }

        let t = &self.system_env.theme;
        w.push_u8(t.color_scheme as u8);
        w.push_u8(t.contrast as u8);
        w.push_bool(t.reduced_motion);
        w.push_bool(t.reduced_transparency);
        w.push_u32(t.accent_rgba);
        w.push_u16(t.font_scale_x1000);
        w.push_u16(t.scale_factor_x100);

        let l = &self.system_env.locale;
        w.push_u8(l.text_direction as u8);
        w.push_bool(l.uses_24h_clock);
        w.push_str(&l.tag);
        w.push_str(&l.language);
        w.push_str(&l.region);
        w.push_str(&l.timezone);

        match &self.last_clipboard_notice {
            Some(notice) => {
                w.push_u8(1);
                w.push_str(notice);
            }
            None => w.push_u8(0),
        }

        let checksum = xxh64(&w.buf, 0);
        w.push_u64(checksum);
        w.buf
    }

    /// Decodes a compact binary `NWS1` v2 snapshot back into [`WorkspaceState`],
    /// strictly verifying the trailing XXH64 checksum and rejecting v1 or corrupt payloads.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN + CHECKSUM_LEN {
            return None;
        }
        let (payload, checksum_tail) = bytes.split_at(bytes.len() - CHECKSUM_LEN);
        let expected_checksum = u64::from_le_bytes(checksum_tail.try_into().ok()?);
        if xxh64(payload, 0) != expected_checksum {
            return None;
        }

        let mut r = BinReader::new(payload);
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
            crdt_deltas_sent: r.read_u64()?,
            crdt_deltas_received: r.read_u64()?,
            crdt_bytes_synced: r.read_u64()?,
            crdt_partitioned: r.read_bool()?,
            crdt_pending_remote_ops: r.read_u32()?,
        };

        let route_count = r.read_count()?;
        let mut route_entries = Vec::with_capacity(route_count.min(64));
        for _ in 0..route_count {
            let path = r.read_str()?;
            route_entries.push(StudioRoute::parse_uri(&path)?);
        }
        let route_cursor = r.read_u32()? as usize;
        let router = StudioRouter::from_history(route_entries, route_cursor);

        let focused_task_id = match r.read_u8()? {
            0 => None,
            1 => Some(r.read_u32()?),
            _ => return None,
        };

        let theme = ThemeSnapshot {
            color_scheme: ColorScheme::from_u8(r.read_u8()?)?,
            contrast: ContrastLevel::from_u8(r.read_u8()?)?,
            reduced_motion: r.read_bool()?,
            reduced_transparency: r.read_bool()?,
            accent_rgba: r.read_u32()?,
            font_scale_x1000: r.read_u16()?,
            scale_factor_x100: r.read_u16()?,
        };
        let locale = LocaleSnapshot {
            text_direction: TextDirection::from_u8(r.read_u8()?)?,
            uses_24h_clock: r.read_bool()?,
            tag: r.read_str()?,
            language: r.read_str()?,
            region: r.read_str()?,
            timezone: r.read_str()?,
        };
        let system_env = StudioSystemEnv { theme, locale };

        let last_clipboard_notice = match r.read_u8()? {
            0 => None,
            1 => Some(r.read_str()?),
            _ => return None,
        };

        r.finish()?;

        if history.is_empty() {
            history.push(current.clone());
            cursor = 0;
        } else if cursor >= history.len() {
            cursor = history.len() - 1;
        }

        let trait_store = Self::rebuild_trait_box(&current, &system_env);

        Some(Self {
            current,
            history,
            cursor,
            frame_count,
            palette,
            sync_telemetry,
            router,
            focused_task_id,
            system_env,
            last_clipboard_notice,
            trait_store,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ChannelKind, DocBlockKind, WorkbenchPreset};

    #[test]
    fn binary_nws1_v2_roundtrip_and_fnv64_corruption_rejection() {
        let mut state = WorkspaceState::default();
        state.set_accent(AccentTheme::Violet);
        state.set_radius(CornerRadiusMode::Pill);
        state.set_density(DensityMode::Compact);
        state.toggle_xray();
        state.send_quick_reply(ChannelKind::Engineering, "Zero-JSON NWS1 v2 snapshot");
        state.toggle_doc_checklist(7);
        state.add_doc_block(DocBlockKind::Callout, "Persisted block callout");
        state.set_workbench_preset(WorkbenchPreset::Tablet);
        state.select_workbench_host("skia-metal");
        state.open_palette();
        state.set_palette_query("Yoga");
        state.cycle_sync_transport();

        let bytes = state.to_bytes();
        assert!(bytes.starts_with(&WORKSPACE_STATE_BIN_MAGIC));
        assert_eq!(
            u16::from_le_bytes([bytes[4], bytes[5]]),
            WORKSPACE_STATE_BIN_VERSION
        );
        let restored = WorkspaceState::from_bytes(&bytes).expect("valid NWS1 v2 roundtrip");
        assert_eq!(restored, state);

        // Single-bit flips anywhere in header, payload, or trailing checksum must fail.
        for pos in [0, 4, 6, bytes.len() / 2, bytes.len() - 9, bytes.len() - 1] {
            let mut flipped = bytes.clone();
            flipped[pos] ^= 0x01;
            assert!(
                WorkspaceState::from_bytes(&flipped).is_none(),
                "bitflip at byte {pos} must be rejected"
            );
        }

        // Truncated prefixes fail cleanly without panicking.
        for cut in [0, 1, 4, 6, 14, 32, bytes.len() - 8, bytes.len() - 1] {
            assert!(WorkspaceState::from_bytes(&bytes[..cut]).is_none());
        }

        // Trailing bytes rejected.
        let mut with_tail = bytes.clone();
        with_tail.push(0);
        assert!(WorkspaceState::from_bytes(&with_tail).is_none());

        // Legacy v1 version rejected (zero backwards compatibility).
        let mut v1_forged = bytes.clone();
        v1_forged[4..6].copy_from_slice(&1u16.to_le_bytes());
        let len = v1_forged.len();
        let forged_crc = xxh64(&v1_forged[..len - 8], 0);
        v1_forged[len - 8..].copy_from_slice(&forged_crc.to_le_bytes());
        assert!(WorkspaceState::from_bytes(&v1_forged).is_none());

        // JSON or wrong magic rejected.
        assert!(WorkspaceState::from_bytes(b"{}").is_none());
        assert!(WorkspaceState::from_bytes(b"BAD!\x02\x0000000000").is_none());
    }
}
