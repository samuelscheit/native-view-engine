//! `Overlay` sparse table (`nve.overlay`) plus shared anchor geometry enums
//! (`OverlayLayer`, `Side`, `Alignment`, `OverlayPlacement`, `OverlayFlags`).

use nve_store::{word_flags, NveTable, Str, WordEnum};

/// Semantic top-layer tier of a floating [`Overlay`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum OverlayLayer {
    #[default]
    Popover = 0,
    Modal = 1,
    Tooltip = 2,
    Menu = 3,
}

/// Primary side relative to an anchor rectangle.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Side {
    Top = 0,
    #[default]
    Bottom = 1,
    Left = 2,
    Right = 3,
}

impl Side {
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    #[inline]
    pub const fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Cross-axis alignment relative to an anchor rectangle.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Alignment {
    Start = 0,
    #[default]
    Center = 1,
    End = 2,
}

impl Alignment {
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Start => Self::End,
            Self::Center => Self::Center,
            Self::End => Self::Start,
        }
    }
}

/// All 12 anchored overlay placements (4 sides × 3 cross-axis alignments).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum OverlayPlacement {
    Top = 0,
    TopStart = 1,
    TopEnd = 2,
    #[default]
    Bottom = 3,
    BottomStart = 4,
    BottomEnd = 5,
    Left = 6,
    LeftStart = 7,
    LeftEnd = 8,
    Right = 9,
    RightStart = 10,
    RightEnd = 11,
}

impl OverlayPlacement {
    #[inline]
    pub const fn side(self) -> Side {
        match self {
            Self::Top | Self::TopStart | Self::TopEnd => Side::Top,
            Self::Bottom | Self::BottomStart | Self::BottomEnd => Side::Bottom,
            Self::Left | Self::LeftStart | Self::LeftEnd => Side::Left,
            Self::Right | Self::RightStart | Self::RightEnd => Side::Right,
        }
    }

    #[inline]
    pub const fn alignment(self) -> Alignment {
        match self {
            Self::Top | Self::Bottom | Self::Left | Self::Right => Alignment::Center,
            Self::TopStart | Self::BottomStart | Self::LeftStart | Self::RightStart => {
                Alignment::Start
            }
            Self::TopEnd | Self::BottomEnd | Self::LeftEnd | Self::RightEnd => Alignment::End,
        }
    }

    #[inline]
    pub const fn from_side_align(side: Side, align: Alignment) -> Self {
        match (side, align) {
            (Side::Top, Alignment::Center) => Self::Top,
            (Side::Top, Alignment::Start) => Self::TopStart,
            (Side::Top, Alignment::End) => Self::TopEnd,
            (Side::Bottom, Alignment::Center) => Self::Bottom,
            (Side::Bottom, Alignment::Start) => Self::BottomStart,
            (Side::Bottom, Alignment::End) => Self::BottomEnd,
            (Side::Left, Alignment::Center) => Self::Left,
            (Side::Left, Alignment::Start) => Self::LeftStart,
            (Side::Left, Alignment::End) => Self::LeftEnd,
            (Side::Right, Alignment::Center) => Self::Right,
            (Side::Right, Alignment::Start) => Self::RightStart,
            (Side::Right, Alignment::End) => Self::RightEnd,
        }
    }

    /// Flips the primary side (`Bottom -> Top`, `RightStart -> LeftStart`) while keeping alignment.
    #[inline]
    pub const fn opposite(self) -> Self {
        Self::from_side_align(self.side().opposite(), self.alignment())
    }

    /// Flips cross-axis alignment (`Start <-> End`) while keeping the primary side.
    #[inline]
    pub const fn flip_alignment(self) -> Self {
        Self::from_side_align(self.side(), self.alignment().opposite())
    }
}

word_flags! {
    /// Floating overlay behavioral flags packed into 5 bits.
    pub struct OverlayFlags(u8) {
        const OPEN = 1 << 0;
        const FLIP = 1 << 1;
        const SHIFT = 1 << 2;
        const AUTO_SIZE = 1 << 3;
        const BACKDROP_SCRIM = 1 << 4;
    }
}

/// Cross-platform floating overlay table (`nve.overlay`) driving top-layer promotion,
/// anchor-relative positioning, collision flipping/shifting, and modal backdrop scrims.
#[derive(NveTable)]
#[nve_table(name = "nve.overlay", storage = "sparse", flags = "publish")]
pub struct Overlay {
    #[effect(realize, layout, paint)]
    pub layer: OverlayLayer,
    /// Anchor node `DebugId::id` (`""` = unanchored / viewport-centered modal).
    #[effect(layout, paint)]
    pub anchor_id: Str,
    #[effect(layout, paint)]
    pub placement: OverlayPlacement,
    #[effect(layout, paint)]
    #[default(8.0)]
    pub offset_px: f32,
    #[effect(layout, paint)]
    #[default(8.0)]
    pub viewport_padding_px: f32,
    #[effect(realize, layout, paint)]
    pub options: OverlayFlags,
}
