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

/// Manages per-node declarative [`TransitionSpec`] animations for [`SkiaBackend`].
#[derive(Debug, Clone, Default)]
pub struct MotionDriver {
    transitions: HashMap<NodeId, Vec<TransitionSpec>>,
    last_target: HashMap<NodeId, StyleProps>,
    active: HashMap<NodeId, NodeMotion>,
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
        if !preference.is_reduced() || self.active.is_empty() {
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
        snapped
    }

    /// Returns `true` iff at least one visual style transition is mid-flight.
    #[inline]
    pub fn needs_next_frame(&self) -> bool {
        !self.active.is_empty()
    }

    /// Returns the number of nodes with active in-flight transitions.
    #[inline]
    pub fn active_node_count(&self) -> usize {
        self.active.len()
    }

    /// Sets or clears declarative [`TransitionSpec`]s for `id`.
    pub fn set_node_transitions(&mut self, id: NodeId, specs: Vec<TransitionSpec>) {
        if specs.is_empty() {
            self.transitions.remove(&id);
            self.last_target.remove(&id);
            self.active.remove(&id);
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
        if self.active.is_empty() {
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
        changed
    }

    /// Removes transition metadata and active animations for nodes that were removed from the tree.
    pub fn retain_nodes(&mut self, mut keep: impl FnMut(NodeId) -> bool) {
        self.transitions.retain(|&id, _| keep(id));
        self.last_target.retain(|&id, _| keep(id));
        self.active.retain(|&id, _| keep(id));
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
    }

    /// Bulk-synchronizes typed [`TransitionSpec`] slices from any producer (e.g. `DioxusSurface`).
    pub fn sync_transitions_from<'a>(
        &mut self,
        transitions: impl IntoIterator<Item = (NodeId, &'a [TransitionSpec])>,
    ) {
        let map: HashMap<NodeId, &'a [TransitionSpec]> = transitions.into_iter().collect();
        let existing_ids: Vec<NodeId> = self.motion.transitions.keys().copied().collect();
        for id in existing_ids {
            if !map.contains_key(&id) {
                self.motion.set_node_transitions(id, Vec::new());
            }
        }
        for (id, specs) in map {
            self.set_node_transitions(id, specs.to_vec());
        }
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
        }
    }
}
