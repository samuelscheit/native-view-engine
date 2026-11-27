//! Registry of patchable native-element style attributes: the typed value classes each accepts
//! (for `TypedRsxPatch`), the keyword enum behind keyword attributes, and whether today's string
//! `TemplatePatch` path (`nve_components::css`) applies the attribute at all.
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
/// `apply_numeric_attr`), mirrored so the lowered CSS string lands on the same value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberRule {
    /// `Length::points(v.max(min))`.
    Px { min: f32 },
    /// Unitless `v.clamp(min, max)`.
    Unitless { min: f32, max: f32 },
    /// `v as i32` (integer literals clamp into `i32`).
    ZIndex,
}

/// A keyword attribute's real Rust enum and the CSS keyword of each variant (`None`: the string
/// CSS path cannot express that variant faithfully).
#[derive(Debug, PartialEq, Eq)]
pub struct KeywordEnum {
    pub ty: &'static str,
    pub variants: &'static [(&'static str, Option<&'static str>)],
}

impl KeywordEnum {
    /// The CSS keyword of `variant`, when the enum has it: `Some(None)` = no faithful CSS form.
    pub fn css(&self, variant: &str) -> Option<Option<&'static str>> {
        self.variants
            .iter()
            .find(|(v, _)| *v == variant)
            .map(|(_, css)| *css)
    }
}

/// Value domain of an attribute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttrKind {
    /// Static strings only (`style`, `box-shadow`, `font-family`, the `border` shorthand).
    Text,
    /// Box dimension: `px`/`%`/`auto` lengths; float literals per `float`.
    Dimension {
        float: Option<NumberRule>,
    },
    /// Pixel-only length; float literals per `float`.
    Pixels {
        float: Option<NumberRule>,
    },
    /// Unitless number.
    Number(NumberRule),
    Color,
    Keyword(&'static KeywordEnum),
}

/// A patchable attribute.
#[derive(Debug, PartialEq)]
pub struct AttrSpec {
    /// Canonical kebab-case NVE/CSS name.
    pub name: &'static str,
    pub kind: AttrKind,
    /// `nve_components::css_key_bits` (or `style`) recognizes it: today's string
    /// `TemplatePatch` path applies it. Typed-only attributes wait for `RowPatch` lowering.
    pub css: bool,
}

