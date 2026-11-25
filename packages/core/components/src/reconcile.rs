//! Reusable generic 3-way state divergence reconciler (zero serialization, zero external deps).
//!
//! Shared across:
//! 1. SSR -> Hydration dirty DOM control reconciliation (`<input>`, `<textarea>`, `<select>`,
//!    checkboxes) where user edits made before hydration must survive initial tree adoption.
//! 2. Hot-reload signal archive reconciliation (adopting updated code defaults for untouched
//!    signals while preserving user-mutated signals).
//! 3. Offline / network CRDT & optimistic UI synchronization with deterministic conflict
//!    resolution and in-flight acknowledgement race protection.

use std::hash::Hash;

use nve_abi::hash::HashMap;

/// Lamport logical clock with deterministic peer tie-breaker.
///
/// Ordering compares `counter` first and breaks ties deterministically by `peer_id`.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct LogicalClock {
    pub counter: u64,
    pub peer_id: u32,
}

impl LogicalClock {
    /// Creates a new logical clock at `counter = 0` for `peer_id`.
    #[inline]
    pub const fn new(peer_id: u32) -> Self {
        Self { counter: 0, peer_id }
    }

    /// Creates a logical clock with an explicit `(counter, peer_id)`.
    #[inline]
    pub const fn at(counter: u64, peer_id: u32) -> Self {
        Self { counter, peer_id }
    }

    /// Advances the local clock monotonically (saturating on `u64::MAX`) and returns the new value.
    #[inline]
    pub fn tick(&mut self) -> Self {
        self.counter = self.counter.saturating_add(1);
        *self
    }

    /// Merges an observed `remote` clock into this clock without altering `self.peer_id`.
    ///
    /// Idempotent and monotonic: raises `self.counter` to `max(self.counter, remote.counter)` so
    /// the next [`Self::tick`] strictly dominates both local history and `remote`.
    #[inline]
    pub fn merge(&mut self, remote: Self) -> Self {
        if remote.counter > self.counter {
            self.counter = remote.counter;
        }
        *self
    }
}

/// Input to a single 3-way divergence comparison between a common ancestor `base`, the current
/// `local` state, and an `incoming` remote/code update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeWayEntry<K, V> {
    pub key: K,
    pub base: V,
    pub local: V,
    pub incoming: V,
    pub local_clock: Option<LogicalClock>,
    pub incoming_clock: Option<LogicalClock>,
}

/// Classification of the 3-way divergence between `base`, `local`, and `incoming`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DivergenceKind {
    /// Neither side diverged from `base`.
    Clean,
    /// Only `local` diverged from `base`; preserve local and emit upstream.
    LocalOnlyDirty,
    /// Only `incoming` diverged from `base`; adopt incoming locally.
    IncomingOnly,
    /// Both sides diverged from `base` to the exact same value.
    Converged,
    /// Both sides diverged from `base` to different values; resolved via [`ConflictPolicy`].
    Conflict,
}

/// Policy governing how true 3-way [`DivergenceKind::Conflict`] entries are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConflictPolicy {
    /// Keep `local` and mark `emit_upstream = true` (default for DOM hydration & hot-reload).
    #[default]
    PreferLocal,
    /// Adopt `incoming` and mark `apply_local = true`.
    PreferIncoming,
    /// Pick whichever side carries the higher [`LogicalClock`] (`counter`, then `peer_id`).
    LastWriteWins,
}

/// Deterministic outcome of reconciling one [`ThreeWayEntry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileDecision<K, V> {
    pub key: K,
    pub kind: DivergenceKind,
    pub resolved: V,
    pub emit_upstream: bool,
    pub apply_local: bool,
}

/// Performs a pure, deterministic 3-way divergence reconciliation for one entry.
pub fn reconcile_three_way<K, V: PartialEq + Clone>(
    entry: ThreeWayEntry<K, V>,
    policy: ConflictPolicy,
) -> ReconcileDecision<K, V> {
    let local_changed = entry.local != entry.base;
    let incoming_changed = entry.incoming != entry.base;

    match (local_changed, incoming_changed) {
        (false, false) => ReconcileDecision {
            key: entry.key,
            kind: DivergenceKind::Clean,
            resolved: entry.base,
            emit_upstream: false,
            apply_local: false,
        },
        (true, false) => ReconcileDecision {
            key: entry.key,
            kind: DivergenceKind::LocalOnlyDirty,
            resolved: entry.local,
            emit_upstream: true,
            apply_local: false,
        },
        (false, true) => ReconcileDecision {
            key: entry.key,
            kind: DivergenceKind::IncomingOnly,
            resolved: entry.incoming,
            emit_upstream: false,
            apply_local: true,
        },
        (true, true) if entry.local == entry.incoming => ReconcileDecision {
            key: entry.key,
            kind: DivergenceKind::Converged,
            resolved: entry.local,
            emit_upstream: false,
            apply_local: false,
        },
        (true, true) => resolve_conflict(
            entry.key,
            entry.local,
            entry.incoming,
            entry.local_clock,
            entry.incoming_clock,
            policy,
        ),
    }
}

