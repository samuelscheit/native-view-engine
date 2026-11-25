//! Deterministic state machine, workspace domain model, and bounded time-travel history
//! for **Aether Workspace** + **NVE Studio**.

use nve_engine::BuiltinLayout;
use serde::{Deserialize, Serialize};

/// Maximum number of retained deterministic snapshots in the time-travel ring.
pub const MAX_HISTORY: usize = 64;

/// Maximum stress rows rendered in the Metrics view to keep frame sizes snappy.
pub const MAX_STRESS_ROWS: usize = 96;

/// Responsive breakpoint (in logical points) below which the desktop 3-column layout
/// collapses into a single-column mobile portrait layout with collapsible drawers.
pub const MOBILE_BREAKPOINT_PT: f32 = 620.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveView {
    Board,
    ChannelEngineering,
    ChannelDesign,
    Stream100k,
    Metrics,
}

impl ActiveView {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Board => "Issue Board",
            Self::ChannelEngineering => "# engineering",
            Self::ChannelDesign => "# design-sync",
            Self::Stream100k => "100k Stream",
            Self::Metrics => "Live Metrics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardFilter {
    All,
    Active,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    P0,
    P1,
    P2,
}

impl Priority {
    pub const fn label(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
        }
    }

    pub const fn badge_bg(self) -> &'static str {
        match self {
            Self::P0 => "#450a0a",
            Self::P1 => "#451a03",
            Self::P2 => "#0c4a6e",
        }
    }

    pub const fn badge_fg(self) -> &'static str {
        match self {
            Self::P0 => "#fca5a5",
            Self::P1 => "#fcd34d",
            Self::P2 => "#7dd3fc",
        }
    }

    pub const fn badge_border(self) -> &'static str {
        match self {
            Self::P0 => "#ef4444",
            Self::P1 => "#f59e0b",
            Self::P2 => "#38bdf8",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
}

impl TaskStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Todo => "Todo",
            Self::InProgress => "In Progress",
            Self::Done => "Done",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Todo => Self::InProgress,
            Self::InProgress => Self::Done,
            Self::Done => Self::Todo,
        }
    }

    pub const fn pill_bg(self) -> &'static str {
        match self {
            Self::Todo => "#1e293b",
            Self::InProgress => "#1e1b4b",
            Self::Done => "#052e16",
        }
    }

    pub const fn pill_fg(self) -> &'static str {
        match self {
            Self::Todo => "#cbd5e1",
            Self::InProgress => "#c4b5fd",
            Self::Done => "#86efac",
        }
    }

    pub const fn pill_border(self) -> &'static str {
        match self {
            Self::Todo => "#475569",
            Self::InProgress => "#8b5cf6",
            Self::Done => "#22c55e",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: u32,
    pub key: String,
    pub title: String,
    pub assignee: String,
    pub tag: String,
    pub priority: Priority,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    Engineering,
    Design,
}

impl ChannelKind {
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Engineering => "#engineering",
            Self::Design => "#design-sync",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: u32,
    pub author: String,
    pub badge_bg: String,
    pub badge_fg: String,
    pub timestamp: String,
    pub body: String,
    pub reactions: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccentTheme {
    Cyan,
    Violet,
    Green,
    Amber,
}

impl AccentTheme {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cyan => "Cyan",
            Self::Violet => "Violet",
            Self::Green => "Green",
            Self::Amber => "Amber",
        }
    }

    pub const fn hex(self) -> &'static str {
        match self {
            Self::Cyan => "#38bdf8",
            Self::Violet => "#a78bfa",
            Self::Green => "#22c55e",
            Self::Amber => "#f59e0b",
        }
    }

    /// Subtle dark tinted fill for active sidebar tabs and badges.
    pub const fn tint_bg(self) -> &'static str {
        match self {
            Self::Cyan => "#0c2d42",
            Self::Violet => "#241942",
            Self::Green => "#0c2e1f",
            Self::Amber => "#36220a",
        }
    }

    /// High-contrast dark ink rendered atop solid accent buttons.
    pub const fn on_accent_fg(self) -> &'static str {
        "#0b1120"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CornerRadiusMode {
    Sharp,
    Soft,
    Pill,
}

impl CornerRadiusMode {
    pub const fn px(self) -> u32 {
        match self {
            Self::Sharp => 0,
            Self::Soft => 12,
            Self::Pill => 18,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Sharp => "Sharp",
            Self::Soft => "Soft",
            Self::Pill => "Pill",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DensityMode {
    Normal,
    Compact,
}

impl DensityMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Compact => "Compact",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LayoutEngineMode {
    #[default]
    Taffy,
    Yoga,
}

impl LayoutEngineMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Taffy => "Taffy",
            Self::Yoga => "Yoga",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Taffy => "taffy",
            Self::Yoga => "yoga",
        }
    }

