//! Deterministic 2D geometric spatial focus engine for TV remotes, gamepads, and keyboard D-Pad
//! navigation across tvOS, Android TV, Fire TV, and desktop 10-foot UIs.
//!
//! Combines Android TV `FocusFinder`, W3C CSS Spatial Navigation, and tvOS focus-guide heuristics:
//! - Strict directional half-plane filtering along the primary navigation axis.
//! - Direct-line orthogonal overlap preference (cone bonus) with weighted primary-gap + orthogonal
//!   misalignment scoring and deterministic Euclidean-distance + node-ID tie-breaking.
//! - Per-group focus memory (`last_focused_by_group`) so moving from a shelf into a sidebar and
//!   returning restores the exact previously focused card.
//! - Modal/drawer focus trapping (`traps_focus`) and optional axis wrap-around (`wrap_axis`).
//! - Safe-frame scroll-into-view calculation for `ScrollView` / `VirtualList` viewports.

use std::cmp::Ordering;
use std::collections::BTreeMap;

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
        Self { x, y, width, height }
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

pub use crate::overscan::{FocusDirection, RemoteAction};

/// Logical focus group identifier (e.g. sidebar, hero banner, horizontal shelf, modal dialog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpatialFocusGroupId(pub u32);

/// Focusable layout node registered with [`SpatialFocusEngine`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialFocusNode {
    pub id: u64,
    pub rect: SpatialRect,
    pub group: Option<SpatialFocusGroupId>,
    pub disabled: bool,
    pub traps_focus: bool,
}

impl SpatialFocusNode {
    #[inline]
    pub const fn new(id: u64, rect: SpatialRect) -> Self {
        Self { id, rect, group: None, disabled: false, traps_focus: false }
    }

    #[inline]
    pub const fn with_group(mut self, group: SpatialFocusGroupId) -> Self {
        self.group = Some(group);
        self
    }

    #[inline]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[inline]
    pub const fn with_traps_focus(mut self, traps_focus: bool) -> Self {
        self.traps_focus = traps_focus;
        self
    }

