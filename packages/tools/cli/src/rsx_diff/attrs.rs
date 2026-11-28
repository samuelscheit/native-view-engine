//! Registry of patchable native-element style attributes:
//! - [`lookup_standalone_attr`] covers ONLY the exact numeric standalone RSX attributes handled
//!   by `nve_dioxus::NveMutationWriter` (`width`, `height`, `padding`, `opacity`, `z-index`, ...).
//! - [`lookup_typed_style_arg`] covers scalar/enum/color builder methods inside
//!   `nve_style: TypedStyle::...` chains, with property-overlap bitmasks ([`prop_bits`]) so
//!   overridden calls or `.merge*` chains never clobber later setters or caller overrides.

use super::eval::TypedAttrValue;

/// Bitset of [`TypedAttrValue`] classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueClasses(u16);

impl ValueClasses {
    pub const NONE: Self = Self(0);
    pub const STRING: Self = Self(1);
    pub const BOOL: Self = Self(1 << 1);
    pub const INT: Self = Self(1 << 2);
    pub const FLOAT: Self = Self(1 << 3);
    /// `Length::points(..)`, `Length::ZERO`, `Dimension::Points(..)`.
    pub const PX: Self = Self(1 << 4);
    /// `Length::percent(..)`, `Dimension::Percent(..)`.
    pub const PCT: Self = Self(1 << 5);
    /// `Length::AUTO`, `Dimension::Auto`.
    pub const AUTO: Self = Self(1 << 6);
    pub const COLOR: Self = Self(1 << 7);
    pub const KEYWORD: Self = Self(1 << 8);
    pub const THEME: Self = Self(1 << 9);

    pub const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

/// Property-slot bitmask for overlap detection inside a `TypedStyle` builder chain or between
/// `nve_style` and standalone numeric attributes on the same element.
pub(super) mod prop_bits {
    pub const WIDTH: u64 = 1 << 0;
    pub const HEIGHT: u64 = 1 << 1;
    pub const MIN_WIDTH: u64 = 1 << 2;
    pub const MIN_HEIGHT: u64 = 1 << 3;
    pub const MAX_WIDTH: u64 = 1 << 4;
    pub const MAX_HEIGHT: u64 = 1 << 5;
    pub const FLEX_DIR: u64 = 1 << 6;
    pub const FLEX_WRAP: u64 = 1 << 7;
    pub const JUSTIFY: u64 = 1 << 8;
    pub const ALIGN_ITEMS: u64 = 1 << 9;
    pub const ALIGN_SELF: u64 = 1 << 10;
    pub const FLEX_GROW: u64 = 1 << 11;
    pub const FLEX_SHRINK: u64 = 1 << 12;
    pub const FLEX_BASIS: u64 = 1 << 13;
    pub const ROW_GAP: u64 = 1 << 14;
    pub const COL_GAP: u64 = 1 << 15;
    pub const PAD_TOP: u64 = 1 << 16;
    pub const PAD_RIGHT: u64 = 1 << 17;
    pub const PAD_BOTTOM: u64 = 1 << 18;
    pub const PAD_LEFT: u64 = 1 << 19;
    pub const MAR_TOP: u64 = 1 << 20;
    pub const MAR_RIGHT: u64 = 1 << 21;
    pub const MAR_BOTTOM: u64 = 1 << 22;
    pub const MAR_LEFT: u64 = 1 << 23;
    pub const INSET_TOP: u64 = 1 << 24;
    pub const INSET_RIGHT: u64 = 1 << 25;
    pub const INSET_BOTTOM: u64 = 1 << 26;
    pub const INSET_LEFT: u64 = 1 << 27;
    pub const DISPLAY: u64 = 1 << 28;
    pub const POSITION: u64 = 1 << 29;
    pub const ASPECT_RATIO: u64 = 1 << 30;
    pub const BG: u64 = 1 << 31;
    pub const BORDER_WIDTH: u64 = 1 << 32;
    pub const BORDER_COLOR: u64 = 1 << 33;
    pub const BORDER_RADIUS: u64 = 1 << 34;
    pub const OPACITY: u64 = 1 << 35;
    pub const Z_INDEX: u64 = 1 << 36;
    pub const OVERFLOW_X: u64 = 1 << 37;
    pub const OVERFLOW_Y: u64 = 1 << 38;
    pub const TEXT_COLOR: u64 = 1 << 39;
    pub const FONT_SIZE: u64 = 1 << 40;
    pub const FONT_WEIGHT: u64 = 1 << 41;
    pub const FONT_STYLE: u64 = 1 << 42;
    pub const LINE_HEIGHT: u64 = 1 << 43;
    pub const TEXT_ALIGN: u64 = 1 << 44;
    pub const WHITE_SPACE: u64 = 1 << 45;
    pub const OVERFLOW_WRAP: u64 = 1 << 46;
    pub const TEXT_DECORATION: u64 = 1 << 47;
    pub const CURSOR: u64 = 1 << 48;

