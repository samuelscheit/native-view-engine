use std::collections::VecDeque;

use nve_abi::hash::HashMap;
use nve_abi::SurfaceId;

use super::types::{
    EnvironmentOverrides, FrameworkKind, LifecycleState, SurfaceDescriptor, SurfaceEnvironment,
    SurfaceEvent, SurfaceRecord, TeardownRecord,
};
use crate::error::InteropError;
#[cfg(feature = "layout")]
use crate::layout::IntrinsicSize;

pub(crate) type TeardownHook = Box<dyn FnOnce(&TeardownRecord) + Send + 'static>;
pub(crate) type PendingTeardownHooks = Vec<(usize, TeardownHook)>;

/// Strongly typed dense slot index inside [`SurfaceOrchestrator::free`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurfaceSlotIndex(u32);

/// Compact hot slot header inside [`SurfaceOrchestrator`] carrying an 8-bit ABA generation tag
/// and a cold boxed [`SurfaceRecord`] (16 bytes total — 4 slots per 64-byte cache line).
pub(super) struct SurfaceSlot {
    pub(super) generation: u8,
    pub(super) record: Option<Box<SurfaceRecord>>,
}

const _: () = {
    assert!(core::mem::size_of::<SurfaceSlot>() <= 16);
};

impl SurfaceSlot {
    const fn empty() -> Self {
        Self {
            generation: 0,
            record: None,
        }
    }
}

/// Hierarchical dense-slab registry of surfaces spanning heterogeneous UI frameworks
/// (e.g. Flutter root -> NVE surface -> React Native Fabric island -> SwiftUI island).
///
/// Backed by a compact generational `Vec<SurfaceSlot>` + FIFO free queue (`index:24 | generation:8`,
/// slot `0` reserved sentinel so [`SurfaceId::NULL`] is never live) for O(1) lookup, insert,
/// destroy, and permanent ABA protection (slots whose generation reaches `u8::MAX` are retired
/// instead of wrapping back to `0`).
pub struct SurfaceOrchestrator {
    pub(super) slots: Vec<SurfaceSlot>,
    free: VecDeque<SurfaceSlotIndex>,
    teardown_hooks: HashMap<SurfaceId, TeardownHook>,
    roots: Vec<SurfaceId>,
    pub(super) events: Vec<SurfaceEvent>,
    pub(super) env_stack: Vec<(SurfaceId, usize)>,
    active_count: usize,
}

impl Default for SurfaceOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceOrchestrator {
    /// Stack buffer capacity used by [`Self::ancestor_chain_into`] and `GestureArbitrator`'s
    /// inline surface chain for zero-allocation pointer arbitration across realistic nesting depths.
    pub const INLINE_ANCESTOR_CAP: usize = 32;

    pub fn new() -> Self {
        Self {
            slots: vec![SurfaceSlot::empty()],
            free: VecDeque::new(),
            teardown_hooks: HashMap::default(),
            roots: Vec::new(),
            events: Vec::new(),
            env_stack: Vec::new(),
            active_count: 0,
        }
    }

    #[inline]
    fn slot_ref(&self, id: SurfaceId) -> Option<&SurfaceSlot> {
        let idx = id.index();
        if idx == 0 {
            return None;
        }
        let slot = self.slots.get(idx)?;
        (slot.generation == id.generation() && slot.record.is_some()).then_some(slot)
    }

    #[inline]
    fn slot_mut(&mut self, id: SurfaceId) -> Option<&mut SurfaceSlot> {
        let idx = id.index();
        if idx == 0 {
            return None;
        }
        let slot = self.slots.get_mut(idx)?;
        (slot.generation == id.generation() && slot.record.is_some()).then_some(slot)
    }

    #[inline]
    pub(super) fn get_mut(&mut self, id: SurfaceId) -> Option<&mut SurfaceRecord> {
        self.slot_mut(id)?.record.as_deref_mut()
    }

