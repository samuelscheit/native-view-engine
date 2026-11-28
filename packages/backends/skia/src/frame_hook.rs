//! Generic per-frame animation hook ([`SkiaFrameHook`]) letting capability modules (e.g.
//! `nve-motion-skia`) override paint values on the Skia frame clock without `nve-backend-skia`
//! depending on them.

use nve_backend_headless::HeadlessBackend;
use nve_tables::snapshots::VisualSnapshot;
use nve_tables::transform::Transform2D;
use nve_layout::StoreLayoutEngine;
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::SkiaBackend;

/// Resolved (pre-animation) paint targets of a node: what it settles to once animations end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeTargets {
    pub visual: VisualSnapshot,
    pub transform: Transform2D,
}

/// Resolves [`NodeTargets`] for a live node on demand.
pub type TargetsOf<'a> = &'a dyn Fn(NodeId) -> NodeTargets;

/// Stateful animation plugin driven by [`crate::SkiaBackend`]'s frame clock.
///
/// The backend reports commits and interaction-driven target changes; the hook owns all
/// animation state and overrides visual values while animations run.
pub trait SkiaFrameHook: Send + 'static {
    /// Registers every table this hook reads in the process-global static table registry.
    ///
    /// Called by [`crate::SkiaBackend::register_frame_hook`] before any frame is applied, so a
    /// wire replica (e.g. a hot-swapped cdylib host) maps the hook's table declarations to the
    /// same static slots its typed lookups use instead of fresh dynamic slots.
    fn register_tables(&self);

    /// Reconciles after a committed frame (`changes`).
    fn after_commit(&mut self, store: &NodeStore, changes: &ChangeSet, targets: TargetsOf<'_>);

    /// Reconciles nodes whose resolved targets changed without a commit (pointer/focus state).
    fn sync_nodes(&mut self, store: &NodeStore, dirty: &[NodeId], targets: TargetsOf<'_>);

    /// Advances by `dt_s` seconds; returns `true` when painted values changed.
    fn advance(&mut self, dt_s: f32) -> bool;

    /// Advances by `dt_s` seconds, pushing nodes whose compositor-only properties (`opacity`,
    /// `transform`) changed into `composite_dirty` and nodes whose painted properties (`background`,
    /// `color`, `border`, `border_radius`) changed into `paint_dirty`. Returns `true` when any
    /// painted value changed.
    ///
    /// Hooks that return `true` while leaving both dirty slices unchanged trigger a conservative
    /// full-scene resync.
    fn advance_collecting(
        &mut self,
        dt_s: f32,
        _composite_dirty: &mut Vec<NodeId>,
        _paint_dirty: &mut Vec<NodeId>,
    ) -> bool {
        self.advance(dt_s)
    }

    /// Whether another frame must be scheduled (`false` lets display links sleep).
    fn needs_next_frame(&self) -> bool;

    /// Applies the user's reduced-motion preference; returns `true` when painted values changed.
    fn set_reduced_motion(&mut self, reduced: bool) -> bool;

    /// Applies the user's reduced-motion preference, collecting affected nodes as in
    /// [`Self::advance_collecting`]. Returns `true` when any painted value changed.
    fn set_reduced_motion_collecting(
        &mut self,
        reduced: bool,
        _composite_dirty: &mut Vec<NodeId>,
        _paint_dirty: &mut Vec<NodeId>,
    ) -> bool {
        self.set_reduced_motion(reduced)
    }

    /// Returns `true` when `changes` mutates a table watched by this hook (e.g. `Motion`) that
    /// requires paint-tier scene classification even without a `Visual` paint effect.
    fn watches_changes(&self, _changes: &ChangeSet) -> bool {
        false
    }

    /// Overrides `style` with in-flight animated values of `node`.
    fn apply_visual(&self, _node: NodeId, _style: &mut VisualSnapshot) {}

    /// Whether the animated text color of `node` must win over inherited typography.
    fn overrides_text_color(&self, _node: NodeId) -> bool {
        false
    }

    /// Animated transform of `node`, given its resolved `target`.
    fn resolve_transform(&self, _node: NodeId, target: Option<Transform2D>) -> Option<Transform2D> {
        target
    }

    /// Drops all animation state (backend reset).
    fn reset(&mut self);
}

/// Ordered [`SkiaFrameHook`]s installed on one backend.
#[derive(Default)]
pub struct SkiaFrameHooks {
    hooks: Vec<Box<dyn SkiaFrameHook>>,
}

impl SkiaFrameHooks {
    pub fn register<H: SkiaFrameHook>(&mut self, hook: H) {
        self.hooks.push(Box::new(hook));
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub(crate) fn after_commit(&mut self, store: &NodeStore, changes: &ChangeSet, t: TargetsOf<'_>) {
        for h in &mut self.hooks {
            h.after_commit(store, changes, t);
        }
    }

    pub(crate) fn sync_nodes(&mut self, store: &NodeStore, dirty: &[NodeId], t: TargetsOf<'_>) {
        for h in &mut self.hooks {
            h.sync_nodes(store, dirty, t);
        }
    }

    /// Advances every hook, collecting composite-dirty and paint-dirty nodes.
    /// Returns `(any_changed, needs_full_sync)`.
    pub(crate) fn advance_collecting(
        &mut self,
        dt_s: f32,
        composite_dirty: &mut Vec<NodeId>,
        paint_dirty: &mut Vec<NodeId>,
    ) -> (bool, bool) {
        let mut any_changed = false;
        let mut needs_full_sync = false;
        for h in &mut self.hooks {
            let c_before = composite_dirty.len();
            let p_before = paint_dirty.len();
            if h.advance_collecting(dt_s, composite_dirty, paint_dirty) {
                any_changed = true;
                if composite_dirty.len() == c_before && paint_dirty.len() == p_before {
                    needs_full_sync = true;
                }
            }
        }
        (any_changed, needs_full_sync)
    }

    pub(crate) fn needs_next_frame(&self) -> bool {
        self.hooks.iter().any(|h| h.needs_next_frame())
    }

    /// Applies reduced-motion preference to every hook, collecting dirty nodes.
    /// Returns `(any_changed, needs_full_sync)`.
    pub(crate) fn set_reduced_motion_collecting(
        &mut self,
        reduced: bool,
        composite_dirty: &mut Vec<NodeId>,
        paint_dirty: &mut Vec<NodeId>,
    ) -> (bool, bool) {
        let mut any_changed = false;
        let mut needs_full_sync = false;
        for h in &mut self.hooks {
            let c_before = composite_dirty.len();
            let p_before = paint_dirty.len();
            if h.set_reduced_motion_collecting(reduced, composite_dirty, paint_dirty) {
                any_changed = true;
                if composite_dirty.len() == c_before && paint_dirty.len() == p_before {
                    needs_full_sync = true;
                }
            }
        }
        (any_changed, needs_full_sync)
    }

    #[inline]
    pub(crate) fn watches_changes(&self, changes: &ChangeSet) -> bool {
        self.hooks.iter().any(|h| h.watches_changes(changes))
    }

    #[inline]
    pub(crate) fn apply_visual(&self, node: NodeId, style: &mut VisualSnapshot) {
        for h in &self.hooks {
            h.apply_visual(node, style);
        }
    }

    #[inline]
    pub(crate) fn overrides_text_color(&self, node: NodeId) -> bool {
        self.hooks.iter().any(|h| h.overrides_text_color(node))
    }

    #[inline]
    pub(crate) fn resolve_transform(
        &self,
        node: NodeId,
        target: Option<Transform2D>,
    ) -> Option<Transform2D> {
        self.hooks
            .iter()
            .fold(target, |tf, h| h.resolve_transform(node, tf))
    }

    pub(crate) fn reset(&mut self) {
        for h in &mut self.hooks {
            h.reset();
        }
    }
}

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    /// Installs a per-frame animation hook (e.g. `nve_motion_skia::MotionDriver`).
    pub fn register_frame_hook<H: SkiaFrameHook>(&mut self, hook: H) -> &mut Self {
        hook.register_tables();
        self.frame_hooks.register(hook);
        self
    }

    /// Whether any hook still animates (`false` lets the host's display link sleep).
    #[inline]
    pub fn needs_next_frame(&self) -> bool {
        self.frame_hooks.needs_next_frame()
    }

    /// Applies the reduced-motion preference to every hook, incrementally updating the retained
    /// scene when values changed.
    pub fn set_reduced_motion(&mut self, store: &NodeStore, reduced: bool) {
        let mut composite_dirty = std::mem::take(&mut self.motion_scratch);
        let mut paint_dirty = std::mem::take(&mut self.motion_paint_scratch);
        composite_dirty.clear();
        paint_dirty.clear();
        let (changed, needs_full) = self.frame_hooks.set_reduced_motion_collecting(
            reduced,
            &mut composite_dirty,
            &mut paint_dirty,
        );
        if changed {
            if needs_full {
                self.sync_full_scene(store);
            } else {
                if !composite_dirty.is_empty() {
                    self.update_scene_composite_nodes(store, &composite_dirty);
                }
                if !paint_dirty.is_empty() {
                    self.update_scene_paint_nodes(store, &paint_dirty);
                }
            }
        }
        self.motion_scratch = composite_dirty;
        self.motion_paint_scratch = paint_dirty;
    }

    /// Effective visual of `node` including in-flight animated values.
    pub fn animated_style(&self, store: &NodeStore, node: NodeId) -> Option<VisualSnapshot> {
        if !store.is_live(node) {
            return None;
        }
        let mut style = self.layout_tree.effective_visual(store, node);
        self.frame_hooks.apply_visual(node, &mut style);
        Some(style)
    }

    /// Effective 2D transform of `node` including in-flight animated overrides.
    pub fn animated_transform(&self, store: &NodeStore, node: NodeId) -> Option<Transform2D> {
        if !store.is_live(node) {
            return None;
        }
        self.frame_hooks
            .resolve_transform(node, self.layout_tree.effective_transform(store, node))
            .or(Some(Transform2D::IDENTITY))
    }

    /// Advances every hook by `dt_s`, applying zero-alloc composite or incremental paint updates
    /// when values changed; returns whether another frame is needed.
    pub fn advance_animations(&mut self, store: &NodeStore, dt_s: f32) -> bool {
        let mut composite_dirty = std::mem::take(&mut self.motion_scratch);
        let mut paint_dirty = std::mem::take(&mut self.motion_paint_scratch);
        composite_dirty.clear();
        paint_dirty.clear();
        let (stepped, needs_full) = self.frame_hooks.advance_collecting(
            dt_s,
            &mut composite_dirty,
            &mut paint_dirty,
        );
        if stepped {
            if needs_full {
                self.sync_full_scene(store);
            } else {
                if !composite_dirty.is_empty() {
                    self.update_scene_composite_nodes(store, &composite_dirty);
                }
                if !paint_dirty.is_empty() {
                    self.update_scene_paint_nodes(store, &paint_dirty);
                }
            }
        }
        self.motion_scratch = composite_dirty;
        self.motion_paint_scratch = paint_dirty;
        self.frame_hooks.needs_next_frame()
    }

    pub(crate) fn sync_frame_hooks(&mut self, store: &NodeStore, dirty: &[NodeId]) {
        if self.frame_hooks.is_empty() {
            return;
        }
        let tree = &self.layout_tree;
        let targets = |id| node_targets(tree, store, id);
        self.frame_hooks.sync_nodes(store, dirty, &targets);
    }

    pub(crate) fn frame_hooks_after_commit(&mut self, store: &NodeStore, changes: &ChangeSet) {
        if self.frame_hooks.is_empty() {
            return;
        }
        let tree = &self.layout_tree;
        let targets = |id| node_targets(tree, store, id);
        self.frame_hooks.after_commit(store, changes, &targets);
    }
}

fn node_targets<L: StoreLayoutEngine>(
    tree: &HeadlessBackend<L>,
    store: &NodeStore,
    id: NodeId,
) -> NodeTargets {
    NodeTargets {
        visual: tree.effective_visual(store, id),
        transform: tree
            .effective_transform(store, id)
            .unwrap_or(Transform2D::IDENTITY),
    }
}
