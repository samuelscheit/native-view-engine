//! `Scroll`, `Image`, `TextInput`, and `Link` sparse tables.

use nve_store::{word_flags, NveTable, Res, Rgba, Str, WordEnum};

use crate::input_hints::{AutofillHint, InputMode};

use super::resources::ImageSrc;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum ScrollAxes {
    #[default]
    Vertical = 0,
    Horizontal = 1,
    Both = 2,
    None = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Scrollbars {
    #[default]
    Auto = 0,
    Always = 1,
    Never = 2,
}

/// Scroll container configuration and virtualization hints.
/// Presence replaces `ComponentKind::ScrollView`.
#[derive(NveTable)]
#[nve_table(
    name = "nve.scroll",
    storage = "sparse",
    flags = "publish",
    events = nve_abi::ScrollEvent
)]
pub struct Scroll {
    #[effect(realize, layout, composite)]
    pub axes: ScrollAxes,
    #[effect(paint)]
    pub scrollbars: Scrollbars,
    #[effect(composite)]
    pub paging: bool,
    #[effect(composite)]
    pub bounce: bool,
    /// Optional virtualized top spacer offset in px (`None` = none).
    #[effect(layout, composite)]
    pub virtual_top_spacer: Option<f32>,
    /// Optional virtualized total content width in px (`None` = derived).
    #[effect(layout, composite)]
    pub virtual_total_width: Option<f32>,
    /// Optional virtualized total content height in px (`None` = derived).
    #[effect(layout, composite)]
    pub virtual_total_height: Option<f32>,
    /// Optional programmatic scroll-Y target (`None` = unchanged).
    #[effect(composite)]
    pub requested_scroll_y: Option<f32>,
    /// Optional programmatic scroll-X target (`None` = unchanged).
    #[effect(composite)]
    pub requested_scroll_x: Option<f32>,
    /// Monotonic sequence number for programmatic scroll sync (`0` = no request; incrementing
    /// applies `(requested_scroll_x, requested_scroll_y)` exactly once, including repeat jumps).
    #[effect(composite)]
    pub requested_scroll_seq: u32,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum ImageFit {
    #[default]
    Cover = 0,
    Contain = 1,
    Fill = 2,
    None = 3,
    ScaleDown = 4,
}

impl ImageFit {

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
    pub fit: ImageFit,
    /// Optional tint color (`None` = no tint).
    #[effect(paint)]
    pub tint: Option<Rgba>,
    /// Optional intrinsic width hint in px (`None` = unknown).
    #[effect(measure)]
    pub intrinsic_width: Option<u16>,
    /// Optional intrinsic height hint in px (`None` = unknown).
    #[effect(measure)]
    pub intrinsic_height: Option<u16>,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum InputKind {
    #[default]
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

impl InputKind {

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

/// Text input field. Presence replaces `ComponentKind::TextInput`.
#[derive(NveTable)]
#[nve_table(
    name = "nve.text_input",
    storage = "sparse",
    flags = "publish",
    events = nve_abi::TextInputEvent
)]
pub struct TextInput {
    #[effect(paint, semantics)]
    pub value: Str,
    #[effect(paint, semantics)]
    pub placeholder: Str,
    #[effect(semantics)]
    pub pattern: Str,
    #[effect(realize, measure, semantics)]
    pub multiline: bool,
    #[effect(realize, paint, semantics)]
    pub secure: bool,
    #[effect(semantics)]
    pub read_only: bool,
    #[effect(semantics)]
    pub required: bool,
    #[effect(layout, paint, semantics)]
    pub input_kind: InputKind,
    #[effect(semantics)]
    pub input_mode: InputMode,
    #[effect(semantics)]
    pub autofill: Option<AutofillHint>,
    /// Optional minimum character length (`None` = unconstrained).
    #[effect(semantics)]
    pub min_length: Option<u16>,
    /// Optional maximum character length (`None` = unconstrained).
    #[effect(semantics)]
    pub max_length: Option<u16>,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum LinkTarget {
    #[default]
    Default = 0,
    Blank = 1,
    Self_ = 2,
    Parent = 3,
}

word_flags! {
    /// Hyperlink `rel` relationship flags packed into 3 bits.
    pub struct LinkRel(u8) {
        const NOOPENER = 1 << 0;
        const NOREFERRER = 1 << 1;
        const NOFOLLOW = 1 << 2;
    }
}

/// Cross-platform hyperlink target (`<a>` on web, URL opener / link semantics on native).
#[derive(NveTable)]
#[nve_table(name = "nve.link", storage = "sparse", flags = "publish")]
pub struct Link {
    #[effect(realize, semantics)]
    pub href: Str,
    #[effect(semantics)]
    pub target: LinkTarget,
    #[effect(semantics)]
    pub rel: LinkRel,
}
