//! Value-type resolved snapshots ([`VisualSnapshot`], [`LayoutSnapshot`]) and zero-alloc
//! [`RowPatch`] / [`RowPatchView`] snapshot application helpers.

use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_store::{prop_written, Length, NveTable, RowPatch, RowPatchView, Word};

use crate::layout_ext::{LayoutDirection, LayoutExt, PositionMode};
use crate::tables::{
    AlignSelf, Direction, GridItem, Layout, Position, Transform, Typography, Visual,
};
use crate::transform::Transform2D;

/// Resolved value-type snapshot of core visual + typography properties (used by motion
/// interpolation and pseudo-state resolution; derived directly from `NodeStore` word tables).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualSnapshot {
    pub background: ColorRgba,
    pub text_color: ColorRgba,
    pub border_color: ColorRgba,
    pub border_width: f32,
    pub border_radius: f32,
    pub opacity: f32,
    pub font_size: f32,
    pub z_index: i32,
}

impl VisualSnapshot {
    /// Keeps only inherited text styling: the snapshot of a node that generates no box.
    #[inline]
    pub fn without_box_paint(self) -> Self {
        Self {
            text_color: self.text_color,
            font_size: self.font_size,
            ..Self::default()
        }
    }
}

impl Default for VisualSnapshot {
    fn default() -> Self {
        Self {
            background: ColorRgba::TRANSPARENT,
            text_color: ColorRgba::BLACK,
            border_color: ColorRgba::TRANSPARENT,
            border_width: 0.0,
            border_radius: 0.0,
            opacity: 1.0,
            font_size: 16.0,
            z_index: 0,
        }
    }
}

/// Resolved value-type snapshot of core box-layout properties (derived from `tables::Layout`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutSnapshot {
    pub width: Dimension,
    pub height: Dimension,
    pub min_width: Dimension,
    pub min_height: Dimension,
    pub max_width: Dimension,
    pub max_height: Dimension,
    pub flex_direction: FlexDirection,
    pub justify_content: FlexJustify,
    pub align_items: FlexAlign,
    pub align_self: AlignSelf,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub gap: f32,
    pub padding: EdgeInsets,
    pub margin: EdgeInsets,
}

impl Default for LayoutSnapshot {
    fn default() -> Self {
        Self {
            width: Dimension::Auto,
            height: Dimension::Auto,
            min_width: Dimension::Auto,
            min_height: Dimension::Auto,
            max_width: Dimension::Auto,
            max_height: Dimension::Auto,
            flex_direction: FlexDirection::Column,
            justify_content: FlexJustify::FlexStart,
            align_items: FlexAlign::Stretch,
            align_self: AlignSelf::Auto,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            gap: 0.0,
            padding: EdgeInsets::default(),
            margin: EdgeInsets::default(),
        }
    }
}

#[inline]
pub(crate) fn length_to_dim(l: Length) -> Dimension {
    if l.is_points() {
        Dimension::Points(l.value())
    } else if l.is_percent() {
        Dimension::Percent(l.value())
    } else {
        Dimension::Auto
    }
}

fn apply_visual_and_typo_words(
    v_words: &[Word],
    v_mask: &[u32],
    has_v: bool,
    t_words: &[Word],
    t_mask: &[u32],
    has_t: bool,
    style: &mut VisualSnapshot,
) {
    if has_v {
        if prop_written(v_mask, Visual::BACKGROUND) {
            style.background = ColorRgba::from_u32(Visual::BACKGROUND.get(v_words).0);
        }
        if prop_written(v_mask, Visual::BORDER_WIDTH) {
            style.border_width = Visual::BORDER_WIDTH.get(v_words);
        }
        if prop_written(v_mask, Visual::BORDER_COLOR) {
            style.border_color = ColorRgba::from_u32(Visual::BORDER_COLOR.get(v_words).0);
        }
        if prop_written(v_mask, Visual::BORDER_RADIUS) {
            style.border_radius = Visual::BORDER_RADIUS.get(v_words);
        }
        if prop_written(v_mask, Visual::OPACITY) {
            style.opacity = Visual::OPACITY.get(v_words);
        }
        if prop_written(v_mask, Visual::Z_INDEX) {
            style.z_index = Visual::Z_INDEX.get(v_words);
        }
    }
    if has_t {
        if prop_written(t_mask, Typography::COLOR) {
            style.text_color = ColorRgba::from_u32(Typography::COLOR.get(t_words).0);
        }
        if prop_written(t_mask, Typography::FONT_SIZE) {
            style.font_size = Typography::FONT_SIZE.get(t_words);
        }
    }
}

