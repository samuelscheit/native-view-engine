//! Virtualized 100,000-row Stream view (`ActiveView::Stream100k`) rendered in O(viewport).
//!
//! `Stream100kPanel` subscribes to the `stream` domain only and is backed by a cached
//! Fenwick-tree [`StreamVirtualEngine`] with recycled viewport slots (`vslot-{slot_idx}`), so
//! 100,000-row jumps and scroll events reconcile ONLY the `~13` visible rows without re-rendering
//! the sidebar or Studio panel.

use std::cell::RefCell;
use std::rc::Rc;

use nve_dioxus::prelude::*;

use crate::ui::store::use_studio;
use crate::ui::style::{StudioStyleExt, data_attrs, row_between, text_style, use_studio_style};
use crate::virtual_list::{DEFAULT_STREAM_ROW_COUNT, StreamVirtualEngine};

#[component]
pub(crate) fn Stream100kPanel(is_mobile: bool) -> Element {
    nve_dioxus::render_probe::record_render("Stream100kPanel");
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let density = store.style.read().config.density;
    let stream = store.stream.read().state.clone();
    let engine_cell = dioxus_core::use_hook(|| {
        Rc::new(RefCell::new(StreamVirtualEngine::new(
            DEFAULT_STREAM_ROW_COUNT,
            density,
            stream.variable_heights,
            &stream.expanded_rows,
        )))
    });

    let viewport_h = stream.viewport_height.max(180) as f64;
    let (window, rows_meta, offset_50k, offset_end, page_step) = {
        let mut eng = engine_cell.borrow_mut();
        eng.sync_config(density, stream.variable_heights, &stream.expanded_rows);
        let win = eng.slice(stream.scroll_offset_y as f64, viewport_h, stream.overscan);
        let items: Vec<_> = (win.start..win.end).map(|idx| eng.row_meta(idx)).collect();
        let step = (viewport_h * 0.85).round().max(120.0) as u32;
        (
            win,
            items,
            eng.offset_for_row(50_000),
            eng.offset_for_row(99_999),
            step,
        )
    };

    let mode_lbl = if stream.variable_heights {
        "Fenwick VarH"
    } else {
        "O(1) FixedH"
    };
    let last = window.end.saturating_sub(1);
    let readout_text = if is_mobile {
        format!(
            "#{}-#{last} / 100k | {} rows | {mode_lbl}",
            window.start, window.visible_count
        )
    } else {
        format!(
            "Rows #{}..#{last} / 100,000 | Mounted: {} | Y={}px / {}px | {mode_lbl}",
            window.start,
            window.visible_count,
            window.clamped_scroll_offset as u64,
            window.total_height_px
        )
    };

    let current_scroll_y = window.clamped_scroll_offset as u32;
    let max_scroll_y = window
        .total_height_px
        .saturating_sub(viewport_h as u64)
        .min(u32::MAX as u64) as u32;
    let page_up_y = current_scroll_y.saturating_sub(page_step);
    let page_down_y = current_scroll_y.saturating_add(page_step).min(max_scroll_y);
    let viewport_attrs = data_attrs([
        ("virtual-top-spacer", window.top_spacer_px.to_string()),
        ("virtual-bottom-spacer", window.bottom_spacer_px.to_string()),
        ("virtual-total-height", window.total_height_px.to_string()),
        ("virtual-rendered-count", window.visible_count.to_string()),
        ("virtual-scroll-y", current_scroll_y.to_string()),
    ]);

    let var_h = stream.variable_heights;
    let accent_ol = s.control(true);
    let neutral_ol = s.control(false);
    let row_ol = s.row();
    let pill_r = s.m.pill_radius as f32;
    let btn_r = s.m.button_radius as f32;
    let accent_jump_btn = TypedStyle::new()
        .flex_1()
        .height_px(26.0)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .bg(t.status.accent.subtle)
        .radius(pill_r)
        .border(1.0, accent_ol.color);
    let page_btn = TypedStyle::new()
        .width_px(34.0)
        .height_px(26.0)
        .flex_shrink(0.0)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .bg(t.surface.subtle)
        .radius(pill_r)
        .outline(neutral_ol);

    rsx! {
        view {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_shrink(0.0)
                .flex_row()
                .gap(5.0)
                .outline(row_ol),
            pressable {
                test_id: "stream-jump-top",
                nve_style: TypedStyle::new()
                    .flex_1()
                    .height_px(26.0)
                    .justify_content(FlexJustify::Center)
                    .align_items(FlexAlign::Center)
                    .bg(t.surface.base)
                    .radius(pill_r)
                    .outline(neutral_ol),
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(0)),
                text { nve_style: text_style(t.text.primary, 10.0), "#0" }
            }
            pressable {
                test_id: "stream-jump-50k",
                nve_style: accent_jump_btn.clone(),
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(offset_50k)),
                text { nve_style: text_style(t.text.link, 10.0), "#50k" }
            }
            pressable {
                test_id: "stream-jump-end",
                nve_style: accent_jump_btn,
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(offset_end)),
                text { nve_style: text_style(t.text.link, 10.0), "#99,999" }
            }
            pressable {
                test_id: "stream-page-up",
                nve_style: page_btn.clone(),
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(page_up_y)),
                text { nve_style: text_style(t.text.secondary, 10.0), "Up" }
            }
            pressable {
                test_id: "stream-page-down",
                nve_style: page_btn,
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(page_down_y)),
                text { nve_style: text_style(t.text.secondary, 10.0), "Dn" }
            }
            pressable {
                test_id: "stream-toggle-heights",
                nve_style: TypedStyle::new()
                    .width_px(48.0)
                    .height_px(26.0)
                    .flex_shrink(0.0)
                    .justify_content(FlexJustify::Center)
                    .align_items(FlexAlign::Center)
                    .bg(s.control_bg(var_h))
                    .radius(pill_r)
                    .outline(s.control(var_h)),
                onpress: move |_| store.update(|st| st.toggle_stream_variable_heights()),
                text {
                    nve_style: text_style(if var_h { t.text.link } else { t.text.secondary }, 10.0),
                    if var_h { "Var H" } else { "Fix H" }
                }
            }
        }

        view {
            test_id: "stream-window-readout",
            nve_style: row_between()
                .padding_xy(9.0, 5.0)
                .bg(t.surface.base)
                .radius(btn_r)
                .outline(s.card()),
            text { nve_style: text_style(t.text.secondary, 10.0), "{readout_text}" }
        }

        // O(viewport) recycled-slot container: mounts ONLY `[start..end)` with exact 1:1 row
        // pitch (`gap: 0px`) and zero node churn across 100,000-row jumps.
        scrollview {
            test_id: "stream-viewport",
            nve_table: viewport_attrs,
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_1()
                .flex_col()
                .gap(0.0)
                .outline(row_ol),
            for (slot_idx, row) in rows_meta.into_iter().enumerate() {
                {
                    let row_idx = row.index;
                    let summary_lbl = format!("[{}] STR-{:05} {} {}", row.priority.label(), row.issue_number, row.verb, row.subject);
                    let meta_lbl = if is_mobile {
                        format!("{} | {}px", row.author, row.height_px)
                    } else {
                        format!("{} | {} | {}us | {}px", row.author, row.tag, row.latency_us, row.height_px)
                    };
                    let row_bg = if row.is_expanded { t.status.accent.subtle } else { s.stripe(row_idx) };
                    let rbc = if row.is_expanded && !s.xray() { t.status.accent.text } else { s.list_row_border() };
                    rsx! {
                        pressable {
                            key: "vslot-{slot_idx}",
                            test_id: "stream-row-{row_idx}",
                            nve_style: TypedStyle::new()
                                .width_pct(100.0)
                                .height_px(row.height_px as f32)
                                .flex_shrink(0.0)
                                .flex_col()
                                .justify_content(FlexJustify::Center)
                                .padding_x(9.0)
                                .gap(2.0)
                                .bg(row_bg)
                                .radius(pill_r)
                                .border(1.0, rbc),
                            onpress: move |_| store.update(|st| st.toggle_stream_row_expanded(row_idx)),
                            view {
                                nve_style: TypedStyle::new()
                                    .width_pct(100.0)
                                    .flex_row()
                                    .justify_content(FlexJustify::SpaceBetween)
                                    .align_items(FlexAlign::Center),
                                text { nve_style: text_style(t.text.primary, 10.0), "{summary_lbl}" }
                                text { nve_style: text_style(t.text.tertiary, 9.0), "{meta_lbl}" }
                            }
                            if row.is_expanded {
                                text { nve_style: text_style(t.text.secondary, 10.0), "{row.detail}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
