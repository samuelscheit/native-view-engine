//! Zero-string-parse typed style attribute (`TypedStyle`) passed directly through Dioxus
//! `AttributeValue::Any` into `NveMutationWriter`, writing straight into `NodeStore` word tables.

mod apply;
mod builders;
mod layout_ext;
mod paint;

pub use builders::{
    parse_flex_align, parse_flex_direction, parse_flex_justify, PendingResponsiveRule,
    PendingStateRule, PendingStylePatch,
};

use std::borrow::Cow;
use std::rc::Rc;

use dioxus_core::{AttributeValue, IntoAttributeValue};
use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_tables::interaction::CursorKind;
use nve_tables::layout_ext::LayoutExt;
use nve_tables::media::MediaQuery;
use nve_tables::paint::BackgroundFill;
use nve_tables::tables::BoxShadow;
use nve_tables::transform::Transform2D;
use nve_tables::{LayoutSnapshot, VisualSnapshot};

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
/// `Interaction`, `Responsive`, `GridLayout`, `GridItem`) without formatting or parsing any CSS
/// strings. Capability-module tables (e.g. `nve_motion::Motion`) attach via `nve_table`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStyle {
    pub(crate) layout: LayoutSnapshot,
    pub(crate) style: VisualSnapshot,
    pub(crate) has_layout: bool,
    pub(crate) has_style: bool,
    pub font_weight: Option<Cow<'static, str>>,
    pub text_align: Option<Cow<'static, str>>,
    /// `Some(empty)` explicitly clears shadows; `None` leaves them untouched.
    pub box_shadows: Option<std::sync::Arc<[BoxShadow]>>,
    pub overflow: Option<Cow<'static, str>>,
    pub object_fit: Option<Cow<'static, str>>,
    pub layout_ext: Option<Box<LayoutExt>>,
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
            // Text color and size start unset (alpha 0 / size 0, skipped by `apply`), so a
            // surface-only style never writes an explicit black 16px `Typography` row that
            // would block inherited text color.
            style: VisualSnapshot { text_color: ColorRgba::TRANSPARENT, font_size: 0.0, ..VisualSnapshot::default() },
            has_layout: false,
            has_style: false,
            font_weight: None,
            text_align: None,
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

    /// Read-only view of the typed layout properties (mutate via builder methods so the
    /// `has_layout` presence flag stays in sync).
    #[inline]
    pub const fn layout(&self) -> &LayoutSnapshot {
        &self.layout
    }

    /// Read-only view of the typed visual style properties (mutate via builder methods so the
    /// `has_style` presence flag and the unset text color / font size sentinels stay in sync).
    #[inline]
    pub const fn style(&self) -> &VisualSnapshot {
        &self.style
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
        self.has_layout = true;
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
        self.has_layout = true;
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
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn min_width_px(self, v: f32) -> Self {
        self.min_width(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn min_height(mut self, dim: Dimension) -> Self {
        self.layout.min_height = Self::sanitize_dim(dim);
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn min_height_px(self, v: f32) -> Self {
        self.min_height(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn max_width(mut self, dim: Dimension) -> Self {
        self.layout.max_width = Self::sanitize_dim(dim);
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn max_width_px(self, v: f32) -> Self {
        self.max_width(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn max_height(mut self, dim: Dimension) -> Self {
        self.layout.max_height = Self::sanitize_dim(dim);
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn max_height_px(self, v: f32) -> Self {
        self.max_height(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    // --- Flexbox layout ---

    #[inline]
    pub fn flex_direction(mut self, dir: FlexDirection) -> Self {
        self.layout.flex_direction = dir;
        self.has_layout = true;
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
    pub fn justify_content(mut self, justify: FlexJustify) -> Self {
        self.layout.justify_content = justify;
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn justify_content_str(self, justify: &str) -> Self {
        self.justify_content(parse_flex_justify(justify))
    }

    #[inline]
    pub fn align_self(mut self, align: nve_tables::tables::AlignSelf) -> Self {
        self.layout.align_self = align;
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn align_items(mut self, align: FlexAlign) -> Self {
        self.layout.align_items = align;
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn align_items_str(self, align: &str) -> Self {
        self.align_items(parse_flex_align(align))
    }

    #[inline]
    pub fn flex_grow(mut self, v: f32) -> Self {
        self.layout.flex_grow = Self::sanitize(v).max(0.0);
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn flex_shrink(mut self, v: f32) -> Self {
        self.layout.flex_shrink = Self::sanitize(v).max(0.0);
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn gap(mut self, v: f32) -> Self {
        self.layout.gap = Self::sanitize(v).max(0.0);
        self.has_layout = true;
        self
    }

    // --- Box spacing ---

    #[inline]
    pub fn padding(mut self, v: f32) -> Self {
        self.layout.padding = EdgeInsets::all(Self::sanitize(v).max(0.0));
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn padding_xy(mut self, horizontal: f32, vertical: f32) -> Self {
        self.layout.padding = EdgeInsets::symmetric(
            Self::sanitize(vertical).max(0.0),
            Self::sanitize(horizontal).max(0.0),
        );
        self.has_layout = true;
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
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn margin(mut self, v: f32) -> Self {
        self.layout.margin = EdgeInsets::all(Self::sanitize(v));
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn margin_xy(mut self, horizontal: f32, vertical: f32) -> Self {
        self.layout.margin =
            EdgeInsets::symmetric(Self::sanitize(vertical), Self::sanitize(horizontal));
        self.has_layout = true;
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
        self.has_layout = true;
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
