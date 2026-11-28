//! Pluggable text measurement (`TextMeasurer`), the inherited [`TextStyle`] every measurer
//! receives, and the deterministic cross-platform `HeuristicTextMeasurer` fallback used by every
//! layout engine.

use std::sync::Arc;

use nve_abi::xxh64;
use nve_components::tables::{FontStyle, OverflowWrap, Typography, WhiteSpace, WordBreak};
use nve_store::{Length, NodeId, NodeStore, Rgba, StrSpan};

/// Fully resolved, inherited typography of one node: everything a host needs to measure text
/// exactly as it paints it. `F` is the family: a [`ResolvedFamily`] into the store while layout
/// keeps it per node ([`ResolvedTextStyle`]), a borrowed `&str` when handed to a [`TextMeasurer`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle<F> {
    pub color: Rgba,
    /// CSS font-family list as authored; empty = the host's default UI font.
    pub font_family: F,
    /// Logical points, finite and `>= 0`.
    pub font_size: f32,
    /// CSS weight (`100..=900` once a host clamps it).
    pub font_weight: u16,
    pub font_style: FontStyle,
    /// Extra advance after every character, logical points.
    pub letter_spacing: f32,
    /// Points = absolute; percent = unit-less multiplier × 100 of `font_size`; auto = `normal`.
    pub line_height: Length,
    pub white_space: WhiteSpace,
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrap,
    /// Maximum painted lines (`0` = unlimited); measured height never exceeds them.
    pub max_lines: u32,
}

/// A node's resolved font family: where it lives in the store plus the `xxh64` of its text.
/// Equality is by content hash alone, so an in-place `StrArena` replace that keeps the span
/// (`"Arial"` → `"Inter"`) still compares unequal, while the same family at another span does not.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResolvedFamily {
    pub span: StrSpan,
    pub hash: u64,
}

impl ResolvedFamily {
    /// The family `span` of `store`, hashed now (`0` for the empty default family).
    #[inline]
    pub fn new(store: &NodeStore, span: StrSpan) -> Self {
        let text = store.text(span);
        Self { span, hash: if text.is_empty() { 0 } else { xxh64(text.as_bytes(), 0) } }
    }
}

impl PartialEq for ResolvedFamily {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

/// Inherited typography kept per node during a layout pass.
pub type ResolvedTextStyle = TextStyle<ResolvedFamily>;

impl<F: Default> Default for TextStyle<F> {
    fn default() -> Self {
        Self {
            color: Rgba::BLACK,
            font_family: F::default(),
            font_size: HeuristicTextMeasurer::DEFAULT_FONT_SIZE,
            font_weight: 400,
            font_style: FontStyle::Normal,
            letter_spacing: 0.0,
            line_height: Length::AUTO,
            white_space: WhiteSpace::Normal,
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            max_lines: 0,
        }
    }
}

impl<F> TextStyle<F> {
    /// The same style with the family mapped to another representation.
    #[inline]
    pub fn map_family<'s, G>(&'s self, f: impl FnOnce(&'s F) -> G) -> TextStyle<G> {
        TextStyle {
            color: self.color,
            font_family: f(&self.font_family),
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
            letter_spacing: self.letter_spacing,
            line_height: self.line_height,
            white_space: self.white_space,
            word_break: self.word_break,
            overflow_wrap: self.overflow_wrap,
            max_lines: self.max_lines,
        }
    }

    /// `false` for `white-space: nowrap | pre`: lines break only at `\n`, never at `max_width`.
    #[inline]
    pub fn wraps(&self) -> bool {
        !matches!(self.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre)
    }

    /// The line height in points, `None` for `normal` (the font's own metrics).
    #[inline]
    pub fn line_height_points(&self) -> Option<f32> {
        let lh = if self.line_height.is_percent() {
            self.line_height.value() * self.font_size / 100.0
        } else {
            self.line_height.resolve(self.font_size)?
        };
        (lh.is_finite() && lh > 0.0).then_some(lh)
    }
}

impl<F: PartialEq> TextStyle<F> {
    /// `true` when both styles measure identically: every field except paint-only `color`,
    /// floats compared by bits (so `NaN` never forces endless re-measurement).
    #[inline]
    pub fn measure_eq(&self, other: &Self) -> bool {
        self.font_family == other.font_family
            && self.font_size.to_bits() == other.font_size.to_bits()
            && self.font_weight == other.font_weight
            && self.font_style == other.font_style
            && self.letter_spacing.to_bits() == other.letter_spacing.to_bits()
            && self.line_height == other.line_height
            && self.white_space == other.white_space
            && self.word_break == other.word_break
            && self.overflow_wrap == other.overflow_wrap
            && self.max_lines == other.max_lines
    }
}

impl<F: AsRef<str>> TextStyle<F> {
    /// The measurer view of an owned-family style.
    #[inline]
    pub fn borrowed(&self) -> TextStyle<&str> {
        self.map_family(AsRef::as_ref)
    }
}

impl TextStyle<&str> {
    /// Default typography at `font_size` (the host's default family).
    pub fn sized(font_size: f32) -> Self {
        Self {
            font_size,
            ..Self::default()
        }
    }
}

impl ResolvedTextStyle {
    /// The measurer view of this style, its family read from `store`.
    #[inline]
    pub fn in_store<'a>(&self, store: &'a NodeStore) -> TextStyle<&'a str> {
        self.map_family(|family| store.text(family.span))
    }
}

