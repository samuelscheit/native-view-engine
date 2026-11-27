//! Value-type layout/visual snapshots and visibility helpers over `NodeStore`. Box-generation
//! predicates (`display: contents`, box paint, clipping) live in `nve_components::tables` so
//! every backend shares them.

use nve_abi::ColorRgba;
use nve_components::style_patch::{LayoutSnapshot, VisualSnapshot};
use nve_components::tables::{Layout as NveLayout, Typography, box_visual, is_node_display_none};
use nve_store::{Length, NodeId as StoreNodeId, NodeStore};

#[inline]
pub(crate) fn is_display_none(store: &NodeStore, id: StoreNodeId) -> bool {
    is_node_display_none(store, id)
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
pub fn layout_snapshot_of(store: &NodeStore, id: StoreNodeId) -> LayoutSnapshot {
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
            nve_components::tables::FlexDirection::Column => nve_abi::FlexDirection::Column,
            nve_components::tables::FlexDirection::Row => nve_abi::FlexDirection::Row,
            nve_components::tables::FlexDirection::ColumnReverse => {
                nve_abi::FlexDirection::ColumnReverse
            }
            nve_components::tables::FlexDirection::RowReverse => nve_abi::FlexDirection::RowReverse,
        },
        justify_content: match l.justify_content() {
            nve_components::tables::JustifyContent::FlexStart => nve_abi::FlexJustify::FlexStart,
            nve_components::tables::JustifyContent::Center => nve_abi::FlexJustify::Center,
            nve_components::tables::JustifyContent::FlexEnd => nve_abi::FlexJustify::FlexEnd,
            nve_components::tables::JustifyContent::SpaceBetween => {
                nve_abi::FlexJustify::SpaceBetween
            }
            nve_components::tables::JustifyContent::SpaceAround => {
                nve_abi::FlexJustify::SpaceAround
            }
            nve_components::tables::JustifyContent::SpaceEvenly => {
                nve_abi::FlexJustify::SpaceEvenly
            }
        },
        align_items: match l.align_items() {
            nve_components::tables::AlignItems::Stretch => nve_abi::FlexAlign::Stretch,
            nve_components::tables::AlignItems::FlexStart => nve_abi::FlexAlign::FlexStart,
            nve_components::tables::AlignItems::Center => nve_abi::FlexAlign::Center,
            nve_components::tables::AlignItems::FlexEnd => nve_abi::FlexAlign::FlexEnd,
            nve_components::tables::AlignItems::Baseline => nve_abi::FlexAlign::Baseline,
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
pub fn visual_snapshot_of(store: &NodeStore, id: StoreNodeId) -> VisualSnapshot {
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
