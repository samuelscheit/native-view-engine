//! Registry of Tier-1 patchable native-element style attributes and the value classes each accepts.
//!
//! Identity (`test_id`, `key`), event handlers (`on*`), content props and every unknown attribute
//! are deliberately absent: an edit to them is a logic/structure change that needs a Tier-2 build.

use super::eval::TypedAttrValue;

/// Bitset of [`TypedAttrValue`] classes an attribute accepts. Static strings (raw CSS values) are
/// accepted by every attribute until Track 2 deletes author style strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueClasses(u8);

impl ValueClasses {
    /// Static strings only (`style`, `font-family`, the `border` shorthand).
    pub const STRING: Self = Self(0);
    /// `px(..)`, `pct(..)`, `Length::Auto`; bare numbers lower to `px`.
    pub const LENGTH: Self = Self(1);
    /// Unitless numbers (`opacity`, `flex-grow`, `z-index`, `font-weight`).
    pub const NUMBER: Self = Self(1 << 1);
    /// `rgb(..)`, `hex(0x..)`, `ColorRgba::*`.
    pub const COLOR: Self = Self(1 << 2);
    /// `Enum::Variant` keywords (`FlexDirection::Column`).
    pub const KEYWORD: Self = Self(1 << 3);

    pub const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

/// A patchable attribute: canonical kebab-case NVE/CSS name plus accepted value classes.
#[derive(Debug, PartialEq, Eq)]
pub struct AttrSpec {
    pub name: &'static str,
    pub accepts: ValueClasses,
}

impl AttrSpec {
    /// Whether `value` is a well-typed value for this attribute.
    pub fn accepts(&self, value: &TypedAttrValue) -> bool {
        match value {
            TypedAttrValue::String(_) => true,
            other => self.accepts.intersects(other.classes()),
        }
    }
}

const LEN: ValueClasses = ValueClasses::LENGTH;
const NUM: ValueClasses = ValueClasses::NUMBER;
const COLOR: ValueClasses = ValueClasses::COLOR;
const KW: ValueClasses = ValueClasses::KEYWORD;
const STR: ValueClasses = ValueClasses::STRING;

macro_rules! attr_table {
    ($($name:literal => $accepts:expr),* $(,)?) => {
        fn lookup_canonical(name: &str) -> Option<&'static AttrSpec> {
            match name {
                $($name => Some(const { &AttrSpec { name: $name, accepts: $accepts } }),)*
                _ => None,
            }
        }

        /// Every patchable attribute's canonical kebab-case name.
        #[cfg(test)]
        pub(super) const PATCHABLE_ATTR_NAMES: &[&str] = &[$($name),*];
    };
}

attr_table! {
    "style" => STR,
    // Box sizing.
    "width" => LEN, "height" => LEN,
    "min-width" => LEN, "min-height" => LEN, "max-width" => LEN, "max-height" => LEN,
    "aspect-ratio" => NUM, "box-sizing" => KW,
    // Flex layout.
    "display" => KW, "position" => KW,
    "flex-direction" => KW, "flex-wrap" => KW, "justify-content" => KW,
    "align-items" => KW, "align-self" => KW,
    "flex" => NUM, "flex-grow" => NUM, "flex-shrink" => NUM, "flex-basis" => LEN,
    "gap" => LEN, "row-gap" => LEN, "column-gap" => LEN,
    "top" => LEN, "right" => LEN, "bottom" => LEN, "left" => LEN,
    // Spacing.
    "padding" => LEN, "padding-horizontal" => LEN, "padding-vertical" => LEN,
    "padding-top" => LEN, "padding-right" => LEN, "padding-bottom" => LEN, "padding-left" => LEN,
    "margin" => LEN,
    "margin-top" => LEN, "margin-right" => LEN, "margin-bottom" => LEN, "margin-left" => LEN,
    // Visual.
    "background" => COLOR, "background-color" => COLOR, "color" => COLOR,
    "border" => STR, "border-color" => COLOR, "border-width" => LEN,
    "border-top-width" => LEN, "border-right-width" => LEN,
    "border-bottom-width" => LEN, "border-left-width" => LEN,
    "border-radius" => LEN, "opacity" => NUM, "z-index" => NUM,
    "overflow" => KW, "overflow-x" => KW, "overflow-y" => KW, "visibility" => KW,
    // Typography.
    "font-family" => STR, "font-size" => LEN, "font-weight" => NUM.or(KW), "font-style" => KW,
    "line-height" => NUM.or(LEN), "text-align" => KW, "text-decoration" => KW,
    "white-space" => KW, "overflow-wrap" => KW, "vertical-align" => KW,
    // Interaction.
    "cursor" => KW,
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
    let canonical = std::str::from_utf8(&buf[..bytes.len()]).ok()?;
    lookup_canonical(canonical)
}

/// Static element identity attributes (`test_id` and its aliases).
pub fn is_test_id_attr(name: &str) -> bool {
    matches!(name, "test_id" | "testID" | "data-testid" | "test-id")
}
