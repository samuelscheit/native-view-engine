use nve_abi::SurfaceId;

use super::types::{
    EnvironmentOverrides, FrameworkKind, LifecycleState, SafeAreaInsets, SurfaceDescriptor,
    SurfaceEnvironment, SurfaceEvent, SurfaceRecord, TeardownRecord, ThemeContext,
};
use crate::error::InteropError;
#[cfg(feature = "layout")]
use crate::layout::IntrinsicSize;

type TeardownHook = Box<dyn FnOnce(&TeardownRecord) + Send + 'static>;

/// One dense slot inside [`SurfaceOrchestrator`] carrying an 8-bit ABA generation tag.
struct SurfaceSlot {
    generation: u8,
    record: Option<SurfaceRecord>,
    teardown_hook: Option<TeardownHook>,
}

impl SurfaceSlot {
    const fn empty() -> Self {
        Self {
            generation: 0,
            record: None,
            teardown_hook: None,
        }
    }
}

/// Hierarchical dense-slab registry of surfaces spanning heterogeneous UI frameworks
/// (e.g. Flutter root -> NVE surface -> React Native Fabric island -> SwiftUI island).
///
/// Backed by a flat generational `Vec<SurfaceSlot>` + free list (`index:24 | generation:8`,
/// slot `0` reserved sentinel so [`SurfaceId::NULL`] is never live) for O(1) lookup, insert,
/// destroy, and ABA stale-handle rejection with zero tree-node allocations.
pub struct SurfaceOrchestrator {
    slots: Vec<SurfaceSlot>,
    free: Vec<u32>,
    roots: Vec<SurfaceId>,
    events: Vec<SurfaceEvent>,
    active_count: usize,
}