    pub const GAP: u64 = ROW_GAP | COL_GAP;
    pub const PAD_X: u64 = PAD_LEFT | PAD_RIGHT;
    pub const PAD_Y: u64 = PAD_TOP | PAD_BOTTOM;
    pub const PAD_ALL: u64 = PAD_X | PAD_Y;
    pub const MAR_X: u64 = MAR_LEFT | MAR_RIGHT;
    pub const MAR_Y: u64 = MAR_TOP | MAR_BOTTOM;
    pub const MAR_ALL: u64 = MAR_X | MAR_Y;
    pub const FLEX_SHORTHAND: u64 = FLEX_GROW | FLEX_SHRINK | FLEX_BASIS;
    pub const OVERFLOW_BOTH: u64 = OVERFLOW_X | OVERFLOW_Y;
    pub const ALL: u64 = u64::MAX;
}

/// How a compiled float literal reaches the store on the typed attribute path (`nve-dioxus`
/// `apply_numeric_attr` / `TypedStyle` builders), mirrored so binary `RowPatch` lowering lands on
/// the exact same value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberRule {
    /// `Length::points(v.max(min))`.
    Px { min: f32 },
    /// Unitless `v.clamp(min, max)`.
    Unitless { min: f32, max: f32 },
    /// `v as i32` (integer literals clamp into `i32`).
    ZIndex,
}

/// A keyword attribute's real `#[repr(u8)]` Rust enum and its `(VariantName, discriminant)` table.
#[derive(Debug, PartialEq, Eq)]
pub struct KeywordEnum {
    pub ty: &'static str,
    /// Optional `nve_abi` / `nve_tables::layout_ext` enum alias sharing identical variant names
    /// and discriminants (e.g. `FlexJustify`, `FlexAlign`, `PositionMode`).
    pub alt_ty: Option<&'static str>,
    pub variants: &'static [(&'static str, u8)],
}

impl KeywordEnum {
    #[inline]
    pub fn matches_ty(&self, ty: &str) -> bool {
        ty == self.ty || self.alt_ty == Some(ty)
    }

    /// Returns the `u8` discriminant of `variant` when it belongs to this enum.
    pub fn discriminant(&self, variant: &str) -> Option<u8> {
        self.variants
            .iter()
            .find(|(v, _)| *v == variant)
            .map(|(_, d)| *d)
    }

    /// Matches a kebab-case or lowercase keyword string (e.g. `"space-between"`, `"nowrap"`)
    /// case-insensitively with `-` ignored against this enum's Rust variant names.
    pub fn parse_keyword_str(&self, raw: &str) -> Option<u8> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        self.variants.iter().find_map(|&(name, disc)| {
            eq_ignore_ascii_case_and_dash(name, trimmed).then_some(disc)
        })
    }
}

