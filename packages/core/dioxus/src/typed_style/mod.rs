//! Zero-string-parse typed style attribute (`TypedStyle`) passed directly through Dioxus
//! `AttributeValue::Any` into `NveMutationWriter`, writing straight into `NodeStore` word tables.

mod apply;
mod builders;
mod clear;
mod layout_ext;
mod merge;
mod paint;

pub use builders::{
    parse_flex_align, parse_flex_direction, parse_flex_justify, PendingResponsiveRule,
    PendingStateRule, PendingStylePatch,
};

use std::borrow::Cow;
use std::rc::Rc;

use dioxus_core::{AttributeValue, IntoAttributeValue};
use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_store::Length;
use nve_tables::interaction::CursorKind;
use nve_tables::layout_ext::LayoutExt;
use nve_tables::media::MediaQuery;
use nve_tables::paint::BackgroundFill;
use nve_tables::tables::{
    BoxShadow, FlexWrap, FontStyle, ImageFit, Overflow, OverflowWrap, TextAlign, TextDecoration,
    WhiteSpace,
};
use nve_tables::transform::Transform2D;
use nve_tables::{LayoutSnapshot, VisualSnapshot};

use merge::{
    LayoutExtMask, LayoutMask, StyleMask, L_ALIGN_ITEMS, L_ALIGN_SELF, L_FLEX_DIR, L_FLEX_GROW,
    L_FLEX_SHRINK, L_GAP, L_HEIGHT, L_JUSTIFY, L_MAR_ALL, L_MAR_BOTTOM, L_MAR_LEFT, L_MAR_RIGHT,
    L_MAR_TOP, L_MAR_X, L_MAR_Y, L_MAX_HEIGHT, L_MAX_WIDTH, L_MIN_HEIGHT, L_MIN_WIDTH, L_PAD_ALL,
    L_PAD_BOTTOM, L_PAD_LEFT, L_PAD_RIGHT, L_PAD_TOP, L_PAD_X, L_PAD_Y, L_WIDTH,
};

/// Attribute wire name recognized by [`crate::NveMutationWriter`] for direct [`TypedStyle`] props.
pub const NVE_TYPED_STYLE_ATTR: &str = "nve_style";

/// Attribute wire name recognized by [`crate::NveMutationWriter`] for standalone [`MediaQuery`]
/// conditional visibility (`nve_show_on`).
pub const NVE_SHOW_ON_ATTR: &str = "nve_show_on";

/// Ergonomic `IntoAttributeValue` wrapper carrying a [`MediaQuery`] into `rsx!` `nve_show_on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShowOn(pub MediaQuery);

impl From<MediaQuery> for ShowOn {
    #[inline]
    fn from(query: MediaQuery) -> Self {
        Self(query)
    }
}

impl IntoAttributeValue for ShowOn {
    #[inline]
    fn into_value(self) -> AttributeValue {
        AttributeValue::Any(Rc::new(self.0))
    }
}

/// Direct binary layout + visual style payload forwarded via Dioxus `AttributeValue::Any`.
///
/// Writes directly into `NodeStore` word tables (`Layout`, `Visual`, `Typography`, `Transform`,
/// `Interaction`, `Responsive`, `GridLayout`, `GridItem`, `Image`) without formatting or parsing
/// any CSS strings. Capability-module tables (e.g. `nve_motion::Motion`) attach via `nve_table`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStyle {
    pub(crate) layout: LayoutSnapshot,
    pub(crate) style: VisualSnapshot,
    pub(crate) layout_mask: LayoutMask,
    pub(crate) style_mask: StyleMask,
    pub(crate) layout_ext_mask: LayoutExtMask,
    pub(crate) overflow_axes: u8,
    pub flex_wrap: Option<FlexWrap>,
    pub font_family: Option<Cow<'static, str>>,
    pub line_height: Option<Length>,
    pub font_weight: Option<u16>,
    pub font_style: Option<FontStyle>,
    pub text_align: Option<TextAlign>,
    pub white_space: Option<WhiteSpace>,
    pub overflow_wrap: Option<OverflowWrap>,
    pub text_decoration: Option<TextDecoration>,
    /// `Some(empty)` explicitly clears shadows; `None` leaves them untouched.
    pub box_shadows: Option<std::sync::Arc<[BoxShadow]>>,
    pub(crate) overflow: Option<(Overflow, Overflow)>,
    pub object_fit: Option<ImageFit>,
    pub(crate) layout_ext: Option<Box<LayoutExt>>,
    pub transform: Option<Transform2D>,
    pub background_fill: Option<BackgroundFill>,
    pub show_on: Option<MediaQuery>,
    pub responsive: Vec<PendingResponsiveRule>,
    pub cursor: Option<CursorKind>,
    pub state_rules: Vec<PendingStateRule>,
}

