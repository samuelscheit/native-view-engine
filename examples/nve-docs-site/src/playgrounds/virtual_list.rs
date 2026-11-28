//! Interactive `"virtual-list-100k"` demo showcasing 100,000 virtualized rows with O(1) slice
//! math, instant category filtering, jump-to-row controls, and live visible window telemetry.

use dioxus_core::Callback;
use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, RowHeights, VirtualList,
    VirtualListState, use_theme,
};

pub const VIRTUAL_LIST_TOTAL_ROWS: usize = 100_000;
const ROW_HEIGHT_PX: u32 = 34;
const VIEWPORT_HEIGHT_PX: f64 = 204.0;
const OVERSCAN_ROWS: usize = 2;

pub const VIRTUAL_LIST_SOURCE: &str = r#"use nve_dioxus::prelude::*;
use nve_ui::{RowHeights, VirtualList};

#[component]
pub fn VirtualList100kDemo() -> Element {
    let heights = RowHeights::uniform(100_000, 34);
    let render_row = Callback::new(move |(idx, selected): (usize, bool)| {
        rsx! {
            view {
                nve_style: TypedStyle::row().width_pct(100.0).justify(FlexJustify::SpaceBetween).padding_x(10.0),
                text { "Row #{idx:05} — Frame Telemetry Event" }
                text { if selected { "Selected" } else { "0.14ms" } }
            }
        }
    });

    rsx! {
        VirtualList {
            heights: heights,
            viewport_height: 204.0,
            overscan: 2,
            render_row: render_row,
        }
    }
}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowFilter {
    All,
    Critical,
    Layout,
    Sync,
}

impl RowFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All (100,000)",
            Self::Critical => "Critical (25,000)",
            Self::Layout => "Layout (25,000)",
            Self::Sync => "Sync (50,000)",
        }
    }

    fn count(self) -> usize {
        match self {
            Self::All => VIRTUAL_LIST_TOTAL_ROWS,
            Self::Critical => 25_000,
            Self::Layout => 25_000,
            Self::Sync => 50_000,
        }
    }

    fn source_index(self, filtered_idx: usize) -> usize {
        match self {
            Self::All => filtered_idx,
            Self::Critical => filtered_idx * 4,
            Self::Layout => filtered_idx * 4 + 1,
            Self::Sync => filtered_idx * 2 + 1,
        }
    }

    fn tag_for(source_idx: usize) -> (&'static str, &'static str) {
        match source_idx % 4 {
            0 => ("CRIT", "frame-budget"),
            1 => ("LAYT", "taffy-pass"),
            2 => ("SYNC", "crdt-delta"),
            _ => ("DRAW", "skia-flush"),
        }
    }
}

