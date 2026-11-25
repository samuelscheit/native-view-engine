//! Docked / drawer **NVE Studio** live self-editing meta-workbench panel.
//!
//! Built with shared `nve-ui` primitives (`Card`, `Button`, `Badge`, `Separator`), exposing
//! Accent swatches, Corner Radius, Density, Live Taffy <-> Yoga Layout Engine switcher,
//! X-Ray HUD, Live Sync & multi-peer binary `NVC1` CRDT telemetry (with Sync Now and
//! Partition/Heal controls), and deterministic Time-Travel Scrubber.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_dioxus::traits::TraitScope;
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator};

use crate::state::{
    AccentTheme, CornerRadiusMode, DensityMode, LayoutEngineMode, SyncTelemetryState,
    WorkspaceSnapshot, WorkspaceState,
};
use crate::theme::{
    badge_min_w, card_border, control_border, panel_border, row_border, LayoutMetrics, CARD_ALT_BG,
    CONTROL_BG, PANEL_BG, STRETCH_COL, TEXT_MUTED, TEXT_PRIMARY,
};
use crate::traits::{studio_compact_override, StudioTraitPreviewChip};
use crate::ui::mutate_state;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_studio_panel(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    sync_telemetry: &SyncTelemetryState,
    snap_idx: usize,
    hist_len: usize,
    can_undo: bool,
    can_redo: bool,
    m: LayoutMetrics,
    is_mobile: bool,
) -> Element {
    let studio = &current.studio;
    let accent = studio.accent;
    let accent_hex = accent.hex();
    let accent_tint = accent.tint_bg();
    let xray = studio.xray;
    let layout_engine = studio.layout_engine;

    let (panel_bw, panel_bc) = panel_border(xray);
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);

    let action_label = current.action_label.as_str();

    let undo_opacity = if can_undo { "1.0" } else { "0.4" };
    let redo_opacity = if can_redo { "1.0" } else { "0.4" };

    let studio_panel_style = if is_mobile {
        format!(
            "width: 100%; flex-shrink: 0; flex-direction: column; padding: 7px; gap: 5px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};"
        )
    } else {
        format!(
            "width: 210px; height: 100%; flex-shrink: 0; flex-direction: column; padding: 7px; gap: 5px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};"
        )
    };
    let section_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 5px; gap: 3px; background-color: {CARD_ALT_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );

    let layout_engine_str = layout_engine.as_str();
    let layout_readout_lbl = layout_engine.readout_label();

    let xray_bg = if xray { "#0c2d42" } else { CONTROL_BG };
    let xray_bc = if xray { "#38bdf8" } else { "#334155" };
    let xray_lbl = if xray { "X-Ray HUD: ON" } else { "X-Ray HUD: OFF" };
    let xray_btn_style = format!(
        "width: 100%; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {xray_bg}; border-radius: {}px; border-width: 1px; border-color: {xray_bc};",
        m.button_radius
    );

    let s_xray = Arc::clone(store);
    let s_sync = Arc::clone(store);
    let s_crdt_sync = Arc::clone(store);
    let s_crdt_part = Arc::clone(store);
    let s_tt_first = Arc::clone(store);
    let s_tt_undo = Arc::clone(store);
    let s_tt_redo = Arc::clone(store);
    let s_tt_live = Arc::clone(store);

    let sync_transport_lbl = sync_telemetry.transport.label();
    let sync_transport_slug = sync_telemetry.transport.as_str();
    let sync_peers_lbl = format!(
        "{} {}",
        sync_telemetry.connected_peers,
        if sync_telemetry.connected_peers == 1 { "peer" } else { "peers" }
    );
    let sync_metrics_lbl = format!(
        "{} | {} | {}",
        sync_telemetry.format_kbps(),
        sync_telemetry.format_compression(),
        sync_telemetry.format_rtt()
    );
    let crdt_summary = format!(
        "NVC1 {}/{}d | {}B",
        sync_telemetry.crdt_deltas_sent,
        sync_telemetry.crdt_deltas_received,
        sync_telemetry.crdt_bytes_synced
    );
    let crdt_status_lbl = if sync_telemetry.crdt_partitioned { "SPLIT" } else { "MESH" };
    let crdt_status_var = if sync_telemetry.crdt_partitioned {
        BadgeVariant::Danger
    } else {
        BadgeVariant::Success
    };
    let part_btn_lbl = if sync_telemetry.crdt_partitioned {
        "Heal Mesh"
    } else {
        "Partition"
    };
    let part_btn_var = if sync_telemetry.crdt_partitioned {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Destructive
    };

    let live_badge_style = format!(
        "min-width: {}px; height: 17px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        badge_min_w("LIVE SELF-EDIT"),
        m.pill_radius
    );
    let sync_cycle_style = format!(
        "min-width: {}px; height: 17px; min-height: 17px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        badge_min_w(sync_transport_lbl),
        m.pill_radius
    );
    let crdt_badge_style = format!(
        "min-width: {}px; height: 15px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 3px; padding-vertical: 0px; border-radius: {}px;",
        badge_min_w(crdt_status_lbl),
        m.pill_radius
    );
    let crdt_sync_btn_style = format!(
        "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        m.pill_radius
    );
    let crdt_part_btn_style = format!(
        "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; border-radius: {}px;",
        m.pill_radius
    );
    let tt_undo_style = format!(
        "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: #334155; opacity: {undo_opacity};",
        m.pill_radius
    );
    let tt_redo_style = format!(
        "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: #334155; opacity: {redo_opacity};",
        m.pill_radius
    );

    rsx! {
        view {
            test_id: "studio-panel",
            "layout-engine": "{layout_engine_str}",
            style: "{studio_panel_style}",

            // Studio Header
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; border-width: {row_bw}px; border-color: {row_bc};",
                text {
                    style: "color: {TEXT_PRIMARY}; font-size: 12px;",
                    "NVE STUDIO"
                }
                Badge {
                    label: "LIVE SELF-EDIT".to_string(),
                    variant: BadgeVariant::Neutral,
                    style: Some(live_badge_style),
                }
            }

            // Live Environment TraitKey Readout (inherits root TraitProvider; compact preview uses TraitScope)
            StudioTraitPreviewChip {
                test_id: Some("studio-trait-live".into()),
            }
            if !is_mobile {
                TraitScope {
                    overrides: studio_compact_override(true),
                    StudioTraitPreviewChip {
                        test_id: Some("studio-trait-scoped".into()),
                    }
                }
            }

            // Accent Swatches (2x2)
            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                style: Some(section_card_style.clone()),
                text { style: "color: {TEXT_MUTED}; font-size: 9px;", "ACCENT" }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    for &(theme, tid) in [
                        (AccentTheme::Cyan, "accent-cyan"),
                        (AccentTheme::Violet, "accent-violet"),
                    ].iter() {
                        {
                            let sel = accent == theme;
                            let swatch = theme.hex();
                            let bg = if sel { swatch } else { CONTROL_BG };
                            let (bw, bc) = control_border(xray, sel, swatch);
                            let s_acc = Arc::clone(store);
                            let bstyle = format!(
                                "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                m.pill_radius
                            );
                            rsx! {
                                Button {
                                    key: "{tid}",
                                    variant: if sel { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                                    size: ButtonSize::Sm,
                                    test_id: Some(tid.into()),
                                    style: Some(bstyle),
                                    on_press: move |_| mutate_state(&s_acc, version, |st| st.set_accent(theme)),
                                    "{theme.label()}"
                                }
                            }
                        }
                    }
                }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    for &(theme, tid) in [
                        (AccentTheme::Green, "accent-green"),
                        (AccentTheme::Amber, "accent-amber"),
                    ].iter() {
                        {
                            let sel = accent == theme;
                            let swatch = theme.hex();
                            let bg = if sel { swatch } else { CONTROL_BG };
                            let (bw, bc) = control_border(xray, sel, swatch);
                            let s_acc = Arc::clone(store);
                            let bstyle = format!(
                                "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                m.pill_radius
                            );
                            rsx! {
                                Button {
                                    key: "{tid}",
                                    variant: if sel { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                                    size: ButtonSize::Sm,
                                    test_id: Some(tid.into()),
                                    style: Some(bstyle),
                                    on_press: move |_| mutate_state(&s_acc, version, |st| st.set_accent(theme)),
                                    "{theme.label()}"
                                }
                            }
                        }
                    }
                }
            }

            // Corner Radius + Density Card
            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                style: Some(section_card_style.clone()),
                text { style: "color: {TEXT_MUTED}; font-size: 9px;", "RADIUS (0 / 12 / 18)" }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    for &(rmode, tid, short_lbl) in [
                        (CornerRadiusMode::Sharp, "radius-sharp", "Sharp"),
                        (CornerRadiusMode::Soft, "radius-soft", "Soft"),
                        (CornerRadiusMode::Pill, "radius-pill", "Pill"),
                    ].iter() {
                        {
                            let sel = studio.radius == rmode;
                            let bg = if sel { accent_tint } else { CONTROL_BG };
                            let (bw, bc) = control_border(xray, sel, accent_hex);
                            let s_rad = Arc::clone(store);
                            let bstyle = format!(
                                "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                m.pill_radius
                            );
                            rsx! {
                                Button {
                                    key: "{tid}",
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    test_id: Some(tid.into()),
                                    style: Some(bstyle),
                                    on_press: move |_| mutate_state(&s_rad, version, |st| st.set_radius(rmode)),
                                    "{short_lbl}"
                                }
                            }
                        }
                    }
                }

                text { style: "color: {TEXT_MUTED}; font-size: 9px;", "DENSITY" }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    for &(dmode, tid) in [
                        (DensityMode::Normal, "density-normal"),
                        (DensityMode::Compact, "density-compact"),
                    ].iter() {
                        {
                            let sel = studio.density == dmode;
                            let bg = if sel { accent_tint } else { CONTROL_BG };
                            let (bw, bc) = control_border(xray, sel, accent_hex);
                            let s_den = Arc::clone(store);
                            let bstyle = format!(
                                "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                m.pill_radius
                            );
                            rsx! {
                                Button {
                                    key: "{tid}",
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    test_id: Some(tid.into()),
                                    style: Some(bstyle),
                                    on_press: move |_| mutate_state(&s_den, version, |st| st.set_density(dmode)),
                                    "{dmode.label()}"
                                }
                            }
                        }
                    }
                }
            }

            // Live Taffy <-> Yoga Layout Engine Switcher + Inspector Readout
            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                style: Some(section_card_style.clone()),
                text { style: "color: {TEXT_MUTED}; font-size: 9px;", "LAYOUT ENGINE" }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    for &(emode, tid) in [
                        (LayoutEngineMode::Taffy, "layout-taffy"),
                        (LayoutEngineMode::Yoga, "layout-yoga"),
                    ].iter() {
                        {
                            let sel = layout_engine == emode;
                            let bg = if sel { accent_tint } else { CONTROL_BG };
                            let (bw, bc) = control_border(xray, sel, accent_hex);
                            let s_eng = Arc::clone(store);
                            let bstyle = format!(
                                "flex: 1; height: 20px; min-height: 20px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                m.pill_radius
                            );
                            rsx! {
                                Button {
                                    key: "{tid}",
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    test_id: Some(tid.into()),
                                    style: Some(bstyle),
                                    on_press: move |_| mutate_state(&s_eng, version, |st| st.set_layout_engine(emode)),
                                    "{emode.label()}"
                                }
                            }
                        }
                    }
                }
                view {
                    test_id: "layout-engine-readout",
                    "layout-engine": "{layout_engine_str}",
                    style: "width: 100%; flex-shrink: 0;",
                    text {
                        style: "color: {accent_hex}; font-size: 9px;",
                        "{layout_readout_lbl}"
                    }
                }
            }

            // X-Ray HUD Toggle
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("toggle-xray".into()),
                style: Some(xray_btn_style),
                on_press: move |_| mutate_state(&s_xray, version, |st| st.toggle_xray()),
                "{xray_lbl}"
            }

            // Live Sync / Pairing + Binary NVC1 CRDT Mesh Telemetry Card
            Card {
                variant: CardVariant::Subtle,
                test_id: Some("studio-sync-telemetry".into()),
                padding: Some(5.0),
                gap: Some(3.0),
                style: Some(section_card_style.clone()),
                view {
                    "sync-transport": "{sync_transport_slug}",
                    style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                    text { style: "color: {TEXT_MUTED}; font-size: 9px;", "LIVE SYNC" }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("sync-transport-cycle".into()),
                        accessibility_label: Some("Cycle live sync transport mode".into()),
                        style: Some(sync_cycle_style),
                        on_press: move |_| mutate_state(&s_sync, version, |st| st.cycle_sync_transport()),
                        "{sync_transport_lbl}"
                    }
                }
                view {
                    style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                    text { style: "color: {TEXT_PRIMARY}; font-size: 9px;", "{sync_peers_lbl}" }
                    text { style: "color: #cbd5e1; font-size: 9px;", "{sync_metrics_lbl}" }
                }
                Separator {}
                view {
                    test_id: "studio-crdt-metrics",
                    style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                    text { style: "color: {accent_hex}; font-size: 9px;", "{crdt_summary}" }
                    Badge {
                        label: crdt_status_lbl.to_string(),
                        variant: crdt_status_var,
                        style: Some(crdt_badge_style),
                    }
                }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("crdt-sync-now".into()),
                        accessibility_label: Some("Sync binary CRDT deltas now".into()),
                        style: Some(crdt_sync_btn_style),
                        on_press: move |_| mutate_state(&s_crdt_sync, version, |st| st.sync_crdt_now()),
                        "Sync Now"
                    }
                    Button {
                        variant: part_btn_var,
                        size: ButtonSize::Sm,
                        test_id: Some("crdt-toggle-partition".into()),
                        accessibility_label: Some("Simulate or heal CRDT network partition".into()),
                        style: Some(crdt_part_btn_style),
                        on_press: move |_| mutate_state(&s_crdt_part, version, |st| st.toggle_crdt_partition()),
                        "{part_btn_lbl}"
                    }
                }
            }

            // Deterministic Time-Travel Scrubber
            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                style: Some(section_card_style),
                text { style: "color: {TEXT_MUTED}; font-size: 9px;", "TIME TRAVEL" }
                text {
                    style: "color: {TEXT_PRIMARY}; font-size: 10px;",
                    "Snapshot {snap_idx} / {hist_len}"
                }
                text {
                    style: "color: {accent_hex}; font-size: 9px;",
                    "{action_label}"
                }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("tt-first".into()),
                        style: Some(tt_undo_style.clone()),
                        on_press: move |_| mutate_state(&s_tt_first, version, |st| st.scrub_first()),
                        "<< #1"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("tt-undo".into()),
                        style: Some(tt_undo_style),
                        on_press: move |_| mutate_state(&s_tt_undo, version, |st| st.scrub_undo()),
                        "< Undo"
                    }
                }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("tt-redo".into()),
                        style: Some(tt_redo_style.clone()),
                        on_press: move |_| mutate_state(&s_tt_redo, version, |st| st.scrub_redo()),
                        "Redo >"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("tt-live".into()),
                        style: Some(tt_redo_style),
                        on_press: move |_| mutate_state(&s_tt_live, version, |st| st.scrub_live()),
                        "Live >>"
                    }
                }
            }
        }
    }
}
