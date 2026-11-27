//! Zero-string-parse typed style attribute (`TypedStyle`) passed directly through Dioxus
//! `AttributeValue::Any` into `NveMutationWriter`, writing straight into `NodeStore` word tables.

mod apply;
mod builders;
mod layout_ext;
mod paint;

pub use builders::{parse_flex_align, parse_flex_direction, parse_flex_justify};

use std::borrow::Cow;
use std::rc::Rc;

use dioxus_core::{AttributeValue, IntoAttributeValue};
use nve_abi::{Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_components::interaction::{CursorKind, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::paint::BackgroundFill;
use nve_components::transform::Transform2D;
use nve_components::{LayoutSnapshot, VisualSnapshot};

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
/// `Interaction`, `Responsive`, `Motion`, `GridLayout`, `GridItem`) without formatting or
/// parsing any CSS strings.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStyle {
    pub layout: LayoutSnapshot,
    pub style: VisualSnapshot,
    pub has_layout: bool,
    pub has_style: bool,
    pub font_weight: Option<Cow<'static, str>>,
    pub text_align: Option<Cow<'static, str>>,
    pub box_shadow: Option<Cow<'static, str>>,
    pub overflow: Option<Cow<'static, str>>,
    pub object_fit: Option<Cow<'static, str>>,
    pub layout_ext: Option<Box<LayoutExt>>,
    pub transform: Option<Transform2D>,
    pub background_fill: Option<BackgroundFill>,
    pub show_on: Option<MediaQuery>,
    pub responsive: Vec<ResponsiveRule>,
    pub cursor: Option<CursorKind>,
    pub state_rules: Vec<StateStyleRule>,
    pub transitions: Vec<TransitionSpec>,
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
            style: VisualSnapshot::default(),
            has_layout: false,
            has_style: false,
            font_weight: None,
            text_align: None,
            box_shadow: None,
            overflow: None,
            object_fit: None,
            layout_ext: None,
            transform: None,
            background_fill: None,
            show_on: None,
            responsive: Vec::new(),
            cursor: None,
            state_rules: Vec::new(),
            transitions: Vec::new(),
        }
    }

    #[inline]
    pub(super) fn sanitize(v: f32) -> f32 {
        if v.is_finite() { v } else { 0.0 }
    }

    #[inline]
    pub(super) fn ext_mut(&mut self) -> &mut LayoutExt {
        self.layout_ext
            .get_or_insert_with(|| Box::new(LayoutExt::default()))
    }

    // --- Layout dimensions ---

    #[inline]
    pub fn width(mut self, dim: Dimension) -> Self {
        self.layout.width = dim;
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
        self.layout.height = dim;
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
        self.layout.min_width = dim;
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn min_width_px(self, v: f32) -> Self {
        self.min_width(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn min_height(mut self, dim: Dimension) -> Self {
        self.layout.min_height = dim;
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn min_height_px(self, v: f32) -> Self {
        self.min_height(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn max_width(mut self, dim: Dimension) -> Self {
        self.layout.max_width = dim;
        self.has_layout = true;
        self
    }

    #[inline]
    pub fn max_width_px(self, v: f32) -> Self {
        self.max_width(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn max_height(mut self, dim: Dimension) -> Self {
        self.layout.max_height = dim;
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