    pub(crate) fn allocate_slot_with_limit(
        &mut self,
        max_index: u32,
    ) -> Result<SurfaceId, InteropError> {
        if let Some(slot_idx) = self.free.pop_front() {
            let generation = self.slots[slot_idx.0 as usize].generation;
            Ok(SurfaceId::new(slot_idx.0, generation))
        } else {
            let idx_usize = self.slots.len();
            if idx_usize > max_index as usize {
                return Err(InteropError::SurfaceSlotsExhausted);
            }
            self.slots.push(SurfaceSlot::empty());
            Ok(SurfaceId::new(idx_usize as u32, 0))
        }
    }

    /// Registers a surface (root or child), inheriting environment from its parent when present
    /// and sealing the invariant `child.environment == child.overrides.apply(parent.environment)`.
    pub fn register_surface(&mut self, mut desc: SurfaceDescriptor) -> Result<SurfaceId, InteropError> {
        desc.overrides.validate()?;
        if let Some(ref env) = desc.environment {
            env.validate()?;
        }
        let resolved_env = match (desc.parent, desc.environment) {
            (Some(parent_id), Some(mut explicit_env)) => {
                let parent_env = &self
                    .get(parent_id)
                    .ok_or(InteropError::ParentNotFound(parent_id))?
                    .environment;
                desc.overrides.apply_to(&mut explicit_env);
                desc.overrides.seal_against_parent(&explicit_env, parent_env);
                explicit_env
            }
            (Some(parent_id), None) => {
                let mut env = self
                    .get(parent_id)
                    .ok_or(InteropError::ParentNotFound(parent_id))?
                    .environment
                    .clone();
                desc.overrides.apply_to(&mut env);
                env
            }
            (None, Some(mut explicit_env)) => {
                desc.overrides.apply_to(&mut explicit_env);
                // Roots have no parent to block cascades from; keep overrides empty so direct root
                // environment updates are never silently frozen.
                desc.overrides = EnvironmentOverrides::default();
                explicit_env
            }
            (None, None) => {
                let mut env = SurfaceEnvironment::default();
                desc.overrides.apply_to(&mut env);
                desc.overrides = EnvironmentOverrides::default();
                env
            }
        };

        let id = self.allocate_slot_with_limit(SurfaceId::MAX_INDEX)?;

        let record = Box::new(SurfaceRecord {
            id,
            parent: desc.parent,
            children: Vec::new(),
            framework: desc.framework,
            component_name: desc.component_name,
            state: LifecycleState::Mounted,
            environment: resolved_env,
            overrides: desc.overrides,
            #[cfg(feature = "layout")]
            viewport_size: desc.initial_size,
            #[cfg(feature = "window")]
            presentation: desc.presentation,
        });

        if let Some(parent_id) = desc.parent {
            if let Some(parent) = self.get_mut(parent_id) {
                parent.children.push(id);
            }
        } else {
            self.roots.push(id);
        }

        self.slots[id.index()].record = Some(record);
        self.active_count += 1;
        self.events.push(SurfaceEvent::Mounted {
            surface_id: id,
            parent: desc.parent,
            framework: desc.framework,
        });

        Ok(id)
    }

    /// Convenience helper to register a root surface.
    pub fn register_root(
        &mut self,
        framework: FrameworkKind,
        component_name: impl Into<String>,
        environment: SurfaceEnvironment,
    ) -> Result<SurfaceId, InteropError> {
        self.register_surface(
            SurfaceDescriptor::new(framework, component_name).with_environment(environment),
        )
    }

    /// Convenience helper to register a child surface under an existing parent.
    pub fn register_child(
        &mut self,
        parent: SurfaceId,
        framework: FrameworkKind,
        component_name: impl Into<String>,
    ) -> Result<SurfaceId, InteropError> {
        self.register_surface(SurfaceDescriptor::new(framework, component_name).with_parent(parent))
    }

    /// Registers a one-shot teardown callback invoked when `surface_id` is destroyed.
    pub fn register_teardown_hook<F>(
        &mut self,
        surface_id: SurfaceId,
        hook: F,
    ) -> Result<(), InteropError>
    where
        F: FnOnce(&TeardownRecord) + Send + 'static,
    {
        if self.slot_ref(surface_id).is_none() {
            return Err(InteropError::SurfaceNotFound(surface_id));
        }
        self.teardown_hooks.insert(surface_id, Box::new(hook));
        Ok(())
    }

