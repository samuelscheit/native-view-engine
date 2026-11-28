//! Zero-copy Taffy `StyleView` implementing `CoreStyle`, Flexbox, Grid, and OOF style traits
//! directly over `NodeStore` word columns.

use nve_tables::layout_ext::{GridSingleTrack, GridTrack, GridTrackUnit};
use nve_tables::tables::{
    AlignContent as NveAlignContent, AlignItems as NveAlignItems, AlignSelf as NveAlignSelf,
    BoxSizing as NveBoxSizing, Display as NveDisplay, FlexDirection as NveFlexDirection,
    FlexWrap as NveFlexWrap, GridItem, GridLayout, GridLine as NveGridLine, GridTracks,
    GridTracksIter, JustifyContent as NveJustifyContent, Layout as NveLayout,
    Overflow as NveOverflow, Position as NvePosition, Visual,
};
use nve_store::{Length, NodeId, NodeStore};

use taffy::geometry::{Line, MinMax, Point, Rect, Size};
use taffy::style::{
    AlignContent, AlignItems, AlignSelf, BoxGenerationMode, BoxSizing, CoreStyle, Dimension,
    Direction, FlexDirection, FlexWrap, FlexboxContainerStyle, FlexboxItemStyle,
    GenericGridTemplateComponent, GenericRepetition, GridContainerStyle, GridItemStyle,
    GridPlacement, GridTemplateArea, LengthPercentage, LengthPercentageAuto,
    MaxTrackSizingFunction, MinTrackSizingFunction, OofItemStyle, Overflow, Position,
    RepetitionCount, TrackSizingFunction,
};

#[derive(Clone, Copy)]
pub(super) struct StyleView<'a> {
    pub(super) store: &'a NodeStore,
    pub(super) id: NodeId,
    pub(super) is_root: bool,
    pub(super) root_fills_viewport: bool,
    pub(super) viewport: (f32, f32),
    pub(super) rtl: bool,
}

impl<'a> StyleView<'a> {
    #[inline(always)]
    fn layout(&self) -> nve_tables::tables::LayoutRef<'a> {
        self.store.row::<NveLayout>(self.id)
    }

    #[inline(always)]
    fn visual(&self) -> nve_tables::tables::VisualRef<'a> {
        self.store.row::<Visual>(self.id)
    }

    #[inline(always)]
    fn dim(l: Length) -> Dimension {
        if l.is_points() {
            Dimension::length(l.value())
        } else if l.is_percent() {
            Dimension::percent(l.value() / 100.0)
        } else {
            Dimension::auto()
        }
    }

    #[inline(always)]
    fn lpa(l: Length) -> LengthPercentageAuto {
        if l.is_points() {
            LengthPercentageAuto::length(l.value())
        } else if l.is_percent() {
            LengthPercentageAuto::percent(l.value() / 100.0)
        } else {
            LengthPercentageAuto::auto()
        }
    }

    #[inline(always)]
    fn lp(l: Length) -> LengthPercentage {
        if l.is_percent() {
            LengthPercentage::percent(l.value() / 100.0)
        } else if l.is_points() {
            LengthPercentage::length(l.value())
        } else {
            LengthPercentage::length(0.0)
        }
    }

    #[inline(always)]
    fn placement(start: NveGridLine, end: NveGridLine) -> Line<GridPlacement<String>> {
        fn map_line(g: NveGridLine) -> GridPlacement<String> {
            match g {
                NveGridLine::Auto => GridPlacement::Auto,
                NveGridLine::Line(n) => GridPlacement::Line(n.into()),
                NveGridLine::Span(n) => GridPlacement::Span(n),
            }
        }
        Line {
            start: map_line(start),
            end: map_line(end),
        }
    }
}

#[allow(clippy::ptr_arg)]
fn line_names(v: &Vec<String>) -> core::slice::Iter<'_, String> {
    v.iter()
}
type NoNames<'a> = core::iter::Map<
    core::slice::Iter<'a, Vec<String>>,
    fn(&Vec<String>) -> core::slice::Iter<'_, String>,
