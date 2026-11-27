//! Raw FFI declarations for the `c_api/yoga_shim.cpp` wrapper around real React Native Yoga.

use std::ffi::c_void;

use nve_tables::tables::{
    AlignItems, Display, FlexDirection, JustifyContent, LayoutDirection, LayoutRef, Position,
};
use nve_store::{Length, LengthKind};

#[repr(C)]
pub struct YGConfig {
    _private: [u8; 0],
}

#[repr(C)]
pub struct YGNode {
    _private: [u8; 0],
}

pub type YGConfigRef = *mut YGConfig;
pub type YGNodeRef = *mut YGNode;

pub type NveYogaMeasureCb = extern "C" fn(
    user_data: *mut c_void,
    width: f32,
    width_mode: u8,
    height: f32,
    height_mode: u8,
    out_width: *mut f32,
    out_height: *mut f32,
);

#[repr(C)]
pub struct NveYogaMeasureHook {
    pub callback: NveYogaMeasureCb,
    pub user_data: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NveYogaLayoutProps {
    pub width_tag: u8,
    pub width_val: f32,
    pub height_tag: u8,
    pub height_val: f32,
    pub min_width_tag: u8,
    pub min_width_val: f32,
    pub min_height_tag: u8,
    pub min_height_val: f32,
    pub max_width_tag: u8,
    pub max_width_val: f32,
    pub max_height_tag: u8,
    pub max_height_val: f32,
    pub flex_direction: u8,
    pub justify_content: u8,
    pub align_items: u8,
    pub _reserved: u8,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub gap: f32,
    pub pad_top: f32,
    pub pad_right: f32,
    pub pad_bottom: f32,
    pub pad_left: f32,
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NveYogaRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NveYogaLayoutExt {
    pub display: u8,
    pub flex_basis_tag: u8,
    pub flex_basis_val: f32,
    pub has_aspect_ratio: u8,
    pub aspect_ratio: f32,
    pub position_mode: u8,
    pub top_tag: u8,
    pub top_val: f32,
    pub right_tag: u8,
    pub right_val: f32,
    pub bottom_tag: u8,
    pub bottom_val: f32,
    pub left_tag: u8,
    pub left_val: f32,
    pub direction: u8,
    pub has_pad_start: u8,
    pub pad_start: f32,
    pub has_pad_end: u8,
    pub pad_end: f32,
    pub has_mar_start: u8,
    pub mar_start: f32,
    pub has_mar_end: u8,
    pub mar_end: f32,
}

#[inline]
fn encode_len(l: Length) -> (u8, f32) {
    match l.kind() {
        LengthKind::Auto => (0, 0.0),
        LengthKind::Px => (1, l.value()),
        LengthKind::Percent => (2, l.value()),
    }
}

#[inline]
fn len_px(l: Length) -> f32 {
    match l.kind() {
        LengthKind::Px => l.value(),
        _ => 0.0,
    }
}

impl NveYogaLayoutProps {
    pub fn from_store_layout(l: &LayoutRef<'_>) -> Self {
        let (width_tag, width_val) = encode_len(l.width());
        let (height_tag, height_val) = encode_len(l.height());
        let (min_width_tag, min_width_val) = encode_len(l.min_width());
        let (min_height_tag, min_height_val) = encode_len(l.min_height());
        let (max_width_tag, max_width_val) = encode_len(l.max_width());
        let (max_height_tag, max_height_val) = encode_len(l.max_height());
        let pad_inline = l.padding_inline_logical();
        let mar_inline = l.margin_inline_logical();
        Self {
            width_tag,
            width_val,
            height_tag,
            height_val,
            min_width_tag,
            min_width_val,
            min_height_tag,
            min_height_val,
            max_width_tag,
            max_width_val,
            max_height_tag,
            max_height_val,
            flex_direction: match l.flex_direction() {
                FlexDirection::Column => 0,
                FlexDirection::Row => 1,
                FlexDirection::ColumnReverse => 2,
                FlexDirection::RowReverse => 3,
            },
            justify_content: match l.justify_content() {
                JustifyContent::FlexStart => 0,
                JustifyContent::Center => 1,
                JustifyContent::FlexEnd => 2,
                JustifyContent::SpaceBetween => 3,
                JustifyContent::SpaceAround => 4,
                JustifyContent::SpaceEvenly => 5,
            },
            align_items: match l.align_items() {
                AlignItems::Stretch => 0,
                AlignItems::FlexStart => 1,
                AlignItems::Center => 2,
                AlignItems::FlexEnd => 3,
                AlignItems::Baseline => 4,
            },
            _reserved: 0,
            flex_grow: l.flex_grow(),
            flex_shrink: l.flex_shrink(),
            gap: len_px(l.row_gap()).max(len_px(l.column_gap())),
            pad_top: len_px(l.padding_top()),
            pad_right: if pad_inline { 0.0 } else { len_px(l.padding_right()) },
            pad_bottom: len_px(l.padding_bottom()),
            pad_left: if pad_inline { 0.0 } else { len_px(l.padding_left()) },
            margin_top: len_px(l.margin_top()),
            margin_right: if mar_inline { 0.0 } else { len_px(l.margin_right()) },
            margin_bottom: len_px(l.margin_bottom()),
            margin_left: if mar_inline { 0.0 } else { len_px(l.margin_left()) },
        }
    }
}

impl NveYogaLayoutExt {
    pub fn from_store_layout(l: &LayoutRef<'_>) -> Self {
        let (flex_basis_tag, flex_basis_val) = encode_len(l.flex_basis());
        let (top_tag, top_val) = encode_len(l.inset_top());
        let (right_tag, right_val) = encode_len(l.inset_right());
        let (bottom_tag, bottom_val) = encode_len(l.inset_bottom());
        let (left_tag, left_val) = encode_len(l.inset_left());
        let ar = l.aspect_ratio().filter(|&r| r.is_finite() && r > 0.0);
        let pad_inline = l.padding_inline_logical();
        let mar_inline = l.margin_inline_logical();
        Self {
            display: u8::from(l.display() == Display::None),
            flex_basis_tag,
            flex_basis_val,
            has_aspect_ratio: u8::from(ar.is_some()),
            aspect_ratio: ar.unwrap_or(0.0),
            position_mode: match l.position() {
                Position::Relative => 0,
                Position::Absolute => 1,
                Position::Sticky => 2,
            },
            top_tag,
            top_val,
            right_tag,
            right_val,
            bottom_tag,
            bottom_val,
            left_tag,
            left_val,
            direction: match l.direction() {
                LayoutDirection::Inherit => 0,
                LayoutDirection::Ltr => 1,
                LayoutDirection::Rtl => 2,
            },
            has_pad_start: u8::from(pad_inline),
            pad_start: if pad_inline { len_px(l.padding_left()) } else { 0.0 },
            has_pad_end: u8::from(pad_inline),
            pad_end: if pad_inline { len_px(l.padding_right()) } else { 0.0 },
            has_mar_start: u8::from(mar_inline),
            mar_start: if mar_inline { len_px(l.margin_left()) } else { 0.0 },
            has_mar_end: u8::from(mar_inline),
            mar_end: if mar_inline { len_px(l.margin_right()) } else { 0.0 },
        }
    }
}

unsafe extern "C" {
    pub fn nve_yg_config_new() -> YGConfigRef;
    pub fn nve_yg_config_free(config: YGConfigRef);
    pub fn nve_yg_config_set_point_scale_factor(config: YGConfigRef, scale: f32);
    pub fn nve_yg_node_new_with_config(config: YGConfigRef) -> YGNodeRef;
    pub fn nve_yg_node_detach_and_clear(node: YGNodeRef);
    pub fn nve_yg_node_free(node: YGNodeRef);
    pub fn nve_yg_node_set_is_text(node: YGNodeRef, is_text: bool);
    pub fn nve_yg_node_set_measure_hook(node: YGNodeRef, hook: *mut NveYogaMeasureHook);
    pub fn nve_yg_node_mark_dirty(node: YGNodeRef);
    pub fn nve_yg_node_set_children(
        parent: YGNodeRef,
        children: *const YGNodeRef,
        count: usize,
    );
    pub fn nve_yg_node_apply_layout(node: YGNodeRef, props: *const NveYogaLayoutProps);
    pub fn nve_yg_node_apply_layout_ext(node: YGNodeRef, ext: *const NveYogaLayoutExt);
    pub fn nve_yg_node_set_fixed_size(node: YGNodeRef, width: f32, height: f32);
    pub fn nve_yg_node_calculate_layout(root: YGNodeRef, avail_width: f32, avail_height: f32);
    pub fn nve_yg_node_get_layout_rect(node: YGNodeRef) -> NveYogaRect;
}
