//! Lightweight zero-idle-CPU [`MotionDriver`] for [`crate::SkiaBackend`], reading declarative
//! `Motion` (`TransitionList`) word tables directly from `&NodeStore`.

use std::collections::HashMap;

use nve_abi::NodeId;
use nve_components::motion::{
    lerp_style_property, resolve_transition_for, MotionPreference, TimedTransition,
    TransitionProperty, TransitionSpec,
};
use nve_components::style_patch::VisualSnapshot;
use nve_components::tables::{Motion, Transitions};
use nve_components::transform::{lerp_transform_2d, Transform2D};
use nve_engine::StoreLayoutEngine;
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore, NveTable};

use crate::SkiaBackend;

const VISUAL_PROPS: [TransitionProperty; 5] = [
    TransitionProperty::BackgroundColor,
    TransitionProperty::Color,
    TransitionProperty::BorderColor,
    TransitionProperty::BorderRadius,
    TransitionProperty::Opacity,
];

#[derive(Debug, Clone, Copy)]
struct ChannelAnim {
    from: VisualSnapshot,
    to: VisualSnapshot,
    timer: TimedTransition,
}

#[derive(Debug, Clone, Default)]
struct NodeMotion {
    channels: [Option<ChannelAnim>; 5],
}

impl NodeMotion {
    fn is_empty(&self) -> bool {
        self.channels.iter().all(Option::is_none)
    }

    fn apply_to(&self, style: &mut VisualSnapshot) {
        for (idx, prop) in VISUAL_PROPS.into_iter().enumerate() {
            if let Some(ch) = self.channels[idx] {
                lerp_style_property(&ch.from, &ch.to, prop, ch.timer.progress(), style);
            }
        }
    }

    fn step(&mut self, dt_s: f32, pref: MotionPreference) -> bool {
        let mut stepped = false;
        for slot in &mut self.channels {
            if let Some(ch) = slot {
                stepped = true;
                if ch.timer.step(dt_s, pref) {
                    *slot = None;
                }
            }
        }
        stepped
    }
}

#[inline]
fn prop_differs(a: &VisualSnapshot, b: &VisualSnapshot, prop: TransitionProperty) -> bool {
    match prop {
        TransitionProperty::BackgroundColor => a.background != b.background,
        TransitionProperty::Color => a.text_color != b.text_color,
        TransitionProperty::BorderColor => a.border_color != b.border_color,
        TransitionProperty::BorderRadius => (a.border_radius - b.border_radius).abs() > 1e-4,
        TransitionProperty::Opacity => (a.opacity - b.opacity).abs() > 1e-4,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy)]
struct TransformAnim {
    from: Transform2D,
    to: Transform2D,
    timer: TimedTransition,
}

#[inline]
fn transform_differs(a: &Transform2D, b: &Transform2D) -> bool {
    (a.translate_x - b.translate_x).abs() > 1e-4
        || (a.translate_y - b.translate_y).abs() > 1e-4
        || (a.scale_x - b.scale_x).abs() > 1e-4
        || (a.scale_y - b.scale_y).abs() > 1e-4
        || (a.rotate_deg - b.rotate_deg).abs() > 1e-4
        || (a.origin_x - b.origin_x).abs() > 1e-4
        || (a.origin_y - b.origin_y).abs() > 1e-4
}

/// Manages per-node declarative [`TransitionSpec`] animations for [`SkiaBackend`].
#[derive(Debug, Clone, Default)]
pub struct MotionDriver {
    preference: MotionPreference,
    pub(crate) transitions: HashMap<NodeId, Vec<TransitionSpec>>,
    last_target: HashMap<NodeId, VisualSnapshot>,
    active: HashMap<NodeId, NodeMotion>,
    last_transform: HashMap<NodeId, Transform2D>,
    active_transform: HashMap<NodeId, TransformAnim>,
}

impl MotionDriver {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn preference(&self) -> MotionPreference {
        self.preference
    }

    pub fn set_preference(&mut self, pref: MotionPreference) -> bool {
        if self.preference == pref {
            return false;
        }
        self.preference = pref;
        if pref == MotionPreference::Reduce
            && (!self.active.is_empty() || !self.active_transform.is_empty())
        {
            self.active.clear();
            self.active_transform.clear();
            return true;
        }
        false
    }

