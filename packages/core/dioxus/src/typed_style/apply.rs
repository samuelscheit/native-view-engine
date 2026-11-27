//! Direct `TypedStyle` application and diff-clearing onto `NodeStore` word tables.

use std::sync::Arc;

use nve_abi::{Dimension, FlexAlign, FlexDirection, FlexJustify};
use nve_tables::interaction::{CursorKind, StateStyleRule};
use nve_tables::layout_ext::{LayoutDirection, PositionMode};
use nve_tables::media::ResponsiveRule;
use nve_tables::tables::{
    AlignItems, BackgroundFillRes, BoxShadows, Direction, Display,
    FlexDirection as TableFlexDir, GridItem, GridLayout, GridLine as TableGridLine, GridTracks,
    Image, ImageFit, Interaction, JustifyContent, Layout, Overflow, Position, Responsive,
    ResponsiveRules, StateRules, TextAlign, Transform, Typography, Visual,
};
use nve_store::{Length, NodeId, Rgba, RowPatch, TempResources, Transaction};

use super::{pack_media_query, PendingStylePatch, TypedStyle};

impl TypedStyle {
    /// Writes only the `{Layout, Visual, Typography}` tables selected by `layers` into `node`
    /// without touching unselected style tables or unrelated tables (`Transform`, `Motion`, etc.).
    pub fn apply_style_layers_to_tx(&self, tx: &mut Transaction<'_>, node: NodeId, layers: u8) {
        if layers & nve_tables::STYLE_LAYER_LAYOUT != 0 {
            if self.has_layout {
                let l = &self.layout;
                tx.edit::<Layout>(node)
                    .width(dim_to_length(l.width))
                    .height(dim_to_length(l.height))
                    .min_width(dim_to_length(l.min_width))
                    .min_height(dim_to_length(l.min_height))
                    .max_width(dim_to_length(l.max_width))
                    .max_height(dim_to_length(l.max_height))
                    .flex_direction(match l.flex_direction {
                        FlexDirection::Column => TableFlexDir::Column,
                        FlexDirection::Row => TableFlexDir::Row,
                        FlexDirection::ColumnReverse => TableFlexDir::ColumnReverse,
                        FlexDirection::RowReverse => TableFlexDir::RowReverse,
                    })
                    .justify_content(match l.justify_content {
                        FlexJustify::FlexStart => JustifyContent::FlexStart,
                        FlexJustify::Center => JustifyContent::Center,
                        FlexJustify::FlexEnd => JustifyContent::FlexEnd,
                        FlexJustify::SpaceBetween => JustifyContent::SpaceBetween,
                        FlexJustify::SpaceAround => JustifyContent::SpaceAround,
                        FlexJustify::SpaceEvenly => JustifyContent::SpaceEvenly,
                    })
                    .align_items(match l.align_items {
                        FlexAlign::Stretch => AlignItems::Stretch,
                        FlexAlign::FlexStart => AlignItems::FlexStart,
                        FlexAlign::Center => AlignItems::Center,
                        FlexAlign::FlexEnd => AlignItems::FlexEnd,
                        FlexAlign::Baseline => AlignItems::Baseline,
                    })
                    .flex_grow(l.flex_grow)
                    .flex_shrink(l.flex_shrink)
                    .row_gap(Length::points(l.gap))
                    .column_gap(Length::points(l.gap))
                    .padding_top(Length::points(l.padding.top))
                    .padding_right(Length::points(l.padding.right))
                    .padding_bottom(Length::points(l.padding.bottom))
                    .padding_left(Length::points(l.padding.left))
                    .margin_top(Length::points(l.margin.top))
                    .margin_right(Length::points(l.margin.right))
                    .margin_bottom(Length::points(l.margin.bottom))
                    .margin_left(Length::points(l.margin.left));
            }
            if let Some(ref ext) = self.layout_ext {
                let mut row = tx
                    .edit::<Layout>(node)
                    .display(ext.display)
                    .position(match ext.position {
                        PositionMode::Relative => Position::Relative,
                        PositionMode::Absolute => Position::Absolute,
                        PositionMode::Sticky => Position::Sticky,
                    })
                    .flex_basis(dim_to_length(ext.flex_basis))
                    .inset_top(dim_to_length(ext.insets.top))
                    .inset_right(dim_to_length(ext.insets.right))
                    .inset_bottom(dim_to_length(ext.insets.bottom))
                    .inset_left(dim_to_length(ext.insets.left));
                if let Some(ar) = ext.aspect_ratio {
                    row = row.aspect_ratio(Some(ar));
                }
                if let Some(dir) = ext.direction {
                    row = row.direction(match dir {
                        LayoutDirection::Ltr => Direction::Ltr,
                        LayoutDirection::Rtl => Direction::Rtl,
                    });
                }
                if ext.padding_inline.start.is_some() || ext.padding_inline.end.is_some() {
                    row = row.padding_inline_logical(true);
                    if let Some(v) = ext.padding_inline.start {
                        row = row.padding_left(Length::points(v));
                    }
                    if let Some(v) = ext.padding_inline.end {
                        row = row.padding_right(Length::points(v));
                    }
                }
                if ext.margin_inline.start.is_some() || ext.margin_inline.end.is_some() {
                    row = row.margin_inline_logical(true);
                    if let Some(v) = ext.margin_inline.start {
                        row = row.margin_left(Length::points(v));
                    }
                    if let Some(v) = ext.margin_inline.end {
                        row = row.margin_right(Length::points(v));
                    }
                }
                if let Some(ref g) = ext.grid {
                    if let Some(cg) = g.column_gap {
                        row = row.column_gap(Length::points(cg));
                    }
                    if let Some(rg) = g.row_gap {
                        row = row.row_gap(Length::points(rg));
                    }
                }
                let _ = row;
            }
        }

        if layers & nve_tables::STYLE_LAYER_VISUAL != 0 {
            if self.has_style {
                let s = &self.style;
                tx.edit::<Visual>(node)
                    .background(Rgba(s.background.to_u32()))
                    .border_color(Rgba(s.border_color.to_u32()))
                    .border_width(s.border_width)
                    .border_radius(s.border_radius)
                    .opacity(s.opacity)
                    .z_index(s.z_index);
            }
            if let Some(ref shadows) = self.box_shadows {
                tx.edit::<Visual>(node).box_shadows(
                    (!shadows.is_empty()).then(|| BoxShadows(shadows.as_slice().into())),
                );
            }
            if let Some(ref ov) = self.overflow {
                let mode = match ov.trim() {
                    "hidden" => Overflow::Hidden,
                    "scroll" => Overflow::Scroll,
                    "auto" => Overflow::Auto,
                    _ => Overflow::Visible,
                };
                tx.edit::<Visual>(node).overflow_x(mode).overflow_y(mode);
            }
            if let Some(ref fill) = self.background_fill {
                tx.edit::<Visual>(node)
                    .background_fill(Some(BackgroundFillRes(fill.clone())));
            }
        }

        if layers & nve_tables::STYLE_LAYER_TYPOGRAPHY != 0 {
            if self.has_style {
                let s = &self.style;
                if s.text_color.a > 0 || s.font_size > 0.0 {
                    let mut typo = tx.edit::<Typography>(node);
                    if s.text_color.a > 0 {
                        typo = typo.color(Rgba(s.text_color.to_u32()));
                    }
                    if s.font_size > 0.0 {
                        typo.font_size(s.font_size);
                    }
                }
            }
            if let Some(ref fw) = self.font_weight {
                let w = nve_tables::parse_font_weight(fw);
                tx.edit::<Typography>(node).font_weight(w);
            }
            if let Some(ref ta) = self.text_align {
                let align = match ta.trim() {
                    "center" => TextAlign::Center,
                    "right" | "end" => TextAlign::Right,
                    "justify" => TextAlign::Justify,
                    _ => TextAlign::Left,
                };
                tx.edit::<Typography>(node).text_align(align);
            }
        }
    }