    pub const fn readout_label(self) -> &'static str {
        match self {
            Self::Taffy => "Active: Taffy (Rust)",
            Self::Yoga => "Active: Yoga (C++20)",
        }
    }

    pub const fn to_builtin(self) -> BuiltinLayout {
        match self {
            Self::Taffy => BuiltinLayout::Taffy,
            Self::Yoga => BuiltinLayout::Yoga,
        }
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::Taffy => Self::Yoga,
            Self::Yoga => Self::Taffy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stream100kState {
    pub scroll_offset_y: u32,
    pub viewport_height: u32,
    pub overscan: usize,
    pub variable_heights: bool,
    pub expanded_rows: Vec<usize>,
}

impl Default for Stream100kState {
    fn default() -> Self {
        Self {
            scroll_offset_y: 0,
            viewport_height: 360,
            overscan: 2,
            variable_heights: true,
            expanded_rows: Vec::new(),
        }
    }
}

fn default_viewport_width() -> u32 {
    960
}

fn default_viewport_height() -> u32 {
    640
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioConfig {
    pub panel_open: bool,
    pub accent: AccentTheme,
    pub radius: CornerRadiusMode,
    pub density: DensityMode,
    pub xray: bool,
    #[serde(default)]
    pub layout_engine: LayoutEngineMode,
}

impl Default for StudioConfig {
    fn default() -> Self {
        Self {
            panel_open: true,
            accent: AccentTheme::Cyan,
            radius: CornerRadiusMode::Soft,
            density: DensityMode::Normal,
            xray: false,
            layout_engine: LayoutEngineMode::Taffy,
        }
    }
}

/// Complete serializable snapshot of the workspace & live studio configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub active_view: ActiveView,
    pub board_filter: BoardFilter,
    pub tasks: Vec<TaskItem>,
    pub next_task_id: u32,
    pub eng_messages: Vec<ChatMessage>,
    pub design_messages: Vec<ChatMessage>,
    pub next_msg_id: u32,
    pub stress_rows: usize,
    #[serde(default)]
    pub stream: Stream100kState,
    pub studio: StudioConfig,
    pub action_label: String,
    #[serde(default = "default_viewport_width")]
    pub viewport_width: u32,
    #[serde(default = "default_viewport_height")]
    pub viewport_height: u32,
    #[serde(default)]
    pub mobile_nav_open: bool,
    #[serde(default)]
    pub mobile_studio_open: bool,
}

impl Default for WorkspaceSnapshot {
    fn default() -> Self {
        Self {
            active_view: ActiveView::Board,
            board_filter: BoardFilter::All,
            tasks: vec![
                TaskItem {
                    id: 1,
                    key: "AET-101".into(),
                    title: "Zero-copy EventRing ABI".into(),
                    assignee: "@maya".into(),
                    tag: "ABI".into(),
                    priority: Priority::P0,
                    status: TaskStatus::InProgress,
                },
                TaskItem {
                    id: 2,
                    key: "AET-102".into(),
                    title: "Qt 6 + AppKit scroll view".into(),
                    assignee: "@liam".into(),
                    tag: "Hosts".into(),
                    priority: Priority::P1,
                    status: TaskStatus::Todo,
                },
                TaskItem {
                    id: 3,
                    key: "AET-103".into(),
                    title: "Tier-2 cdylib state swap".into(),
                    assignee: "@sara".into(),
                    tag: "HotSwap".into(),
                    priority: Priority::P2,
                    status: TaskStatus::Done,
                },
                TaskItem {
                    id: 4,
                    key: "AET-104".into(),
                    title: "Sub-ms Taffy layout diff".into(),
                    assignee: "@dev".into(),
                    tag: "Layout".into(),
                    priority: Priority::P1,
                    status: TaskStatus::Todo,
                },
            ],
            next_task_id: 5,
            eng_messages: vec![
                ChatMessage {
                    id: 1,
                    author: "maya".into(),
                    badge_bg: "#0c4a6e".into(),
                    badge_fg: "#7dd3fc".into(),
                    timestamp: "09:41".into(),
                    body: "Single ABI drives all 4 hosts!".into(),
                    reactions: 4,
                },
                ChatMessage {
                    id: 2,
                    author: "liam".into(),
                    badge_bg: "#2e1065".into(),
                    badge_fg: "#c4b5fd".into(),
                    timestamp: "09:44".into(),
                    body: "p99 diff holds 0.38ms @ 1k.".into(),
                    reactions: 3,
                },
                ChatMessage {
                    id: 3,
                    author: "sara".into(),
                    badge_bg: "#052e16".into(),
                    badge_fg: "#86efac".into(),
                    timestamp: "09:49".into(),
                    body: "Tier-2 reload kept snapshots.".into(),
                    reactions: 2,
                },
            ],
            design_messages: vec![
                ChatMessage {
                    id: 101,
                    author: "aria".into(),
                    badge_bg: "#451a03".into(),
                    badge_fg: "#fcd34d".into(),
                    timestamp: "10:05".into(),
                    body: "Dark slate contrast verified.".into(),
                    reactions: 5,
                },
                ChatMessage {
                    id: 102,
                    author: "noah".into(),
                    badge_bg: "#0c4a6e".into(),
                    badge_fg: "#7dd3fc".into(),
                    timestamp: "10:12".into(),
                    body: "Try X-Ray HUD + Pill radius!".into(),
                    reactions: 3,
                },
            ],
            next_msg_id: 200,
            stress_rows: 4,
            stream: Stream100kState::default(),
            studio: StudioConfig::default(),
            action_label: "Workspace Boot".into(),
            viewport_width: default_viewport_width(),
            viewport_height: default_viewport_height(),
            mobile_nav_open: false,
            mobile_studio_open: false,
        }
    }
}

/// Top-level runtime state persisted across Tier-2 `cdylib` hot-swaps and time-travel scrubs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub current: WorkspaceSnapshot,
    pub history: Vec<WorkspaceSnapshot>,
    pub cursor: usize,
    pub frame_count: u64,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        let initial = WorkspaceSnapshot::default();
        Self {
            history: vec![initial.clone()],
            current: initial,
            cursor: 0,
            frame_count: 1,
        }
    }
}

