//! Value-type layout/visual snapshots and visibility helpers over `NodeStore`.

use nve_abi::ColorRgba;
use nve_tables::snapshots::{LayoutSnapshot, VisualSnapshot};
use nve_tables::tables::{Layout as NveLayout, Typography, box_visual};
use nve_store::{Length, NodeId, NodeStore};

#[inline]
pub fn length_to_dim(l: Length) -> nve_abi::Dimension {
    if l.is_points() {
        nve_abi::Dimension::Points(l.value())
    } else if l.is_percent() {
        nve_abi::Dimension::Percent(l.value())
    } else {
        nve_abi::Dimension::Auto
    }
}

/// Extracts a value-type [`LayoutSnapshot`] from `store` for `id`.
pub fn layout_snapshot_of(store: &NodeStore, id: NodeId) -> LayoutSnapshot {
    let Some(l) = store.try_row::<NveLayout>(id) else {
        return LayoutSnapshot::default();
    };
    let px = |len: Length| if len.is_points() { len.value() } else { 0.0 };
    LayoutSnapshot {
        width: length_to_dim(l.width()),
        height: length_to_dim(l.height()),
        min_width: length_to_dim(l.min_width()),
        min_height: length_to_dim(l.min_height()),
        max_width: length_to_dim(l.max_width()),
        max_height: length_to_dim(l.max_height()),
        flex_direction: l.flex_direction(),
        justify_content: l.justify_content().into(),
        align_items: l.align_items().into(),
        align_self: l.align_self(),
        flex_grow: l.flex_grow(),
        flex_shrink: l.flex_shrink(),
        gap: px(l.row_gap()).max(px(l.column_gap())),
        padding: nve_abi::EdgeInsets {
            top: px(l.padding_top()),
            right: px(l.padding_right()),
            bottom: px(l.padding_bottom()),
            left: px(l.padding_left()),
        },
        margin: nve_abi::EdgeInsets {
            top: px(l.margin_top()),
            right: px(l.margin_right()),
            bottom: px(l.margin_bottom()),
            left: px(l.margin_left()),
        },
    }
}

/// Extracts a value-type [`VisualSnapshot`] from `store` for `id`.
pub fn visual_snapshot_of(store: &NodeStore, id: NodeId) -> VisualSnapshot {
    let mut snap = VisualSnapshot::default();
    if let Some(v) = box_visual(store, id) {
        snap.background = ColorRgba::from_u32(v.background().0);
        snap.border_color = ColorRgba::from_u32(v.border_color().0);
        snap.border_width = v.border_width();
        snap.border_radius = v.border_radius();
        snap.opacity = v.opacity();
        snap.z_index = v.z_index();
    }
    if let Some(t) = store.try_row::<Typography>(id) {
        snap.text_color = ColorRgba::from_u32(t.color().0);
        snap.font_size = t.font_size();
    }
    snap
}
