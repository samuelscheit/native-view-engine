//! Breakpoint-general, strictly typed responsive `@media` queries, sparse style patches,
//! native viewport evaluation, and deterministic CSS `@media` emission.
//!
//! Zero serialization and zero arbitrary strings: [`MediaQuery`] is a compact 8-byte `#[repr(C)]`
//! `Copy` struct directly shareable in memory across Rust, FFI, WASM, and native backends with
//! 0% CSS injection surface.

use std::fmt::Write as _;

pub use crate::client_hints::{
    ClientHintsSnapshot, SSR_ACCEPT_CH_HEADER, SSR_CRITICAL_CH_HEADER, SSR_VARY_HEADER,
};
pub use crate::style_patch::{Display, FlexWrap, Overflow, StylePatch, TextAlign};

/// Backwards-compatible alias for the unified [`StylePatch`] shared by responsive `@media` and
/// interactive pseudo-state rules.
pub type ResponsiveStylePatch = StylePatch;

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

/// One conditional responsive style rule pairing a [`MediaQuery`] with a [`StylePatch`].
#[derive(Debug, Clone, PartialEq)]
pub struct ResponsiveRule {
    pub query: MediaQuery,
    pub patch: StylePatch,
}

impl ResponsiveRule {
    #[inline]
    pub const fn new(query: MediaQuery, patch: StylePatch) -> Self {
        Self { query, patch }
    }
}
