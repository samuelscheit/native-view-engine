//! Skia adapter for `nve-motion`: [`MotionDriver`], a zero-idle-CPU [`SkiaFrameHook`] animating
//! declarative [`Motion`] transitions read zero-copy from `&NodeStore`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_abi::hash::{HashMap, HashSet};
use nve_backend_skia::{SkiaFrameHook, TargetsOf};
use nve_components::snapshots::VisualSnapshot;
use nve_components::transform::Transform2D;
use nve_motion::{
    lerp_style_property, lerp_transform_2d, resolve_transition_for, Motion, MotionPreference,
    TimedTransition, TransitionProperty, Transitions,
};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};

#[cfg(test)]
mod tests;

const VISUAL_PROPS: [TransitionProperty; 5] = [
    TransitionProperty::BackgroundColor,
    TransitionProperty::Color,
    TransitionProperty::BorderColor,
    TransitionProperty::BorderRadius,
    TransitionProperty::Opacity,
];
const COLOR_CHANNEL: usize = 1;

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
        for (prop, ch) in VISUAL_PROPS.into_iter().zip(&self.channels) {
            if let Some(ch) = ch {
                lerp_style_property(&ch.from, &ch.to, prop, ch.timer.progress(), style);
            }
        }
    }

    /// Steps every channel; returns `true` when any channel was running.
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

#[derive(Debug, Clone, Copy)]
struct TransformAnim {
    from: Transform2D,
    to: Transform2D,
    timer: TimedTransition,
}

impl TransformAnim {
    #[inline]
    fn current(&self) -> Transform2D {
        lerp_transform_2d(&self.from, &self.to, self.timer.progress())
    }
}

fn visual_differs(a: &VisualSnapshot, b: &VisualSnapshot, prop: TransitionProperty) -> bool {
    match prop {
        TransitionProperty::BackgroundColor => a.background != b.background,
        TransitionProperty::Color => a.text_color != b.text_color,
        TransitionProperty::BorderColor => a.border_color != b.border_color,
        TransitionProperty::BorderRadius => (a.border_radius - b.border_radius).abs() > 1e-4,
        TransitionProperty::Opacity => (a.opacity - b.opacity).abs() > 1e-4,
        _ => false,
    }
}

fn transform_differs(a: &Transform2D, b: &Transform2D) -> bool {
    [
        (a.translate_x, b.translate_x),
        (a.translate_y, b.translate_y),
        (a.scale_x, b.scale_x),
        (a.scale_y, b.scale_y),
        (a.rotate_deg, b.rotate_deg),
        (a.origin_x, b.origin_x),
        (a.origin_y, b.origin_y),
    ]
    .iter()
    .any(|(x, y)| (x - y).abs() > 1e-4)
}

/// Starts a timer for the transition governing `prop` on `node`, unless it completes instantly.
fn start_timer(
    store: &NodeStore,
    node: NodeId,
    prop: TransitionProperty,
    pref: MotionPreference,
) -> Option<TimedTransition> {
    let row = store.try_row::<Motion>(node)?;
    let specs = store.resource::<Transitions>(row.transitions())?;
    let spec = resolve_transition_for(specs.iter(), prop)?;
    let mut timer = TimedTransition::new(0.0, 1.0, &spec);
    (!timer.step(0.0, pref)).then_some(timer)
}

/// Per-node declarative transition driver for `nve_backend_skia::SkiaBackend`.
///
/// Tracks nodes carrying a [`Motion`] row, remembers each node's last resolved target, and
/// animates from the currently painted value whenever a target changes.
#[derive(Debug, Clone, Default)]
pub struct MotionDriver {
    preference: MotionPreference,
    tracked: HashSet<NodeId>,
    last_target: HashMap<NodeId, VisualSnapshot>,
    last_transform: HashMap<NodeId, Transform2D>,
    active: HashMap<NodeId, NodeMotion>,
    active_transform: HashMap<NodeId, TransformAnim>,
    scratch: Vec<NodeId>,
}

impl MotionDriver {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn preference(&self) -> MotionPreference {
        self.preference
    }

    fn untrack(&mut self, id: NodeId) {
        self.tracked.remove(&id);
        self.active.remove(&id);
        self.active_transform.remove(&id);
    }

    fn retain_live(&mut self, store: &NodeStore) {
        self.tracked.retain(|&id| store.is_live(id));
        self.last_target.retain(|&id, _| store.is_live(id));
        self.last_transform.retain(|&id, _| store.is_live(id));
        self.active.retain(|&id, _| store.is_live(id));
        self.active_transform.retain(|&id, _| store.is_live(id));
    }

