//! Deterministic 2D spatial focus engine for TV remotes, gamepads, and keyboard D-Pad navigation.
//!
//! Data layout: focusables live in one dense, reused `Vec<SpatialFocusNode>`; a second dense
//! `Vec<u32>` maps `NodeId::index()` to that arena slot, so lookups are two array reads and a
//! warm [`SpatialFocusEngine::move_focus`] is one branch-light linear scan with zero heap traffic.
//!
//! Behavior:
//! - Explicit per-direction overrides (`next_up` / `next_down` / ...) win over geometry; an
//!   override naming the node itself pins focus in that direction.
//! - Per-group focus memory (bounded LRU): re-entering a shelf restores its previously focused card.
//! - Modal focus traps: a group containing an eligible `traps_focus` node confines movement to
//!   that group; focus left outside an active trap is pulled into it on the next move.
//! - Initial focus prefers nodes inside the viewport (no jump-scroll on scrolled pages).
//! - Optional axis wrap-around and per-action auto-repeat throttling.

use nve_store::NodeId;

use crate::action::{FocusDirection, RemoteAction};
use crate::geometry::{
    CandidateRank, ScrollAdjustment, ScrollRegion, SpatialRect, WrapRank, score_candidate,
    wrap_rank,
};
use crate::throttle::RepeatThrottle;

/// Logical focus group identifier (e.g. sidebar, hero banner, horizontal shelf, modal dialog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpatialFocusGroupId(pub u32);

/// Focusable layout node registered with [`SpatialFocusEngine`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialFocusNode {
    pub id: NodeId,
    pub rect: SpatialRect,
    pub group: Option<SpatialFocusGroupId>,
    pub disabled: bool,
    pub traps_focus: bool,
    /// Explicit neighbors indexed by [`FocusDirection::slot`]; [`NodeId::NULL`] means geometric.
    pub next: [NodeId; 4],
}

impl SpatialFocusNode {
    #[inline]
    pub const fn new(id: NodeId, rect: SpatialRect) -> Self {
        Self {
            id,
            rect,
            group: None,
            disabled: false,
            traps_focus: false,
            next: [NodeId::NULL; 4],
        }
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

    /// Routes `direction` to `target` before any geometric search (Android `nextFocus*`).
    #[inline]
    pub const fn with_next(mut self, direction: FocusDirection, target: NodeId) -> Self {
        self.next[direction.slot()] = target;
        self
    }

    #[inline]
    fn is_eligible(&self) -> bool {
        !self.disabled && self.rect.is_valid()
    }
}

/// Timestamped input fed to [`SpatialFocusEngine::handle_input`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteInput {
    pub action: RemoteAction,
    /// `true` for OS auto-repeat of a held key.
    pub repeat: bool,
    pub timestamp_us: u64,
}

/// Result of [`SpatialFocusEngine::handle_input`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusOutcome {
    /// Focus moved to (or was initialized on) this node.
    Moved(NodeId),
    /// Directional input with no reachable target; focus is unchanged.
    Blocked,
    /// Auto-repeat dropped by the repeat throttle.
    Throttled,
    /// Non-directional action for the application (Select, Back, media, digits, ...).
    Action(RemoteAction),
}

/// Arena slot sentinel for "no node at this index".
const EMPTY_SLOT: u32 = u32::MAX;

/// Groups whose last focused node is remembered; the least recently focused group is evicted
/// beyond this (long-lived apps visit unbounded numbers of dynamic shelves).
pub const GROUP_MEMORY_CAPACITY: usize = 64;

/// Pure-Rust, deterministic 2D geometric spatial focus engine.
#[derive(Debug, Clone, Default)]
pub struct SpatialFocusEngine {
    nodes: Vec<SpatialFocusNode>,
    slot_by_index: Vec<u32>,
    /// Least recently focused group first.
    group_memory: Vec<(SpatialFocusGroupId, NodeId)>,
    focused: Option<NodeId>,
    /// Visible area in node-rect coordinates, preferred by initial focus.
    viewport: Option<SpatialRect>,
    wrap_axis: bool,
    throttle: RepeatThrottle,
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

