//! Domain models, configuration enums, live sync telemetry, and initial seed snapshot
//! for **Aether Workspace** + **NVE Studio**.

use nve_ui::Accent;

use crate::ui_islands::IslandsSnapshot;

use super::docs::{DocBlockKind, WorkspaceDoc, default_seed_docs};
use super::shared::Shared;
use super::workbench::WorkbenchState;

/// Maximum number of retained deterministic snapshots in the time-travel ring.
pub const MAX_HISTORY: usize = 64;

/// Maximum stress rows rendered in the Metrics view to keep frame sizes snappy.
pub const MAX_STRESS_ROWS: usize = 96;

/// Maximum number of telemetry samples plotted by the live `SkiaCanvas` sparkline.
pub const MAX_TELEMETRY_SAMPLES: usize = 24;

/// Responsive breakpoint (in logical points) below which the desktop 3-column layout
/// collapses into a single-column mobile portrait layout with collapsible drawers.
pub const MOBILE_BREAKPOINT_PT: f32 = 620.0;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, nve_abi::ReprEnum)]
pub enum ActiveView {
    Board = 0,
    ChannelEngineering = 1,
    ChannelDesign = 2,
    Docs = 3,
    Workbench = 4,
    Stream100k = 5,
    Islands = 6,
    Metrics = 7,
}

impl ActiveView {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Board => "Issue Board",
            Self::ChannelEngineering => "# engineering",
            Self::ChannelDesign => "# design-sync",
            Self::Docs => "Workspace Docs",
            Self::Workbench => "Meta Workbench",
            Self::Stream100k => "100k Stream",
            Self::Islands => "Multi-Host Islands",
            Self::Metrics => "Live Metrics",
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, nve_abi::ReprEnum)]
pub enum BoardFilter {
    All = 0,
    Active = 1,
    Done = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, nve_abi::ReprEnum)]
pub enum Priority {
    P0 = 0,
    P1 = 1,
    P2 = 2,
}

impl Priority {
    pub const fn label(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, nve_abi::ReprEnum)]
pub enum TaskStatus {
    Todo = 0,
    InProgress = 1,
    Done = 2,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    pub id: u32,
    pub key: String,
    pub title: String,
    pub assignee: String,
    pub tag: String,
    pub priority: Priority,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: u32,
    pub author: String,
    pub timestamp: String,
    pub body: String,
    pub reactions: u32,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, nve_abi::ReprEnum)]
pub enum AccentTheme {
    Cyan = 0,
    Violet = 1,
    Green = 2,
    Amber = 3,
}

impl AccentTheme {
    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cyan => "Cyan",
            Self::Violet => "Violet",
            Self::Green => "Green",
            Self::Amber => "Amber",
        }
    }

    /// The unified-theme accent for this workspace choice (contrast-safe in every polarity).
    pub const fn accent(self) -> Accent {
        match self {
            Self::Cyan => Accent::CYAN,
            Self::Violet => Accent::VIOLET,
            Self::Green => Accent::GREEN,
            Self::Amber => Accent::AMBER,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, nve_abi::ReprEnum)]
pub enum CornerRadiusMode {
    Sharp = 0,
    Soft = 1,
    Pill = 2,
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

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, nve_abi::ReprEnum)]
pub enum DensityMode {
    Normal = 0,
    Compact = 1,
}

impl DensityMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Compact => "Compact",
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, nve_abi::ReprEnum)]
pub enum LayoutEngineMode {
    #[default]
    Taffy = 0,
    Yoga = 1,
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

    pub const fn toggled(self) -> Self {
        match self {
            Self::Taffy => Self::Yoga,
            Self::Yoga => Self::Taffy,
        }
    }
}

/// Active transport backing NVE Live Sync / Pairing sessions.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, nve_abi::ReprEnum)]
pub enum SyncTransportMode {
    #[default]
    LanQuic = 0,
    UsbMux = 1,
    RelayWss = 2,
    Offline = 3,
}

