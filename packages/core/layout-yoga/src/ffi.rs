//! Raw FFI declarations for the `c_api/yoga_shim.cpp` wrapper around real React Native Yoga.

use std::ffi::c_void;

use nve_abi::{Dimension, FlexAlign, FlexDirection, FlexJustify, LayoutProps};

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
#[derive(Debug, Clone, Copy)]
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

fn encode_dim(d: Dimension) -> (u8, f32) {
    match d {
        Dimension::Auto => (0, 0.0),
        Dimension::Points(v) => (1, v),
        Dimension::Percent(v) => (2, v),
    }
}

impl From<&LayoutProps> for NveYogaLayoutProps {
    fn from(l: &LayoutProps) -> Self {
        let (width_tag, width_val) = encode_dim(l.width);
        let (height_tag, height_val) = encode_dim(l.height);
        let (min_width_tag, min_width_val) = encode_dim(l.min_width);
        let (min_height_tag, min_height_val) = encode_dim(l.min_height);
        let (max_width_tag, max_width_val) = encode_dim(l.max_width);
        let (max_height_tag, max_height_val) = encode_dim(l.max_height);
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
            flex_direction: match l.flex_direction {
                FlexDirection::Column => 0,
                FlexDirection::Row => 1,
                FlexDirection::ColumnReverse => 2,
                FlexDirection::RowReverse => 3,
            },
            justify_content: match l.justify_content {
                FlexJustify::FlexStart => 0,
                FlexJustify::Center => 1,
                FlexJustify::FlexEnd => 2,
                FlexJustify::SpaceBetween => 3,
                FlexJustify::SpaceAround => 4,
                FlexJustify::SpaceEvenly => 5,
            },
            align_items: match l.align_items {
                FlexAlign::Stretch => 0,
                FlexAlign::FlexStart => 1,
                FlexAlign::Center => 2,
                FlexAlign::FlexEnd => 3,
                FlexAlign::Baseline => 4,
            },
            _reserved: 0,
            flex_grow: l.flex_grow,
            flex_shrink: l.flex_shrink,
            gap: l.gap,
            pad_top: l.padding.top,
            pad_right: l.padding.right,
            pad_bottom: l.padding.bottom,
            pad_left: l.padding.left,
            margin_top: l.margin.top,
            margin_right: l.margin.right,
            margin_bottom: l.margin.bottom,
            margin_left: l.margin.left,
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
    pub fn nve_yg_node_set_fixed_size(node: YGNodeRef, width: f32, height: f32);
    pub fn nve_yg_node_calculate_layout(root: YGNodeRef, avail_width: f32, avail_height: f32);
    pub fn nve_yg_node_get_layout_rect(node: YGNodeRef) -> NveYogaRect;
}