impl Default for TypedStyle {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl TypedStyle {
    #[inline]
    pub fn new() -> Self {
        Self {
            layout: LayoutSnapshot::default(),
            style: VisualSnapshot {
                text_color: ColorRgba::TRANSPARENT,
                font_size: 0.0,
                ..VisualSnapshot::default()
            },
            layout_mask: LayoutMask::EMPTY,
            style_mask: StyleMask::EMPTY,
            layout_ext_mask: LayoutExtMask::EMPTY,
            overflow_axes: 0,
            flex_wrap: None,
            font_family: None,
            line_height: None,
            font_weight: None,
            font_style: None,
            text_align: None,
            white_space: None,
            overflow_wrap: None,
            text_decoration: None,
            box_shadows: None,
            overflow: None,
            object_fit: None,
            layout_ext: None,
            transform: None,
            background_fill: None,
            show_on: None,
            responsive: Vec::new(),
            cursor: None,
            state_rules: Vec::new(),
        }
    }

    /// Shorthand for [`Self::new()`] with `flex-direction: row`.
    #[inline]
    pub fn row() -> Self {
        Self::new().flex_row()
    }

    /// Shorthand for [`Self::new()`] with `flex-direction: column`.
    #[inline]
    pub fn col() -> Self {
        Self::new().flex_col()
    }

    /// Read-only view of the typed layout properties (mutate via builder methods so field masks
    /// stay in sync).
    #[inline]
    pub const fn layout(&self) -> &LayoutSnapshot {
        &self.layout
    }

    /// Read-only view of the typed visual style properties (mutate via builder methods so field
    /// masks stay in sync).
    #[inline]
    pub const fn style(&self) -> &VisualSnapshot {
        &self.style
    }

    /// Read-only view of the `(overflow_x, overflow_y)` pair when either axis was set.
    #[inline]
    pub const fn overflow_value(&self) -> Option<(Overflow, Overflow)> {
        self.overflow
    }

    /// Read-only view of extended layout properties (mutate via builder methods so field masks
    /// stay in sync).
    #[inline]
    pub fn layout_ext(&self) -> Option<&LayoutExt> {
        self.layout_ext.as_deref()
    }

    #[inline]
    pub(super) fn sanitize(v: f32) -> f32 {
        let max = nve_abi::num::MAX_STYLE_SCALAR;
        if v.is_finite() { v.clamp(-max, max) } else { 0.0 }
    }

    #[inline]
    fn sanitize_dim(dim: Dimension) -> Dimension {
        match dim {
            Dimension::Auto => Dimension::Auto,
            Dimension::Points(v) => Dimension::Points(Self::sanitize(v)),
            Dimension::Percent(v) => Dimension::Percent(Self::sanitize(v)),
        }
    }

    #[inline]
    pub(super) fn ext_mut(&mut self) -> &mut LayoutExt {
        self.layout_ext
            .get_or_insert_with(|| Box::new(LayoutExt::default()))
    }

    // --- Layout dimensions ---

    #[inline]
    pub fn width(mut self, dim: Dimension) -> Self {
        self.layout.width = Self::sanitize_dim(dim);
        self.layout_mask |= L_WIDTH;
        self
    }

    #[inline]
    pub fn width_px(self, v: f32) -> Self {
        self.width(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn width_pct(self, v: f32) -> Self {
        self.width(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0)))
    }

    #[inline]
    pub fn width_auto(self) -> Self {
        self.width(Dimension::Auto)
    }

    #[inline]
    pub fn height(mut self, dim: Dimension) -> Self {
        self.layout.height = Self::sanitize_dim(dim);
        self.layout_mask |= L_HEIGHT;
        self
    }

    #[inline]
    pub fn height_px(self, v: f32) -> Self {
        self.height(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn height_pct(self, v: f32) -> Self {
        self.height(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0)))
    }

    #[inline]
    pub fn height_auto(self) -> Self {
        self.height(Dimension::Auto)
    }

    #[inline]
    pub fn min_width(mut self, dim: Dimension) -> Self {
        self.layout.min_width = Self::sanitize_dim(dim);
        self.layout_mask |= L_MIN_WIDTH;
        self
    }

    #[inline]
    pub fn min_width_px(self, v: f32) -> Self {
        self.min_width(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn min_width_pct(self, v: f32) -> Self {
        self.min_width(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0)))
    }

    #[inline]
    pub fn min_height(mut self, dim: Dimension) -> Self {
        self.layout.min_height = Self::sanitize_dim(dim);
        self.layout_mask |= L_MIN_HEIGHT;
        self
    }

    #[inline]
    pub fn min_height_px(self, v: f32) -> Self {
        self.min_height(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn min_height_pct(self, v: f32) -> Self {
        self.min_height(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0)))
    }