fn eq_ignore_ascii_case_and_dash(variant: &str, keyword: &str) -> bool {
    let mut a = variant.bytes();
    let mut b = keyword.bytes().filter(|&c| c != b'-' && c != b'_');
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) if x.eq_ignore_ascii_case(&y) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// Value domain of an attribute or `TypedStyle` builder argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttrKind {
    /// Box dimension: `Dimension::*` / `Length::*` (`px`/`%`/`auto`); `min` bounds `Points`/`Percent`.
    Dimension { min: f32 },
    /// Percentage float (`TypedStyle::width_pct(50.0)` -> `Length::percent(v.clamp(0.0, 100.0))`).
    Percent,
    /// Pixel-only length; float literals per `float`.
    Pixels { float: Option<NumberRule> },
    /// Standalone numeric RSX attribute (`width: 120.0`): float only (matches `NveMutationWriter`).
    StandalonePx { min: f32 },
    /// Unitless number.
    Number(NumberRule),
    /// Font weight (`u16` in `1..=1000`).
    FontWeight,
    /// Line height (`Length::points`, `Length::percent`, `Length::AUTO`, or positive ratio float).
    LineHeight,
    /// `TextDecoration` bitflags constant or keyword string.
    TextDecoration,
    Color,
    Keyword(&'static KeywordEnum),
    /// `TypedStyle::*_str("...")` keyword string helper.
    KeywordStr(&'static KeywordEnum),
}

/// A patchable attribute or `TypedStyle` argument specification.
#[derive(Debug, PartialEq)]
pub struct AttrSpec {
    /// Canonical kebab-case NVE name.
    pub name: &'static str,
    pub kind: AttrKind,
    /// Property slots written by this attribute (for chain/standalone overlap guards).
    pub bits: u64,
}

impl AttrSpec {
    /// Value classes this attribute accepts.
    pub const fn accepts(&self) -> ValueClasses {
        match self.kind {
            AttrKind::Dimension { .. } => ValueClasses::PX
                .or(ValueClasses::PCT)
                .or(ValueClasses::AUTO)
                .or(ValueClasses::THEME),
            AttrKind::Percent => ValueClasses::FLOAT.or(ValueClasses::THEME),
            AttrKind::Pixels { float } => ValueClasses::PX
                .or(ValueClasses::THEME)
                .or(float_class(float)),
            AttrKind::StandalonePx { .. } => ValueClasses::FLOAT.or(ValueClasses::THEME),
            AttrKind::Number(NumberRule::ZIndex) => ValueClasses::FLOAT.or(ValueClasses::INT),
            AttrKind::Number(_) => ValueClasses::FLOAT.or(ValueClasses::THEME),
            AttrKind::FontWeight => ValueClasses::INT,
            AttrKind::LineHeight => ValueClasses::PX
                .or(ValueClasses::PCT)
                .or(ValueClasses::AUTO)
                .or(ValueClasses::FLOAT),
            AttrKind::TextDecoration => ValueClasses::KEYWORD.or(ValueClasses::STRING),
            AttrKind::Color => ValueClasses::COLOR.or(ValueClasses::THEME),
            AttrKind::Keyword(_) => ValueClasses::KEYWORD,
            AttrKind::KeywordStr(_) => ValueClasses::STRING,
        }
    }

    /// Whether `value` is a well-typed value for this attribute (keyword enums: the type must
    /// match and the variant must exist).
    pub fn accepts_value(&self, value: &TypedAttrValue) -> bool {
        if !self.accepts().intersects(value.class()) {
            return false;
        }
        match (self.kind, value) {
            (AttrKind::Keyword(kw), TypedAttrValue::Enum { ty, variant }) => {
                ty.as_deref().is_none_or(|t| kw.matches_ty(t)) && kw.discriminant(variant).is_some()
            }
            (AttrKind::TextDecoration, TypedAttrValue::Enum { ty, variant }) => {
                ty.as_deref().is_none_or(|t| t == "TextDecoration")
                    && matches!(
                        variant.as_str(),
                        "EMPTY" | "UNDERLINE" | "OVERLINE" | "LINE_THROUGH"
                    )
            }
            _ => true,
        }
    }
}

const fn float_class(rule: Option<NumberRule>) -> ValueClasses {
    match rule {
        Some(_) => ValueClasses::FLOAT,
        None => ValueClasses::NONE,
    }
}

/// Declares a keyword enum table pulling compile-checked `#[repr(u8)]` discriminants directly
/// from the real table enum (zero CSS strings, zero duplicated enum match arms).
macro_rules! keyword_enum {
    ($name:ident: $ty:ident $(| $alt:ident)? = $enum_ty:ty { $($variant:ident),* $(,)? }) => {
        pub const $name: KeywordEnum = KeywordEnum {
            ty: stringify!($ty),
            alt_ty: keyword_enum!(@alt $($alt)?),
            variants: &[$((stringify!($variant), <$enum_ty>::$variant as u8)),*],
        };
    };
    (@alt) => { None };
    (@alt $alt:ident) => { Some(stringify!($alt)) };
}

keyword_enum!(DISPLAY: Display = nve_tables::tables::Display {
    Flex, None, Grid, Block, Inline, InlineBlock, InlineFlex, Contents,
});
keyword_enum!(FLEX_DIRECTION: FlexDirection = nve_tables::tables::FlexDirection {
    Column, Row, ColumnReverse, RowReverse,
});
keyword_enum!(FLEX_WRAP: FlexWrap = nve_tables::tables::FlexWrap {
    NoWrap, Wrap, WrapReverse,
});
keyword_enum!(JUSTIFY_CONTENT: JustifyContent | FlexJustify = nve_tables::tables::JustifyContent {
    FlexStart, Center, FlexEnd, SpaceBetween, SpaceAround, SpaceEvenly,
});
keyword_enum!(ALIGN_ITEMS: AlignItems | FlexAlign = nve_tables::tables::AlignItems {
    Stretch, FlexStart, Center, FlexEnd, Baseline,
});
keyword_enum!(ALIGN_SELF: AlignSelf = nve_tables::tables::AlignSelf {
    Auto, Stretch, FlexStart, Center, FlexEnd, Baseline,
});
keyword_enum!(POSITION: PositionMode = nve_tables::tables::Position {
    Relative, Absolute, Sticky,
});
keyword_enum!(OVERFLOW: Overflow = nve_tables::tables::Overflow {
    Visible, Hidden, Scroll, Auto,
});
keyword_enum!(OVERFLOW_WRAP: OverflowWrap = nve_tables::tables::OverflowWrap {
    Normal, Anywhere, BreakWord,
});
keyword_enum!(WHITE_SPACE: WhiteSpace = nve_tables::tables::WhiteSpace {
    Normal, NoWrap, Pre, PreWrap, PreLine, BreakSpaces,
});
keyword_enum!(TEXT_ALIGN: TextAlign = nve_tables::tables::TextAlign {
    Start, End, Left, Right, Center, Justify,
});
keyword_enum!(FONT_STYLE: FontStyle = nve_tables::tables::FontStyle {
    Normal, Italic, Oblique,
});
keyword_enum!(CURSOR: CursorKind = nve_tables::CursorKind {
    Auto, Default, Pointer, Text, Grab, Grabbing, NotAllowed,
    ColResize, RowResize, Move, Crosshair, Progress, Wait,
});

const fn spec(name: &'static str, kind: AttrKind, bits: u64) -> AttrSpec {
    AttrSpec { name, kind, bits }
}

const NON_NEGATIVE_PX: Option<NumberRule> = Some(NumberRule::Px { min: 0.0 });
const SIGNED_PX: Option<NumberRule> = Some(NumberRule::Px {
    min: f32::NEG_INFINITY,
});
const STANDALONE_PX: AttrKind = AttrKind::StandalonePx { min: 0.0 };
const STANDALONE_SIGNED: AttrKind = AttrKind::StandalonePx {
    min: f32::NEG_INFINITY,
};
const DIM: AttrKind = AttrKind::Dimension { min: 0.0 };
const INSET_DIM: AttrKind = AttrKind::Dimension {
    min: f32::NEG_INFINITY,
};
const PX: AttrKind = AttrKind::Pixels {
    float: NON_NEGATIVE_PX,
};
const SIGNED: AttrKind = AttrKind::Pixels { float: SIGNED_PX };
const GROW: AttrKind = AttrKind::Number(NumberRule::Unitless {
    min: 0.0,
    max: f32::INFINITY,
});
const OPACITY: AttrKind = AttrKind::Number(NumberRule::Unitless { min: 0.0, max: 1.0 });
const Z_INDEX: AttrKind = AttrKind::Number(NumberRule::ZIndex);
const FONT_SIZE_PX: AttrKind = AttrKind::Pixels {
    float: Some(NumberRule::Px { min: 1.0 }),
};

// ============================================================================
// Standalone native RSX numeric attributes (exact 22 handled by NveMutationWriter)
// ============================================================================

macro_rules! standalone_table {
    ($($name:literal => ($kind:expr, $bits:expr);)*) => {
        fn lookup_standalone_canonical(name: &str) -> Option<&'static AttrSpec> {
            match name {
                $($name => Some(const { &spec($name, $kind, $bits) }),)*
                _ => None,
            }
        }

        /// Every standalone numeric native RSX attribute's canonical kebab-case name.
        #[cfg(test)]
        pub(super) const STANDALONE_ATTR_NAMES: &[&str] = &[$($name),*];
    };
}