    #[inline]
    fn is_eligible(&self) -> bool {
        !self.disabled && self.rect.is_valid()
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

#[derive(Debug, Clone, Copy)]
struct CandidateRank {
    tier: u8,
    score: f32,
    euclidean_sq: f32,
    id: u64,
}

impl CandidateRank {
    fn cmp_rank(&self, other: &Self) -> Ordering {
        self.tier
            .cmp(&other.tier)
            .then_with(|| self.score.total_cmp(&other.score))
            .then_with(|| self.euclidean_sq.total_cmp(&other.euclidean_sq))
            .then_with(|| self.id.cmp(&other.id))
    }
}

#[derive(Debug, Clone, Copy)]
struct WrapRank {
    tier: u8,
    primary_extreme: f32,
    ortho_center_delta: f32,
    id: u64,
}

impl WrapRank {
    fn cmp_rank(&self, other: &Self) -> Ordering {
        self.tier
            .cmp(&other.tier)
            .then_with(|| self.primary_extreme.total_cmp(&other.primary_extreme))
            .then_with(|| self.ortho_center_delta.total_cmp(&other.ortho_center_delta))
            .then_with(|| self.id.cmp(&other.id))
    }
}

/// Pure-Rust, deterministic 2D geometric spatial focus engine.
#[derive(Debug, Clone, Default)]
pub struct SpatialFocusEngine {
    nodes: BTreeMap<u64, SpatialFocusNode>,
    focused_id: Option<u64>,
    last_focused_by_group: BTreeMap<SpatialFocusGroupId, u64>,
    wrap_axis: bool,
}

impl SpatialFocusEngine {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_wrap_axis(mut self, wrap: bool) -> Self {
        self.wrap_axis = wrap;
        self
    }

    #[inline]
    pub fn set_wrap_axis(&mut self, wrap: bool) {
        self.wrap_axis = wrap;
    }

    #[inline]
    pub fn wrap_axis(&self) -> bool {
        self.wrap_axis
    }

    /// Maps standard key names (`"ArrowUp"`, `"Enter"`, `"Select"`, `"Escape"`, `"Back"`,
    /// `"GoBack"`, `"MediaPlayPause"`, `"ContextMenu"`, `"Menu"`, `"ChannelUp"`, `"ChannelDown"`)
    /// to a [`RemoteAction`].
    #[inline]
    pub fn from_key_name(key: &str) -> Option<RemoteAction> {
        RemoteAction::from_key_name(key)
    }

    pub fn register(&mut self, node: SpatialFocusNode) {
        self.nodes.insert(node.id, node);
    }

    pub fn register_all<I: IntoIterator<Item = SpatialFocusNode>>(&mut self, nodes: I) {
        for node in nodes {
            self.register(node);
        }
    }

    /// Removes a node by ID, clearing active focus and stale group memory referencing it.
    pub fn unregister(&mut self, id: u64) -> Option<SpatialFocusNode> {
        let removed = self.nodes.remove(&id);
        if self.focused_id == Some(id) {
            self.focused_id = None;
        }
        self.last_focused_by_group.retain(|_, &mut v| v != id);
        removed
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.focused_id = None;
        self.last_focused_by_group.clear();
    }

    #[inline]
    pub fn node(&self, id: u64) -> Option<&SpatialFocusNode> {
        self.nodes.get(&id)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[inline]
    pub fn focused(&self) -> Option<u64> {
        self.focused_id
    }

    #[inline]
    pub fn last_focused_in_group(&self, group: SpatialFocusGroupId) -> Option<u64> {
        self.last_focused_by_group.get(&group).copied()
    }

    /// Sets focus to `id` if registered, enabled, and geometrically valid; updates group memory.
    pub fn set_focus(&mut self, id: u64) -> bool {
        let Some(node) = self.nodes.get(&id).copied() else {
            return false;
        };
        if !node.is_eligible() {
            return false;
        }
        self.focused_id = Some(id);
        if let Some(g) = node.group {
            self.last_focused_by_group.insert(g, id);
        }
        true
    }

    #[inline]
    pub fn clear_focus(&mut self) {
        self.focused_id = None;
    }

    /// Moves focus one step in `direction` and returns the newly focused node ID, or `None` if no
    /// valid navigation target exists in that direction.
    pub fn move_focus(&mut self, direction: FocusDirection) -> Option<u64> {
        let Some(current) = self
            .focused_id
            .and_then(|id| self.nodes.get(&id).copied())
            .filter(|n| n.rect.is_valid())
        else {
            self.focused_id = None;
            return self.focus_initial();
        };

        let trap = self.active_trap(&current);
        let mut best: Option<(u64, CandidateRank)> = None;

        for (&cand_id, cand) in &self.nodes {
            if cand_id == current.id || !cand.is_eligible() || !trap.allows(cand) {
                continue;
            }
            if let Some(rank) = score_candidate(&current.rect, cand_id, &cand.rect, direction)
                && best
                    .as_ref()
                    .is_none_or(|(_, prev)| rank.cmp_rank(prev) == Ordering::Less)
            {
                best = Some((cand_id, rank));
            }
        }

        let geometric_target = match best {
            Some((id, _)) => Some(id),
            None if self.wrap_axis => self.find_wrap_target(&current, direction, trap),
            None => None,
        }?;

        let final_id = self.resolve_group_memory(&current, geometric_target, trap);
        self.focused_id = Some(final_id);
        if let Some(group) = self.nodes.get(&final_id).and_then(|n| n.group) {
            self.last_focused_by_group.insert(group, final_id);
        }
        Some(final_id)
    }

    /// Dispatches a [`RemoteAction`], updating spatial focus on directional moves.
    pub fn handle_action(&mut self, action: RemoteAction) -> Option<u64> {
        match action {
            RemoteAction::Move(dir) => self.move_focus(dir),
            _ => self.focused_id,
        }
    }

    /// Computes a clamped scroll offset adjustment bringing `node_id` + `margin_px` into view.
    pub fn compute_scroll_into_view(
        &self,
        node_id: u64,
        region: ScrollRegion,
    ) -> Option<ScrollAdjustment> {
        let node = self.nodes.get(&node_id)?;
        if !node.rect.is_valid()
            || !region.viewport_origin_x.is_finite()
            || !region.viewport_origin_y.is_finite()
            || !region.viewport_width.is_finite()
            || !region.viewport_height.is_finite()
            || !region.content_width.is_finite()
            || !region.content_height.is_finite()
            || region.viewport_width <= 0.0
            || region.viewport_height <= 0.0
        {
            return None;
        }

        let margin = if region.margin_px.is_finite() { region.margin_px.max(0.0) } else { 0.0 };
        let max_x = (region.content_width - region.viewport_width).max(0.0);
        let max_y = (region.content_height - region.viewport_height).max(0.0);
        let cur_x = region.viewport_origin_x.clamp(0.0, max_x);
        let cur_y = region.viewport_origin_y.clamp(0.0, max_y);

        let target_offset_x = adjust_scroll_axis(
            node.rect.x,
            node.rect.width,
            cur_x,
            region.viewport_width,
            max_x,
            margin,
        );
        let target_offset_y = adjust_scroll_axis(
            node.rect.y,
            node.rect.height,
            cur_y,
            region.viewport_height,
            max_y,
            margin,
        );
        let changed = (target_offset_x - region.viewport_origin_x).abs() > SCROLL_EPS
            || (target_offset_y - region.viewport_origin_y).abs() > SCROLL_EPS;

        Some(ScrollAdjustment { target_offset_x, target_offset_y, changed })
    }

    fn focus_initial(&mut self) -> Option<u64> {
        let has_trapping = self.nodes.values().any(|n| n.is_eligible() && n.traps_focus);
        let initial = self
            .nodes
            .values()
            .filter(|n| n.is_eligible() && (!has_trapping || n.traps_focus))
            .min_by(|a, b| {
                a.rect
                    .y
                    .total_cmp(&b.rect.y)
                    .then_with(|| a.rect.x.total_cmp(&b.rect.x))
                    .then_with(|| a.id.cmp(&b.id))
            })
            .copied()?;
        self.focused_id = Some(initial.id);
        if let Some(g) = initial.group {
            self.last_focused_by_group.insert(g, initial.id);
        }
        Some(initial.id)
    }

    fn active_trap(&self, current: &SpatialFocusNode) -> FocusTrapFilter {
        if let Some(g) = current.group {
            let group_traps = current.traps_focus
                || self.nodes.values().any(|n| n.group == Some(g) && n.traps_focus);
            if group_traps {
                return FocusTrapFilter::Group(g);
            }
        } else if current.traps_focus {
            return FocusTrapFilter::TrappingOnly;
        }
        FocusTrapFilter::None
    }

    fn resolve_group_memory(
        &self,
        current: &SpatialFocusNode,
        geometric_target: u64,
        trap: FocusTrapFilter,
    ) -> u64 {
        let Some(target_group) = self.nodes.get(&geometric_target).and_then(|n| n.group) else {
            return geometric_target;
        };
        if current.group == Some(target_group) {
            return geometric_target;
        }
        if let Some(&remembered_id) = self.last_focused_by_group.get(&target_group)
            && remembered_id != current.id
            && let Some(remembered) = self.nodes.get(&remembered_id)
            && remembered.is_eligible()
            && remembered.group == Some(target_group)
            && trap.allows(remembered)
        {
            return remembered_id;
        }
        geometric_target
    }

    fn find_wrap_target(
        &self,
        current: &SpatialFocusNode,
        direction: FocusDirection,
        trap: FocusTrapFilter,
    ) -> Option<u64> {
        let cur = projected_spans(&current.rect, direction);
        let mut best: Option<(u64, WrapRank)> = None;

        for (&cand_id, cand) in &self.nodes {
            if cand_id == current.id || !cand.is_eligible() || !trap.allows(cand) {
                continue;
            }
            let c = projected_spans(&cand.rect, direction);
            let rank = WrapRank {
                tier: u8::from(span_overlap(cur.o0, cur.o1, c.o0, c.o1) <= 0.0),
                primary_extreme: c.p0,
                ortho_center_delta: (c.oc - cur.oc).abs(),
                id: cand_id,
            };
            if best
                .as_ref()
                .is_none_or(|(_, prev)| rank.cmp_rank(prev) == Ordering::Less)
            {
                best = Some((cand_id, rank));
            }
        }
        best.map(|(id, _)| id)
    }
}

#[derive(Debug, Clone, Copy)]
enum FocusTrapFilter {
    None,
    Group(SpatialFocusGroupId),
    TrappingOnly,
}

impl FocusTrapFilter {
    #[inline]
    fn allows(self, node: &SpatialFocusNode) -> bool {
        match self {
            Self::None => true,
            Self::Group(g) => node.group == Some(g),
            Self::TrappingOnly => node.traps_focus,
        }
    }
}

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

fn score_candidate(
    cur_rect: &SpatialRect,
    cand_id: u64,
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
    let primary_dist = if primary_edge_gap >= 0.0 { primary_edge_gap } else { primary_center_delta };

    let (tier, score) = if ortho_overlap > 0.0 {
        (0u8, primary_dist + 0.5 * ortho_center_delta)
    } else {
        (1u8, primary_dist + 2.0 * ortho_edge_gap + 0.5 * ortho_center_delta)
    };

    let (cx, cy) = cur_rect.center();
    let (tx, ty) = cand_rect.center();
    let (dx, dy) = (tx - cx, ty - cy);

    Some(CandidateRank { tier, score, euclidean_sq: dx * dx + dy * dy, id: cand_id })
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