impl Default for SurfaceOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceOrchestrator {
    pub fn new() -> Self {
        Self {
            slots: vec![SurfaceSlot::empty()],
            free: Vec::new(),
            roots: Vec::new(),
            events: Vec::new(),
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
    fn get_mut(&mut self, id: SurfaceId) -> Option<&mut SurfaceRecord> {
        self.slot_mut(id)?.record.as_mut()
    }

    /// Registers a surface (root or child), inheriting environment from its parent when present.
    pub fn register_surface(&mut self, desc: SurfaceDescriptor) -> Result<SurfaceId, InteropError> {
        let inherited_env = match desc.parent {
            Some(parent_id) => {
                let parent = self
                    .get(parent_id)
                    .ok_or(InteropError::ParentNotFound(parent_id))?;
                if parent.state == LifecycleState::Destroyed {
                    return Err(InteropError::SurfaceDestroyed(parent_id));
                }
                parent.environment.clone()
            }
            None => SurfaceEnvironment::default(),
        };

        let id = match desc.id {
            Some(explicit) => {
                let idx = explicit.index();
                if explicit.is_null() || idx == 0 {
                    return Err(InteropError::InvalidSurfaceId(explicit));
                }
                if idx < self.slots.len() {
                    if self.slots[idx].record.is_some() {
                        return Err(InteropError::DuplicateSurface(explicit));
                    }
                    if let Some(pos) = self.free.iter().position(|&f| f == idx as u32) {
                        self.free.swap_remove(pos);
                    }
                } else {
                    let old_len = self.slots.len();
                    self.slots.resize_with(idx + 1, SurfaceSlot::empty);
                    for gap in (old_len..idx).rev() {
                        self.free.push(gap as u32);
                    }
                }
                self.slots[idx].generation = explicit.generation();
                explicit
            }
            None => {
                if let Some(idx) = self.free.pop() {
                    let generation = self.slots[idx as usize].generation;
                    SurfaceId::new(idx, generation)
                } else {
                    let idx_usize = self.slots.len();
                    if idx_usize > SurfaceId::MAX_INDEX as usize {
                        return Err(InteropError::InvalidSurfaceId(SurfaceId::NULL));
                    }
                    self.slots.push(SurfaceSlot::empty());
                    SurfaceId::new(idx_usize as u32, 0)
                }
            }
        };

        let mut resolved_env = desc.environment.unwrap_or(inherited_env);
        apply_overrides(&mut resolved_env, &desc.overrides);

        let record = SurfaceRecord {
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
        };

        if let Some(parent_id) = desc.parent {
            if let Some(parent) = self.get_mut(parent_id) {
                parent.children.push(id);
            }
        } else {
            self.roots.push(id);
        }

        self.slots[id.index()].record = Some(record);
        self.slots[id.index()].teardown_hook = None;
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
        let slot = self
            .slot_mut(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        if slot
            .record
            .as_ref()
            .is_some_and(|r| r.state == LifecycleState::Destroyed)
        {
            return Err(InteropError::SurfaceDestroyed(surface_id));
        }
        slot.teardown_hook = Some(Box::new(hook));
        Ok(())
    }

    #[inline]
    pub fn get(&self, surface_id: SurfaceId) -> Option<&SurfaceRecord> {
        self.slot_ref(surface_id)?.record.as_ref()
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

    /// Transitions a single surface to `next_state` (for `Destroyed`, use [`Self::destroy_surface`]).
    pub fn transition_state(
        &mut self,
        surface_id: SurfaceId,
        next_state: LifecycleState,
    ) -> Result<(), InteropError> {
        if next_state == LifecycleState::Destroyed {
            self.destroy_surface(surface_id)?;
            return Ok(());
        }
        let record = self
            .get_mut(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        let prev = record.state;
        if !prev.can_transition_to(next_state) {
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

    /// Propagates a lifecycle transition (`Resumed` or `Paused`) to `surface_id` and all descendants.
    pub fn propagate_lifecycle(
        &mut self,
        surface_id: SurfaceId,
        next_state: LifecycleState,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        if next_state == LifecycleState::Destroyed {
            let torn_down = self.destroy_surface(surface_id)?;
            return Ok(torn_down.into_iter().map(|r| r.surface_id).collect());
        }
        let root_state = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .state;
        if !root_state.can_transition_to(next_state) {
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
                && rec.state != LifecycleState::Destroyed
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
        if rec.state == LifecycleState::Destroyed {
            return Err(InteropError::SurfaceDestroyed(surface_id));
        }
        rec.viewport_size = size;
        Ok(())
    }

    /// Updates the environment at `surface_id` and propagates it down the subtree,
    /// respecting each descendant's [`EnvironmentOverrides`].
    pub fn update_environment(
        &mut self,
        surface_id: SurfaceId,
        environment: SurfaceEnvironment,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let rec = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        if rec.state == LifecycleState::Destroyed {
            return Err(InteropError::SurfaceDestroyed(surface_id));
        }

        let mut updated = Vec::new();
        self.propagate_env_recursive(surface_id, &environment, &mut updated);
        Ok(updated)
    }

    fn propagate_env_recursive(
        &mut self,
        surface_id: SurfaceId,
        incoming: &SurfaceEnvironment,
        updated: &mut Vec<SurfaceId>,
    ) {
        let (effective, children) = {
            let Some(rec) = self.get_mut(surface_id) else {
                return;
            };
            let mut next = incoming.clone();
            apply_overrides(&mut next, &rec.overrides);
            let changed = rec.environment != next;
            if changed {
                rec.environment = next.clone();
            }
            let effective = rec.environment.clone();
            let children = rec.children.clone();
            if changed {
                updated.push(surface_id);
                self.events.push(SurfaceEvent::EnvironmentPropagated {
                    surface_id,
                    environment: next,
                });
            }
            (effective, children)
        };

        for child_id in children {
            self.propagate_env_recursive(child_id, &effective, updated);
        }
    }

    /// Convenience helper to update only the theme at `surface_id` and cascade to children.
    pub fn set_theme(
        &mut self,
        surface_id: SurfaceId,
        theme: ThemeContext,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let mut env = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .clone();
        env.theme = theme;
        self.update_environment(surface_id, env)
    }

    /// Convenience helper to update safe-area insets at `surface_id` and cascade to children.
    pub fn set_safe_area(
        &mut self,
        surface_id: SurfaceId,
        safe_area: SafeAreaInsets,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let mut env = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .clone();
        env.safe_area = safe_area;
        self.update_environment(surface_id, env)
    }

    /// Convenience helper to update display scale factor at `surface_id` and cascade to children.
    pub fn set_scale_factor(
        &mut self,
        surface_id: SurfaceId,
        scale_factor: f32,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let mut env = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .clone();
        env.scale_factor = scale_factor;
        self.update_environment(surface_id, env)
    }

    /// Reparents `surface_id` under `new_parent` (or promotes it to a root if `None`),
    /// preventing cycles and re-propagating environment from the new parent.
    pub fn reparent(
        &mut self,
        surface_id: SurfaceId,
        new_parent: Option<SurfaceId>,
    ) -> Result<(), InteropError> {
        let old_parent = {
            let rec = self
                .get(surface_id)
                .ok_or(InteropError::SurfaceNotFound(surface_id))?;
            if rec.state == LifecycleState::Destroyed {
                return Err(InteropError::SurfaceDestroyed(surface_id));
            }
            rec.parent
        };

        if old_parent == new_parent {
            return Ok(());
        }

        if let Some(target_parent) = new_parent {
            let parent_rec = self
                .get(target_parent)
                .ok_or(InteropError::ParentNotFound(target_parent))?;
            if parent_rec.state == LifecycleState::Destroyed {
                return Err(InteropError::SurfaceDestroyed(target_parent));
            }
            let ancestors = self.ancestor_chain(target_parent)?;
            if ancestors.contains(&surface_id) {
                return Err(InteropError::InvalidParentCycle {
                    surface: surface_id,
                    proposed_parent: target_parent,
                });
            }
        }

        // Detach from old parent / root list.
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

        // Attach to new parent / root list.
        let new_parent_env = match new_parent {
            Some(new_pid) => {
                let Some(new_p) = self.get_mut(new_pid) else {
                    return Err(InteropError::ParentNotFound(new_pid));
                };
                new_p.children.push(surface_id);
                Some(new_p.environment.clone())
            }
            None => {
                self.roots.push(surface_id);
                None
            }
        };

        if let Some(rec) = self.get_mut(surface_id) {
            rec.parent = new_parent;
        }

        self.events.push(SurfaceEvent::Reparented {
            surface_id,
            old_parent,
            new_parent,
        });

        if let Some(env) = new_parent_env {
            let mut updated = Vec::new();
            self.propagate_env_recursive(surface_id, &env, &mut updated);
        }

        Ok(())
    }

    /// Destroys `surface_id` and its entire subtree in strict bottom-up (post-order) sequence,
    /// firing teardown hooks, detaching from parents, bumping slot generations, and recycling
    /// slots into the free list so zero leaks or ABA aliasing can occur.
    pub fn destroy_surface(
        &mut self,
        surface_id: SurfaceId,
    ) -> Result<Vec<TeardownRecord>, InteropError> {
        let parent_of_root = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .parent;

        // Collect post-order traversal (deepest leaves first, target root last).
        let mut order = Vec::new();
        self.collect_post_order(surface_id, &mut order);

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
        for id in order {
            let idx = id.index();
            let (rec, hook) = match self.slots.get_mut(idx) {
                Some(slot) if slot.generation == id.generation() => {
                    let rec = slot.record.take();
                    let hook = slot.teardown_hook.take();
                    if rec.is_some() {
                        slot.generation = slot.generation.wrapping_add(1);
                        self.free.push(id.index_u32());
                        self.active_count = self.active_count.saturating_sub(1);
                    }
                    (rec, hook)
                }
                _ => (None, None),
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
                if let Some(hook) = hook {
                    hook(&teardown);
                }
                self.events.push(SurfaceEvent::Destroyed {
                    surface_id: id,
                    framework: teardown.framework,
                });
                records.push(teardown);
            }
        }

        Ok(records)
    }

    fn collect_post_order(&self, surface_id: SurfaceId, out: &mut Vec<SurfaceId>) {
        if let Some(rec) = self.get(surface_id) {
            for &child in &rec.children {
                self.collect_post_order(child, out);
            }
            out.push(surface_id);
        }
    }

    /// Drains all orchestrator events accumulated since the last drain.
    pub fn drain_events(&mut self) -> Vec<SurfaceEvent> {
        std::mem::take(&mut self.events)
    }
}

fn apply_overrides(env: &mut SurfaceEnvironment, overrides: &EnvironmentOverrides) {
    if let Some(theme) = &overrides.theme {
        env.theme = theme.clone();
    }
    if let Some(safe_area) = overrides.safe_area {
        env.safe_area = safe_area;
    }
    if let Some(scale) = overrides.scale_factor {
        env.scale_factor = scale;
    }
    if let Some(locale) = &overrides.locale {
        env.locale = locale.clone();
    }
}
