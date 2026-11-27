//! `Layout` (dense) and the CSS Grid tables.

use nve_store::{word_enum, Length, NveTable, Res};

use super::resources::GridTracks;

word_enum! {
    pub enum FlexDirection: 2 { Column = 0, Row = 1, ColumnReverse = 2, RowReverse = 3 }
}

word_enum! {
    pub enum JustifyContent: 3 {
        FlexStart = 0, Center = 1, FlexEnd = 2, SpaceBetween = 3, SpaceAround = 4, SpaceEvenly = 5,
    }
}

word_enum! {
    pub enum AlignItems: 3 { Stretch = 0, FlexStart = 1, Center = 2, FlexEnd = 3, Baseline = 4 }
}

word_enum! {
    /// `Auto` defers to the parent's `align-items`.
    pub enum AlignSelf: 3 {
        Auto = 0, Stretch = 1, FlexStart = 2, Center = 3, FlexEnd = 4, Baseline = 5,
    }
}

word_enum! {
    pub enum AlignContent: 3 {
        Stretch = 0, FlexStart = 1, Center = 2, FlexEnd = 3, SpaceBetween = 4, SpaceAround = 5,
        SpaceEvenly = 6,
    }
}

word_enum! {
    pub enum FlexWrap: 2 { NoWrap = 0, Wrap = 1, WrapReverse = 2 }
}

word_enum! {
    /// Native engines lay out `Block` / `Inline*` as their flex / block equivalents; web emits
    /// them verbatim.
    pub enum Display: 3 {
        Flex = 0, None = 1, Grid = 2, Block = 3, Inline = 4, InlineBlock = 5, InlineFlex = 6,
        Contents = 7,
    }
}

word_enum! {
    pub enum Position: 2 { Relative = 0, Absolute = 1, Sticky = 2, Fixed = 3 }
}

word_enum! {
    pub enum Direction: 2 { Inherit = 0, Ltr = 1, Rtl = 2 }
}

word_enum! {
    pub enum BoxSizing: 1 { BorderBox = 0, ContentBox = 1 }
}

/// Box geometry of every node (Flexbox / Grid item / absolute position). Dense: every node lays
/// out, and keeping insets here keeps the layout loop free of presence branches.
#[derive(NveTable)]
#[nve_table(name = "nve.layout", storage = "dense", flags = "publish")]
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
    /// `width / height`; `NaN` = none.
    #[effect(layout)]
    #[default(f32::NAN)]
    pub aspect_ratio: f32,
    #[effect(layout)]
    #[bits(2)]
    pub flex_direction: FlexDirection,
    #[effect(layout)]
    #[bits(3)]
    pub justify_content: JustifyContent,
    #[effect(layout)]
    #[bits(3)]
    pub align_items: AlignItems,
    #[effect(layout)]
    #[bits(3)]
    pub align_self: AlignSelf,
    #[effect(layout)]
    #[bits(3)]
    pub align_content: AlignContent,
    #[effect(layout)]
    #[bits(2)]
    pub flex_wrap: FlexWrap,
    #[effect(layout)]
    #[bits(3)]
    pub display: Display,
    #[effect(layout)]
    #[bits(2)]
    pub position: Position,
    #[effect(layout)]
    #[bits(2)]
    pub direction: Direction,
    #[effect(layout)]
    #[bits(1)]
    pub box_sizing: BoxSizing,
    /// `padding_left/right` are inline-start/end (flip under RTL).
    #[effect(layout)]
    #[bits(1)]
    pub padding_inline_logical: bool,
    /// `margin_left/right` are inline-start/end (flip under RTL).
    #[effect(layout)]
    #[bits(1)]
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

/// One grid line: `Auto`, a 1-based (negative = from end) line, or a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GridLine {
    #[default]
    Auto,
    Line(i16),
    Span(u16),
}

impl GridLine {
    /// 16-bit packing: tag in bits 14-15, signed 14-bit value below.
    #[inline]
    pub const fn to_word(self) -> u32 {
        match self {
            Self::Auto => 0,
            Self::Line(v) => 1 << 14 | (v as u32 & 0x3FFF),
            Self::Span(v) => 2 << 14 | (v as u32 & 0x3FFF),
        }
    }

    #[inline]
    pub const fn from_word(bits: u32) -> Self {
        let v = bits & 0x3FFF;
        match (bits >> 14) & 0b11 {
            1 => Self::Line(((v << 18) as i32 >> 18) as i16),
            2 => Self::Span(v as u16),
            _ => Self::Auto,
        }
    }
}

impl nve_store::WordValue for GridLine {
    const BITS: u8 = 16;
    const KIND: nve_store::FieldKind = nve_store::FieldKind::Enum;
    #[inline]
    fn to_bits(self) -> u32 {
        self.to_word()
    }
    #[inline]
    fn from_bits(bits: u32) -> Self {
        Self::from_word(bits)
    }
}

/// CSS Grid item placement.
#[derive(NveTable)]
#[nve_table(name = "nve.grid_item", storage = "sparse", flags = "publish")]
pub struct GridItem {
    #[effect(layout)]
    #[bits(16)]
    pub column_start: GridLine,
    #[effect(layout)]
    #[bits(16)]
    pub column_end: GridLine,
    #[effect(layout)]
    #[bits(16)]
    pub row_start: GridLine,
    #[effect(layout)]
    #[bits(16)]
    pub row_end: GridLine,
}

const _: () = {
    assert!(Layout::DESC.defaults.len() == 25);
    assert!(GridItem::DESC.defaults.len() == 2);
};
