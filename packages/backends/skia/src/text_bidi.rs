//! Single-line HarfBuzz + Unicode BiDi text layout, grapheme-safe visual hit-testing,
//! logical-to-visual caret X mapping, visual arrow navigation, and disjoint multi-run
//! selection geometry powered by `cosmic-text`.

use cosmic_text::{Buffer, Metrics, Shaping, Wrap};
use nve_layout::TextMeasurer;
use unicode_segmentation::UnicodeSegmentation;

use crate::font::BitmapFontTextMeasurer;
use crate::text::CosmicTextEngine;
use crate::text_cache::{line_height_for, sanitize_font_size, sanitize_font_weight};
use crate::text_grapheme::{
    clamp_grapheme_boundary, grapheme_boundaries, next_grapheme_boundary, prev_grapheme_boundary,
};

/// One UAX #29 extended grapheme cluster positioned in visual left-to-right line coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualGraphemeCluster {
    /// Logical UTF-8 start byte offset in the source string (always a grapheme boundary).
    pub byte_start: usize,
    /// Logical UTF-8 end byte offset in the source string (always a grapheme boundary).
    pub byte_end: usize,
    /// Visual left edge in logical pixels relative to line origin.
    pub x_left: f32,
    /// Visual right edge in logical pixels relative to line origin.
    pub x_right: f32,
    /// `true` when this cluster belongs to an odd BiDi embedding level (RTL run).
    pub is_rtl: bool,
}

/// Shaped single-line BiDi layout containing visual left-to-right grapheme cluster geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedLineLayout {
    /// `true` when the resolved Unicode BiDi paragraph direction is RTL.
    pub rtl: bool,
    /// Total visual advance width of the line in logical pixels (including trailing spaces).
    pub line_width: f32,
    /// Grapheme clusters ordered left-to-right by visual X coordinate.
    pub clusters: Vec<VisualGraphemeCluster>,
}

impl ShapedLineLayout {
    /// Returns `true` if the paragraph or any shaped cluster is RTL.
    #[inline]
    pub fn has_rtl(&self) -> bool {
        self.rtl || self.clusters.iter().any(|c| c.is_rtl)
    }

    /// Maps a logical UTF-8 byte offset (clamped to a UAX #29 grapheme boundary) to its visual
    /// caret X offset in logical pixels relative to line origin.
    ///
    /// At LTR/RTL run boundaries, paragraph-direction-matching clusters take precedence so caret
    /// motion remains monotonic within every run.
    pub fn caret_x_at(&self, text: &str, byte_index: usize) -> f32 {
        if text.is_empty() || self.clusters.is_empty() {
            return 0.0;
        }
        let idx = clamp_grapheme_boundary(text, byte_index);

        let mut start_match: Option<&VisualGraphemeCluster> = None;
        let mut end_match: Option<&VisualGraphemeCluster> = None;
        for c in &self.clusters {
            if c.byte_start == idx && start_match.is_none() {
                start_match = Some(c);
            }
            if c.byte_end == idx && end_match.is_none() {
                end_match = Some(c);
            }
        }

        let leading_edge = |c: &VisualGraphemeCluster| if c.is_rtl { c.x_right } else { c.x_left };
        let trailing_edge = |c: &VisualGraphemeCluster| if c.is_rtl { c.x_left } else { c.x_right };

        match (end_match, start_match) {
            (Some(end_c), Some(start_c)) => {
                if end_c.is_rtl == self.rtl && start_c.is_rtl != self.rtl {
                    trailing_edge(end_c)
                } else if start_c.is_rtl == self.rtl {
                    leading_edge(start_c)
                } else {
                    trailing_edge(end_c)
                }
            }
            (None, Some(start_c)) => leading_edge(start_c),
            (Some(end_c), None) => trailing_edge(end_c),
            (None, None) => {
                if idx == 0 {
                    if self.rtl { self.line_width } else { 0.0 }
                } else if self.rtl {
                    0.0
                } else {
                    self.line_width
                }
            }
        }
    }

