//! Diff-clearing of dropped [`TypedStyle`] properties and tables from `NodeStore`.

use nve_abi::FlexDirection;
use nve_store::{GridLine, Length, NodeId, Rgba, Transaction};
use nve_tables::interaction::CursorKind;
use nve_tables::tables::{
    AlignItems, AlignSelf, Direction, Display, FlexWrap, GridItem, GridLayout, Image, ImageFit,
    Interaction, JustifyContent, Layout, Overflow, Position, Responsive, Transform, Typography,
    Visual,
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
use super::TypedStyle;

impl TypedStyle {
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
        let self_em = self.layout_ext_mask.bits();
        let next_em = next.map_or(0, |n| n.layout_ext_mask.bits());
        let dropped_em = self_em & !next_em;

        if (self_em & E_GRID_LAYOUT_ANY) != 0 {
            if (next_em & E_GRID_LAYOUT_ANY) == 0 {
                tx.detach::<GridLayout>(node);
            } else if (dropped_em & E_GRID_LAYOUT_ANY) != 0 && tx.store().has::<GridLayout>(node) {
                let mut gl = tx.edit::<GridLayout>(node);
                if dropped_em & E_GRID_COLS != 0 {
                    gl = gl.template_columns(None);
                }
                if dropped_em & E_GRID_ROWS != 0 {
                    gl = gl.template_rows(None);
                }
                let _ = gl;
            }
        }
        if (self_em & E_GRID_ITEM_ANY) != 0 {
            if (next_em & E_GRID_ITEM_ANY) == 0 {
                tx.detach::<GridItem>(node);
            } else if (dropped_em & E_GRID_ITEM_ANY) != 0 && tx.store().has::<GridItem>(node) {
                let mut gi = tx.edit::<GridItem>(node);
                if dropped_em & E_GRID_COL_ITEM != 0 {
                    gi = gi.column_start(GridLine::Auto).column_end(GridLine::Auto);
                }
                if dropped_em & E_GRID_ROW_ITEM != 0 {
                    gi = gi.row_start(GridLine::Auto).row_end(GridLine::Auto);
                }
                let _ = gi;
            }
        }

        if self.any_layout() && next.is_none_or(|n| !n.any_layout()) {
            tx.detach::<Layout>(node);
            return;
        }
        let Some(next) = next else { return };
        let next_lm = next.layout_mask.bits();
        let dropped = self.layout_mask.bits() & !next_lm;
        let dropped_wrap = self.flex_wrap.is_some() && next.flex_wrap.is_none();
        let dropped_ext = dropped_em & E_LAYOUT_TABLE_BITS;
        let drop_pad_logical =
            (self_em & E_PAD_INLINE_ANY) != 0 && (next_em & E_PAD_INLINE_ANY) == 0;
        let drop_mar_logical =
            (self_em & E_MAR_INLINE_ANY) != 0 && (next_em & E_MAR_INLINE_ANY) == 0;
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
        if dropped & L_GAP != 0 {
            if next_em & E_ROW_GAP == 0 {
                row = row.row_gap(Length::ZERO);
            }
            if next_em & E_COL_GAP == 0 {
                row = row.column_gap(Length::ZERO);
            }
        }
        if dropped & L_PAD_TOP != 0 { row = row.padding_top(Length::ZERO); }
        if dropped & L_PAD_RIGHT != 0 && next_em & E_PAD_INLINE_END == 0 {
            row = row.padding_right(Length::ZERO);
        }
        if dropped & L_PAD_BOTTOM != 0 { row = row.padding_bottom(Length::ZERO); }
        if dropped & L_PAD_LEFT != 0 && next_em & E_PAD_INLINE_START == 0 {
            row = row.padding_left(Length::ZERO);
        }
        if dropped & L_MAR_TOP != 0 { row = row.margin_top(Length::ZERO); }
        if dropped & L_MAR_RIGHT != 0 && next_em & E_MAR_INLINE_END == 0 {
            row = row.margin_right(Length::ZERO);
        }
        if dropped & L_MAR_BOTTOM != 0 { row = row.margin_bottom(Length::ZERO); }
        if dropped & L_MAR_LEFT != 0 && next_em & E_MAR_INLINE_START == 0 {
            row = row.margin_left(Length::ZERO);
        }
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
        if dropped_ext & E_PAD_INLINE_START != 0 && next_lm & L_PAD_LEFT == 0 {
            row = row.padding_left(Length::ZERO);
        }
        if dropped_ext & E_PAD_INLINE_END != 0 && next_lm & L_PAD_RIGHT == 0 {
            row = row.padding_right(Length::ZERO);
        }
        if drop_pad_logical {
            row = row.padding_inline_logical(false);
        }
        if dropped_ext & E_MAR_INLINE_START != 0 && next_lm & L_MAR_LEFT == 0 {
            row = row.margin_left(Length::ZERO);
        }
        if dropped_ext & E_MAR_INLINE_END != 0 && next_lm & L_MAR_RIGHT == 0 {
            row = row.margin_right(Length::ZERO);
        }
        if drop_mar_logical {
            row = row.margin_inline_logical(false);
        }
        if dropped_ext & E_COL_GAP != 0 && next_lm & L_GAP == 0 {
            row = row.column_gap(Length::ZERO);
        }
        if dropped_ext & E_ROW_GAP != 0 && next_lm & L_GAP == 0 {
            row = row.row_gap(Length::ZERO);
        }
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
        let dropped_ov = self.overflow_axes & !next.overflow_axes;
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
        if self.style_mask.bits() & S_TEXT_COLOR != 0 && next.style_mask.bits() & S_TEXT_COLOR == 0
        {
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
