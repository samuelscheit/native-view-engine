//! Multi-Viewport Meta Workbench view (`ActiveView::Workbench`) built on shared `nve-ui`
//! components (`Card`, `TabsRoot`, `TabList`, `TabTrigger`, `Button`, `Badge`).
//!
//! Renders a synchronized live Multi-Viewport preview grid (**Desktop `1280×800`**,
//! **Tablet `820×1180`**, **Mobile `390×844`**) reflecting live `WorkspaceState` design tokens,
//! layout engine, and issue/doc counts, alongside 9 live **Connected Host Surfaces** cards.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, TabList, TabTrigger,
    TabsRoot,
};

use crate::state::{
    SyncTelemetryState, WorkbenchPreset, WorkspaceSnapshot, WorkspaceState, HOST_SURFACES,
};
use crate::theme::{
    badge_min_w, card_border, control_border, row_border, LayoutMetrics, BORDER_DEFAULT, CANVAS_BG,
    CARD_ALT_BG, CARD_BG, CONTROL_BG, PANEL_BG, STRETCH_COL, TEXT_MUTED, TEXT_PRIMARY,
    TEXT_SECONDARY,
};
use crate::ui::mutate_state;

fn format_tenths_ms(tenths: u32) -> String {
    format!("{}.{}ms", tenths / 10, tenths % 10)
}