fn resolve_conflict<K, V>(
    key: K,
    local: V,
    incoming: V,
    local_clock: Option<LogicalClock>,
    incoming_clock: Option<LogicalClock>,
    policy: ConflictPolicy,
) -> ReconcileDecision<K, V> {
    let incoming_wins = match policy {
        ConflictPolicy::PreferLocal => false,
        ConflictPolicy::PreferIncoming => true,
        ConflictPolicy::LastWriteWins => match (local_clock, incoming_clock) {
            (Some(lc), Some(ic)) => ic > lc,
            (None, Some(_)) => true,
            (Some(_), None) | (None, None) => false,
        },
    };

    if incoming_wins {
        ReconcileDecision {
            key,
            kind: DivergenceKind::Conflict,
            resolved: incoming,
            emit_upstream: false,
            apply_local: true,
        }
    } else {
        ReconcileDecision {
            key,
            kind: DivergenceKind::Conflict,
            resolved: local,
            emit_upstream: true,
            apply_local: false,
        }
    }
}

/// Aggregated result returned by [`StateReconciler::reconcile_incoming_batch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileBatch<K, V> {
    pub decisions: Vec<ReconcileDecision<K, V>>,
    pub to_apply_local: Vec<(K, V)>,
    pub to_emit_upstream: Vec<(K, V, LogicalClock)>,
}

impl<K, V> Default for ReconcileBatch<K, V> {
    fn default() -> Self {
        Self {
            decisions: Vec::new(),
            to_apply_local: Vec::new(),
            to_emit_upstream: Vec::new(),
        }
    }
}

impl<K, V> ReconcileBatch<K, V> {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }

    #[inline]
    pub fn conflict_count(&self) -> usize {
        self.decisions
            .iter()
            .filter(|d| d.kind == DivergenceKind::Conflict)
            .count()
    }
}

/// Stateful 3-way reconciler maintaining last-known synchronized `baseline`, an outbox of
/// `dirty_local` optimistic mutations, and a monotonic [`LogicalClock`].
#[derive(Debug, Clone)]
pub struct StateReconciler<K: Eq + Hash + Clone, V: PartialEq + Clone> {
    baseline: HashMap<K, V>,
    dirty_local: HashMap<K, (V, LogicalClock)>,
    clock: LogicalClock,
}

impl<K: Eq + Hash + Clone, V: PartialEq + Clone> StateReconciler<K, V> {
    /// Creates an empty reconciler owned by `peer_id`.
    pub fn new(peer_id: u32) -> Self {
        Self {
            baseline: HashMap::default(),
            dirty_local: HashMap::default(),
            clock: LogicalClock::new(peer_id),
        }
    }

    #[inline]
    pub const fn peer_id(&self) -> u32 {
        self.clock.peer_id
    }

    #[inline]
    pub const fn clock(&self) -> LogicalClock {
        self.clock
    }

    /// Seeds or advances the known clean baseline for `key`.
    pub fn seed_baseline(&mut self, key: K, val: V) {
        if let Some((local_val, _)) = self.dirty_local.get(&key)
            && local_val == &val
        {
            self.dirty_local.remove(&key);
        }
        self.baseline.insert(key, val);
    }

    /// Returns the last-known synchronized baseline value for `key`, if any.
    #[inline]
    pub fn baseline(&self, key: &K) -> Option<&V> {
        self.baseline.get(key)
    }

    /// Returns the effective current local value for `key` (dirty local edit, else baseline).
    #[inline]
    pub fn local_value(&self, key: &K) -> Option<&V> {
        self.dirty_local
            .get(key)
            .map(|(v, _)| v)
            .or_else(|| self.baseline.get(key))
    }

    /// Records an optimistic local edit, advancing the local [`LogicalClock`].
    ///
    /// If `val` reverts back to the current `baseline`, the dirty entry is cleared.
    pub fn record_local(&mut self, key: K, val: V) -> LogicalClock {
        let clk = self.clock.tick();
        self.record_local_inner(key, val, clk);
        clk
    }

    /// Records an optimistic local edit stamped with an explicit external clock, merging `clock`
    /// into the local clock frontier.
    pub fn record_local_at(&mut self, key: K, val: V, clock: LogicalClock) -> LogicalClock {
        self.clock.merge(clock);
        self.record_local_inner(key, val, clock);
        clock
    }

    fn record_local_inner(&mut self, key: K, val: V, clk: LogicalClock) {
        if self.baseline.get(&key) == Some(&val) {
            self.dirty_local.remove(&key);
        } else {
            self.dirty_local.insert(key, (val, clk));
        }
    }

