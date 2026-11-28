//! Skia adapter for `nve-motion`: [`MotionDriver`], a zero-idle-CPU [`SkiaFrameHook`] animating
//! declarative [`Motion`] transitions read zero-copy from `&NodeStore`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_abi::hash::{HashMap, HashSet};
use nve_backend_skia::{SkiaFrameHook, TargetsOf};
use nve_tables::snapshots::VisualSnapshot;
use nve_tables::transform::Transform2D;
use nve_abi::ColorRgba;
use nve_motion::{
    lerp_color_rgba, lerp_f32, lerp_transform_2d, resolve_transition_for, Motion, MotionPreference,
    TimedTransition, TransitionProperty, Transitions,
};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};

#[cfg(test)]
mod tests;

/// Animated visual channels, in [`NodeMotion::channels`] order.
const VISUAL_PROPS: [TransitionProperty; 5] = [
    TransitionProperty::BackgroundColor,
    TransitionProperty::Color,
    TransitionProperty::BorderColor,
    TransitionProperty::BorderRadius,
    TransitionProperty::Opacity,
];
const COLOR_CHANNEL: usize = 1;
const OPACITY_CHANNEL: usize = 4;

/// Endpoints of one animated channel (only the channel's own value, not a whole snapshot).
#[derive(Debug, Clone, Copy)]
enum Span {
    Color(ColorRgba, ColorRgba),
    Scalar(f32, f32),
}

fn channel_span(from: &VisualSnapshot, to: &VisualSnapshot, idx: usize) -> Span {
    match idx {
        0 => Span::Color(from.background, to.background),
        1 => Span::Color(from.text_color, to.text_color),
        2 => Span::Color(from.border_color, to.border_color),
        3 => Span::Scalar(from.border_radius, to.border_radius),
        _ => Span::Scalar(from.opacity, to.opacity),
    }
}

fn channel_differs(a: &VisualSnapshot, b: &VisualSnapshot, idx: usize) -> bool {
    match channel_span(a, b, idx) {
        Span::Color(x, y) => x != y,
        Span::Scalar(x, y) => (x - y).abs() > 1e-4,
    }
}

#[derive(Debug, Clone, Copy)]
struct ChannelAnim {
    span: Span,
    timer: TimedTransition,
}

impl ChannelAnim {
    fn apply(&self, idx: usize, style: &mut VisualSnapshot) {
        let t = self.timer.progress();
        match (self.span, idx) {
            (Span::Color(a, b), 0) => style.background = lerp_color_rgba(a, b, t),
            (Span::Color(a, b), 1) => style.text_color = lerp_color_rgba(a, b, t),
            (Span::Color(a, b), _) => style.border_color = lerp_color_rgba(a, b, t),
            (Span::Scalar(a, b), 3) => style.border_radius = lerp_f32(a, b, t).max(0.0),
            (Span::Scalar(a, b), _) => style.opacity = lerp_f32(a, b, t).clamp(0.0, 1.0),
        }
    }
}

