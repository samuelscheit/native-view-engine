//! Spatial-navigation geometry: rectangles, directional candidate scoring, wrap ranking, and
//! safe-frame scroll-into-view math.
//!
//! Scoring combines Android TV `FocusFinder`, W3C CSS Spatial Navigation, and tvOS focus-guide
//! heuristics: strict half-plane filtering along the primary axis, an orthogonal-overlap tier
//! (cone bonus), weighted primary-gap + orthogonal-misalignment score, then Euclidean distance and
//! node id as deterministic tie-breakers.

use std::cmp::Ordering;

use nve_store::NodeId;

use crate::action::FocusDirection;

const EPS: f32 = 1e-3;
const SCROLL_EPS: f32 = 1e-4;

/// Axis-aligned 2D layout rectangle in logical surface coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpatialRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl SpatialRect {
    #[inline]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Returns `true` iff all coordinates are finite and both dimensions are strictly positive.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }

    #[inline]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    #[inline]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    #[inline]
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    /// Returns `true` iff two valid rectangles have strictly overlapping interior area.
    #[inline]
    pub fn intersects(&self, other: &Self) -> bool {
        self.is_valid()
            && other.is_valid()
            && self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    /// Returns `true` iff `(px, py)` lies within or on the boundary of a valid rectangle.
    #[inline]
    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        self.is_valid()
            && px.is_finite()
            && py.is_finite()
            && px >= self.x
            && px <= self.right()
            && py >= self.y
            && py <= self.bottom()
    }
}

/// Scrollable viewport geometry and overscan/safe-frame margin used for auto-scroll calculations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollRegion {
    pub viewport_origin_x: f32,
    pub viewport_origin_y: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub content_width: f32,
    pub content_height: f32,
    pub margin_px: f32,
}

/// Computed scroll offset adjustment bringing a focused node + safe margin into view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollAdjustment {
    pub target_offset_x: f32,
    pub target_offset_y: f32,
    pub changed: bool,
}

impl ScrollRegion {
    /// Computes a clamped scroll offset bringing `rect` + `margin_px` into the viewport, or
    /// `None` for non-finite / degenerate geometry.
    pub fn scroll_into_view(&self, rect: &SpatialRect) -> Option<ScrollAdjustment> {
        let finite = [
            self.viewport_origin_x,
            self.viewport_origin_y,
            self.viewport_width,
            self.viewport_height,
            self.content_width,
            self.content_height,
        ]
        .iter()
        .all(|v| v.is_finite());
        if !rect.is_valid() || !finite || self.viewport_width <= 0.0 || self.viewport_height <= 0.0
        {
            return None;
        }

        let margin = if self.margin_px.is_finite() {
            self.margin_px.max(0.0)
        } else {
            0.0
        };
        let max_x = (self.content_width - self.viewport_width).max(0.0);
        let max_y = (self.content_height - self.viewport_height).max(0.0);
        let cur_x = self.viewport_origin_x.clamp(0.0, max_x);
        let cur_y = self.viewport_origin_y.clamp(0.0, max_y);

        let target_offset_x = adjust_scroll_axis(
            rect.x,
            rect.width,
            cur_x,
            self.viewport_width,
            max_x,
            margin,
        );
        let target_offset_y = adjust_scroll_axis(
            rect.y,
            rect.height,
            cur_y,
            self.viewport_height,
            max_y,
            margin,
        );
        let changed = (target_offset_x - self.viewport_origin_x).abs() > SCROLL_EPS
            || (target_offset_y - self.viewport_origin_y).abs() > SCROLL_EPS;

        Some(ScrollAdjustment {
            target_offset_x,
            target_offset_y,
            changed,
        })
    }
}

fn adjust_scroll_axis(
    item_start: f32,
    item_size: f32,
    cur_offset: f32,
    viewport_size: f32,
    max_offset: f32,
    margin: f32,
) -> f32 {
    let item_end = item_start + item_size;
    if item_size >= viewport_size {
        if item_start <= cur_offset && item_end >= cur_offset + viewport_size {
            return cur_offset;
        }
        return item_start.clamp(0.0, max_offset);
    }

    let effective_margin = margin.min((viewport_size - item_size) * 0.5);
    let desired_start = item_start - effective_margin;
    let desired_end = item_end + effective_margin;

    let next = if desired_start < cur_offset {
        desired_start
    } else if desired_end > cur_offset + viewport_size {
        desired_end - viewport_size
    } else {
        cur_offset
    };

    next.clamp(0.0, max_offset)
}

/// Directional candidate rank; lower is better.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CandidateRank {
    tier: u8,
    score: f32,
    euclidean_sq: f32,
    id: NodeId,
}

