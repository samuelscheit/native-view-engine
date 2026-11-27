//! Multi-Viewport Meta Workbench view (`ActiveView::Workbench`) built on shared `nve-ui`
//! components (`Card`, `TabsRoot`, `TabList`, `TabTrigger`, `Button`, `Badge`).
//!
//! Renders a synchronized live Multi-Viewport preview grid (**Desktop `1280×800`**,
//! **Tablet `820×1180`**, **Mobile `390×844`**) reflecting live Studio design tokens, layout
//! engine, and issue/doc counts, alongside 9 live **Connected Host Surfaces** cards.
//!
//! Subscribes to the `workbench`, `studio`, and `sync` domains.

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, TabList,
    TabTrigger, TabsRoot,
};

use crate::state::{HOST_SURFACES, StudioConfig, SyncTelemetryState, WorkbenchPreset};
use crate::theme::badge_min_w;
use crate::ui::store::{WorkbenchPreview, use_studio};
use crate::ui::style::{data_attrs, use_studio_style};

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

fn telemetry_summary(t: &SyncTelemetryState) -> String {
    format!(
        "{} | {} peers | {} | {} | RTT {}",
        t.transport.label(),
        t.connected_peers,
        t.format_kbps(),
        t.format_compression(),
        t.format_rtt()
    )
}

