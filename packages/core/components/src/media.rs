//! Breakpoint-general, strictly typed responsive `@media` queries, sparse style patches,
//! native viewport evaluation, and deterministic CSS `@media` emission.
//!
//! Zero serialization and zero arbitrary strings: [`MediaQuery`] is a compact 8-byte `#[repr(C)]`
//! `Copy` struct directly shareable in memory across Rust, FFI, WASM, and native backends with
//! 0% CSS injection surface.

use std::fmt::Write as _;

use nve_abi::{
    ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify, LayoutProps,
    StyleProps,
};

/// Bitflag in [`MediaQuery::flags`] indicating [`MediaQuery::min_width_px`] is active.
pub const MEDIA_FLAG_HAS_MIN_WIDTH: u8 = 1 << 0;

/// Bitflag in [`MediaQuery::flags`] indicating [`MediaQuery::max_width_px`] is active.
pub const MEDIA_FLAG_HAS_MAX_WIDTH: u8 = 1 << 1;

/// Bitflag in [`MediaQuery::flags`] indicating `reduced_motion` constraint is active.
pub const MEDIA_FLAG_HAS_REDUCED_MOTION: u8 = 1 << 2;

/// Bitflag in [`MediaQuery::flags`] carrying the boolean `reduced_motion` predicate value.
pub const MEDIA_FLAG_REDUCED_MOTION_VAL: u8 = 1 << 3;

/// Preferred color scheme predicate.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaColorScheme {
    Dark = 1,
    Light = 2,
}

impl MediaColorScheme {
    #[inline]
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw { 1 => Some(Self::Dark), 2 => Some(Self::Light), _ => None }
    }
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self { Self::Dark => "dark", Self::Light => "light" }
    }
    #[inline]
    pub const fn negated_css(self) -> &'static str {
        match self { Self::Dark => "light", Self::Light => "dark" }
    }
}

/// Primary pointer precision predicate.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaPointer {
    Fine = 1,
    Coarse = 2,
}

impl MediaPointer {
    #[inline]
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw { 1 => Some(Self::Fine), 2 => Some(Self::Coarse), _ => None }
    }
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self { Self::Fine => "fine", Self::Coarse => "coarse" }
    }
    #[inline]
    pub const fn negated_css(self) -> &'static str {
        match self { Self::Fine => "coarse", Self::Coarse => "fine" }
    }
}

/// Strictly typed `flex-wrap` override for [`ResponsiveStylePatch`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexWrap {
    NoWrap = 0,
    Wrap = 1,
    WrapReverse = 2,
}

impl FlexWrap {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self { Self::NoWrap => "nowrap", Self::Wrap => "wrap", Self::WrapReverse => "wrap-reverse" }
    }
}

/// Strictly typed `overflow` override for [`ResponsiveStylePatch`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Overflow {
    Visible = 0,
    Hidden = 1,
    Scroll = 2,
    Auto = 3,
}

impl Overflow {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Scroll => "scroll",
            Self::Auto => "auto",
        }
    }
}

/// Strictly typed `text-align` override for [`ResponsiveStylePatch`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Left = 0,
    Center = 1,
    Right = 2,
    Justify = 3,
}

impl TextAlign {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "justify",
        }
    }
}

/// Live viewport & user-preference snapshot evaluated natively by [`MediaQuery::matches`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaEnv {
    pub width_pt: f32,
    pub height_pt: f32,
    pub dark_mode: bool,
    pub reduced_motion: bool,
    pub coarse_pointer: bool,
}

impl Default for MediaEnv {
    fn default() -> Self {
        Self {
            width_pt: 1280.0,
            height_pt: 800.0,
            dark_mode: false,
            reduced_motion: false,
            coarse_pointer: false,
        }
    }
}