    #[inline]
    pub fn get(&self, surface_id: SurfaceId) -> Option<&SurfaceRecord> {
        self.slot_ref(surface_id)?.record.as_deref()
    }

    #[inline]
    pub fn roots(&self) -> &[SurfaceId] {
        &self.roots
    }

    pub fn children(&self, surface_id: SurfaceId) -> Result<&[SurfaceId], InteropError> {
        self.get(surface_id)
            .map(|s| s.children.as_slice())
            .ok_or(InteropError::SurfaceNotFound(surface_id))
    }

    /// Writes the ancestor chain starting from `surface_id` up to its root into `out` with zero
    /// heap allocations.
    ///
    /// Returns `Ok(Some(len))` when the chain fits in `out`, `Ok(None)` if the hierarchy is deeper
    /// than `out.len()` (caller may fall back to [`Self::ancestor_chain`]), or
    /// `Err(InteropError::SurfaceNotFound)` if `surface_id` is missing/stale.
    #[inline]
    pub fn ancestor_chain_into(
        &self,
        surface_id: SurfaceId,
        out: &mut [SurfaceId],
    ) -> Result<Option<usize>, InteropError> {
        let mut rec = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        let mut len = 0usize;
        let mut current = surface_id;
        loop {
            if len == out.len() {
                return Ok(None);
            }
            out[len] = current;
            len += 1;
            match rec.parent {
                Some(parent_id) => match self.get(parent_id) {
                    Some(parent_rec) => {
                        current = parent_id;
                        rec = parent_rec;
                    }
                    None => break,
                },
                None => break,
            }
        }
        Ok(Some(len))
    }

    /// Returns the ancestor chain starting from `surface_id` itself up to the root.
    pub fn ancestor_chain(&self, surface_id: SurfaceId) -> Result<Vec<SurfaceId>, InteropError> {
        if self.get(surface_id).is_none() {
            return Err(InteropError::SurfaceNotFound(surface_id));
        }
        let mut chain = Vec::new();
        let mut cursor = Some(surface_id);
        while let Some(id) = cursor {
            chain.push(id);
            cursor = self.get(id).and_then(|r| r.parent);
        }
        Ok(chain)
    }

