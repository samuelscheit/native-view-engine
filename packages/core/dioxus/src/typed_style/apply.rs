//! Direct `TypedStyle` application and diff-clearing onto `NodeStore` word tables.

use std::sync::Arc;

use nve_abi::{Dimension, FlexDirection};
use nve_store::{Length, NodeId, Rgba, RowPatch, TempResources, Transaction};
use nve_tables::interaction::{CursorKind, StateStyleRule};
use nve_tables::layout_ext::{LayoutDirection, PositionMode};
use nve_tables::media::ResponsiveRule;
use nve_tables::tables::{
    AlignItems, AlignSelf, BackgroundFillRes, BoxShadows, Direction, Display, FlexWrap, GridItem,
    GridLayout, GridTracks, Image, ImageFit, Interaction, JustifyContent, Layout, Overflow,
    Position, Responsive, ResponsiveRules, StateRules, Transform, Typography, Visual,
};

use super::merge::{
    E_ASPECT_RATIO, E_COL_GAP, E_DIRECTION, E_DISPLAY, E_FLEX_BASIS, E_INSET_BOTTOM, E_INSET_LEFT,
    E_INSET_RIGHT, E_INSET_TOP, E_MAR_INLINE, E_PAD_INLINE, E_POSITION, E_ROW_GAP, L_ALIGN_ITEMS,
    L_ALIGN_SELF, L_FLEX_DIR, L_FLEX_GROW, L_FLEX_SHRINK, L_GAP, L_HEIGHT, L_JUSTIFY, L_MAR_BOTTOM,
    L_MAR_LEFT, L_MAR_RIGHT, L_MAR_TOP, L_MAX_HEIGHT, L_MAX_WIDTH, L_MIN_HEIGHT, L_MIN_WIDTH,
    L_PAD_BOTTOM, L_PAD_LEFT, L_PAD_RIGHT, L_PAD_TOP, L_WIDTH, OV_X, OV_Y, S_BG, S_BORDER_COLOR,
    S_BORDER_RADIUS, S_BORDER_WIDTH, S_FONT_SIZE, S_OPACITY, S_TEXT_COLOR, S_VISUAL_BITS,
    S_Z_INDEX,
};
use super::{pack_media_query, PendingStylePatch, TypedStyle};

impl TypedStyle {
    /// Writes only the `{Layout, Visual, Typography}` tables selected by `layers` into `node`
    /// without touching unselected style tables or unrelated tables (`Transform`, `Motion`, etc.).
    pub fn apply_style_layers_to_tx(&self, tx: &mut Transaction<'_>, node: NodeId, layers: u8) {
        if layers & nve_tables::STYLE_LAYER_LAYOUT != 0 {
            self.apply_layout_layer(tx, node);
        }
        if layers & nve_tables::STYLE_LAYER_VISUAL != 0 {
            self.apply_visual_layer(tx, node);
        }
        if layers & nve_tables::STYLE_LAYER_TYPOGRAPHY != 0 {
            self.apply_typography_layer(tx, node);
        }
    }

