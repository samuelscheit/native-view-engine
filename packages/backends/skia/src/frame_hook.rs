//! Generic per-frame animation hook ([`SkiaFrameHook`]) letting capability modules (e.g.
//! `nve-motion-skia`) override paint values on the Skia frame clock without `nve-backend-skia`
//! depending on them.

use nve_backend_headless::HeadlessBackend;
use nve_tables::snapshots::VisualSnapshot;
use nve_tables::transform::Transform2D;
use nve_engine::StoreLayoutEngine;
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
    /// Reconciles after a committed frame (`changes`).
    fn after_commit(&mut self, store: &NodeStore, changes: &ChangeSet, targets: TargetsOf<'_>);

    /// Reconciles nodes whose resolved targets changed without a commit (pointer/focus state).
    fn sync_nodes(&mut self, store: &NodeStore, dirty: &[NodeId], targets: TargetsOf<'_>);

    /// Advances by `dt_s` seconds; returns `true` when painted values changed.
    fn advance(&mut self, dt_s: f32) -> bool;

    /// Whether another frame must be scheduled (`false` lets display links sleep).
    fn needs_next_frame(&self) -> bool;

    /// Applies the user's reduced-motion preference; returns `true` when painted values changed.
    fn set_reduced_motion(&mut self, reduced: bool) -> bool;

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

    pub(crate) fn advance(&mut self, dt_s: f32) -> bool {
        self.hooks.iter_mut().fold(false, |changed, h| h.advance(dt_s) | changed)
    }

    pub(crate) fn needs_next_frame(&self) -> bool {
        self.hooks.iter().any(|h| h.needs_next_frame())
    }

    pub(crate) fn set_reduced_motion(&mut self, reduced: bool) -> bool {
        self.hooks
            .iter_mut()
            .fold(false, |changed, h| h.set_reduced_motion(reduced) | changed)
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
        self.frame_hooks.register(hook);
        self
    }

    /// Whether any hook still animates (`false` lets the host's display link sleep).
    #[inline]
    pub fn needs_next_frame(&self) -> bool {
        self.frame_hooks.needs_next_frame()
    }

    /// Applies the reduced-motion preference to every hook, repainting when values changed.
    pub fn set_reduced_motion(&mut self, store: &NodeStore, reduced: bool) {
        if self.frame_hooks.set_reduced_motion(reduced) {
            self.rebuild_display_list(store);
        }
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

    /// Advances every hook by `dt_s`, repainting when values changed; returns whether another
    /// frame is needed.
    pub fn advance_animations(&mut self, store: &NodeStore, dt_s: f32) -> bool {
        if self.frame_hooks.advance(dt_s) {
            self.rebuild_display_list(store);
        }
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