>;
const NO_NAMES: &[Vec<String>] = &[];

#[inline]
fn map_min_unit(u: GridTrackUnit) -> MinTrackSizingFunction {
    match u {
        GridTrackUnit::Auto | GridTrackUnit::Fr(_) => MinTrackSizingFunction::auto(),
        GridTrackUnit::Px(v) => MinTrackSizingFunction::length(v),
        GridTrackUnit::Percent(v) => MinTrackSizingFunction::percent(v / 100.0),
        GridTrackUnit::MinContent => MinTrackSizingFunction::min_content(),
        GridTrackUnit::MaxContent => MinTrackSizingFunction::max_content(),
    }
}

#[inline]
fn map_max_unit(u: GridTrackUnit) -> MaxTrackSizingFunction {
    match u {
        GridTrackUnit::Auto => MaxTrackSizingFunction::auto(),
        GridTrackUnit::Px(v) => MaxTrackSizingFunction::length(v),
        GridTrackUnit::Percent(v) => MaxTrackSizingFunction::percent(v / 100.0),
        GridTrackUnit::Fr(v) => MaxTrackSizingFunction::fr(v),
        GridTrackUnit::MinContent => MaxTrackSizingFunction::min_content(),
        GridTrackUnit::MaxContent => MaxTrackSizingFunction::max_content(),
    }
}

#[inline]
fn map_single_track(t: GridSingleTrack) -> TrackSizingFunction {
    match t {
        GridSingleTrack::Auto => MinMax {
            min: MinTrackSizingFunction::auto(),
            max: MaxTrackSizingFunction::auto(),
        },
        GridSingleTrack::Px(v) => MinMax {
            min: MinTrackSizingFunction::length(v),
            max: MaxTrackSizingFunction::length(v),
        },
        GridSingleTrack::Percent(v) => MinMax {
            min: MinTrackSizingFunction::percent(v / 100.0),
            max: MaxTrackSizingFunction::percent(v / 100.0),
        },
        GridSingleTrack::Fr(v) => MinMax {
            min: MinTrackSizingFunction::auto(),
            max: MaxTrackSizingFunction::fr(v),
        },
        GridSingleTrack::MinMax(min, max) => MinMax {
            min: map_min_unit(min),
            max: map_max_unit(max),
        },
    }
}

#[derive(Clone, Copy)]
pub(super) struct WordRepetition {
    count: u16,
    track: TrackSizingFunction,
}

impl GenericRepetition for WordRepetition {
    type CustomIdent = String;
    type RepetitionTrackList<'a> = core::iter::Once<TrackSizingFunction>;
    type TemplateLineNames<'a> = NoNames<'a>;

    fn count(&self) -> RepetitionCount {
        RepetitionCount::Count(self.count)
    }

    fn tracks(&self) -> Self::RepetitionTrackList<'_> {
        core::iter::once(self.track)
    }

    fn lines_names(&self) -> Self::TemplateLineNames<'_> {
        NO_NAMES
            .iter()
            .map(line_names as fn(&Vec<String>) -> core::slice::Iter<'_, String>)
    }
}

#[derive(Clone)]
pub(super) struct GridTrackIter<'a>(GridTracksIter<'a>);

