//! Raw FFI declarations for the `c_api/yoga_shim.cpp` wrapper around real React Native Yoga.

use std::ffi::c_void;

use nve_tables::tables::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, Direction, Display, FlexDirection, FlexWrap,
    JustifyContent, LayoutRef, Overflow, Position, VisualRef,
};
use nve_store::Length;

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
    pub row_gap: f32,
    pub column_gap: f32,
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
    pub flex_wrap: u8,
    pub align_content: u8,
    pub align_self: u8,
    pub box_sizing: u8,
    pub overflow: u8,
    pub border_width: f32,
}

#[inline]
fn encode_len(l: Length) -> (u8, f32) {
    if l.is_points() {
        (1, l.value())
    } else if l.is_percent() {
        (2, l.value())
    } else {
        (0, 0.0)
    }
}

#[inline]
fn len_px(l: Length) -> f32 {
    if l.is_points() { l.value() } else { 0.0 }
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
            row_gap: len_px(l.row_gap()),
            column_gap: len_px(l.column_gap()),
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
    pub fn from_store(l: Option<&LayoutRef<'_>>, v: Option<&VisualRef<'_>>) -> Self {
        let overflow = v.map_or(0, |vis| {
            let map = |o: Overflow| match o {
                Overflow::Visible => 0u8,
                Overflow::Hidden => 1u8,
                Overflow::Scroll | Overflow::Auto => 2u8,
            };
            map(vis.overflow_x()).max(map(vis.overflow_y()))
        });
        let bw = v.map_or(0.0, |vis| vis.border_width());
        let border_width = if bw.is_finite() && bw > 0.0 { bw } else { 0.0 };
        let Some(l) = l else {
            return Self {
                overflow,
                border_width,
                ..Default::default()
            };
        };
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
                // Yoga has no `position: fixed` and this adapter does not hoist: a fixed box is
                // laid out as absolute against its parent, which matches CSS only when that
                // parent is the layout root (known gap, see `docs/quality-audit.md`).
                Position::Absolute | Position::Fixed => 1,
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
                Direction::Inherit => 0,
                Direction::Ltr => 1,
                Direction::Rtl => 2,
            },
            has_pad_start: u8::from(pad_inline),
            pad_start: if pad_inline { len_px(l.padding_left()) } else { 0.0 },
            has_pad_end: u8::from(pad_inline),
            pad_end: if pad_inline { len_px(l.padding_right()) } else { 0.0 },
            has_mar_start: u8::from(mar_inline),
            mar_start: if mar_inline { len_px(l.margin_left()) } else { 0.0 },
            has_mar_end: u8::from(mar_inline),
            mar_end: if mar_inline { len_px(l.margin_right()) } else { 0.0 },
            flex_wrap: match l.flex_wrap() {
                FlexWrap::NoWrap => 0,
                FlexWrap::Wrap => 1,
                FlexWrap::WrapReverse => 2,
            },
            align_content: match l.align_content() {
                AlignContent::Stretch => 0,
                AlignContent::FlexStart => 1,
                AlignContent::Center => 2,
                AlignContent::FlexEnd => 3,
                AlignContent::SpaceBetween => 4,
                AlignContent::SpaceAround => 5,
                AlignContent::SpaceEvenly => 6,
            },
            align_self: match l.align_self() {
                AlignSelf::Auto => 0,
                AlignSelf::Stretch => 1,
                AlignSelf::FlexStart => 2,
                AlignSelf::Center => 3,
                AlignSelf::FlexEnd => 4,
                AlignSelf::Baseline => 5,
            },
            box_sizing: match l.box_sizing() {
                BoxSizing::BorderBox => 0,
                BoxSizing::ContentBox => 1,
            },
            overflow,
            border_width,
        }
    }
}

