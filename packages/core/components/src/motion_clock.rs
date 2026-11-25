//! Time-driven eased transitions ([`TimedTransition`]), unified [`MotionTrack`]s, and the
//! zero-idle-CPU [`MotionClock`] / [`MotionGroup`] track manager.

use std::borrow::Cow;

use nve_abi::hash::HashMap;
use nve_abi::NodeId;

use crate::motion::{lerp_f32, EasingCurve, TransitionProperty, TransitionSpec};
use crate::spring::{MotionPreference, SpringSpec, SpringState};

/// Time-driven eased scalar transition supporting delay, mid-flight retargeting, and
/// reduced-motion instant completion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedTransition {
    pub from: f32,
    pub to: f32,
    pub value: f32,
    pub duration_s: f32,
    pub delay_s: f32,
    pub elapsed_s: f32,
    pub easing: EasingCurve,
    pub reduced_motion_instant: bool,
}

impl TimedTransition {
    /// Creates a new [`TimedTransition`] from `from` toward `to` governed by `spec`.
    pub fn new(from: f32, to: f32, spec: &TransitionSpec) -> Self {
        let f = if from.is_finite() { from } else { 0.0 };
        let dst = if to.is_finite() { to } else { 0.0 };
        let duration_s = spec.duration_s().max(0.0);
        let delay_s = spec.delay_s().max(0.0);
        let instant = duration_s <= 0.0 && delay_s <= 0.0;
        Self {
            from: f,
            to: dst,
            value: if instant { dst } else { f },
            duration_s,
            delay_s,
            elapsed_s: 0.0,
            easing: spec.easing,
            reduced_motion_instant: spec.reduced_motion_instant,
        }
    }

    /// Retargets smoothly from the current interpolated value toward `new_to`.
    pub fn retarget(&mut self, new_to: f32) {
        if !new_to.is_finite() {
            return;
        }
        self.from = self.value;
        self.to = new_to;
        self.elapsed_s = 0.0;
        if self.duration_s <= 0.0 && self.delay_s <= 0.0 {
            self.value = new_to;
        }
    }

    /// Returns normalized eased progress in `[0.0, 1.0]` (or curve overshoot).
    pub fn progress(&self) -> f32 {
        if self.elapsed_s < self.delay_s {
            return 0.0;
        }
        if self.duration_s <= 0.0 {
            return 1.0;
        }
        let raw = ((self.elapsed_s - self.delay_s) / self.duration_s).clamp(0.0, 1.0);
        self.easing.sample(raw)
    }

    /// Returns `true` once elapsed time has reached `delay_s + duration_s`.
    #[inline]
    pub fn is_settled(&self) -> bool {
        self.elapsed_s >= self.delay_s + self.duration_s
            || (self.value - self.to).abs() <= f32::EPSILON && self.elapsed_s >= self.delay_s
    }

    /// Snaps immediately to `to`.
    #[inline]
    pub fn snap_to_target(&mut self) {
        self.elapsed_s = self.delay_s + self.duration_s;
        self.value = self.to;
    }

    /// Advances by `dt_s` seconds under `preference`, returning `true` when settled.
    pub fn step(&mut self, dt_s: f32, preference: MotionPreference) -> bool {
        if preference.is_reduced() && self.reduced_motion_instant {
            self.snap_to_target();
            return true;
        }
        if dt_s.is_finite() && dt_s > 0.0 {
            self.elapsed_s += dt_s;
        }
        if self.elapsed_s >= self.delay_s + self.duration_s {
            self.snap_to_target();
            return true;
        }
        let eased = self.progress();
        self.value = lerp_f32(self.from, self.to, eased);
        false
    }
}

/// Unified animated track: either interruptible spring physics or a timed eased transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionTrack {
    Spring { spec: SpringSpec, state: SpringState },
    Timed(TimedTransition),
}

impl MotionTrack {
    #[inline]
    pub fn value(&self) -> f32 {
        match self {
            Self::Spring { state, .. } => state.value,
            Self::Timed(t) => t.value,
        }
    }

    #[inline]
    pub fn target(&self) -> f32 {
        match self {
            Self::Spring { state, .. } => state.target,
            Self::Timed(t) => t.to,
        }
    }

    #[inline]
    pub fn velocity(&self) -> f32 {
        match self {
            Self::Spring { state, .. } => state.velocity,
            Self::Timed(_) => 0.0,
        }
    }

