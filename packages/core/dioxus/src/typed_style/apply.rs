//! Direct `TypedStyle` application and diff-clearing onto `NodeStore` word tables.

use std::sync::Arc;

use nve_abi::Dimension;
use nve_store::{Length, NodeId, Rgba, RowPatch, TempResources, Transaction};
use nve_tables::interaction::StateStyleRule;
use nve_tables::layout_ext::{LayoutDirection, PositionMode};
use nve_tables::media::ResponsiveRule;
use nve_tables::tables::{
    BackgroundFillRes, BoxShadows, Direction, GridItem, GridLayout, GridTracks, Image, Interaction,
    Layout, Position, Responsive, ResponsiveRules, StateRules, Transform, Typography, Visual,
};

use super::merge::{
    E_ASPECT_RATIO, E_COL_GAP, E_DIRECTION, E_DISPLAY, E_FLEX_BASIS, E_GRID_COLS, E_GRID_COL_ITEM,
    E_GRID_ITEM_ANY, E_GRID_LAYOUT_ANY, E_GRID_ROWS, E_GRID_ROW_ITEM, E_INSET_BOTTOM, E_INSET_LEFT,
    E_INSET_RIGHT, E_INSET_TOP, E_LAYOUT_TABLE_BITS, E_MAR_INLINE_ANY, E_MAR_INLINE_END,
    E_MAR_INLINE_START, E_PAD_INLINE_ANY, E_PAD_INLINE_END, E_PAD_INLINE_START, E_POSITION,
    E_ROW_GAP, L_ALIGN_ITEMS, L_ALIGN_SELF, L_FLEX_DIR, L_FLEX_GROW, L_FLEX_SHRINK, L_GAP,
    L_HEIGHT, L_JUSTIFY, L_MAR_BOTTOM, L_MAR_LEFT, L_MAR_RIGHT, L_MAR_TOP, L_MAX_HEIGHT,
    L_MAX_WIDTH, L_MIN_HEIGHT, L_MIN_WIDTH, L_PAD_BOTTOM, L_PAD_LEFT, L_PAD_RIGHT, L_PAD_TOP,
    L_WIDTH, OV_X, OV_Y, S_BG, S_BORDER_COLOR, S_BORDER_RADIUS, S_BORDER_WIDTH, S_FONT_SIZE,
    S_OPACITY, S_TEXT_COLOR, S_VISUAL_BITS, S_Z_INDEX,
};
use super::{pack_media_query, PendingStylePatch, TypedStyle};

impl TypedStyle {
    /// Writes only the `{Layout, Visual, Typography}` tables selected by `layers` into `node`
    /// without touching unselected style tables or unrelated tables (`Transform`, `Motion`, etc.).
    pub fn apply_style_layers_to_tx(&self, tx: &mut Transaction<'_>, node: NodeId, layers: u8) {
        self.apply_style_layers_with_prev(None, tx, node, layers);
    }