    /// Writes this [`TypedStyle`] directly into `node`'s `NodeStore` word tables with zero CSS
    /// string parsing.
    pub fn apply_to_tx(&self, tx: &mut Transaction<'_>, node: NodeId) {
        self.apply_style_layers_to_tx(tx, node, nve_tables::STYLE_LAYER_ALL);

        if let Some(ref ext) = self.layout_ext {
            if let Some(ref g) = ext.grid {
                let mut gl = tx.edit::<GridLayout>(node);
                if !g.template_columns.is_empty() {
                    gl = gl.template_columns(Some(GridTracks(g.template_columns.clone().into())));
                }
                if !g.template_rows.is_empty() {
                    gl = gl.template_rows(Some(GridTracks(g.template_rows.clone().into())));
                }
                let _ = gl;
            }
            if !ext.grid_item.column.is_auto() || !ext.grid_item.row.is_auto() {
                tx.edit::<GridItem>(node)
                    .column_start(to_table_grid_line(ext.grid_item.column.start))
                    .column_end(to_table_grid_line(ext.grid_item.column.end))
                    .row_start(to_table_grid_line(ext.grid_item.row.start))
                    .row_end(to_table_grid_line(ext.grid_item.row.end));
            }
        }

        if let Some(ref fit) = self.object_fit {
            let mode = match fit.trim() {
                "contain" => ImageFit::Contain,
                "fill" | "stretch" => ImageFit::Fill,
                "none" | "center" => ImageFit::None,
                "scale-down" => ImageFit::ScaleDown,
                _ => ImageFit::Cover,
            };
            tx.edit::<Image>(node).fit(mode);
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
        if self.has_layout && next.is_none_or(|n| !n.has_layout) {
            tx.detach::<Layout>(node);
        }
        if self.layout_ext.is_some() && next.is_none_or(|n| n.layout_ext.is_none()) {
            tx.detach::<GridLayout>(node);
            tx.detach::<GridItem>(node);
            if next.is_some_and(|n| n.has_layout) {
                tx.edit::<Layout>(node)
                    .display(Display::Flex)
                    .position(Position::Relative)
                    .flex_basis(Length::AUTO)
                    .inset_top(Length::AUTO)
                    .inset_right(Length::AUTO)
                    .inset_bottom(Length::AUTO)
                    .inset_left(Length::AUTO)
                    .aspect_ratio(None)
                    .direction(Direction::Inherit)
                    .padding_inline_logical(false)
                    .margin_inline_logical(false);
            } else if !self.has_layout {
                tx.detach::<Layout>(node);
            }
        }
        if self.has_style && next.is_none_or(|n| !n.has_style) {
            tx.detach::<Visual>(node);
            tx.detach::<Typography>(node);
        } else if self.has_style {
            let clear_color =
                self.style.text_color.a > 0 && next.is_none_or(|n| n.style.text_color.a == 0);
            let clear_size =
                self.style.font_size > 0.0 && next.is_none_or(|n| n.style.font_size <= 0.0);
            if (clear_color || clear_size) && tx.store().has::<Typography>(node) {
                let mut typo = tx.edit::<Typography>(node);
                if clear_color {
                    typo = typo.clear_color();
                }
                if clear_size {
                    typo.clear_font_size();
                }
            }
        }
        if self.font_weight.is_some()
            && next.is_none_or(|n| n.font_weight.is_none())
            && tx.store().has::<Typography>(node)
        {
            tx.edit::<Typography>(node).clear_font_weight();
        }
        if self.text_align.is_some()
            && next.is_none_or(|n| n.text_align.is_none())
            && tx.store().has::<Typography>(node)
        {
            tx.edit::<Typography>(node).clear_text_align();
        }
        if self.box_shadows.is_some()
            && next.is_none_or(|n| n.box_shadows.is_none())
            && tx.store().has::<Visual>(node)
        {
            tx.edit::<Visual>(node).box_shadows(None);
        }
        if self.overflow.is_some()
            && next.is_none_or(|n| n.overflow.is_none())
            && tx.store().has::<Visual>(node)
        {
            tx.edit::<Visual>(node)
                .overflow_x(Overflow::Visible)
                .overflow_y(Overflow::Visible);
        }
        if self.background_fill.is_some()
            && next.is_none_or(|n| n.background_fill.is_none())
            && tx.store().has::<Visual>(node)
        {
            tx.edit::<Visual>(node).background_fill(None);
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

#[inline]
fn to_table_grid_line(line: nve_tables::layout_ext::GridLine) -> TableGridLine {
    match line {
        nve_tables::layout_ext::GridLine::Auto => TableGridLine::Auto,
        nve_tables::layout_ext::GridLine::Line(n) => TableGridLine::Line(n),
        nve_tables::layout_ext::GridLine::Span(s) => TableGridLine::Span(s),
    }
}