/// Resolves one node's inherited [`ResolvedTextStyle`] from its parent's resolved typography and
/// any explicit fields on `id`'s [`Typography`] row. Shared by every layout engine so invalid
/// (`< 0` or non-finite) sizes and spacings fall back identically across Taffy and Yoga.
#[inline]
pub fn resolve_inherited_text(
    store: &NodeStore,
    id: NodeId,
    mut parent: ResolvedTextStyle,
) -> ResolvedTextStyle {
    let Some(t) = store.try_row::<Typography>(id) else {
        return parent;
    };
    let set = t.set();
    // Rows without `set` bits (older producers) mark a field explicit by a non-default value.
    let explicit = |bit: u32, differs: bool| set & bit != 0 || (set == 0 && differs);
    if explicit(Typography::SET_COLOR, t.color() != Rgba::BLACK) {
        parent.color = t.color();
    }
    if explicit(Typography::SET_FONT_SIZE, t.font_size().to_bits() != 16.0f32.to_bits()) {
        let raw_fs = t.font_size();
        if raw_fs.is_finite() && raw_fs >= 0.0 {
            parent.font_size = raw_fs;
        }
    }
    if explicit(Typography::SET_FONT_WEIGHT, t.font_weight() != 400) {
        parent.font_weight = t.font_weight();
    }
    if explicit(Typography::SET_LINE_HEIGHT, t.line_height() != Length::AUTO) {
        let lh = t.line_height();
        // `NaN` / `inf` / negative heights are invalid CSS: keep the inherited value.
        if lh.is_auto() || (lh.value().is_finite() && lh.value() >= 0.0) {
            parent.line_height = lh;
        }
    }
    if explicit(Typography::SET_LETTER_SPACING, t.letter_spacing() != 0.0) {
        let ls = t.letter_spacing();
        if ls.is_finite() {
            parent.letter_spacing = ls;
        }
    }
    if explicit(Typography::SET_FONT_FAMILY, !t.font_family().is_empty()) {
        parent.font_family = ResolvedFamily::new(store, t.font_family_span());
    }
    if explicit(Typography::SET_MAX_LINES, t.max_lines() != 0) {
        parent.max_lines = t.max_lines();
    }
    if explicit(Typography::SET_FONT_STYLE, t.font_style() != FontStyle::Normal) {
        parent.font_style = t.font_style();
    }
    if explicit(Typography::SET_WHITE_SPACE, t.white_space() != WhiteSpace::Normal) {
        parent.white_space = t.white_space();
    }
    if explicit(Typography::SET_WORD_BREAK, t.word_break() != WordBreak::Normal) {
        parent.word_break = t.word_break();
    }
    if explicit(Typography::SET_OVERFLOW_WRAP, t.overflow_wrap() != OverflowWrap::Normal) {
        parent.overflow_wrap = t.overflow_wrap();
    }
    parent
}

/// Pluggable text measurement provider used by layout engines to size intrinsic text nodes
/// (including multi-line and width-constrained wrapping).
pub trait TextMeasurer: Send + Sync {
    /// Measures `text` with the node's resolved typography under an optional `max_width`
    /// constraint (`Some(0.0)` is a min-content probe). Returns `(width, height)` in logical
    /// points.
    fn measure(&self, text: &str, style: &TextStyle<&str>, max_width: Option<f32>) -> (f32, f32);
}

/// Default deterministic text measurer matching NVE's cross-platform heuristic
/// (`font_size * 0.58 + letter_spacing` per character, `font_size * 1.35` `normal` line height),
/// with explicit `\n` line breaks, `max_width` wrapping (none for `nowrap` / `pre`), explicit
/// line heights, and `max_lines`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeuristicTextMeasurer;