    fn apply_style_layers_with_prev(
        &self,
        prev: Option<&TypedStyle>,
        tx: &mut Transaction<'_>,
        node: NodeId,
        layers: u8,
    ) {
        if layers & nve_tables::STYLE_LAYER_LAYOUT != 0 {
            self.apply_layout_layer(tx, node);
        }
        if layers & nve_tables::STYLE_LAYER_VISUAL != 0 {
            self.apply_visual_layer(prev, tx, node);
        }
        if layers & nve_tables::STYLE_LAYER_TYPOGRAPHY != 0 {
            self.apply_typography_layer(prev, tx, node);
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
        let em = self.layout_ext_mask.bits();
        if (em & E_LAYOUT_TABLE_BITS) != 0
            && let Some(ref ext) = self.layout_ext
        {
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
            if em & E_PAD_INLINE_ANY != 0 {
                row = row.padding_inline_logical(true);
                if em & E_PAD_INLINE_START != 0
                    && let Some(v) = ext.padding_inline.start
                {
                    row = row.padding_left(Length::points(v));
                }
                if em & E_PAD_INLINE_END != 0
                    && let Some(v) = ext.padding_inline.end
                {
                    row = row.padding_right(Length::points(v));
                }
            }
            if em & E_MAR_INLINE_ANY != 0 {
                row = row.margin_inline_logical(true);
                if em & E_MAR_INLINE_START != 0
                    && let Some(v) = ext.margin_inline.start
                {
                    row = row.margin_left(Length::points(v));
                }
                if em & E_MAR_INLINE_END != 0
                    && let Some(v) = ext.margin_inline.end
                {
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

    fn apply_visual_layer(
        &self,
        prev: Option<&TypedStyle>,
        tx: &mut Transaction<'_>,
        node: NodeId,
    ) {
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
        if let Some(ref shadows) = self.box_shadows
            && !prev.is_some_and(|p| p.box_shadows.as_ref() == Some(shadows))
        {
            tx.edit::<Visual>(node)
                .box_shadows((!shadows.is_empty()).then(|| BoxShadows(Arc::clone(shadows))));
        }
        let axes = self.overflow_axes;
        if axes != 0
            && let Some((ox, oy)) = self.overflow
        {
            let mut vis = tx.edit::<Visual>(node);
            if axes & OV_X != 0 { vis = vis.overflow_x(ox); }
            if axes & OV_Y != 0 { vis = vis.overflow_y(oy); }
            let _ = vis;
        }
        if let Some(ref fill) = self.background_fill
            && !prev.is_some_and(|p| p.background_fill.as_ref() == Some(fill))
        {
            tx.edit::<Visual>(node)
                .background_fill(Some(BackgroundFillRes(fill.clone())));
        }
    }

    fn apply_typography_layer(
        &self,
        prev: Option<&TypedStyle>,
        tx: &mut Transaction<'_>,
        node: NodeId,
    ) {
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
        if let Some(ref ff) = self.font_family
            && !prev.is_some_and(|p| p.font_family.as_ref() == Some(ff))
        {
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
        self.apply_to_tx_with_prev(None, tx, node);
    }

    /// Writes this [`TypedStyle`] into `node`, skipping heap-backed resource reallocations
    /// (`GridTracks`, `BackgroundFill`, `StateRules`, `ResponsiveRules`) that are unchanged from
    /// `prev`.
    pub(crate) fn apply_to_tx_with_prev(
        &self,
        prev: Option<&TypedStyle>,
        tx: &mut Transaction<'_>,
        node: NodeId,
    ) {
        self.apply_style_layers_with_prev(prev, tx, node, nve_tables::STYLE_LAYER_ALL);

        let em = self.layout_ext_mask.bits();
        if let Some(ref ext) = self.layout_ext {
            if (em & E_GRID_LAYOUT_ANY) != 0
                && let Some(ref g) = ext.grid
            {
                let prev_grid = prev
                    .and_then(|p| p.layout_ext.as_deref())
                    .and_then(|e| e.grid.as_deref());
                let prev_em = prev.map_or(0, |p| p.layout_ext_mask.bits());
                let write_cols = (em & E_GRID_COLS) != 0
                    && !((prev_em & E_GRID_COLS) != 0
                        && prev_grid.is_some_and(|pg| pg.template_columns == g.template_columns));
                let write_rows = (em & E_GRID_ROWS) != 0
                    && !((prev_em & E_GRID_ROWS) != 0
                        && prev_grid.is_some_and(|pg| pg.template_rows == g.template_rows));
                if write_cols || write_rows {
                    let mut gl = tx.edit::<GridLayout>(node);
                    if write_cols {
                        gl = gl.template_columns(
                            (!g.template_columns.is_empty())
                                .then(|| GridTracks(g.template_columns.clone().into())),
                        );
                    }
                    if write_rows {
                        gl = gl.template_rows(
                            (!g.template_rows.is_empty())
                                .then(|| GridTracks(g.template_rows.clone().into())),
                        );
                    }
                    let _ = gl;
                }
            }
            if (em & E_GRID_ITEM_ANY) != 0 {
                let mut gi = tx.edit::<GridItem>(node);
                if em & E_GRID_COL_ITEM != 0 {
                    gi = gi
                        .column_start(ext.grid_item.column.start)
                        .column_end(ext.grid_item.column.end);
                }
                if em & E_GRID_ROW_ITEM != 0 {
                    gi = gi
                        .row_start(ext.grid_item.row.start)
                        .row_end(ext.grid_item.row.end);
                }
                let _ = gi;
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
            if !self.state_rules.is_empty()
                && !prev.is_some_and(|p| p.state_rules == self.state_rules)
            {
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
            if !self.responsive.is_empty()
                && !prev.is_some_and(|p| p.responsive == self.responsive)
            {
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
