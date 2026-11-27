//! Visual, typography, responsive, interaction-state, and transition builders on [`TypedStyle`].

use std::borrow::Cow;

use nve_abi::{ColorRgba, FlexAlign, FlexDirection, FlexJustify};
use nve_components::interaction::{CursorKind, InteractionState, StateStyleRule};
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::style_patch::StylePatch;

use super::TypedStyle;

impl TypedStyle {
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

    // --- Typography / Shadow / Overflow / Image fit ---

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

    // --- Responsive visibility & `@media` style rules ---

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

#[inline]
pub fn parse_flex_direction(value: &str) -> FlexDirection {
    match value.trim() {
        "row" => FlexDirection::Row,
        "column-reverse" | "columnReverse" => FlexDirection::ColumnReverse,
        "row-reverse" | "rowReverse" => FlexDirection::RowReverse,
        _ => FlexDirection::Column,
    }
}

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