/// One miniature live viewport of the workspace.
#[component]
fn PreviewFrame(
    test_id: &'static str,
    title: &'static str,
    dims: &'static str,
    layout_tag: &'static str,
    columns: u8,
    show_chrome: bool,
    preview: WorkbenchPreview,
    config: StudioConfig,
) -> Element {
    nve_dioxus::render_probe::record_render("PreviewFrame");
    let s = use_studio_style();
    let t = s.t;
    let preview_radius = s.m.card_radius.min(14);
    let inner_pad = if s.m.card_pad <= 8 { 5 } else { 7 };
    let metrics_line = format!(
        "{} issues | {} docs",
        preview.issue_count, preview.doc_count
    );
    let token_line = format!(
        "{} | {}px | {}",
        config.accent.label(),
        config.radius.px(),
        config.layout_engine.label()
    );
    let dims_badge_style = s.compact_control(
        &format!("min-width: {}px; height: 16px;", badge_min_w(dims)),
        3,
        s.m.pill_radius,
        Some(true),
    );
    let (base, subtle, link) = (
        t.surface.base.hex(),
        t.surface.subtle.hex(),
        t.text.link.hex(),
    );

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some(test_id.into()),
            padding: Some(inner_pad as f32),
            gap: Some(5.0),
            style: Some(format!(
                "flex: 1; padding: {inner_pad}px; gap: 5px; background-color: {base}; border-radius: {preview_radius}px; {}",
                s.card()
            )),
            if show_chrome {
                view {
                    style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center;",
                    text { style: "color: {t.text.primary.hex()}; font-size: 10px;", "{title}" }
                    Badge { label: dims.to_string(), variant: BadgeVariant::Neutral, style: Some(dims_badge_style) }
                }
            }
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; padding: 6px; gap: 5px; background-color: {t.surface.canvas.hex()}; border-radius: {s.m.button_radius}px; {s.control(true)}",
                view {
                    style: "width: 100%; height: 18px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 5px; background-color: {base}; border-radius: {t.radius.sm}px;",
                    text { style: "color: {link}; font-size: 9px;", "AETHER" }
                    text { style: "color: {t.text.tertiary.hex()}; font-size: 8px;", "{layout_tag}" }
                }
                view {
                    style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 4px;",
                    if columns >= 2 {
                        view {
                            style: "width: 28px; flex-shrink: 0; flex-direction: column; gap: 3px; padding: 3px; background-color: {base}; border-radius: {t.radius.sm}px;",
                            view { style: "width: 100%; height: 6px; background-color: {t.status.accent.solid.hex()}; border-radius: 2px;" }
                            view { style: "width: 100%; height: 6px; background-color: {subtle}; border-radius: 2px;" }
                            view { style: "width: 100%; height: 6px; background-color: {subtle}; border-radius: 2px;" }
                        }
                    }
                    view {
                        style: "flex: 1; flex-direction: column; gap: 3px; padding: 4px; background-color: {t.surface.raised.hex()}; border-radius: {s.m.pill_radius}px; border-width: 1px; border-color: {t.border.default.hex()};",
                        text { style: "color: {t.text.primary.hex()}; font-size: 9px;", "{preview.top_task_key} | {preview.active_doc_slug}" }
                        text { style: "color: {t.text.secondary.hex()}; font-size: 8px;", "{metrics_line}" }
                        text { style: "color: {link}; font-size: 8px;", "{token_line}" }
                    }
                    if columns >= 3 {
                        view {
                            style: "width: 26px; flex-shrink: 0; flex-direction: column; gap: 3px; padding: 3px; background-color: {t.surface.sunken.hex()}; border-radius: {t.radius.sm}px; border-width: 1px; border-color: {t.border.default.hex()};",
                            view { style: "width: 100%; height: 8px; background-color: {t.status.accent.subtle.hex()}; border-radius: 2px; border-width: 1px; border-color: {t.status.accent.border.hex()};" }
                            view { style: "width: 100%; height: 8px; background-color: {subtle}; border-radius: 2px;" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn WorkbenchView(is_mobile: bool) -> Element {
    nve_dioxus::render_probe::record_render("WorkbenchView");
    let store = use_studio();
    let s = use_studio_style();
    let wb = store.workbench.read();
    let (state, preview) = (wb.state.clone(), wb.preview());
    drop(wb);
    let config = store.style.read().config;
    let telemetry = store.sync.read().telemetry.clone();
    let preset = state.preset;
    let show_chrome = state.show_device_chrome;

    let live_tokens_summary = format!(
        "{} | {}px | {} | {} | {} issues | {} docs",
        config.accent.label(),
        config.radius.px(),
        config.density.label(),
        config.layout_engine.label(),
        preview.issue_count,
        preview.doc_count
    );
    let host_metrics_base = (telemetry.format_kbps(), telemetry.format_compression());
    let panel_card = |extra: &str| {
        format!(
            "width: 100%; flex-shrink: 0; {extra} background-color: {}; border-radius: {}px; {}",
            s.t.surface.base.hex(),
            s.m.button_radius,
            s.card()
        )
    };
    let viewport_grid_dir = if is_mobile { "column" } else { "row" };
    let host_chunk_size = if is_mobile { 1 } else { 3 };
    let frames = [
        (
            preset.includes_desktop(),
            "wb-frame-desktop",
            "Desktop",
            "1280x800",
            "3-col",
            3u8,
        ),
        (
            preset.includes_tablet(),
            "wb-frame-tablet",
            "Tablet",
            "820x1180",
            "2-col",
            2,
        ),
        (
            preset.includes_mobile(),
            "wb-frame-mobile",
            "Mobile",
            "390x844",
            "1-col",
            1,
        ),
    ];

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("wb-controls".into()),
            padding: Some(8.0),
            gap: Some(5.0),
            style: Some(panel_card("padding: 8px; gap: 5px;")),
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 4px; align-items: center; {s.row()}",
                view {
                    style: "flex: 1;",
                    TabsRoot {
                        value: Some(preset.as_str().to_string()),
                        id_prefix: Some("wb-preset".into()),
                        style: Some("width: 100%; gap: 0px;".into()),
                        on_change: move |next: String| {
                            let p = parse_preset(&next);
                            store.update(|st| st.set_workbench_preset(p));
                        },
                        TabList {
                            accessibility_label: Some("Workbench viewport preset".into()),
                            style: Some("width: 100%; flex-direction: row; gap: 4px; padding: 0px; background-color: transparent; border-width: 0px; border-color: transparent; border-radius: 0px;".into()),
                            for (p, tid, lbl) in [
                                (WorkbenchPreset::All, "wb-preset-all", "All (3)"),
                                (WorkbenchPreset::Desktop, "wb-preset-desktop", "Desktop"),
                                (WorkbenchPreset::Tablet, "wb-preset-tablet", "Tablet"),
                                (WorkbenchPreset::Mobile, "wb-preset-mobile", "Mobile"),
                            ] {
                                TabTrigger {
                                    key: "{tid}",
                                    value: p.as_str().to_string(),
                                    test_id: Some(tid.into()),
                                    style: Some(s.compact_control("flex: 1; height: 24px;", 2, s.m.pill_radius, Some(preset == p))),
                                    "{lbl}"
                                }
                            }
                        }
                    }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("wb-toggle-chrome".into()),
                    style: Some(s.compact_control("width: 56px; height: 24px; min-height: 24px;", 2, s.m.pill_radius, Some(show_chrome))),
                    on_press: move |_| store.update(|st| st.toggle_workbench_chrome()),
                    "Chrome"
                }
            }
            text {
                test_id: "wb-live-tokens",
                style: "color: {s.t.text.tertiary.hex()}; font-size: 10px;",
                "{live_tokens_summary}"
            }
        }

        scrollview {
            style: "width: 100%; flex: 1; flex-direction: column; gap: {s.m.list_gap}px; {s.row()}",
            view {
                test_id: "wb-viewport-grid",
                nve_table: data_attrs([("workbench-preset", preset.as_str().to_owned())]),
                style: "width: 100%; flex-shrink: 0; flex-direction: {viewport_grid_dir}; gap: 6px;",
                for (shown, test_id, title, dims, layout_tag, columns) in frames {
                    if shown {
                        PreviewFrame {
                            key: "{test_id}",
                            test_id,
                            title,
                            dims,
                            layout_tag,
                            columns,
                            show_chrome,
                            preview: preview.clone(),
                            config,
                        }
                    }
                }
            }

            Card {
                variant: CardVariant::Outlined,
                test_id: Some("wb-telemetry-banner".into()),
                padding: Some(7.0),
                gap: Some(6.0),
                style: Some(panel_card("flex-direction: row; justify-content: space-between; align-items: center; gap: 6px; padding: 7px;")),
                view {
                    style: "flex: 1; flex-direction: column; gap: 2px;",
                    text { style: "color: {s.t.text.primary.hex()}; font-size: 10px;", "CONNECTED HOST SURFACES (9)" }
                    text { style: "color: {s.t.text.link.hex()}; font-size: 9px;", "{telemetry_summary(&telemetry)}" }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("wb-sync-cycle".into()),
                    style: Some(s.compact_control(
                        &format!("min-width: {}px; height: 22px; min-height: 22px;", badge_min_w(telemetry.transport.label())),
                        4,
                        s.m.pill_radius,
                        Some(true),
                    )),
                    on_press: move |_| store.update(|st| st.cycle_sync_transport()),
                    "{telemetry.transport.label()}"
                }
            }

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
                                let is_sel = state.selected_host_slug == slug;
                                let rtt = if telemetry.rtt_ms_tenths == 0 { 0 } else { telemetry.rtt_ms_tenths + spec.rtt_offset_tenths };
                                let host_metrics = format!("{} | {} | {}", host_metrics_base.0, host_metrics_base.1, format_tenths_ms(rtt));
                                let bg = if is_sel { s.t.surface.sunken.hex() } else { s.t.surface.raised.hex() };
                                rsx! {
                                    pressable {
                                        key: "wb-h-{slug}",
                                        test_id: "wb-host-{slug}",
                                        style: "flex: 1; flex-direction: column; padding: 7px; gap: 3px; background-color: {bg}; border-radius: {s.m.button_radius}px; {s.control(is_sel)}",
                                        onpress: move |_| store.update(|st| st.select_workbench_host(slug)),
                                        view {
                                            style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                                            text { style: "color: {s.t.text.primary.hex()}; font-size: 10px;", "{spec.name}" }
                                            Badge {
                                                label: "LIVE".to_string(),
                                                variant: BadgeVariant::Success,
                                                style: Some(s.compact_control("min-width: 34px; height: 15px;", 2, s.t.radius.sm as u32, None)),
                                            }
                                        }
                                        text { style: "color: {s.t.text.tertiary.hex()}; font-size: 9px;", "{spec.pipeline}" }
                                        text { style: "color: {s.t.text.secondary.hex()}; font-size: 9px;", "{host_metrics}" }
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