    fn apply_layout_layer(&self, tx: &mut Transaction<'_>, node: NodeId) {
        let lm = self.layout_mask.bits();
        if lm != 0 || self.flex_wrap.is_some() {
            let l = &self.layout;
            let mut row = tx.edit::<Layout>(node);
            if lm & L_WIDTH != 0 { row = row.width(dim_to_length(l.width)); }
            if lm & L_HEIGHT != 0 { row = row.height(dim_to_length(l.height)); }
            if lm & L_MIN_WIDTH != 0 { row = row.min_width(dim_to_length(l.min_width)); }
            if lm & L_MIN_HEIGHT != 0 { row = row.min_height(dim_to_length(l.min_height)); }
            if lm & L_MAX_WIDTH != 0 { row = row.max_width(dim_to_length(l.max_width)); }
            if lm & L_MAX_HEIGHT != 0 { row = row.max_height(dim_to_length(l.max_height)); }
            if lm & L_FLEX_DIR != 0 { row = row.flex_direction(l.flex_direction); }
            if lm & L_JUSTIFY != 0 { row = row.justify_content(l.justify_content.into()); }
            if lm & L_ALIGN_ITEMS != 0 { row = row.align_items(l.align_items.into()); }
            if lm & L_ALIGN_SELF != 0 { row = row.align_self(l.align_self); }
            if lm & L_FLEX_GROW != 0 { row = row.flex_grow(l.flex_grow); }
            if lm & L_FLEX_SHRINK != 0 { row = row.flex_shrink(l.flex_shrink); }
            if lm & L_GAP != 0 {
                let g = Length::points(l.gap);
                row = row.row_gap(g).column_gap(g);
            }
            if lm & L_PAD_TOP != 0 { row = row.padding_top(Length::points(l.padding.top)); }
            if lm & L_PAD_RIGHT != 0 { row = row.padding_right(Length::points(l.padding.right)); }
            if lm & L_PAD_BOTTOM != 0 { row = row.padding_bottom(Length::points(l.padding.bottom)); }
            if lm & L_PAD_LEFT != 0 { row = row.padding_left(Length::points(l.padding.left)); }
            if lm & L_MAR_TOP != 0 { row = row.margin_top(Length::points(l.margin.top)); }
            if lm & L_MAR_RIGHT != 0 { row = row.margin_right(Length::points(l.margin.right)); }
            if lm & L_MAR_BOTTOM != 0 { row = row.margin_bottom(Length::points(l.margin.bottom)); }
            if lm & L_MAR_LEFT != 0 { row = row.margin_left(Length::points(l.margin.left)); }
            if let Some(fw) = self.flex_wrap {
                row = row.flex_wrap(fw);
            }
            let _ = row;
        }
        if let Some(ref ext) = self.layout_ext {
            let em = self.effective_ext_mask();
            let mut row = tx.edit::<Layout>(node);
            if em & E_DISPLAY != 0 {
                row = row.display(ext.display);
            }
            if em & E_POSITION != 0 {
                row = row.position(match ext.position {
                    PositionMode::Relative => Position::Relative,
                    PositionMode::Absolute => Position::Absolute,
                    PositionMode::Sticky => Position::Sticky,
                });
            }
            if em & E_FLEX_BASIS != 0 {
                row = row.flex_basis(dim_to_length(ext.flex_basis));
            }
            if em & E_INSET_TOP != 0 { row = row.inset_top(dim_to_length(ext.insets.top)); }
            if em & E_INSET_RIGHT != 0 { row = row.inset_right(dim_to_length(ext.insets.right)); }
            if em & E_INSET_BOTTOM != 0 { row = row.inset_bottom(dim_to_length(ext.insets.bottom)); }
            if em & E_INSET_LEFT != 0 { row = row.inset_left(dim_to_length(ext.insets.left)); }
            if em & E_ASPECT_RATIO != 0
                && let Some(ar) = ext.aspect_ratio
            {
                row = row.aspect_ratio(Some(ar));
            }
            if em & E_DIRECTION != 0
                && let Some(dir) = ext.direction
            {
                row = row.direction(match dir {
                    LayoutDirection::Ltr => Direction::Ltr,
                    LayoutDirection::Rtl => Direction::Rtl,
                });
            }
            if em & E_PAD_INLINE != 0
                && (ext.padding_inline.start.is_some() || ext.padding_inline.end.is_some())
            {
                row = row.padding_inline_logical(true);
                if let Some(v) = ext.padding_inline.start {
                    row = row.padding_left(Length::points(v));
                }
                if let Some(v) = ext.padding_inline.end {
                    row = row.padding_right(Length::points(v));
                }
            }
            if em & E_MAR_INLINE != 0
                && (ext.margin_inline.start.is_some() || ext.margin_inline.end.is_some())
            {
                row = row.margin_inline_logical(true);
                if let Some(v) = ext.margin_inline.start {
                    row = row.margin_left(Length::points(v));
                }
                if let Some(v) = ext.margin_inline.end {
                    row = row.margin_right(Length::points(v));
                }
            }
            if let Some(ref g) = ext.grid {
                if em & E_COL_GAP != 0
                    && let Some(cg) = g.column_gap
                {
                    row = row.column_gap(Length::points(cg));
                }
                if em & E_ROW_GAP != 0
                    && let Some(rg) = g.row_gap
                {
                    row = row.row_gap(Length::points(rg));
                }
            }
            let _ = row;
        }
    }

