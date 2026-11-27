//! Visual, typography, responsive, and interaction-state builders on [`TypedStyle`].

use std::borrow::Cow;
use std::sync::Arc;

use nve_abi::{ColorRgba, FlexAlign, FlexDirection, FlexJustify};
use nve_components::interaction::{CursorKind, InteractionState};
use nve_components::layout_ext::GridTrack;
use nve_components::media::MediaQuery;
use nve_components::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
use nve_components::StylePatchExt;
use nve_store::RowPatch;

use super::TypedStyle;

/// Staged style patch for [`TypedStyle`] responsive and interaction-state rules.
///
/// Wraps a table-generic [`RowPatch`] (forwarding every [`StylePatchExt`] fluent builder directly)
/// plus optional uninterned child resources (`BackgroundFill`, `GridTrack` lists) that are interned
/// via [`nve_store::TempResources`] during [`TypedStyle::apply_to_tx`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingStylePatch {
    pub patch: RowPatch,
    pub background_fill: Option<BackgroundFill>,
    pub grid_template_columns: Option<Arc<[GridTrack]>>,
    pub grid_template_rows: Option<Arc<[GridTrack]>>,
}

impl PendingStylePatch {
    #[inline]
    pub const fn new() -> Self {
        Self {
            patch: RowPatch::new(),
            background_fill: None,
            grid_template_columns: None,
            grid_template_rows: None,
        }
    }

    #[inline]
    pub fn has_pending_resources(&self) -> bool {
        self.background_fill.is_some()
            || self.grid_template_columns.is_some()
            || self.grid_template_rows.is_some()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.patch.is_empty() && !self.has_pending_resources()
    }

    #[inline]
    pub fn background_fill(mut self, fill: BackgroundFill) -> Self {
        self.background_fill = Some(fill);
        self
    }

    #[inline]
    pub fn linear_gradient(
        mut self,
        angle_deg: f32,
        stops: impl Into<Vec<GradientStop>>,
    ) -> Self {
        self.background_fill = Some(BackgroundFill::Linear(LinearGradient::new(
            angle_deg, stops,
        )));
        self
    }

    #[inline]
    pub fn radial_gradient(
        mut self,
        center_x: f32,
        center_y: f32,
        radius: f32,
        stops: impl Into<Vec<GradientStop>>,
    ) -> Self {
        self.background_fill = Some(BackgroundFill::Radial(RadialGradient::new(
            center_x, center_y, radius, stops,
        )));
        self
    }

    #[inline]
    pub fn grid_cols(mut self, tracks: impl Into<Vec<GridTrack>>) -> Self {
        self.grid_template_columns = Some(Arc::from(tracks.into()));
        self
    }

    #[inline]
    pub fn grid_rows(mut self, tracks: impl Into<Vec<GridTrack>>) -> Self {
        self.grid_template_rows = Some(Arc::from(tracks.into()));
        self
    }
}

impl From<RowPatch> for PendingStylePatch {
    #[inline]
    fn from(patch: RowPatch) -> Self {
        Self {
            patch,
            background_fill: None,
            grid_template_columns: None,
            grid_template_rows: None,
        }
    }
}

impl StylePatchExt for PendingStylePatch {
    #[inline(always)]
    fn row_patch(&self) -> &RowPatch {
        &self.patch
    }

    #[inline(always)]
    fn row_patch_mut(&mut self) -> &mut RowPatch {
        &mut self.patch
    }
}

/// Staged `(MediaQuery, PendingStylePatch)` rule attached to [`TypedStyle`].
#[derive(Debug, Clone, PartialEq)]
pub struct PendingResponsiveRule {
    pub query: MediaQuery,
    pub patch: PendingStylePatch,
}