/// Steps `timer`; returns `(settled, painted value changed)`. Ticks inside the delay change
/// nothing visible.
fn step_timer(timer: &mut TimedTransition, dt_s: f32, pref: MotionPreference) -> (bool, bool) {
    let before = timer.progress();
    let settled = timer.step(dt_s, pref);
    (settled, settled || timer.progress() != before)
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
        for (idx, ch) in self.channels.iter().enumerate() {
            if let Some(ch) = ch {
                ch.apply(idx, style);
            }
        }
    }

    /// Steps every channel; returns `(paint_changed, composite_changed)`.
    fn step(&mut self, dt_s: f32, pref: MotionPreference) -> (bool, bool) {
        let mut paint_changed = false;
        let mut composite_changed = false;
        for (idx, slot) in self.channels.iter_mut().enumerate() {
            if let Some(ch) = slot {
                let (settled, moved) = step_timer(&mut ch.timer, dt_s, pref);
                if moved {
                    if idx == OPACITY_CHANNEL {
                        composite_changed = true;
                    } else {
                        paint_changed = true;
                    }
                }
                if settled {
                    *slot = None;
                }
            }
        }
        (paint_changed, composite_changed)
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
/// animates the changed channels from the currently painted value. Work per commit is
/// proportional to the changed nodes (and tracked descendants of changed ancestors, whose
/// inherited styles may have moved), never to every tracked node.
#[derive(Debug, Clone, Default)]
pub struct MotionDriver {
    preference: MotionPreference,
    tracked: HashSet<NodeId>,
    last_target: HashMap<NodeId, VisualSnapshot>,
    last_transform: HashMap<NodeId, Transform2D>,
    active: HashMap<NodeId, NodeMotion>,
    active_transform: HashMap<NodeId, TransformAnim>,
    changed: HashSet<NodeId>,
    scratch: Vec<NodeId>,
    seen_epoch_marks: Vec<u64>,
    seen_epoch: u64,
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
        self.last_target.remove(&id);
        self.last_transform.remove(&id);
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

    /// `id` or one of its ancestors changed this frame.
    fn affected(&self, store: &NodeStore, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(n) = cur {
            if self.changed.contains(&n) {
                return true;
            }
            cur = store.parent(n);
        }
        false
    }

    fn sync_visual(&mut self, store: &NodeStore, id: NodeId, target: VisualSnapshot) {
        let Some(prev) = self.last_target.insert(id, target) else {
            return;
        };
        let mut current = prev;
        let had_active = if let Some(existing) = self.active.get(&id) {
            existing.apply_to(&mut current);
            true
        } else {
            false
        };
        let mut any_diff = false;
        for idx in 0..VISUAL_PROPS.len() {
            if channel_differs(&prev, &target, idx) {
                any_diff = true;
                break;
            }
        }
        if !any_diff {
            return;
        }
        if had_active {
            let is_empty = if let Some(motion) = self.active.get_mut(&id) {
                for (idx, prop) in VISUAL_PROPS.into_iter().enumerate() {
                    if channel_differs(&prev, &target, idx) {
                        motion.channels[idx] =
                            start_timer(store, id, prop, self.preference).map(|timer| ChannelAnim {
                                span: channel_span(&current, &target, idx),
                                timer,
                            });
                    }
                }
                motion.is_empty()
            } else {
                false
            };
            if is_empty {
                self.active.remove(&id);
            }
        } else {
            let mut motion = NodeMotion::default();
            for (idx, prop) in VISUAL_PROPS.into_iter().enumerate() {
                if channel_differs(&prev, &target, idx) {
                    motion.channels[idx] =
                        start_timer(store, id, prop, self.preference).map(|timer| ChannelAnim {
                            span: channel_span(&current, &target, idx),
                            timer,
                        });
                }
            }
            if !motion.is_empty() {
                self.active.insert(id, motion);
            }
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
                if let Some(existing) = self.active_transform.get_mut(&id) {
                    *existing = TransformAnim { from, to: target, timer };
                } else {
                    self.active_transform.insert(id, TransformAnim { from, to: target, timer });
                }
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
    fn register_tables(&self) {
        let _ = Motion::slot();
    }

    fn after_commit(&mut self, store: &NodeStore, changes: &ChangeSet, targets: TargetsOf<'_>) {
        if !changes.removed.is_empty() {
            self.retain_live(store);
        }
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
        if self.tracked.is_empty() || (changes.created.is_empty() && changes.touched.is_empty()) {
            return;
        }
        self.changed.clear();
        self.changed.extend(changes.created.iter().chain(changes.touched.iter()).copied());
        let mut ids = std::mem::take(&mut self.scratch);
        ids.clear();
        ids.extend(self.tracked.iter().copied().filter(|&id| self.affected(store, id)));
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
        if !(dt_s.is_finite() && dt_s > 0.0) {
            return false;
        }
        self.step_all_collecting(dt_s, None, None)
    }

    fn advance_collecting(
        &mut self,
        dt_s: f32,
        composite_dirty: &mut Vec<NodeId>,
        paint_dirty: &mut Vec<NodeId>,
    ) -> bool {
        if !(dt_s.is_finite() && dt_s > 0.0) {
            return false;
        }
        self.step_all_collecting(dt_s, Some(composite_dirty), Some(paint_dirty))
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
        // A zero-length step under `Reduce` snaps exactly the transitions that opted into
        // `reduced_motion_instant`; the others keep animating.
        pref.is_reduced() && self.step_all_collecting(0.0, None, None)
    }

    fn set_reduced_motion_collecting(
        &mut self,
        reduced: bool,
        composite_dirty: &mut Vec<NodeId>,
        paint_dirty: &mut Vec<NodeId>,
    ) -> bool {
        let pref = MotionPreference::from_reduced_motion(reduced);
        if self.preference == pref {
            return false;
        }
        self.preference = pref;
        pref.is_reduced()
            && self.step_all_collecting(0.0, Some(composite_dirty), Some(paint_dirty))
    }

    fn watches_changes(&self, changes: &ChangeSet) -> bool {
        changes.table(Motion::slot()).is_some()
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

impl MotionDriver {
    /// Steps every running animation by `dt_s`, deduplicating composite-dirty node IDs in zero
    /// steady-state allocations via epoch marks; returns whether any painted value changed.
    fn step_all_collecting(
        &mut self,
        dt_s: f32,
        mut composite_dirty: Option<&mut Vec<NodeId>>,
        mut paint_dirty: Option<&mut Vec<NodeId>>,
    ) -> bool {
        let pref = self.preference;
        self.seen_epoch = self.seen_epoch.wrapping_add(1).max(1);
        let cur_epoch = self.seen_epoch;
        let seen_marks = &mut self.seen_epoch_marks;
        let mut changed = false;

        self.active.retain(|&id, motion| {
            let (paint_moved, comp_moved) = motion.step(dt_s, pref);
            if paint_moved {
                changed = true;
                if let Some(ref mut out) = paint_dirty {
                    out.push(id);
                }
            }
            if comp_moved {
                changed = true;
                if let Some(ref mut out) = composite_dirty {
                    let idx = id.index();
                    if idx >= seen_marks.len() {
                        seen_marks.resize(idx + 1, 0);
                    }
                    if seen_marks[idx] != cur_epoch {
                        seen_marks[idx] = cur_epoch;
                        out.push(id);
                    }
                }
            }
            !motion.is_empty()
        });

        self.active_transform.retain(|&id, anim| {
            let (settled, moved) = step_timer(&mut anim.timer, dt_s, pref);
            if moved {
                changed = true;
                if let Some(ref mut out) = composite_dirty {
                    let idx = id.index();
                    if idx >= seen_marks.len() {
                        seen_marks.resize(idx + 1, 0);
                    }
                    if seen_marks[idx] != cur_epoch {
                        seen_marks[idx] = cur_epoch;
                        out.push(id);
                    }
                }
            }
            !settled
        });
        changed
    }
}