    fn apply_visual_layer(&self, tx: &mut Transaction<'_>, node: NodeId) {
        let vm = self.style_mask.bits() & S_VISUAL_BITS;
        if vm != 0 {
            let s = &self.style;
            let mut vis = tx.edit::<Visual>(node);
            if vm & S_BG != 0 { vis = vis.background(Rgba(s.background.to_u32())); }
            if vm & S_BORDER_COLOR != 0 { vis = vis.border_color(Rgba(s.border_color.to_u32())); }
            if vm & S_BORDER_WIDTH != 0 { vis = vis.border_width(s.border_width); }
            if vm & S_BORDER_RADIUS != 0 { vis = vis.border_radius(s.border_radius); }
            if vm & S_OPACITY != 0 { vis = vis.opacity(s.opacity); }
            if vm & S_Z_INDEX != 0 { vis = vis.z_index(s.z_index); }
            let _ = vis;
        }
        if let Some(ref shadows) = self.box_shadows {
            tx.edit::<Visual>(node)
                .box_shadows((!shadows.is_empty()).then(|| BoxShadows(Arc::clone(shadows))));
        }
        if let Some((ox, oy)) = self.overflow {
            let axes = self.effective_overflow_axes();
            let mut vis = tx.edit::<Visual>(node);
            if axes & OV_X != 0 { vis = vis.overflow_x(ox); }
            if axes & OV_Y != 0 { vis = vis.overflow_y(oy); }
            let _ = vis;
        }
        if let Some(ref fill) = self.background_fill {
            tx.edit::<Visual>(node)
                .background_fill(Some(BackgroundFillRes(fill.clone())));
        }
    }

    fn apply_typography_layer(&self, tx: &mut Transaction<'_>, node: NodeId) {
        if !self.any_typography() {
            return;
        }
        let mut typo = tx.edit::<Typography>(node);
        let sm = self.style_mask.bits();
        if sm & S_TEXT_COLOR != 0 {
            typo = typo.color(Rgba(self.style.text_color.to_u32()));
        }
        if sm & S_FONT_SIZE != 0 {
            typo = typo.font_size(self.style.font_size);
        }
        if let Some(ref ff) = self.font_family {
            typo = typo.font_family(ff);
        }
        if let Some(lh) = self.line_height {
            typo = typo.line_height(lh);
        }
        if let Some(w) = self.font_weight {
            typo = typo.font_weight(w);
        }
        if let Some(fs) = self.font_style {
            typo = typo.font_style(fs);
        }
        if let Some(align) = self.text_align {
            typo = typo.text_align(align);
        }
        if let Some(ws) = self.white_space {
            typo = typo.white_space(ws);
        }
        if let Some(ow) = self.overflow_wrap {
            typo = typo.overflow_wrap(ow);
        }
        if let Some(td) = self.text_decoration {
            typo = typo.text_decoration(td);
        }
        let _ = typo;
    }

