//! Pure conversion helpers mapping NVE [`LayoutProps`] and [`LayoutExt`] onto `taffy::Style`.

use nve_abi::{Dimension, FlexAlign, FlexDirection, FlexJustify, LayoutProps};
use nve_components::layout_ext::{
    GridLine, GridSingleTrack, GridTrack, GridTrackPlacement, GridTrackUnit, LayoutDirection,
    LayoutExt, PositionMode,
};
use nve_components::style_patch::Display;
use taffy::geometry::MinMax;
use taffy::prelude::{
    self as tf, FromFr, FromLength, FromPercent, TaffyAuto, TaffyGridLine, TaffyGridSpan,
};
use taffy::style::{Direction as TaffyDirection, GridTemplateRepetition};

pub fn to_taffy_dim(d: Dimension) -> tf::Dimension {
    match d {
        Dimension::Auto => tf::Dimension::auto(),
        Dimension::Points(v) => tf::Dimension::length(v),
        Dimension::Percent(v) => tf::Dimension::percent(v / 100.0),
    }
}

pub fn to_taffy_lpa(d: Dimension) -> tf::LengthPercentageAuto {
    match d {
        Dimension::Auto => tf::LengthPercentageAuto::auto(),
        Dimension::Points(v) => tf::LengthPercentageAuto::length(v),
        Dimension::Percent(v) => tf::LengthPercentageAuto::percent(v / 100.0),
    }
}

#[inline]
fn to_taffy_min_track(u: GridTrackUnit) -> tf::MinTrackSizingFunction {
    match u {
        GridTrackUnit::Auto | GridTrackUnit::Fr(_) => tf::MinTrackSizingFunction::auto(),
        GridTrackUnit::Px(v) => tf::MinTrackSizingFunction::length(v.max(0.0)),
        GridTrackUnit::Percent(v) => tf::MinTrackSizingFunction::percent(v.max(0.0) / 100.0),
        GridTrackUnit::MinContent => tf::MinTrackSizingFunction::min_content(),
        GridTrackUnit::MaxContent => tf::MinTrackSizingFunction::max_content(),
    }
}

#[inline]
fn to_taffy_max_track(u: GridTrackUnit) -> tf::MaxTrackSizingFunction {
    match u {
        GridTrackUnit::Auto => tf::MaxTrackSizingFunction::auto(),
        GridTrackUnit::Px(v) => tf::MaxTrackSizingFunction::length(v.max(0.0)),
        GridTrackUnit::Percent(v) => tf::MaxTrackSizingFunction::percent(v.max(0.0) / 100.0),
        GridTrackUnit::Fr(v) => tf::MaxTrackSizingFunction::fr(v.max(0.0)),
        GridTrackUnit::MinContent => tf::MaxTrackSizingFunction::min_content(),
        GridTrackUnit::MaxContent => tf::MaxTrackSizingFunction::max_content(),
    }
}

#[inline]
fn to_taffy_single_track(t: GridSingleTrack) -> tf::TrackSizingFunction {
    match t {
        GridSingleTrack::Auto => tf::TrackSizingFunction::AUTO,
        GridSingleTrack::Px(v) => tf::TrackSizingFunction::from_length(v.max(0.0)),
        GridSingleTrack::Percent(v) => tf::TrackSizingFunction::from_percent(v.max(0.0) / 100.0),
        GridSingleTrack::Fr(v) => tf::TrackSizingFunction::from_fr(v.max(0.0)),
        GridSingleTrack::MinMax(min, max) => MinMax {
            min: to_taffy_min_track(min),
            max: to_taffy_max_track(max),
        },
    }
}

