//! Zero-copy Pre-Rendered Frame-0 Scene Format (`SceneRelocView<'a>` / `encode_scene_reloc`).
//!
//! Stores a complete, already-laid-out Frame-0 render & layout tree inside a single `NVERELOC`
//! container (`RELOC_KIND_SCENE`):
//! - [`RelocSceneRoot`] (64 bytes, viewport + root index + parallel node & layout slabs)
//! - [`RelocSceneNode`] (128 bytes, pre-computed layout rect + style + text/tag/image/shadow/draw_list/children)
//! - [`RelocLayoutProps`] (80 bytes, lossless `nve_abi::LayoutProps` for re-layout on resize)
//!
//! Any native backend (`SkiaBackend`, `HeadlessBackend`, C++ `nve-fabric-bridge`) can map and paint
//! a pre-baked `frame0.nvereloc` in microseconds with zero allocations before `VirtualDom` boots.

use std::mem::{align_of, offset_of, size_of};

use nve_abi::{
    ColorRgba, ComponentKind, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify,
    FrameId, LayoutProps, NodeId, StyleProps, SurfaceId,
};

use crate::mmap_arena::ArenaPod;
use crate::{LayoutRect, Viewport};

use super::format::{
    RelocBuf, RelocHeader, RelocSlice, RelocStr, RelocWriter, RELOC_KIND_SCENE,
};
use super::view::RelocView;
use super::{RelocError, VerifyMode};

/// Node clips its children (e.g. `ScrollView` or `overflow: hidden`).
pub const SCENE_NODE_CLIP_CHILDREN: u8 = 1 << 0;
/// Node carries explicit style props (does not inherit parent `Text` color/font_size).
pub const SCENE_NODE_HAS_EXPLICIT_STYLE: u8 = 1 << 1;
/// Node subscribes to interactive pointer/input events (`event_mask != 0`).
pub const SCENE_NODE_INTERACTIVE: u8 = 1 << 2;
/// Node has `display: none` and should be skipped during painting.
pub const SCENE_NODE_HIDDEN: u8 = 1 << 3;

/// Lossless 80-byte, zero-padding `#[repr(C)]` encoding of `nve_abi::LayoutProps`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelocLayoutProps {
    pub width_val: f32,
    pub height_val: f32,
    pub min_width_val: f32,
    pub min_height_val: f32,
    pub max_width_val: f32,
    pub max_height_val: f32,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub gap: f32,
    pub padding: [f32; 4],
    pub margin: [f32; 4],
    pub width_tag: u8,
    pub height_tag: u8,
    pub min_width_tag: u8,
    pub min_height_tag: u8,
    pub max_width_tag: u8,
    pub max_height_tag: u8,
    pub flex_direction: u8,
    pub justify_content: u8,
    pub align_items: u8,
    pub _pad: [u8; 3],
}

impl RelocLayoutProps {
    #[inline]
    fn encode_dim(d: Dimension) -> (u8, f32) {
        match d {
            Dimension::Auto => (0, 0.0),
            Dimension::Points(v) => (1, v),
            Dimension::Percent(v) => (2, v),
        }
    }

    #[inline]
    fn decode_dim(tag: u8, val: f32) -> Dimension {
        match tag {
            1 => Dimension::Points(val),
            2 => Dimension::Percent(val),
            _ => Dimension::Auto,
        }
    }

    pub fn from_abi(lp: &LayoutProps) -> Self {
        let (width_tag, width_val) = Self::encode_dim(lp.width);
        let (height_tag, height_val) = Self::encode_dim(lp.height);
        let (min_width_tag, min_width_val) = Self::encode_dim(lp.min_width);
        let (min_height_tag, min_height_val) = Self::encode_dim(lp.min_height);
        let (max_width_tag, max_width_val) = Self::encode_dim(lp.max_width);
        let (max_height_tag, max_height_val) = Self::encode_dim(lp.max_height);
        Self {
            width_val,
            height_val,
            min_width_val,
            min_height_val,
            max_width_val,
            max_height_val,
            flex_grow: lp.flex_grow,
            flex_shrink: lp.flex_shrink,
            gap: lp.gap,
            padding: [
                lp.padding.top,
                lp.padding.right,
                lp.padding.bottom,
                lp.padding.left,
            ],
            margin: [
                lp.margin.top,
                lp.margin.right,
                lp.margin.bottom,
                lp.margin.left,
            ],
            width_tag,
            height_tag,
            min_width_tag,
            min_height_tag,
            max_width_tag,
            max_height_tag,
            flex_direction: lp.flex_direction as u8,
            justify_content: lp.justify_content as u8,
            align_items: lp.align_items as u8,
            _pad: [0; 3],
        }
    }

