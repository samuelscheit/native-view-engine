//! Host prop parsing and application helpers for [`crate::StudioWorkspaceModule`].
//!
//! Supports direct binary `NvePropsRef`, binary `NveBinValue`, and UTF-8 JSON fallback props,
//! including deep-link `"route"` navigation and responsive viewport sizing.

use crate::state::{AccentTheme, ActiveView, LayoutEngineMode, SyncTransportMode, WorkspaceState};

pub(crate) fn parse_accent(s: &str) -> Option<AccentTheme> {
    if s.eq_ignore_ascii_case("cyan") {
        Some(AccentTheme::Cyan)
    } else if s.eq_ignore_ascii_case("violet") {
        Some(AccentTheme::Violet)
    } else if s.eq_ignore_ascii_case("green") {
        Some(AccentTheme::Green)
    } else if s.eq_ignore_ascii_case("amber") {
        Some(AccentTheme::Amber)
    } else {
        None
    }
}

pub(crate) fn parse_layout_engine(s: &str) -> Option<LayoutEngineMode> {
    if s.eq_ignore_ascii_case("taffy") {
        Some(LayoutEngineMode::Taffy)
    } else if s.eq_ignore_ascii_case("yoga") {
        Some(LayoutEngineMode::Yoga)
    } else {
        None
    }
}

pub(crate) fn parse_active_view(s: &str) -> Option<ActiveView> {
    if s.eq_ignore_ascii_case("board") {
        Some(ActiveView::Board)
    } else if s.eq_ignore_ascii_case("engineering") {
        Some(ActiveView::ChannelEngineering)
    } else if s.eq_ignore_ascii_case("design") {
        Some(ActiveView::ChannelDesign)
    } else if s.eq_ignore_ascii_case("docs") || s.eq_ignore_ascii_case("doc") {
        Some(ActiveView::Docs)
    } else if s.eq_ignore_ascii_case("workbench")
        || s.eq_ignore_ascii_case("wb")
        || s.eq_ignore_ascii_case("preview")
    {
        Some(ActiveView::Workbench)
    } else if s.eq_ignore_ascii_case("stream")
        || s.eq_ignore_ascii_case("stream100k")
        || s.eq_ignore_ascii_case("100k")
    {
        Some(ActiveView::Stream100k)
    } else if s.eq_ignore_ascii_case("islands") || s.eq_ignore_ascii_case("island") {
        Some(ActiveView::Islands)
    } else if s.eq_ignore_ascii_case("metrics") {
        Some(ActiveView::Metrics)
    } else {
        None
    }
}

#[derive(Default)]
pub(crate) struct PropUpdates<'a> {
    pub studio_open: Option<bool>,
    pub accent: Option<&'a str>,
    pub layout_engine: Option<&'a str>,
    pub view: Option<&'a str>,
    pub route: Option<&'a str>,
    pub viewport_width: Option<f64>,
    pub viewport_height: Option<f64>,
    pub stream_scroll_y: Option<u64>,
    pub sync_transport: Option<&'a str>,
    pub sync_peers: Option<u16>,
    pub sync_kbps: Option<f64>,
    pub sync_compression: Option<f64>,
    pub sync_rtt_ms: Option<f64>,
}

pub(crate) fn apply_prop_updates(guard: &mut WorkspaceState, upd: PropUpdates<'_>) -> bool {
    let mut changed = false;
    if let Some(open) = upd.studio_open
        && guard.current.studio.panel_open != open
    {
        guard.toggle_studio_panel();
        changed = true;
    }
    if let Some(accent) = upd.accent.and_then(parse_accent) {
        guard.set_accent(accent);
        changed = true;
    }
    if let Some(mode) = upd.layout_engine.and_then(parse_layout_engine) {
        guard.set_layout_engine(mode);
        changed = true;
    }
    if let Some(uri) = upd.route
        && guard.navigate_uri(uri)
    {
        changed = true;
    } else if let Some(v) = upd.view.and_then(parse_active_view) {
        guard.select_view(v);
        changed = true;
    }
    if upd.viewport_width.is_some() || upd.viewport_height.is_some() {
        let next_w = upd
            .viewport_width
            .map(|v| v as f32)
            .unwrap_or(guard.current.viewport_width as f32);
        let next_h = upd
            .viewport_height
            .map(|v| v as f32)
            .unwrap_or(guard.current.viewport_height as f32);
        if guard.set_viewport_size(next_w, next_h) {
            changed = true;
        }
    }
    if let Some(scroll_y) = upd.stream_scroll_y {
        guard.set_stream_scroll_offset(scroll_y.min(u32::MAX as u64) as u32);
        changed = true;
    }
    if upd.sync_transport.is_some()
        || upd.sync_peers.is_some()
        || upd.sync_kbps.is_some()
        || upd.sync_compression.is_some()
        || upd.sync_rtt_ms.is_some()
    {
        let mut tel = guard.sync_telemetry.clone();
        if let Some(mode) = upd.sync_transport.and_then(SyncTransportMode::parse) {
            tel.transport = mode;
        }
        if let Some(peers) = upd.sync_peers {
            tel.connected_peers = peers;
        }
        if let Some(kbps) = upd.sync_kbps.filter(|v| v.is_finite() && *v >= 0.0) {
            tel.kbps_tenths = (kbps * 10.0).round() as u32;
        }
        if let Some(ratio) = upd.sync_compression.filter(|v| v.is_finite() && *v >= 0.0) {
            tel.compression_tenths = ((ratio * 10.0).round() as u32).min(u16::MAX as u32) as u16;
        }
        if let Some(rtt) = upd.sync_rtt_ms.filter(|v| v.is_finite() && *v >= 0.0) {
            tel.rtt_ms_tenths = (rtt * 10.0).round() as u32;
        }
        if guard.update_sync_telemetry(tel) {
            changed = true;
        }
    }
    changed
}
