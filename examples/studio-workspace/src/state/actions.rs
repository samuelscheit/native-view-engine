//! State transition actions for navigation, Issue Board, chat channels, 100k Stream,
//! NVE Studio controls, Live Sync telemetry, `Cmd+K` Command Palette, and `<Island>` sandbox.

use crate::ui_islands::{format_disposition, format_lifecycle, GestureSimMode, IslandSandboxRuntime};
use crate::ui_palette::{execute_palette_action, filter_palette_items};

use super::models::{
    AccentTheme, ActiveView, BoardFilter, ChannelKind, ChatMessage, CornerRadiusMode, DensityMode,
    LayoutEngineMode, Priority, SyncTelemetryState, TaskItem, TaskStatus, MAX_STRESS_ROWS,
    MAX_TELEMETRY_SAMPLES, MOBILE_BREAKPOINT_PT,
};
use super::WorkspaceState;

const NEW_TASK_TEMPLATES: &[(&str, &str, &str, Priority)] = &[
    ("Local-first CRDT sync", "@maya", "Sync", Priority::P0),
    ("WASM bundle size audit", "@liam", "Web", Priority::P1),
    ("Keyboard nav focus ring", "@aria", "UX", Priority::P2),
    ("Perfetto trace export", "@sara", "Perf", Priority::P1),
    ("Yoga vs Taffy parity", "@dev", "Layout", Priority::P2),
];

impl WorkspaceState {
    pub fn is_mobile_portrait(&self) -> bool {
        (self.current.viewport_width as f32) < MOBILE_BREAKPOINT_PT
    }

    /// Updates viewport dimensions without creating a spurious time-travel snapshot.
    /// Returns `true` if layout-relevant dimensions changed.
    pub fn set_viewport_size(&mut self, width: f32, height: f32) -> bool {
        let w = if width.is_finite() && width > 0.0 {
            width.round() as u32
        } else {
            return false;
        };
        let h = if height.is_finite() && height > 0.0 {
            height.round() as u32
        } else {
            self.current.viewport_height
        };
        if self.current.viewport_width == w && self.current.viewport_height == h {
            return false;
        }
        let was_mobile = self.is_mobile_portrait();
        self.current.viewport_width = w;
        self.current.viewport_height = h;
        // Keep the 100k Stream viewport height proportional to available center height so
        // the rendered visible + overscan window stays strictly inside the viewport bounds.
        self.current.stream.viewport_height = h.saturating_sub(280).max(180);
        if was_mobile && !self.is_mobile_portrait() {
            self.current.mobile_nav_open = false;
            self.current.mobile_studio_open = false;
        }
        // Keep initial snapshot in sync when resized before first user action.
        if self.history.len() == 1 && self.cursor == 0 {
            self.history[0].viewport_width = w;
            self.history[0].viewport_height = h;
            self.history[0].stream.viewport_height = self.current.stream.viewport_height;
        }
        true
    }

    pub fn select_view(&mut self, view: ActiveView) {
        if self.current.active_view == view {
            if self.current.mobile_nav_open {
                self.current.mobile_nav_open = false;
            }
            return;
        }
        let label = format!("View -> {}", view.title());
        self.commit_action(label, |s| {
            s.active_view = view;
            s.mobile_nav_open = false;
        });
    }

    pub fn toggle_mobile_nav(&mut self) {
        let next = !self.current.mobile_nav_open;
        let label = if next { "Nav -> Open" } else { "Nav -> Close" };
        self.commit_action(label, |s| {
            s.mobile_nav_open = next;
            if next {
                s.mobile_studio_open = false;
            }
        });
    }