    pub fn to_abi(&self) -> LayoutProps {
        let flex_direction = match self.flex_direction {
            1 => FlexDirection::Row,
            2 => FlexDirection::ColumnReverse,
            3 => FlexDirection::RowReverse,
            _ => FlexDirection::Column,
        };
        let justify_content = match self.justify_content {
            1 => FlexJustify::Center,
            2 => FlexJustify::FlexEnd,
            3 => FlexJustify::SpaceBetween,
            4 => FlexJustify::SpaceAround,
            5 => FlexJustify::SpaceEvenly,
            _ => FlexJustify::FlexStart,
        };
        let align_items = match self.align_items {
            1 => FlexAlign::FlexStart,
            2 => FlexAlign::Center,
            3 => FlexAlign::FlexEnd,
            4 => FlexAlign::Baseline,
            _ => FlexAlign::Stretch,
        };
        LayoutProps {
            width: Self::decode_dim(self.width_tag, self.width_val),
            height: Self::decode_dim(self.height_tag, self.height_val),
            min_width: Self::decode_dim(self.min_width_tag, self.min_width_val),
            min_height: Self::decode_dim(self.min_height_tag, self.min_height_val),
            max_width: Self::decode_dim(self.max_width_tag, self.max_width_val),
            max_height: Self::decode_dim(self.max_height_tag, self.max_height_val),
            flex_direction,
            justify_content,
            align_items,
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            gap: self.gap,
            padding: EdgeInsets {
                top: self.padding[0],
                right: self.padding[1],
                bottom: self.padding[2],
                left: self.padding[3],
            },
            margin: EdgeInsets {
                top: self.margin[0],
                right: self.margin[1],
                bottom: self.margin[2],
                left: self.margin[3],
            },
        }
    }
}

/// 128-byte (2 cache lines), 8-byte-aligned zero-copy pre-rendered scene node.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelocSceneNode {
    pub node_id: u32,
    pub parent_index: i32,
    pub event_mask: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub bg_rgba: u32,
    pub text_rgba: u32,
    pub border_rgba: u32,
    pub z_index: i32,
    pub border_width: f32,
    pub border_radius: f32,
    pub opacity: f32,
    pub font_size: f32,
    pub kind: u8,
    pub flags: u8,
    pub text_align: u8,
    pub image_resize_mode: u8,
    pub font_weight: u16,
    pub _pad0: u16,
    pub text: RelocStr,
    pub tag: RelocStr,
    pub image_src: RelocStr,
    pub box_shadow: RelocStr,
    pub draw_list: RelocSlice<u8>,
    pub children: RelocSlice<u32>,
    pub reserved: u64,
}

impl RelocSceneNode {
    #[inline]
    pub const fn computed_rect(&self) -> LayoutRect {
        LayoutRect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    #[inline]
    pub const fn style_props(&self) -> StyleProps {
        StyleProps {
            background: ColorRgba::from_u32(self.bg_rgba),
            text_color: ColorRgba::from_u32(self.text_rgba),
            border_color: ColorRgba::from_u32(self.border_rgba),
            border_width: self.border_width,
            border_radius: self.border_radius,
            opacity: self.opacity,
            font_size: self.font_size,
            z_index: self.z_index,
        }
    }

    #[inline]
    pub const fn component_kind(&self) -> ComponentKind {
        match self.kind {
            0 => ComponentKind::Root,
            1 => ComponentKind::View,
            2 => ComponentKind::Text,
            3 => ComponentKind::ScrollView,
            4 => ComponentKind::Image,
            5 => ComponentKind::TextInput,
            6 => ComponentKind::Pressable,
            7 => ComponentKind::SkiaCanvas,
            _ => ComponentKind::Custom,
        }
    }

    #[inline]
    pub const fn clips_children(&self) -> bool {
        (self.flags & SCENE_NODE_CLIP_CHILDREN) != 0
    }

    #[inline]
    pub const fn has_explicit_style(&self) -> bool {
        (self.flags & SCENE_NODE_HAS_EXPLICIT_STYLE) != 0
    }

    #[inline]
    pub const fn is_interactive(&self) -> bool {
        (self.flags & SCENE_NODE_INTERACTIVE) != 0
    }

    #[inline]
    pub const fn is_hidden(&self) -> bool {
        (self.flags & SCENE_NODE_HIDDEN) != 0
    }
}

/// 64-byte root table for a pre-rendered Frame-0 `NVERELOC` scene (`RELOC_KIND_SCENE`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelocSceneRoot {
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub scale_factor: f32,
    pub root_node_index: u32,
    pub surface_id: u32,
    pub clear_rgba: u32,
    pub frame_id: u64,
    pub nodes: RelocSlice<RelocSceneNode>,
    pub layouts: RelocSlice<RelocLayoutProps>,
    pub app_tag: RelocStr,
    pub reserved: u64,
}

