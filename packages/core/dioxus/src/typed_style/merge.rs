//! Bitmask tracking and field-granular merging (`TypedStyle::merge` / `merge_opt`) so caller
//! `nve_style` overrides only overwrite properties they explicitly set.

use core::ops::BitOrAssign;

use nve_tables::layout_ext::LayoutExt;

use super::TypedStyle;

/// Typed bitmask tracking which [`nve_tables::LayoutSnapshot`] fields were set on a [`TypedStyle`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayoutMask(u32);

impl LayoutMask {
    pub(crate) const EMPTY: Self = Self(0);
    #[inline]
    pub(crate) const fn bits(self) -> u32 { self.0 }
    #[inline]
    pub(crate) const fn is_empty(self) -> bool { self.0 == 0 }
}

impl BitOrAssign<u32> for LayoutMask {
    #[inline]
    fn bitor_assign(&mut self, rhs: u32) { self.0 |= rhs; }
}

/// Typed bitmask tracking which [`nve_tables::VisualSnapshot`] fields were set on a [`TypedStyle`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StyleMask(u16);

impl StyleMask {
    pub(crate) const EMPTY: Self = Self(0);
    #[inline]
    pub(crate) const fn bits(self) -> u16 { self.0 }
}

impl BitOrAssign<u16> for StyleMask {
    #[inline]
    fn bitor_assign(&mut self, rhs: u16) { self.0 |= rhs; }
}

/// Typed bitmask tracking which [`LayoutExt`] fields were set on a [`TypedStyle`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayoutExtMask(u32);

impl LayoutExtMask {
    pub(crate) const EMPTY: Self = Self(0);
    #[inline]
    pub(crate) const fn bits(self) -> u32 { self.0 }
}

impl BitOrAssign<u32> for LayoutExtMask {
    #[inline]
    fn bitor_assign(&mut self, rhs: u32) { self.0 |= rhs; }
}

// LayoutSnapshot field presence bits (`TypedStyle::layout_mask`).
pub(super) const L_WIDTH: u32 = 1 << 0;
pub(super) const L_HEIGHT: u32 = 1 << 1;
pub(super) const L_MIN_WIDTH: u32 = 1 << 2;
pub(super) const L_MIN_HEIGHT: u32 = 1 << 3;
pub(super) const L_MAX_WIDTH: u32 = 1 << 4;
pub(super) const L_MAX_HEIGHT: u32 = 1 << 5;
pub(super) const L_FLEX_DIR: u32 = 1 << 6;
pub(super) const L_JUSTIFY: u32 = 1 << 7;
pub(super) const L_ALIGN_ITEMS: u32 = 1 << 8;
pub(super) const L_ALIGN_SELF: u32 = 1 << 9;
pub(super) const L_FLEX_GROW: u32 = 1 << 10;
pub(super) const L_FLEX_SHRINK: u32 = 1 << 11;
pub(super) const L_GAP: u32 = 1 << 12;
pub(super) const L_PAD_TOP: u32 = 1 << 13;
pub(super) const L_PAD_RIGHT: u32 = 1 << 14;
pub(super) const L_PAD_BOTTOM: u32 = 1 << 15;
pub(super) const L_PAD_LEFT: u32 = 1 << 16;
pub(super) const L_MAR_TOP: u32 = 1 << 17;
pub(super) const L_MAR_RIGHT: u32 = 1 << 18;
pub(super) const L_MAR_BOTTOM: u32 = 1 << 19;
pub(super) const L_MAR_LEFT: u32 = 1 << 20;

pub(super) const L_PAD_ALL: u32 = L_PAD_TOP | L_PAD_RIGHT | L_PAD_BOTTOM | L_PAD_LEFT;
pub(super) const L_PAD_X: u32 = L_PAD_LEFT | L_PAD_RIGHT;
pub(super) const L_PAD_Y: u32 = L_PAD_TOP | L_PAD_BOTTOM;
pub(super) const L_MAR_ALL: u32 = L_MAR_TOP | L_MAR_RIGHT | L_MAR_BOTTOM | L_MAR_LEFT;
pub(super) const L_MAR_X: u32 = L_MAR_LEFT | L_MAR_RIGHT;
pub(super) const L_MAR_Y: u32 = L_MAR_TOP | L_MAR_BOTTOM;

