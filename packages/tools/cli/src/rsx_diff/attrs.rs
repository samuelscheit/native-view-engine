//! Registry of patchable native-element style attributes: the typed value classes each accepts
//! (for `TypedRsxPatch`) and the `#[repr(u8)]` keyword enum discriminants for direct binary
//! `RowPatch` word lowering (zero CSS string emission).
//!
//! Identity (`test_id`, `key`), event handlers (`on*`), content props and every unknown attribute
//! are deliberately absent: an edit to them is a logic/structure change that needs a Tier-2 build.

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

/// How a compiled float literal reaches the store on the typed attribute path (`nve-dioxus`
/// `apply_numeric_attr`), mirrored so binary `RowPatch` lowering lands on the exact same value.
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
    pub variants: &'static [(&'static str, u8)],
}

impl KeywordEnum {
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

/// Value domain of an attribute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttrKind {
    /// Non-scalar or shorthand text attributes (`box-shadow`, `font-family`, `border`): cannot be
    /// encoded into scalar `RowPatch` words and fall back cleanly to Tier-2.
    Text,
    /// Box dimension: `px`/`%`/`auto` lengths; float literals per `float`.
    Dimension { float: Option<NumberRule> },
    /// Pixel-only length; float literals per `float`.
    Pixels { float: Option<NumberRule> },
    /// Unitless number.
    Number(NumberRule),
    /// Font weight (`"bold"`, `"semibold"`, `"600"` or numeric `1..=1000`).
    FontWeight,
    /// Line height (`Length::points`, `Length::percent`, `Length::AUTO`, unitless ratio or string).
    LineHeight,
    /// `text-decoration-line` bitflags (`"none"`, `"underline"`, `"line-through"`, `"overline"`).
    TextDecoration,
    Color,
    Keyword(&'static KeywordEnum),
}

/// A patchable attribute.
#[derive(Debug, PartialEq)]
pub struct AttrSpec {
    /// Canonical kebab-case NVE name.
    pub name: &'static str,
    pub kind: AttrKind,
}

impl AttrSpec {
    /// Value classes this attribute accepts.
    pub const fn accepts(&self) -> ValueClasses {
        let s = ValueClasses::STRING;
        match self.kind {
            AttrKind::Text | AttrKind::TextDecoration => s,
            AttrKind::Dimension { float } => s
                .or(ValueClasses::PX)
                .or(ValueClasses::PCT)
                .or(ValueClasses::AUTO)
                .or(ValueClasses::THEME)
                .or(float_class(float)),
            AttrKind::Pixels { float } => s
                .or(ValueClasses::PX)
                .or(ValueClasses::THEME)
                .or(float_class(float)),
            AttrKind::Number(NumberRule::ZIndex) => s.or(ValueClasses::FLOAT).or(ValueClasses::INT),
            AttrKind::Number(_) => s.or(ValueClasses::FLOAT).or(ValueClasses::THEME),
            AttrKind::FontWeight => s.or(ValueClasses::INT),
            AttrKind::LineHeight => s
                .or(ValueClasses::PX)
                .or(ValueClasses::PCT)
                .or(ValueClasses::AUTO)
                .or(ValueClasses::FLOAT),
            AttrKind::Color => s.or(ValueClasses::COLOR).or(ValueClasses::THEME),
            AttrKind::Keyword(_) => s.or(ValueClasses::KEYWORD),
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
                ty.as_deref().is_none_or(|t| t == kw.ty) && kw.discriminant(variant).is_some()
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
    ($name:ident: $ty:ident = $enum_ty:ty { $($variant:ident),* $(,)? }) => {
        pub const $name: KeywordEnum = KeywordEnum {
            ty: stringify!($ty),
            variants: &[$((stringify!($variant), <$enum_ty>::$variant as u8)),*],
        };
    };
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
keyword_enum!(JUSTIFY_CONTENT: JustifyContent = nve_tables::tables::JustifyContent {
    FlexStart, Center, FlexEnd, SpaceBetween, SpaceAround, SpaceEvenly,
});
keyword_enum!(ALIGN_ITEMS: AlignItems = nve_tables::tables::AlignItems {
    Stretch, FlexStart, Center, FlexEnd, Baseline,
});
keyword_enum!(ALIGN_SELF: AlignSelf = nve_tables::tables::AlignSelf {
    Auto, Stretch, FlexStart, Center, FlexEnd, Baseline,
});
keyword_enum!(POSITION: Position = nve_tables::tables::Position {
    Relative, Absolute, Sticky, Fixed,
});
keyword_enum!(BOX_SIZING: BoxSizing = nve_tables::tables::BoxSizing {
    BorderBox, ContentBox,
});
keyword_enum!(VISIBILITY: Visibility = nve_tables::tables::Visibility {
    Inherit, Visible, Hidden,
});
keyword_enum!(OVERFLOW: Overflow = nve_tables::tables::Overflow {
    Visible, Hidden, Scroll, Auto,
});
keyword_enum!(OVERFLOW_WRAP: OverflowWrap = nve_tables::tables::OverflowWrap {
    Normal, Anywhere, BreakWord,
});
keyword_enum!(WORD_BREAK: WordBreak = nve_tables::tables::WordBreak {
    Normal, BreakAll, KeepAll, BreakWord,
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
keyword_enum!(VERTICAL_ALIGN: VerticalAlign = nve_tables::tables::VerticalAlign {
    Baseline, Top, Middle, Bottom, Sub, Super, TextTop, TextBottom,
});
keyword_enum!(CURSOR: CursorKind = nve_tables::CursorKind {
    Auto, Default, Pointer, Text, Grab, Grabbing, NotAllowed,
    ColResize, RowResize, Move, Crosshair, Progress, Wait,
});

const fn spec(name: &'static str, kind: AttrKind) -> AttrSpec {
    AttrSpec { name, kind }
}

const NON_NEGATIVE_PX: Option<NumberRule> = Some(NumberRule::Px { min: 0.0 });
const SIGNED_PX: Option<NumberRule> = Some(NumberRule::Px {
    min: f32::NEG_INFINITY,
});
const DIM: AttrKind = AttrKind::Dimension {
    float: NON_NEGATIVE_PX,
};
const INSET: AttrKind = AttrKind::Dimension { float: SIGNED_PX };
const PX: AttrKind = AttrKind::Pixels {
    float: NON_NEGATIVE_PX,
};
const SIGNED: AttrKind = AttrKind::Pixels { float: SIGNED_PX };
const GROW: AttrKind = AttrKind::Number(NumberRule::Unitless {
    min: 0.0,
    max: f32::INFINITY,
});

macro_rules! attr_table {
    ($($name:literal => $kind:expr;)*) => {
        fn lookup_canonical(name: &str) -> Option<&'static AttrSpec> {
            match name {
                $($name => Some(const { &spec($name, $kind) }),)*
                _ => None,
            }
        }

        /// Every patchable attribute's canonical kebab-case name.
        #[cfg(test)]
        pub(super) const PATCHABLE_ATTR_NAMES: &[&str] = &[$($name),*];
    };
}

attr_table! {
    // Box sizing.
    "width" => DIM; "height" => DIM;
    "min-width" => DIM; "min-height" => DIM;
    "max-width" => DIM; "max-height" => DIM;
    "box-sizing" => AttrKind::Keyword(&BOX_SIZING);
    "aspect-ratio" => AttrKind::Number(NumberRule::Unitless { min: 0.0, max: f32::INFINITY });
    // Flex layout.
    "display" => AttrKind::Keyword(&DISPLAY);
    "position" => AttrKind::Keyword(&POSITION);
    "flex-direction" => AttrKind::Keyword(&FLEX_DIRECTION);
    "flex-wrap" => AttrKind::Keyword(&FLEX_WRAP);
    "justify-content" => AttrKind::Keyword(&JUSTIFY_CONTENT);
    "align-items" => AttrKind::Keyword(&ALIGN_ITEMS);
    "align-self" => AttrKind::Keyword(&ALIGN_SELF);
    "flex" => GROW; "flex-grow" => GROW; "flex-shrink" => GROW;
    "flex-basis" => AttrKind::Dimension { float: NON_NEGATIVE_PX };
    "gap" => PX; "row-gap" => PX; "column-gap" => PX;
    "top" => INSET; "right" => INSET; "bottom" => INSET; "left" => INSET;
    // Spacing.
    "padding" => PX; "padding-horizontal" => PX; "padding-vertical" => PX;
    "padding-top" => AttrKind::Pixels { float: None };
    "padding-right" => AttrKind::Pixels { float: None };
    "padding-bottom" => AttrKind::Pixels { float: None };
    "padding-left" => AttrKind::Pixels { float: None };
    "margin" => SIGNED;
    "margin-top" => SIGNED; "margin-right" => SIGNED;
    "margin-bottom" => SIGNED; "margin-left" => SIGNED;
    // Visual.
    "background" => AttrKind::Color; "background-color" => AttrKind::Color;
    "color" => AttrKind::Color; "border-color" => AttrKind::Color;
    "border" => AttrKind::Text;
    "border-width" => PX;
    "border-top-width" => PX; "border-right-width" => PX;
    "border-bottom-width" => PX; "border-left-width" => PX;
    "border-radius" => PX;
    "box-shadow" => AttrKind::Text;
    "opacity" => AttrKind::Number(NumberRule::Unitless { min: 0.0, max: 1.0 });
    "z-index" => AttrKind::Number(NumberRule::ZIndex);
    "visibility" => AttrKind::Keyword(&VISIBILITY);
    "overflow" => AttrKind::Keyword(&OVERFLOW);
    "overflow-x" => AttrKind::Keyword(&OVERFLOW);
    "overflow-y" => AttrKind::Keyword(&OVERFLOW);
    // Typography.
    "font-family" => AttrKind::Text;
    "font-size" => AttrKind::Pixels { float: Some(NumberRule::Px { min: 1.0 }) };
    "font-weight" => AttrKind::FontWeight;
    "font-style" => AttrKind::Keyword(&FONT_STYLE);
    "line-height" => AttrKind::LineHeight;
    "letter-spacing" => SIGNED;
    "text-align" => AttrKind::Keyword(&TEXT_ALIGN);
    "text-decoration" => AttrKind::TextDecoration;
    "white-space" => AttrKind::Keyword(&WHITE_SPACE);
    "overflow-wrap" => AttrKind::Keyword(&OVERFLOW_WRAP);
    "word-break" => AttrKind::Keyword(&WORD_BREAK);
    "vertical-align" => AttrKind::Keyword(&VERTICAL_ALIGN);
    // Interaction.
    "cursor" => AttrKind::Keyword(&CURSOR);
}

/// Longest canonical attribute name (`border-bottom-width`), bounding the normalization buffer.
const MAX_ATTR_NAME: usize = 32;

/// Resolves a `snake_case` or `kebab-case` RSX attribute name to its patchable spec without
/// allocating. Returns `None` for identity, event, content and unknown attributes.
pub fn lookup_patchable_attr(name: &str) -> Option<&'static AttrSpec> {
    let bytes = name.as_bytes();
    if bytes.len() > MAX_ATTR_NAME {
        return None;
    }
    let mut buf = [0u8; MAX_ATTR_NAME];
    for (dst, &b) in buf.iter_mut().zip(bytes) {
        *dst = if b == b'_' { b'-' } else { b };
    }
    lookup_canonical(std::str::from_utf8(&buf[..bytes.len()]).ok()?)
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