    /// Writes this [`TypedStyle`] directly into `node`'s `NodeStore` word tables with zero CSS
    /// string parsing.
    pub fn apply_to_tx(&self, tx: &mut Transaction<'_>, node: NodeId) {
        self.apply_style_layers_to_tx(tx, node, nve_tables::STYLE_LAYER_ALL);

        if let Some(ref ext) = self.layout_ext {
            if let Some(ref g) = ext.grid {
                if !g.template_columns.is_empty() || !g.template_rows.is_empty() {
                    let mut gl = tx.edit::<GridLayout>(node);
                    if !g.template_columns.is_empty() {
                        gl = gl.template_columns(Some(GridTracks(g.template_columns.clone().into())));
                    }
                    if !g.template_rows.is_empty() {
                        gl = gl.template_rows(Some(GridTracks(g.template_rows.clone().into())));
                    }
                    let _ = gl;
                }
            }
            if !ext.grid_item.column.is_auto() || !ext.grid_item.row.is_auto() {
                tx.edit::<GridItem>(node)
                    .column_start(ext.grid_item.column.start)
                    .column_end(ext.grid_item.column.end)
                    .row_start(ext.grid_item.row.start)
                    .row_end(ext.grid_item.row.end);
            }
        }

        if let Some(fit) = self.object_fit {
            tx.edit::<Image>(node).fit(fit);
        }

        if let Some(tf) = self.transform {
            tx.edit::<Transform>(node)
                .translate_x(tf.translate_x)
                .translate_y(tf.translate_y)
                .scale_x(tf.scale_x)
                .scale_y(tf.scale_y)
                .rotate_deg(tf.rotate_deg)
                .origin_x(tf.origin_x)
                .origin_y(tf.origin_y);
        }

        if self.cursor.is_some() || !self.state_rules.is_empty() {
            if let Some(c) = self.cursor {
                tx.edit::<Interaction>(node).cursor(c);
            }
            if !self.state_rules.is_empty() {
                if self.state_rules.iter().any(|r| r.patch.has_pending_resources()) {
                    tx.with_temp_resources(|scope| {
                        let rules: Arc<[StateStyleRule]> = self
                            .state_rules
                            .iter()
                            .map(|r| StateStyleRule::new(r.state, lower_pending_patch(&r.patch, scope)))
                            .collect();
                        scope
                            .tx()
                            .edit::<Interaction>(node)
                            .state_rules(Some(StateRules(rules)));
                    });
                } else {
                    let rules: Arc<[StateStyleRule]> = self
                        .state_rules
                        .iter()
                        .map(|r| StateStyleRule::new(r.state, r.patch.patch.clone()))
                        .collect();
                    tx.edit::<Interaction>(node)
                        .state_rules(Some(StateRules(rules)));
                }
            }
        }

        if self.show_on.is_some() || !self.responsive.is_empty() {
            if let Some(q) = self.show_on {
                let (w, env) = pack_media_query(q);
                tx.edit::<Responsive>(node)
                    .show_on_widths(w)
                    .show_on_env(env);
            }
            if !self.responsive.is_empty() {
                if self.responsive.iter().any(|r| r.patch.has_pending_resources()) {
                    tx.with_temp_resources(|scope| {
                        let rules: Arc<[ResponsiveRule]> = self
                            .responsive
                            .iter()
                            .map(|r| ResponsiveRule::new(r.query, lower_pending_patch(&r.patch, scope)))
                            .collect();
                        scope
                            .tx()
                            .edit::<Responsive>(node)
                            .rules(Some(ResponsiveRules(rules)));
                    });
                } else {
                    let rules: Arc<[ResponsiveRule]> = self
                        .responsive
                        .iter()
                        .map(|r| ResponsiveRule::new(r.query, r.patch.patch.clone()))
                        .collect();
                    tx.edit::<Responsive>(node)
                        .rules(Some(ResponsiveRules(rules)));
                }
            }
        }
    }

    /// Resets fields/tables that were written by `self` but are no longer set in `next` (or all
    /// of them when `next` is `None`).
    pub fn clear_diff_from(
        &self,
        next: Option<&TypedStyle>,
        tx: &mut Transaction<'_>,
        node: NodeId,
    ) {
        self.clear_layout_diff(next, tx, node);
        self.clear_visual_diff(next, tx, node);
        self.clear_typography_diff(next, tx, node);

        if self.object_fit.is_some()
            && next.is_none_or(|n| n.object_fit.is_none())
            && tx.store().has::<Image>(node)
        {
            tx.edit::<Image>(node).fit(ImageFit::Cover);
        }
        if self.transform.is_some() && next.is_none_or(|n| n.transform.is_none()) {
            tx.detach::<Transform>(node);
        }
        if self.cursor.is_some()
            && next.is_none_or(|n| n.cursor.is_none())
            && tx.store().has::<Interaction>(node)
        {
            tx.edit::<Interaction>(node).cursor(CursorKind::Auto);
        }
        if !self.state_rules.is_empty()
            && next.is_none_or(|n| n.state_rules.is_empty())
            && tx.store().has::<Interaction>(node)
        {
            tx.edit::<Interaction>(node).state_rules(None);
        }
        if self.show_on.is_some()
            && next.is_none_or(|n| n.show_on.is_none())
            && tx.store().has::<Responsive>(node)
        {
            tx.edit::<Responsive>(node)
                .show_on_widths(0)
                .show_on_env(0);
        }
        if !self.responsive.is_empty()
            && next.is_none_or(|n| n.responsive.is_empty())
            && tx.store().has::<Responsive>(node)
        {
            tx.edit::<Responsive>(node).rules(None);
        }
    }