// VisualSnapshot field presence bits (`TypedStyle::style_mask`).
pub(super) const S_BG: u16 = 1 << 0;
pub(super) const S_BORDER_COLOR: u16 = 1 << 1;
pub(super) const S_BORDER_WIDTH: u16 = 1 << 2;
pub(super) const S_BORDER_RADIUS: u16 = 1 << 3;
pub(super) const S_OPACITY: u16 = 1 << 4;
pub(super) const S_Z_INDEX: u16 = 1 << 5;
pub(super) const S_TEXT_COLOR: u16 = 1 << 6;
pub(super) const S_FONT_SIZE: u16 = 1 << 7;

pub(super) const S_VISUAL_BITS: u16 =
    S_BG | S_BORDER_COLOR | S_BORDER_WIDTH | S_BORDER_RADIUS | S_OPACITY | S_Z_INDEX;
pub(super) const S_TYPO_BITS: u16 = S_TEXT_COLOR | S_FONT_SIZE;

// LayoutExt field presence bits (`TypedStyle::layout_ext_mask`).
pub(super) const E_DISPLAY: u32 = 1 << 0;
pub(super) const E_POSITION: u32 = 1 << 1;
pub(super) const E_FLEX_BASIS: u32 = 1 << 2;
pub(super) const E_INSET_TOP: u32 = 1 << 3;
pub(super) const E_INSET_RIGHT: u32 = 1 << 4;
pub(super) const E_INSET_BOTTOM: u32 = 1 << 5;
pub(super) const E_INSET_LEFT: u32 = 1 << 6;
pub(super) const E_ASPECT_RATIO: u32 = 1 << 7;
pub(super) const E_DIRECTION: u32 = 1 << 8;
pub(super) const E_PAD_INLINE_START: u32 = 1 << 9;
pub(super) const E_PAD_INLINE_END: u32 = 1 << 10;
pub(super) const E_MAR_INLINE_START: u32 = 1 << 11;
pub(super) const E_MAR_INLINE_END: u32 = 1 << 12;
pub(super) const E_GRID_COLS: u32 = 1 << 13;
pub(super) const E_GRID_ROWS: u32 = 1 << 14;
pub(super) const E_COL_GAP: u32 = 1 << 15;
pub(super) const E_ROW_GAP: u32 = 1 << 16;
pub(super) const E_GRID_COL_ITEM: u32 = 1 << 17;
pub(super) const E_GRID_ROW_ITEM: u32 = 1 << 18;

pub(super) const E_PAD_INLINE_ANY: u32 = E_PAD_INLINE_START | E_PAD_INLINE_END;
pub(super) const E_MAR_INLINE_ANY: u32 = E_MAR_INLINE_START | E_MAR_INLINE_END;
pub(super) const E_GRID_LAYOUT_ANY: u32 = E_GRID_COLS | E_GRID_ROWS;
pub(super) const E_GRID_ITEM_ANY: u32 = E_GRID_COL_ITEM | E_GRID_ROW_ITEM;
pub(super) const E_LAYOUT_TABLE_BITS: u32 = E_DISPLAY
    | E_POSITION
    | E_FLEX_BASIS
    | E_INSET_TOP
    | E_INSET_RIGHT
    | E_INSET_BOTTOM
    | E_INSET_LEFT
    | E_ASPECT_RATIO
    | E_DIRECTION
    | E_PAD_INLINE_ANY
    | E_MAR_INLINE_ANY
    | E_COL_GAP
    | E_ROW_GAP;

// Overflow axis presence bits (`TypedStyle::overflow_axes`).
pub(super) const OV_X: u8 = 1 << 0;
pub(super) const OV_Y: u8 = 1 << 1;
pub(super) const OV_BOTH: u8 = OV_X | OV_Y;

impl TypedStyle {
    #[inline]
    pub(super) const fn any_layout(&self) -> bool {
        !self.layout_mask.is_empty()
            || self.flex_wrap.is_some()
            || (self.layout_ext_mask.bits() & E_LAYOUT_TABLE_BITS) != 0
    }

    #[inline]
    pub(super) const fn any_visual(&self) -> bool {
        (self.style_mask.bits() & S_VISUAL_BITS) != 0
            || self.box_shadows.is_some()
            || self.overflow_axes != 0
            || self.background_fill.is_some()
    }

