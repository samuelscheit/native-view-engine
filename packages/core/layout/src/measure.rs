//! Pluggable text measurement (`TextMeasurer`) and the deterministic cross-platform
//! `HeuristicTextMeasurer` fallback used by every layout engine.

use std::sync::Arc;

/// Pluggable text measurement provider used by layout engines to size intrinsic text nodes
/// (including multi-line and width-constrained wrapping).
pub trait TextMeasurer: Send + Sync {
    /// Measures `text` at `font_size` logical points under an optional `max_width` constraint.
    ///
    /// Returns `(width, height)` in logical points.
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32);

    /// Measures `text` at `font_size` logical points and CSS `font_weight` (`100..=900`, default `400`).
    ///
    /// Defaults to [`TextMeasurer::measure`] for measurers that are weight-invariant.
    fn measure_with_weight(
        &self,
        text: &str,
        font_size: f32,
        _font_weight: u16,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        self.measure(text, font_size, max_width)
    }
}

/// Default deterministic text measurer matching NVE's cross-platform heuristic
/// (`font_size * 0.58` average character advance, `font_size * 1.35` line height),
/// with explicit `\n` line break and `max_width` wrapping support.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeuristicTextMeasurer;

impl HeuristicTextMeasurer {
    pub const CHAR_WIDTH_FACTOR: f32 = 0.58;
    pub const LINE_HEIGHT_FACTOR: f32 = 1.35;
    pub const DEFAULT_FONT_SIZE: f32 = 16.0;
}

impl TextMeasurer for HeuristicTextMeasurer {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        let fs = if font_size.is_finite() && font_size > 0.0 {
            font_size
        } else {
            Self::DEFAULT_FONT_SIZE
        };
        let char_w = fs * Self::CHAR_WIDTH_FACTOR;
        let line_h = fs * Self::LINE_HEIGHT_FACTOR;

        let mut max_line_w = 0.0f32;
        let mut total_lines = 0.0f32;

        match max_width {
            Some(mw) if !mw.is_finite() || mw <= 0.0 => {
                // Min-content probe (e.g. Yoga `AtMost 0` or Taffy `MinContent`).
                let explicit_lines = text.split('\n').count().max(1) as f32;
                (0.0, explicit_lines * line_h)
            }
            Some(mw) => {
                // Allow a sub-millipixel tolerance so float32 add/sub round-trips of padding +
                // border (e.g. `(6.38 + 18.0) - 18.0 = 6.3799992 < 6.3800001`) never spuriously
                // wrap a fitting line onto an extra line.
                let wrap_eps = (mw * 1e-4).max(1e-3);
                for line in text.split('\n') {
                    let raw_w = line.chars().count() as f32 * char_w;
                    if raw_w > mw + wrap_eps {
                        let wrapped = ((raw_w - wrap_eps) / mw).ceil().max(1.0);
                        max_line_w = max_line_w.max(mw);
                        total_lines += wrapped;
                    } else {
                        max_line_w = max_line_w.max(raw_w);
                        total_lines += 1.0;
                    }
                }
                let final_w = max_line_w.max(1.0).min(mw);
                (final_w, total_lines.max(1.0) * line_h)
            }
            None => {
                for line in text.split('\n') {
                    let raw_w = line.chars().count() as f32 * char_w;
                    max_line_w = max_line_w.max(raw_w);
                    total_lines += 1.0;
                }
                (max_line_w.max(1.0), total_lines.max(1.0) * line_h)
            }
        }
    }
}

impl<F> TextMeasurer for F
where
    F: Fn(&str, f32, Option<f32>) -> (f32, f32) + Send + Sync,
{
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        (self)(text, font_size, max_width)
    }
}

impl<T: TextMeasurer + ?Sized> TextMeasurer for Arc<T> {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        (**self).measure(text, font_size, max_width)
    }

    fn measure_with_weight(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        (**self).measure_with_weight(text, font_size, font_weight, max_width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_text_measurer_matches_single_line_and_wraps() {
        let m = HeuristicTextMeasurer;

        // Empty text produces zero size.
        assert_eq!(m.measure("", 16.0, None), (0.0, 0.0));

        // Single-line unconstrained matches font_size * 0.58 and font_size * 1.35.
        let (w, h) = m.measure("Hello World", 20.0, None);
        assert!((w - 11.0 * 20.0 * 0.58).abs() < 1e-4);
        assert!((h - 20.0 * 1.35).abs() < 1e-4);

        // Wrapping under max_width increases line count proportionally.
        let (ww, wh) = m.measure("01234567890123456789", 20.0, Some(100.0));
        assert!((ww - 100.0).abs() < 1e-4);
        // 20 * 11.6 = 232px -> ceil(232 / 100) = 3 lines
        assert!((wh - 3.0 * 20.0 * 1.35).abs() < 1e-4);

        // Explicit newline increases line count.
        let (nw, nh) = m.measure("Line 1\nLine 2", 16.0, None);
        assert!((nw - 6.0 * 16.0 * 0.58).abs() < 1e-4);
        assert!((nh - 2.0 * 16.0 * 1.35).abs() < 1e-4);

        // Float32 padding+border round-trip (`(w + 18.0) - 18.0` losing 1 ULP) must not
        // spuriously wrap a single-line string onto two lines.
        let (single_w, single_h) = m.measure("8", 11.0, None);
        let round_trip_mw = (single_w + 18.0_f32) - 18.0_f32;
        assert_eq!(m.measure("8", 11.0, Some(round_trip_mw)).1, single_h);

        // Closure blanket impl & Arc<dyn TextMeasurer> work identically.
        let custom: Arc<dyn TextMeasurer> =
            Arc::new(|text: &str, fs: f32, _mw: Option<f32>| (text.len() as f32 * fs, fs * 2.0));
        assert_eq!(custom.measure("abc", 10.0, None), (30.0, 20.0));
    }
}