impl HeuristicTextMeasurer {
    pub const CHAR_WIDTH_FACTOR: f32 = 0.58;
    pub const LINE_HEIGHT_FACTOR: f32 = 1.35;
    pub const DEFAULT_FONT_SIZE: f32 = 16.0;
}

impl TextMeasurer for HeuristicTextMeasurer {
    fn measure(&self, text: &str, style: &TextStyle<&str>, max_width: Option<f32>) -> (f32, f32) {
        if text.is_empty() || style.font_size == 0.0 {
            return (0.0, 0.0);
        }
        let fs = if style.font_size.is_finite() && style.font_size > 0.0 {
            style.font_size
        } else {
            Self::DEFAULT_FONT_SIZE
        };
        let sized = TextStyle { font_size: fs, ..*style };
        let char_w = (fs * Self::CHAR_WIDTH_FACTOR + style.letter_spacing).max(0.0);
        let line_h = sized.line_height_points().unwrap_or(fs * Self::LINE_HEIGHT_FACTOR);
        let height = |lines: f32| {
            let lines = lines.max(1.0);
            let capped = if style.max_lines > 0 { lines.min(style.max_lines as f32) } else { lines };
            capped * line_h
        };
        let line_widths = || text.split('\n').map(|line| line.chars().count() as f32 * char_w);

        match max_width.filter(|_| style.wraps()) {
            Some(mw) if !mw.is_finite() || mw <= 0.0 => {
                // Min-content probe (e.g. Yoga `AtMost 0` or Taffy `MinContent`).
                (0.0, height(text.split('\n').count() as f32))
            }
            Some(mw) => {
                // Allow a few-ULP float32 tolerance (~1e-5 relative/absolute) so padding+border
                // add/sub round-trips (e.g. `(6.38_f32 + 18.0) - 18.0 = 6.3799992 < 6.3800001`)
                // do not spuriously wrap an exact-fit single line onto two lines.
                let wrap_eps = (mw * 1e-5).max(1e-5);
                let (mut max_line_w, mut total_lines) = (0.0f32, 0.0f32);
                for raw_w in line_widths() {
                    if raw_w > mw + wrap_eps {
                        max_line_w = max_line_w.max(mw);
                        total_lines += ((raw_w - wrap_eps) / mw).ceil().max(1.0);
                    } else {
                        max_line_w = max_line_w.max(raw_w);
                        total_lines += 1.0;
                    }
                }
                (max_line_w.max(1.0).min(mw), height(total_lines))
            }
            None => {
                let (max_line_w, lines) =
                    line_widths().fold((0.0f32, 0.0f32), |(w, n), lw| (w.max(lw), n + 1.0));
                (max_line_w.max(1.0), height(lines))
            }
        }
    }
}

impl<F> TextMeasurer for F
where
    F: Fn(&str, &TextStyle<&str>, Option<f32>) -> (f32, f32) + Send + Sync,
{
    fn measure(&self, text: &str, style: &TextStyle<&str>, max_width: Option<f32>) -> (f32, f32) {
        (self)(text, style, max_width)
    }
}