    #[inline]
    pub fn throttle(&self) -> &RepeatThrottle {
        &self.throttle
    }

    #[inline]
    pub fn throttle_mut(&mut self) -> &mut RepeatThrottle {
        &mut self.throttle
    }

    /// Sets the visible area (same coordinate space as node rects) that initial focus prefers.
    #[inline]
    pub fn set_viewport(&mut self, viewport: Option<SpatialRect>) {
        self.viewport = viewport;
    }

    /// Inserts `node`. A node already registered at the same `NodeId::index()` is replaced
    /// whatever its generation; when the replaced id differs, focus and group memory that
    /// referenced it are cleared. [`NodeId::NULL`] is ignored.
    pub fn register(&mut self, node: SpatialFocusNode) {
        if node.id.is_null() {
            return;
        }
        let index = node.id.index();
        if let Some(pos) = self.slot_at(index) {
            let replaced = std::mem::replace(&mut self.nodes[pos], node).id;
            if replaced != node.id {
                self.forget(replaced);
            }
            return;
        }
        if index >= self.slot_by_index.len() {
            self.slot_by_index.resize(index + 1, EMPTY_SLOT);
        }
        self.slot_by_index[index] = arena_slot(self.nodes.len());
        self.nodes.push(node);
    }

    pub fn register_all<I: IntoIterator<Item = SpatialFocusNode>>(&mut self, nodes: I) {
        for node in nodes {
            self.register(node);
        }
    }

    /// Replaces every registered node while keeping focus, group memory, and buffer capacity
    /// (just-in-time layout snapshots). Stale focus / memory ids are validated lazily (memory of
    /// groups outside the snapshot, e.g. behind an open modal, survives).
    pub fn replace_nodes<I: IntoIterator<Item = SpatialFocusNode>>(&mut self, nodes: I) {
        self.clear_nodes();
        self.register_all(nodes);
    }

    /// Removes a node by id, clearing active focus and group memory referencing it.
    pub fn unregister(&mut self, id: NodeId) -> Option<SpatialFocusNode> {
        let pos = self.position(id)?;
        let removed = self.nodes.swap_remove(pos);
        self.slot_by_index[id.index()] = EMPTY_SLOT;
        if let Some(moved) = self.nodes.get(pos) {
            self.slot_by_index[moved.id.index()] = arena_slot(pos);
        }
        self.forget(id);
        Some(removed)
    }

    /// Drops focus and group memory referencing `id`.
    fn forget(&mut self, id: NodeId) {
        if self.focused == Some(id) {
            self.focused = None;
        }
        self.group_memory
            .retain(|&(_, remembered)| remembered != id);
    }

    /// Removes every node, focus, group memory, and throttle state.
    pub fn clear(&mut self) {
        self.clear_nodes();
        self.focused = None;
        self.group_memory.clear();
        self.throttle.reset();
    }

    #[inline]
    pub fn node(&self, id: NodeId) -> Option<&SpatialFocusNode> {
        self.position(id).map(|pos| &self.nodes[pos])
    }

