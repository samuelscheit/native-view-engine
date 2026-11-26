//! Lightweight zero-idle-CPU [`MotionDriver`] for [`crate::SkiaBackend`].
//!
//! Smoothly interpolates visual [`StyleProps`] changes (`background`, `text_color`, `border_color`,
//! `border_radius`, `opacity`) driven by `InteractionState` transitions (`:hover`, `:active`,
//! `:focus`, `:focus-visible`, `:disabled`) or style mutations whenever a node declares matching
//! [`TransitionSpec`]s. Snaps immediately under [`MotionPreference::Reduce`] and reports
//! `needs_next_frame() == false` the instant all tracks settle.

use std::collections::HashMap;

use nve_abi::{NodeId, StyleProps};
use nve_components::motion::{
    lerp_style_property, resolve_transition_for, MotionPreference, TimedTransition,
    TransitionProperty, TransitionSpec,
};
use nve_components::transform::{lerp_transform_2d, Transform2D};
use nve_engine::LayoutEngine;

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
    from: StyleProps,
    to: StyleProps,
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

    fn apply_to(&self, style: &mut StyleProps) {
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
fn prop_differs(a: &StyleProps, b: &StyleProps, prop: TransitionProperty) -> bool {
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
    transitions: HashMap<NodeId, Vec<TransitionSpec>>,
    last_target: HashMap<NodeId, StyleProps>,
    active: HashMap<NodeId, NodeMotion>,
    last_transform: HashMap<NodeId, Transform2D>,
    active_transform: HashMap<NodeId, TransformAnim>,
    preference: MotionPreference,
}

impl MotionDriver {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn preference(&self) -> MotionPreference {
        self.preference
    }

    /// Updates [`MotionPreference`]; when switching to [`MotionPreference::Reduce`], snaps any
    /// active reduced-motion-instant tracks immediately. Returns `true` if any track snapped.
    pub fn set_preference(&mut self, preference: MotionPreference) -> bool {
        self.preference = preference;
        if !preference.is_reduced() || (self.active.is_empty() && self.active_transform.is_empty()) {
            return false;
        }
        let mut snapped = false;
        self.active.retain(|_, motion| {
            for slot in &mut motion.channels {
                if let Some(ch) = slot
                    && ch.timer.reduced_motion_instant
                {
                    *slot = None;
                    snapped = true;
                }
            }
            !motion.is_empty()
        });
        self.active_transform.retain(|_, anim| {
            if anim.timer.reduced_motion_instant {
                snapped = true;
                false
            } else {
                true
            }
        });
        snapped
    }

    /// Returns `true` iff at least one visual style or transform transition is mid-flight.
    #[inline]
    pub fn needs_next_frame(&self) -> bool {
        !self.active.is_empty() || !self.active_transform.is_empty()
    }

    /// Returns `true` when `id` currently has registered transition specs.
    #[inline]
    pub fn has_transitions(&self, id: NodeId) -> bool {
        self.transitions.contains_key(&id)
    }

    /// Returns the number of nodes with active in-flight transitions.
    #[inline]
    pub fn active_node_count(&self) -> usize {
        self.active.len()
            + self
                .active_transform
                .keys()
                .filter(|id| !self.active.contains_key(id))
                .count()
    }

    /// Sets or clears declarative [`TransitionSpec`]s for `id`.
    pub fn set_node_transitions(&mut self, id: NodeId, specs: Vec<TransitionSpec>) {
        if specs.is_empty() {
            self.transitions.remove(&id);
            self.last_target.remove(&id);
            self.active.remove(&id);
            self.last_transform.remove(&id);
            self.active_transform.remove(&id);
        } else {
            self.transitions.insert(id, specs);
        }
    }

    /// Seeds the baseline target style for `id` if it has transitions and no baseline yet.
    pub fn seed_baseline(&mut self, id: NodeId, style: StyleProps) {
        if self.transitions.contains_key(&id) {
            self.last_target.entry(id).or_insert(style);
        }
    }

    /// Seeds the baseline [`Transform2D`] for `id` if it has transitions and no baseline yet.
    pub fn seed_transform_baseline(&mut self, id: NodeId, transform: Transform2D) {
        if self.transitions.contains_key(&id) {
            self.last_transform.entry(id).or_insert(transform);
        }
    }

    /// Resolves the currently animated [`Transform2D`] for `id`, falling back to `base`.
    #[inline]
    pub fn resolve_animated_transform(
        &self,
        id: NodeId,
        base: Option<Transform2D>,
    ) -> Option<Transform2D> {
        if let Some(anim) = self.active_transform.get(&id) {
            Some(lerp_transform_2d(&anim.from, &anim.to, anim.timer.progress()))
        } else {
            base
        }
    }

    /// Synchronizes a node's newly resolved target [`Transform2D`], starting or smoothly
    /// interrupting a `TransitionProperty::Transform` animation when configured.
    pub fn sync_transform_target(&mut self, id: NodeId, target: Transform2D) {
        let Some(specs) = self.transitions.get(&id) else {
            self.active_transform.remove(&id);
            self.last_transform.remove(&id);
            return;
        };
        let Some(prev_target) = self.last_transform.insert(id, target) else {
            return;
        };
        let current = self
            .active_transform
            .get(&id)
            .map_or(prev_target, |a| lerp_transform_2d(&a.from, &a.to, a.timer.progress()));
        if !transform_differs(&current, &target) {
            self.active_transform.remove(&id);
            return;
        }
        let Some(spec) = resolve_transition_for(specs, TransitionProperty::Transform) else {
            self.active_transform.remove(&id);
            return;
        };
        let instant_reduced = self.preference.is_reduced() && spec.reduced_motion_instant;
        if instant_reduced || (spec.duration_ms == 0 && spec.delay_ms == 0) {
            self.active_transform.remove(&id);
            return;
        }
        self.active_transform.insert(
            id,
            TransformAnim {
                from: current,
                to: target,
                timer: TimedTransition::new(0.0, 1.0, &spec),
            },
        );
    }

    /// Applies any active interpolated visual overrides for `id` onto `style`.
    /// Zero hash lookups when idle (`self.active.is_empty()`).
    #[inline]
    pub fn apply_animated_style(&self, id: NodeId, style: &mut StyleProps) {
        if self.active.is_empty() {
            return;
        }
        if let Some(motion) = self.active.get(&id) {
            motion.apply_to(style);
        }
    }

    /// Synchronizes a node's newly resolved target [`StyleProps`], starting or smoothly
    /// interrupting per-property transitions from the exact currently rendered visual state.
    pub fn sync_target(&mut self, id: NodeId, target: StyleProps) {
        let Some(specs) = self.transitions.get(&id) else {
            self.active.remove(&id);
            self.last_target.remove(&id);
            return;
        };

        let Some(prev_target) = self.last_target.insert(id, target) else {
            // Initial mount baseline: do not animate in from default unstyled state.
            return;
        };

        let mut current = prev_target;
        if let Some(existing) = self.active.get(&id) {
            existing.apply_to(&mut current);
        }

        let mut next_motion = self.active.remove(&id).unwrap_or_default();
        for (idx, prop) in VISUAL_PROPS.into_iter().enumerate() {
            if !prop_differs(&current, &target, prop) {
                next_motion.channels[idx] = None;
                continue;
            }
            let Some(spec) = resolve_transition_for(specs, prop) else {
                next_motion.channels[idx] = None;
                continue;
            };
            let instant_reduced = self.preference.is_reduced() && spec.reduced_motion_instant;
            if instant_reduced || (spec.duration_ms == 0 && spec.delay_ms == 0) {
                next_motion.channels[idx] = None;
                continue;
            }
            next_motion.channels[idx] = Some(ChannelAnim {
                from: current,
                to: target,
                timer: TimedTransition::new(0.0, 1.0, &spec),
            });
        }

        if !next_motion.is_empty() {
            self.active.insert(id, next_motion);
        }
    }

    /// Advances all active transitions by `dt_s` seconds, dropping settled nodes.
    /// Returns `true` if any active transition stepped and the display list needs a rebuild.
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

    /// Removes transition metadata and active animations for nodes that were removed from the tree.
    pub fn retain_nodes(&mut self, mut keep: impl FnMut(NodeId) -> bool) {
        self.transitions.retain(|&id, _| keep(id));
        self.last_target.retain(|&id, _| keep(id));
        self.active.retain(|&id, _| keep(id));
        self.last_transform.retain(|&id, _| keep(id));
        self.active_transform.retain(|&id, _| keep(id));
    }
}

impl<L: LayoutEngine> SkiaBackend<L> {
    /// Returns a read-only handle to the backend's [`MotionDriver`].
    #[inline]
    pub fn motion_driver(&self) -> &MotionDriver {
        &self.motion
    }

    /// Returns the active [`MotionPreference`] (`Full` or `Reduce`).
    #[inline]
    pub fn motion_preference(&self) -> MotionPreference {
        self.motion.preference()
    }

    /// Updates [`MotionPreference`]; switching to [`MotionPreference::Reduce`] immediately snaps
    /// any active reduced-motion transitions and rebuilds the display list.
    pub fn set_motion_preference(&mut self, preference: MotionPreference) {
        if self.motion.set_preference(preference) {
            self.rebuild_display_list();
        }
    }

    /// Returns `true` while at least one style transition is still animating, allowing native
    /// display links to drop to 0% idle CPU/GPU once settled.
    #[inline]
    pub fn needs_next_frame(&self) -> bool {
        self.motion.needs_next_frame()
    }

    /// Attaches or clears zero-serialization [`TransitionSpec`]s on `id`.
    pub fn set_node_transitions(&mut self, id: NodeId, transitions: Vec<TransitionSpec>) {
        self.motion.set_node_transitions(id, transitions);
        if let Some(style) = self.layout_tree.effective_style(id) {
            self.motion.seed_baseline(id, style);
        }
        let tf = self
            .layout_tree
            .effective_transform(id)
            .unwrap_or(Transform2D::IDENTITY);
        self.motion.seed_transform_baseline(id, tf);
    }

    /// Returns the currently rendered (interpolated) [`StyleProps`] for `node_id`.
    pub fn animated_style(&self, node_id: NodeId) -> Option<StyleProps> {
        let mut style = self.layout_tree.effective_style(node_id)?;
        self.motion.apply_animated_style(node_id, &mut style);
        Some(style)
    }

    /// Advances active style transitions by `dt_s` seconds, rebuilding the `DisplayList` when
    /// anything stepped. Returns `self.needs_next_frame()`.
    pub fn advance_animations(&mut self, dt_s: f32) -> bool {
        if self.motion.advance(dt_s) {
            self.rebuild_display_list();
        }
        self.motion.needs_next_frame()
    }

    pub(crate) fn sync_motion_targets(&mut self, dirty_ids: &[NodeId]) {
        if self.motion.transitions.is_empty() {
            return;
        }
        for &id in dirty_ids {
            if let Some(target) = self.layout_tree.effective_style(id) {
                self.motion.sync_target(id, target);
            }
            let tf = self
                .layout_tree
                .effective_transform(id)
                .unwrap_or(Transform2D::IDENTITY);
            self.motion.sync_transform_target(id, tf);
        }
    }

    pub(crate) fn reconcile_motion_after_frame(&mut self) {
        let tree = &self.layout_tree;
        self.motion.retain_nodes(|id| tree.node(id).is_some());
        if self.motion.transitions.is_empty() {
            return;
        }
        let tracked: Vec<NodeId> = self.motion.transitions.keys().copied().collect();
        for id in tracked {
            if let Some(target) = self.layout_tree.effective_style(id) {
                self.motion.sync_target(id, target);
            }
            let tf = self
                .layout_tree
                .effective_transform(id)
                .unwrap_or(Transform2D::IDENTITY);
            self.motion.sync_transform_target(id, tf);
        }
    }
}
