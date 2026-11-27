//! `Scroll`, `Image`, and `TextInput` sparse tables.

use nve_store::{word_enum, FieldKind, NveTable, Res, Rgba, Str, WordValue};

use crate::input_hints::{AutofillHint, InputMode};

use super::resources::ImageSrc;

word_enum! {
    pub enum ScrollAxes: 2 { Vertical = 0, Horizontal = 1, Both = 2, None = 3 }
}

word_enum! {
    pub enum Scrollbars: 2 { Auto = 0, Always = 1, Never = 2 }
}

/// Scroll container configuration and virtualization hints.
/// Presence replaces `ComponentKind::ScrollView`.
#[derive(NveTable)]
#[nve_table(name = "nve.scroll", storage = "sparse", flags = "publish")]
pub struct Scroll {
    #[effect(realize, layout, composite)]
    #[bits(2)]
    pub axes: ScrollAxes,
    #[effect(paint)]
    #[bits(2)]
    pub scrollbars: Scrollbars,
    #[effect(composite)]
    #[bits(1)]
    pub paging: bool,
    #[effect(composite)]
    #[bits(1)]
    pub bounce: bool,
    /// Virtualized top spacer offset in px (`NaN` = none).
    #[effect(layout, composite)]
    #[default(f32::NAN)]
    pub virtual_top_spacer: f32,
    /// Virtualized total content width in px (`NaN` = derived).
    #[effect(layout, composite)]
    #[default(f32::NAN)]
    pub virtual_total_width: f32,
    /// Virtualized total content height in px (`NaN` = derived).
    #[effect(layout, composite)]
    #[default(f32::NAN)]
    pub virtual_total_height: f32,
    /// Programmatic scroll-Y target (`NaN` = unchanged).
    #[effect(composite)]
    #[default(f32::NAN)]
    pub requested_scroll_y: f32,
    /// Programmatic scroll-X target (`NaN` = unchanged).
    #[effect(composite)]
    #[default(f32::NAN)]
    pub requested_scroll_x: f32,
}

word_enum! {
    pub enum ImageFit: 3 { Cover = 0, Contain = 1, Fill = 2, None = 3, ScaleDown = 4 }
}

impl ImageFit {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
            Self::Fill => "fill",
            Self::None => "none",
            Self::ScaleDown => "scale-down",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("cover") {
            Some(Self::Cover)
        } else if s.eq_ignore_ascii_case("contain") {
            Some(Self::Contain)
        } else if s.eq_ignore_ascii_case("fill") || s.eq_ignore_ascii_case("stretch") {
            Some(Self::Fill)
        } else if s.eq_ignore_ascii_case("none") || s.eq_ignore_ascii_case("center") {
            Some(Self::None)
        } else if s.eq_ignore_ascii_case("scale-down") || s.eq_ignore_ascii_case("scaledown") {
            Some(Self::ScaleDown)
        } else {
            None
        }
    }
}

/// Image node table. Presence replaces `ComponentKind::Image`.
#[derive(NveTable)]
#[nve_table(name = "nve.image", storage = "sparse", flags = "publish")]
pub struct Image {
    #[effect(realize, paint, measure)]
    pub src: Res<ImageSrc>,
    #[effect(semantics)]
    pub alt: Str,
    #[effect(paint)]
    #[bits(3)]
    pub fit: ImageFit,
    #[effect(paint)]
    #[bits(1)]
    pub tint_enabled: bool,
    #[effect(paint)]
    pub tint: Rgba,
    /// Optional intrinsic width hint in px (`0` = unknown).
    #[effect(measure)]
    #[bits(16)]
    pub intrinsic_width: u16,
    /// Optional intrinsic height hint in px (`0` = unknown).
    #[effect(measure)]
    #[bits(16)]
    pub intrinsic_height: u16,
}

word_enum! {
    pub enum InputKind: 4 {
        Text = 0,
        Email = 1,
        Password = 2,
        Tel = 3,
        Url = 4,
        Number = 5,
        Search = 6,
        Date = 7,
        Time = 8,
        Hidden = 9,
    }
}