/// Staged `(InteractionState, PendingStylePatch)` rule attached to [`TypedStyle`].
#[derive(Debug, Clone, PartialEq)]
pub struct PendingStateRule {
    pub state: InteractionState,
    pub patch: PendingStylePatch,
}

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
    pub fn at(mut self, query: MediaQuery, patch: impl Into<PendingStylePatch>) -> Self {
        self.responsive.push(PendingResponsiveRule {
            query,
            patch: patch.into(),
        });
        self
    }

    #[inline]
    pub fn at_with(
        self,
        query: MediaQuery,
        build: impl FnOnce(PendingStylePatch) -> PendingStylePatch,
    ) -> Self {
        self.at(query, build(PendingStylePatch::new()))
    }

    #[inline]
    pub fn on_compact(self, patch: impl Into<PendingStylePatch>) -> Self {
        self.at(MediaQuery::compact(), patch)
    }

    #[inline]
    pub fn on_medium(self, patch: impl Into<PendingStylePatch>) -> Self {
        self.at(MediaQuery::medium(), patch)
    }

    #[inline]
    pub fn on_expanded(self, patch: impl Into<PendingStylePatch>) -> Self {
        self.at(MediaQuery::expanded(), patch)
    }

    // --- Interactive pseudo-state rules & typed pointer cursor ---

    #[inline]
    pub fn cursor(mut self, cursor: CursorKind) -> Self {
        self.cursor = Some(cursor);
        self
    }

    #[inline]
    pub fn on_state(
        mut self,
        state: InteractionState,
        patch: impl Into<PendingStylePatch>,
    ) -> Self {
        self.state_rules.push(PendingStateRule {
            state,
            patch: patch.into(),
        });
        self
    }

    #[inline]
    pub fn on_state_with(
        self,
        state: InteractionState,
        build: impl FnOnce(PendingStylePatch) -> PendingStylePatch,
    ) -> Self {
        self.on_state(state, build(PendingStylePatch::new()))
    }

    #[inline]
    pub fn on_hover(self, patch: impl Into<PendingStylePatch>) -> Self {
        self.on_state(InteractionState::HOVER, patch)
    }

    #[inline]
    pub fn on_hover_with(
        self,
        build: impl FnOnce(PendingStylePatch) -> PendingStylePatch,
    ) -> Self {
        self.on_state_with(InteractionState::HOVER, build)
    }

    #[inline]
    pub fn on_active(self, patch: impl Into<PendingStylePatch>) -> Self {
        self.on_state(InteractionState::ACTIVE, patch)
    }

    #[inline]
    pub fn on_active_with(
        self,
        build: impl FnOnce(PendingStylePatch) -> PendingStylePatch,
    ) -> Self {
        self.on_state_with(InteractionState::ACTIVE, build)
    }

    #[inline]
    pub fn on_focus(self, patch: impl Into<PendingStylePatch>) -> Self {
        self.on_state(InteractionState::FOCUS, patch)
    }

    #[inline]
    pub fn on_focus_with(
        self,
        build: impl FnOnce(PendingStylePatch) -> PendingStylePatch,
    ) -> Self {
        self.on_state_with(InteractionState::FOCUS, build)
    }

    #[inline]
    pub fn on_focus_visible(self, patch: impl Into<PendingStylePatch>) -> Self {
        self.on_state(InteractionState::FOCUS_VISIBLE, patch)
    }

    #[inline]
    pub fn on_focus_visible_with(
        self,
        build: impl FnOnce(PendingStylePatch) -> PendingStylePatch,
    ) -> Self {
        self.on_state_with(InteractionState::FOCUS_VISIBLE, build)
    }

    #[inline]
    pub fn on_disabled(self, patch: impl Into<PendingStylePatch>) -> Self {
        self.on_state(InteractionState::DISABLED, patch)
    }

    #[inline]
    pub fn on_disabled_with(
        self,
        build: impl FnOnce(PendingStylePatch) -> PendingStylePatch,
    ) -> Self {
        self.on_state_with(InteractionState::DISABLED, build)
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
