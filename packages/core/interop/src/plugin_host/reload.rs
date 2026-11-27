use nve_abi::hash::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use nve_abi::{NveGetInterfaceFn, SurfaceId};

use crate::error::InteropError;
use crate::surface::{FrameworkKind, SurfaceEnvironment, SurfaceOrchestrator};

use super::manifest::{
    resolve_and_validate_tables, PluginError, PluginId, PluginMetadata, PluginRequirement,
    PluginSourceKind,
};
use super::sandbox::NvePluginSlot;

/// Safe, typed registry hosting isolated `nve_get_interface` plugin slots with transactional
/// zero-downtime hot-swapping over [`crate::InteropRuntime`]'s shared [`SurfaceOrchestrator`].
///
/// Mutating lifecycle operations (`register`, `load`, `hot_swap`, `unload`) are intentionally
/// routed exclusively through [`crate::InteropRuntime`] so plugins always share a single
/// [`SurfaceOrchestrator`] and cascade teardown stays strictly synchronized.
pub struct NvePluginHost {
    next_plugin_id: PluginId,
    slots: HashMap<PluginId, NvePluginSlot>,
    name_index: HashMap<String, PluginId>,
    surface_index: HashMap<SurfaceId, PluginId>,
}

impl NvePluginHost {
    pub(crate) fn new() -> Self {
        Self {
            next_plugin_id: 1,
            slots: HashMap::default(),
            name_index: HashMap::default(),
            surface_index: HashMap::default(),
        }
    }

    fn validate_name(&self, name: &str) -> Result<(), PluginError> {
        if name.trim().is_empty() {
            return Err(PluginError::InvalidPluginName(name.to_owned()));
        }
        if self.name_index.contains_key(name) {
            return Err(PluginError::DuplicatePluginName(name.to_owned()));
        }
        Ok(())
    }

    fn alloc_surface(
        surfaces: &mut SurfaceOrchestrator,
        name: &str,
    ) -> Result<SurfaceId, PluginError> {
        surfaces
            .register_root(FrameworkKind::Nve, name, SurfaceEnvironment::default())
            .map_err(|err| match err {
                InteropError::SurfaceSlotsExhausted => PluginError::SurfaceSlotsExhausted,
                other => PluginError::SurfaceAllocationFailed(other.to_string()),
            })
    }

    /// Registers an in-process `nve_get_interface` plugin onto the shared `surfaces` orchestrator.
    ///
    /// Interface resolution and capability checks run BEFORE allocating a surface slot, so
    /// pre-create failures burn 0 surface slots/generations and emit 0 orchestrator events.
    pub(crate) fn register_in_process(
        &mut self,
        surfaces: &mut SurfaceOrchestrator,
        name: impl Into<String>,
        get_interface: NveGetInterfaceFn,
        req: PluginRequirement,
    ) -> Result<PluginId, PluginError> {
        let name = name.into();
        self.validate_name(&name)?;
        let plugin_id = self.next_plugin_id;
        let next_id = plugin_id
            .checked_add(1)
            .ok_or(PluginError::PluginIdsExhausted)?;

        // Validate ABI tables BEFORE reserving a generational SurfaceId.
        let tables = resolve_and_validate_tables(get_interface, req)?;
        let surface_id = Self::alloc_surface(surfaces, &name)?;

        let slot = match NvePluginSlot::from_resolved_tables(
            plugin_id,
            name.clone(),
            surface_id,
            1,
            tables,
            None,
            PluginSourceKind::InProcess,
            #[cfg(not(target_arch = "wasm32"))]
            None,
        ) {
            Ok(slot) => slot,
            Err(err) => {
                let destroy_res = surfaces.destroy_surface(surface_id);
                debug_assert!(
                    destroy_res.is_ok(),
                    "rollback destroy_surface({surface_id}) failed: {destroy_res:?}"
                );
                return Err(err);
            }
        };
        self.next_plugin_id = next_id;
        self.name_index.insert(name, plugin_id);
        self.surface_index.insert(surface_id, plugin_id);
        self.slots.insert(plugin_id, slot);
        Ok(plugin_id)
    }