    fn clear_layout_diff(&self, next: Option<&TypedStyle>, tx: &mut Transaction<'_>, node: NodeId) {
        if self.layout_ext.is_some() && next.is_none_or(|n| n.layout_ext.is_none()) {
            tx.detach::<GridLayout>(node);
            tx.detach::<GridItem>(node);
        }
        if self.any_layout() && next.is_none_or(|n| !n.any_layout()) {
            tx.detach::<Layout>(node);
            return;
        }
        let Some(next) = next else { return };
        let dropped = self.layout_mask.bits() & !next.layout_mask.bits();
        let dropped_wrap = self.flex_wrap.is_some() && next.flex_wrap.is_none();
        let dropped_ext = self.effective_ext_mask() & !next.effective_ext_mask();
        if dropped == 0 && !dropped_wrap && dropped_ext == 0 {
            return;
        }
        let mut row = tx.edit::<Layout>(node);
        if dropped & L_WIDTH != 0 { row = row.width(Length::AUTO); }
        if dropped & L_HEIGHT != 0 { row = row.height(Length::AUTO); }
        if dropped & L_MIN_WIDTH != 0 { row = row.min_width(Length::AUTO); }
        if dropped & L_MIN_HEIGHT != 0 { row = row.min_height(Length::AUTO); }
        if dropped & L_MAX_WIDTH != 0 { row = row.max_width(Length::AUTO); }
        if dropped & L_MAX_HEIGHT != 0 { row = row.max_height(Length::AUTO); }
        if dropped & L_FLEX_DIR != 0 { row = row.flex_direction(FlexDirection::Column); }
        if dropped & L_JUSTIFY != 0 { row = row.justify_content(JustifyContent::FlexStart); }
        if dropped & L_ALIGN_ITEMS != 0 { row = row.align_items(AlignItems::Stretch); }
        if dropped & L_ALIGN_SELF != 0 { row = row.align_self(AlignSelf::Auto); }
        if dropped & L_FLEX_GROW != 0 { row = row.flex_grow(0.0); }
        if dropped & L_FLEX_SHRINK != 0 { row = row.flex_shrink(1.0); }
        if dropped & L_GAP != 0 { row = row.row_gap(Length::ZERO).column_gap(Length::ZERO); }
        if dropped & L_PAD_TOP != 0 { row = row.padding_top(Length::ZERO); }
        if dropped & L_PAD_RIGHT != 0 { row = row.padding_right(Length::ZERO); }
        if dropped & L_PAD_BOTTOM != 0 { row = row.padding_bottom(Length::ZERO); }
        if dropped & L_PAD_LEFT != 0 { row = row.padding_left(Length::ZERO); }
        if dropped & L_MAR_TOP != 0 { row = row.margin_top(Length::ZERO); }
        if dropped & L_MAR_RIGHT != 0 { row = row.margin_right(Length::ZERO); }
        if dropped & L_MAR_BOTTOM != 0 { row = row.margin_bottom(Length::ZERO); }
        if dropped & L_MAR_LEFT != 0 { row = row.margin_left(Length::ZERO); }
        if dropped_wrap { row = row.flex_wrap(FlexWrap::NoWrap); }
        if dropped_ext & E_DISPLAY != 0 { row = row.display(Display::Flex); }
        if dropped_ext & E_POSITION != 0 { row = row.position(Position::Relative); }
        if dropped_ext & E_FLEX_BASIS != 0 { row = row.flex_basis(Length::AUTO); }
        if dropped_ext & E_INSET_TOP != 0 { row = row.inset_top(Length::AUTO); }
        if dropped_ext & E_INSET_RIGHT != 0 { row = row.inset_right(Length::AUTO); }
        if dropped_ext & E_INSET_BOTTOM != 0 { row = row.inset_bottom(Length::AUTO); }
        if dropped_ext & E_INSET_LEFT != 0 { row = row.inset_left(Length::AUTO); }
        if dropped_ext & E_ASPECT_RATIO != 0 { row = row.aspect_ratio(None); }
        if dropped_ext & E_DIRECTION != 0 { row = row.direction(Direction::Inherit); }
        if dropped_ext & E_PAD_INLINE != 0 { row = row.padding_inline_logical(false); }
        if dropped_ext & E_MAR_INLINE != 0 { row = row.margin_inline_logical(false); }
        let _ = row;
    }