impl CandidateRank {
    #[inline]
    pub(crate) fn beats(&self, other: &Self) -> bool {
        self.tier
            .cmp(&other.tier)
            .then_with(|| self.score.total_cmp(&other.score))
            .then_with(|| self.euclidean_sq.total_cmp(&other.euclidean_sq))
            .then_with(|| self.id.cmp(&other.id))
            == Ordering::Less
    }
}

/// Wrap-around candidate rank (far edge along the primary axis); lower is better.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WrapRank {
    tier: u8,
    primary_extreme: f32,
    ortho_center_delta: f32,
    id: NodeId,
}

impl WrapRank {
    #[inline]
    pub(crate) fn beats(&self, other: &Self) -> bool {
        self.tier
            .cmp(&other.tier)
            .then_with(|| self.primary_extreme.total_cmp(&other.primary_extreme))
            .then_with(|| self.ortho_center_delta.total_cmp(&other.ortho_center_delta))
            .then_with(|| self.id.cmp(&other.id))
            == Ordering::Less
    }
}

/// Rectangle projected onto (primary, orthogonal) axes so every direction scores as "positive".
#[derive(Debug, Clone, Copy)]
struct ProjectedSpans {
    p0: f32,
    p1: f32,
    pc: f32,
    o0: f32,
    o1: f32,
    oc: f32,
}

#[inline]
fn projected_spans(rect: &SpatialRect, direction: FocusDirection) -> ProjectedSpans {
    let (cx, cy) = rect.center();
    match direction {
        FocusDirection::Right => ProjectedSpans {
            p0: rect.x,
            p1: rect.right(),
            pc: cx,
            o0: rect.y,
            o1: rect.bottom(),
            oc: cy,
        },
        FocusDirection::Left => ProjectedSpans {
            p0: -rect.right(),
            p1: -rect.x,
            pc: -cx,
            o0: rect.y,
            o1: rect.bottom(),
            oc: cy,
        },
        FocusDirection::Down => ProjectedSpans {
            p0: rect.y,
            p1: rect.bottom(),
            pc: cy,
            o0: rect.x,
            o1: rect.right(),
            oc: cx,
        },
        FocusDirection::Up => ProjectedSpans {
            p0: -rect.bottom(),
            p1: -rect.y,
            pc: -cy,
            o0: rect.x,
            o1: rect.right(),
            oc: cx,
        },
    }
}

#[inline]
fn span_overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

#[inline]
fn span_gap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (b0 - a1).max(a0 - b1).max(0.0)
}

/// Ranks `cand_rect` as a `direction` move target from `cur_rect`, or `None` when it does not lie
/// in the directional half-plane.
pub(crate) fn score_candidate(
    cur_rect: &SpatialRect,
    cand_id: NodeId,
    cand_rect: &SpatialRect,
    direction: FocusDirection,
) -> Option<CandidateRank> {
    let cur = projected_spans(cur_rect, direction);
    let cand = projected_spans(cand_rect, direction);

    let primary_center_delta = cand.pc - cur.pc;
    let primary_edge_gap = cand.p0 - cur.p1;
    let leading_advances = cand.p0 > cur.p0 + EPS;
    let trailing_advances = cand.p1 > cur.p1 + EPS;

    if primary_center_delta <= EPS {
        return None;
    }
    if primary_edge_gap < -EPS && !(leading_advances && trailing_advances) {
        return None;
    }

    let ortho_overlap = span_overlap(cur.o0, cur.o1, cand.o0, cand.o1);
    let ortho_edge_gap = span_gap(cur.o0, cur.o1, cand.o0, cand.o1);
    let ortho_center_delta = (cand.oc - cur.oc).abs();
    let primary_dist = if primary_edge_gap >= 0.0 {
        primary_edge_gap
    } else {
        primary_center_delta
    };

    let (tier, score) = if ortho_overlap > 0.0 {
        (0u8, primary_dist + 0.5 * ortho_center_delta)
    } else {
        (
            1u8,
            primary_dist + 2.0 * ortho_edge_gap + 0.5 * ortho_center_delta,
        )
    };

    let (cx, cy) = cur_rect.center();
    let (tx, ty) = cand_rect.center();
    let (dx, dy) = (tx - cx, ty - cy);

    Some(CandidateRank {
        tier,
        score,
        euclidean_sq: dx * dx + dy * dy,
        id: cand_id,
    })
}

/// Ranks `cand_rect` as a wrap-around target: prefers orthogonal overlap, then the far edge.
pub(crate) fn wrap_rank(
    cur_rect: &SpatialRect,
    cand_id: NodeId,
    cand_rect: &SpatialRect,
    direction: FocusDirection,
) -> WrapRank {
    let cur = projected_spans(cur_rect, direction);
    let c = projected_spans(cand_rect, direction);
    WrapRank {
        tier: u8::from(span_overlap(cur.o0, cur.o1, c.o0, c.o1) <= 0.0),
        primary_extreme: c.p0,
        ortho_center_delta: (c.oc - cur.oc).abs(),
        id: cand_id,
    }
}