    /// Returns all descendants of `surface_id` in pre-order (excluding `surface_id`).
    pub fn descendants(&self, surface_id: SurfaceId) -> Result<Vec<SurfaceId>, InteropError> {
        let root = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        let mut out = Vec::new();
        let mut stack: Vec<SurfaceId> = root.children.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(node) = self.get(id) {
                for &child in node.children.iter().rev() {
                    stack.push(child);
                }
            }
        }
        Ok(out)
    }

    /// Active (non-destroyed) surface count.
    #[inline]
    pub fn active_count(&self) -> usize {
        self.active_count
    }

    /// Transitions a single surface to a non-destructive lifecycle state (`Resumed`/`Paused`).
    ///
    /// Rejects `LifecycleState::Destroyed` with [`InteropError::InvalidLifecycleTransition`] so
    /// destruction happens through one single mechanism ([`Self::destroy_surface`] or
    /// [`crate::InteropRuntime::destroy_surface_cascade`]).
    pub fn transition_state(
        &mut self,
        surface_id: SurfaceId,
        next_state: LifecycleState,
    ) -> Result<(), InteropError> {
        let record = self
            .get_mut(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        let prev = record.state;
        if next_state == LifecycleState::Destroyed || !prev.can_transition_to(next_state) {
            return Err(InteropError::InvalidLifecycleTransition {
                surface: surface_id,
                from: prev,
                to: next_state,
            });
        }
        if prev != next_state {
            record.state = next_state;
            self.events.push(SurfaceEvent::LifecycleChanged {
                surface_id,
                from: prev,
                to: next_state,
            });
        }
        Ok(())
    }

    /// Propagates a non-destructive lifecycle transition (`Resumed` or `Paused`) to `surface_id`
    /// and all descendants in pre-order.
    ///
    /// Rejects `LifecycleState::Destroyed` with [`InteropError::InvalidLifecycleTransition`]
    /// (use [`Self::destroy_surface`] or [`crate::InteropRuntime::destroy_surface_cascade`]).
    pub fn propagate_lifecycle(
        &mut self,
        surface_id: SurfaceId,
        next_state: LifecycleState,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let root_state = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .state;
        if next_state == LifecycleState::Destroyed || !root_state.can_transition_to(next_state) {
            return Err(InteropError::InvalidLifecycleTransition {
                surface: surface_id,
                from: root_state,
                to: next_state,
            });
        }

        let mut targets = vec![surface_id];
        targets.extend(self.descendants(surface_id)?);

        let mut transitioned = Vec::new();
        for id in targets {
            if let Some(rec) = self.get_mut(id)
                && rec.state != next_state
                && rec.state.can_transition_to(next_state)
            {
                let from = rec.state;
                rec.state = next_state;
                transitioned.push(id);
                self.events.push(SurfaceEvent::LifecycleChanged {
                    surface_id: id,
                    from,
                    to: next_state,
                });
            }
        }
        Ok(transitioned)
    }

    /// Updates a surface's viewport size after layout negotiation.
    #[cfg(feature = "layout")]
    pub fn set_viewport_size(
        &mut self,
        surface_id: SurfaceId,
        size: IntrinsicSize,
    ) -> Result<(), InteropError> {
        let rec = self
            .get_mut(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        rec.viewport_size = size;
        Ok(())
    }

    /// Reparents `surface_id` under `new_parent` (or promotes it to a root if `None`),
    /// validating existence and acyclicity before mutating any state.
    pub fn reparent(
        &mut self,
        surface_id: SurfaceId,
        new_parent: Option<SurfaceId>,
    ) -> Result<(), InteropError> {
        let old_parent = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .parent;

        if old_parent == new_parent {
            return Ok(());
        }

        let new_parent_env = if let Some(target_parent) = new_parent {
            let parent_rec = self
                .get(target_parent)
                .ok_or(InteropError::ParentNotFound(target_parent))?;
            let mut cursor = Some(target_parent);
            while let Some(ancestor_id) = cursor {
                if ancestor_id == surface_id {
                    return Err(InteropError::InvalidParentCycle {
                        surface: surface_id,
                        proposed_parent: target_parent,
                    });
                }
                cursor = self.get(ancestor_id).and_then(|r| r.parent);
            }
            Some(parent_rec.environment.clone())
        } else {
            None
        };

        // All validation passed; mutate tree links atomically.
        match old_parent {
            Some(old_pid) => {
                if let Some(old_p) = self.get_mut(old_pid) {
                    old_p.children.retain(|&c| c != surface_id);
                }
            }
            None => {
                self.roots.retain(|&r| r != surface_id);
            }
        }

        match new_parent {
            Some(new_pid) => {
                if let Some(new_p) = self.get_mut(new_pid) {
                    new_p.children.push(surface_id);
                }
            }
            None => {
                self.roots.push(surface_id);
            }
        }

        if let Some(rec) = self.get_mut(surface_id) {
            rec.parent = new_parent;
            if new_parent.is_none() {
                rec.overrides = EnvironmentOverrides::default();
            }
        }

        self.events.push(SurfaceEvent::Reparented {
            surface_id,
            old_parent,
            new_parent,
        });

        if let Some(env) = new_parent_env {
            self.propagate_env_iterative(surface_id, env, None);
        }

        Ok(())
    }

    /// Destroys `surface_id` and its entire subtree in strict bottom-up (post-order) sequence.
    ///
    /// All state mutations (parent/root detachment, slot retirement/recycling, `active_count`,
    /// hook extraction, and `Destroyed` events) finish before any user teardown hook runs. Every
    /// registered teardown hook in the destroyed subtree executes even if an earlier hook panics;
    /// the first panic (if any) is resumed only after all hooks have completed.
    pub fn destroy_surface(
        &mut self,
        surface_id: SurfaceId,
    ) -> Result<Vec<TeardownRecord>, InteropError> {
        let (records, pending_hooks) = self.destroy_surface_parts(surface_id)?;
        Self::fire_teardown_hooks(&records, pending_hooks);
        Ok(records)
    }

    /// Commits all orchestrator teardown mutations and returns the torn-down records plus deferred
    /// user teardown hooks so callers (such as [`crate::InteropRuntime::destroy_surface_cascade`])
    /// can clean up sibling subsystems before any user hook runs.
    pub(crate) fn destroy_surface_parts(
        &mut self,
        surface_id: SurfaceId,
    ) -> Result<(Vec<TeardownRecord>, PendingTeardownHooks), InteropError> {
        let parent_of_root = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .parent;

        // Collect post-order traversal iteratively (deepest leaves first, target root last).
        let order = self.collect_post_order_iterative(surface_id);

        // Detach the target surface from its parent or root list.
        match parent_of_root {
            Some(pid) => {
                if let Some(parent) = self.get_mut(pid) {
                    parent.children.retain(|&c| c != surface_id);
                }
            }
            None => {
                self.roots.retain(|&r| r != surface_id);
            }
        }

        let mut records = Vec::with_capacity(order.len());
        let mut pending_hooks: Vec<(usize, TeardownHook)> = Vec::new();
        for id in order {
            let idx = id.index();
            let rec = match self.slots.get_mut(idx) {
                Some(slot) if slot.generation == id.generation() => {
                    let rec = slot.record.take();
                    if rec.is_some() {
                        if let Some(next_gen) = slot.generation.checked_add(1) {
                            slot.generation = next_gen;
                            self.free.push_back(SurfaceSlotIndex(id.index_u32()));
                        }
                        debug_assert!(self.active_count > 0);
                        self.active_count -= 1;
                    }
                    rec
                }
                _ => None,
            };
            let hook = if self.teardown_hooks.is_empty() {
                None
            } else {
                self.teardown_hooks.remove(&id)
            };
            if let Some(mut rec) = rec {
                let previous_state = rec.state;
                rec.state = LifecycleState::Destroyed;
                let teardown = TeardownRecord {
                    surface_id: id,
                    framework: rec.framework,
                    component_name: rec.component_name,
                    previous_state,
                };
                self.events.push(SurfaceEvent::Destroyed {
                    surface_id: id,
                    framework: teardown.framework,
                });
                let rec_idx = records.len();
                records.push(teardown);
                if let Some(hook) = hook {
                    pending_hooks.push((rec_idx, hook));
                }
            }
        }

        Ok((records, pending_hooks))
    }

    /// Executes every deferred teardown hook in bottom-up order, isolating panics so subsequent
    /// hooks always run, then resumes the first caught panic (if any) once all hooks have run.
    pub(crate) fn fire_teardown_hooks(
        records: &[TeardownRecord],
        pending_hooks: PendingTeardownHooks,
    ) {
        let mut first_panic: Option<Box<dyn std::any::Any + Send>> = None;
        for (rec_idx, hook) in pending_hooks {
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hook(&records[rec_idx]);
            })) && first_panic.is_none()
            {
                first_panic = Some(payload);
            }
        }
        if let Some(payload) = first_panic {
            std::panic::resume_unwind(payload);
        }
    }

    fn collect_post_order_iterative(&self, root: SurfaceId) -> Vec<SurfaceId> {
        let mut out = Vec::new();
        let mut stack: Vec<(SurfaceId, bool)> = vec![(root, false)];
        while let Some((id, visited)) = stack.pop() {
            if visited {
                out.push(id);
            } else if let Some(rec) = self.get(id) {
                stack.push((id, true));
                for &child in rec.children.iter().rev() {
                    stack.push((child, false));
                }
            }
        }
        out
    }

    /// Drains all orchestrator events accumulated since the last drain.
    pub fn drain_events(&mut self) -> Vec<SurfaceEvent> {
        std::mem::take(&mut self.events)
    }
}