    #[inline]
    pub fn needs_next_frame(&self) -> bool {
        !self.active.is_empty() || !self.active_transform.is_empty()
    }

    pub fn set_node_transitions(&mut self, id: NodeId, specs: Vec<TransitionSpec>) {
        if specs.is_empty() {
            self.transitions.remove(&id);
            self.active.remove(&id);
            self.active_transform.remove(&id);
        } else {
            self.transitions.insert(id, specs);
        }
    }

    pub fn seed_baseline(&mut self, id: NodeId, style: VisualSnapshot) {
        self.last_target.entry(id).or_insert(style);
    }

    pub fn seed_transform_baseline(&mut self, id: NodeId, tf: Transform2D) {
        self.last_transform.entry(id).or_insert(tf);
    }

    pub fn apply_animated_style(&self, id: NodeId, style: &mut VisualSnapshot) {
        if let Some(motion) = self.active.get(&id) {
            motion.apply_to(style);
        }
    }

    #[inline]
    pub fn has_active_color(&self, id: NodeId) -> bool {
        self.active
            .get(&id)
            .is_some_and(|motion| motion.channels[1].is_some())
    }

    pub fn resolve_animated_transform(
        &self,
        id: NodeId,
        target: Option<Transform2D>,
    ) -> Option<Transform2D> {
        if let Some(anim) = self.active_transform.get(&id) {
            Some(lerp_transform_2d(&anim.from, &anim.to, anim.timer.progress()))
        } else {
            target
        }
    }

    pub fn sync_transform_target(&mut self, id: NodeId, target: Transform2D) {
        let Some(specs) = self.transitions.get(&id) else {
            self.last_transform.insert(id, target);
            return;
        };
        let Some(prev) = self.last_transform.insert(id, target) else {
            return;
        };
        if !transform_differs(&prev, &target) {
            return;
        }
        let current = self
            .active_transform
            .get(&id)
            .map(|a| lerp_transform_2d(&a.from, &a.to, a.timer.progress()))
            .unwrap_or(prev);
        if let Some(spec) = resolve_transition_for(specs, TransitionProperty::Transform) {
            let mut timer = TimedTransition::new(0.0, 1.0, &spec);
            if !timer.step(0.0, self.preference) {
                self.active_transform.insert(
                    id,
                    TransformAnim {
                        from: current,
                        to: target,
                        timer,
                    },
                );
                return;
            }
        }
        self.active_transform.remove(&id);
    }

    pub fn sync_target(&mut self, id: NodeId, target: VisualSnapshot) {
        let Some(specs) = self.transitions.get(&id) else {
            self.last_target.insert(id, target);
            return;
        };
        let Some(prev) = self.last_target.insert(id, target) else {
            return;
        };
        let mut current = prev;
        if let Some(existing) = self.active.get(&id) {
            existing.apply_to(&mut current);
        }
        let mut next_motion = self.active.remove(&id).unwrap_or_default();
        for (idx, prop) in VISUAL_PROPS.into_iter().enumerate() {
            if !prop_differs(&prev, &target, prop) {
                continue;
            }
            if let Some(spec) = resolve_transition_for(specs, prop) {
                let mut timer = TimedTransition::new(0.0, 1.0, &spec);
                if !timer.step(0.0, self.preference) {
                    next_motion.channels[idx] = Some(ChannelAnim {
                        from: current,
                        to: target,
                        timer,
                    });
                } else {
                    next_motion.channels[idx] = None;
                }
            } else {
                next_motion.channels[idx] = None;
            }
        }
        if !next_motion.is_empty() {
            self.active.insert(id, next_motion);
        }
    }

    pub fn advance(&mut self, dt_s: f32) -> bool {
        if self.active.is_empty() && self.active_transform.is_empty() {
            return false;
        }
        let pref = self.preference;
        let mut changed = false;
        self.active.retain(|_, motion| {
            if motion.step(dt_s, pref) {
                changed = true;
            }
            !motion.is_empty()
        });
        self.active_transform.retain(|_, anim| {
            changed = true;
            !anim.timer.step(dt_s, pref)
        });
        changed
    }

