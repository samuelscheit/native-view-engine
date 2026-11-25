//! Deterministic state machine, workspace domain model, bounded time-travel history,
//! and zero-JSON binary snapshot codec for **Aether Workspace** + **NVE Studio**.

mod actions;
mod codec;
mod history;
mod models;

pub use codec::{WORKSPACE_STATE_BIN_MAGIC, WORKSPACE_STATE_BIN_VERSION};
pub use models::{
    AccentTheme, ActiveView, BoardFilter, ChannelKind, ChatMessage, CornerRadiusMode, DensityMode,
    LayoutEngineMode, Priority, Stream100kState, StudioConfig, SyncTelemetryState,
    SyncTransportMode, TaskItem, TaskStatus, WorkspaceSnapshot, MAX_HISTORY, MAX_STRESS_ROWS,
    MAX_TELEMETRY_SAMPLES, MOBILE_BREAKPOINT_PT,
};

use crate::ui_palette::CommandPaletteState;

/// Top-level runtime state persisted across Tier-2 `cdylib` hot-swaps and time-travel scrubs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceState {
    pub current: WorkspaceSnapshot,
    pub history: Vec<WorkspaceSnapshot>,
    pub cursor: usize,
    pub frame_count: u64,
    pub palette: CommandPaletteState,
    pub sync_telemetry: SyncTelemetryState,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        let initial = WorkspaceSnapshot::default();
        Self {
            history: vec![initial.clone()],
            current: initial,
            cursor: 0,
            frame_count: 1,
            palette: CommandPaletteState::default(),
            sync_telemetry: SyncTelemetryState::default(),
        }
    }
}
