//! Virtualized 100,000-row Stream view (`ActiveView::Stream100k`) rendered in O(viewport).
//!
//! `Stream100kPanel` subscribes to the `stream` domain only and is backed by a cached
//! Fenwick-tree [`StreamVirtualEngine`] with recycled viewport slots (`vslot-{slot_idx}`), so
//! 100,000-row jumps and scroll events reconcile ONLY the `~13` visible rows without re-rendering
//! the sidebar or Studio panel.

use std::cell::RefCell;
use std::rc::Rc;

use nve_dioxus::prelude::*;
use nve_ui::ColorHex;

use crate::ui::store::use_studio;
use crate::ui::style::{data_attrs, use_studio_style};
use crate::virtual_list::{StreamVirtualEngine, DEFAULT_STREAM_ROW_COUNT};

#[component]
pub(crate) fn Stream100kPanel(is_mobile: bool) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let density = store.studio.read().config.density;
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
        (win, items, eng.offset_for_row(50_000), eng.offset_for_row(99_999), step)
    };

    let mode_lbl = if stream.variable_heights { "Fenwick VarH" } else { "O(1) FixedH" };
    let last = window.end.saturating_sub(1);
    let readout_text = if is_mobile {
        format!("#{}-#{last} / 100k | {} rows | {mode_lbl}", window.start, window.visible_count)
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
    let max_scroll_y = window.total_height_px.saturating_sub(viewport_h as u64).min(u32::MAX as u64) as u32;
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
    let vh_ol = s.control(var_h);
    let accent_ol = s.control(true);
    let neutral_ol = s.control(false);
    let row_ol = s.row();
    let pill_r = format!("{}px", s.m.pill_radius);
    let btn_r = format!("{}px", s.m.button_radius);
    let card = s.card();
    let (accent_bg, accent_ink) = (t.status.accent.subtle.hex(), t.text.link.hex());
    let (subtle_bg, base_bg) = (t.surface.subtle.hex(), t.surface.base.hex());
    let (primary, secondary, tertiary) = (t.text.primary.hex(), t.text.secondary.hex(), t.text.tertiary.hex());

    rsx! {
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px;",
            border_width: "{row_ol.width}px",
            border_color: "{row_ol.color.hex()}",
            pressable {
                test_id: "stream-jump-top",
                style: "flex: 1; height: 26px; justify-content: center; align-items: center; background-color: {base_bg}; {neutral_ol}",
                border_radius: "{pill_r}",
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(0)),
                text { style: "color: {primary}; font-size: 10px;", "#0" }
            }
            pressable {
                test_id: "stream-jump-50k",
                style: "flex: 1; height: 26px; justify-content: center; align-items: center; border-width: 1px;",
                background_color: "{accent_bg}",
                border_color: "{accent_ol.color.hex()}",
                border_radius: "{pill_r}",
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(offset_50k)),
                text { style: "font-size: 10px;", color: "{accent_ink}", "#50k" }
            }
            pressable {
                test_id: "stream-jump-end",
                style: "flex: 1; height: 26px; justify-content: center; align-items: center; border-width: 1px;",
                background_color: "{accent_bg}",
                border_color: "{accent_ol.color.hex()}",
                border_radius: "{pill_r}",
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(offset_end)),
                text { style: "font-size: 10px;", color: "{accent_ink}", "#99,999" }
            }
            pressable {
                test_id: "stream-page-up",
                style: "width: 34px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {subtle_bg}; {neutral_ol}",
                border_radius: "{pill_r}",
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(page_up_y)),
                text { style: "color: {secondary}; font-size: 10px;", "Up" }
            }
            pressable {
                test_id: "stream-page-down",
                style: "width: 34px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {subtle_bg}; {neutral_ol}",
                border_radius: "{pill_r}",
                onpress: move |_| store.update(|st| st.set_stream_scroll_offset(page_down_y)),
                text { style: "color: {secondary}; font-size: 10px;", "Dn" }
            }
            pressable {
                test_id: "stream-toggle-heights",
                style: "width: 48px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center;",
                background_color: "{s.control_bg(var_h)}",
                border_radius: "{pill_r}",
                border_width: "{vh_ol.width}px",
                border_color: "{vh_ol.color.hex()}",
                onpress: move |_| store.update(|st| st.toggle_stream_variable_heights()),
                text {
                    style: "font-size: 10px;",
                    color: if var_h { "{accent_ink}" } else { "{secondary}" },
                    if var_h { "Var H" } else { "Fix H" }
                }
            }
        }

        view {
            test_id: "stream-window-readout",
            style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 9px; padding-vertical: 5px; background-color: {base_bg};",
            border_radius: "{btn_r}",
            border_width: "{card.width}px",
            border_color: "{card.color.hex()}",
            text { style: "color: {secondary}; font-size: 10px;", "{readout_text}" }
        }

        // O(viewport) recycled-slot container: mounts ONLY `[start..end)` with exact 1:1 row
        // pitch (`gap: 0px`) and zero node churn across 100,000-row jumps.
        scrollview {
            test_id: "stream-viewport",
            nve_table: viewport_attrs,
            style: "width: 100%; flex: 1; flex-direction: column; gap: 0px;",
            border_width: "{row_ol.width}px",
            border_color: "{row_ol.color.hex()}",
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
                            style: "width: 100%; flex-shrink: 0; flex-direction: column; justify-content: center; padding-horizontal: 9px; gap: 2px; border-width: 1px;",
                            height: "{row.height_px}px",
                            background_color: "{row_bg.hex()}",
                            border_radius: "{pill_r}",
                            border_color: "{rbc.hex()}",
                            onpress: move |_| store.update(|st| st.toggle_stream_row_expanded(row_idx)),
                            view {
                                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                                text { style: "color: {primary}; font-size: 10px;", "{summary_lbl}" }
                                text { style: "color: {tertiary}; font-size: 9px;", "{meta_lbl}" }
                            }
                            if row.is_expanded {
                                text { style: "color: {secondary}; font-size: 10px;", "{row.detail}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