fn apply_layout_words(l_words: &[Word], l_mask: &[u32], layout: &mut LayoutSnapshot) {
    if prop_written(l_mask, Layout::FLEX_DIRECTION) {
        layout.flex_direction = Layout::FLEX_DIRECTION.get(l_words);
    }
    if prop_written(l_mask, Layout::JUSTIFY_CONTENT) {
        layout.justify_content = Layout::JUSTIFY_CONTENT.get(l_words).into();
    }
    if prop_written(l_mask, Layout::ALIGN_ITEMS) {
        layout.align_items = Layout::ALIGN_ITEMS.get(l_words).into();
    }
    if prop_written(l_mask, Layout::FLEX_GROW) {
        layout.flex_grow = Layout::FLEX_GROW.get(l_words);
    }
    if prop_written(l_mask, Layout::FLEX_SHRINK) {
        layout.flex_shrink = Layout::FLEX_SHRINK.get(l_words);
    }
    if prop_written(l_mask, Layout::ROW_GAP) {
        layout.gap = Layout::ROW_GAP.get(l_words).value();
    } else if prop_written(l_mask, Layout::COLUMN_GAP) {
        layout.gap = Layout::COLUMN_GAP.get(l_words).value();
    }
    if prop_written(l_mask, Layout::WIDTH) {
        layout.width = length_to_dim(Layout::WIDTH.get(l_words));
    }
    if prop_written(l_mask, Layout::HEIGHT) {
        layout.height = length_to_dim(Layout::HEIGHT.get(l_words));
    }
    if prop_written(l_mask, Layout::MIN_WIDTH) {
        layout.min_width = length_to_dim(Layout::MIN_WIDTH.get(l_words));
    }
    if prop_written(l_mask, Layout::MAX_WIDTH) {
        layout.max_width = length_to_dim(Layout::MAX_WIDTH.get(l_words));
    }
    if prop_written(l_mask, Layout::MIN_HEIGHT) {
        layout.min_height = length_to_dim(Layout::MIN_HEIGHT.get(l_words));
    }
    if prop_written(l_mask, Layout::MAX_HEIGHT) {
        layout.max_height = length_to_dim(Layout::MAX_HEIGHT.get(l_words));
    }
    if prop_written(l_mask, Layout::PADDING_TOP) {
        layout.padding.top = Layout::PADDING_TOP.get(l_words).value();
    }
    if prop_written(l_mask, Layout::PADDING_RIGHT) {
        layout.padding.right = Layout::PADDING_RIGHT.get(l_words).value();
    }
    if prop_written(l_mask, Layout::PADDING_BOTTOM) {
        layout.padding.bottom = Layout::PADDING_BOTTOM.get(l_words).value();
    }
    if prop_written(l_mask, Layout::PADDING_LEFT) {
        layout.padding.left = Layout::PADDING_LEFT.get(l_words).value();
    }
    if prop_written(l_mask, Layout::MARGIN_TOP) {
        layout.margin.top = Layout::MARGIN_TOP.get(l_words).value();
    }
    if prop_written(l_mask, Layout::MARGIN_RIGHT) {
        layout.margin.right = Layout::MARGIN_RIGHT.get(l_words).value();
    }
    if prop_written(l_mask, Layout::MARGIN_BOTTOM) {
        layout.margin.bottom = Layout::MARGIN_BOTTOM.get(l_words).value();
    }
    if prop_written(l_mask, Layout::MARGIN_LEFT) {
        layout.margin.left = Layout::MARGIN_LEFT.get(l_words).value();
    }
}