    /// Hit-tests a visual `local_x` (logical pixels from line origin) into the closest valid
    /// UAX #29 logical grapheme byte boundary.
    pub fn hit_test_x(&self, text: &str, local_x: f32) -> usize {
        if text.is_empty() || !local_x.is_finite() || self.clusters.is_empty() {
            return 0;
        }
        let first = &self.clusters[0];
        if local_x <= first.x_left {
            return if first.is_rtl {
                first.byte_end
            } else {
                first.byte_start
            };
        }
        let last = &self.clusters[self.clusters.len() - 1];
        if local_x >= self.line_width.max(last.x_right) {
            return if last.is_rtl {
                last.byte_start
            } else {
                last.byte_end
            };
        }

        let mut best_cluster = first;
        let mut best_dist = f32::INFINITY;
        for c in &self.clusters {
            if local_x >= c.x_left && local_x <= c.x_right {
                best_cluster = c;
                break;
            }
            let dist = if local_x < c.x_left {
                c.x_left - local_x
            } else {
                local_x - c.x_right
            };
            if dist < best_dist {
                best_dist = dist;
                best_cluster = c;
            }
        }

        let mid = (best_cluster.x_left + best_cluster.x_right) * 0.5;
        let chosen = if best_cluster.is_rtl {
            if local_x < mid {
                best_cluster.byte_end
            } else {
                best_cluster.byte_start
            }
        } else if local_x <= mid {
            best_cluster.byte_start
        } else {
            best_cluster.byte_end
        };
        clamp_grapheme_boundary(text, chosen)
    }

    /// Computes visual `(x_left, width)` selection spans for logical range `start..end`.
    ///
    /// Pure LTR/RTL selections produce one contiguous span; mixed BiDi selections that cross
    /// an LTR/RTL reversal produce disjoint visual spans ordered left-to-right.
    pub fn selection_spans_for_range(
        &self,
        text: &str,
        start: usize,
        end: usize,
    ) -> Vec<(f32, f32)> {
        if text.is_empty() || self.clusters.is_empty() {
            return Vec::new();
        }
        let lo = clamp_grapheme_boundary(text, start.min(end));
        let hi = clamp_grapheme_boundary(text, start.max(end));
        if lo >= hi {
            return Vec::new();
        }

        let mut spans: Vec<(f32, f32)> = Vec::new();
        let mut active: Option<(f32, f32)> = None;

        for c in &self.clusters {
            let selected = c.byte_end > lo && c.byte_start < hi;
            if selected {
                active = Some(match active {
                    Some((min_x, max_x)) if c.x_left <= max_x + 0.5 => {
                        (min_x.min(c.x_left), max_x.max(c.x_right))
                    }
                    Some((min_x, max_x)) => {
                        let w = max_x - min_x;
                        if w > 0.0 {
                            spans.push((min_x, w));
                        }
                        (c.x_left, c.x_right)
                    }
                    None => (c.x_left, c.x_right),
                });
            } else if let Some((min_x, max_x)) = active.take() {
                let w = max_x - min_x;
                if w > 0.0 {
                    spans.push((min_x, w));
                }
            }
        }

        if let Some((min_x, max_x)) = active {
            let w = max_x - min_x;
            if w > 0.0 {
                spans.push((min_x, w));
            }
        }

        spans
    }

    /// Steps the caret visually left (`delta < 0`) or right (`delta > 0`) by one grapheme stop
    /// across mixed LTR/RTL runs without ever landing mid-UTF-8 or mid-grapheme.
    pub fn move_caret_visual(&self, text: &str, caret: usize, delta: i32) -> usize {
        let idx = clamp_grapheme_boundary(text, caret);
        if delta == 0 || text.is_empty() {
            return idx;
        }
        if !self.has_rtl() {
            return if delta < 0 {
                prev_grapheme_boundary(text, idx)
            } else {
                next_grapheme_boundary(text, idx)
            };
        }

        let mut stops: Vec<(f32, usize)> = grapheme_boundaries(text)
            .into_iter()
            .map(|b| (self.caret_x_at(text, b), b))
            .collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        let Some(pos) = stops.iter().position(|&(_, b)| b == idx) else {
            return idx;
        };
        let next_pos = if delta < 0 {
            pos.saturating_sub(1)
        } else {
            (pos + 1).min(stops.len() - 1)
        };
        stops[next_pos].1
    }
}