fn to_taffy_grid_tracks(tracks: &[GridTrack]) -> Vec<tf::GridTemplateComponent<String>> {
    tracks
        .iter()
        .map(|&track| match track {
            GridTrack::Auto => tf::GridTemplateComponent::Single(tf::TrackSizingFunction::AUTO),
            GridTrack::Px(v) => {
                tf::GridTemplateComponent::Single(tf::TrackSizingFunction::from_length(v.max(0.0)))
            }
            GridTrack::Percent(v) => tf::GridTemplateComponent::Single(
                tf::TrackSizingFunction::from_percent(v.max(0.0) / 100.0),
            ),
            GridTrack::Fr(v) => {
                tf::GridTemplateComponent::Single(tf::TrackSizingFunction::from_fr(v.max(0.0)))
            }
            GridTrack::MinMax(min, max) => tf::GridTemplateComponent::Single(MinMax {
                min: to_taffy_min_track(min),
                max: to_taffy_max_track(max),
            }),
            GridTrack::Repeat(count, single) => {
                tf::GridTemplateComponent::Repeat(GridTemplateRepetition {
                    count: tf::RepetitionCount::Count(count.max(1)),
                    tracks: vec![to_taffy_single_track(single)],
                    line_names: Vec::new(),
                })
            }
        })
        .collect()
}

#[inline]
fn to_taffy_grid_line(line: GridLine) -> tf::GridPlacement<String> {
    match line {
        GridLine::Auto => tf::GridPlacement::Auto,
        GridLine::Line(n) => tf::GridPlacement::from_line_index(n),
        GridLine::Span(n) => tf::GridPlacement::from_span(n.max(1)),
    }
}

#[inline]
fn to_taffy_grid_placement(p: GridTrackPlacement) -> tf::Line<tf::GridPlacement<String>> {
    tf::Line {
        start: to_taffy_grid_line(p.start),
        end: to_taffy_grid_line(p.end),
    }
}

/// Converts base [`LayoutProps`] to a default LTR Flexbox `taffy::Style`.
pub fn to_taffy_style(l: &LayoutProps) -> tf::Style {
    to_taffy_style_with_ext(l, None, LayoutDirection::Ltr)
}

