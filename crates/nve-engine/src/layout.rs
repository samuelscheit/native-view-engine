//! Composable, zero-cost-monomorphizable and runtime-switchable layout engine abstractions.
//!
//! Backends such as `HeadlessBackend<L>` and `SkiaBackend<L>` are generic over
//! [`LayoutEngine`] (defaulting to Taffy for zero dynamic-dispatch overhead) while also
//! supporting dynamic runtime switching via [`DynLayoutEngine`] (`Box<dyn LayoutEngine>`).

use std::sync::Arc;

use nve_abi::{ComponentKind, LayoutProps, NodeId, StyleProps};

use crate::{LayoutRect, Viewport};

/// Pluggable text measurement provider used by layout engines to size intrinsic text nodes
/// (including multi-line and width-constrained wrapping).
pub trait TextMeasurer: Send + Sync {
    /// Measures `text` at `font_size` logical points under an optional `max_width` constraint.
    ///
    /// Returns `(width, height)` in logical points.
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32);
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

/// Core layout engine trait implemented by Taffy (`nve-backend-headless`), real React Native
/// Yoga (`nve-layout-yoga`), custom layout engines, and [`DynLayoutEngine`] (`Box<dyn LayoutEngine>`).
///
/// Computed rectangles returned by [`LayoutEngine::layout_of`] are expressed in surface-space
/// (root-relative absolute) coordinates in logical points.
pub trait LayoutEngine: Send {
    /// Human-readable engine identifier (e.g. `"taffy"`, `"yoga"`).
    fn name(&self) -> &'static str;

    /// Creates a node if absent or updates its [`ComponentKind`] if present.
    fn upsert_node(&mut self, id: NodeId, kind: ComponentKind);

    /// Removes a node from the layout tree if present.
    fn remove_node(&mut self, id: NodeId);

    /// Applies Flexbox geometry properties (`LayoutProps`) to a node.
    fn set_layout(&mut self, id: NodeId, layout: &LayoutProps);

    /// Applies visual style properties (`StyleProps`) relevant to layout (e.g. `font_size`).
    fn set_style(&mut self, id: NodeId, style: &StyleProps);

    /// Updates a node's text payload used for intrinsic text measurement.
    fn set_text(&mut self, id: NodeId, text: &str);

    /// Replaces the ordered child list of `parent`.
    fn set_children(&mut self, parent: NodeId, children: &[NodeId]);

    /// Installs the [`TextMeasurer`] used for intrinsic text sizing and invalidates cached text measurements.
    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>);

    /// Computes layout for the tree rooted at `root` within `viewport`.
    fn compute_layout(&mut self, root: NodeId, viewport: Viewport);

    /// Returns the computed surface-space (root-relative) [`LayoutRect`] for `id`, if available.
    fn layout_of(&self, id: NodeId) -> Option<LayoutRect>;

    /// Clears all nodes and cached layout results while retaining the installed [`TextMeasurer`].
    fn reset(&mut self);
}

/// Type alias for a runtime-switchable boxed layout engine.
pub type DynLayoutEngine = Box<dyn LayoutEngine>;

impl<L: LayoutEngine + ?Sized> LayoutEngine for Box<L> {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn upsert_node(&mut self, id: NodeId, kind: ComponentKind) {
        (**self).upsert_node(id, kind);
    }

    fn remove_node(&mut self, id: NodeId) {
        (**self).remove_node(id);
    }

    fn set_layout(&mut self, id: NodeId, layout: &LayoutProps) {
        (**self).set_layout(id, layout);
    }

    fn set_style(&mut self, id: NodeId, style: &StyleProps) {
        (**self).set_style(id, style);
    }

    fn set_text(&mut self, id: NodeId, text: &str) {
        (**self).set_text(id, text);
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        (**self).set_children(parent, children);
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        (**self).set_text_measurer(measurer);
    }

    fn compute_layout(&mut self, root: NodeId, viewport: Viewport) {
        (**self).compute_layout(root, viewport);
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        (**self).layout_of(id)
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