    pub fn retain_nodes(&mut self, mut keep: impl FnMut(NodeId) -> bool) {
        self.transitions.retain(|&id, _| keep(id));
        self.last_target.retain(|&id, _| keep(id));
        self.active.retain(|&id, _| keep(id));
        self.last_transform.retain(|&id, _| keep(id));
        self.active_transform.retain(|&id, _| keep(id));
    }
}

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    #[inline]
    pub fn motion_driver(&self) -> &MotionDriver {
        &self.motion
    }

    #[inline]
    pub fn motion_preference(&self) -> MotionPreference {
        self.motion.preference()
    }

    pub fn set_motion_preference(&mut self, store: &NodeStore, preference: MotionPreference) {
        if self.motion.set_preference(preference) {
            self.rebuild_display_list(store);
        }
    }

    #[inline]
    pub fn needs_next_frame(&self) -> bool {
        self.motion.needs_next_frame()
    }

    pub fn animated_style(&self, store: &NodeStore, node_id: StoreNodeId) -> Option<VisualSnapshot> {
        if !store.is_live(node_id) {
            return None;
        }
        let mut style = self.layout_tree.effective_visual(store, node_id);
        self.motion.apply_animated_style(node_id.raw(), &mut style);
        Some(style)
    }

    pub fn advance_animations(&mut self, store: &NodeStore, dt_s: f32) -> bool {
        if self.motion.advance(dt_s) {
            self.rebuild_display_list(store);
        }
        self.motion.needs_next_frame()
    }

    pub(crate) fn sync_motion_targets(&mut self, store: &NodeStore, dirty_ids: &[NodeId]) {
        if self.motion.transitions.is_empty() {
            return;
        }
        for &raw in dirty_ids {
            let id = StoreNodeId(raw);
            if !store.is_live(id) {
                continue;
            }
            let target = self.layout_tree.effective_visual(store, id);
            self.motion.sync_target(raw, target);
            let tf = self
                .layout_tree
                .effective_transform(store, id)
                .unwrap_or(Transform2D::IDENTITY);
            self.motion.sync_transform_target(raw, tf);
        }
    }

    pub(crate) fn reconcile_motion_after_frame(&mut self, store: &NodeStore, changes: &ChangeSet) {
        self.motion.retain_nodes(|raw| store.is_live(StoreNodeId(raw)));
        // Clean up entries when the `Motion` table is detached from a live node.
        if let Some(tc) = changes.table(Motion::slot()) {
            for &nid in &tc.detached {
                self.motion.set_node_transitions(nid.raw(), Vec::new());
            }
        }
        // Sync any newly added/updated `Motion` table rows into `MotionDriver`.
        for &id in changes.created.iter().chain(changes.touched.iter()) {
            if let Some(m) = store.try_row::<Motion>(id) {
                let res = m.transitions();
                let specs = if !res.is_none() {
                    store
                        .resource::<Transitions>(res)
                        .map(|t| t.0.to_vec())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                self.motion.set_node_transitions(id.raw(), specs);
                let style = self.layout_tree.effective_visual(store, id);
                self.motion.seed_baseline(id.raw(), style);
                let tf = self
                    .layout_tree
                    .effective_transform(store, id)
                    .unwrap_or(Transform2D::IDENTITY);
                self.motion.seed_transform_baseline(id.raw(), tf);
            } else if self.motion.transitions.contains_key(&id.raw()) {
                self.motion.set_node_transitions(id.raw(), Vec::new());
            }
        }
        if self.motion.transitions.is_empty() {
            return;
        }
        let tracked: Vec<NodeId> = self.motion.transitions.keys().copied().collect();
        for raw in tracked {
            let id = StoreNodeId(raw);
            if !store.is_live(id) {
                continue;
            }
            let target = self.layout_tree.effective_visual(store, id);
            self.motion.sync_target(raw, target);
            let tf = self
                .layout_tree
                .effective_transform(store, id)
                .unwrap_or(Transform2D::IDENTITY);
            self.motion.sync_transform_target(raw, tf);
        }
    }
}