#[component]
pub fn VirtualList100kDemo() -> Element {
    let theme = use_theme();
    let mut filter = use_signal(|| RowFilter::All);
    let mut jump_target = use_signal(|| 0usize);
    let mut jump_epoch = use_signal(|| 0u32);
    let mut selected_row = use_signal(|| Some(0usize));

    let current_filter = filter();
    let total_rows = current_filter.count();
    let clamped_jump = jump_target().min(total_rows.saturating_sub(1));
    let scroll_offset = (clamped_jump as f64) * f64::from(ROW_HEIGHT_PX);
    let heights = RowHeights::uniform(total_rows, ROW_HEIGHT_PX);

    let mut preview_state = VirtualListState::new(heights.clone(), VIEWPORT_HEIGHT_PX, OVERSCAN_ROWS);
    preview_state.set_scroll_offset(scroll_offset);
    let slice = preview_state.slice();

    let accent_color = theme.status.accent.text;
    let text_primary = theme.text.primary;
    let text_muted = theme.text.tertiary;
    let border_color = theme.border.default;

    let render_row = Callback::new(move |(idx, is_selected): (usize, bool)| {
        let src_idx = current_filter.source_index(idx);
        let (kind_tag, subsystem) = RowFilter::tag_for(src_idx);
        let latency_us = 110 + ((src_idx * 37) % 240);
        let row_fg = if is_selected { accent_color } else { text_primary };
        let weight = if is_selected { 700 } else { 500 };

        rsx! {
            view {
                nve_style: TypedStyle::row()
                    .width_pct(100.0)
                    .height_pct(100.0)
                    .justify(FlexJustify::SpaceBetween)
                    .align(FlexAlign::Center)
                    .padding_x(10.0)
                    .border(1.0, border_color),
                view {
                    nve_style: TypedStyle::row().align(FlexAlign::Center).gap(10.0),
                    text {
                        nve_style: TypedStyle::new()
                            .font_size(11.0)
                            .font_weight(700)
                            .color(accent_color),
                        "[{kind_tag}]"
                    }
                    text {
                        nve_style: TypedStyle::new()
                            .font_size(12.0)
                            .font_weight(weight)
                            .color(row_fg),
                        "Row #{src_idx:05} · {subsystem}"
                    }
                }
                text {
                    nve_style: TypedStyle::new().font_size(11.0).color(text_muted),
                    "{latency_us} µs"
                }
            }
        }
    });

    let selected_label = selected_row()
        .map(|r| format!("#{:05}", current_filter.source_index(r.min(total_rows.saturating_sub(1)))))
        .unwrap_or_else(|| "None".to_string());

    let list_key = format!("vl-{:?}-{}", current_filter, jump_epoch());

    rsx! {
        view {
            test_id: "pg-vlist-root",
            nve_style: TypedStyle::col().width_pct(100.0).gap(10.0),

            // Live virtualization telemetry bar
            view {
                test_id: "pg-vlist-telemetry",
                nve_style: TypedStyle::row()
                    .width_pct(100.0)
                    .justify(FlexJustify::SpaceBetween)
                    .align(FlexAlign::Center)
                    .flex_wrap(FlexWrap::Wrap)
                    .gap(8.0),
                view {
                    nve_style: TypedStyle::row()
                        .flex_wrap(FlexWrap::Wrap)
                        .gap(6.0)
                        .align(FlexAlign::Center),
                    Badge {
                        test_id: Some("pg-vlist-total".to_string()),
                        label: format!("Rows: {total_rows}"),
                        variant: BadgeVariant::Accent,
                    }
                    Badge {
                        test_id: Some("pg-vlist-window".to_string()),
                        label: format!("Mounted: {} ({}..{})", slice.visible_count, slice.start, slice.end),
                        variant: BadgeVariant::Success,
                    }
                    Badge {
                        test_id: Some("pg-vlist-selected".to_string()),
                        label: format!("Selected: {selected_label}"),
                        variant: BadgeVariant::Neutral,
                    }
                }
                text {
                    nve_style: TypedStyle::new().font_size(11.0).color(theme.text.tertiary),
                    "O(1) uniform windowing · 34px rows"
                }
            }

            // Filter + Jump controls
            view {
                nve_style: TypedStyle::row()
                    .width_pct(100.0)
                    .justify(FlexJustify::SpaceBetween)
                    .flex_wrap(FlexWrap::Wrap)
                    .gap(8.0),
                view {
                    nve_style: TypedStyle::row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                    for (fid, variant_f) in [
                        ("all", RowFilter::All),
                        ("critical", RowFilter::Critical),
                        ("layout", RowFilter::Layout),
                        ("sync", RowFilter::Sync),
                    ] {
                        Button {
                            key: "flt-{fid}",
                            test_id: Some(format!("pg-vlist-filter-{fid}")),
                            variant: if current_filter == variant_f {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            },
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                filter.set(variant_f);
                                jump_target.set(0);
                                selected_row.set(Some(0));
                                jump_epoch += 1;
                            },
                            "{variant_f.label()}"
                        }
                    }
                }
                view {
                    nve_style: TypedStyle::row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                    for (label, target) in [
                        ("Top", 0usize),
                        ("#25k", 25_000usize),
                        ("#75k", 75_000usize),
                        ("End", 99_999usize),
                    ] {
                        Button {
                            key: "jmp-{target}",
                            test_id: Some(format!("pg-vlist-jump-{target}")),
                            variant: ButtonVariant::Outline,
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                let clamped = target.min(filter().count().saturating_sub(1));
                                jump_target.set(clamped);
                                selected_row.set(Some(clamped));
                                jump_epoch += 1;
                            },
                            "Jump {label}"
                        }
                    }
                }
            }

            // Real nve_ui::VirtualList (keyed by filter + jump epoch so jump buttons re-center)
            for lk in [list_key] {
                VirtualList {
                    key: "{lk}",
                    test_id: Some("pg-vlist-list".to_string()),
                    accessibility_label: Some("100,000 row virtualized telemetry list".to_string()),
                    heights: heights.clone(),
                    viewport_height: VIEWPORT_HEIGHT_PX as f32,
                    overscan: OVERSCAN_ROWS,
                    initial_scroll_offset: scroll_offset as f32,
                    initial_selected: selected_row(),
                    on_select: move |idx| selected_row.set(Some(idx)),
                    render_row: render_row,
                }
            }
        }
    }
}
