//! `Visual` (dense), `Typography` / `TextContent` (sparse), `Transform`, `Responsive`.

use nve_store::{word_flags, Length, NveTable, Res, Rgba, Str, WordEnum};

use super::resources::{BackgroundFillRes, BoxShadows, ResponsiveRules};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Overflow {
    #[default]
    Visible = 0,
    Hidden = 1,
    Scroll = 2,
    Auto = 3,
}

/// CSS-accurate inherited visibility (`Inherit` follows parent; explicit `Visible` overrides a
/// `Hidden` ancestor).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Visibility {
    #[default]
    Inherit = 0,
    Visible = 1,
    Hidden = 2,
}

impl Visibility {
    /// Resolves whether a node is effectively hidden under CSS visibility inheritance rules.
    #[inline(always)]
    pub const fn resolve(self, inherited_hidden: bool) -> bool {
        match self {
            Self::Hidden => true,
            Self::Visible => false,
            Self::Inherit => inherited_hidden,
        }
    }
}

/// Paint + compositor inputs every node can have. Dense: read by every paint pass.
#[derive(NveTable)]
#[nve_table(name = "nve.visual", storage = "dense", flags = "publish")]
pub struct Visual {
    #[effect(paint)]
    pub background: Rgba,
    #[effect(paint)]
    pub border_color: Rgba,
    /// Border thickness also insets content (border-box layout).
    #[effect(layout, paint)]
    pub border_width: f32,
    #[effect(paint)]
    pub border_radius: f32,
    #[effect(composite)]
    #[default(1.0)]
    pub opacity: f32,
    #[effect(composite)]
    pub z_index: i32,
    /// Overflow affects scroll-container sizing (layout) and clipping (paint).
    #[effect(layout, paint)]
    pub overflow_x: Overflow,
    #[effect(layout, paint)]
    pub overflow_y: Overflow,
    #[effect(paint)]
    pub visibility: Visibility,
    #[effect(paint)]
    pub box_shadows: Res<BoxShadows>,
    /// Gradient fills (solid fills use `background`).
    #[effect(paint)]
    pub background_fill: Res<BackgroundFillRes>,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum FontStyle {
    #[default]
    Normal = 0,
    Italic = 1,
    Oblique = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum TextAlign {
    #[default]
    Start = 0,
    End = 1,
    Left = 2,
    Right = 3,
    Center = 4,
    Justify = 5,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum WhiteSpace {
    #[default]
    Normal = 0,
    NoWrap = 1,
    Pre = 2,
    PreWrap = 3,
    PreLine = 4,
    BreakSpaces = 5,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum WordBreak {
    #[default]
    Normal = 0,
    BreakAll = 1,
    KeepAll = 2,
    BreakWord = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum OverflowWrap {
    #[default]
    Normal = 0,
    Anywhere = 1,
    BreakWord = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum VerticalAlign {
    #[default]
    Baseline = 0,
    Top = 1,
    Middle = 2,
    Bottom = 3,
    Sub = 4,
    Super = 5,
    TextTop = 6,
    TextBottom = 7,
}

word_flags! {
    /// `text-decoration-line` bitflags packed into 3 bits.
    pub struct TextDecoration(u8) {
        const UNDERLINE = 1 << 0;
        const OVERLINE = 1 << 1;
        const LINE_THROUGH = 1 << 2;
    }
}

/// Inherited text styling. Hidden word 0 tracks per-field explicit overrides via
/// `#[nve_table(inherited)]`; unset fields inherit branchlessly through `ResolvedRow<Typography>`.
#[derive(NveTable)]
#[nve_table(name = "nve.typography", storage = "sparse", flags = "publish", inherited)]
pub struct Typography {
    #[effect(paint)]
    #[default(Rgba::BLACK)]
    pub color: Rgba,
    #[effect(measure)]
    #[default(16.0)]
    pub font_size: f32,
    /// Points = px; percent = unit-less multiplier × 100; `auto` = `normal`.
    #[effect(measure)]
    #[default(Length::AUTO)]
    pub line_height: Length,
    #[effect(measure)]
    pub letter_spacing: f32,
    #[effect(measure)]
    pub font_family: Str,
    /// Optional positive line clamp (`None` = unlimited; `0` is unrepresentable by type).
    /// Non-inherited in CSS (`-webkit-line-clamp` applies only to the declaring container).
    #[effect(measure)]
    #[no_inherit]
    pub max_lines: Option<core::num::NonZeroU16>,
    #[effect(measure)]
    #[default(400u16)]
    pub font_weight: u16,
    #[effect(measure)]
    pub font_style: FontStyle,
    #[effect(measure)]
    pub white_space: WhiteSpace,
    #[effect(measure)]
    pub word_break: WordBreak,
    #[effect(measure)]
    pub overflow_wrap: OverflowWrap,
    #[effect(paint)]
    pub text_align: TextAlign,
    #[effect(paint)]
    pub text_decoration: TextDecoration,
    /// Non-inherited in CSS (`vertical-align` applies only to the declaring element).
    #[effect(paint)]
    #[no_inherit]
    pub vertical_align: VerticalAlign,
}

/// Text run content. Presence makes a node a text leaf (DOM inline text, `UILabel`, measured
/// leaf in layout).
#[derive(NveTable)]
#[nve_table(name = "nve.text", storage = "sparse", flags = "publish")]
pub struct TextContent {
    #[effect(measure, paint, semantics)]
    pub text: Str,
}

/// 2D affine transform (compositor-only).
#[derive(NveTable)]
#[nve_table(name = "nve.transform", storage = "sparse", flags = "publish")]
pub struct Transform {
    #[effect(composite)]
    pub translate_x: f32,
    #[effect(composite)]
    pub translate_y: f32,
    #[effect(composite)]
    #[default(1.0)]
    pub scale_x: f32,
    #[effect(composite)]
    #[default(1.0)]
    pub scale_y: f32,
    #[effect(composite)]
    pub rotate_deg: f32,
    /// Transform origin as a fraction of the box (`0.5` = center).
    #[effect(composite)]
    #[default(0.5)]
    pub origin_x: f32,
    #[effect(composite)]
    #[default(0.5)]
    pub origin_y: f32,
}

/// `@media` visibility gate and responsive rule patches.
#[derive(NveTable)]
#[nve_table(name = "nve.responsive", storage = "sparse", flags = "publish")]
pub struct Responsive {
    /// `MediaQuery` bytes 0..4 (`min_width_px | max_width_px << 16`); `0` = unconstrained.
    #[effect(layout, paint)]
    pub show_on_widths: u32,
    /// `MediaQuery` bytes 4..8 (`flags | color_scheme << 8 | pointer << 16`).
    #[effect(layout, paint)]
    pub show_on_env: u32,
    #[effect(layout, paint)]
    pub rules: Res<ResponsiveRules>,
}

impl Responsive {
    /// Packs a [`crate::media::MediaQuery`] into `(show_on_widths, show_on_env)` words.
    #[inline]
    pub const fn pack_media_query(q: crate::media::MediaQuery) -> (u32, u32) {
        let widths = (q.min_width_px as u32) | ((q.max_width_px as u32) << 16);
        let env = (q.flags as u32)
            | ((q.color_scheme as u32) << 8)
            | ((q.pointer as u32) << 16);
        (widths, env)
    }

    /// Unpacks `(show_on_widths, show_on_env)` back into a [`crate::media::MediaQuery`], returning
    /// `None` when no constraints are active.
    #[inline]
    pub const fn unpack_media_query(widths: u32, env: u32) -> Option<crate::media::MediaQuery> {
        let q = crate::media::MediaQuery {
            min_width_px: (widths & 0xFFFF) as u16,
            max_width_px: ((widths >> 16) & 0xFFFF) as u16,
            flags: (env & 0xFF) as u8,
            color_scheme: ((env >> 8) & 0xFF) as u8,
            pointer: ((env >> 16) & 0xFF) as u8,
            _pad: 0,
        };
        if q.flags == 0 && q.color_scheme == 0 && q.pointer == 0 {
            None
        } else {
            Some(q)
        }
    }
}

impl ResponsiveRef<'_> {
    /// Decodes the packed `show_on` visibility [`crate::media::MediaQuery`], if any.
    #[inline]
    pub fn show_on(&self) -> Option<crate::media::MediaQuery> {
        Responsive::unpack_media_query(self.show_on_widths(), self.show_on_env())
    }
}