/// Converts [`LayoutProps`] plus optional [`LayoutExt`] and resolved [`LayoutDirection`] into
/// a complete `taffy::Style` supporting Flexbox, CSS Grid, `flex-basis`, `aspect-ratio`,
/// `position` (`relative`/`absolute`/`sticky`) + insets, and RTL logical edges.
pub fn to_taffy_style_with_ext(
    l: &LayoutProps,
    ext: Option<&LayoutExt>,
    effective_dir: LayoutDirection,
) -> tf::Style {
    use taffy::style::{
        AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignmentSafety,
    };
    let justify_kw = match l.justify_content {
        FlexJustify::FlexStart => AlignContentKeyword::FlexStart,
        FlexJustify::Center => AlignContentKeyword::Center,
        FlexJustify::FlexEnd => AlignContentKeyword::FlexEnd,
        FlexJustify::SpaceBetween => AlignContentKeyword::SpaceBetween,
        FlexJustify::SpaceAround => AlignContentKeyword::SpaceAround,
        FlexJustify::SpaceEvenly => AlignContentKeyword::SpaceEvenly,
    };
    let align_kw = match l.align_items {
        FlexAlign::Stretch => AlignItemsKeyword::Stretch,
        FlexAlign::FlexStart => AlignItemsKeyword::FlexStart,
        FlexAlign::Center => AlignItemsKeyword::Center,
        FlexAlign::FlexEnd => AlignItemsKeyword::FlexEnd,
        FlexAlign::Baseline => AlignItemsKeyword::Baseline,
    };

    let (padding, margin) = if let Some(ext) = ext {
        (
            ext.padding_inline.resolve_into(l.padding, effective_dir),
            ext.margin_inline.resolve_into(l.margin, effective_dir),
        )
    } else {
        (l.padding, l.margin)
    };

    let mut style = tf::Style {
        display: match ext.map_or(Display::Flex, |e| e.display) {
            Display::Flex => tf::Display::Flex,
            Display::Grid => tf::Display::Grid,
            Display::None => tf::Display::None,
        },
        position: match ext.map_or(PositionMode::Relative, |e| e.position) {
            PositionMode::Relative => tf::Position::Relative,
            PositionMode::Absolute => tf::Position::Absolute,
            PositionMode::Sticky => tf::Position::Sticky,
        },
        direction: match effective_dir {
            LayoutDirection::Ltr => TaffyDirection::Ltr,
            LayoutDirection::Rtl => TaffyDirection::Rtl,
        },
        flex_direction: match l.flex_direction {
            FlexDirection::Column => tf::FlexDirection::Column,
            FlexDirection::Row => tf::FlexDirection::Row,
            FlexDirection::ColumnReverse => tf::FlexDirection::ColumnReverse,
            FlexDirection::RowReverse => tf::FlexDirection::RowReverse,
        },
        justify_content: AlignContent {
            keyword: justify_kw,
            safety: AlignmentSafety::Default,
        },
        align_items: AlignItems {
            keyword: align_kw,
            safety: AlignmentSafety::Default,
        },
        flex_grow: l.flex_grow,
        flex_shrink: l.flex_shrink,
        gap: tf::Size {
            width: tf::LengthPercentage::length(l.gap),
            height: tf::LengthPercentage::length(l.gap),
        },
        size: tf::Size {
            width: to_taffy_dim(l.width),
            height: to_taffy_dim(l.height),
        },
        min_size: tf::Size {
            width: to_taffy_lpa(l.min_width),
            height: to_taffy_lpa(l.min_height),
        },
        max_size: tf::Size {
            width: to_taffy_lpa(l.max_width),
            height: to_taffy_lpa(l.max_height),
        },
        padding: tf::Rect {
            top: tf::LengthPercentage::length(padding.top),
            right: tf::LengthPercentage::length(padding.right),
            bottom: tf::LengthPercentage::length(padding.bottom),
            left: tf::LengthPercentage::length(padding.left),
        },
        margin: tf::Rect {
            top: tf::LengthPercentageAuto::length(margin.top),
            right: tf::LengthPercentageAuto::length(margin.right),
            bottom: tf::LengthPercentageAuto::length(margin.bottom),
            left: tf::LengthPercentageAuto::length(margin.left),
        },
        ..Default::default()
    };

    if let Some(ext) = ext {
        if ext.flex_basis != Dimension::Auto {
            style.flex_basis = to_taffy_dim(ext.flex_basis);
        }
        if let Some(ratio) = ext.aspect_ratio
            && ratio.is_finite()
            && ratio > 0.0
        {
            style.aspect_ratio = Some(ratio);
        }
        if ext.position != PositionMode::Sticky && !ext.insets.is_auto() {
            style.inset = tf::Rect {
                top: to_taffy_lpa(ext.insets.top),
                right: to_taffy_lpa(ext.insets.right),
                bottom: to_taffy_lpa(ext.insets.bottom),
                left: to_taffy_lpa(ext.insets.left),
            };
        }
        if let Some(ref grid) = ext.grid {
            if !grid.template_columns.is_empty() {
                style.grid_template_columns = to_taffy_grid_tracks(&grid.template_columns);
            }
            if !grid.template_rows.is_empty() {
                style.grid_template_rows = to_taffy_grid_tracks(&grid.template_rows);
            }
            if let Some(cg) = grid.column_gap {
                style.gap.width = tf::LengthPercentage::length(cg.max(0.0));
            }
            if let Some(rg) = grid.row_gap {
                style.gap.height = tf::LengthPercentage::length(rg.max(0.0));
            }
        }
        if !ext.grid_item.column.is_auto() {
            style.grid_column = to_taffy_grid_placement(ext.grid_item.column);
        }
        if !ext.grid_item.row.is_auto() {
            style.grid_row = to_taffy_grid_placement(ext.grid_item.row);
        }
    }

    style
}
