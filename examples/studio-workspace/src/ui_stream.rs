//! Virtualized 100,000-row Stream view (`ActiveView::Stream100k`) rendered in O(viewport).
//!
//! Isolated into its own Dioxus `#[component]` (`Stream100kPanel`) with a local `Signal<u64>`,
//! backed by a cached Fenwick-tree [`StreamVirtualEngine`] and recycled viewport slots
//! (`vslot-{slot_idx}`), so 100,000-row jumps and scroll events reconcile ONLY the `~13` visible
//! rows without re-rendering the sidebar or Studio panel.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

use crate::state::{AccentTheme, DensityMode, WorkspaceState};
use crate::theme::{
    card_border, control_border, row_border, CARD_ALT_BG, CARD_BG, CONTROL_BG, TEXT_SECONDARY,
    XRAY_ROW,
};
use crate::virtual_list::{StreamVirtualEngine, DEFAULT_STREAM_ROW_COUNT};

#[derive(Clone)]
pub(crate) struct SharedStore(pub Arc<Mutex<WorkspaceState>>);

impl PartialEq for SharedStore {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

fn bump_local(
    store: &Arc<Mutex<WorkspaceState>>,
    mut local_rev: Signal<u64>,
    f: impl FnOnce(&mut WorkspaceState),
) {
    if let Ok(mut guard) = store.lock() {
        f(&mut guard);
    }
    let next = local_rev().wrapping_add(1);
    local_rev.set(next);
}

#[component]
pub(crate) fn Stream100kPanel(
    store: SharedStore,
    accent: AccentTheme,
    density: DensityMode,
    xray: bool,
    pill_radius: u32,
    button_radius: u32,
    is_mobile: bool,
    parent_rev: u64,
) -> Element {
    let _ = parent_rev;
    let engine_cell = dioxus_core::use_hook(|| {
        let guard = store.0.lock().unwrap_or_else(|e| e.into_inner());
        Rc::new(RefCell::new(StreamVirtualEngine::new(
            DEFAULT_STREAM_ROW_COUNT,
            guard.current.studio.density,
            guard.current.stream.variable_heights,
            &guard.current.stream.expanded_rows,
        )))
    });
    let local_rev = use_signal(|| 0u64);
    let _rev = local_rev();

    let stream = {
        let guard = store.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.current.stream.clone()
    };

    let accent_hex = accent.hex();
    let accent_tint = accent.tint_bg();
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);

    let viewport_h = stream.viewport_height.max(180) as f64;

    let (window, rows_meta, offset_50k, offset_end, page_step) = {
        let mut eng = engine_cell.borrow_mut();
        eng.sync_config(density, stream.variable_heights, &stream.expanded_rows);
        let win = eng.slice(stream.scroll_offset_y as f64, viewport_h, stream.overscan);
        let mut items = Vec::with_capacity(win.visible_count);
        for idx in win.start..win.end {
            items.push(eng.row_meta(idx));
        }
        let off_50k = eng.offset_for_row(50_000);
        let off_end = eng.offset_for_row(99_999);
        let step = (viewport_h * 0.85).round().max(120.0) as u32;
        (win, items, off_50k, off_end, step)
    };

    let mode_lbl = if stream.variable_heights {
        "Fenwick VarH"
    } else {
        "O(1) FixedH"
    };
    let readout_text = if is_mobile {
        format!(
            "#{}-#{} / 100k | {} rows | {}",
            window.start,
            window.end.saturating_sub(1),
            window.visible_count,
            mode_lbl
        )
    } else {
        format!(
            "Rows #{}..#{} / 100,000 | Mounted: {} | Y={}px / {}px | {}",
            window.start,
            window.end.saturating_sub(1),
            window.visible_count,
            window.clamped_scroll_offset as u64,
            window.total_height_px,
            mode_lbl
        )
    };

    let current_scroll_y = window.clamped_scroll_offset as u32;
    let max_scroll_y = window
        .total_height_px
        .saturating_sub(viewport_h as u64)
        .min(u32::MAX as u64) as u32;
    let page_up_y = current_scroll_y.saturating_sub(page_step);
    let page_down_y = current_scroll_y.saturating_add(page_step).min(max_scroll_y);

    let top_spacer_attr = format!("{}", window.top_spacer_px);
    let bottom_spacer_attr = format!("{}", window.bottom_spacer_px);
    let total_height_attr = format!("{}", window.total_height_px);
    let mounted_count_attr = format!("{}", window.visible_count);
    let scroll_y_attr = format!("{current_scroll_y}");

    let (vh_bw, vh_bc) = control_border(xray, stream.variable_heights, accent_hex);
    let vh_bg = if stream.variable_heights {
        accent_tint
    } else {
        CONTROL_BG
    };
    let vh_fg = if stream.variable_heights {
        accent_hex
    } else {
        TEXT_SECONDARY
    };
    let vh_btn_label = if stream.variable_heights {
        "Var H"
    } else {
        "Fix H"
    };

    let s_top = Arc::clone(&store.0);
    let s_50k = Arc::clone(&store.0);
    let s_end = Arc::clone(&store.0);
    let s_up = Arc::clone(&store.0);
    let s_down = Arc::clone(&store.0);
    let s_var = Arc::clone(&store.0);
    let pill_r = format!("{pill_radius}px");
    let btn_r = format!("{button_radius}px");