    /// Registered nodes in arena order (unordered).
    #[inline]
    pub fn nodes(&self) -> &[SpatialFocusNode] {
        &self.nodes
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
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    #[inline]
    pub fn last_focused_in_group(&self, group: SpatialFocusGroupId) -> Option<NodeId> {
        self.group_memory
            .iter()
            .find(|(g, _)| *g == group)
            .map(|&(_, id)| id)
    }

    /// Sets focus to `id` if registered, enabled, and geometrically valid; updates group memory.
    pub fn set_focus(&mut self, id: NodeId) -> bool {
        match self.node(id).copied() {
            Some(node) if node.is_eligible() => {
                self.commit_focus(&node);
                true
            }
            _ => false,
        }
    }

    #[inline]
    pub fn clear_focus(&mut self) {
        self.focused = None;
    }

    /// Moves focus one step in `direction` and returns the newly focused node, or `None` if no
    /// valid navigation target exists in that direction. Zero allocations once group memory has
    /// seen every group.
    pub fn move_focus(&mut self, direction: FocusDirection) -> Option<NodeId> {
        let Some(current) = self
            .focused
            .and_then(|id| self.node(id).copied())
            .filter(|n| n.rect.is_valid())
        else {
            self.focused = None;
            return self.focus_initial();
        };

        let trap = self.active_trap(&current);
        if matches!(trap, FocusTrapFilter::None)
            && self.nodes.iter().any(|n| n.traps_focus && n.is_eligible())
        {
            // Focus sits behind an active trap (e.g. a modal opened over it): enter the trap.
            return self.focus_initial();
        }
        let explicit = current.next[direction.slot()];
        if explicit == current.id {
            return None;
        }
        let target = match self.node(explicit) {
            Some(node) if node.is_eligible() && trap.allows(node) => *node,
            _ => {
                let geometric = match self.best_geometric(&current, direction, trap) {
                    Some(id) => Some(id),
                    None if self.wrap_axis => self.best_wrap(&current, direction, trap),
                    None => None,
                }?;
                self.resolve_group_memory(&current, geometric, trap)?
            }
        };
        self.commit_focus(&target);
        Some(target.id)
    }

    /// Returns `true` when [`Self::handle_input`] would not drop `input` as a throttled repeat.
    #[inline]
    pub fn would_admit(&self, input: &RemoteInput) -> bool {
        self.throttle
            .would_admit(input.action, input.repeat, input.timestamp_us)
    }

    /// Dispatches timestamped input: throttles auto-repeat, moves focus on directional actions,
    /// and hands every other action back to the caller.
    pub fn handle_input(&mut self, input: RemoteInput) -> FocusOutcome {
        if !self
            .throttle
            .admit(input.action, input.repeat, input.timestamp_us)
        {
            return FocusOutcome::Throttled;
        }
        match input.action {
            RemoteAction::Move(direction) => self
                .move_focus(direction)
                .map_or(FocusOutcome::Blocked, FocusOutcome::Moved),
            action => FocusOutcome::Action(action),
        }
    }

    /// Computes a clamped scroll offset bringing `node_id` + `region.margin_px` into view.
    pub fn compute_scroll_into_view(
        &self,
        node_id: NodeId,
        region: ScrollRegion,
    ) -> Option<ScrollAdjustment> {
        region.scroll_into_view(&self.node(node_id)?.rect)
    }

    #[inline]
    fn slot_at(&self, index: usize) -> Option<usize> {
        let slot = *self.slot_by_index.get(index)?;
        (slot != EMPTY_SLOT).then_some(slot as usize)
    }

    /// Arena position of exactly `id` (index and generation).
    #[inline]
    fn position(&self, id: NodeId) -> Option<usize> {
        self.slot_at(id.index())
            .filter(|&pos| self.nodes[pos].id == id)
    }

    fn clear_nodes(&mut self) {
        for node in &self.nodes {
            self.slot_by_index[node.id.index()] = EMPTY_SLOT;
        }
        self.nodes.clear();
    }

    /// Focuses `node` and moves its group to the most recently used end of the bounded memory.
    fn commit_focus(&mut self, node: &SpatialFocusNode) {
        self.focused = Some(node.id);
        let Some(group) = node.group else { return };
        if let Some(last) = self.group_memory.last_mut()
            && last.0 == group
        {
            last.1 = node.id;
            return;
        }
        if let Some(i) = self.group_memory.iter().position(|(g, _)| *g == group) {
            self.group_memory.remove(i);
        } else if self.group_memory.len() >= GROUP_MEMORY_CAPACITY {
            self.group_memory.remove(0);
        }
        self.group_memory.push((group, node.id));
    }

    /// Top-left-most eligible node (trapping nodes only while a trap is active), preferring
    /// nodes that intersect the viewport.
    fn focus_initial(&mut self) -> Option<NodeId> {
        let has_trapping = self.nodes.iter().any(|n| n.is_eligible() && n.traps_focus);
        let viewport = self.viewport;
        let hidden = |n: &SpatialFocusNode| viewport.is_some_and(|v| !v.intersects(&n.rect));
        let initial = self
            .nodes
            .iter()
            .filter(|n| n.is_eligible() && (!has_trapping || n.traps_focus))
            .min_by(|a, b| {
                hidden(a)
                    .cmp(&hidden(b))
                    .then_with(|| a.rect.y.total_cmp(&b.rect.y))
                    .then_with(|| a.rect.x.total_cmp(&b.rect.x))
                    .then_with(|| a.id.cmp(&b.id))
            })
            .copied()?;
        self.commit_focus(&initial);
        Some(initial.id)
    }

    fn active_trap(&self, current: &SpatialFocusNode) -> FocusTrapFilter {
        match current.group {
            Some(g)
                if current.traps_focus
                    || self
                        .nodes
                        .iter()
                        .any(|n| n.group == Some(g) && n.traps_focus && n.is_eligible()) =>
            {
                FocusTrapFilter::Group(g)
            }
            None if current.traps_focus => FocusTrapFilter::TrappingOnly,
            _ => FocusTrapFilter::None,
        }
    }

    #[inline]
    fn candidates<'a>(
        &'a self,
        current: &'a SpatialFocusNode,
        trap: FocusTrapFilter,
    ) -> impl Iterator<Item = &'a SpatialFocusNode> + 'a {
        self.nodes
            .iter()
            .filter(move |c| c.id != current.id && c.is_eligible() && trap.allows(c))
    }