fn apply_layout_ext_words(
    l_words: &[Word],
    l_mask: &[u32],
    has_l: bool,
    gi_words: &[Word],
    gi_mask: &[u32],
    has_gi: bool,
    ext: &mut LayoutExt,
) {
    if has_l {
        if prop_written(l_mask, Layout::DISPLAY) {
            ext.display = Layout::DISPLAY.get(l_words);
        }
        if prop_written(l_mask, Layout::FLEX_BASIS) {
            ext.flex_basis = length_to_dim(Layout::FLEX_BASIS.get(l_words));
        }
        if prop_written(l_mask, Layout::ASPECT_RATIO)
            && let Some(ar) = Layout::ASPECT_RATIO.get(l_words)
        {
            ext.aspect_ratio = Some(ar);
        }
        if prop_written(l_mask, Layout::POSITION) {
            ext.position = match Layout::POSITION.get(l_words) {
                Position::Relative => PositionMode::Relative,
                Position::Absolute | Position::Fixed => PositionMode::Absolute,
                Position::Sticky => PositionMode::Sticky,
            };
        }
        if prop_written(l_mask, Layout::INSET_TOP) {
            ext.insets.top = length_to_dim(Layout::INSET_TOP.get(l_words));
        }
        if prop_written(l_mask, Layout::INSET_RIGHT) {
            ext.insets.right = length_to_dim(Layout::INSET_RIGHT.get(l_words));
        }
        if prop_written(l_mask, Layout::INSET_BOTTOM) {
            ext.insets.bottom = length_to_dim(Layout::INSET_BOTTOM.get(l_words));
        }
        if prop_written(l_mask, Layout::INSET_LEFT) {
            ext.insets.left = length_to_dim(Layout::INSET_LEFT.get(l_words));
        }
        if prop_written(l_mask, Layout::COLUMN_GAP) {
            ext.grid_mut().column_gap = Some(Layout::COLUMN_GAP.get(l_words).value());
        }
        if prop_written(l_mask, Layout::ROW_GAP) {
            ext.grid_mut().row_gap = Some(Layout::ROW_GAP.get(l_words).value());
        }
        if prop_written(l_mask, Layout::DIRECTION) {
            ext.direction = match Layout::DIRECTION.get(l_words) {
                Direction::Ltr => Some(LayoutDirection::Ltr),
                Direction::Rtl => Some(LayoutDirection::Rtl),
                Direction::Inherit => None,
            };
        }
        let pad_logical = prop_written(l_mask, Layout::PADDING_INLINE_LOGICAL)
            && Layout::PADDING_INLINE_LOGICAL.get(l_words);
        if pad_logical {
            if prop_written(l_mask, Layout::PADDING_LEFT) {
                ext.padding_inline.start = Some(Layout::PADDING_LEFT.get(l_words).value());
            }
            if prop_written(l_mask, Layout::PADDING_RIGHT) {
                ext.padding_inline.end = Some(Layout::PADDING_RIGHT.get(l_words).value());
            }
        }
        let mar_logical = prop_written(l_mask, Layout::MARGIN_INLINE_LOGICAL)
            && Layout::MARGIN_INLINE_LOGICAL.get(l_words);
        if mar_logical {
            if prop_written(l_mask, Layout::MARGIN_LEFT) {
                ext.margin_inline.start = Some(Layout::MARGIN_LEFT.get(l_words).value());
            }
            if prop_written(l_mask, Layout::MARGIN_RIGHT) {
                ext.margin_inline.end = Some(Layout::MARGIN_RIGHT.get(l_words).value());
            }
        }
    }
    if has_gi {
        if prop_written(gi_mask, GridItem::COLUMN_START) {
            ext.grid_item.column.start = GridItem::COLUMN_START.get(gi_words);
        }
        if prop_written(gi_mask, GridItem::COLUMN_END) {
            ext.grid_item.column.end = GridItem::COLUMN_END.get(gi_words);
        }
        if prop_written(gi_mask, GridItem::ROW_START) {
            ext.grid_item.row.start = GridItem::ROW_START.get(gi_words);
        }
        if prop_written(gi_mask, GridItem::ROW_END) {
            ext.grid_item.row.end = GridItem::ROW_END.get(gi_words);
        }
    }
}