impl AttrSpec {
    /// Value classes this attribute accepts.
    pub const fn accepts(&self) -> ValueClasses {
        let s = ValueClasses::STRING;
        match self.kind {
            AttrKind::Text => s,
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
                ty.as_deref().is_none_or(|t| t == kw.ty) && kw.css(variant).is_some()
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

/// Declares a keyword enum table and compile-checks that every variant exists on the real type.
macro_rules! keyword_enum {
    ($name:ident: $ty:ident = $($seg:ident)::+ { $($variant:ident => $css:expr),* $(,)? }) => {
        pub const $name: KeywordEnum = KeywordEnum {
            ty: stringify!($ty),
            variants: &[$((stringify!($variant), $css)),*],
        };
        const _: () = {
            use $($seg)::+ as Real;
            let _: &[Real] = &[$(Real::$variant),*];
        };
    };
}

keyword_enum!(DISPLAY: Display = nve_components::tables::Display {
    Flex => Some("flex"), None => Some("none"), Grid => Some("grid"), Block => Some("block"),
    Inline => Some("inline"), InlineBlock => Some("inline-block"),
    InlineFlex => Some("inline-flex"), Contents => Some("contents"),
});
keyword_enum!(FLEX_DIRECTION: FlexDirection = nve_components::tables::FlexDirection {
    Column => Some("column"), Row => Some("row"),
    ColumnReverse => Some("column-reverse"), RowReverse => Some("row-reverse"),
});
keyword_enum!(FLEX_WRAP: FlexWrap = nve_components::tables::FlexWrap {
    NoWrap => Some("nowrap"), Wrap => Some("wrap"), WrapReverse => Some("wrap-reverse"),
});
keyword_enum!(JUSTIFY_CONTENT: JustifyContent = nve_components::tables::JustifyContent {
    FlexStart => Some("flex-start"), Center => Some("center"), FlexEnd => Some("flex-end"),
    SpaceBetween => Some("space-between"), SpaceAround => Some("space-around"),
    SpaceEvenly => Some("space-evenly"),
});
keyword_enum!(ALIGN_ITEMS: AlignItems = nve_components::tables::AlignItems {
    Stretch => Some("stretch"), FlexStart => Some("flex-start"), Center => Some("center"),
    FlexEnd => Some("flex-end"), Baseline => Some("baseline"),
});
keyword_enum!(ALIGN_SELF: AlignSelf = nve_components::tables::AlignSelf {
    Auto => Some("auto"), Stretch => Some("stretch"), FlexStart => Some("flex-start"),
    Center => Some("center"), FlexEnd => Some("flex-end"), Baseline => Some("baseline"),
});
keyword_enum!(POSITION: Position = nve_components::tables::Position {
    Relative => Some("relative"), Absolute => Some("absolute"), Sticky => Some("sticky"),
    Fixed => Some("fixed"),
});
keyword_enum!(BOX_SIZING: BoxSizing = nve_components::tables::BoxSizing {
    BorderBox => Some("border-box"), ContentBox => Some("content-box"),
});
keyword_enum!(VISIBILITY: Visibility = nve_components::tables::Visibility {
    Inherit => Some("inherit"), Visible => Some("visible"), Hidden => Some("hidden"),
});
keyword_enum!(OVERFLOW: Overflow = nve_components::tables::Overflow {
    Visible => Some("visible"), Hidden => Some("hidden"), Scroll => Some("scroll"),
    Auto => Some("auto"),
});
keyword_enum!(OVERFLOW_WRAP: OverflowWrap = nve_components::tables::OverflowWrap {
    Normal => Some("normal"), Anywhere => Some("anywhere"), BreakWord => Some("break-word"),
});
keyword_enum!(WORD_BREAK: WordBreak = nve_components::tables::WordBreak {
    Normal => Some("normal"), BreakAll => Some("break-all"), KeepAll => Some("keep-all"),
    BreakWord => Some("break-word"),
});
// `css.rs` has no `break-spaces`.
keyword_enum!(WHITE_SPACE: WhiteSpace = nve_components::tables::WhiteSpace {
    Normal => Some("normal"), NoWrap => Some("nowrap"), Pre => Some("pre"),
    PreWrap => Some("pre-wrap"), PreLine => Some("pre-line"), BreakSpaces => None,
});
// `css.rs` maps the strings `start`/`end` to `Left`/`Right`, not to these variants.
keyword_enum!(TEXT_ALIGN: TextAlign = nve_components::tables::TextAlign {
    Start => None, End => None, Left => Some("left"), Right => Some("right"),
    Center => Some("center"), Justify => Some("justify"),
});
keyword_enum!(FONT_STYLE: FontStyle = nve_components::tables::FontStyle {
    Normal => Some("normal"), Italic => Some("italic"), Oblique => Some("oblique"),
});
keyword_enum!(VERTICAL_ALIGN: VerticalAlign = nve_components::tables::VerticalAlign {
    Baseline => Some("baseline"), Top => Some("top"), Middle => Some("middle"),
    Bottom => Some("bottom"), Sub => Some("sub"), Super => Some("super"),
    TextTop => Some("text-top"), TextBottom => Some("text-bottom"),
});
keyword_enum!(CURSOR: CursorKind = nve_components::CursorKind {
    Auto => Some("auto"), Default => Some("default"), Pointer => Some("pointer"),
    Text => Some("text"), Grab => Some("grab"), Grabbing => Some("grabbing"),
    NotAllowed => Some("not-allowed"), ColResize => Some("col-resize"),
    RowResize => Some("row-resize"), Move => Some("move"), Crosshair => Some("crosshair"),
    Progress => Some("progress"), Wait => Some("wait"),
});

const fn spec(name: &'static str, kind: AttrKind, css: bool) -> AttrSpec {
    AttrSpec { name, kind, css }
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
    ($($name:literal => $kind:expr, $css:literal;)*) => {
        fn lookup_canonical(name: &str) -> Option<&'static AttrSpec> {
            match name {
                $($name => Some(const { &spec($name, $kind, $css) }),)*
                _ => None,
            }
        }

        /// Every patchable attribute's canonical kebab-case name.
        #[cfg(test)]
        pub(super) const PATCHABLE_ATTR_NAMES: &[&str] = &[$($name),*];
    };
}

attr_table! {
    "style" => AttrKind::Text, true;
    // Box sizing.
    "width" => DIM, true; "height" => DIM, true;
    "min-width" => DIM, true; "min-height" => DIM, true;
    "max-width" => DIM, true; "max-height" => DIM, true;
    "box-sizing" => AttrKind::Keyword(&BOX_SIZING), false;
    "aspect-ratio" => AttrKind::Number(NumberRule::Unitless { min: 0.0, max: f32::INFINITY }), false;
    // Flex layout.
    "display" => AttrKind::Keyword(&DISPLAY), true;
    "position" => AttrKind::Keyword(&POSITION), false;
    "flex-direction" => AttrKind::Keyword(&FLEX_DIRECTION), true;
    "flex-wrap" => AttrKind::Keyword(&FLEX_WRAP), true;
    "justify-content" => AttrKind::Keyword(&JUSTIFY_CONTENT), true;
    "align-items" => AttrKind::Keyword(&ALIGN_ITEMS), true;
    "align-self" => AttrKind::Keyword(&ALIGN_SELF), false;
    "flex" => GROW, true; "flex-grow" => GROW, true; "flex-shrink" => GROW, true;
    "flex-basis" => AttrKind::Dimension { float: NON_NEGATIVE_PX }, true;
    "gap" => PX, true; "row-gap" => PX, true; "column-gap" => PX, true;
    "top" => INSET, false; "right" => INSET, false; "bottom" => INSET, false; "left" => INSET, false;
    // Spacing.
    "padding" => PX, true; "padding-horizontal" => PX, true; "padding-vertical" => PX, true;
    // Numeric `padding-{top,right,bottom,left}` aren't in `apply_numeric_attr` yet; string & `Length::points` lower cleanly.
    "padding-top" => AttrKind::Pixels { float: None }, true;
    "padding-right" => AttrKind::Pixels { float: None }, true;
    "padding-bottom" => AttrKind::Pixels { float: None }, true;
    "padding-left" => AttrKind::Pixels { float: None }, true;
    "margin" => SIGNED, true;
    "margin-top" => SIGNED, false; "margin-right" => SIGNED, false;
    "margin-bottom" => SIGNED, false; "margin-left" => SIGNED, false;
    // Visual.
    "background" => AttrKind::Color, true; "background-color" => AttrKind::Color, true;
    "color" => AttrKind::Color, true; "border-color" => AttrKind::Color, true;
    "border" => AttrKind::Text, false;
    "border-width" => PX, true;
    "border-top-width" => PX, false; "border-right-width" => PX, false;
    "border-bottom-width" => PX, false; "border-left-width" => PX, false;
    "border-radius" => PX, true;
    "box-shadow" => AttrKind::Text, true;
    "opacity" => AttrKind::Number(NumberRule::Unitless { min: 0.0, max: 1.0 }), true;
    "z-index" => AttrKind::Number(NumberRule::ZIndex), true;
    "visibility" => AttrKind::Keyword(&VISIBILITY), true;
    "overflow" => AttrKind::Keyword(&OVERFLOW), true;
    "overflow-x" => AttrKind::Keyword(&OVERFLOW), true;
    "overflow-y" => AttrKind::Keyword(&OVERFLOW), true;
    // Typography. `line-height` / `font-weight` numbers are not honoured by the typed path.
    "font-family" => AttrKind::Text, true;
    "font-size" => AttrKind::Pixels { float: Some(NumberRule::Px { min: 1.0 }) }, true;
    "font-weight" => AttrKind::Text, true;
    "font-style" => AttrKind::Keyword(&FONT_STYLE), true;
    "line-height" => AttrKind::Text, true;
    "text-align" => AttrKind::Keyword(&TEXT_ALIGN), true;
    "text-decoration" => AttrKind::Text, true;
    "white-space" => AttrKind::Keyword(&WHITE_SPACE), true;
    "overflow-wrap" => AttrKind::Keyword(&OVERFLOW_WRAP), true;
    "word-break" => AttrKind::Keyword(&WORD_BREAK), true;
    "vertical-align" => AttrKind::Keyword(&VERTICAL_ALIGN), false;
    // Interaction.
    "cursor" => AttrKind::Keyword(&CURSOR), false;
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