standalone_table! {
    "width" => (STANDALONE_PX, prop_bits::WIDTH);
    "height" => (STANDALONE_PX, prop_bits::HEIGHT);
    "min-width" => (STANDALONE_PX, prop_bits::MIN_WIDTH);
    "min-height" => (STANDALONE_PX, prop_bits::MIN_HEIGHT);
    "max-width" => (STANDALONE_PX, prop_bits::MAX_WIDTH);
    "max-height" => (STANDALONE_PX, prop_bits::MAX_HEIGHT);
    "flex" => (GROW, prop_bits::FLEX_GROW);
    "flex-grow" => (GROW, prop_bits::FLEX_GROW);
    "flex-shrink" => (GROW, prop_bits::FLEX_SHRINK);
    "flex-basis" => (STANDALONE_PX, prop_bits::FLEX_BASIS);
    "gap" => (STANDALONE_PX, prop_bits::GAP);
    "row-gap" => (STANDALONE_PX, prop_bits::ROW_GAP);
    "column-gap" => (STANDALONE_PX, prop_bits::COL_GAP);
    "padding" => (STANDALONE_PX, prop_bits::PAD_ALL);
    "padding-horizontal" => (STANDALONE_PX, prop_bits::PAD_X);
    "padding-vertical" => (STANDALONE_PX, prop_bits::PAD_Y);
    "margin" => (STANDALONE_SIGNED, prop_bits::MAR_ALL);
    "border-width" => (STANDALONE_PX, prop_bits::BORDER_WIDTH);
    "border-radius" => (STANDALONE_PX, prop_bits::BORDER_RADIUS);
    "opacity" => (OPACITY, prop_bits::OPACITY);
    "z-index" => (Z_INDEX, prop_bits::Z_INDEX);
    "font-size" => (AttrKind::StandalonePx { min: 1.0 }, prop_bits::FONT_SIZE);
}