const _: () = {
    assert!(Visual::DESC.words() <= 16);
    assert!(Typography::DESC.words() <= 16);
    assert!(Layout::DESC.words() <= 32);
    assert!(GridItem::DESC.words() <= 8);
    assert!(Transform::DESC.words() <= 8);
};

/// Applies all present visual + typography overrides from `patch` onto `style`.
pub fn apply_patch_view_to_visual(patch: RowPatchView<'_>, style: &mut VisualSnapshot) {
    let vn = Visual::DESC.words();
    let tn = Typography::DESC.words();
    let mut vw = [0u32; 16];
    let mut vm = [0u32; 16];
    vw[..vn].copy_from_slice(Visual::DESC.defaults);
    let has_v = patch.apply_table_with_mask::<Visual>(&mut vw[..vn], &mut vm[..vn]);
    let mut tw = [0u32; 16];
    let mut tm = [0u32; 16];
    tw[..tn].copy_from_slice(Typography::DESC.defaults);
    let has_t = patch.apply_table_with_mask::<Typography>(&mut tw[..tn], &mut tm[..tn]);
    apply_visual_and_typo_words(&vw[..vn], &vm[..vn], has_v, &tw[..tn], &tm[..tn], has_t, style);
}

/// Applies all present layout overrides from `patch` onto `layout`.
pub fn apply_patch_view_to_layout(patch: RowPatchView<'_>, layout: &mut LayoutSnapshot) {
    let ln = Layout::DESC.words();
    let mut lw = [0u32; 32];
    let mut lm = [0u32; 32];
    lw[..ln].copy_from_slice(Layout::DESC.defaults);
    if patch.apply_table_with_mask::<Layout>(&mut lw[..ln], &mut lm[..ln]) {
        apply_layout_words(&lw[..ln], &lm[..ln], layout);
    }
}

/// Applies all present extended-layout overrides from `patch` onto `ext`.
pub fn apply_patch_view_to_layout_ext(patch: RowPatchView<'_>, ext: &mut LayoutExt) {
    let ln = Layout::DESC.words();
    let gin = GridItem::DESC.words();
    let mut lw = [0u32; 32];
    let mut lm = [0u32; 32];
    lw[..ln].copy_from_slice(Layout::DESC.defaults);
    let has_l = patch.apply_table_with_mask::<Layout>(&mut lw[..ln], &mut lm[..ln]);
    let mut giw = [0u32; 8];
    let mut gim = [0u32; 8];
    giw[..gin].copy_from_slice(GridItem::DESC.defaults);
    let has_gi = patch.apply_table_with_mask::<GridItem>(&mut giw[..gin], &mut gim[..gin]);
    apply_layout_ext_words(&lw[..ln], &lm[..ln], has_l, &giw[..gin], &gim[..gin], has_gi, ext);
}

/// Extracts a [`Transform2D`] override from a [`RowPatchView`], if any transform field was patched.
pub fn transform_from_patch_view(patch: RowPatchView<'_>) -> Option<Transform2D> {
    let n = Transform::DESC.words();
    let mut w = [0u32; 8];
    w[..n].copy_from_slice(Transform::DESC.defaults);
    if !patch.apply_table::<Transform>(&mut w[..n]) {
        return None;
    }
    Some(Transform2D {
        translate_x: Transform::TRANSLATE_X.get(&w[..n]),
        translate_y: Transform::TRANSLATE_Y.get(&w[..n]),
        scale_x: Transform::SCALE_X.get(&w[..n]),
        scale_y: Transform::SCALE_Y.get(&w[..n]),
        rotate_deg: Transform::ROTATE_DEG.get(&w[..n]),
        origin_x: Transform::ORIGIN_X.get(&w[..n]),
        origin_y: Transform::ORIGIN_Y.get(&w[..n]),
    })
}