/// Compact 8-byte `#[repr(C)]` `Copy` responsive media query predicate directly shareable in
/// memory across Rust, FFI, WASM, and native slabs with zero serialization or parsing.
///
/// Width bounds use inclusive integer CSS-pixel endpoints (`min_width_px..=max_width_px`) mapped
/// to half-open `< (max_width_px + 1.0)` native evaluation and `max-width:{max}.98px` CSS
/// emission so adjacent breakpoints (`max_width(959)` / `min_width(960)`) never overlap or gap.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MediaQuery {
    pub min_width_px: u16,
    pub max_width_px: u16,
    pub flags: u8,
    /// `0` = unconstrained, `1` = [`MediaColorScheme::Dark`], `2` = [`MediaColorScheme::Light`].
    pub color_scheme: u8,
    /// `0` = unconstrained, `1` = [`MediaPointer::Fine`], `2` = [`MediaPointer::Coarse`].
    pub pointer: u8,
    pub _pad: u8,
}

const _: () = {
    assert!(std::mem::size_of::<MediaQuery>() == 8);
    assert!(std::mem::align_of::<MediaQuery>() == 2);
};

impl MediaQuery {
    pub const EMPTY: Self = Self {
        min_width_px: 0,
        max_width_px: 0,
        flags: 0,
        color_scheme: 0,
        pointer: 0,
        _pad: 0,
    };

    #[inline]
    pub const fn min_width(px: u16) -> Self {
        Self {
            min_width_px: px,
            flags: MEDIA_FLAG_HAS_MIN_WIDTH,
            ..Self::EMPTY
        }
    }

    #[inline]
    pub const fn max_width(px: u16) -> Self {
        Self {
            max_width_px: px,
            flags: MEDIA_FLAG_HAS_MAX_WIDTH,
            ..Self::EMPTY
        }
    }

    #[inline]
    pub const fn width_range(min_px: u16, max_px: u16) -> Self {
        Self {
            min_width_px: min_px,
            max_width_px: max_px,
            flags: MEDIA_FLAG_HAS_MIN_WIDTH | MEDIA_FLAG_HAS_MAX_WIDTH,
            ..Self::EMPTY
        }
    }

    /// Compact width class (`< 600pt`, aligned with `nve_interop::traits::SizeClass::Compact`).
    #[inline] pub const fn compact() -> Self { Self::max_width(599) }
    /// Medium width class (`600..1024pt`, aligned with `SizeClass::Medium`).
    #[inline] pub const fn medium() -> Self { Self::width_range(600, 1023) }
    /// Expanded width class (`>= 1024pt`, aligned with `SizeClass::Expanded`).
    #[inline] pub const fn expanded() -> Self { Self::min_width(1024) }
    /// Non-compact width class (`>= 600pt`: Medium or Expanded).
    #[inline] pub const fn not_compact() -> Self { Self::min_width(600) }

    #[inline]
    pub const fn dark() -> Self {
        Self {
            color_scheme: MediaColorScheme::Dark as u8,
            ..Self::EMPTY
        }
    }

    #[inline]
    pub const fn light() -> Self {
        Self {
            color_scheme: MediaColorScheme::Light as u8,
            ..Self::EMPTY
        }
    }

    #[inline]
    pub const fn reduced_motion() -> Self {
        Self {
            flags: MEDIA_FLAG_HAS_REDUCED_MOTION | MEDIA_FLAG_REDUCED_MOTION_VAL,
            ..Self::EMPTY
        }
    }

    #[inline]
    pub const fn coarse_pointer() -> Self {
        Self {
            pointer: MediaPointer::Coarse as u8,
            ..Self::EMPTY
        }
    }

    #[inline]
    pub const fn and_min_width(mut self, px: u16) -> Self {
        self.min_width_px = px;
        self.flags |= MEDIA_FLAG_HAS_MIN_WIDTH;
        self
    }

    #[inline]
    pub const fn and_max_width(mut self, px: u16) -> Self {
        self.max_width_px = px;
        self.flags |= MEDIA_FLAG_HAS_MAX_WIDTH;
        self
    }

    #[inline]
    pub const fn and_color_scheme(mut self, scheme: MediaColorScheme) -> Self {
        self.color_scheme = scheme as u8;
        self
    }

    #[inline]
    pub const fn and_reduced_motion(mut self, reduced: bool) -> Self {
        self.flags |= MEDIA_FLAG_HAS_REDUCED_MOTION;
        if reduced {
            self.flags |= MEDIA_FLAG_REDUCED_MOTION_VAL;
        } else {
            self.flags &= !MEDIA_FLAG_REDUCED_MOTION_VAL;
        }
        self
    }