    fn best_geometric(
        &self,
        current: &SpatialFocusNode,
        direction: FocusDirection,
        trap: FocusTrapFilter,
    ) -> Option<NodeId> {
        let mut best: Option<CandidateRank> = None;
        let mut best_id = None;
        for cand in self.candidates(current, trap) {
            if let Some(rank) = score_candidate(&current.rect, cand.id, &cand.rect, direction)
                && best.as_ref().is_none_or(|prev| rank.beats(prev))
            {
                best = Some(rank);
                best_id = Some(cand.id);
            }
        }
        best_id
    }

    fn best_wrap(
        &self,
        current: &SpatialFocusNode,
        direction: FocusDirection,
        trap: FocusTrapFilter,
    ) -> Option<NodeId> {
        let mut best: Option<WrapRank> = None;
        let mut best_id = None;
        for cand in self.candidates(current, trap) {
            let rank = wrap_rank(&current.rect, cand.id, &cand.rect, direction);
            if best.as_ref().is_none_or(|prev| rank.beats(prev)) {
                best = Some(rank);
                best_id = Some(cand.id);
            }
        }
        best_id
    }

    /// Entering a different group restores that group's remembered node when still reachable.
    fn resolve_group_memory(
        &self,
        current: &SpatialFocusNode,
        geometric: NodeId,
        trap: FocusTrapFilter,
    ) -> Option<SpatialFocusNode> {
        let target = *self.node(geometric)?;
        let Some(group) = target.group.filter(|&g| current.group != Some(g)) else {
            return Some(target);
        };
        let remembered = self
            .last_focused_in_group(group)
            .filter(|&id| id != current.id)
            .and_then(|id| self.node(id))
            .filter(|n| n.is_eligible() && n.group == Some(group) && trap.allows(n));
        Some(remembered.copied().unwrap_or(target))
    }
}

/// Arena positions are bounded by the number of distinct 24-bit node indices, so they always fit
/// below [`EMPTY_SLOT`].
#[inline]
fn arena_slot(pos: usize) -> u32 {
    const _: () = assert!(NodeId::MAX_INDEX < EMPTY_SLOT);
    pos as u32
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
