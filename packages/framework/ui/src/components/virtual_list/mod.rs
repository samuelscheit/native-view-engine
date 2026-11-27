//! High-performance virtualized list engine (`HeightsRevision`, `FenwickHeightTree`, `RowHeights`,
//! `VirtualListState`, and [`VirtualList`]) supporting 100,000+ uniform or variable-height rows
//! in strictly O(log N) prefix-sum/lower-bound time and O(viewport) rendered DOM nodes.

mod engine;
mod fenwick;
mod state;

pub use engine::{HeightsRevision, RowHeights};
pub use fenwick::FenwickHeightTree;
pub use state::{ScrollAlign, VirtualListState, VirtualSlice};

use dioxus_core::{Callback, EventHandler};
use dioxus_core_macro::Props;
use dioxus_signals::{CopyValue, ReadSignal};
use nve_abi::{ColorRgba, FlexAlign};
use nve_dioxus::prelude::*;

use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle};
use crate::theme::use_theme;
use engine::{clamp_scroll, sanitize_extent};
use state::compute_handle_key;

/// Lightweight scalar view state stored inside [`VirtualList`]. Never holds a `RowHeights` clone,
/// keeping a parent-owned `Signal<RowHeights>` at `Arc::strong_count == 1` so in-place
/// `set_row_height` updates perform zero copy-on-write allocations.
#[derive(Debug, Clone, Copy, PartialEq)]
struct VirtualViewState {
    scroll_offset: f64,
    selected_index: Option<usize>,
}

/// Non-reactive geometry sync snapshot tracked in a `CopyValue` so height/viewport change checks
/// never trigger redundant subscriber notifications during render.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GeometrySyncState {
    last_revision: HeightsRevision,
    last_len: usize,
    last_viewport: f64,
    last_overscan: usize,
}