    #[inline]
    pub const fn and_pointer(mut self, pointer: MediaPointer) -> Self {
        self.pointer = pointer as u8;
        self
    }

    #[inline]
    pub const fn min_width_opt(&self) -> Option<u16> {
        if (self.flags & MEDIA_FLAG_HAS_MIN_WIDTH) != 0 {
            Some(self.min_width_px)
        } else {
            None
        }
    }

    #[inline]
    pub const fn max_width_opt(&self) -> Option<u16> {
        if (self.flags & MEDIA_FLAG_HAS_MAX_WIDTH) != 0 {
            Some(self.max_width_px)
        } else {
            None
        }
    }

    #[inline]
    pub const fn color_scheme_opt(&self) -> Option<MediaColorScheme> {
        MediaColorScheme::from_u8(self.color_scheme)
    }

    #[inline]
    pub const fn reduced_motion_opt(&self) -> Option<bool> {
        if (self.flags & MEDIA_FLAG_HAS_REDUCED_MOTION) != 0 {
            Some((self.flags & MEDIA_FLAG_REDUCED_MOTION_VAL) != 0)
        } else {
            None
        }
    }

    #[inline]
    pub const fn pointer_opt(&self) -> Option<MediaPointer> {
        MediaPointer::from_u8(self.pointer)
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        (self.flags
            & (MEDIA_FLAG_HAS_MIN_WIDTH | MEDIA_FLAG_HAS_MAX_WIDTH | MEDIA_FLAG_HAS_REDUCED_MOTION))
            == 0
            && self.color_scheme == 0
            && self.pointer == 0
    }

    /// Returns `false` for impossible width ranges (`min_width_px > max_width_px`) or invalid
    /// raw discriminant bytes.
    #[inline]
    pub const fn is_valid(&self) -> bool {
        let valid_scheme = self.color_scheme <= 2;
        let valid_ptr = self.pointer <= 2;
        let valid_range = match (self.min_width_opt(), self.max_width_opt()) {
            (Some(min), Some(max)) => min <= max,
            _ => true,
        };
        valid_scheme && valid_ptr && valid_range
    }

    /// Evaluates this query natively against `env` using `< (max_width_px as f32 + 1.0)`
    /// half-open semantics matching CSS `max-width: {max}.98px`.
    pub fn matches(&self, env: &MediaEnv) -> bool {
        if !self.is_valid() {
            return false;
        }
        let min_opt = self.min_width_opt();
        let max_opt = self.max_width_opt();
        if min_opt.is_some() || max_opt.is_some() {
            if !env.width_pt.is_finite() {
                return false;
            }
            if let Some(min) = min_opt
                && env.width_pt < f32::from(min)
            {
                return false;
            }
            if let Some(max) = max_opt
                && env.width_pt >= f32::from(max) + 1.0
            {
                return false;
            }
        }
        if let Some(scheme) = self.color_scheme_opt()
            && env.dark_mode != matches!(scheme, MediaColorScheme::Dark)
        {
            return false;
        }
        if let Some(want_reduced) = self.reduced_motion_opt()
            && env.reduced_motion != want_reduced
        {
            return false;
        }
        if let Some(ptr) = self.pointer_opt()
            && env.coarse_pointer != matches!(ptr, MediaPointer::Coarse)
        {
            return false;
        }
        true
    }

    /// Writes a deterministic, injection-free CSS `@media` condition into `out`.
    pub fn write_css_condition(&self, out: &mut String) -> bool {
        if !self.is_valid() || self.is_empty() {
            return false;
        }
        let mut first = true;
        let mut push_sep = |buf: &mut String| {
            if !first {
                buf.push_str(" and ");
            }
            first = false;
        };

        if let Some(min) = self.min_width_opt() {
            push_sep(out);
            let _ = write!(out, "(min-width:{min}px)");
        }
        if let Some(max) = self.max_width_opt() {
            push_sep(out);
            let _ = write!(out, "(max-width:{max}.98px)");
        }
        if let Some(scheme) = self.color_scheme_opt() {
            push_sep(out);
            let _ = write!(out, "(prefers-color-scheme:{})", scheme.as_css());
        }
        if let Some(rm) = self.reduced_motion_opt() {
            push_sep(out);
            let kw = if rm { "reduce" } else { "no-preference" };
            let _ = write!(out, "(prefers-reduced-motion:{kw})");
        }
        if let Some(ptr) = self.pointer_opt() {
            push_sep(out);
            let _ = write!(out, "(pointer:{})", ptr.as_css());
        }
        !first
    }