    #[inline]
    pub fn dirty_count(&self) -> usize {
        self.dirty_local.len()
    }

    #[inline]
    pub fn dirty_keys(&self) -> impl Iterator<Item = &K> {
        self.dirty_local.keys()
    }

    #[inline]
    pub fn dirty_entries(&self) -> impl Iterator<Item = (&K, &V, LogicalClock)> {
        self.dirty_local.iter().map(|(k, (v, c))| (k, v, *c))
    }

    /// Acknowledges that `val` for `key` has been durably synced upstream.
    ///
    /// Always advances `baseline` to `val`, and clears the dirty flag **only** if the current
    /// local value still equals `val`.
    pub fn ack_synced(&mut self, key: &K, val: &V) -> bool {
        self.baseline.insert(key.clone(), val.clone());
        match self.dirty_local.get(key) {
            Some((local_val, _)) if local_val == val => {
                self.dirty_local.remove(key);
                true
            }
            Some(_) => false,
            None => true,
        }
    }

    /// Reconciles a batch of incoming `(key, incoming_value, incoming_clock)` updates against
    /// `baseline` and `dirty_local` using `policy`.
    pub fn reconcile_incoming_batch(
        &mut self,
        entries: impl IntoIterator<Item = (K, V, Option<LogicalClock>)>,
        policy: ConflictPolicy,
    ) -> ReconcileBatch<K, V> {
        let mut batch = ReconcileBatch::default();

        for (key, incoming, incoming_clock) in entries {
            if let Some(ic) = incoming_clock {
                self.clock.merge(ic);
            }

            let base_opt = self.baseline.get(&key).cloned();
            let dirty_opt = self.dirty_local.get(&key).cloned();

            match (base_opt, dirty_opt) {
                (None, None) => {
                    self.baseline.insert(key.clone(), incoming.clone());
                    batch.to_apply_local.push((key.clone(), incoming.clone()));
                    batch.decisions.push(ReconcileDecision {
                        key,
                        kind: DivergenceKind::IncomingOnly,
                        resolved: incoming,
                        emit_upstream: false,
                        apply_local: true,
                    });
                }
                (None, Some((local, local_clk))) => {
                    self.baseline.insert(key.clone(), incoming.clone());
                    if local == incoming {
                        self.dirty_local.remove(&key);
                        batch.decisions.push(ReconcileDecision {
                            key,
                            kind: DivergenceKind::Converged,
                            resolved: local,
                            emit_upstream: false,
                            apply_local: false,
                        });
                    } else {
                        let decision = resolve_conflict(
                            key.clone(),
                            local,
                            incoming,
                            Some(local_clk),
                            incoming_clock,
                            policy,
                        );
                        if decision.apply_local {
                            self.dirty_local.remove(&key);
                            batch.to_apply_local.push((key, decision.resolved.clone()));
                        } else if decision.emit_upstream {
                            batch.to_emit_upstream.push((
                                key,
                                decision.resolved.clone(),
                                local_clk,
                            ));
                        }
                        batch.decisions.push(decision);
                    }
                }
                (Some(base), dirty) => {
                    let (local, local_clock) = match dirty {
                        Some((v, c)) => (v, Some(c)),
                        None => (base.clone(), None),
                    };
                    let decision = reconcile_three_way(
                        ThreeWayEntry {
                            key: key.clone(),
                            base,
                            local,
                            incoming: incoming.clone(),
                            local_clock,
                            incoming_clock,
                        },
                        policy,
                    );

                    match decision.kind {
                        DivergenceKind::Clean => {}
                        DivergenceKind::LocalOnlyDirty => {
                            let clk = local_clock.unwrap_or_else(|| self.clock.tick());
                            batch
                                .to_emit_upstream
                                .push((key, decision.resolved.clone(), clk));
                        }
                        DivergenceKind::IncomingOnly => {
                            self.baseline.insert(key.clone(), incoming);
                            batch.to_apply_local.push((key, decision.resolved.clone()));
                        }
                        DivergenceKind::Converged => {
                            self.baseline.insert(key.clone(), incoming);
                            self.dirty_local.remove(&key);
                        }
                        DivergenceKind::Conflict => {
                            self.baseline.insert(key.clone(), incoming);
                            if decision.apply_local {
                                self.dirty_local.remove(&key);
                                batch.to_apply_local.push((key, decision.resolved.clone()));
                            } else if decision.emit_upstream {
                                let clk = local_clock.unwrap_or_else(|| self.clock.tick());
                                batch
                                    .to_emit_upstream
                                    .push((key, decision.resolved.clone(), clk));
                            }
                        }
                    }

                    batch.decisions.push(decision);
                }
            }
        }

        batch
    }
}