const NEW_TASK_TEMPLATES: &[(&str, &str, &str, Priority)] = &[
    ("Local-first CRDT sync", "@maya", "Sync", Priority::P0),
    ("WASM bundle size audit", "@liam", "Web", Priority::P1),
    ("Keyboard nav focus ring", "@aria", "UX", Priority::P2),
    ("Perfetto trace export", "@sara", "Perf", Priority::P1),
    ("Yoga vs Taffy parity", "@dev", "Layout", Priority::P2),
];

impl WorkspaceState {
    /// Commits a deterministic state transition onto the bounded time-travel history stack.
    fn commit_action<F>(&mut self, label: impl Into<String>, mutate: F)
    where
        F: FnOnce(&mut WorkspaceSnapshot),
    {
        // Truncate any speculative redo future if we branched from an older cursor.
        if self.cursor + 1 < self.history.len() {
            self.history.truncate(self.cursor + 1);
        }
        mutate(&mut self.current);
        self.current.action_label = label.into();
        self.history.push(self.current.clone());
        if self.history.len() > MAX_HISTORY {
            let overflow = self.history.len() - MAX_HISTORY;
            self.history.drain(0..overflow);
        }
        self.cursor = self.history.len().saturating_sub(1);
        self.frame_count = self.frame_count.saturating_add(1);
    }

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

    pub fn scrub_first(&mut self) {
        if self.cursor == 0 || self.history.is_empty() {
            return;
        }
        self.cursor = 0;
        self.current = self.history[0].clone();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn scrub_undo(&mut self) {
        if self.cursor == 0 || self.history.is_empty() {
            return;
        }
        self.cursor -= 1;
        self.current = self.history[self.cursor].clone();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn scrub_redo(&mut self) {
        if self.cursor + 1 >= self.history.len() {
            return;
        }
        self.cursor += 1;
        self.current = self.history[self.cursor].clone();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn scrub_live(&mut self) {
        let last = self.history.len().saturating_sub(1);
        if self.cursor == last || self.history.is_empty() {
            return;
        }
        self.cursor = last;
        self.current = self.history[last].clone();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.history.len()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn snapshot_index_1based(&self) -> usize {
        self.cursor + 1
    }

    pub fn active_task_count(&self) -> usize {
        self.current
            .tasks
            .iter()
            .filter(|t| t.status != TaskStatus::Done)
            .count()
    }

    pub fn done_task_count(&self) -> usize {
        self.current
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Done)
            .count()
    }

    pub fn total_message_count(&self) -> usize {
        self.current.eng_messages.len() + self.current.design_messages.len()
    }

    pub fn filtered_tasks(&self) -> Vec<TaskItem> {
        self.current
            .tasks
            .iter()
            .filter(|t| match self.current.board_filter {
                BoardFilter::All => true,
                BoardFilter::Active => t.status != TaskStatus::Done,
                BoardFilter::Done => t.status == TaskStatus::Done,
            })
            .cloned()
            .collect()
    }

    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Option<Self> {
        let mut parsed: Self = serde_json::from_slice(bytes).ok()?;
        if parsed.history.is_empty() {
            parsed.history.push(parsed.current.clone());
            parsed.cursor = 0;
        } else if parsed.cursor >= parsed.history.len() {
            parsed.cursor = parsed.history.len() - 1;
        }
        Some(parsed)
    }
}