    #[inline]
    pub(super) const fn any_typography(&self) -> bool {
        (self.style_mask.bits() & S_TYPO_BITS) != 0
            || self.font_family.is_some()
            || self.line_height.is_some()
            || self.font_weight.is_some()
            || self.font_style.is_some()
            || self.text_align.is_some()
            || self.white_space.is_some()
            || self.overflow_wrap.is_some()
            || self.text_decoration.is_some()
    }

    /// Merges every property set on `other` on top of `self` (last-writer-wins per field).
    pub fn merge(&mut self, other: &TypedStyle) -> &mut Self {
        let lm = other.layout_mask.bits();
        if lm != 0 {
            let l = &other.layout;
            if lm & L_WIDTH != 0 { self.layout.width = l.width; }
            if lm & L_HEIGHT != 0 { self.layout.height = l.height; }
            if lm & L_MIN_WIDTH != 0 { self.layout.min_width = l.min_width; }
            if lm & L_MIN_HEIGHT != 0 { self.layout.min_height = l.min_height; }
            if lm & L_MAX_WIDTH != 0 { self.layout.max_width = l.max_width; }
            if lm & L_MAX_HEIGHT != 0 { self.layout.max_height = l.max_height; }
            if lm & L_FLEX_DIR != 0 { self.layout.flex_direction = l.flex_direction; }
            if lm & L_JUSTIFY != 0 { self.layout.justify_content = l.justify_content; }
            if lm & L_ALIGN_ITEMS != 0 { self.layout.align_items = l.align_items; }
            if lm & L_ALIGN_SELF != 0 { self.layout.align_self = l.align_self; }
            if lm & L_FLEX_GROW != 0 { self.layout.flex_grow = l.flex_grow; }
            if lm & L_FLEX_SHRINK != 0 { self.layout.flex_shrink = l.flex_shrink; }
            if lm & L_GAP != 0 { self.layout.gap = l.gap; }
            if lm & L_PAD_TOP != 0 { self.layout.padding.top = l.padding.top; }
            if lm & L_PAD_RIGHT != 0 { self.layout.padding.right = l.padding.right; }
            if lm & L_PAD_BOTTOM != 0 { self.layout.padding.bottom = l.padding.bottom; }
            if lm & L_PAD_LEFT != 0 { self.layout.padding.left = l.padding.left; }
            if lm & L_MAR_TOP != 0 { self.layout.margin.top = l.margin.top; }
            if lm & L_MAR_RIGHT != 0 { self.layout.margin.right = l.margin.right; }
            if lm & L_MAR_BOTTOM != 0 { self.layout.margin.bottom = l.margin.bottom; }
            if lm & L_MAR_LEFT != 0 { self.layout.margin.left = l.margin.left; }
            self.layout_mask |= lm;
        }

        let sm = other.style_mask.bits();
        if sm != 0 {
            let s = &other.style;
            if sm & S_BG != 0 { self.style.background = s.background; }
            if sm & S_BORDER_COLOR != 0 { self.style.border_color = s.border_color; }
            if sm & S_BORDER_WIDTH != 0 { self.style.border_width = s.border_width; }
            if sm & S_BORDER_RADIUS != 0 { self.style.border_radius = s.border_radius; }
            if sm & S_OPACITY != 0 { self.style.opacity = s.opacity; }
            if sm & S_Z_INDEX != 0 { self.style.z_index = s.z_index; }
            if sm & S_TEXT_COLOR != 0 { self.style.text_color = s.text_color; }
            if sm & S_FONT_SIZE != 0 { self.style.font_size = s.font_size; }
            self.style_mask |= sm;
        }

        if let Some(v) = other.flex_wrap { self.flex_wrap = Some(v); }
        if let Some(ref v) = other.font_family { self.font_family = Some(v.clone()); }
        if let Some(v) = other.line_height { self.line_height = Some(v); }
        if let Some(v) = other.font_weight { self.font_weight = Some(v); }
        if let Some(v) = other.font_style { self.font_style = Some(v); }
        if let Some(v) = other.text_align { self.text_align = Some(v); }
        if let Some(v) = other.white_space { self.white_space = Some(v); }
        if let Some(v) = other.overflow_wrap { self.overflow_wrap = Some(v); }
        if let Some(v) = other.text_decoration { self.text_decoration = Some(v); }
        if let Some(ref v) = other.box_shadows { self.box_shadows = Some(v.clone()); }

        if let Some((ox, oy)) = other.overflow {
            let other_axes = other.overflow_axes;
            if other_axes != 0 {
                let (mut cx, mut cy) = self.overflow.unwrap_or((
                    nve_tables::tables::Overflow::Visible,
                    nve_tables::tables::Overflow::Visible,
                ));
                if other_axes & OV_X != 0 { cx = ox; }
                if other_axes & OV_Y != 0 { cy = oy; }
                self.overflow = Some((cx, cy));
                self.overflow_axes |= other_axes;
            }
        }

        if let Some(v) = other.object_fit { self.object_fit = Some(v); }
        if let Some(ref other_ext) = other.layout_ext {
            let em = other.layout_ext_mask.bits();
            if em != 0 {
                merge_layout_ext(self.ext_mut(), other_ext, em);
                self.layout_ext_mask |= em;
            }
        }
        if let Some(v) = other.transform { self.transform = Some(v); }
        if let Some(ref v) = other.background_fill { self.background_fill = Some(v.clone()); }
        if let Some(v) = other.show_on { self.show_on = Some(v); }
        if !other.responsive.is_empty() {
            self.responsive.extend(other.responsive.iter().cloned());
        }
        if let Some(v) = other.cursor { self.cursor = Some(v); }
        if let Some(v) = other.capture_mode { self.capture_mode = Some(v); }
        if !other.state_rules.is_empty() {
            self.state_rules.extend(other.state_rules.iter().cloned());
        }
        self
    }