    rsx! {
        // Quick-Jump & Virtual Mode Toolbar
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px;",
            border_width: "{row_bw}px",
            border_color: "{row_bc}",
            pressable {
                test_id: "stream-jump-top",
                style: "flex: 1; height: 26px; justify-content: center; align-items: center; background-color: #111827; border-width: 1px; border-color: #334155;",
                border_radius: "{pill_r}",
                onpress: move |_| bump_local(&s_top, local_rev, |st| st.set_stream_scroll_offset(0)),
                text { style: "color: #f8fafc; font-size: 10px;", "#0" }
            }
            pressable {
                test_id: "stream-jump-50k",
                style: "flex: 1; height: 26px; justify-content: center; align-items: center; border-width: 1px;",
                background_color: "{accent_tint}",
                border_color: "{accent_hex}",
                border_radius: "{pill_r}",
                onpress: move |_| bump_local(&s_50k, local_rev, |st| st.set_stream_scroll_offset(offset_50k)),
                text { style: "font-size: 10px;", color: "{accent_hex}", "#50k" }
            }
            pressable {
                test_id: "stream-jump-end",
                style: "flex: 1; height: 26px; justify-content: center; align-items: center; border-width: 1px;",
                background_color: "{accent_tint}",
                border_color: "{accent_hex}",
                border_radius: "{pill_r}",
                onpress: move |_| bump_local(&s_end, local_rev, |st| st.set_stream_scroll_offset(offset_end)),
                text { style: "font-size: 10px;", color: "{accent_hex}", "#99,999" }
            }
            pressable {
                test_id: "stream-page-up",
                style: "width: 34px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: #1f2937; border-width: 1px; border-color: #334155;",
                border_radius: "{pill_r}",
                onpress: move |_| bump_local(&s_up, local_rev, |st| st.set_stream_scroll_offset(page_up_y)),
                text { style: "color: #cbd5e1; font-size: 10px;", "Up" }
            }
            pressable {
                test_id: "stream-page-down",
                style: "width: 34px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: #1f2937; border-width: 1px; border-color: #334155;",
                border_radius: "{pill_r}",
                onpress: move |_| bump_local(&s_down, local_rev, |st| st.set_stream_scroll_offset(page_down_y)),
                text { style: "color: #cbd5e1; font-size: 10px;", "Dn" }
            }
            pressable {
                test_id: "stream-toggle-heights",
                style: "width: 48px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center;",
                background_color: "{vh_bg}",
                border_radius: "{pill_r}",
                border_width: "{vh_bw}px",
                border_color: "{vh_bc}",
                onpress: move |_| bump_local(&s_var, local_rev, |st| st.toggle_stream_variable_heights()),
                text { style: "font-size: 10px;", color: "{vh_fg}", "{vh_btn_label}" }
            }
        }

        // Virtual Window Telemetry Readout
        view {
            test_id: "stream-window-readout",
            style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 9px; padding-vertical: 5px; background-color: #111827;",
            border_radius: "{btn_r}",
            border_width: "{card_bw}px",
            border_color: "{card_bc}",
            text {
                style: "color: #cbd5e1; font-size: 10px;",
                "{readout_text}"
            }
        }

        // O(viewport) Recycled-Slot Virtualized Container — mounts ONLY `[start..end)`
        // with exact 1:1 row pitch (`gap: 0px`) matching `RowHeights` heights and zero
        // node churn across 100,000-row jumps.
        scrollview {
            test_id: "stream-viewport",
            "virtual-top-spacer": "{top_spacer_attr}",
            "virtual-bottom-spacer": "{bottom_spacer_attr}",
            "virtual-total-height": "{total_height_attr}",
            "virtual-rendered-count": "{mounted_count_attr}",
            "virtual-scroll-y": "{scroll_y_attr}",
            style: "width: 100%; flex: 1; flex-direction: column; gap: 0px;",
            border_width: "{row_bw}px",
            border_color: "{row_bc}",
            for (slot_idx, row) in rows_meta.iter().enumerate() {
                {
                    let row_idx = row.index;
                    let row_tid = format!("stream-row-{row_idx}");
                    let summary_lbl = format!(
                        "[{}] STR-{:05} {} {}",
                        row.priority.label(),
                        row.issue_number,
                        row.verb,
                        row.subject
                    );
                    let meta_lbl = if is_mobile {
                        format!("{} | {}px", row.author, row.height_px)
                    } else {
                        format!("{} | {} | {}us | {}px", row.author, row.tag, row.latency_us, row.height_px)
                    };
                    let row_bg = if row.is_expanded {
                        accent_tint
                    } else if row_idx % 2 == 0 {
                        CARD_BG
                    } else {
                        CARD_ALT_BG
                    };
                    let rbc = if xray {
                        XRAY_ROW
                    } else if row.is_expanded {
                        accent_hex
                    } else {
                        "#1e293b"
                    };
                    let row_h = row.height_px;
                    let expanded = row.is_expanded;
                    let detail_str = row.detail;
                    let s_exp = Arc::clone(&store.0);
                    rsx! {
                        pressable {
                            key: "vslot-{slot_idx}",
                            test_id: "{row_tid}",
                            style: "width: 100%; flex-shrink: 0; flex-direction: column; justify-content: center; padding-horizontal: 9px; gap: 2px; border-width: 1px;",
                            height: "{row_h}px",
                            background_color: "{row_bg}",
                            border_radius: "{pill_r}",
                            border_color: "{rbc}",
                            onpress: move |_| bump_local(&s_exp, local_rev, |st| st.toggle_stream_row_expanded(row_idx)),
                            view {
                                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                                text { style: "color: #f8fafc; font-size: 10px;", "{summary_lbl}" }
                                text { style: "color: #94a3b8; font-size: 9px;", "{meta_lbl}" }
                            }
                            if expanded {
                                text {
                                    style: "color: #cbd5e1; font-size: 10px;",
                                    "{detail_str}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