const _: () = {
    use core::mem::{align_of, offset_of, size_of};

    assert!(size_of::<NveYogaMeasureHook>() == 2 * size_of::<usize>());
    assert!(offset_of!(NveYogaMeasureHook, callback) == 0);
    assert!(offset_of!(NveYogaMeasureHook, user_data) == size_of::<usize>());

    assert!(size_of::<NveYogaRect>() == 16);
    assert!(align_of::<NveYogaRect>() == 4);
    assert!(offset_of!(NveYogaRect, x) == 0);
    assert!(offset_of!(NveYogaRect, y) == 4);
    assert!(offset_of!(NveYogaRect, width) == 8);
    assert!(offset_of!(NveYogaRect, height) == 12);

    assert!(size_of::<NveYogaLayoutProps>() == 100);
    assert!(align_of::<NveYogaLayoutProps>() == 4);
    assert!(offset_of!(NveYogaLayoutProps, width_tag) == 0);
    assert!(offset_of!(NveYogaLayoutProps, width_val) == 4);
    assert!(offset_of!(NveYogaLayoutProps, height_tag) == 8);
    assert!(offset_of!(NveYogaLayoutProps, height_val) == 12);
    assert!(offset_of!(NveYogaLayoutProps, min_width_tag) == 16);
    assert!(offset_of!(NveYogaLayoutProps, min_width_val) == 20);
    assert!(offset_of!(NveYogaLayoutProps, min_height_tag) == 24);
    assert!(offset_of!(NveYogaLayoutProps, min_height_val) == 28);
    assert!(offset_of!(NveYogaLayoutProps, max_width_tag) == 32);
    assert!(offset_of!(NveYogaLayoutProps, max_width_val) == 36);
    assert!(offset_of!(NveYogaLayoutProps, max_height_tag) == 40);
    assert!(offset_of!(NveYogaLayoutProps, max_height_val) == 44);
    assert!(offset_of!(NveYogaLayoutProps, flex_direction) == 48);
    assert!(offset_of!(NveYogaLayoutProps, justify_content) == 49);
    assert!(offset_of!(NveYogaLayoutProps, align_items) == 50);
    assert!(offset_of!(NveYogaLayoutProps, _reserved) == 51);
    assert!(offset_of!(NveYogaLayoutProps, flex_grow) == 52);
    assert!(offset_of!(NveYogaLayoutProps, flex_shrink) == 56);
    assert!(offset_of!(NveYogaLayoutProps, row_gap) == 60);
    assert!(offset_of!(NveYogaLayoutProps, column_gap) == 64);
    assert!(offset_of!(NveYogaLayoutProps, pad_top) == 68);
    assert!(offset_of!(NveYogaLayoutProps, pad_right) == 72);
    assert!(offset_of!(NveYogaLayoutProps, pad_bottom) == 76);
    assert!(offset_of!(NveYogaLayoutProps, pad_left) == 80);
    assert!(offset_of!(NveYogaLayoutProps, margin_top) == 84);
    assert!(offset_of!(NveYogaLayoutProps, margin_right) == 88);
    assert!(offset_of!(NveYogaLayoutProps, margin_bottom) == 92);
    assert!(offset_of!(NveYogaLayoutProps, margin_left) == 96);

    assert!(size_of::<NveYogaLayoutExt>() == 92);
    assert!(align_of::<NveYogaLayoutExt>() == 4);
    assert!(offset_of!(NveYogaLayoutExt, display) == 0);
    assert!(offset_of!(NveYogaLayoutExt, flex_basis_tag) == 1);
    assert!(offset_of!(NveYogaLayoutExt, flex_basis_val) == 4);
    assert!(offset_of!(NveYogaLayoutExt, has_aspect_ratio) == 8);
    assert!(offset_of!(NveYogaLayoutExt, aspect_ratio) == 12);
    assert!(offset_of!(NveYogaLayoutExt, position_mode) == 16);
    assert!(offset_of!(NveYogaLayoutExt, top_tag) == 17);
    assert!(offset_of!(NveYogaLayoutExt, top_val) == 20);
    assert!(offset_of!(NveYogaLayoutExt, right_tag) == 24);
    assert!(offset_of!(NveYogaLayoutExt, right_val) == 28);
    assert!(offset_of!(NveYogaLayoutExt, bottom_tag) == 32);
    assert!(offset_of!(NveYogaLayoutExt, bottom_val) == 36);
    assert!(offset_of!(NveYogaLayoutExt, left_tag) == 40);
    assert!(offset_of!(NveYogaLayoutExt, left_val) == 44);
    assert!(offset_of!(NveYogaLayoutExt, direction) == 48);
    assert!(offset_of!(NveYogaLayoutExt, has_pad_start) == 49);
    assert!(offset_of!(NveYogaLayoutExt, pad_start) == 52);
    assert!(offset_of!(NveYogaLayoutExt, has_pad_end) == 56);
    assert!(offset_of!(NveYogaLayoutExt, pad_end) == 60);
    assert!(offset_of!(NveYogaLayoutExt, has_mar_start) == 64);
    assert!(offset_of!(NveYogaLayoutExt, mar_start) == 68);
    assert!(offset_of!(NveYogaLayoutExt, has_mar_end) == 72);
    assert!(offset_of!(NveYogaLayoutExt, mar_end) == 76);
    assert!(offset_of!(NveYogaLayoutExt, flex_wrap) == 80);
    assert!(offset_of!(NveYogaLayoutExt, align_content) == 81);
    assert!(offset_of!(NveYogaLayoutExt, align_self) == 82);
    assert!(offset_of!(NveYogaLayoutExt, box_sizing) == 83);
    assert!(offset_of!(NveYogaLayoutExt, overflow) == 84);
    assert!(offset_of!(NveYogaLayoutExt, border_width) == 88);
};

unsafe extern "C" {
    pub fn nve_yg_config_new() -> YGConfigRef;
    pub fn nve_yg_config_free(config: YGConfigRef);
    pub fn nve_yg_config_set_point_scale_factor(config: YGConfigRef, scale: f32);
    pub fn nve_yg_node_new_with_config(config: YGConfigRef) -> YGNodeRef;
    pub fn nve_yg_node_detach_and_clear(node: YGNodeRef);
    pub fn nve_yg_node_free(node: YGNodeRef);
    pub fn nve_yg_node_set_measure_hook(node: YGNodeRef, hook: *mut NveYogaMeasureHook) -> bool;
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
    pub fn nve_yg_node_measure_layout(
        root: YGNodeRef,
        min_w: f32,
        max_w: f32,
        min_h: f32,
        max_h: f32,
        ideal: u8,
    ) -> NveYogaRect;
    pub fn nve_yg_node_get_layout_rect(node: YGNodeRef) -> NveYogaRect;
}
