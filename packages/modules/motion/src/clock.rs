//! Time-driven eased transitions ([`TimedTransition`]), unified [`MotionTrack`]s, and the
//! zero-idle-CPU [`MotionClock`] track slab.

use nve_abi::{Key64, NodeId};

use crate::easing::EasingCurve;
use crate::interpolate::lerp_f32;
use crate::spring::{MotionPreference, SpringSpec, SpringState};
use crate::table::{TransitionProperty, TransitionSpec};

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
    /// Settled once the full delay + duration elapsed, or immediately for a no-op
    /// (`from == to`) transition. Never mid-flight: an overshooting curve crossing `to` keeps
    /// running.
    pub fn is_settled(&self) -> bool {
        self.elapsed_s >= self.delay_s + self.duration_s || self.from == self.to
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

/// Identifier of a [`MotionClock`] track: a compile-time-hashable name or a node property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MotionKey {
    Named(Key64),
    Node(NodeId, TransitionProperty),
}

impl MotionKey {
    /// Named key hashed with `xxh64` (`const`, so call sites can bake it at compile time).
    #[inline]
    pub const fn named(name: &str) -> Self {
        Self::Named(Key64::of(name))
    }
}

impl From<&str> for MotionKey {
    #[inline]
    fn from(name: &str) -> Self {
        Self::named(name)
    }
}

impl From<(NodeId, TransitionProperty)> for MotionKey {
    #[inline]
    fn from((id, prop): (NodeId, TransitionProperty)) -> Self {
        Self::Node(id, prop)
    }
}

/// Completion notification emitted once when an active track settles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionCompletion {
    pub key: MotionKey,
    pub final_value: f32,
}

/// Zero-idle-CPU coordinator for named and node-keyed spring & timed tracks.
///
/// Tracks live in a dense slab sorted by [`MotionKey`] (parallel `keys` / `tracks` / `active`
/// columns): lookups are a binary search, a node's tracks are contiguous, and [`Self::tick_with`]
/// visits tracks in key order with zero allocations. `has_active_animations()` lets display links
/// sleep once everything settled; completions fire even when [`MotionPreference::Reduce`] snaps.
#[derive(Debug, Clone, Default)]
pub struct MotionClock {
    keys: Vec<MotionKey>,
    tracks: Vec<MotionTrack>,
    active: Vec<bool>,
    preference: MotionPreference,
    active_count: usize,
}

impl MotionClock {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_preference(preference: MotionPreference) -> Self {
        Self {
            preference,
            ..Self::default()
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
        self.keys.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    #[inline]
    fn find(&self, key: &MotionKey) -> Result<usize, usize> {
        self.keys.binary_search(key)
    }

    /// Writes `track` at `key` given its search result `at` (inserting in sorted position) and
    /// keeps `active_count` exact.
    fn put(&mut self, at: Result<usize, usize>, key: MotionKey, track: MotionTrack, active: bool) {
        match at {
            Ok(i) => {
                match (self.active[i], active) {
                    (false, true) => self.active_count += 1,
                    (true, false) => self.active_count -= 1,
                    _ => {}
                }
                self.tracks[i] = track;
                self.active[i] = active;
            }
            Err(i) => {
                self.active_count += usize::from(active);
                self.keys.insert(i, key);
                self.tracks.insert(i, track);
                self.active.insert(i, active);
            }
        }
    }

    /// Starts or mid-flight retargets a spring track, preserving exact live velocity if `key`
    /// already holds a track.
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
        let at = self.find(&key);
        let state = match at {
            Ok(i) => match self.tracks[i] {
                MotionTrack::Spring { mut state, .. } => {
                    state.retarget(target);
                    state
                }
                other => SpringState::with_velocity(other.value(), other.velocity(), target),
            },
            Err(_) => SpringState::with_velocity(initial_if_absent, velocity_if_absent, target),
        };
        let active = !state.is_settled(&spec);
        self.put(at, key, MotionTrack::Spring { spec, state }, active);
    }

    /// Starts or smoothly retargets a timed eased transition track.
    pub fn animate_transition(
        &mut self,
        key: impl Into<MotionKey>,
        spec: TransitionSpec,
        initial_if_absent: f32,
        target: f32,
    ) {
        let key = key.into();
        let at = self.find(&key);
        let from = at.map_or(initial_if_absent, |i| self.tracks[i].value());
        let transition = TimedTransition::new(from, target, &spec);
        let active = !transition.is_settled();
        self.put(at, key, MotionTrack::Timed(transition), active);
    }

    /// Immediately sets `key` to a settled scalar `value` without animating.
    pub fn set_immediate(&mut self, key: impl Into<MotionKey>, value: f32) {
        let track = MotionTrack::Spring {
            spec: SpringSpec::snappy(),
            state: SpringState::settled(value),
        };
        let key = key.into();
        self.put(self.find(&key), key, track, false);
    }