/// High-performance virtualized scroll list rendering only the visible slice plus overscan,
/// framed by `top_spacer` and `bottom_spacer` views and wired for keyboard navigation.
///
/// Accepts either a parent `Signal<RowHeights>` (zero-copy reactive subscription with in-place
/// `set_row_height` updates) or a plain `RowHeights` value via [`ReadSignal<RowHeights>`].
#[component]
pub fn VirtualList(
    heights: ReadSignal<RowHeights>,
    viewport_height: f32,
    render_row: Callback<(usize, bool), Element>,
    #[props(default = 3)] overscan: usize,
    #[props(default)] initial_scroll_offset: f32,
    #[props(default)] initial_selected: Option<usize>,
    #[props(default)] on_select: Option<EventHandler<usize>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let theme = use_theme();
    let sanitized_vp = sanitize_extent(f64::from(viewport_height));

    let mut view_state = use_signal(|| {
        let h = heights.peek();
        let len = h.len();
        VirtualViewState {
            scroll_offset: clamp_scroll(
                f64::from(initial_scroll_offset),
                h.total_height(),
                sanitized_vp,
            ),
            selected_index: initial_selected.and_then(|sel| (len > 0).then(|| sel.min(len - 1))),
        }
    });

    let mut sync_state = dioxus_core::use_hook(|| {
        let h = heights.peek();
        CopyValue::new(GeometrySyncState {
            last_revision: h.revision(),
            last_len: h.len(),
            last_viewport: sanitized_vp,
            last_overscan: overscan,
        })
    });

    let base_id = test_id.as_deref().unwrap_or("nve-virtual-list");
    let top_spacer_id = format!("{base_id}-top-spacer");
    let bottom_spacer_id = format!("{base_id}-bottom-spacer");

    let (slice, rows) = {
        // Subscribe to parent `ReadSignal<RowHeights>` without cloning `RowHeights` or `Arc`.
        let h = heights.read();
        let cur_rev = h.revision();
        let cur_len = h.len();
        let total_h = h.total_height();

        let (scroll_offset, selected_index) = {
            let cur_vs = *view_state.read();
            let mut sync = sync_state.write();
            let changed = sync.last_revision != cur_rev
                || sync.last_len != cur_len
                || (sync.last_viewport - sanitized_vp).abs() > f64::EPSILON
                || sync.last_overscan != overscan;
            if changed {
                sync.last_revision = cur_rev;
                sync.last_len = cur_len;
                sync.last_viewport = sanitized_vp;
                sync.last_overscan = overscan;

                let clamped_scroll = clamp_scroll(cur_vs.scroll_offset, total_h, sanitized_vp);
                let clamped_sel = cur_vs
                    .selected_index
                    .and_then(|sel| (cur_len > 0).then(|| sel.min(cur_len - 1)));
                if (clamped_scroll - cur_vs.scroll_offset).abs() > f64::EPSILON
                    || clamped_sel != cur_vs.selected_index
                {
                    let mut vs = view_state.write();
                    vs.scroll_offset = clamped_scroll;
                    vs.selected_index = clamped_sel;
                }
                (clamped_scroll, clamped_sel)
            } else {
                (cur_vs.scroll_offset, cur_vs.selected_index)
            }
        };

        let slice = h.slice(scroll_offset, sanitized_vp, overscan);
        let mut rows = Vec::with_capacity(slice.visible_count);

        for idx in slice.start..slice.end {
            let is_selected = selected_index == Some(idx);
            let row_h = h.row_height(idx) as f32;
            let row_tid = format!("{base_id}-row-{idx}");
            let aria_selected = if is_selected { "true" } else { "false" };

            let row_style = TypedStyleBuilder::new()
                .width_pct(100.0)
                .height_px(row_h)
                .flex_shrink(0.0)
                .flex_row()
                .align_items(FlexAlign::Center)
                .surface(TypedSurfaceStyle {
                    background: if is_selected {
                        theme.surface.subtle
                    } else {
                        ColorRgba::TRANSPARENT
                    },
                    border_width: if is_selected { 1.0 } else { 0.0 },
                    border_color: if is_selected {
                        theme.border.focus
                    } else {
                        ColorRgba::TRANSPARENT
                    },
                    border_radius: 0.0,
                    opacity: 1.0,
                    box_shadow: "none",
                })
                .finish();

            let content = render_row.call((idx, is_selected));
            rows.push(rsx! {
                pressable {
                    key: "{idx}",
                    test_id: "{row_tid}",
                    accessibility_role: "listitem",
                    aria_selected: "{aria_selected}",
                    tab_index: if is_selected { 0 } else { -1 },
                    nve_style: row_style,
                    onpress: move |_| {
                        if view_state.peek().selected_index != Some(idx) {
                            view_state.write().selected_index = Some(idx);
                        }
                        if let Some(handler) = on_select {
                            handler.call(idx);
                        }
                    },
                    {content}
                }
            });
        }

        (slice, rows)
    };

    let scroll_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_px(sanitized_vp.max(1.0) as f32)
        .flex_col()
        .overflow("hidden")
        .surface(TypedSurfaceStyle {
            background: theme.surface.base,
            border_width: 1.0,
            border_color: theme.border.default,
            border_radius: theme.radius.md,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let top_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_px(slice.top_spacer_px as f32)
        .flex_shrink(0.0)
        .finish();

    let bottom_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_px(slice.bottom_spacer_px as f32)
        .flex_shrink(0.0)
        .finish();

    rsx! {
        scrollview {
            test_id: "{base_id}",
            accessibility_role: "list",
            accessibility_label: accessibility_label,
            tab_index: 0,
            nve_style: scroll_style,
            style: style,
            onscroll: move |ev: ScrollEvent| {
                let y = f64::from(ev.data().y);
                let next = {
                    let h = heights.peek();
                    clamp_scroll(y, h.total_height(), sanitized_vp)
                };
                if (view_state.peek().scroll_offset - next).abs() > f64::EPSILON {
                    view_state.write().scroll_offset = next;
                }
            },
            onkeydown: move |ev: KeyEvent| {
                let outcome = {
                    let cur_vs = *view_state.peek();
                    let h = heights.peek();
                    compute_handle_key(
                        &h,
                        cur_vs.scroll_offset,
                        sanitized_vp,
                        cur_vs.selected_index,
                        ev.data().key.as_str(),
                    )
                };
                if let Some((next_idx, next_scroll)) = outcome {
                    {
                        let mut vs = view_state.write();
                        vs.selected_index = Some(next_idx);
                        vs.scroll_offset = next_scroll;
                    }
                    if let Some(handler) = on_select {
                        handler.call(next_idx);
                    }
                }
            },
            view {
                test_id: "{top_spacer_id}",
                nve_style: top_style,
            }
            {rows.into_iter()}
            view {
                test_id: "{bottom_spacer_id}",
                nve_style: bottom_style,
            }
        }
    }
}