/// Longest canonical attribute name, bounding the stack normalization buffer.
const MAX_ATTR_NAME: usize = 32;

/// Resolves a `snake_case` or `kebab-case` standalone native RSX attribute name to its spec
/// without allocating. Returns `None` for non-existent, identity, event, content, and unknown attrs.
pub fn lookup_standalone_attr(name: &str) -> Option<&'static AttrSpec> {
    let bytes = name.as_bytes();
    if bytes.len() > MAX_ATTR_NAME {
        return None;
    }
    let mut buf = [0u8; MAX_ATTR_NAME];
    for (dst, &b) in buf.iter_mut().zip(bytes) {
        *dst = if b == b'_' { b'-' } else { b };
    }
    lookup_standalone_canonical(std::str::from_utf8(&buf[..bytes.len()]).ok()?)
}

// ============================================================================
// `nve_style: TypedStyle::...` builder method arguments
// ============================================================================

/// Resolves `(method_name, arg_index)` inside a `TypedStyle::...` builder chain to its [`AttrSpec`].
pub fn lookup_typed_style_arg(method: &str, arg_idx: usize) -> Option<&'static AttrSpec> {
    use prop_bits as B;
    match (method, arg_idx) {
        // Box sizing (`Dimension`)
        ("width", 0) => Some(const { &spec("width", DIM, B::WIDTH) }),
        ("height", 0) => Some(const { &spec("height", DIM, B::HEIGHT) }),
        ("min_width", 0) => Some(const { &spec("min-width", DIM, B::MIN_WIDTH) }),
        ("min_height", 0) => Some(const { &spec("min-height", DIM, B::MIN_HEIGHT) }),
        ("max_width", 0) => Some(const { &spec("max-width", DIM, B::MAX_WIDTH) }),
        ("max_height", 0) => Some(const { &spec("max-height", DIM, B::MAX_HEIGHT) }),
        ("flex_basis", 0) => Some(const { &spec("flex-basis", DIM, B::FLEX_BASIS) }),
        ("inset_top", 0) => Some(const { &spec("top", INSET_DIM, B::INSET_TOP) }),
        ("inset_right", 0) => Some(const { &spec("right", INSET_DIM, B::INSET_RIGHT) }),
        ("inset_bottom", 0) => Some(const { &spec("bottom", INSET_DIM, B::INSET_BOTTOM) }),
        ("inset_left", 0) => Some(const { &spec("left", INSET_DIM, B::INSET_LEFT) }),
        // Box sizing (`*_px`)
        ("width_px", 0) => Some(const { &spec("width", PX, B::WIDTH) }),
        ("height_px", 0) => Some(const { &spec("height", PX, B::HEIGHT) }),
        ("min_width_px", 0) => Some(const { &spec("min-width", PX, B::MIN_WIDTH) }),
        ("min_height_px", 0) => Some(const { &spec("min-height", PX, B::MIN_HEIGHT) }),
        ("max_width_px", 0) => Some(const { &spec("max-width", PX, B::MAX_WIDTH) }),
        ("max_height_px", 0) => Some(const { &spec("max-height", PX, B::MAX_HEIGHT) }),
        ("flex_basis_px", 0) => Some(const { &spec("flex-basis", PX, B::FLEX_BASIS) }),
        ("inset_top_px", 0) => Some(const { &spec("top", SIGNED, B::INSET_TOP) }),
        ("inset_right_px", 0) => Some(const { &spec("right", SIGNED, B::INSET_RIGHT) }),
        ("inset_bottom_px", 0) => Some(const { &spec("bottom", SIGNED, B::INSET_BOTTOM) }),
        ("inset_left_px", 0) => Some(const { &spec("left", SIGNED, B::INSET_LEFT) }),
        // Box sizing (`*_pct`)
        ("width_pct", 0) => Some(const { &spec("width", AttrKind::Percent, B::WIDTH) }),
        ("height_pct", 0) => Some(const { &spec("height", AttrKind::Percent, B::HEIGHT) }),
        ("min_width_pct", 0) => Some(const { &spec("min-width", AttrKind::Percent, B::MIN_WIDTH) }),
        ("min_height_pct", 0) => {
            Some(const { &spec("min-height", AttrKind::Percent, B::MIN_HEIGHT) })
        }
        ("max_width_pct", 0) => Some(const { &spec("max-width", AttrKind::Percent, B::MAX_WIDTH) }),
        ("max_height_pct", 0) => {
            Some(const { &spec("max-height", AttrKind::Percent, B::MAX_HEIGHT) })
        }
        ("flex_basis_pct", 0) => {
            Some(const { &spec("flex-basis", AttrKind::Percent, B::FLEX_BASIS) })
        }
        // Flex & layout enums/scalars
        ("display", 0) => Some(const { &spec("display", AttrKind::Keyword(&DISPLAY), B::DISPLAY) }),
        ("position", 0) => {
            Some(const { &spec("position", AttrKind::Keyword(&POSITION), B::POSITION) })
        }
        ("flex_direction", 0) => Some(const {
            &spec(
                "flex-direction",
                AttrKind::Keyword(&FLEX_DIRECTION),
                B::FLEX_DIR,
            )
        }),
        ("flex_direction_str", 0) => Some(const {
            &spec(
                "flex-direction",
                AttrKind::KeywordStr(&FLEX_DIRECTION),
                B::FLEX_DIR,
            )
        }),
        ("flex_wrap", 0) => {
            Some(const { &spec("flex-wrap", AttrKind::Keyword(&FLEX_WRAP), B::FLEX_WRAP) })
        }
        ("justify_content" | "justify", 0) => Some(const {
            &spec(
                "justify-content",
                AttrKind::Keyword(&JUSTIFY_CONTENT),
                B::JUSTIFY,
            )
        }),
        ("justify_content_str", 0) => Some(const {
            &spec(
                "justify-content",
                AttrKind::KeywordStr(&JUSTIFY_CONTENT),
                B::JUSTIFY,
            )
        }),
        ("align_items" | "align", 0) => Some(const {
            &spec(
                "align-items",
                AttrKind::Keyword(&ALIGN_ITEMS),
                B::ALIGN_ITEMS,
            )
        }),
        ("align_items_str", 0) => Some(const {
            &spec(
                "align-items",
                AttrKind::KeywordStr(&ALIGN_ITEMS),
                B::ALIGN_ITEMS,
            )
        }),
        ("align_self", 0) => Some(const {
            &spec("align-self", AttrKind::Keyword(&ALIGN_SELF), B::ALIGN_SELF)
        }),
        ("flex", 0) => Some(const { &spec("flex-shorthand", GROW, B::FLEX_SHORTHAND) }),
        ("flex_grow", 0) => Some(const { &spec("flex-grow", GROW, B::FLEX_GROW) }),
        ("flex_shrink", 0) => Some(const { &spec("flex-shrink", GROW, B::FLEX_SHRINK) }),
        ("aspect_ratio", 0) => Some(const { &spec("aspect-ratio", GROW, B::ASPECT_RATIO) }),
        // Spacing
        ("gap", 0) => Some(const { &spec("gap", PX, B::GAP) }),
        ("row_gap", 0) => Some(const { &spec("row-gap", PX, B::ROW_GAP) }),
        ("column_gap", 0) => Some(const { &spec("column-gap", PX, B::COL_GAP) }),
        ("padding", 0) => Some(const { &spec("padding", PX, B::PAD_ALL) }),
        ("padding_x", 0) => Some(const { &spec("padding-horizontal", PX, B::PAD_X) }),
        ("padding_y", 0) => Some(const { &spec("padding-vertical", PX, B::PAD_Y) }),
        ("padding_xy", 0) => Some(const { &spec("padding-horizontal", PX, B::PAD_X) }),
        ("padding_xy", 1) => Some(const { &spec("padding-vertical", PX, B::PAD_Y) }),
        ("padding_top", 0) => Some(const { &spec("padding-top", PX, B::PAD_TOP) }),
        ("padding_right", 0) => Some(const { &spec("padding-right", PX, B::PAD_RIGHT) }),
        ("padding_bottom", 0) => Some(const { &spec("padding-bottom", PX, B::PAD_BOTTOM) }),
        ("padding_left", 0) => Some(const { &spec("padding-left", PX, B::PAD_LEFT) }),
        ("margin", 0) => Some(const { &spec("margin", SIGNED, B::MAR_ALL) }),
        ("margin_x", 0) => Some(const { &spec("margin-horizontal", SIGNED, B::MAR_X) }),
        ("margin_y", 0) => Some(const { &spec("margin-vertical", SIGNED, B::MAR_Y) }),
        ("margin_xy", 0) => Some(const { &spec("margin-horizontal", SIGNED, B::MAR_X) }),
        ("margin_xy", 1) => Some(const { &spec("margin-vertical", SIGNED, B::MAR_Y) }),
        ("margin_top", 0) => Some(const { &spec("margin-top", SIGNED, B::MAR_TOP) }),
        ("margin_right", 0) => Some(const { &spec("margin-right", SIGNED, B::MAR_RIGHT) }),
        ("margin_bottom", 0) => Some(const { &spec("margin-bottom", SIGNED, B::MAR_BOTTOM) }),
        ("margin_left", 0) => Some(const { &spec("margin-left", SIGNED, B::MAR_LEFT) }),
        // Visual
        ("bg" | "background", 0) => Some(const { &spec("background-color", AttrKind::Color, B::BG) }),
        ("border", 0) => Some(const { &spec("border-width", PX, B::BORDER_WIDTH) }),
        ("border", 1) => Some(const { &spec("border-color", AttrKind::Color, B::BORDER_COLOR) }),
        ("border_width", 0) => Some(const { &spec("border-width", PX, B::BORDER_WIDTH) }),
        ("border_color", 0) => Some(const { &spec("border-color", AttrKind::Color, B::BORDER_COLOR) }),
        ("radius" | "border_radius", 0) => {
            Some(const { &spec("border-radius", PX, B::BORDER_RADIUS) })
        }
        ("opacity", 0) => Some(const { &spec("opacity", OPACITY, B::OPACITY) }),
        ("z_index", 0) => Some(const { &spec("z-index", Z_INDEX, B::Z_INDEX) }),
        ("overflow" | "overflow_mode", 0) => Some(const {
            &spec("overflow", AttrKind::Keyword(&OVERFLOW), B::OVERFLOW_BOTH)
        }),
        ("overflow_x", 0) => {
            Some(const { &spec("overflow-x", AttrKind::Keyword(&OVERFLOW), B::OVERFLOW_X) })
        }
        ("overflow_y", 0) => {
            Some(const { &spec("overflow-y", AttrKind::Keyword(&OVERFLOW), B::OVERFLOW_Y) })
        }
        // Typography
        ("color" | "text_color", 0) => {
            Some(const { &spec("color", AttrKind::Color, B::TEXT_COLOR) })
        }
        ("font_size", 0) => Some(const { &spec("font-size", FONT_SIZE_PX, B::FONT_SIZE) }),
        ("font_weight", 0) => {
            Some(const { &spec("font-weight", AttrKind::FontWeight, B::FONT_WEIGHT) })
        }
        ("font_style", 0) => Some(const {
            &spec("font-style", AttrKind::Keyword(&FONT_STYLE), B::FONT_STYLE)
        }),
        ("line_height", 0) => {
            Some(const { &spec("line-height", AttrKind::LineHeight, B::LINE_HEIGHT) })
        }
        ("line_height_px", 0) => Some(const { &spec("line-height-px", PX, B::LINE_HEIGHT) }),
        ("line_height_ratio", 0) => {
            Some(const { &spec("line-height", AttrKind::LineHeight, B::LINE_HEIGHT) })
        }
        ("text_align", 0) => Some(const {
            &spec("text-align", AttrKind::Keyword(&TEXT_ALIGN), B::TEXT_ALIGN)
        }),
        ("white_space", 0) => Some(const {
            &spec(
                "white-space",
                AttrKind::Keyword(&WHITE_SPACE),
                B::WHITE_SPACE,
            )
        }),
        ("overflow_wrap", 0) => Some(const {
            &spec(
                "overflow-wrap",
                AttrKind::Keyword(&OVERFLOW_WRAP),
                B::OVERFLOW_WRAP,
            )
        }),
        ("text_decoration", 0) => Some(const {
            &spec(
                "text-decoration",
                AttrKind::TextDecoration,
                B::TEXT_DECORATION,
            )
        }),
        // Interaction
        ("cursor", 0) => Some(const { &spec("cursor", AttrKind::Keyword(&CURSOR), B::CURSOR) }),
        _ => None,
    }
}

/// Static element identity attributes (`test_id` and its aliases).
pub fn is_test_id_attr(name: &str) -> bool {
    matches!(name, "test_id" | "testID" | "data-testid")
}

/// Native element tags of `nve_dioxus::dioxus_elements` (everything else is a component).
pub fn is_native_tag(tag: &str) -> bool {
    matches!(
        tag,
        "view"
            | "div"
            | "text"
            | "span"
            | "p"
            | "a"
            | "pressable"
            | "button"
            | "scrollview"
            | "image"
            | "textinput"
            | "input"
    )
}