impl CosmicTextEngine {
    /// Shapes a single line of text with full HarfBuzz + Unicode BiDi run ordering and returns
    /// visual left-to-right UAX #29 grapheme cluster geometry.
    pub fn shape_single_line(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
    ) -> ShapedLineLayout {
        if text.is_empty() {
            return ShapedLineLayout {
                rtl: false,
                line_width: 0.0,
                clusters: Vec::new(),
            };
        }

        let fs = sanitize_font_size(font_size);
        let weight = sanitize_font_weight(font_weight);

        if !self.has_system_fonts() {
            return shape_single_line_bitmap_fallback(text, fs);
        }

        let lh = line_height_for(fs);
        let attrs = Self::attrs_for_weight(weight);

        let (rtl, mut line_width, raw_clusters) = {
            let mut font_system = self.lock_font_system();
            let mut buffer = Buffer::new_empty(Metrics::new(fs, lh));
            buffer.set_wrap(Wrap::None);
            buffer.set_size(None, None);
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut font_system, false);

            let Some(run) = buffer.layout_runs().next() else {
                return shape_single_line_bitmap_fallback(text, fs);
            };

            let mut out: Vec<VisualGraphemeCluster> = Vec::with_capacity(run.glyphs.len());
            for glyph in run.glyphs {
                let g_start = glyph.start.min(text.len());
                let g_end = glyph.end.min(text.len()).max(g_start);
                let gr_start = clamp_grapheme_boundary(text, g_start);
                let mut gr_end = clamp_grapheme_boundary(text, g_end);
                if gr_end < g_end || (gr_end == gr_start && gr_start < text.len()) {
                    gr_end = next_grapheme_boundary(text, gr_end);
                }

                let is_rtl = glyph.level.is_rtl();
                let gx0 = glyph.x;
                let gx1 = glyph.x + glyph.w.max(0.0);
                let slice = &text[gr_start..gr_end];
                let sub_graphemes: Vec<(usize, &str)> = slice.grapheme_indices(true).collect();

                if sub_graphemes.len() <= 1 {
                    // Merge multiple glyphs belonging to the same UAX #29 grapheme cluster
                    // (e.g. decomposed `e` + U+0301 or multi-glyph ZWJ emoji sequences).
                    if let Some(existing) = out
                        .iter_mut()
                        .find(|c| c.byte_start == gr_start && c.byte_end == gr_end)
                    {
                        existing.x_left = existing.x_left.min(gx0);
                        existing.x_right = existing.x_right.max(gx1);
                    } else {
                        out.push(VisualGraphemeCluster {
                            byte_start: gr_start,
                            byte_end: gr_end,
                            x_left: gx0,
                            x_right: gx1,
                            is_rtl,
                        });
                    }
                } else {
                    // Subdivide multi-grapheme ligatures (e.g. Latin `fi` or Arabic `لا`)
                    // proportionally in the glyph's visual direction.
                    let count = sub_graphemes.len() as f32;
                    let step = (gx1 - gx0) / count;
                    for (slot, (off, g)) in sub_graphemes.iter().enumerate() {
                        let c_start = gr_start + *off;
                        let c_end = c_start + g.len();
                        let (cx0, cx1) = if is_rtl {
                            let right = gx1 - (slot as f32) * step;
                            (right - step, right)
                        } else {
                            let left = gx0 + (slot as f32) * step;
                            (left, left + step)
                        };
                        out.push(VisualGraphemeCluster {
                            byte_start: c_start,
                            byte_end: c_end,
                            x_left: cx0,
                            x_right: cx1,
                            is_rtl,
                        });
                    }
                }
            }

            (run.rtl, run.line_w, out)
        };

        let mut clusters = raw_clusters;
        // Sort left-to-right by visual X coordinate (stable preserves HarfBuzz visual tie order).
        clusters.sort_by(|a, b| a.x_left.total_cmp(&b.x_left));

        // Preserve visible advance for trailing ASCII spaces if a font/shaper drops them.
        let space_w = (self.measure_text("a a", fs, None).0 - self.measure_text("aa", fs, None).0)
            .max(fs * 0.28);
        for c in &mut clusters {
            if &text[c.byte_start..c.byte_end] == " " && (c.x_right - c.x_left) <= 0.01 {
                c.x_right = c.x_left + space_w;
            }
        }
        for c in &clusters {
            line_width = line_width.max(c.x_right);
        }

        ShapedLineLayout {
            rtl,
            line_width,
            clusters,
        }
    }
}

fn shape_single_line_bitmap_fallback(text: &str, font_size: f32) -> ShapedLineLayout {
    let mut clusters = Vec::new();
    let mut x = 0.0f32;
    let space_w = (font_size * 0.35).max(3.0);
    for (idx, g) in text.grapheme_indices(true) {
        let w = if g == " " {
            space_w
        } else {
            BitmapFontTextMeasurer
                .measure(g, font_size, None)
                .0
                .max(space_w)
        };
        clusters.push(VisualGraphemeCluster {
            byte_start: idx,
            byte_end: idx + g.len(),
            x_left: x,
            x_right: x + w,
            is_rtl: false,
        });
        x += w;
    }
    ShapedLineLayout {
        rtl: false,
        line_width: x,
        clusters,
    }
}
