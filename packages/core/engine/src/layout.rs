//! Composable, zero-cost-monomorphizable and runtime-switchable `NodeStore` layout engine
//! abstractions (`StoreLayoutEngine`, `DynLayoutEngine`, `TextMeasurer`).
//!
//! Backends such as `HeadlessBackend<L>` and `SkiaBackend<L>` are generic over
//! [`StoreLayoutEngine`] (defaulting to `TaffyStoreLayout` for zero dynamic-dispatch overhead)
//! while also supporting dynamic runtime switching via [`DynLayoutEngine`]
//! (`Box<dyn StoreLayoutEngine>`).

use std::sync::Arc;

use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::{LayoutRect, Viewport};

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
                for line in text.split('\n') {
                    let raw_w = line.chars().count() as f32 * char_w;
                    if raw_w > mw {
                        let wrapped = (raw_w / mw).ceil().max(1.0);
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

/// Identifies built-in layout engines shipped with NVE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinLayout {
    Taffy,
    Yoga,
}

impl BuiltinLayout {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Taffy => "taffy",
            Self::Yoga => "yoga",
        }
    }
}

/// Direct `NodeStore` layout engine trait implemented by `TaffyStoreLayout`
/// (`nve-backend-headless`), real React Native Yoga (`nve-layout-yoga`), custom layout engines,
/// and [`DynLayoutEngine`] (`Box<dyn StoreLayoutEngine>`).
///
/// Computed rectangles returned by [`StoreLayoutEngine::layout_of`] are expressed in
/// surface-space (root-relative absolute) coordinates in logical points.
pub trait StoreLayoutEngine: Send {
    /// Human-readable engine identifier (e.g. `"taffy"`, `"yoga"`).
    fn name(&self) -> &'static str;

    /// Invalidates cached layout entries for every layout/measure/structural change in `changes`.
    fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet);

    /// Computes layout for `root` at `viewport` directly from `store` word columns.
    ///
    /// **Self-gating contract**: the engine is the single owner of the clean-frame gate. Callers
    /// invoke this on every frame after [`StoreLayoutEngine::sync_changes`]; when nothing that
    /// affects layout changed since the last pass (no layout/measure/structural change, same
    /// `root` and viewport, no measurer swap or reset), the engine returns `false` without
    /// running a pass and every [`StoreLayoutEngine::layout_of`] result stays current. Returns
    /// `true` when a pass ran (layout results may have changed). A pass that unwinds (e.g. a
    /// panicking host `TextMeasurer`) must leave the engine dirty so the next call retries.
    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool;

    /// Returns the absolute rounded layout rect of `id` from the latest pass, if live.
    fn layout_of(&self, id: NodeId) -> Option<LayoutRect>;

    /// Installs a custom text measurer and invalidates cached text measurements.
    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>);

    /// Clears all cached layout state.
    fn reset(&mut self);
}

/// Type alias for a runtime-switchable boxed `NodeStore` layout engine.
pub type DynLayoutEngine = Box<dyn StoreLayoutEngine>;

impl<L: StoreLayoutEngine + ?Sized> StoreLayoutEngine for Box<L> {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        (**self).sync_changes(store, changes);
    }

    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool {
        (**self).compute_layout(store, root, viewport)
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        (**self).layout_of(id)
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        (**self).set_text_measurer(measurer);
    }

    fn reset(&mut self) {
        (**self).reset();
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

        // Closure blanket impl & Arc<dyn TextMeasurer> work identically.
        let custom: Arc<dyn TextMeasurer> =
            Arc::new(|text: &str, fs: f32, _mw: Option<f32>| (text.len() as f32 * fs, fs * 2.0));
        assert_eq!(custom.measure("abc", 10.0, None), (30.0, 20.0));
    }
}
