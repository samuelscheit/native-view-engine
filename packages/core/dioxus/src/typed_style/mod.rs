//! Zero-string-parse typed style attribute (`TypedStyle`) passed directly through Dioxus
//! `AttributeValue::Any` into `NveMutationWriter`.
//!
//! Eliminates the `StyleBuf` CSS string formatting -> `apply_typed_attr_with_raw` string
//! reparsing round-trip on hot UI render paths while staying 100% compatible with optional
//! inline `style: "..."` user overrides.

mod layout_ext;

use std::borrow::Cow;
use std::rc::Rc;

use dioxus_core::{AttributeValue, IntoAttributeValue};
use nve_abi::{
    ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify, LayoutProps,
    StyleProps,
};
use nve_components::interaction::{CursorKind, InteractionState, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::style_patch::StylePatch;
use nve_components::transform::Transform2D;

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
/// Copies `LayoutProps`, `StyleProps`, optional raw style fields (`font-weight`, `text-align`,
/// `box-shadow`, `overflow`, `object-fit`), lazy extended layout (`LayoutExt`), `Transform2D`,
/// and optional responsive `@media` / interactive pseudo-state / transition rules straight into
/// `NodeRecord` inside `NveMutationWriter` without formatting or parsing any CSS.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStyle {
    pub layout: LayoutProps,
    pub style: StyleProps,
    pub has_layout: bool,
    pub has_style: bool,
    pub font_weight: Option<Cow<'static, str>>,
    pub text_align: Option<Cow<'static, str>>,
    pub box_shadow: Option<Cow<'static, str>>,
    pub overflow: Option<Cow<'static, str>>,
    pub object_fit: Option<Cow<'static, str>>,
    pub layout_ext: Option<Box<LayoutExt>>,
    pub transform: Option<Transform2D>,
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
    /// Creates an empty `TypedStyle` with default ABI `LayoutProps`/`StyleProps` and both
    /// `has_layout` and `has_style` cleared.
    #[inline]
    pub fn new() -> Self {
        Self {
            layout: LayoutProps::default(),
            style: StyleProps::default(),
            has_layout: false,
            has_style: false,
            font_weight: None,
            text_align: None,
            box_shadow: None,
            overflow: None,
            object_fit: None,
            layout_ext: None,
            transform: None,
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

    // --- Visual style ---

    #[inline]
    pub fn bg(mut self, color: ColorRgba) -> Self {
        self.style.background = color;
        self.has_style = true;
        self
    }

    #[inline]
    pub fn background(self, color: ColorRgba) -> Self {
        self.bg(color)
    }

    #[inline]
    pub fn color(mut self, color: ColorRgba) -> Self {
        self.style.text_color = color;
        self.has_style = true;
        self
    }

    #[inline]
    pub fn text_color(self, color: ColorRgba) -> Self {
        self.color(color)
    }

    #[inline]
    pub fn border(mut self, width: f32, color: ColorRgba) -> Self {
        self.style.border_width = Self::sanitize(width).max(0.0);
        self.style.border_color = color;
        self.has_style = true;
        self
    }

    #[inline]
    pub fn border_width(mut self, width: f32) -> Self {
        self.style.border_width = Self::sanitize(width).max(0.0);
        self.has_style = true;
        self
    }

    #[inline]
    pub fn border_color(mut self, color: ColorRgba) -> Self {
        self.style.border_color = color;
        self.has_style = true;
        self
    }

    #[inline]
    pub fn radius(mut self, radius: f32) -> Self {
        self.style.border_radius = Self::sanitize(radius).max(0.0);
        self.has_style = true;
        self
    }

    #[inline]
    pub fn border_radius(self, radius: f32) -> Self {
        self.radius(radius)
    }

    #[inline]
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.style.opacity = Self::sanitize(opacity).clamp(0.0, 1.0);
        self.has_style = true;
        self
    }

    #[inline]
    pub fn font_size(mut self, px: f32) -> Self {
        self.style.font_size = Self::sanitize(px).max(1.0);
        self.has_style = true;
        self
    }

    #[inline]
    pub fn z_index(mut self, z: i32) -> Self {
        self.style.z_index = z;
        self.has_style = true;
        self
    }

    // --- Non-ABI raw style fields ---

    #[inline]
    pub fn font_weight(mut self, weight: impl Into<Cow<'static, str>>) -> Self {
        self.font_weight = Some(weight.into());
        self
    }

    #[inline]
    pub fn text_align(mut self, align: impl Into<Cow<'static, str>>) -> Self {
        self.text_align = Some(align.into());
        self
    }

    #[inline]
    pub fn box_shadow(mut self, shadow: impl Into<Cow<'static, str>>) -> Self {
        self.box_shadow = Some(shadow.into());
        self
    }

    #[inline]
    pub fn overflow(mut self, mode: impl Into<Cow<'static, str>>) -> Self {
        self.overflow = Some(mode.into());
        self
    }

    #[inline]
    pub fn object_fit(mut self, mode: impl Into<Cow<'static, str>>) -> Self {
        self.object_fit = Some(mode.into());
        self
    }

    // --- Declarative responsive visibility & `@media` style rules ---

    #[inline]
    pub fn show_on(mut self, query: MediaQuery) -> Self {
        self.show_on = Some(query);
        self
    }

    #[inline]
    pub fn at(mut self, query: MediaQuery, patch: StylePatch) -> Self {
        self.responsive.push(ResponsiveRule::new(query, patch));
        self
    }

    #[inline]
    pub fn at_with(
        self,
        query: MediaQuery,
        build: impl FnOnce(StylePatch) -> StylePatch,
    ) -> Self {
        self.at(query, build(StylePatch::new()))
    }

    #[inline]
    pub fn on_compact(self, patch: StylePatch) -> Self {
        self.at(MediaQuery::compact(), patch)
    }

    #[inline]
    pub fn on_medium(self, patch: StylePatch) -> Self {
        self.at(MediaQuery::medium(), patch)
    }

    #[inline]
    pub fn on_expanded(self, patch: StylePatch) -> Self {
        self.at(MediaQuery::expanded(), patch)
    }

    // --- Interactive pseudo-state rules & typed pointer cursor ---

    #[inline]
    pub fn cursor(mut self, cursor: CursorKind) -> Self {
        self.cursor = Some(cursor);
        self
    }

    #[inline]
    pub fn on_state(mut self, state: InteractionState, patch: StylePatch) -> Self {
        self.state_rules.push(StateStyleRule::new(state, patch));
        self
    }

    #[inline]
    pub fn on_state_with(
        self,
        state: InteractionState,
        build: impl FnOnce(StylePatch) -> StylePatch,
    ) -> Self {
        self.on_state(state, build(StylePatch::new()))
    }

    #[inline]
    pub fn on_hover(self, patch: StylePatch) -> Self {
        self.on_state(InteractionState::HOVER, patch)
    }

    #[inline]
    pub fn on_hover_with(self, build: impl FnOnce(StylePatch) -> StylePatch) -> Self {
        self.on_state_with(InteractionState::HOVER, build)
    }

    #[inline]
    pub fn on_active(self, patch: StylePatch) -> Self {
        self.on_state(InteractionState::ACTIVE, patch)
    }

    #[inline]
    pub fn on_active_with(self, build: impl FnOnce(StylePatch) -> StylePatch) -> Self {
        self.on_state_with(InteractionState::ACTIVE, build)
    }

    #[inline]
    pub fn on_focus(self, patch: StylePatch) -> Self {
        self.on_state(InteractionState::FOCUS, patch)
    }

    #[inline]
    pub fn on_focus_with(self, build: impl FnOnce(StylePatch) -> StylePatch) -> Self {
        self.on_state_with(InteractionState::FOCUS, build)
    }

    #[inline]
    pub fn on_focus_visible(self, patch: StylePatch) -> Self {
        self.on_state(InteractionState::FOCUS_VISIBLE, patch)
    }

    #[inline]
    pub fn on_focus_visible_with(self, build: impl FnOnce(StylePatch) -> StylePatch) -> Self {
        self.on_state_with(InteractionState::FOCUS_VISIBLE, build)
    }

    #[inline]
    pub fn on_disabled(self, patch: StylePatch) -> Self {
        self.on_state(InteractionState::DISABLED, patch)
    }

    #[inline]
    pub fn on_disabled_with(self, build: impl FnOnce(StylePatch) -> StylePatch) -> Self {
        self.on_state_with(InteractionState::DISABLED, build)
    }

    // --- Declarative style transitions ---

    #[inline]
    pub fn transition(mut self, spec: TransitionSpec) -> Self {
        self.transitions.push(spec);
        self
    }

    #[inline]
    pub fn transitions(mut self, specs: &[TransitionSpec]) -> Self {
        self.transitions.extend_from_slice(specs);
        self
    }
}

impl IntoAttributeValue for TypedStyle {
    #[inline]
    fn into_value(self) -> AttributeValue {
        AttributeValue::Any(Rc::new(self))
    }
}

/// Parses a CSS/RN `flex-direction` keyword into [`FlexDirection`] with zero allocations.
#[inline]
pub fn parse_flex_direction(value: &str) -> FlexDirection {
    match value.trim() {
        "row" => FlexDirection::Row,
        "column-reverse" | "columnReverse" => FlexDirection::ColumnReverse,
        "row-reverse" | "rowReverse" => FlexDirection::RowReverse,
        _ => FlexDirection::Column,
    }
}

/// Parses a CSS/RN `justify-content` keyword into [`FlexJustify`] with zero allocations.
#[inline]
pub fn parse_flex_justify(value: &str) -> FlexJustify {
    match value.trim() {
        "center" => FlexJustify::Center,
        "flex-end" | "end" => FlexJustify::FlexEnd,
        "space-between" | "spaceBetween" => FlexJustify::SpaceBetween,
        "space-around" | "spaceAround" => FlexJustify::SpaceAround,
        "space-evenly" | "spaceEvenly" => FlexJustify::SpaceEvenly,
        _ => FlexJustify::FlexStart,
    }
}

/// Parses a CSS/RN `align-items` keyword into [`FlexAlign`] with zero allocations.
#[inline]
pub fn parse_flex_align(value: &str) -> FlexAlign {
    match value.trim() {
        "flex-start" | "start" => FlexAlign::FlexStart,
        "center" => FlexAlign::Center,
        "flex-end" | "end" => FlexAlign::FlexEnd,
        "baseline" => FlexAlign::Baseline,
        _ => FlexAlign::Stretch,
    }
}