    /// Emits exact complementary CSS `@media` conditions that hide a node whenever `self` does
    /// NOT match (De Morgan negation: `!(A && B)` -> `[!A, !B]`).
    pub fn write_negated_css_conditions(&self, out: &mut Vec<String>) {
        if !self.is_valid() {
            out.push("all".to_owned());
            return;
        }
        if let Some(min) = self.min_width_opt()
            && min > 0
        {
            out.push(format!("(max-width:{}.98px)", min - 1));
        }
        if let Some(max) = self.max_width_opt() {
            out.push(format!("(min-width:{}px)", u32::from(max) + 1));
        }
        if let Some(scheme) = self.color_scheme_opt() {
            out.push(format!("(prefers-color-scheme:{})", scheme.negated_css()));
        }
        if let Some(rm) = self.reduced_motion_opt() {
            let kw = if rm { "no-preference" } else { "reduce" };
            out.push(format!("(prefers-reduced-motion:{kw})"));
        }
        if let Some(ptr) = self.pointer_opt() {
            out.push(format!("(pointer:{})", ptr.negated_css()));
        }
    }
}

/// Sparse, strictly typed layout & visual style overrides applied when a [`MediaQuery`] matches.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ResponsiveStylePatch {
    pub flex_direction: Option<FlexDirection>,
    pub justify_content: Option<FlexJustify>,
    pub align_items: Option<FlexAlign>,
    pub flex_grow: Option<f32>,
    pub flex_shrink: Option<f32>,
    pub gap: Option<f32>,
    pub width: Option<Dimension>,
    pub height: Option<Dimension>,
    pub min_width: Option<Dimension>,
    pub max_width: Option<Dimension>,
    pub min_height: Option<Dimension>,
    pub max_height: Option<Dimension>,
    pub padding: Option<EdgeInsets>,
    pub margin: Option<EdgeInsets>,
    pub background: Option<ColorRgba>,
    pub text_color: Option<ColorRgba>,
    pub border_width: Option<f32>,
    pub border_color: Option<ColorRgba>,
    pub border_radius: Option<f32>,
    pub opacity: Option<f32>,
    pub font_size: Option<f32>,
    pub z_index: Option<i32>,
    pub flex_wrap: Option<FlexWrap>,
    pub overflow: Option<Overflow>,
    pub text_align: Option<TextAlign>,
    pub font_weight: Option<u16>,
}

impl ResponsiveStylePatch {
    #[inline]
    pub const fn new() -> Self {
        Self {
            flex_direction: None, justify_content: None, align_items: None,
            flex_grow: None, flex_shrink: None, gap: None,
            width: None, height: None, min_width: None, max_width: None,
            min_height: None, max_height: None, padding: None, margin: None,
            background: None, text_color: None, border_width: None, border_color: None,
            border_radius: None, opacity: None, font_size: None, z_index: None,
            flex_wrap: None, overflow: None, text_align: None, font_weight: None,
        }
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.flex_direction.is_none() && self.justify_content.is_none() && self.align_items.is_none()
            && self.flex_grow.is_none() && self.flex_shrink.is_none() && self.gap.is_none()
            && self.width.is_none() && self.height.is_none() && self.min_width.is_none()
            && self.max_width.is_none() && self.min_height.is_none() && self.max_height.is_none()
            && self.padding.is_none() && self.margin.is_none() && self.background.is_none()
            && self.text_color.is_none() && self.border_width.is_none() && self.border_color.is_none()
            && self.border_radius.is_none() && self.opacity.is_none() && self.font_size.is_none()
            && self.z_index.is_none() && self.flex_wrap.is_none() && self.overflow.is_none()
            && self.text_align.is_none() && self.font_weight.is_none()
    }