    /// Advances all active tracks by `dt_s` seconds and returns key-ordered completions.
    pub fn tick(&mut self, dt_s: f32) -> Vec<MotionCompletion> {
        let mut completed = Vec::new();
        self.tick_with(dt_s, |ev| completed.push(ev));
        completed
    }

    /// Advances all active tracks by `dt_s` seconds, invoking `on_complete` in key order for every
    /// track that settles on this tick (zero allocations).
    pub fn tick_with(&mut self, dt_s: f32, mut on_complete: impl FnMut(MotionCompletion)) {
        self.step_active_with(dt_s, |key, value, settled| {
            if settled {
                on_complete(MotionCompletion {
                    key,
                    final_value: value,
                });
            }
        });
    }

    /// Advances all active tracks by `dt_s` seconds, invoking `on_step(key, new_value, settled)` in
    /// key order for every active track stepped on this tick (zero allocations).
    pub fn step_active_with(&mut self, dt_s: f32, mut on_step: impl FnMut(MotionKey, f32, bool)) {
        if self.active_count == 0 {
            return;
        }
        let pref = self.preference;
        // Stop scanning once every track that was active at the start has been stepped.
        let mut remaining = self.active_count;
        for i in 0..self.keys.len() {
            if remaining == 0 {
                break;
            }
            if !self.active[i] {
                continue;
            }
            remaining -= 1;
            let settled = self.tracks[i].step(dt_s, pref);
            if settled {
                self.active[i] = false;
                self.active_count -= 1;
            }
            on_step(self.keys[i], self.tracks[i].value(), settled);
        }
    }

    /// Advances active tracks that satisfy `keep(ctx, key)` by `dt_s` while pruning any track for
    /// which `keep(ctx, key)` returns `false` (e.g. removed `NodeId`s), preserving key order with
    /// zero heap allocations.
    pub fn step_and_prune_active_with<C>(
        &mut self,
        dt_s: f32,
        ctx: &mut C,
        mut keep: impl FnMut(&C, MotionKey) -> bool,
        mut on_step: impl FnMut(&mut C, MotionKey, f32, bool),
    ) {
        if self.keys.is_empty() {
            return;
        }
        let pref = self.preference;
        let len = self.keys.len();
        let mut write = 0usize;
        for read in 0..len {
            let key = self.keys[read];
            if !keep(ctx, key) {
                if self.active[read] {
                    self.active_count -= 1;
                }
                continue;
            }
            let mut track = self.tracks[read];
            let mut is_active = self.active[read];
            if is_active {
                let settled = track.step(dt_s, pref);
                if settled {
                    is_active = false;
                    self.active_count -= 1;
                }
                on_step(ctx, key, track.value(), settled);
            }
            if write != read {
                self.keys[write] = key;
            }
            self.tracks[write] = track;
            self.active[write] = is_active;
            write += 1;
        }
        self.keys.truncate(write);
        self.tracks.truncate(write);
        self.active.truncate(write);
    }

    #[inline]
    pub fn value(&self, key: &MotionKey) -> Option<f32> {
        self.track(key).map(MotionTrack::value)
    }

    #[inline]
    pub fn velocity(&self, key: &MotionKey) -> Option<f32> {
        self.track(key).map(MotionTrack::velocity)
    }

    #[inline]
    pub fn track(&self, key: &MotionKey) -> Option<&MotionTrack> {
        self.find(key).ok().map(|i| &self.tracks[i])
    }

    /// Removes `key`, returning whether it existed.
    pub fn remove(&mut self, key: &MotionKey) -> bool {
        let Ok(i) = self.find(key) else {
            return false;
        };
        self.remove_range(i..i + 1);
        true
    }

    /// Removes every track bound to `node` (one contiguous run of the sorted slab).
    pub fn remove_node(&mut self, node: NodeId) {
        let start = self
            .keys
            .partition_point(|k| *k < MotionKey::Node(node, TransitionProperty::All));
        let end = start
            + self.keys[start..]
                .iter()
                .take_while(|k| matches!(k, MotionKey::Node(n, _) if *n == node))
                .count();
        self.remove_range(start..end);
    }

    fn remove_range(&mut self, range: std::ops::Range<usize>) {
        self.active_count -= self.active[range.clone()].iter().filter(|a| **a).count();
        self.keys.drain(range.clone());
        self.tracks.drain(range.clone());
        self.active.drain(range);
    }

    pub fn clear(&mut self) {
        self.keys.clear();
        self.tracks.clear();
        self.active.clear();
        self.active_count = 0;
    }
}
