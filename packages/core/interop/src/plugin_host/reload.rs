use nve_abi::hash::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use nve_abi::{NveGetInterfaceFn, SurfaceId};

use crate::surface::{FrameworkKind, SurfaceEnvironment, SurfaceOrchestrator};

use super::manifest::{PluginError, PluginId, PluginMetadata, PluginRequirement};
use super::sandbox::NvePluginSlot;

/// Safe, typed registry hosting isolated `nve_get_interface` plugin slots with transactional
/// zero-downtime hot-swapping and generational [`SurfaceOrchestrator`] surface allocation.
pub struct NvePluginHost {
    next_plugin_id: PluginId,
    surfaces: SurfaceOrchestrator,
    slots: HashMap<PluginId, NvePluginSlot>,
    name_index: HashMap<String, PluginId>,
}

impl Default for NvePluginHost {
    fn default() -> Self {
        Self::new()
    }
}

impl NvePluginHost {
    pub fn new() -> Self {
        Self {
            next_plugin_id: 1,
            surfaces: SurfaceOrchestrator::new(),
            slots: HashMap::default(),
            name_index: HashMap::default(),
        }
    }

    /// Returns the underlying generational [`SurfaceOrchestrator`] managing plugin surface IDs.
    pub fn surfaces(&self) -> &SurfaceOrchestrator {
        &self.surfaces
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

    fn alloc_surface(&mut self, name: &str) -> Result<SurfaceId, PluginError> {
        self.surfaces
            .register_root(FrameworkKind::Nve, name, SurfaceEnvironment::default())
            .map_err(|_| PluginError::SurfaceSlotsExhausted)
    }

    /// Registers an in-process `nve_get_interface` plugin with ABI capability validation and a
    /// freshly allocated generational `SurfaceId`.
    pub fn register_in_process(
        &mut self,
        name: impl Into<String>,
        get_interface: NveGetInterfaceFn,
        req: PluginRequirement,
    ) -> Result<PluginId, PluginError> {
        let name = name.into();
        self.validate_name(&name)?;
        let plugin_id = self.next_plugin_id;
        let surface_id = self.alloc_surface(&name)?;

        let slot = match NvePluginSlot::from_in_process(
            plugin_id,
            name.clone(),
            surface_id,
            1,
            get_interface,
            req,
            None,
        ) {
            Ok(slot) => slot,
            Err(err) => {
                let _ = self.surfaces.destroy_surface(surface_id);
                return Err(err);
            }
        };
        self.next_plugin_id = self.next_plugin_id.saturating_add(1);
        self.name_index.insert(name, plugin_id);
        self.slots.insert(plugin_id, slot);
        Ok(plugin_id)
    }

    /// Loads a dynamic library (`cdylib`) exporting `nve_get_interface` into an isolated slot.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_dynamic(
        &mut self,
        name: impl Into<String>,
        dylib_path: &Path,
        req: PluginRequirement,
    ) -> Result<PluginId, PluginError> {
        let name = name.into();
        self.validate_name(&name)?;
        let plugin_id = self.next_plugin_id;
        let surface_id = self.alloc_surface(&name)?;

        let slot = match NvePluginSlot::from_dylib(
            plugin_id,
            name.clone(),
            surface_id,
            1,
            dylib_path,
            req,
            None,
        ) {
            Ok(slot) => slot,
            Err(err) => {
                let _ = self.surfaces.destroy_surface(surface_id);
                return Err(err);
            }
        };
        self.next_plugin_id = self.next_plugin_id.saturating_add(1);
        self.name_index.insert(name, plugin_id);
        self.slots.insert(plugin_id, slot);
        Ok(plugin_id)
    }

    /// Transactionally hot-swaps an existing plugin slot to a new in-process `nve_get_interface`
    /// implementation while preserving its isolated `SurfaceId` and snapshotted state.
    ///
    /// If the replacement fails ABI checks or returns a null handle, the existing plugin remains
    /// mounted and fully functional.
    pub fn hot_swap_in_process(
        &mut self,
        plugin_id: PluginId,
        new_get_interface: NveGetInterfaceFn,
        req: PluginRequirement,
    ) -> Result<u64, PluginError> {
        let (name, surface_id, next_gen, saved_state) = {
            let existing = self
                .slots
                .get_mut(&plugin_id)
                .ok_or(PluginError::PluginNotFound(plugin_id))?;
            let state = existing.snapshot_state();
            let meta = existing.metadata().clone();
            (meta.name, meta.surface_id, meta.generation + 1, state)
        };

        // Construct & validate the replacement BEFORE touching or dropping the existing slot!
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
    /// preserving its isolated `SurfaceId` and snapshotted state.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn hot_swap_dynamic(
        &mut self,
        plugin_id: PluginId,
        new_dylib_path: &Path,
        req: PluginRequirement,
    ) -> Result<u64, PluginError> {
        let (name, surface_id, next_gen, saved_state) = {
            let existing = self
                .slots
                .get_mut(&plugin_id)
                .ok_or(PluginError::PluginNotFound(plugin_id))?;
            let state = existing.snapshot_state();
            let meta = existing.metadata().clone();
            (meta.name, meta.surface_id, meta.generation + 1, state)
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

    /// Unloads and destroys a mounted plugin slot cleanly, retiring or bumping its surface generation.
    pub fn unload(&mut self, plugin_id: PluginId) -> Result<PluginMetadata, PluginError> {
        let slot = self
            .slots
            .remove(&plugin_id)
            .ok_or(PluginError::PluginNotFound(plugin_id))?;
        let meta = slot.metadata().clone();
        self.name_index.remove(&meta.name);
        drop(slot);
        let _ = self.surfaces.destroy_surface(meta.surface_id);
        Ok(meta)
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
}