    #[inline]
    fn sanitize(v: f32) -> f32 {
        if v.is_finite() { v } else { 0.0 }
    }

    #[inline] pub fn flex_direction(mut self, dir: FlexDirection) -> Self { self.flex_direction = Some(dir); self }
    #[inline] pub fn flex_row(self) -> Self { self.flex_direction(FlexDirection::Row) }
    #[inline] pub fn flex_col(self) -> Self { self.flex_direction(FlexDirection::Column) }
    #[inline] pub fn justify_content(mut self, j: FlexJustify) -> Self { self.justify_content = Some(j); self }
    #[inline] pub fn align_items(mut self, a: FlexAlign) -> Self { self.align_items = Some(a); self }
    #[inline] pub fn flex_grow(mut self, v: f32) -> Self { self.flex_grow = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn flex_shrink(mut self, v: f32) -> Self { self.flex_shrink = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn gap(mut self, v: f32) -> Self { self.gap = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn width(mut self, d: Dimension) -> Self { self.width = Some(d); self }
    #[inline] pub fn width_px(self, v: f32) -> Self { self.width(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn width_pct(self, v: f32) -> Self { self.width(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0))) }
    #[inline] pub fn width_auto(self) -> Self { self.width(Dimension::Auto) }
    #[inline] pub fn height(mut self, d: Dimension) -> Self { self.height = Some(d); self }
    #[inline] pub fn height_px(self, v: f32) -> Self { self.height(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn height_pct(self, v: f32) -> Self { self.height(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0))) }
    #[inline] pub fn height_auto(self) -> Self { self.height(Dimension::Auto) }
    #[inline] pub fn min_width(mut self, d: Dimension) -> Self { self.min_width = Some(d); self }
    #[inline] pub fn min_width_px(self, v: f32) -> Self { self.min_width(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn max_width(mut self, d: Dimension) -> Self { self.max_width = Some(d); self }
    #[inline] pub fn max_width_px(self, v: f32) -> Self { self.max_width(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn min_height(mut self, d: Dimension) -> Self { self.min_height = Some(d); self }
    #[inline] pub fn min_height_px(self, v: f32) -> Self { self.min_height(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn max_height(mut self, d: Dimension) -> Self { self.max_height = Some(d); self }
    #[inline] pub fn max_height_px(self, v: f32) -> Self { self.max_height(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn padding(mut self, v: f32) -> Self { self.padding = Some(EdgeInsets::all(Self::sanitize(v).max(0.0))); self }
    #[inline] pub fn padding_xy(mut self, h: f32, v: f32) -> Self { self.padding = Some(EdgeInsets::symmetric(Self::sanitize(v).max(0.0), Self::sanitize(h).max(0.0))); self }
    #[inline] pub fn padding_insets(mut self, insets: EdgeInsets) -> Self { self.padding = Some(insets); self }
    #[inline] pub fn margin(mut self, v: f32) -> Self { self.margin = Some(EdgeInsets::all(Self::sanitize(v))); self }
    #[inline] pub fn margin_xy(mut self, h: f32, v: f32) -> Self { self.margin = Some(EdgeInsets::symmetric(Self::sanitize(v), Self::sanitize(h))); self }
    #[inline] pub fn margin_insets(mut self, insets: EdgeInsets) -> Self { self.margin = Some(insets); self }
    #[inline] pub fn bg(mut self, c: ColorRgba) -> Self { self.background = Some(c); self }
    #[inline] pub fn background(self, c: ColorRgba) -> Self { self.bg(c) }
    #[inline] pub fn color(mut self, c: ColorRgba) -> Self { self.text_color = Some(c); self }
    #[inline] pub fn text_color(self, c: ColorRgba) -> Self { self.color(c) }
    #[inline] pub fn border(mut self, w: f32, c: ColorRgba) -> Self { self.border_width = Some(Self::sanitize(w).max(0.0)); self.border_color = Some(c); self }
    #[inline] pub fn border_width(mut self, w: f32) -> Self { self.border_width = Some(Self::sanitize(w).max(0.0)); self }
    #[inline] pub fn border_color(mut self, c: ColorRgba) -> Self { self.border_color = Some(c); self }
    #[inline] pub fn radius(mut self, r: f32) -> Self { self.border_radius = Some(Self::sanitize(r).max(0.0)); self }
    #[inline] pub fn border_radius(self, r: f32) -> Self { self.radius(r) }
    #[inline] pub fn opacity(mut self, o: f32) -> Self { self.opacity = Some(Self::sanitize(o).clamp(0.0, 1.0)); self }
    #[inline] pub fn font_size(mut self, px: f32) -> Self { self.font_size = Some(Self::sanitize(px).max(1.0)); self }
    #[inline] pub fn z_index(mut self, z: i32) -> Self { self.z_index = Some(z); self }
    #[inline] pub fn flex_wrap(mut self, wrap: FlexWrap) -> Self { self.flex_wrap = Some(wrap); self }
    #[inline] pub fn overflow(mut self, ov: Overflow) -> Self { self.overflow = Some(ov); self }
    #[inline] pub fn text_align(mut self, align: TextAlign) -> Self { self.text_align = Some(align); self }
    #[inline] pub fn font_weight(mut self, weight: u16) -> Self { self.font_weight = Some(weight.clamp(1, 1000)); self }