impl SyncTransportMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LanQuic => "LAN QUIC",
            Self::UsbMux => "USB Mux",
            Self::RelayWss => "Relay WSS",
            Self::Offline => "Offline",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LanQuic => "lan-quic",
            Self::UsbMux => "usb-mux",
            Self::RelayWss => "relay-wss",
            Self::Offline => "offline",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        use SyncTransportMode as M;
        const TABLE: &[(&str, M)] = &[
            ("lan", M::LanQuic),
            ("quic", M::LanQuic),
            ("lan-quic", M::LanQuic),
            ("lan_quic", M::LanQuic),
            ("usb", M::UsbMux),
            ("usb-mux", M::UsbMux),
            ("usb_mux", M::UsbMux),
            ("relay", M::RelayWss),
            ("wss", M::RelayWss),
            ("relay-wss", M::RelayWss),
            ("relay_wss", M::RelayWss),
            ("offline", M::Offline),
            ("none", M::Offline),
        ];
        let s = raw.trim();
        TABLE
            .iter()
            .find_map(|&(k, v)| s.eq_ignore_ascii_case(k).then_some(v))
    }

    pub const fn next(self) -> Self {
        match self {
            Self::LanQuic => Self::UsbMux,
            Self::UsbMux => Self::RelayWss,
            Self::RelayWss => Self::Offline,
            Self::Offline => Self::LanQuic,
        }
    }
}

/// Live Sync / Pairing telemetry displayed in the docked NVE Studio inspector card.
///
/// Rates, ratios, and latencies are stored in fixed-point tenths so `WorkspaceState` preserves
/// exact `Eq` determinism across binary snapshot/restore and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTelemetryState {
    pub transport: SyncTransportMode,
    pub connected_peers: u16,
    /// Wire bandwidth in tenths of KB/s (e.g. `38` => `"3.8 KB/s"`).
    pub kbps_tenths: u32,
    /// Delta compression ratio in tenths (e.g. `64` => `"6.4x"`).
    pub compression_tenths: u16,
    /// Smoothed round-trip time in tenths of milliseconds (e.g. `14` => `"1.4ms"`).
    pub rtt_ms_tenths: u32,
    /// Cumulative `NveCrdtDelta` binary CRDT deltas transmitted by this peer.
    pub crdt_deltas_sent: u64,
    /// Cumulative `NveCrdtDelta` binary CRDT deltas received and merged by this peer.
    pub crdt_deltas_received: u64,
    /// Total binary CRDT bytes (`NveCrdtDelta` + `NVE_DGST`) synced across peers.
    pub crdt_bytes_synced: u64,
    /// Whether the interactive CRDT mesh is currently partitioned.
    pub crdt_partitioned: bool,
    /// Pending remote peer edits queued while partitioned for catch-up on heal.
    pub crdt_pending_remote_ops: u32,
}

impl Default for SyncTelemetryState {
    fn default() -> Self {
        Self {
            transport: SyncTransportMode::LanQuic,
            connected_peers: 2,
            kbps_tenths: 38,
            compression_tenths: 64,
            rtt_ms_tenths: 14,
            crdt_deltas_sent: 0,
            crdt_deltas_received: 0,
            crdt_bytes_synced: 0,
            crdt_partitioned: false,
            crdt_pending_remote_ops: 0,
        }
    }
}

impl SyncTelemetryState {
    pub fn format_kbps(&self) -> String {
        format!("{}.{} KB/s", self.kbps_tenths / 10, self.kbps_tenths % 10)
    }

    pub fn format_compression(&self) -> String {
        format!(
            "{}.{}x",
            self.compression_tenths / 10,
            self.compression_tenths % 10
        )
    }