    #[inline]
    pub fn max_width(mut self, dim: Dimension) -> Self {
        self.layout.max_width = Self::sanitize_dim(dim);
        self.layout_mask |= L_MAX_WIDTH;
        self
    }

    #[inline]
    pub fn max_width_px(self, v: f32) -> Self {
        self.max_width(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn max_width_pct(self, v: f32) -> Self {
        self.max_width(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0)))
    }

    #[inline]
    pub fn max_height(mut self, dim: Dimension) -> Self {
        self.layout.max_height = Self::sanitize_dim(dim);
        self.layout_mask |= L_MAX_HEIGHT;
        self
    }

    #[inline]
    pub fn max_height_px(self, v: f32) -> Self {
        self.max_height(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn max_height_pct(self, v: f32) -> Self {
        self.max_height(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0)))
    }

    // --- Flexbox layout ---

    #[inline]
    pub fn flex_direction(mut self, dir: FlexDirection) -> Self {
        self.layout.flex_direction = dir;
        self.layout_mask |= L_FLEX_DIR;
        self
    }

    #[inline]
    pub fn flex_direction_str(self, dir: &str) -> Self {
        self.flex_direction(parse_flex_direction(dir))
    }

    #[inline]
    pub fn flex_row(self) -> Self {
        self.flex_direction(FlexDirection::Row)
    }

    #[inline]
    pub fn flex_col(self) -> Self {
        self.flex_direction(FlexDirection::Column)
    }

    #[inline]
    pub fn flex_row_reverse(self) -> Self {
        self.flex_direction(FlexDirection::RowReverse)
    }

    #[inline]
    pub fn flex_col_reverse(self) -> Self {
        self.flex_direction(FlexDirection::ColumnReverse)
    }

    #[inline]
    pub fn flex_wrap(mut self, wrap: FlexWrap) -> Self {
        self.flex_wrap = Some(wrap);
        self
    }

    #[inline]
    pub fn justify_content(mut self, justify: FlexJustify) -> Self {
        self.layout.justify_content = justify;
        self.layout_mask |= L_JUSTIFY;
        self
    }

    /// Shorthand for [`Self::justify_content`].
    #[inline]
    pub fn justify(self, justify: FlexJustify) -> Self {
        self.justify_content(justify)
    }

    #[inline]
    pub fn justify_content_str(self, justify: &str) -> Self {
        self.justify_content(parse_flex_justify(justify))
    }

    #[inline]
    pub fn align_self(mut self, align: nve_tables::tables::AlignSelf) -> Self {
        self.layout.align_self = align;
        self.layout_mask |= L_ALIGN_SELF;
        self
    }

    #[inline]
    pub fn align_items(mut self, align: FlexAlign) -> Self {
        self.layout.align_items = align;
        self.layout_mask |= L_ALIGN_ITEMS;
        self
    }

    /// Shorthand for [`Self::align_items`].
    #[inline]
    pub fn align(self, align: FlexAlign) -> Self {
        self.align_items(align)
    }

    #[inline]
    pub fn align_items_str(self, align: &str) -> Self {
        self.align_items(parse_flex_align(align))
    }

    /// CSS `flex: <grow>` shorthand (`flex-grow: grow; flex-shrink: 1; flex-basis: 0%`).
    #[inline]
    pub fn flex(self, grow: f32) -> Self {
        self.flex_grow(grow).flex_shrink(1.0).flex_basis_pct(0.0)
    }

    /// Shorthand for `flex(1.0)` (`flex-grow: 1; flex-shrink: 1; flex-basis: 0%`).
    #[inline]
    pub fn flex_1(self) -> Self {
        self.flex(1.0)
    }

    #[inline]
    pub fn flex_grow(mut self, v: f32) -> Self {
        self.layout.flex_grow = Self::sanitize(v).max(0.0);
        self.layout_mask |= L_FLEX_GROW;
        self
    }

    #[inline]
    pub fn flex_shrink(mut self, v: f32) -> Self {
        self.layout.flex_shrink = Self::sanitize(v).max(0.0);
        self.layout_mask |= L_FLEX_SHRINK;
        self
    }

    #[inline]
    pub fn gap(mut self, v: f32) -> Self {
        self.layout.gap = Self::sanitize(v).max(0.0);
        self.layout_mask |= L_GAP;
        self
    }

    // --- Box spacing ---

    #[inline]
    pub fn padding(mut self, v: f32) -> Self {
        self.layout.padding = EdgeInsets::all(Self::sanitize(v).max(0.0));
        self.layout_mask |= L_PAD_ALL;
        self
    }

    #[inline]
    pub fn padding_xy(mut self, horizontal: f32, vertical: f32) -> Self {
        self.layout.padding = EdgeInsets::symmetric(
            Self::sanitize(vertical).max(0.0),
            Self::sanitize(horizontal).max(0.0),
        );
        self.layout_mask |= L_PAD_ALL;
        self
    }