fn parse_preset(val: &str) -> WorkbenchPreset {
    match val {
        "desktop" => WorkbenchPreset::Desktop,
        "tablet" => WorkbenchPreset::Tablet,
        "mobile" => WorkbenchPreset::Mobile,
        _ => WorkbenchPreset::All,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_preview_frame(
    test_id: &'static str,
    title: &'static str,
    dims: &'static str,
    layout_tag: &'static str,
    columns: u8,
    current: &WorkspaceSnapshot,
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let show_chrome = current.workbench.show_device_chrome;
    let preview_radius = m.card_radius.min(14);
    let inner_pad = if m.card_pad <= 8 { 5 } else { 7 };
    let issue_count = current.tasks.len();
    let doc_count = current.docs.len();
    let engine_lbl = current.studio.layout_engine.label();
    let top_task_key = current
        .tasks
        .first()
        .map(|t| t.key.as_str())
        .unwrap_or("NONE");
    let active_doc_slug = current
        .docs
        .iter()
        .find(|d| d.id == current.active_doc_id)
        .or_else(|| current.docs.first())
        .map(|d| d.slug.as_str())
        .unwrap_or("docs");
    let metrics_line = format!("{issue_count} issues | {doc_count} docs");
    let token_line = format!(
        "{} | {}px | {}",
        current.studio.accent.label(),
        current.studio.radius.px(),
        engine_lbl
    );
    let frame_card_style = format!(
        "flex: 1; padding: {inner_pad}px; gap: 5px; background-color: {PANEL_BG}; border-radius: {preview_radius}px; border-width: {card_bw}px; border-color: {card_bc};"
    );
    let dims_badge_style = format!(
        "min-width: {}px; height: 16px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 3px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        badge_min_w(dims),
        m.pill_radius
    );

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some(test_id.into()),
            padding: Some(inner_pad as f32),
            gap: Some(5.0),
            style: Some(frame_card_style),

            if show_chrome {
                view {
                    style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center;",
                    text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "{title}" }
                    Badge {
                        label: dims.to_string(),
                        variant: BadgeVariant::Neutral,
                        style: Some(dims_badge_style),
                    }
                }
            }

            // Synchronized Miniature Live Viewport Surface
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; padding: 6px; gap: 5px; background-color: {CANVAS_BG}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {accent_hex};",
                view {
                    style: "width: 100%; height: 18px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 5px; background-color: {PANEL_BG}; border-radius: 4px;",
                    text { style: "color: {accent_hex}; font-size: 9px;", "AETHER" }
                    text { style: "color: {TEXT_MUTED}; font-size: 8px;", "{layout_tag}" }
                }

                view {
                    style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 4px;",
                    if columns >= 2 {
                        view {
                            style: "width: 28px; flex-shrink: 0; flex-direction: column; gap: 3px; padding: 3px; background-color: {PANEL_BG}; border-radius: 4px;",
                            view { style: "width: 100%; height: 6px; background-color: {accent_hex}; border-radius: 2px;" }
                            view { style: "width: 100%; height: 6px; background-color: {CONTROL_BG}; border-radius: 2px;" }
                            view { style: "width: 100%; height: 6px; background-color: {CONTROL_BG}; border-radius: 2px;" }
                        }
                    }
                    view {
                        style: "flex: 1; flex-direction: column; gap: 3px; padding: 4px; background-color: {CARD_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                        text { style: "color: {TEXT_PRIMARY}; font-size: 9px;", "{top_task_key} | {active_doc_slug}" }
                        text { style: "color: {TEXT_SECONDARY}; font-size: 8px;", "{metrics_line}" }
                        text { style: "color: {accent_hex}; font-size: 8px;", "{token_line}" }
                    }
                    if columns >= 3 {
                        view {
                            style: "width: 26px; flex-shrink: 0; flex-direction: column; gap: 3px; padding: 3px; background-color: {CARD_ALT_BG}; border-radius: 4px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                            view { style: "width: 100%; height: 8px; background-color: {accent_tint}; border-radius: 2px; border-width: 1px; border-color: {accent_hex};" }
                            view { style: "width: 100%; height: 8px; background-color: {CONTROL_BG}; border-radius: 2px;" }
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_workbench_view(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    sync_telemetry: &SyncTelemetryState,
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
    is_mobile: bool,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);
    let preset = current.workbench.preset;
    let show_chrome = current.workbench.show_device_chrome;

    let (chr_bw, chr_bc) = control_border(xray, show_chrome, accent_hex);
    let chr_bg = if show_chrome { accent_tint } else { CONTROL_BG };

    let live_tokens_summary = format!(
        "{} | {}px | {} | {} | {} issues | {} docs",
        current.studio.accent.label(),
        current.studio.radius.px(),
        current.studio.density.label(),
        current.studio.layout_engine.label(),
        current.tasks.len(),
        current.docs.len()
    );

    let telemetry_summary = format!(
        "{} | {} peers | {} | {} | RTT {}",
        sync_telemetry.transport.label(),
        sync_telemetry.connected_peers,
        sync_telemetry.format_kbps(),
        sync_telemetry.format_compression(),
        sync_telemetry.format_rtt()
    );

    let s_preset = Arc::clone(store);
    let s_chrome = Arc::clone(store);
    let s_sync = Arc::clone(store);
    let viewport_grid_dir = if is_mobile { "column" } else { "row" };
    let host_chunk_size = if is_mobile { 1 } else { 3 };

    let controls_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 8px; gap: 5px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );
    let chrome_btn_style = format!(
        "width: 56px; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {chr_bg}; border-radius: {}px; border-width: {chr_bw}px; border-color: {chr_bc};",
        m.pill_radius
    );
    let banner_card_style = format!(
        "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; gap: 6px; padding: 7px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );
    let sync_btn_style = format!(
        "min-width: {}px; height: 22px; min-height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        badge_min_w(sync_telemetry.transport.label()),
        m.pill_radius
    );

    rsx! {
        // 1. Preset Tabs + Synchronized Live Token Strip (`nve-ui` Card + Tabs + Button)
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("wb-controls".into()),
            padding: Some(8.0),
            gap: Some(5.0),
            style: Some(controls_card_style),
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 4px; align-items: center; border-width: {row_bw}px; border-color: {row_bc};",
                view {
                    style: "flex: 1;",
                    TabsRoot {
                        value: Some(preset.as_str().to_string()),
                        id_prefix: Some("wb-preset".into()),
                        style: Some("width: 100%; gap: 0px;".into()),
                        on_change: move |next: String| {
                            let p = parse_preset(&next);
                            mutate_state(&s_preset, version, |st| st.set_workbench_preset(p));
                        },
                        TabList {
                            accessibility_label: Some("Workbench viewport preset".into()),
                            style: Some("width: 100%; flex-direction: row; gap: 4px; padding: 0px; background-color: transparent; border-width: 0px; border-color: transparent; border-radius: 0px;".into()),
                            for &(p, tid, lbl) in [
                                (WorkbenchPreset::All, "wb-preset-all", "All (3)"),
                                (WorkbenchPreset::Desktop, "wb-preset-desktop", "Desktop"),
                                (WorkbenchPreset::Tablet, "wb-preset-tablet", "Tablet"),
                                (WorkbenchPreset::Mobile, "wb-preset-mobile", "Mobile"),
                            ].iter() {
                                {
                                    let is_sel = preset == p;
                                    let (bw, bc) = control_border(xray, is_sel, accent_hex);
                                    let bg = if is_sel { accent_tint } else { CONTROL_BG };
                                    let trig_style = format!(
                                        "flex: 1; height: 24px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                        m.pill_radius
                                    );
                                    rsx! {
                                        TabTrigger {
                                            key: "{tid}",
                                            value: p.as_str().to_string(),
                                            test_id: Some(tid.into()),
                                            style: Some(trig_style),
                                            "{lbl}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("wb-toggle-chrome".into()),
                    style: Some(chrome_btn_style),
                    on_press: move |_| mutate_state(&s_chrome, version, |st| st.toggle_workbench_chrome()),
                    "Chrome"
                }
            }
            text {
                test_id: "wb-live-tokens",
                style: "color: {TEXT_MUTED}; font-size: 10px;",
                "{live_tokens_summary}"
            }
        }

        // 2. Scrollable Multi-Viewport Grid + 9 Connected Host Surfaces
        scrollview {
            style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",

            view {
                test_id: "wb-viewport-grid",
                nve_table: {
                    let preset_str = preset.as_str().to_owned();
                    nve_dioxus::table_attr_fn(move |tx, node| {
                        tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                            nve_web_html::DataAttrList::from_pairs(vec![(
                                "workbench-preset".to_owned(),
                                preset_str.clone(),
                            )]),
                        ));
                    })
                },
                style: "width: 100%; flex-shrink: 0; flex-direction: {viewport_grid_dir}; gap: 6px;",
                if preset.includes_desktop() {
                    {render_preview_frame("wb-frame-desktop", "Desktop", "1280x800", "3-col", 3, current, accent_hex, accent_tint, xray, m)}
                }
                if preset.includes_tablet() {
                    {render_preview_frame("wb-frame-tablet", "Tablet", "820x1180", "2-col", 2, current, accent_hex, accent_tint, xray, m)}
                }
                if preset.includes_mobile() {
                    {render_preview_frame("wb-frame-mobile", "Mobile", "390x844", "1-col", 1, current, accent_hex, accent_tint, xray, m)}
                }
            }

            // Live Connected Host Surfaces Header + SyncTelemetry Banner
            Card {
                variant: CardVariant::Outlined,
                test_id: Some("wb-telemetry-banner".into()),
                padding: Some(7.0),
                gap: Some(6.0),
                style: Some(banner_card_style),
                view {
                    style: "flex: 1; flex-direction: column; gap: 2px;",
                    text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "CONNECTED HOST SURFACES (9)" }
                    text { style: "color: {accent_hex}; font-size: 9px;", "{telemetry_summary}" }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("wb-sync-cycle".into()),
                    style: Some(sync_btn_style),
                    on_press: move |_| mutate_state(&s_sync, version, |st| st.cycle_sync_transport()),
                    "{sync_telemetry.transport.label()}"
                }
            }

            // 9 Connected Host Surface Cards (3x3 desktop, 1-col mobile portrait)
            view {
                test_id: "wb-hosts-grid",
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 6px;",
                for (row_idx, chunk) in HOST_SURFACES.chunks(host_chunk_size).enumerate() {
                    view {
                        key: "wb-host-row-{row_idx}",
                        style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px;",
                        for spec in chunk.iter() {
                            {
                                let slug = spec.slug;
                                let host_tid = format!("wb-host-{slug}");
                                let is_sel = current.workbench.selected_host_slug == slug;
                                let (bw, bc) = control_border(xray, is_sel, accent_hex);
                                let bg = if is_sel { CARD_ALT_BG } else { CARD_BG };
                                let rtt_tenths = if sync_telemetry.rtt_ms_tenths == 0 {
                                    0
                                } else {
                                    sync_telemetry.rtt_ms_tenths + spec.rtt_offset_tenths
                                };
                                let host_metrics = format!(
                                    "{} | {} | {}",
                                    sync_telemetry.format_kbps(),
                                    sync_telemetry.format_compression(),
                                    format_tenths_ms(rtt_tenths)
                                );
                                let live_badge_style = format!(
                                    "min-width: 34px; height: 15px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {}; border-radius: 4px; border-width: 0px; border-color: transparent;",
                                    spec.badge_bg
                                );
                                let s_host = Arc::clone(store);
                                rsx! {
                                    pressable {
                                        key: "wb-h-{slug}",
                                        test_id: "{host_tid}",
                                        style: "flex: 1; flex-direction: column; padding: 7px; gap: 3px; background-color: {bg}; border-radius: {m.button_radius}px; border-width: {bw}px; border-color: {bc};",
                                        onpress: move |_| mutate_state(&s_host, version, |st| st.select_workbench_host(slug)),
                                        view {
                                            style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                                            text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "{spec.name}" }
                                            Badge {
                                                label: "LIVE".to_string(),
                                                variant: BadgeVariant::Neutral,
                                                style: Some(live_badge_style),
                                            }
                                        }
                                        text { style: "color: {TEXT_MUTED}; font-size: 9px;", "{spec.pipeline}" }
                                        text { style: "color: {TEXT_SECONDARY}; font-size: 9px;", "{host_metrics}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