/// Applies all present visual + typography overrides from owned `patch` onto `style`.
pub fn apply_patch_to_visual(patch: &RowPatch, style: &mut VisualSnapshot) {
    let vn = Visual::DESC.words();
    let tn = Typography::DESC.words();
    let mut vw = [0u32; 16];
    let mut vm = [0u32; 16];
    vw[..vn].copy_from_slice(Visual::DESC.defaults);
    let has_v = patch.apply_table_with_mask::<Visual>(&mut vw[..vn], &mut vm[..vn]);
    let mut tw = [0u32; 16];
    let mut tm = [0u32; 16];
    tw[..tn].copy_from_slice(Typography::DESC.defaults);
    let has_t = patch.apply_table_with_mask::<Typography>(&mut tw[..tn], &mut tm[..tn]);
    apply_visual_and_typo_words(&vw[..vn], &vm[..vn], has_v, &tw[..tn], &tm[..tn], has_t, style);
}

/// Applies all present layout overrides from owned `patch` onto `layout`.
pub fn apply_patch_to_layout(patch: &RowPatch, layout: &mut LayoutSnapshot) {
    let ln = Layout::DESC.words();
    let mut lw = [0u32; 32];
    let mut lm = [0u32; 32];
    lw[..ln].copy_from_slice(Layout::DESC.defaults);
    if patch.apply_table_with_mask::<Layout>(&mut lw[..ln], &mut lm[..ln]) {
        apply_layout_words(&lw[..ln], &lm[..ln], layout);
    }
}

/// Applies all present extended-layout overrides from owned `patch` onto `ext`.
pub fn apply_patch_to_layout_ext(patch: &RowPatch, ext: &mut LayoutExt) {
    let ln = Layout::DESC.words();
    let gin = GridItem::DESC.words();
    let mut lw = [0u32; 32];
    let mut lm = [0u32; 32];
    lw[..ln].copy_from_slice(Layout::DESC.defaults);
    let has_l = patch.apply_table_with_mask::<Layout>(&mut lw[..ln], &mut lm[..ln]);
    let mut giw = [0u32; 8];
    let mut gim = [0u32; 8];
    giw[..gin].copy_from_slice(GridItem::DESC.defaults);
    let has_gi = patch.apply_table_with_mask::<GridItem>(&mut giw[..gin], &mut gim[..gin]);
    apply_layout_ext_words(&lw[..ln], &lm[..ln], has_l, &giw[..gin], &gim[..gin], has_gi, ext);
}

/// Extracts a [`Transform2D`] override from an owned [`RowPatch`], if any transform field was patched.
pub fn transform_from_patch(patch: &RowPatch) -> Option<Transform2D> {
    let n = Transform::DESC.words();
    let mut w = [0u32; 8];
    w[..n].copy_from_slice(Transform::DESC.defaults);
    if !patch.apply_table::<Transform>(&mut w[..n]) {
        return None;
    }
    Some(Transform2D {
        translate_x: Transform::TRANSLATE_X.get(&w[..n]),
        translate_y: Transform::TRANSLATE_Y.get(&w[..n]),
        scale_x: Transform::SCALE_X.get(&w[..n]),
        scale_y: Transform::SCALE_Y.get(&w[..n]),
        rotate_deg: Transform::ROTATE_DEG.get(&w[..n]),
        origin_x: Transform::ORIGIN_X.get(&w[..n]),
        origin_y: Transform::ORIGIN_Y.get(&w[..n]),
    })
}