    #[inline]
    pub fn padding_x(mut self, v: f32) -> Self {
        let p = Self::sanitize(v).max(0.0);
        self.layout.padding.left = p;
        self.layout.padding.right = p;
        self.layout_mask |= L_PAD_X;
        self
    }

    #[inline]
    pub fn padding_y(mut self, v: f32) -> Self {
        let p = Self::sanitize(v).max(0.0);
        self.layout.padding.top = p;
        self.layout.padding.bottom = p;
        self.layout_mask |= L_PAD_Y;
        self
    }

    #[inline]
    pub fn padding_top(mut self, v: f32) -> Self {
        self.layout.padding.top = Self::sanitize(v).max(0.0);
        self.layout_mask |= L_PAD_TOP;
        self
    }

    #[inline]
    pub fn padding_right(mut self, v: f32) -> Self {
        self.layout.padding.right = Self::sanitize(v).max(0.0);
        self.layout_mask |= L_PAD_RIGHT;
        self
    }

    #[inline]
    pub fn padding_bottom(mut self, v: f32) -> Self {
        self.layout.padding.bottom = Self::sanitize(v).max(0.0);
        self.layout_mask |= L_PAD_BOTTOM;
        self
    }

    #[inline]
    pub fn padding_left(mut self, v: f32) -> Self {
        self.layout.padding.left = Self::sanitize(v).max(0.0);
        self.layout_mask |= L_PAD_LEFT;
        self
    }

    #[inline]
    pub fn padding_insets(mut self, insets: EdgeInsets) -> Self {
        self.layout.padding = EdgeInsets {
            top: Self::sanitize(insets.top).max(0.0),
            right: Self::sanitize(insets.right).max(0.0),
            bottom: Self::sanitize(insets.bottom).max(0.0),
            left: Self::sanitize(insets.left).max(0.0),
        };
        self.layout_mask |= L_PAD_ALL;
        self
    }

    #[inline]
    pub fn margin(mut self, v: f32) -> Self {
        self.layout.margin = EdgeInsets::all(Self::sanitize(v));
        self.layout_mask |= L_MAR_ALL;
        self
    }

    #[inline]
    pub fn margin_xy(mut self, horizontal: f32, vertical: f32) -> Self {
        self.layout.margin =
            EdgeInsets::symmetric(Self::sanitize(vertical), Self::sanitize(horizontal));
        self.layout_mask |= L_MAR_ALL;
        self
    }

    #[inline]
    pub fn margin_x(mut self, v: f32) -> Self {
        let m = Self::sanitize(v);
        self.layout.margin.left = m;
        self.layout.margin.right = m;
        self.layout_mask |= L_MAR_X;
        self
    }

    #[inline]
    pub fn margin_y(mut self, v: f32) -> Self {
        let m = Self::sanitize(v);
        self.layout.margin.top = m;
        self.layout.margin.bottom = m;
        self.layout_mask |= L_MAR_Y;
        self
    }

    #[inline]
    pub fn margin_top(mut self, v: f32) -> Self {
        self.layout.margin.top = Self::sanitize(v);
        self.layout_mask |= L_MAR_TOP;
        self
    }

    #[inline]
    pub fn margin_right(mut self, v: f32) -> Self {
        self.layout.margin.right = Self::sanitize(v);
        self.layout_mask |= L_MAR_RIGHT;
        self
    }

    #[inline]
    pub fn margin_bottom(mut self, v: f32) -> Self {
        self.layout.margin.bottom = Self::sanitize(v);
        self.layout_mask |= L_MAR_BOTTOM;
        self
    }

    #[inline]
    pub fn margin_left(mut self, v: f32) -> Self {
        self.layout.margin.left = Self::sanitize(v);
        self.layout_mask |= L_MAR_LEFT;
        self
    }

    #[inline]
    pub fn margin_insets(mut self, insets: EdgeInsets) -> Self {
        self.layout.margin = EdgeInsets {
            top: Self::sanitize(insets.top),
            right: Self::sanitize(insets.right),
            bottom: Self::sanitize(insets.bottom),
            left: Self::sanitize(insets.left),
        };
        self.layout_mask |= L_MAR_ALL;
        self
    }
}

impl IntoAttributeValue for TypedStyle {
    #[inline]
    fn into_value(self) -> AttributeValue {
        AttributeValue::Any(Rc::new(self))
    }
}

#[inline]
pub(crate) fn pack_media_query(q: MediaQuery) -> (u32, u32) {
    let widths = u32::from(q.min_width_px) | (u32::from(q.max_width_px) << 16);
    let env = u32::from(q.flags)
        | ((q.color_scheme as u32) << 8)
        | ((q.pointer as u32) << 16);
    (widths, env)
}