// SAFETY: All three scene structs are `#[repr(C)]` with zero implicit padding bytes.
unsafe impl ArenaPod for RelocLayoutProps {}
unsafe impl ArenaPod for RelocSceneNode {}
unsafe impl ArenaPod for RelocSceneRoot {}

const _: () = {
    assert!(size_of::<RelocLayoutProps>() == 80);
    assert!(align_of::<RelocLayoutProps>() == 4);
    assert!(offset_of!(RelocLayoutProps, padding) == 36);
    assert!(offset_of!(RelocLayoutProps, margin) == 52);
    assert!(offset_of!(RelocLayoutProps, width_tag) == 68);
    assert!(offset_of!(RelocLayoutProps, _pad) == 77);

    assert!(size_of::<RelocSceneNode>() == 128);
    assert!(align_of::<RelocSceneNode>() == 8);
    assert!(offset_of!(RelocSceneNode, node_id) == 0);
    assert!(offset_of!(RelocSceneNode, parent_index) == 4);
    assert!(offset_of!(RelocSceneNode, event_mask) == 8);
    assert!(offset_of!(RelocSceneNode, x) == 16);
    assert!(offset_of!(RelocSceneNode, bg_rgba) == 32);
    assert!(offset_of!(RelocSceneNode, border_width) == 48);
    assert!(offset_of!(RelocSceneNode, kind) == 64);
    assert!(offset_of!(RelocSceneNode, font_weight) == 68);
    assert!(offset_of!(RelocSceneNode, text) == 72);
    assert!(offset_of!(RelocSceneNode, tag) == 80);
    assert!(offset_of!(RelocSceneNode, image_src) == 88);
    assert!(offset_of!(RelocSceneNode, box_shadow) == 96);
    assert!(offset_of!(RelocSceneNode, draw_list) == 104);
    assert!(offset_of!(RelocSceneNode, children) == 112);
    assert!(offset_of!(RelocSceneNode, reserved) == 120);

    assert!(size_of::<RelocSceneRoot>() == 64);
    assert!(align_of::<RelocSceneRoot>() == 8);
    assert!(offset_of!(RelocSceneRoot, viewport_width) == 0);
    assert!(offset_of!(RelocSceneRoot, root_node_index) == 12);
    assert!(offset_of!(RelocSceneRoot, frame_id) == 24);
    assert!(offset_of!(RelocSceneRoot, nodes) == 32);
    assert!(offset_of!(RelocSceneRoot, layouts) == 40);
    assert!(offset_of!(RelocSceneRoot, app_tag) == 48);
    assert!(offset_of!(RelocSceneRoot, reserved) == 56);
};

/// Borrowed input for one node passed to [`encode_scene_reloc`].
#[derive(Debug, Clone, Copy)]
pub struct SceneRelocNodeInput<'a> {
    pub node_id: NodeId,
    pub parent_index: i32,
    pub kind: ComponentKind,
    pub flags: u8,
    pub text_align: u8,
    pub image_resize_mode: u8,
    pub font_weight: u16,
    pub event_mask: u64,
    pub computed_rect: LayoutRect,
    pub style: StyleProps,
    pub layout: LayoutProps,
    pub text: &'a str,
    pub tag: &'a str,
    pub image_src: &'a str,
    pub box_shadow: &'a str,
    pub draw_list: &'a [u8],
    pub children: &'a [u32],
}

/// Complete borrowed Frame-0 scene passed to [`encode_scene_reloc`].
#[derive(Debug, Clone, Copy)]
pub struct SceneRelocInput<'a> {
    pub schema_version: u32,
    pub surface_id: SurfaceId,
    pub frame_id: FrameId,
    pub viewport: Viewport,
    pub clear_color: ColorRgba,
    pub root_node_index: u32,
    pub app_tag: &'a str,
    pub nodes: &'a [SceneRelocNodeInput<'a>],
}