    /// Loads a dynamic library (`cdylib`) exporting `nve_get_interface` into an isolated slot
    /// backed by the shared `surfaces` orchestrator.
    ///
    /// Library loading, symbol lookup, and capability checks run BEFORE allocating a surface slot,
    /// so pre-create failures burn 0 surface slots/generations and emit 0 orchestrator events.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn load_dynamic(
        &mut self,
        surfaces: &mut SurfaceOrchestrator,
        name: impl Into<String>,
        dylib_path: &Path,
        req: PluginRequirement,
    ) -> Result<PluginId, PluginError> {
        let name = name.into();
        self.validate_name(&name)?;
        let plugin_id = self.next_plugin_id;
        let next_id = plugin_id
            .checked_add(1)
            .ok_or(PluginError::PluginIdsExhausted)?;

        // Open dylib and validate ABI tables BEFORE reserving a generational SurfaceId.
        let (staged, tables) =
            NvePluginSlot::open_and_validate_dylib(plugin_id, 1, dylib_path, req)?;
        let surface_id = Self::alloc_surface(surfaces, &name)?;

        let slot = match NvePluginSlot::from_resolved_tables(
            plugin_id,
            name.clone(),
            surface_id,
            1,
            tables,
            None,
            PluginSourceKind::DynamicLibrary(dylib_path.to_path_buf()),
            Some(staged),
        ) {
            Ok(slot) => slot,
            Err(err) => {
                let destroy_res = surfaces.destroy_surface(surface_id);
                debug_assert!(
                    destroy_res.is_ok(),
                    "rollback destroy_surface({surface_id}) failed: {destroy_res:?}"
                );
                return Err(err);
            }
        };
        self.next_plugin_id = next_id;
        self.name_index.insert(name, plugin_id);
        self.surface_index.insert(surface_id, plugin_id);
        self.slots.insert(plugin_id, slot);
        Ok(plugin_id)
    }

    /// Transactionally hot-swaps an existing plugin slot to a new in-process `nve_get_interface`
    /// implementation while verifying its surface is still live and preserving snapshotted state.
    pub(crate) fn hot_swap_in_process(
        &mut self,
        surfaces: &SurfaceOrchestrator,
        plugin_id: PluginId,
        new_get_interface: NveGetInterfaceFn,
        req: PluginRequirement,
    ) -> Result<u64, PluginError> {
        let (name, surface_id, next_gen, saved_state) = {
            let existing = self
                .slots
                .get_mut(&plugin_id)
                .ok_or(PluginError::PluginNotFound(plugin_id))?;
            let surface_id = existing.surface_id();
            if surfaces.get(surface_id).is_none() {
                return Err(PluginError::SurfaceNotLive(surface_id));
            }
            let next_gen = existing
                .generation()
                .checked_add(1)
                .ok_or(PluginError::GenerationExhausted(plugin_id))?;
            let name = existing.metadata().name.clone();
            let state = existing.snapshot_state();
            (name, surface_id, next_gen, state)
        };

        let replacement = NvePluginSlot::from_in_process(
            plugin_id,
            name,
            surface_id,
            next_gen,
            new_get_interface,
            req,
            Some(&saved_state),
        )?;

        let old = self.slots.insert(plugin_id, replacement);
        drop(old);
        Ok(next_gen)
    }

    /// Transactionally hot-swaps an existing plugin slot to a freshly built `cdylib` while
    /// verifying its surface is still live and preserving snapshotted state.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn hot_swap_dynamic(
        &mut self,
        surfaces: &SurfaceOrchestrator,
        plugin_id: PluginId,
        new_dylib_path: &Path,
        req: PluginRequirement,
    ) -> Result<u64, PluginError> {
        let (name, surface_id, next_gen, saved_state) = {
            let existing = self
                .slots
                .get_mut(&plugin_id)
                .ok_or(PluginError::PluginNotFound(plugin_id))?;
            let surface_id = existing.surface_id();
            if surfaces.get(surface_id).is_none() {
                return Err(PluginError::SurfaceNotLive(surface_id));
            }
            let next_gen = existing
                .generation()
                .checked_add(1)
                .ok_or(PluginError::GenerationExhausted(plugin_id))?;
            let name = existing.metadata().name.clone();
            let state = existing.snapshot_state();
            (name, surface_id, next_gen, state)
        };

        let replacement = NvePluginSlot::from_dylib(
            plugin_id,
            name,
            surface_id,
            next_gen,
            new_dylib_path,
            req,
            Some(&saved_state),
        )?;

        let old = self.slots.insert(plugin_id, replacement);
        drop(old);
        Ok(next_gen)
    }

    /// Removes and drops a mounted plugin slot and clears its indices, returning its metadata
    /// so the caller (`InteropRuntime::unload_plugin`) can run full surface cascade teardown.
    pub(crate) fn take_for_unload(&mut self, plugin_id: PluginId) -> Result<PluginMetadata, PluginError> {
        let slot = self
            .slots
            .remove(&plugin_id)
            .ok_or(PluginError::PluginNotFound(plugin_id))?;
        let meta = slot.metadata().clone();
        self.name_index.remove(&meta.name);
        self.surface_index.remove(&meta.surface_id);
        drop(slot);
        Ok(meta)
    }

    /// Removes any plugin slot bound to an already-destroyed surface in O(1) time (called by
    /// `InteropRuntime::destroy_surface_cascade`).
    pub(crate) fn detach_destroyed_surface(&mut self, surface_id: SurfaceId) {
        if let Some(pid) = self.surface_index.remove(&surface_id)
            && let Some(slot) = self.slots.remove(&pid)
        {
            self.name_index.remove(&slot.metadata().name);
            drop(slot);
        }
    }

    pub fn get(&self, plugin_id: PluginId) -> Option<&NvePluginSlot> {
        self.slots.get(&plugin_id)
    }

    pub fn get_mut(&mut self, plugin_id: PluginId) -> Option<&mut NvePluginSlot> {
        self.slots.get_mut(&plugin_id)
    }

    pub fn find_by_name(&self, name: &str) -> Option<PluginId> {
        self.name_index.get(name).copied()
    }

    pub fn find_by_surface(&self, surface_id: SurfaceId) -> Option<PluginId> {
        self.surface_index.get(&surface_id).copied()
    }

    pub fn active_count(&self) -> usize {
        self.slots.len()
    }

    /// Lists all mounted plugins sorted deterministically by `plugin_id`.
    pub fn list(&self) -> Vec<PluginMetadata> {
        let mut out: Vec<PluginMetadata> =
            self.slots.values().map(|s| s.metadata().clone()).collect();
        out.sort_by_key(|m| m.plugin_id);
        out
    }

    #[cfg(test)]
    pub(super) fn set_next_plugin_id_for_test(&mut self, next: PluginId) {
        self.next_plugin_id = next;
    }
}