    fn clear_visual_diff(&self, next: Option<&TypedStyle>, tx: &mut Transaction<'_>, node: NodeId) {
        if self.any_visual() && next.is_none_or(|n| !n.any_visual()) {
            tx.detach::<Visual>(node);
            return;
        }
        let Some(next) = next else { return };
        if !tx.store().has::<Visual>(node) {
            return;
        }
        let dropped =
            (self.style_mask.bits() & S_VISUAL_BITS) & !(next.style_mask.bits() & S_VISUAL_BITS);
        let dropped_ov = self.effective_overflow_axes() & !next.effective_overflow_axes();
        let drop_shadows = self.box_shadows.is_some() && next.box_shadows.is_none();
        let drop_fill = self.background_fill.is_some() && next.background_fill.is_none();
        if dropped == 0 && dropped_ov == 0 && !drop_shadows && !drop_fill {
            return;
        }
        let mut vis = tx.edit::<Visual>(node);
        if dropped & S_BG != 0 { vis = vis.background(Rgba::TRANSPARENT); }
        if dropped & S_BORDER_COLOR != 0 { vis = vis.border_color(Rgba::TRANSPARENT); }
        if dropped & S_BORDER_WIDTH != 0 { vis = vis.border_width(0.0); }
        if dropped & S_BORDER_RADIUS != 0 { vis = vis.border_radius(0.0); }
        if dropped & S_OPACITY != 0 { vis = vis.opacity(1.0); }
        if dropped & S_Z_INDEX != 0 { vis = vis.z_index(0); }
        if dropped_ov & OV_X != 0 { vis = vis.overflow_x(Overflow::Visible); }
        if dropped_ov & OV_Y != 0 { vis = vis.overflow_y(Overflow::Visible); }
        if drop_shadows { vis = vis.box_shadows(None); }
        if drop_fill { vis = vis.background_fill(None); }
        let _ = vis;
    }

    fn clear_typography_diff(
        &self,
        next: Option<&TypedStyle>,
        tx: &mut Transaction<'_>,
        node: NodeId,
    ) {
        if self.any_typography() && next.is_none_or(|n| !n.any_typography()) {
            tx.detach::<Typography>(node);
            return;
        }
        let Some(next) = next else { return };
        if !tx.store().has::<Typography>(node) {
            return;
        }
        let mut typo = tx.edit::<Typography>(node);
        if self.style_mask.bits() & S_TEXT_COLOR != 0 && next.style_mask.bits() & S_TEXT_COLOR == 0 {
            typo = typo.clear_color();
        }
        if self.style_mask.bits() & S_FONT_SIZE != 0 && next.style_mask.bits() & S_FONT_SIZE == 0 {
            typo = typo.clear_font_size();
        }
        if self.font_family.is_some() && next.font_family.is_none() {
            typo = typo.clear_font_family();
        }
        if self.line_height.is_some() && next.line_height.is_none() {
            typo = typo.clear_line_height();
        }
        if self.font_weight.is_some() && next.font_weight.is_none() {
            typo = typo.clear_font_weight();
        }
        if self.font_style.is_some() && next.font_style.is_none() {
            typo = typo.clear_font_style();
        }
        if self.text_align.is_some() && next.text_align.is_none() {
            typo = typo.clear_text_align();
        }
        if self.white_space.is_some() && next.white_space.is_none() {
            typo = typo.clear_white_space();
        }
        if self.overflow_wrap.is_some() && next.overflow_wrap.is_none() {
            typo = typo.clear_overflow_wrap();
        }
        if self.text_decoration.is_some() && next.text_decoration.is_none() {
            typo = typo.clear_text_decoration();
        }
        let _ = typo;
    }
}

fn lower_pending_patch(pending: &PendingStylePatch, scope: &mut TempResources<'_>) -> RowPatch {
    let mut patch = pending.patch.clone();
    if let Some(ref fill) = pending.background_fill {
        let id = scope.intern(&BackgroundFillRes(fill.clone()));
        patch.set_res_id(Visual::BACKGROUND_FILL, id);
    }
    if let Some(ref cols) = pending.grid_template_columns {
        let id = scope.intern(&GridTracks(cols.clone()));
        patch.set_res_id(GridLayout::TEMPLATE_COLUMNS, id);
    }
    if let Some(ref rows) = pending.grid_template_rows {
        let id = scope.intern(&GridTracks(rows.clone()));
        patch.set_res_id(GridLayout::TEMPLATE_ROWS, id);
    }
    patch
}

#[inline]
fn dim_to_length(dim: Dimension) -> Length {
    match dim {
        Dimension::Auto => Length::AUTO,
        Dimension::Points(v) => Length::points(v),
        Dimension::Percent(v) => Length::percent(v),
    }
}