/// Encodes a complete pre-laid-out Frame-0 scene into a 64-byte-aligned `NVERELOC` [`RelocBuf`].
pub fn encode_scene_reloc(input: &SceneRelocInput<'_>) -> Result<RelocBuf, RelocError> {
    let n = input.nodes.len();
    if n > 0 && (input.root_node_index as usize) >= n {
        return Err(RelocError::InvalidScene("root_node_index out of bounds"));
    }
    let est_cap = size_of::<RelocSceneRoot>() + n * (size_of::<RelocSceneNode>() + 128) + 256;
    let mut w = RelocWriter::with_capacity(RELOC_KIND_SCENE, input.schema_version, 0, est_cap);

    let mut pods = Vec::with_capacity(n);
    let mut layouts = Vec::with_capacity(n);

    for node in input.nodes {
        for &child_idx in node.children {
            if (child_idx as usize) >= n {
                return Err(RelocError::InvalidScene("child index out of bounds"));
            }
        }
        let text = w.write_str(node.text)?;
        let tag = w.write_str(node.tag)?;
        let image_src = w.write_str(node.image_src)?;
        let box_shadow = w.write_str(node.box_shadow)?;
        let draw_list = w.write_bytes(node.draw_list, 4)?;
        let children = w.write_slice(node.children)?;

        pods.push(RelocSceneNode {
            node_id: node.node_id,
            parent_index: node.parent_index,
            event_mask: node.event_mask,
            x: node.computed_rect.x,
            y: node.computed_rect.y,
            width: node.computed_rect.width,
            height: node.computed_rect.height,
            bg_rgba: node.style.background.to_u32(),
            text_rgba: node.style.text_color.to_u32(),
            border_rgba: node.style.border_color.to_u32(),
            z_index: node.style.z_index,
            border_width: node.style.border_width,
            border_radius: node.style.border_radius,
            opacity: node.style.opacity,
            font_size: node.style.font_size,
            kind: nve_abi::component_kind_to_scene_u8(node.kind as u16),
            flags: node.flags,
            text_align: node.text_align,
            image_resize_mode: node.image_resize_mode,
            font_weight: node.font_weight,
            _pad0: 0,
            text,
            tag,
            image_src,
            box_shadow,
            draw_list,
            children,
            reserved: 0,
        });
        layouts.push(RelocLayoutProps::from_abi(&node.layout));
    }

    let nodes_slice = w.write_slice(&pods)?;
    let layouts_slice = w.write_slice(&layouts)?;
    let app_tag = w.write_str(input.app_tag)?;

    let root = RelocSceneRoot {
        viewport_width: input.viewport.width,
        viewport_height: input.viewport.height,
        scale_factor: input.viewport.scale_factor,
        root_node_index: input.root_node_index,
        surface_id: input.surface_id,
        clear_rgba: input.clear_color.to_u32(),
        frame_id: input.frame_id,
        nodes: nodes_slice,
        layouts: layouts_slice,
        app_tag,
        reserved: 0,
    };
    let root_ptr = w.write_val(&root)?;
    w.finish(root_ptr)
}

/// Zero-copy borrowed view directly over a pre-rendered Frame-0 `NVERELOC` scene.
#[derive(Clone, Copy)]
pub struct SceneRelocView<'a> {
    view: RelocView<'a>,
    root: &'a RelocSceneRoot,
    nodes: &'a [RelocSceneNode],
    layouts: &'a [RelocLayoutProps],
    strings_verified: bool,
}

impl<'a> SceneRelocView<'a> {
    /// Opens and validates a zero-copy Frame-0 scene view according to `mode`.
    pub fn from_bytes(bytes: &'a [u8], mode: VerifyMode) -> Result<Self, RelocError> {
        let view = RelocView::from_bytes(bytes, mode)?;
        Self::from_view(view)
    }

