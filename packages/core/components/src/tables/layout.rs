//! `Layout` (dense) and the CSS Grid tables.

use nve_store::{Length, NveTable, Res, WordEnum};

pub use nve_store::GridLine;

use super::resources::GridTracks;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum FlexDirection {
    #[default]
    Column = 0,
    Row = 1,
    ColumnReverse = 2,
    RowReverse = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum JustifyContent {
    #[default]
    FlexStart = 0,
    Center = 1,
    FlexEnd = 2,
    SpaceBetween = 3,
    SpaceAround = 4,
    SpaceEvenly = 5,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum AlignItems {
    #[default]
    Stretch = 0,
    FlexStart = 1,
    Center = 2,
    FlexEnd = 3,
    Baseline = 4,
}

/// `Auto` defers to the parent's `align-items`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum AlignSelf {
    #[default]
    Auto = 0,
    Stretch = 1,
    FlexStart = 2,
    Center = 3,
    FlexEnd = 4,
    Baseline = 5,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum AlignContent {
    #[default]
    Stretch = 0,
    FlexStart = 1,
    Center = 2,
    FlexEnd = 3,
    SpaceBetween = 4,
    SpaceAround = 5,
    SpaceEvenly = 6,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum FlexWrap {
    #[default]
    NoWrap = 0,
    Wrap = 1,
    WrapReverse = 2,
}

/// Native engines lay out `Block` / `Inline*` as their flex / block equivalents; web emits
/// them verbatim.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Display {
    #[default]
    Flex = 0,
    None = 1,
    Grid = 2,
    Block = 3,
    Inline = 4,
    InlineBlock = 5,
    InlineFlex = 6,
    Contents = 7,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Position {
    #[default]
    Relative = 0,
    Absolute = 1,
    Sticky = 2,
    Fixed = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Direction {
    #[default]
    Inherit = 0,
    Ltr = 1,
    Rtl = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum BoxSizing {
    #[default]
    BorderBox = 0,
    ContentBox = 1,
}

/// Box geometry of every node (Flexbox / Grid item / absolute position). Dense: every node lays
/// out, and keeping insets here keeps the layout loop free of presence branches.
#[derive(NveTable)]
#[nve_table(
    name = "nve.layout",
    storage = "dense",
    flags = "publish",
    events = nve_abi::LayoutEvent
)]
pub struct Layout {
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub width: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub height: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub min_width: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub min_height: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub max_width: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub max_height: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub flex_basis: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub inset_top: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub inset_right: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub inset_bottom: Length,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub inset_left: Length,
    #[effect(layout)]
    pub flex_grow: f32,
    #[effect(layout)]
    #[default(1.0)]
    pub flex_shrink: f32,
    #[effect(layout)]
    pub row_gap: Length,
    #[effect(layout)]
    pub column_gap: Length,
    #[effect(layout)]
    pub padding_top: Length,
    /// Physical right, or inline-end when `padding_inline_logical`.
    #[effect(layout)]
    pub padding_right: Length,
    #[effect(layout)]
    pub padding_bottom: Length,
    /// Physical left, or inline-start when `padding_inline_logical`.
    #[effect(layout)]
    pub padding_left: Length,
    #[effect(layout)]
    pub margin_top: Length,
    /// Physical right, or inline-end when `margin_inline_logical`.
    #[effect(layout)]
    pub margin_right: Length,
    #[effect(layout)]
    pub margin_bottom: Length,
    /// Physical left, or inline-start when `margin_inline_logical`.
    #[effect(layout)]
    pub margin_left: Length,
    /// Optional `width / height` ratio (`None` = unconstrained).
    #[effect(layout)]
    pub aspect_ratio: Option<f32>,
    #[effect(layout)]
    pub flex_direction: FlexDirection,
    #[effect(layout)]
    pub justify_content: JustifyContent,
    #[effect(layout)]
    pub align_items: AlignItems,
    #[effect(layout)]
    pub align_self: AlignSelf,
    #[effect(layout)]
    pub align_content: AlignContent,
    #[effect(layout)]
    pub flex_wrap: FlexWrap,
    #[effect(layout)]
    pub display: Display,
    #[effect(layout)]
    pub position: Position,
    #[effect(layout)]
    pub direction: Direction,
    #[effect(layout)]
    pub box_sizing: BoxSizing,
    /// `padding_left/right` are inline-start/end (flip under RTL).
    #[effect(layout)]
    pub padding_inline_logical: bool,
    /// `margin_left/right` are inline-start/end (flip under RTL).
    #[effect(layout)]
    pub margin_inline_logical: bool,
}

/// CSS Grid container templates (gaps live in `Layout`).
#[derive(NveTable)]
#[nve_table(name = "nve.grid", storage = "sparse", flags = "publish")]
pub struct GridLayout {
    #[effect(layout)]
    pub template_columns: Res<GridTracks>,
    #[effect(layout)]
    pub template_rows: Res<GridTracks>,
}

/// CSS Grid item placement.
#[derive(NveTable)]
#[nve_table(name = "nve.grid_item", storage = "sparse", flags = "publish")]
pub struct GridItem {
    #[effect(layout)]
    pub column_start: GridLine,
    #[effect(layout)]
    pub column_end: GridLine,
    #[effect(layout)]
    pub row_start: GridLine,
    #[effect(layout)]
    pub row_end: GridLine,
}

const _: () = {
    assert!(Layout::DESC.defaults.len() == 26);
    assert!(GridItem::DESC.defaults.len() == 2);
};