    fn sync_visual(&mut self, store: &NodeStore, id: NodeId, target: VisualSnapshot) {
        let Some(prev) = self.last_target.insert(id, target) else {
            return;
        };
        let mut current = prev;
        let mut motion = self.active.remove(&id).unwrap_or_default();
        motion.apply_to(&mut current);
        for (idx, prop) in VISUAL_PROPS.into_iter().enumerate() {
            if visual_differs(&prev, &target, prop) {
                motion.channels[idx] = start_timer(store, id, prop, self.preference)
                    .map(|timer| ChannelAnim { from: current, to: target, timer });
            }
        }
        if !motion.is_empty() {
            self.active.insert(id, motion);
        }
    }

    fn sync_transform(&mut self, store: &NodeStore, id: NodeId, target: Transform2D) {
        let Some(prev) = self.last_transform.insert(id, target) else {
            return;
        };
        if !transform_differs(&prev, &target) {
            return;
        }
        let from = self.active_transform.get(&id).map_or(prev, TransformAnim::current);
        match start_timer(store, id, TransitionProperty::Transform, self.preference) {
            Some(timer) => {
                self.active_transform.insert(id, TransformAnim { from, to: target, timer });
            }
            None => {
                self.active_transform.remove(&id);
            }
        }
    }

    fn sync(&mut self, store: &NodeStore, id: NodeId, targets: TargetsOf<'_>) {
        let t = targets(id);
        self.sync_visual(store, id, t.visual);
        self.sync_transform(store, id, t.transform);
    }
}

impl SkiaFrameHook for MotionDriver {
    fn after_commit(&mut self, store: &NodeStore, changes: &ChangeSet, targets: TargetsOf<'_>) {
        self.retain_live(store);
        if let Some(tc) = changes.table(Motion::slot()) {
            for &id in &tc.detached {
                self.untrack(id);
            }
        }
        for &id in changes.created.iter().chain(changes.touched.iter()) {
            if store.has::<Motion>(id) {
                if self.tracked.insert(id) {
                    let t = targets(id);
                    self.last_target.entry(id).or_insert(t.visual);
                    self.last_transform.entry(id).or_insert(t.transform);
                }
            } else if self.tracked.contains(&id) {
                self.untrack(id);
            }
        }
        let mut ids = std::mem::take(&mut self.scratch);
        ids.clear();
        ids.extend(self.tracked.iter().copied());
        for &id in &ids {
            self.sync(store, id, targets);
        }
        self.scratch = ids;
    }

    fn sync_nodes(&mut self, store: &NodeStore, dirty: &[NodeId], targets: TargetsOf<'_>) {
        for &id in dirty {
            if self.tracked.contains(&id) && store.is_live(id) {
                self.sync(store, id, targets);
            }
        }
    }

    fn advance(&mut self, dt_s: f32) -> bool {
        let pref = self.preference;
        let mut changed = false;
        self.active.retain(|_, motion| {
            changed |= motion.step(dt_s, pref);
            !motion.is_empty()
        });
        self.active_transform.retain(|_, anim| {
            changed = true;
            !anim.timer.step(dt_s, pref)
        });
        changed
    }

    fn needs_next_frame(&self) -> bool {
        !self.active.is_empty() || !self.active_transform.is_empty()
    }

    fn set_reduced_motion(&mut self, reduced: bool) -> bool {
        let pref = MotionPreference::from_reduced_motion(reduced);
        if self.preference == pref {
            return false;
        }
        self.preference = pref;
        let had_active = self.needs_next_frame();
        if pref.is_reduced() {
            self.active.clear();
            self.active_transform.clear();
        }
        pref.is_reduced() && had_active
    }

    fn apply_visual(&self, node: NodeId, style: &mut VisualSnapshot) {
        if let Some(motion) = self.active.get(&node) {
            motion.apply_to(style);
        }
    }

    fn overrides_text_color(&self, node: NodeId) -> bool {
        self.active
            .get(&node)
            .is_some_and(|m| m.channels[COLOR_CHANNEL].is_some())
    }

    fn resolve_transform(&self, node: NodeId, target: Option<Transform2D>) -> Option<Transform2D> {
        self.active_transform
            .get(&node)
            .map(TransformAnim::current)
            .or(target)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}