impl<T: TextMeasurer + ?Sized> TextMeasurer for Arc<T> {
    fn measure(&self, text: &str, style: &TextStyle<&str>, max_width: Option<f32>) -> (f32, f32) {
        (**self).measure(text, style, max_width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sized(fs: f32) -> TextStyle<&'static str> {
        TextStyle::sized(fs)
    }

    #[test]
    fn heuristic_text_measurer_matches_single_line_and_wraps() {
        let m = HeuristicTextMeasurer;

        // Empty text produces zero size.
        assert_eq!(m.measure("", &sized(16.0), None), (0.0, 0.0));

        // Single-line unconstrained matches font_size * 0.58 and font_size * 1.35.
        let (w, h) = m.measure("Hello World", &sized(20.0), None);
        assert!((w - 11.0 * 20.0 * 0.58).abs() < 1e-4);
        assert!((h - 20.0 * 1.35).abs() < 1e-4);

        // Wrapping under max_width increases line count proportionally.
        let (ww, wh) = m.measure("01234567890123456789", &sized(20.0), Some(100.0));
        assert!((ww - 100.0).abs() < 1e-4);
        // 20 * 11.6 = 232px -> ceil(232 / 100) = 3 lines
        assert!((wh - 3.0 * 20.0 * 1.35).abs() < 1e-4);

        // Explicit newline increases line count.
        let (nw, nh) = m.measure("Line 1\nLine 2", &sized(16.0), None);
        assert!((nw - 6.0 * 16.0 * 0.58).abs() < 1e-4);
        assert!((nh - 2.0 * 16.0 * 1.35).abs() < 1e-4);

        // Float32 padding+border round-trip (`(w + 18.0) - 18.0` losing 1 ULP) must not
        // spuriously wrap a single-line string onto two lines.
        let (single_w, single_h) = m.measure("8", &sized(11.0), None);
        let round_trip_mw = (single_w + 18.0_f32) - 18.0_f32;
        assert_eq!(m.measure("8", &sized(11.0), Some(round_trip_mw)).1, single_h);

        // Closure blanket impl & Arc<dyn TextMeasurer> work identically.
        let custom: Arc<dyn TextMeasurer> =
            Arc::new(|text: &str, s: &TextStyle<&str>, _mw: Option<f32>| {
                (text.len() as f32 * s.font_size, s.font_size * 2.0)
            });
        assert_eq!(custom.measure("abc", &sized(10.0), None), (30.0, 20.0));
    }

    #[test]
    fn heuristic_text_measurer_honours_the_resolved_typography() {
        let m = HeuristicTextMeasurer;
        let text = "0123456789";
        let base = m.measure(text, &sized(20.0), None);

        // Letter spacing widens every character.
        let spaced = TextStyle { letter_spacing: 2.0, ..sized(20.0) };
        assert!((m.measure(text, &spaced, None).0 - (base.0 + 10.0 * 2.0)).abs() < 1e-3);

        // `nowrap` / `pre` ignore the width constraint (also the min-content probe).
        for ws in [WhiteSpace::NoWrap, WhiteSpace::Pre] {
            let s = TextStyle { white_space: ws, ..sized(20.0) };
            assert_eq!(m.measure(text, &s, Some(50.0)), base, "{ws:?}");
            assert_eq!(m.measure(text, &s, Some(0.0)), base, "{ws:?}");
        }
        let pre_wrap = TextStyle { white_space: WhiteSpace::PreWrap, ..sized(20.0) };
        assert!(m.measure(text, &pre_wrap, Some(50.0)).1 > base.1);

        // Line heights: points are absolute, percent is a multiplier of the font size.
        let points = TextStyle { line_height: Length::points(30.0), ..sized(20.0) };
        assert_eq!(m.measure("a\nb", &points, None).1, 60.0);
        let percent = TextStyle { line_height: Length::percent(150.0), ..sized(20.0) };
        assert_eq!(m.measure("a\nb", &percent, None).1, 60.0);
        assert_eq!(percent.line_height_points(), Some(30.0));
        assert_eq!(sized(20.0).line_height_points(), None);

        // `max_lines` caps wrapped and explicit lines.
        let two = TextStyle { max_lines: 2, ..points };
        assert_eq!(m.measure(text, &two, Some(20.0)).1, 60.0);
        assert_eq!(m.measure("a\nb\nc\nd", &two, None).1, 60.0);
    }

    #[test]
    fn resolution_sanitizes_line_height_and_hashes_the_family() {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let [parent, nan, negative, family] = [(); 4].map(|_| tx.create());
        tx.edit::<Typography>(parent).line_height(Length::points(18.0)).font_family("Inter");
        // Raw `NaN` points bits (tag `00`): no constructor produces them, a wire frame can.
        tx.edit::<Typography>(nan)
            .line_height(Length(f32::NAN.to_bits() & !0b11))
            .set(Typography::SET_LINE_HEIGHT);
        tx.edit::<Typography>(negative).line_height(Length::points(-4.0));
        tx.edit::<Typography>(family).font_family("Arial");
        tx.commit_into(&mut nve_store::ChangeSet::default());
        let p = resolve_inherited_text(&store, parent, ResolvedTextStyle::default());
        for child in [nan, negative] {
            let r = resolve_inherited_text(&store, child, p);
            assert_eq!(r.line_height, Length::points(18.0), "invalid heights inherit");
            assert!(r.measure_eq(&r), "bitwise float equality is reflexive");
        }
        let inter = p.font_family;
        assert_eq!(inter.hash, xxh64(b"Inter", 0));
        let arial = resolve_inherited_text(&store, family, p);
        assert!(!arial.measure_eq(&p), "same-length families differ by content");
        assert!(arial.measure_eq(&TextStyle { color: Rgba::WHITE, ..arial }), "color is paint-only");
        assert_eq!(ResolvedTextStyle::default().font_family.hash, 0);
    }
}