    /// Fluent by-value counterpart to [`Self::merge`].
    #[inline]
    pub fn merged(mut self, other: &TypedStyle) -> Self {
        self.merge(other);
        self
    }

    /// Merges `other` on top of `self` when `Some`, returning `self` unchanged on `None`.
    #[inline]
    pub fn merge_opt(mut self, other: Option<&TypedStyle>) -> Self {
        if let Some(o) = other {
            self.merge(o);
        }
        self
    }
}

fn merge_layout_ext(dst: &mut LayoutExt, src: &LayoutExt, mask: u32) {
    if mask & E_DISPLAY != 0 { dst.display = src.display; }
    if mask & E_POSITION != 0 { dst.position = src.position; }
    if mask & E_FLEX_BASIS != 0 { dst.flex_basis = src.flex_basis; }
    if mask & E_INSET_TOP != 0 { dst.insets.top = src.insets.top; }
    if mask & E_INSET_RIGHT != 0 { dst.insets.right = src.insets.right; }
    if mask & E_INSET_BOTTOM != 0 { dst.insets.bottom = src.insets.bottom; }
    if mask & E_INSET_LEFT != 0 { dst.insets.left = src.insets.left; }
    if mask & E_ASPECT_RATIO != 0 { dst.aspect_ratio = src.aspect_ratio; }
    if mask & E_DIRECTION != 0 { dst.direction = src.direction; }
    if mask & E_PAD_INLINE_START != 0 { dst.padding_inline.start = src.padding_inline.start; }
    if mask & E_PAD_INLINE_END != 0 { dst.padding_inline.end = src.padding_inline.end; }
    if mask & E_MAR_INLINE_START != 0 { dst.margin_inline.start = src.margin_inline.start; }
    if mask & E_MAR_INLINE_END != 0 { dst.margin_inline.end = src.margin_inline.end; }
    if let Some(ref g) = src.grid {
        if mask & E_GRID_COLS != 0 { dst.grid_mut().template_columns = g.template_columns.clone(); }
        if mask & E_GRID_ROWS != 0 { dst.grid_mut().template_rows = g.template_rows.clone(); }
        if mask & E_COL_GAP != 0 { dst.grid_mut().column_gap = g.column_gap; }
        if mask & E_ROW_GAP != 0 { dst.grid_mut().row_gap = g.row_gap; }
    }
    if mask & E_GRID_COL_ITEM != 0 { dst.grid_item.column = src.grid_item.column; }
    if mask & E_GRID_ROW_ITEM != 0 { dst.grid_item.row = src.grid_item.row; }
}