    #[inline]
    pub fn is_settled(&self) -> bool {
        match self {
            Self::Spring { spec, state } => state.is_settled(spec),
            Self::Timed(t) => t.is_settled(),
        }
    }

    #[inline]
    pub fn retarget(&mut self, new_target: f32) {
        match self {
            Self::Spring { state, .. } => state.retarget(new_target),
            Self::Timed(t) => t.retarget(new_target),
        }
    }

    #[inline]
    pub fn step(&mut self, dt_s: f32, preference: MotionPreference) -> bool {
        match self {
            Self::Spring { spec, state } => state.step(spec, dt_s, preference),
            Self::Timed(t) => t.step(dt_s, preference),
        }
    }
}

/// Identifier for a track managed by [`MotionClock`] / [`MotionGroup`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MotionKey {
    Named(Cow<'static, str>),
    NodeProp(NodeId, TransitionProperty),
}

impl From<&'static str> for MotionKey {
    #[inline]
    fn from(name: &'static str) -> Self {
        Self::Named(Cow::Borrowed(name))
    }
}

impl From<String> for MotionKey {
    #[inline]
    fn from(name: String) -> Self {
        Self::Named(Cow::Owned(name))
    }
}

impl From<(NodeId, TransitionProperty)> for MotionKey {
    #[inline]
    fn from((id, prop): (NodeId, TransitionProperty)) -> Self {
        Self::NodeProp(id, prop)
    }
}

/// Deterministic completion notification emitted once when an active track settles.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionCompletion {
    pub key: MotionKey,
    pub final_value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TrackEntry {
    track: MotionTrack,
    active: bool,
}

/// Zero-idle-CPU coordinator for named and node-keyed spring & timed transitions.
///
/// Tracks whether any animation is active (`has_active_animations()`) so native display links
/// sleep at 0% idle CPU/GPU once settled, and fires deterministic [`MotionCompletion`] events
/// even when [`MotionPreference::Reduce`] snaps animations immediately.
#[derive(Debug, Clone, Default)]
pub struct MotionClock {
    tracks: HashMap<MotionKey, TrackEntry>,
    preference: MotionPreference,
    active_count: usize,
}

/// Ergonomic alias for [`MotionClock`].
pub type MotionGroup = MotionClock;

impl MotionClock {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_preference(preference: MotionPreference) -> Self {
        Self {
            tracks: HashMap::default(),
            preference,
            active_count: 0,
        }
    }

    #[inline]
    pub fn preference(&self) -> MotionPreference {
        self.preference
    }

    #[inline]
    pub fn set_preference(&mut self, preference: MotionPreference) {
        self.preference = preference;
    }

    /// Returns `true` iff at least one spring or transition is still running.
    #[inline]
    pub fn has_active_animations(&self) -> bool {
        self.active_count > 0
    }

