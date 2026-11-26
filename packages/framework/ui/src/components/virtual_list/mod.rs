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
use nve_abi::{ColorRgba, FlexAlign};
use nve_dioxus::prelude::*;

use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle};
use crate::theme::use_theme_rgba;

/// High-performance virtualized scroll list rendering only the visible slice plus overscan,
/// framed by `top_spacer` and `bottom_spacer` views and wired for keyboard navigation.
#[component]
pub fn VirtualList(
    heights: RowHeights,
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
    let (theme, colors) = use_theme_rgba();
    let mut state = use_signal(|| {
        let mut st = VirtualListState::new(heights.clone(), viewport_height, overscan);
        st.set_scroll_offset(initial_scroll_offset);
        st.selected_index = initial_selected;
        st
    });

    let sanitized_vp = if viewport_height.is_finite() && viewport_height > 0.0 {
        viewport_height
    } else {
        0.0
    };

    {
        let mut st = state.write_unchecked();
        let mut geometry_changed = false;
        if st.heights.revision() != heights.revision() || st.heights != heights {
            st.heights = heights;
            geometry_changed = true;
        }
        if (st.viewport_height - sanitized_vp).abs() > f32::EPSILON {
            st.viewport_height = sanitized_vp;
            geometry_changed = true;
        }
        if geometry_changed {
            let cur = st.scroll_offset;
            st.set_scroll_offset(cur);
        }
        st.overscan = overscan;
    }

    let base_id = test_id.as_deref().unwrap_or("nve-virtual-list");
    let top_spacer_id = format!("{base_id}-top-spacer");
    let bottom_spacer_id = format!("{base_id}-bottom-spacer");

    let (slice, rows) = {
        let st = state.read();
        let slice = st.slice();
        let selected_index = st.selected_index;
        let mut rows = Vec::with_capacity(slice.visible_count);

        for idx in slice.start..slice.end {
            let is_selected = selected_index == Some(idx);
            let row_h = st.heights.row_height(idx);
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
                        colors.surface_subtle
                    } else {
                        ColorRgba::TRANSPARENT
                    },
                    border_width: if is_selected { 1.0 } else { 0.0 },
                    border_color: if is_selected {
                        colors.border_focus
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
                        state.write().selected_index = Some(idx);
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
        .height_px(sanitized_vp.max(1.0))
        .flex_col()
        .overflow("hidden")
        .surface(TypedSurfaceStyle {
            background: colors.surface,
            border_width: 1.0,
            border_color: colors.border,
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let top_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_px(slice.top_spacer_px)
        .flex_shrink(0.0)
        .finish();

    let bottom_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_px(slice.bottom_spacer_px)
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
                let y = ev.data().y;
                state.write().set_scroll_offset(y);
            },
            onkeydown: move |ev: KeyEvent| {
                if let Some(next) = state.write().handle_key(ev.data().key.as_str())
                    && let Some(handler) = on_select
                {
                    handler.call(next);
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