impl InputKind {
    #[inline]
    pub const fn as_html_type(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Email => "email",
            Self::Password => "password",
            Self::Tel => "tel",
            Self::Url => "url",
            Self::Number => "number",
            Self::Search => "search",
            Self::Date => "date",
            Self::Time => "time",
            Self::Hidden => "hidden",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("text") {
            Some(Self::Text)
        } else if s.eq_ignore_ascii_case("email") {
            Some(Self::Email)
        } else if s.eq_ignore_ascii_case("password") {
            Some(Self::Password)
        } else if s.eq_ignore_ascii_case("tel") {
            Some(Self::Tel)
        } else if s.eq_ignore_ascii_case("url") {
            Some(Self::Url)
        } else if s.eq_ignore_ascii_case("number") {
            Some(Self::Number)
        } else if s.eq_ignore_ascii_case("search") {
            Some(Self::Search)
        } else if s.eq_ignore_ascii_case("date") {
            Some(Self::Date)
        } else if s.eq_ignore_ascii_case("time") {
            Some(Self::Time)
        } else if s.eq_ignore_ascii_case("hidden") {
            Some(Self::Hidden)
        } else {
            None
        }
    }
}

/// Optional `AutofillHint` packed into 5 bits (`0` = unset, `1..=16` = `AutofillHint`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OptAutofillHint(pub u8);

impl OptAutofillHint {
    pub const NONE: Self = Self(0);

    #[inline]
    pub const fn some(hint: AutofillHint) -> Self {
        Self((hint as u8) + 1)
    }

    #[inline]
    pub const fn get(self) -> Option<AutofillHint> {
        if self.0 == 0 || (self.0 as usize) > AutofillHint::ALL.len() {
            None
        } else {
            Some(AutofillHint::ALL[(self.0 - 1) as usize])
        }
    }
}

impl WordValue for OptAutofillHint {
    const BITS: u8 = 5;
    const KIND: FieldKind = FieldKind::Enum;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self.0 as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        let v = (bits & 0x1F) as u8;
        if (v as usize) <= AutofillHint::ALL.len() {
            Self(v)
        } else {
            Self::NONE
        }
    }
}

impl WordValue for InputMode {
    const BITS: u8 = 3;
    const KIND: FieldKind = FieldKind::Enum;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        InputMode::ALL
            .get(bits as usize)
            .copied()
            .unwrap_or(Self::Text)
    }
}

/// Text input field. Presence replaces `ComponentKind::TextInput`.
#[derive(NveTable)]
#[nve_table(name = "nve.text_input", storage = "sparse", flags = "publish")]
pub struct TextInput {
    #[effect(paint, semantics)]
    pub value: Str,
    #[effect(paint, semantics)]
    pub placeholder: Str,
    #[effect(semantics)]
    pub pattern: Str,
    #[effect(realize, measure, semantics)]
    #[bits(1)]
    pub multiline: bool,
    #[effect(realize, paint, semantics)]
    #[bits(1)]
    pub secure: bool,
    #[effect(semantics)]
    #[bits(1)]
    pub read_only: bool,
    #[effect(semantics)]
    #[bits(1)]
    pub required: bool,
    #[effect(layout, paint, semantics)]
    #[bits(4)]
    pub input_kind: InputKind,
    #[effect(semantics)]
    #[bits(3)]
    pub input_mode: InputMode,
    #[effect(semantics)]
    #[bits(5)]
    pub autofill: OptAutofillHint,
    /// `0xFFFF` = unset.
    #[effect(semantics)]
    #[bits(16)]
    #[default(u16::MAX)]
    pub min_length: u16,
    /// `0xFFFF` = unset.
    #[effect(semantics)]
    #[bits(16)]
    #[default(u16::MAX)]
    pub max_length: u16,
}

word_enum! {
    pub enum LinkTarget: 2 { Default = 0, Blank = 1, Self_ = 2, Parent = 3 }
}

/// Cross-platform hyperlink target (`<a>` on web, URL opener / link semantics on native).
#[derive(NveTable)]
#[nve_table(name = "nve.link", storage = "sparse", flags = "publish")]
pub struct Link {
    #[effect(realize, semantics)]
    pub href: Str,
    #[effect(semantics)]
    #[bits(2)]
    pub target: LinkTarget,
    /// Bit 0: `noopener`, bit 1: `noreferrer`, bit 2: `nofollow`.
    #[effect(semantics)]
    #[bits(8)]
    pub rel: u8,
}