    pub fn set_stream_scroll_offset(&mut self, offset_y: u32) {
        if self.current.stream.scroll_offset_y == offset_y {
            return;
        }
        self.current.stream.scroll_offset_y = offset_y;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn jump_stream_offset(&mut self, offset_y: u32, label: impl Into<String>) {
        self.commit_action(label, |s| {
            s.stream.scroll_offset_y = offset_y;
        });
    }

    pub fn toggle_stream_variable_heights(&mut self) {
        let next = !self.current.stream.variable_heights;
        let label = if next {
            "Stream -> Variable H"
        } else {
            "Stream -> Fixed H"
        };
        self.commit_action(label, |s| {
            s.stream.variable_heights = next;
        });
    }

    pub fn toggle_stream_row_expanded(&mut self, row_index: usize) {
        let will_expand = !self.current.stream.expanded_rows.contains(&row_index);
        let label = if will_expand {
            format!("Expand #{row_index}")
        } else {
            format!("Collapse #{row_index}")
        };
        self.commit_action(label, |s| {
            if let Some(pos) = s.stream.expanded_rows.iter().position(|&r| r == row_index) {
                s.stream.expanded_rows.remove(pos);
            } else {
                s.stream.expanded_rows.push(row_index);
                s.stream.expanded_rows.sort_unstable();
            }
        });
    }

    pub fn set_board_filter(&mut self, filter: BoardFilter) {
        if self.current.board_filter == filter {
            return;
        }
        let label = match filter {
            BoardFilter::All => "Filter -> All",
            BoardFilter::Active => "Filter -> Active",
            BoardFilter::Done => "Filter -> Done",
        };
        self.commit_action(label, |s| s.board_filter = filter);
    }

    pub fn cycle_task_status(&mut self, task_id: u32) {
        let Some(next_status) = self
            .current
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .map(|t| t.status.next())
        else {
            return;
        };
        let label = format!("Status -> {}", next_status.label());
        self.commit_action(label, |s| {
            if let Some(task) = s.tasks.iter_mut().find(|t| t.id == task_id) {
                task.status = next_status;
            }
        });
    }

    pub fn create_new_task(&mut self) {
        let id = self.current.next_task_id;
        let idx = ((id as usize).saturating_sub(5)) % NEW_TASK_TEMPLATES.len();
        let (title, assignee, tag, priority) = NEW_TASK_TEMPLATES[idx];
        let key = format!("AET-{}", 100 + id);
        let label = format!("New {key}");
        self.commit_action(label, |s| {
            s.tasks.insert(
                0,
                TaskItem {
                    id,
                    key,
                    title: title.into(),
                    assignee: assignee.into(),
                    tag: tag.into(),
                    priority,
                    status: TaskStatus::Todo,
                },
            );
            s.next_task_id = id.saturating_add(1);
            s.active_view = ActiveView::Board;
            if s.board_filter == BoardFilter::Done {
                s.board_filter = BoardFilter::All;
            }
        });
    }

    pub fn send_quick_reply(&mut self, channel: ChannelKind, body: &str) {
        let msg_id = self.current.next_msg_id;
        let label = format!("Sent {}", channel.tag());
        let body_owned = body.to_owned();
        let minute = 50 + (msg_id % 10);
        self.commit_action(label, |s| {
            let msg = ChatMessage {
                id: msg_id,
                author: "you".into(),
                badge_bg: "#1e293b".into(),
                badge_fg: "#f8fafc".into(),
                timestamp: format!("10:{minute:02}"),
                body: body_owned,
                reactions: 1,
            };
            match channel {
                ChannelKind::Engineering => s.eng_messages.push(msg),
                ChannelKind::Design => s.design_messages.push(msg),
            }
            s.next_msg_id = msg_id.saturating_add(1);
        });
    }

    pub fn react_message(&mut self, channel: ChannelKind, msg_id: u32) {
        let exists = match channel {
            ChannelKind::Engineering => self.current.eng_messages.iter().any(|m| m.id == msg_id),
            ChannelKind::Design => self.current.design_messages.iter().any(|m| m.id == msg_id),
        };
        if !exists {
            return;
        }
        let label = format!("+1 in {}", channel.tag());
        self.commit_action(label, |s| {
            let list = match channel {
                ChannelKind::Engineering => &mut s.eng_messages,
                ChannelKind::Design => &mut s.design_messages,
            };
            if let Some(msg) = list.iter_mut().find(|m| m.id == msg_id) {
                msg.reactions = msg.reactions.saturating_add(1);
            }
        });
    }

    pub fn add_stress_rows(&mut self, delta: usize) {
        let next = (self.current.stress_rows + delta).min(MAX_STRESS_ROWS);
        if next == self.current.stress_rows {
            return;
        }
        let label = format!("Stress -> {next} rows");
        self.commit_action(label, |s| s.stress_rows = next);
    }

    pub fn reset_stress_rows(&mut self) {
        if self.current.stress_rows == 4 {
            return;
        }
        self.commit_action("Stress -> Reset (4)", |s| s.stress_rows = 4);
    }

    pub fn toggle_studio_panel(&mut self) {
        if self.is_mobile_portrait() {
            let next = !self.current.mobile_studio_open;
            let label = if next { "Studio -> ON" } else { "Studio -> OFF" };
            self.commit_action(label, |s| {
                s.mobile_studio_open = next;
                if next {
                    s.mobile_nav_open = false;
                }
            });
        } else {
            let next = !self.current.studio.panel_open;
            let label = if next { "Studio -> ON" } else { "Studio -> OFF" };
            self.commit_action(label, |s| s.studio.panel_open = next);
        }
    }

    pub fn set_layout_engine(&mut self, mode: LayoutEngineMode) {
        if self.current.studio.layout_engine == mode {
            return;
        }
        let label = format!("Layout -> {}", mode.label());
        self.commit_action(label, |s| s.studio.layout_engine = mode);
    }

    pub fn toggle_layout_engine(&mut self) {
        let next = self.current.studio.layout_engine.toggled();
        self.set_layout_engine(next);
    }

    pub fn set_accent(&mut self, accent: AccentTheme) {
        if self.current.studio.accent == accent {
            return;
        }
        let label = format!("Accent -> {}", accent.label());
        self.commit_action(label, |s| s.studio.accent = accent);
    }

    pub fn set_radius(&mut self, radius: CornerRadiusMode) {
        if self.current.studio.radius == radius {
            return;
        }
        let label = format!("Radius -> {} ({})", radius.label(), radius.px());
        self.commit_action(label, |s| s.studio.radius = radius);
    }

    pub fn set_density(&mut self, density: DensityMode) {
        if self.current.studio.density == density {
            return;
        }
        let label = format!("Density -> {}", density.label());
        self.commit_action(label, |s| s.studio.density = density);
    }

    pub fn toggle_xray(&mut self) {
        let next = !self.current.studio.xray;
        let label = if next { "X-Ray -> ON" } else { "X-Ray -> OFF" };
        self.commit_action(label, |s| s.studio.xray = next);
    }

    /// Updates live sync / pairing telemetry metrics without polluting time-travel history.
    pub fn update_sync_telemetry(&mut self, telemetry: SyncTelemetryState) -> bool {
        if self.sync_telemetry == telemetry {
            return false;
        }
        self.sync_telemetry = telemetry;
        self.frame_count = self.frame_count.saturating_add(1);
        true
    }

    /// Cycles the active live sync transport mode (LAN QUIC -> USB Mux -> Relay WSS -> Offline).
    pub fn cycle_sync_transport(&mut self) {
        let next = self.sync_telemetry.transport.next();
        self.sync_telemetry.transport = next;
        match next {
            super::models::SyncTransportMode::LanQuic => {
                self.sync_telemetry.connected_peers = 2;
                self.sync_telemetry.kbps_tenths = 38;
                self.sync_telemetry.compression_tenths = 64;
                self.sync_telemetry.rtt_ms_tenths = 14;
            }
            super::models::SyncTransportMode::UsbMux => {
                self.sync_telemetry.connected_peers = 1;
                self.sync_telemetry.kbps_tenths = 24;
                self.sync_telemetry.compression_tenths = 72;
                self.sync_telemetry.rtt_ms_tenths = 4;
            }
            super::models::SyncTransportMode::RelayWss => {
                self.sync_telemetry.connected_peers = 3;
                self.sync_telemetry.kbps_tenths = 51;
                self.sync_telemetry.compression_tenths = 58;
                self.sync_telemetry.rtt_ms_tenths = 264;
            }
            super::models::SyncTransportMode::Offline => {
                self.sync_telemetry.connected_peers = 0;
                self.sync_telemetry.kbps_tenths = 0;
                self.sync_telemetry.compression_tenths = 10;
                self.sync_telemetry.rtt_ms_tenths = 0;
            }
        }
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn set_board_search_query(&mut self, query: impl Into<String>) {
        let next = query.into();
        if self.current.board_search_query == next {
            return;
        }
        self.current.board_search_query = next;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn set_new_task_draft(&mut self, draft: impl Into<String>) {
        let next = draft.into();
        if self.current.new_task_draft == next {
            return;
        }
        self.current.new_task_draft = next;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn create_custom_task(&mut self, title: &str) -> bool {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return false;
        }
        let id = self.current.next_task_id;
        let key = format!("AET-{}", 100 + id);
        let label = format!("New {key}");
        let owned_title = trimmed.to_owned();
        self.commit_action(label, |s| {
            s.tasks.insert(
                0,
                TaskItem {
                    id,
                    key,
                    title: owned_title,
                    assignee: "@you".into(),
                    tag: "Custom".into(),
                    priority: Priority::P1,
                    status: TaskStatus::Todo,
                },
            );
            s.next_task_id = id.saturating_add(1);
            s.new_task_draft.clear();
            s.active_view = ActiveView::Board;
            if s.board_filter == BoardFilter::Done {
                s.board_filter = BoardFilter::All;
            }
        });
        true
    }

    pub fn submit_new_task_draft(&mut self) -> bool {
        let draft = self.current.new_task_draft.clone();
        self.create_custom_task(&draft)
    }

    pub fn draft_for_channel(&self, channel: ChannelKind) -> &str {
        match channel {
            ChannelKind::Engineering => &self.current.eng_draft,
            ChannelKind::Design => &self.current.design_draft,
        }
    }

    pub fn set_channel_draft(&mut self, channel: ChannelKind, draft: impl Into<String>) {
        let next = draft.into();
        let slot = match channel {
            ChannelKind::Engineering => &mut self.current.eng_draft,
            ChannelKind::Design => &mut self.current.design_draft,
        };
        if *slot == next {
            return;
        }
        *slot = next;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn submit_channel_draft(&mut self, channel: ChannelKind) -> bool {
        let body = self.draft_for_channel(channel).trim().to_owned();
        if body.is_empty() {
            return false;
        }
        let msg_id = self.current.next_msg_id;
        let label = format!("Sent {}", channel.tag());
        let minute = 50 + (msg_id % 10);
        self.commit_action(label, |s| {
            let msg = ChatMessage {
                id: msg_id,
                author: "you".into(),
                badge_bg: "#1e293b".into(),
                badge_fg: "#f8fafc".into(),
                timestamp: format!("10:{minute:02}"),
                body,
                reactions: 1,
            };
            match channel {
                ChannelKind::Engineering => {
                    s.eng_messages.push(msg);
                    s.eng_draft.clear();
                }
                ChannelKind::Design => {
                    s.design_messages.push(msg);
                    s.design_draft.clear();
                }
            }
            s.next_msg_id = msg_id.saturating_add(1);
        });
        true
    }

    pub fn record_telemetry_sample(&mut self, sample_us: u32) {
        self.current.telemetry_samples_us.push(sample_us);
        if self.current.telemetry_samples_us.len() > MAX_TELEMETRY_SAMPLES {
            let extra = self.current.telemetry_samples_us.len() - MAX_TELEMETRY_SAMPLES;
            self.current.telemetry_samples_us.drain(0..extra);
        }
    }

    // ---- Cmd+K Command Palette ----

    pub fn open_palette(&mut self) {
        self.palette.open = true;
        self.palette.selected = 0;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn close_palette(&mut self) {
        if !self.palette.open {
            return;
        }
        self.palette.open = false;
        self.palette.query.clear();
        self.palette.selected = 0;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn toggle_palette(&mut self) {
        if self.palette.open {
            self.close_palette();
        } else {
            self.open_palette();
        }
    }

    pub fn set_palette_query(&mut self, query: impl Into<String>) {
        self.palette.query = query.into();
        self.palette.selected = 0;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn move_palette_selection(&mut self, delta: isize) {
        let len = filter_palette_items(self)
            .len()
            .min(crate::ui_palette::MAX_VISIBLE_PALETTE_ITEMS);
        if len == 0 {
            self.palette.selected = 0;
            return;
        }
        let cur = self.palette.selected.min(len - 1) as isize;
        let next = (cur + delta).rem_euclid(len as isize) as usize;
        self.palette.selected = next;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn execute_palette_at(&mut self, index: usize) -> bool {
        let matches = filter_palette_items(self);
        let Some(item) = matches.get(index) else {
            return false;
        };
        let action = item.action.clone();
        execute_palette_action(self, action);
        true
    }

    pub fn execute_palette_selected(&mut self) -> bool {
        let idx = self.palette.selected;
        self.execute_palette_at(idx)
    }

    // ---- Multi-Host <Island> Sandbox (real nve_interop orchestrator, bus, arbitrator) ----

    pub fn toggle_island_lifecycle(&mut self, slug: &str) {
        let mut rt = IslandSandboxRuntime::from_snapshot(&self.current.islands);
        let Some((sid, next_state)) = rt.toggle_island_lifecycle(slug) else {
            return;
        };
        let state_str = format_lifecycle(next_state);
        let label = format!("Island {slug} -> {state_str}");
        self.commit_action(label, |s| {
            if let Some(card) = s.islands.cards.iter_mut().find(|c| c.surface_id == sid) {
                card.lifecycle = state_str.into();
            }
            s.islands.active_surface_count = rt.orchestrator.active_count();
        });
    }

    pub fn broadcast_island_theme(&mut self) {
        let accent = self.current.studio.accent;
        let mut rt = IslandSandboxRuntime::from_snapshot(&self.current.islands);
        let delivered = rt.broadcast_theme(accent);
        let label = format!("Bus theme -> {delivered} islands");
        self.commit_action(label, |s| {
            s.islands.bus_broadcast_count = s.islands.bus_broadcast_count.saturating_add(1);
            s.islands.bus_total_deliveries = s
                .islands
                .bus_total_deliveries
                .saturating_add(delivered as u32);
            s.islands.last_bus_summary = format!(
                "TX #{} topic='theme' accent={} -> {} islands",
                s.islands.bus_broadcast_count,
                accent.label(),
                delivered
            );
            for card in &mut s.islands.cards {
                let rx = rt.delivery_count_for(card.surface_id) as u32;
                card.received_messages = card.received_messages.saturating_add(rx);
            }
        });
    }

    pub fn broadcast_island_sync(&mut self) {
        let frame_count = self.frame_count;
        let task_count = self.current.tasks.len();
        let mut rt = IslandSandboxRuntime::from_snapshot(&self.current.islands);
        let delivered = rt.broadcast_sync(frame_count, task_count);
        let label = format!("Bus sync -> {delivered} islands");
        self.commit_action(label, |s| {
            s.islands.bus_broadcast_count = s.islands.bus_broadcast_count.saturating_add(1);
            s.islands.bus_total_deliveries = s
                .islands
                .bus_total_deliveries
                .saturating_add(delivered as u32);
            s.islands.last_bus_summary = format!(
                "TX #{} topic='workspace.sync' (12B bin) -> {} islands",
                s.islands.bus_broadcast_count, delivered
            );
            for card in &mut s.islands.cards {
                let rx = rt.delivery_count_for(card.surface_id) as u32;
                card.received_messages = card.received_messages.saturating_add(rx);
            }
        });
    }

    pub fn simulate_island_gesture(&mut self, mode: GestureSimMode) {
        let mut rt = IslandSandboxRuntime::from_snapshot(&self.current.islands);
        let decision = rt.simulate_gesture(mode);
        let disp_str = format_disposition(decision.innermost_disposition);
        let reason_str = format!("{:?}", decision.reason);
        let label = format!("Gesture -> {disp_str}");
        self.commit_action(label, |s| {
            s.islands.last_gesture_disposition = disp_str.into();
            s.islands.last_gesture_winner = decision.winner;
            s.islands.last_gesture_reason = reason_str;
            s.islands.last_gesture_cancelled_count = decision.cancelled_surfaces.len();
        });
    }
}