    pub fn format_rtt(&self) -> String {
        format!("{}.{}ms", self.rtt_ms_tenths / 10, self.rtt_ms_tenths % 10)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

pub(crate) fn default_viewport_width() -> u32 {
    960
}

pub(crate) fn default_viewport_height() -> u32 {
    640
}

pub(crate) fn default_telemetry_samples() -> Vec<u32> {
    vec![195, 224, 188, 262, 210, 176, 238, 192]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudioConfig {
    pub panel_open: bool,
    pub accent: AccentTheme,
    pub radius: CornerRadiusMode,
    pub density: DensityMode,
    pub xray: bool,
    pub layout_engine: LayoutEngineMode,
}

impl StudioConfig {
    /// The self-edits that restyle the whole workspace (everything but the panel toggle), so
    /// style subscribers ignore opening and closing the Studio panel.
    pub const fn styling(self) -> Self {
        Self {
            panel_open: false,
            ..self
        }
    }
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub active_view: ActiveView,
    pub board_filter: BoardFilter,
    pub board_search_query: String,
    pub new_task_draft: String,
    pub tasks: Shared<TaskItem>,
    pub next_task_id: u32,
    pub eng_messages: Shared<ChatMessage>,
    pub design_messages: Shared<ChatMessage>,
    pub eng_draft: String,
    pub design_draft: String,
    pub next_msg_id: u32,
    pub stress_rows: usize,
    pub telemetry_samples_us: Vec<u32>,
    pub stream: Stream100kState,
    pub islands: IslandsSnapshot,
    pub docs: Shared<WorkspaceDoc>,
    pub active_doc_id: u32,
    pub next_doc_id: u32,
    pub next_block_id: u32,
    pub doc_new_block_draft: String,
    pub doc_new_block_kind: DocBlockKind,
    pub doc_editing_block_id: Option<u32>,
    pub doc_edit_draft: String,
    pub doc_markdown_preview_open: bool,
    pub workbench: WorkbenchState,
    pub studio: StudioConfig,
    pub action_label: String,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub mobile_nav_open: bool,
    pub mobile_studio_open: bool,
}

impl Default for WorkspaceSnapshot {
    fn default() -> Self {
        let (docs, active_doc_id, next_doc_id, next_block_id) = default_seed_docs();
        Self {
            active_view: ActiveView::Board,
            board_filter: BoardFilter::All,
            board_search_query: String::new(),
            new_task_draft: String::new(),
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
            ]
            .into(),
            next_task_id: 5,
            eng_messages: vec![
                ChatMessage {
                    id: 1,
                    author: "maya".into(),
                    timestamp: "09:41".into(),
                    body: "Single ABI drives all 4 hosts!".into(),
                    reactions: 4,
                },
                ChatMessage {
                    id: 2,
                    author: "liam".into(),
                    timestamp: "09:44".into(),
                    body: "p99 diff holds 0.38ms @ 1k.".into(),
                    reactions: 3,
                },
                ChatMessage {
                    id: 3,
                    author: "sara".into(),
                    timestamp: "09:49".into(),
                    body: "Tier-2 reload kept snapshots.".into(),
                    reactions: 2,
                },
            ]
            .into(),
            design_messages: vec![
                ChatMessage {
                    id: 101,
                    author: "aria".into(),
                    timestamp: "10:05".into(),
                    body: "Dark slate contrast verified.".into(),
                    reactions: 5,
                },
                ChatMessage {
                    id: 102,
                    author: "noah".into(),
                    timestamp: "10:12".into(),
                    body: "Try X-Ray HUD + Pill radius!".into(),
                    reactions: 3,
                },
            ]
            .into(),
            eng_draft: String::new(),
            design_draft: String::new(),
            next_msg_id: 200,
            stress_rows: 4,
            telemetry_samples_us: default_telemetry_samples(),
            stream: Stream100kState::default(),
            islands: IslandsSnapshot::default(),
            docs: docs.into(),
            active_doc_id,
            next_doc_id,
            next_block_id,
            doc_new_block_draft: String::new(),
            doc_new_block_kind: DocBlockKind::Paragraph,
            doc_editing_block_id: None,
            doc_edit_draft: String::new(),
            doc_markdown_preview_open: false,
            workbench: WorkbenchState::default(),
            studio: StudioConfig::default(),
            action_label: "Workspace Boot".into(),
            viewport_width: default_viewport_width(),
            viewport_height: default_viewport_height(),
            mobile_nav_open: false,
            mobile_studio_open: false,
        }
    }
}
