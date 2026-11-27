//! Value-type layout/visual snapshots and visibility/overflow helpers over `NodeStore`.

use nve_abi::ColorRgba;
use nve_tables::snapshots::{LayoutSnapshot, VisualSnapshot};
use nve_tables::tables::{
    is_node_display_none, Layout as NveLayout, Overflow, Typography, Visual,
};
use nve_store::{Length, NodeId, NodeStore};

#[inline]
pub(crate) fn is_display_none(store: &NodeStore, id: NodeId) -> bool {
    is_node_display_none(store, id)
}

#[inline]
pub(crate) fn node_clips_overflow(store: &NodeStore, id: NodeId) -> bool {
    store.try_row::<Visual>(id).is_some_and(|v| {
        matches!(
            v.overflow_x(),
            Overflow::Hidden | Overflow::Scroll | Overflow::Auto
        ) || matches!(
            v.overflow_y(),
            Overflow::Hidden | Overflow::Scroll | Overflow::Auto
        )
    })
}

#[inline]
pub(crate) fn length_to_dim(l: Length) -> nve_abi::Dimension {
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
        flex_direction: match l.flex_direction() {
            nve_tables::tables::FlexDirection::Column => nve_abi::FlexDirection::Column,
            nve_tables::tables::FlexDirection::Row => nve_abi::FlexDirection::Row,
            nve_tables::tables::FlexDirection::ColumnReverse => {
                nve_abi::FlexDirection::ColumnReverse
            }
            nve_tables::tables::FlexDirection::RowReverse => nve_abi::FlexDirection::RowReverse,
        },
        justify_content: match l.justify_content() {
            nve_tables::tables::JustifyContent::FlexStart => nve_abi::FlexJustify::FlexStart,
            nve_tables::tables::JustifyContent::Center => nve_abi::FlexJustify::Center,
            nve_tables::tables::JustifyContent::FlexEnd => nve_abi::FlexJustify::FlexEnd,
            nve_tables::tables::JustifyContent::SpaceBetween => {
                nve_abi::FlexJustify::SpaceBetween
            }
            nve_tables::tables::JustifyContent::SpaceAround => {
                nve_abi::FlexJustify::SpaceAround
            }
            nve_tables::tables::JustifyContent::SpaceEvenly => {
                nve_abi::FlexJustify::SpaceEvenly
            }
        },
        align_items: match l.align_items() {
            nve_tables::tables::AlignItems::Stretch => nve_abi::FlexAlign::Stretch,
            nve_tables::tables::AlignItems::FlexStart => nve_abi::FlexAlign::FlexStart,
            nve_tables::tables::AlignItems::Center => nve_abi::FlexAlign::Center,
            nve_tables::tables::AlignItems::FlexEnd => nve_abi::FlexAlign::FlexEnd,
            nve_tables::tables::AlignItems::Baseline => nve_abi::FlexAlign::Baseline,
        },
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
    if let Some(v) = store.try_row::<Visual>(id) {
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