    /// Applies all present layout and visual overrides onto `layout` and `style` (native path).
    pub fn apply_to(&self, layout: &mut LayoutProps, style: &mut StyleProps) {
        if let Some(v) = self.flex_direction { layout.flex_direction = v; }
        if let Some(v) = self.justify_content { layout.justify_content = v; }
        if let Some(v) = self.align_items { layout.align_items = v; }
        if let Some(v) = self.flex_grow { layout.flex_grow = v; }
        if let Some(v) = self.flex_shrink { layout.flex_shrink = v; }
        if let Some(v) = self.gap { layout.gap = v; }
        if let Some(v) = self.width { layout.width = v; }
        if let Some(v) = self.height { layout.height = v; }
        if let Some(v) = self.min_width { layout.min_width = v; }
        if let Some(v) = self.max_width { layout.max_width = v; }
        if let Some(v) = self.min_height { layout.min_height = v; }
        if let Some(v) = self.max_height { layout.max_height = v; }
        if let Some(v) = self.padding { layout.padding = v; }
        if let Some(v) = self.margin { layout.margin = v; }
        if let Some(v) = self.background { style.background = v; }
        if let Some(v) = self.text_color { style.text_color = v; }
        if let Some(v) = self.border_width { style.border_width = v; }
        if let Some(v) = self.border_color { style.border_color = v; }
        if let Some(v) = self.border_radius { style.border_radius = v; }
        if let Some(v) = self.opacity { style.opacity = v; }
        if let Some(v) = self.font_size { style.font_size = v; }
        if let Some(v) = self.z_index { style.z_index = v; }
    }

    /// Applies layout/style overrides and invokes `on_raw` for the 4 non-ABI CSS properties
    /// (`flex-wrap`, `overflow`, `text-align`, `font-weight`).
    pub fn apply_to_with_raw<F: FnMut(&str, &str)>(
        &self,
        layout: &mut LayoutProps,
        style: &mut StyleProps,
        mut on_raw: F,
    ) {
        self.apply_to(layout, style);
        if let Some(w) = self.flex_wrap { on_raw("flex-wrap", w.as_css()); }
        if let Some(o) = self.overflow { on_raw("overflow", o.as_css()); }
        if let Some(a) = self.text_align { on_raw("text-align", a.as_css()); }
        if let Some(fw) = self.font_weight {
            let mut buf = String::with_capacity(4);
            let _ = write!(&mut buf, "{fw}");
            on_raw("font-weight", &buf);
        }
    }
}

/// One conditional responsive style rule pairing a [`MediaQuery`] with a [`ResponsiveStylePatch`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponsiveRule {
    pub query: MediaQuery,
    pub patch: ResponsiveStylePatch,
}

impl ResponsiveRule {
    #[inline]
    pub const fn new(query: MediaQuery, patch: ResponsiveStylePatch) -> Self {
        Self { query, patch }
    }
}