    #[inline]
    pub fn active_count(&self) -> usize {
        self.active_count
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Starts or mid-flight retargets a spring track, preserving exact live velocity if `key`
    /// already holds a spring.
    pub fn animate_spring(
        &mut self,
        key: impl Into<MotionKey>,
        spec: SpringSpec,
        initial_if_absent: f32,
        target: f32,
    ) {
        self.animate_spring_with_velocity(key, spec, initial_if_absent, 0.0, target);
    }

    /// Variant of [`Self::animate_spring`] specifying an initial velocity if `key` is new.
    pub fn animate_spring_with_velocity(
        &mut self,
        key: impl Into<MotionKey>,
        spec: SpringSpec,
        initial_if_absent: f32,
        velocity_if_absent: f32,
        target: f32,
    ) {
        let key = key.into();
        if let Some(entry) = self.tracks.get_mut(&key) {
            let was_active = entry.active;
            match &mut entry.track {
                MotionTrack::Spring {
                    spec: existing_spec,
                    state,
                } => {
                    *existing_spec = spec;
                    state.retarget(target);
                }
                other => {
                    let cur = other.value();
                    let vel = other.velocity();
                    entry.track = MotionTrack::Spring {
                        spec,
                        state: SpringState::with_velocity(cur, vel, target),
                    };
                }
            }
            let needs_step = !entry.track.is_settled();
            if needs_step && !was_active {
                entry.active = true;
                self.active_count += 1;
            }
            return;
        }
        let state = SpringState::with_velocity(initial_if_absent, velocity_if_absent, target);
        let active = !state.is_settled(&spec);
        if active {
            self.active_count += 1;
        }
        self.tracks.insert(
            key,
            TrackEntry {
                track: MotionTrack::Spring { spec, state },
                active,
            },
        );
    }

    /// Starts or smoothly retargets a timed CSS-easing transition track.
    pub fn animate_transition(
        &mut self,
        key: impl Into<MotionKey>,
        spec: TransitionSpec,
        initial_if_absent: f32,
        target: f32,
    ) {
        let key = key.into();
        let from = self
            .tracks
            .get(&key)
            .map_or(initial_if_absent, |e| e.track.value());
        let was_active = self.tracks.get(&key).is_some_and(|e| e.active);
        let transition = TimedTransition::new(from, target, &spec);
        let active = (from - target).abs() > f32::EPSILON || !transition.is_settled();
        match (was_active, active) {
            (false, true) => self.active_count += 1,
            (true, false) => self.active_count = self.active_count.saturating_sub(1),
            _ => {}
        }
        self.tracks.insert(
            key,
            TrackEntry {
                track: MotionTrack::Timed(transition),
                active,
            },
        );
    }

    /// Immediately sets `key` to a settled scalar `value` without animating.
    pub fn set_immediate(&mut self, key: impl Into<MotionKey>, value: f32) {
        let key = key.into();
        if let Some(prev) = self.tracks.get(&key)
            && prev.active
        {
            self.active_count = self.active_count.saturating_sub(1);
        }
        self.tracks.insert(
            key,
            TrackEntry {
                track: MotionTrack::Spring {
                    spec: SpringSpec::snappy(),
                    state: SpringState::settled(value),
                },
                active: false,
            },
        );
    }

    /// Advances all active tracks by `dt_s` seconds and returns deterministically ordered
    /// [`MotionCompletion`] events for tracks that settled on this tick.
    pub fn tick(&mut self, dt_s: f32) -> Vec<MotionCompletion> {
        let mut completed = Vec::new();
        self.tick_with(dt_s, |ev| completed.push(ev));
        completed
    }

    /// Advances all active tracks by `dt_s` seconds, invoking `on_complete` in deterministic key
    /// order for every track that settles on this tick.
    pub fn tick_with(&mut self, dt_s: f32, mut on_complete: impl FnMut(MotionCompletion)) {
        if self.active_count == 0 {
            return;
        }
        let pref = self.preference;
        let mut settled: Vec<MotionCompletion> = Vec::new();
        for (key, entry) in &mut self.tracks {
            if !entry.active {
                continue;
            }
            if entry.track.step(dt_s, pref) {
                entry.active = false;
                self.active_count = self.active_count.saturating_sub(1);
                settled.push(MotionCompletion {
                    key: key.clone(),
                    final_value: entry.track.value(),
                });
            }
        }
        settled.sort_by(|a, b| a.key.cmp(&b.key));
        for ev in settled {
            on_complete(ev);
        }
    }

    #[inline]
    pub fn value(&self, key: &MotionKey) -> Option<f32> {
        self.tracks.get(key).map(|e| e.track.value())
    }

    #[inline]
    pub fn value_named(&self, name: &str) -> Option<f32> {
        self.tracks.iter().find_map(|(k, e)| {
            matches!(k, MotionKey::Named(n) if n.as_ref() == name).then_some(e.track.value())
        })
    }

    #[inline]
    pub fn velocity(&self, key: &MotionKey) -> Option<f32> {
        self.tracks.get(key).map(|e| e.track.velocity())
    }

    #[inline]
    pub fn track(&self, key: &MotionKey) -> Option<&MotionTrack> {
        self.tracks.get(key).map(|e| &e.track)
    }

    pub fn remove(&mut self, key: &MotionKey) -> bool {
        if let Some(removed) = self.tracks.remove(key) {
            if removed.active {
                self.active_count = self.active_count.saturating_sub(1);
            }
            true
        } else {
            false
        }
    }

    /// Removes all tracks bound to `node_id`.
    pub fn remove_node(&mut self, node_id: NodeId) {
        let mut removed_active = 0usize;
        self.tracks.retain(|k, entry| {
            let drop_it = matches!(k, MotionKey::NodeProp(id, _) if *id == node_id);
            if drop_it && entry.active {
                removed_active += 1;
            }
            !drop_it
        });
        self.active_count = self.active_count.saturating_sub(removed_active);
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.active_count = 0;
    }
}