impl Iterator for GridTrackIter<'_> {
    type Item = GenericGridTemplateComponent<String, WordRepetition>;

    fn next(&mut self) -> Option<Self::Item> {
        let t = self.0.next()?;
        Some(match t {
            GridTrack::Auto => GenericGridTemplateComponent::Single(MinMax {
                min: MinTrackSizingFunction::auto(),
                max: MaxTrackSizingFunction::auto(),
            }),
            GridTrack::Px(v) => GenericGridTemplateComponent::Single(MinMax {
                min: MinTrackSizingFunction::length(v),
                max: MaxTrackSizingFunction::length(v),
            }),
            GridTrack::Percent(v) => GenericGridTemplateComponent::Single(MinMax {
                min: MinTrackSizingFunction::percent(v / 100.0),
                max: MaxTrackSizingFunction::percent(v / 100.0),
            }),
            GridTrack::Fr(v) => GenericGridTemplateComponent::Single(MinMax {
                min: MinTrackSizingFunction::auto(),
                max: MaxTrackSizingFunction::fr(v),
            }),
            GridTrack::MinMax(min, max) => GenericGridTemplateComponent::Single(MinMax {
                min: map_min_unit(min),
                max: map_max_unit(max),
            }),
            GridTrack::Repeat(count, single) => {
                GenericGridTemplateComponent::Repeat(WordRepetition {
                    count: count.max(1),
                    track: map_single_track(single),
                })
            }
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for GridTrackIter<'_> {}

impl CoreStyle for StyleView<'_> {
    type CustomIdent = String;

    #[inline]
    fn box_generation_mode(&self) -> BoxGenerationMode {
        if self.layout().display() == NveDisplay::None
            || nve_tables::tables::is_hidden_text_input(self.store, self.id)
        {
            BoxGenerationMode::None
        } else {
            BoxGenerationMode::Normal
        }
    }

    #[inline]
    fn box_sizing(&self) -> BoxSizing {
        match self.layout().box_sizing() {
            NveBoxSizing::BorderBox => BoxSizing::BorderBox,
            NveBoxSizing::ContentBox => BoxSizing::ContentBox,
        }
    }

    #[inline]
    fn direction(&self) -> Direction {
        if self.rtl {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }

    #[inline]
    fn overflow(&self) -> Point<Overflow> {
        let v = self.visual();
        let map = |o: NveOverflow| match o {
            NveOverflow::Visible => Overflow::Visible,
            NveOverflow::Hidden => Overflow::Hidden,
            NveOverflow::Scroll | NveOverflow::Auto => Overflow::Scroll,
        };
        Point {
            x: map(v.overflow_x()),
            y: map(v.overflow_y()),
        }
    }

    #[inline]
    fn position(&self) -> Position {
        match self.layout().position() {
            NvePosition::Absolute => Position::Absolute,
            NvePosition::Fixed => Position::Fixed,
            // Sticky is computed post-layout over the laid-out rect, matching CSS / `taffy_style.rs`.
            NvePosition::Relative | NvePosition::Sticky => Position::Relative,
        }
    }

    #[inline]
    fn inset(&self) -> Rect<LengthPercentageAuto> {
        let l = self.layout();
        if l.position() == NvePosition::Sticky {
            return Rect::auto();
        }
        Rect {
            top: Self::lpa(l.inset_top()),
            right: Self::lpa(l.inset_right()),
            bottom: Self::lpa(l.inset_bottom()),
            left: Self::lpa(l.inset_left()),
        }
    }

    #[inline]
    fn size(&self) -> Size<Dimension> {
        if self.is_root && self.root_fills_viewport {
            return Size {
                width: Dimension::length(self.viewport.0),
                height: Dimension::length(self.viewport.1),
            };
        }
        let l = self.layout();
        Size {
            width: Self::dim(l.width()),
            height: Self::dim(l.height()),
        }
    }

    #[inline]
    fn min_size(&self) -> Size<LengthPercentageAuto> {
        let l = self.layout();
        Size {
            width: Self::lpa(l.min_width()),
            height: Self::lpa(l.min_height()),
        }
    }

    #[inline]
    fn max_size(&self) -> Size<LengthPercentageAuto> {
        let l = self.layout();
        Size {
            width: Self::lpa(l.max_width()),
            height: Self::lpa(l.max_height()),
        }
    }

    #[inline]
    fn aspect_ratio(&self) -> Option<f32> {
        self.layout()
            .aspect_ratio()
            .filter(|&r| r.is_finite() && r > 0.0)
    }

    #[inline]
    fn margin(&self) -> Rect<LengthPercentageAuto> {
        let l = self.layout();
        let (left, right) = if self.rtl && l.margin_inline_logical() {
            (l.margin_right(), l.margin_left())
        } else {
            (l.margin_left(), l.margin_right())
        };
        Rect {
            top: Self::lpa(l.margin_top()),
            right: Self::lpa(right),
            bottom: Self::lpa(l.margin_bottom()),
            left: Self::lpa(left),
        }
    }

    #[inline]
    fn padding(&self) -> Rect<LengthPercentage> {
        let l = self.layout();
        let (left, right) = if self.rtl && l.padding_inline_logical() {
            (l.padding_right(), l.padding_left())
        } else {
            (l.padding_left(), l.padding_right())
        };
        Rect {
            top: Self::lp(l.padding_top()),
            right: Self::lp(right),
            bottom: Self::lp(l.padding_bottom()),
            left: Self::lp(left),
        }
    }

    #[inline]
    fn border(&self) -> Rect<LengthPercentage> {
        let bw = self.visual().border_width();
        if bw > 0.0 && bw.is_finite() {
            Rect::length(bw)
        } else {
            Rect::zero()
        }
    }
}

impl FlexboxContainerStyle for StyleView<'_> {
    #[inline]
    fn flex_direction(&self) -> FlexDirection {
        match self.layout().flex_direction() {
            NveFlexDirection::Column => FlexDirection::Column,
            NveFlexDirection::Row => FlexDirection::Row,
            NveFlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
            NveFlexDirection::RowReverse => FlexDirection::RowReverse,
        }
    }

    #[inline]
    fn flex_wrap(&self) -> FlexWrap {
        match self.layout().flex_wrap() {
            NveFlexWrap::NoWrap => FlexWrap::NoWrap,
            NveFlexWrap::Wrap => FlexWrap::Wrap,
            NveFlexWrap::WrapReverse => FlexWrap::WrapReverse,
        }
    }

    #[inline]
    fn gap(&self) -> Size<LengthPercentage> {
        let l = self.layout();
        Size {
            width: Self::lp(l.column_gap()),
            height: Self::lp(l.row_gap()),
        }
    }

    #[inline]
    fn align_content(&self) -> AlignContent {
        match self.layout().align_content() {
            NveAlignContent::Stretch => AlignContent::STRETCH,
            NveAlignContent::FlexStart => AlignContent::FLEX_START,
            NveAlignContent::Center => AlignContent::CENTER,
            NveAlignContent::FlexEnd => AlignContent::FLEX_END,
            NveAlignContent::SpaceBetween => AlignContent::SPACE_BETWEEN,
            NveAlignContent::SpaceAround => AlignContent::SPACE_AROUND,
            NveAlignContent::SpaceEvenly => AlignContent::SPACE_EVENLY,
        }
    }

    #[inline]
    fn align_items(&self) -> AlignItems {
        match self.layout().align_items() {
            NveAlignItems::Stretch => AlignItems::STRETCH,
            NveAlignItems::FlexStart => AlignItems::FLEX_START,
            NveAlignItems::Center => AlignItems::CENTER,
            NveAlignItems::FlexEnd => AlignItems::FLEX_END,
            NveAlignItems::Baseline => AlignItems::BASELINE,
        }
    }

    #[inline]
    fn justify_content(&self) -> AlignContent {
        match self.layout().justify_content() {
            NveJustifyContent::FlexStart => AlignContent::FLEX_START,
            NveJustifyContent::Center => AlignContent::CENTER,
            NveJustifyContent::FlexEnd => AlignContent::FLEX_END,
            NveJustifyContent::SpaceBetween => AlignContent::SPACE_BETWEEN,
            NveJustifyContent::SpaceAround => AlignContent::SPACE_AROUND,
            NveJustifyContent::SpaceEvenly => AlignContent::SPACE_EVENLY,
        }
    }
}

impl FlexboxItemStyle for StyleView<'_> {
    #[inline]
    fn flex_basis(&self) -> Dimension {
        Self::dim(self.layout().flex_basis())
    }

    #[inline]
    fn flex_grow(&self) -> f32 {
        self.layout().flex_grow()
    }

    #[inline]
    fn flex_shrink(&self) -> f32 {
        self.layout().flex_shrink()
    }

    #[inline]
    fn align_self(&self) -> Option<AlignSelf> {
        match self.layout().align_self() {
            NveAlignSelf::Auto => None,
            NveAlignSelf::Stretch => Some(AlignSelf::STRETCH),
            NveAlignSelf::FlexStart => Some(AlignSelf::FLEX_START),
            NveAlignSelf::Center => Some(AlignSelf::CENTER),
            NveAlignSelf::FlexEnd => Some(AlignSelf::FLEX_END),
            NveAlignSelf::Baseline => Some(AlignSelf::BASELINE),
        }
    }
}

impl GridContainerStyle for StyleView<'_> {
    type Repetition<'a>
        = WordRepetition
    where
        Self: 'a;
    type TemplateTrackList<'a>
        = GridTrackIter<'a>
    where
        Self: 'a;
    type AutoTrackList<'a>
        = core::iter::Empty<TrackSizingFunction>
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = NoNames<'a>
    where
        Self: 'a;
    type GridTemplateAreas<'a>
        = core::iter::Empty<GridTemplateArea<String>>
    where
        Self: 'a;

    fn grid_template_rows(&self) -> Option<Self::TemplateTrackList<'_>> {
        let g = self.store.try_row::<GridLayout>(self.id)?;
        let res = self.store.resource::<GridTracks>(g.template_rows())?;
        (!res.is_empty()).then(|| GridTrackIter(res.iter()))
    }

    fn grid_template_columns(&self) -> Option<Self::TemplateTrackList<'_>> {
        let g = self.store.try_row::<GridLayout>(self.id)?;
        let res = self.store.resource::<GridTracks>(g.template_columns())?;
        (!res.is_empty()).then(|| GridTrackIter(res.iter()))
    }

    fn grid_auto_rows(&self) -> Self::AutoTrackList<'_> {
        core::iter::empty()
    }

    fn grid_auto_columns(&self) -> Self::AutoTrackList<'_> {
        core::iter::empty()
    }

    fn grid_template_areas(&self) -> Option<Self::GridTemplateAreas<'_>> {
        None
    }

    fn grid_template_column_names(&self) -> Option<Self::TemplateLineNames<'_>> {
        None
    }

    fn grid_template_row_names(&self) -> Option<Self::TemplateLineNames<'_>> {
        None
    }

    fn gap(&self) -> Size<LengthPercentage> {
        FlexboxContainerStyle::gap(self)
    }

    fn align_content(&self) -> AlignContent {
        FlexboxContainerStyle::align_content(self)
    }

    fn justify_content(&self) -> AlignContent {
        FlexboxContainerStyle::justify_content(self)
    }

    fn align_items(&self) -> AlignItems {
        FlexboxContainerStyle::align_items(self)
    }
}

impl GridItemStyle for StyleView<'_> {
    fn grid_row(&self) -> Line<GridPlacement<String>> {
        self.store
            .try_row::<GridItem>(self.id)
            .map_or(Line::auto(), |g| Self::placement(g.row_start(), g.row_end()))
    }

    fn grid_column(&self) -> Line<GridPlacement<String>> {
        self.store.try_row::<GridItem>(self.id).map_or(Line::auto(), |g| {
            Self::placement(g.column_start(), g.column_end())
        })
    }

    fn align_self(&self) -> Option<AlignSelf> {
        FlexboxItemStyle::align_self(self)
    }
}

impl OofItemStyle for StyleView<'_> {
    fn grid_row(&self) -> Line<GridPlacement<String>> {
        GridItemStyle::grid_row(self)
    }

    fn grid_column(&self) -> Line<GridPlacement<String>> {
        GridItemStyle::grid_column(self)
    }

    fn align_self(&self) -> Option<AlignSelf> {
        FlexboxItemStyle::align_self(self)
    }
}
