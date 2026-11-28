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
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, TabList, TabTrigger,
    TabsRoot,
};

use crate::state::{HOST_SURFACES, StudioConfig, SyncTelemetryState, WorkbenchPreset};
use crate::theme::badge_min_w;
use crate::ui::store::{WorkbenchPreview, use_studio};
use crate::ui::style::{
    StudioStyleExt, badge_dims, data_attrs, fixed_dims, min_w_dims, row_between, text_style,
    use_studio_style,
};

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
    let preview_radius = s.m.card_radius.min(14) as f32;
    let inner_pad = if s.m.card_pad <= 8 { 5.0 } else { 7.0 };
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
        badge_dims(badge_min_w(dims) as f32, 16.0),
        3,
        s.m.pill_radius,
        Some(true),
    );
    let sidebar_bar = TypedStyle::new()
        .width_pct(100.0)
        .height_px(6.0)
        .bg(t.surface.subtle)
        .radius(2.0);

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some(test_id.into()),
            padding: Some(inner_pad),
            gap: Some(5.0),
            nve_style: Some(
                TypedStyle::new()
                    .flex_1()
                    .padding(inner_pad)
                    .gap(5.0)
                    .bg(t.surface.base)
                    .radius(preview_radius)
                    .outline(s.card()),
            ),
            if show_chrome {
                view {
                    nve_style: row_between(),
                    text { nve_style: text_style(t.text.primary, 10.0), "{title}" }
                    Badge { label: dims.to_string(), variant: BadgeVariant::Neutral, nve_style: Some(dims_badge_style) }
                }
            }
            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_shrink(0.0)
                    .flex_col()
                    .padding(6.0)
                    .gap(5.0)
                    .bg(t.surface.canvas)
                    .radius(s.m.button_radius as f32)
                    .outline(s.control(true)),
                view {
                    nve_style: row_between()
                        .height_px(18.0)
                        .padding_x(5.0)
                        .bg(t.surface.base)
                        .radius(t.radius.sm),
                    text { nve_style: text_style(t.text.link, 9.0), "AETHER" }
                    text { nve_style: text_style(t.text.tertiary, 8.0), "{layout_tag}" }
                }
                view {
                    nve_style: TypedStyle::new().width_pct(100.0).flex_shrink(0.0).flex_row().gap(4.0),
                    if columns >= 2 {
                        view {
                            nve_style: TypedStyle::new()
                                .width_px(28.0)
                                .flex_shrink(0.0)
                                .flex_col()
                                .gap(3.0)
                                .padding(3.0)
                                .bg(t.surface.base)
                                .radius(t.radius.sm),
                            view {
                                nve_style: TypedStyle::new()
                                    .width_pct(100.0)
                                    .height_px(6.0)
                                    .bg(t.status.accent.solid)
                                    .radius(2.0),
                            }
                            view { nve_style: sidebar_bar.clone() }
                            view { nve_style: sidebar_bar }
                        }
                    }
                    view {
                        nve_style: TypedStyle::new()
                            .flex_1()
                            .flex_col()
                            .gap(3.0)
                            .padding(4.0)
                            .bg(t.surface.raised)
                            .radius(s.m.pill_radius as f32)
                            .border(1.0, t.border.default),
                        text { nve_style: text_style(t.text.primary, 9.0), "{preview.top_task_key} | {preview.active_doc_slug}" }
                        text { nve_style: text_style(t.text.secondary, 8.0), "{metrics_line}" }
                        text { nve_style: text_style(t.text.link, 8.0), "{token_line}" }
                    }
                    if columns >= 3 {
                        view {
                            nve_style: TypedStyle::new()
                                .width_px(26.0)
                                .flex_shrink(0.0)
                                .flex_col()
                                .gap(3.0)
                                .padding(3.0)
                                .bg(t.surface.sunken)
                                .radius(t.radius.sm)
                                .border(1.0, t.border.default),
                            view {
                                nve_style: TypedStyle::new()
                                    .width_pct(100.0)
                                    .height_px(8.0)
                                    .bg(t.status.accent.subtle)
                                    .radius(2.0)
                                    .border(1.0, t.status.accent.border),
                            }
                            view {
                                nve_style: TypedStyle::new()
                                    .width_pct(100.0)
                                    .height_px(8.0)
                                    .bg(t.surface.subtle)
                                    .radius(2.0),
                            }
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
    let viewport_grid_dir = if is_mobile {
        FlexDirection::Column
    } else {
        FlexDirection::Row
    };
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
            nve_style: Some(s.card_surface(8, 5, s.t.surface.base, s.m.button_radius)),
            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_shrink(0.0)
                    .flex_row()
                    .gap(4.0)
                    .align_items(FlexAlign::Center)
                    .outline(s.row()),
                view {
                    nve_style: TypedStyle::new().flex_1(),
                    TabsRoot {
                        value: Some(preset.as_str().to_string()),
                        id_prefix: Some("wb-preset".into()),
                        nve_style: Some(TypedStyle::new().width_pct(100.0).gap(0.0)),
                        on_change: move |next: String| {
                            let p = parse_preset(&next);
                            store.update(|st| st.set_workbench_preset(p));
                        },
                        TabList {
                            accessibility_label: Some("Workbench viewport preset".into()),
                            nve_style: Some(
                                TypedStyle::new()
                                    .width_pct(100.0)
                                    .flex_row()
                                    .gap(4.0)
                                    .padding(0.0)
                                    .bg(ColorRgba::TRANSPARENT)
                                    .border(0.0, ColorRgba::TRANSPARENT)
                                    .radius(0.0),
                            ),
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
                                    nve_style: Some(s.compact_control(
                                        TypedStyle::new().flex_1().height_px(24.0),
                                        2,
                                        s.m.pill_radius,
                                        Some(preset == p),
                                    )),
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
                    nve_style: Some(s.compact_control(fixed_dims(56.0, 24.0), 2, s.m.pill_radius, Some(show_chrome))),
                    on_press: move |_| store.update(|st| st.toggle_workbench_chrome()),
                    "Chrome"
                }
            }
            text {
                test_id: "wb-live-tokens",
                nve_style: text_style(s.t.text.tertiary, 10.0),
                "{live_tokens_summary}"
            }
        }

        scrollview {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_1()
                .flex_col()
                .gap(s.m.list_gap as f32)
                .outline(s.row()),
            view {
                test_id: "wb-viewport-grid",
                nve_table: data_attrs([("workbench-preset", preset.as_str().to_owned())]),
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_shrink(0.0)
                    .flex_direction(viewport_grid_dir)
                    .gap(6.0),
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
                nve_style: Some(
                    s.card_surface(7, 6, s.t.surface.base, s.m.button_radius)
                        .flex_row()
                        .justify_content(FlexJustify::SpaceBetween)
                        .align_items(FlexAlign::Center),
                ),
                view {
                    nve_style: TypedStyle::new().flex_1().flex_col().gap(2.0),
                    text { nve_style: text_style(s.t.text.primary, 10.0), "CONNECTED HOST SURFACES (9)" }
                    text { nve_style: text_style(s.t.text.link, 9.0), "{telemetry_summary(&telemetry)}" }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("wb-sync-cycle".into()),
                    nve_style: Some(s.compact_control(
                        min_w_dims(badge_min_w(telemetry.transport.label()) as f32, 22.0),
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
                nve_style: TypedStyle::new().width_pct(100.0).flex_shrink(0.0).flex_col().gap(6.0),
                for (row_idx, chunk) in HOST_SURFACES.chunks(host_chunk_size).enumerate() {
                    view {
                        key: "wb-host-row-{row_idx}",
                        nve_style: TypedStyle::new().width_pct(100.0).flex_shrink(0.0).flex_row().gap(6.0),
                        for spec in chunk.iter() {
                            {
                                let slug = spec.slug;
                                let is_sel = state.selected_host_slug == slug;
                                let rtt = if telemetry.rtt_ms_tenths == 0 { 0 } else { telemetry.rtt_ms_tenths + spec.rtt_offset_tenths };
                                let host_metrics = format!("{} | {} | {}", host_metrics_base.0, host_metrics_base.1, format_tenths_ms(rtt));
                                let bg = if is_sel { s.t.surface.sunken } else { s.t.surface.raised };
                                rsx! {
                                    pressable {
                                        key: "wb-h-{slug}",
                                        test_id: "wb-host-{slug}",
                                        nve_style: TypedStyle::new()
                                            .flex_1()
                                            .flex_col()
                                            .padding(7.0)
                                            .gap(3.0)
                                            .bg(bg)
                                            .radius(s.m.button_radius as f32)
                                            .outline(s.control(is_sel)),
                                        onpress: move |_| store.update(|st| st.select_workbench_host(slug)),
                                        view {
                                            nve_style: TypedStyle::new()
                                                .width_pct(100.0)
                                                .flex_row()
                                                .justify_content(FlexJustify::SpaceBetween)
                                                .align_items(FlexAlign::Center),
                                            text { nve_style: text_style(s.t.text.primary, 10.0), "{spec.name}" }
                                            Badge {
                                                label: "LIVE".to_string(),
                                                variant: BadgeVariant::Success,
                                                nve_style: Some(s.compact_control(
                                                    badge_dims(34.0, 15.0),
                                                    2,
                                                    s.t.radius.sm as u32,
                                                    None,
                                                )),
                                            }
                                        }
                                        text { nve_style: text_style(s.t.text.tertiary, 9.0), "{spec.pipeline}" }
                                        text { nve_style: text_style(s.t.text.secondary, 9.0), "{host_metrics}" }
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