    /// Wraps an already-opened [`RelocView`] as a [`SceneRelocView`].
    pub fn from_view(view: RelocView<'a>) -> Result<Self, RelocError> {
        view.require_content_kind(RELOC_KIND_SCENE)?;
        let root = view.root::<RelocSceneRoot>()?;
        if root.reserved != 0 {
            return Err(RelocError::InvalidReservedHeader);
        }
        let nodes = view.slice(root.nodes)?;
        let layouts = view.slice(root.layouts)?;
        if !layouts.is_empty() && layouts.len() != nodes.len() {
            return Err(RelocError::InvalidScene("layouts len != nodes len"));
        }
        if !nodes.is_empty() && (root.root_node_index as usize) >= nodes.len() {
            return Err(RelocError::InvalidScene("root_node_index out of bounds"));
        }

        let mut strings_verified = false;
        if view.mode() == VerifyMode::Full {
            let _ = view.str_at(root.app_tag)?;
            for node in nodes {
                if node.reserved != 0 {
                    return Err(RelocError::InvalidReservedHeader);
                }
                if node.parent_index >= 0 && (node.parent_index as usize) >= nodes.len() {
                    return Err(RelocError::InvalidScene("parent_index out of bounds"));
                }
                let _ = view.str_at(node.text)?;
                let _ = view.str_at(node.tag)?;
                let _ = view.str_at(node.image_src)?;
                let _ = view.str_at(node.box_shadow)?;
                let _ = view.bytes_at(node.draw_list)?;
                let children = view.slice(node.children)?;
                for &cid in children {
                    if (cid as usize) >= nodes.len() {
                        return Err(RelocError::InvalidScene("child index out of bounds"));
                    }
                }
            }
            strings_verified = true;
        }

        Ok(Self {
            view,
            root,
            nodes,
            layouts,
            strings_verified,
        })
    }

    #[inline]
    pub fn header(&self) -> &'a RelocHeader {
        self.view.header()
    }

    #[inline]
    pub fn schema_version(&self) -> u32 {
        self.view.header().schema_version
    }

    #[inline]
    pub fn as_bytes(&self) -> &'a [u8] {
        self.view.as_bytes()
    }

    #[inline]
    pub fn root(&self) -> &'a RelocSceneRoot {
        self.root
    }

    #[inline]
    pub fn viewport(&self) -> Viewport {
        Viewport {
            width: self.root.viewport_width,
            height: self.root.viewport_height,
            scale_factor: self.root.scale_factor,
        }
    }

    #[inline]
    pub fn clear_color(&self) -> ColorRgba {
        ColorRgba::from_u32(self.root.clear_rgba)
    }

    #[inline]
    pub fn nodes(&self) -> &'a [RelocSceneNode] {
        self.nodes
    }

    #[inline]
    pub fn layouts(&self) -> &'a [RelocLayoutProps] {
        self.layouts
    }

    #[inline]
    pub fn node(&self, index: usize) -> Option<&'a RelocSceneNode> {
        self.nodes.get(index)
    }

    #[inline]
    pub fn layout_of_index(&self, index: usize) -> Option<LayoutProps> {
        self.layouts.get(index).map(RelocLayoutProps::to_abi)
    }

    #[inline]
    pub fn find_node_by_id(&self, node_id: NodeId) -> Option<(usize, &'a RelocSceneNode)> {
        self.nodes
            .iter()
            .enumerate()
            .find(|(_, n)| n.node_id == node_id)
    }

    #[inline]
    pub fn children_of(&self, node: &RelocSceneNode) -> Result<&'a [u32], RelocError> {
        self.view.slice(node.children)
    }

    #[inline]
    fn resolve_str(&self, s: RelocStr) -> Result<&'a str, RelocError> {
        if self.strings_verified {
            // SAFETY: Every `RelocStr` in `SceneRelocView` was validated during `VerifyMode::Full`.
            unsafe { self.view.str_at_unchecked(s) }
        } else {
            self.view.str_at(s)
        }
    }

    #[inline]
    pub fn text_of(&self, node: &RelocSceneNode) -> Result<&'a str, RelocError> {
        self.resolve_str(node.text)
    }

    #[inline]
    pub fn tag_of(&self, node: &RelocSceneNode) -> Result<&'a str, RelocError> {
        self.resolve_str(node.tag)
    }

    #[inline]
    pub fn image_src_of(&self, node: &RelocSceneNode) -> Result<&'a str, RelocError> {
        self.resolve_str(node.image_src)
    }

    #[inline]
    pub fn box_shadow_of(&self, node: &RelocSceneNode) -> Result<&'a str, RelocError> {
        self.resolve_str(node.box_shadow)
    }

    #[inline]
    pub fn draw_list_of(&self, node: &RelocSceneNode) -> Result<&'a [u8], RelocError> {
        self.view.bytes_at(node.draw_list)
    }

    #[inline]
    pub fn app_tag(&self) -> Result<&'a str, RelocError> {
        self.resolve_str(self.root.app_tag)
    }
}
