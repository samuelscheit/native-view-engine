//! Safe, typed native-speed plugin host registry (`NvePluginHost`) built directly on NVE's
//! modular `nve_get_interface(uint32_t)` C ABI.
//!
//! Supports both dynamic `cdylib` plugins (`load_dynamic` / `hot_swap_dynamic`) and zero-copy
//! in-process `NveGetInterfaceFn` plugins (`register_in_process` / `hot_swap_in_process`) with:
//! - Strict ABI capability verification (`NVE_IFACE_SURFACE` mandatory; optional `NVE_IFACE_LAYOUT`,
//!   `NVE_IFACE_COMPONENT`, `NVE_IFACE_DEV`).
//! - Strictly isolated `SurfaceId` allocation per plugin slot.
//! - Crash-safe transactional hot-swap: snapshots old plugin state, validates and instantiates the
//!   replacement plugin BEFORE dropping the old instance, restoring state onto the same isolated
//!   `SurfaceId` — if the new plugin fails ABI checks or returns a null handle, the host stays
//!   alive with the previous plugin untouched.

use std::collections::HashMap;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(target_arch = "wasm32"))]
use libloading::Library;
use nve_abi::{
    FrameId, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent,
    NveEventBatch, NveGestureDisposition, NveGetInterfaceFn, NveIntrinsicSize, NveLayoutApi,
    NveMapEntry, NveSizeConstraints, NveSurfaceApi, SurfaceId, NVE_IFACE_COMPONENT, NVE_IFACE_DEV,
    NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};

use crate::error::InteropError;

/// Unique handle identifying a mounted plugin inside [`NvePluginHost`].
pub type PluginId = u32;

/// Default starting `SurfaceId` reserved for isolated plugin slots.
pub const DEFAULT_PLUGIN_SURFACE_BASE: SurfaceId = 10_000;

static TEMP_COPY_SEQ: AtomicU64 = AtomicU64::new(1);

/// Probed ABI capability bitmap discovered via `nve_get_interface(uint32_t)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PluginCapabilities {
    pub surface: bool,
    pub layout: bool,
    pub component: bool,
    pub dev: bool,
    pub direct_events: bool,
    pub direct_props: bool,
}

/// Optional interface requirements enforced when mounting or hot-swapping a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PluginRequirement {
    pub require_layout: bool,
    pub require_component: bool,
    pub require_dev: bool,
}

impl PluginRequirement {
    /// Only the core `NVE_IFACE_SURFACE` interface is required.
    pub const SURFACE_ONLY: Self = Self {
        require_layout: false,
        require_component: false,
        require_dev: false,
    };

    /// Requires both `NVE_IFACE_SURFACE` and `NVE_IFACE_COMPONENT` (for prop-driven plugins).
    pub const EMBEDDED_COMPONENT: Self = Self {
        require_layout: false,
        require_component: true,
        require_dev: false,
    };
}

/// Describes how a plugin slot was loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSourceKind {
    InProcess,
    DynamicLibrary(PathBuf),
}

/// Immutable descriptor of a mounted plugin slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMetadata {
    pub plugin_id: PluginId,
    pub name: String,
    pub surface_id: SurfaceId,
    pub generation: u64,
    pub capabilities: PluginCapabilities,
    pub source: PluginSourceKind,
}

/// Errors produced by [`NvePluginHost`] and [`NvePluginSlot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    MissingSurfaceInterface,
    MissingRequiredInterface {
        interface_name: &'static str,
        interface_id: u32,
    },
    NullHandleReturned,
    PluginNotFound(PluginId),
    DuplicatePluginName(String),
    InvalidPluginName(String),
    UnsupportedCapability(&'static str),
    LibraryLoadFailed(String),
    SymbolNotFound(String),
    Io(String),
    PluginPanicked(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSurfaceInterface => {
                f.write_str("plugin did not export required NVE_IFACE_SURFACE (1)")
            }
            Self::MissingRequiredInterface {
                interface_name,
                interface_id,
            } => write!(
                f,
                "plugin missing required interface {interface_name} ({interface_id})"
            ),
            Self::NullHandleReturned => {
                f.write_str("plugin NveSurfaceApi::create returned a null handle")
            }
            Self::PluginNotFound(id) => write!(f, "plugin {id} not found"),
            Self::DuplicatePluginName(name) => write!(f, "plugin '{name}' is already registered"),
            Self::InvalidPluginName(name) => write!(f, "invalid plugin name: '{name}'"),
            Self::UnsupportedCapability(cap) => write!(f, "plugin does not support '{cap}'"),
            Self::LibraryLoadFailed(msg) => write!(f, "failed to load plugin library: {msg}"),
            Self::SymbolNotFound(msg) => write!(f, "missing nve_get_interface symbol: {msg}"),
            Self::Io(msg) => write!(f, "plugin I/O error: {msg}"),
            Self::PluginPanicked(msg) => write!(f, "plugin panicked across ABI boundary: {msg}"),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<PluginError> for InteropError {
    fn from(err: PluginError) -> Self {
        Self::Plugin(err.to_string())
    }
}

/// Resolved function-pointer table set for one loaded plugin.
struct ResolvedTables {
    surface: NveSurfaceApi,
    layout: Option<NveLayoutApi>,
    component: Option<NveComponentApi>,
    dev: Option<NveDevApi>,
    capabilities: PluginCapabilities,
}

fn resolve_and_validate_tables(
    get_iface: NveGetInterfaceFn,
    req: PluginRequirement,
) -> Result<ResolvedTables, PluginError> {
    let surface_ptr = catch_unwind(AssertUnwindSafe(|| unsafe {
        get_iface(NVE_IFACE_SURFACE) as *const NveSurfaceApi
    }))
    .map_err(|_| PluginError::PluginPanicked("nve_get_interface(NVE_IFACE_SURFACE)".into()))?;

    if surface_ptr.is_null() {
        return Err(PluginError::MissingSurfaceInterface);
    }
    let surface = unsafe { *surface_ptr };

    let layout = {
        let p = catch_unwind(AssertUnwindSafe(|| unsafe {
            get_iface(NVE_IFACE_LAYOUT) as *const NveLayoutApi
        }))
        .map_err(|_| PluginError::PluginPanicked("nve_get_interface(NVE_IFACE_LAYOUT)".into()))?;
        (!p.is_null()).then(|| unsafe { *p })
    };
    if req.require_layout && layout.is_none() {
        return Err(PluginError::MissingRequiredInterface {
            interface_name: "NVE_IFACE_LAYOUT",
            interface_id: NVE_IFACE_LAYOUT,
        });
    }

    let component = {
        let p = catch_unwind(AssertUnwindSafe(|| unsafe {
            get_iface(NVE_IFACE_COMPONENT) as *const NveComponentApi
        }))
        .map_err(|_| {
            PluginError::PluginPanicked("nve_get_interface(NVE_IFACE_COMPONENT)".into())
        })?;
        (!p.is_null()).then(|| unsafe { *p })
    };
    if req.require_component && component.is_none() {
        return Err(PluginError::MissingRequiredInterface {
            interface_name: "NVE_IFACE_COMPONENT",
            interface_id: NVE_IFACE_COMPONENT,
        });
    }

    let dev = {
        let p = catch_unwind(AssertUnwindSafe(|| unsafe {
            get_iface(NVE_IFACE_DEV) as *const NveDevApi
        }))
        .map_err(|_| PluginError::PluginPanicked("nve_get_interface(NVE_IFACE_DEV)".into()))?;
        (!p.is_null()).then(|| unsafe { *p })
    };
    if req.require_dev && dev.is_none() {
        return Err(PluginError::MissingRequiredInterface {
            interface_name: "NVE_IFACE_DEV",
            interface_id: NVE_IFACE_DEV,
        });
    }

    let capabilities = PluginCapabilities {
        surface: true,
        layout: layout.is_some(),
        component: component.is_some(),
        dev: dev.is_some(),
        direct_events: surface.dispatch_events_direct.is_some(),
        direct_props: component.and_then(|c| c.set_props_direct).is_some(),
    };

    Ok(ResolvedTables {
        surface,
        layout,
        component,
        dev,
        capabilities,
    })
}

/// Safe RAII wrapper owning one live `nve_get_interface`-compatible plugin instance.
pub struct NvePluginSlot {
    #[cfg(not(target_arch = "wasm32"))]
    _lib: Option<Library>,
    temp_copy: Option<PathBuf>,
    metadata: PluginMetadata,
    surface: NveSurfaceApi,
    layout: Option<NveLayoutApi>,
    component: Option<NveComponentApi>,
    dev: Option<NveDevApi>,
    handle: NveAppHandle,
}

impl NvePluginSlot {
    /// Instantiates an in-process plugin from its `nve_get_interface` entry point.
    pub fn from_in_process(
        plugin_id: PluginId,
        name: String,
        surface_id: SurfaceId,
        generation: u64,
        get_interface: NveGetInterfaceFn,
        req: PluginRequirement,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, PluginError> {
        let tables = resolve_and_validate_tables(get_interface, req)?;
        let handle = catch_unwind(AssertUnwindSafe(|| unsafe {
            (tables.surface.create)(surface_id)
        }))
        .map_err(|_| PluginError::PluginPanicked("NveSurfaceApi::create".into()))?;

        if handle.is_null() {
            return Err(PluginError::NullHandleReturned);
        }

        if let (Some(dev_api), Some(state)) = (tables.dev, prior_state) {
            if !state.is_empty() {
                let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
                    (dev_api.restore_state)(handle, NveBufferSlice::from_slice(state));
                }));
            }
        }

        Ok(Self {
            #[cfg(not(target_arch = "wasm32"))]
            _lib: None,
            temp_copy: None,
            metadata: PluginMetadata {
                plugin_id,
                name,
                surface_id,
                generation,
                capabilities: tables.capabilities,
                source: PluginSourceKind::InProcess,
            },
            surface: tables.surface,
            layout: tables.layout,
            component: tables.component,
            dev: tables.dev,
            handle,
        })
    }

    /// Loads a compiled dynamic library (`cdylib`) via a unique temporary copy so `dlopen` never
    /// serves a stale cached image across hot-swaps.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_dylib(
        plugin_id: PluginId,
        name: String,
        surface_id: SurfaceId,
        generation: u64,
        dylib_path: &Path,
        req: PluginRequirement,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, PluginError> {
        let ext = dylib_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("dylib");
        let stem = dylib_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("nve_plugin");
        let seq = TEMP_COPY_SEQ.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join("nve-plugin-host");
        std::fs::create_dir_all(&temp_dir).map_err(|e| PluginError::Io(e.to_string()))?;
        let copy_path = temp_dir.join(format!(
            "{stem}_p{plugin_id}_g{generation}_{}_{seq}.{ext}",
            std::process::id()
        ));
        std::fs::copy(dylib_path, &copy_path).map_err(|e| PluginError::Io(e.to_string()))?;

        let lib = match unsafe { Library::new(&copy_path) } {
            Ok(lib) => lib,
            Err(err) => {
                let _ = std::fs::remove_file(&copy_path);
                return Err(PluginError::LibraryLoadFailed(err.to_string()));
            }
        };

        let get_iface: NveGetInterfaceFn = match unsafe {
            lib.get::<NveGetInterfaceFn>(b"nve_get_interface")
        } {
            Ok(sym) => *sym,
            Err(err) => {
                drop(lib);
                let _ = std::fs::remove_file(&copy_path);
                return Err(PluginError::SymbolNotFound(err.to_string()));
            }
        };

        let tables = match resolve_and_validate_tables(get_iface, req) {
            Ok(t) => t,
            Err(err) => {
                drop(lib);
                let _ = std::fs::remove_file(&copy_path);
                return Err(err);
            }
        };

        let handle = match catch_unwind(AssertUnwindSafe(|| unsafe {
            (tables.surface.create)(surface_id)
        })) {
            Ok(h) if !h.is_null() => h,
            Ok(_) => {
                drop(lib);
                let _ = std::fs::remove_file(&copy_path);
                return Err(PluginError::NullHandleReturned);
            }
            Err(_) => {
                drop(lib);
                let _ = std::fs::remove_file(&copy_path);
                return Err(PluginError::PluginPanicked("NveSurfaceApi::create".into()));
            }
        };

        if let (Some(dev_api), Some(state)) = (tables.dev, prior_state) {
            if !state.is_empty() {
                let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
                    (dev_api.restore_state)(handle, NveBufferSlice::from_slice(state));
                }));
            }
        }

        Ok(Self {
            _lib: Some(lib),
            temp_copy: Some(copy_path),
            metadata: PluginMetadata {
                plugin_id,
                name,
                surface_id,
                generation,
                capabilities: tables.capabilities,
                source: PluginSourceKind::DynamicLibrary(dylib_path.to_path_buf()),
            },
            surface: tables.surface,
            layout: tables.layout,
            component: tables.component,
            dev: tables.dev,
            handle,
        })
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    pub fn plugin_id(&self) -> PluginId {
        self.metadata.plugin_id
    }

    pub fn surface_id(&self) -> SurfaceId {
        self.metadata.surface_id
    }

    pub fn generation(&self) -> u64 {
        self.metadata.generation
    }

    pub fn capabilities(&self) -> PluginCapabilities {
        self.metadata.capabilities
    }

    /// Renders one frame, borrowing the plugin's binary `CommandBuffer` slice zero-copy.
    pub fn render(&mut self, frame_id: FrameId) -> Result<&[u8], PluginError> {
        let slice = catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.surface.render)(self.handle, frame_id)
        }))
        .map_err(|_| PluginError::PluginPanicked("NveSurfaceApi::render".into()))?;
        Ok(unsafe { slice.as_slice() })
    }

    /// Dispatches an encoded `EventRing` slice into the plugin.
    pub fn dispatch_events(&mut self, encoded_events: &[u8]) -> Result<bool, PluginError> {
        let handled = catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.surface.dispatch_events)(self.handle, NveBufferSlice::from_slice(encoded_events))
        }))
        .map_err(|_| PluginError::PluginPanicked("NveSurfaceApi::dispatch_events".into()))?;
        Ok(handled != 0)
    }

    /// Dispatches zero-copy direct FFI events if the plugin supports `dispatch_events_direct`.
    pub fn dispatch_events_direct(&mut self, events: &[NveDirectEvent]) -> Result<bool, PluginError> {
        let Some(f) = self.surface.dispatch_events_direct else {
            return Err(PluginError::UnsupportedCapability("dispatch_events_direct"));
        };
        let handled = catch_unwind(AssertUnwindSafe(|| unsafe {
            f(self.handle, NveEventBatch::from_slice(events))
        }))
        .map_err(|_| PluginError::PluginPanicked("NveSurfaceApi::dispatch_events_direct".into()))?;
        Ok(handled != 0)
    }

    /// Queries intrinsic size via `NVE_IFACE_LAYOUT` if exported.
    pub fn measure(&mut self, constraints: NveSizeConstraints) -> Option<NveIntrinsicSize> {
        let api = self.layout?;
        catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.measure)(self.handle, constraints)
        }))
        .ok()
    }

    /// Queries gesture disposition via `NVE_IFACE_LAYOUT` if exported.
    pub fn hit_test(&mut self, x: f32, y: f32, pointer_phase: u8) -> NveGestureDisposition {
        let Some(api) = self.layout else {
            return NveGestureDisposition::PassThrough;
        };
        match catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.hit_test)(self.handle, x, y, pointer_phase)
        })) {
            Ok(code) => NveGestureDisposition::from_u8(code),
            Err(_) => NveGestureDisposition::PassThrough,
        }
    }

    /// Updates plugin props via `NVE_IFACE_COMPONENT`.
    pub fn set_props(&mut self, props_bytes: &[u8]) -> Result<bool, PluginError> {
        let Some(api) = self.component else {
            return Err(PluginError::UnsupportedCapability("NVE_IFACE_COMPONENT"));
        };
        let changed = catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.set_props)(self.handle, NveBufferSlice::from_slice(props_bytes))
        }))
        .map_err(|_| PluginError::PluginPanicked("NveComponentApi::set_props".into()))?;
        Ok(changed != 0)
    }

    /// Updates plugin props via zero-copy structured `NveMapEntry` array if exported.
    pub fn set_props_direct(&mut self, entries: &[NveMapEntry]) -> Result<bool, PluginError> {
        let Some(f) = self.component.and_then(|c| c.set_props_direct) else {
            return Err(PluginError::UnsupportedCapability("set_props_direct"));
        };
        let changed = catch_unwind(AssertUnwindSafe(|| unsafe {
            f(
                self.handle,
                entries.as_ptr(),
                entries.len().min(u32::MAX as usize) as u32,
            )
        }))
        .map_err(|_| PluginError::PluginPanicked("NveComponentApi::set_props_direct".into()))?;
        Ok(changed != 0)
    }

    /// Drains component-emitted outbound events via `NVE_IFACE_COMPONENT` if exported.
    pub fn drain_emitted_events(&mut self) -> &[u8] {
        let Some(api) = self.component else {
            return &[];
        };
        match catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.drain_emitted_events)(self.handle)
        })) {
            Ok(slice) => unsafe { slice.as_slice() },
            Err(_) => &[],
        }
    }

    /// Applies a Tier-1 JSON hot patch via `NVE_IFACE_DEV` if exported.
    pub fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
        let Some(api) = self.dev else {
            return false;
        };
        catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.apply_hot_patch)(self.handle, NveBufferSlice::from_slice(patch_json.as_bytes())) != 0
        }))
        .unwrap_or(false)
    }

    /// Snapshots plugin state via `NVE_IFACE_DEV` if exported.
    pub fn snapshot_state(&mut self) -> Vec<u8> {
        let Some(api) = self.dev else {
            return Vec::new();
        };
        match catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.snapshot_state)(self.handle)
        })) {
            Ok(slice) => unsafe { slice.as_slice().to_vec() },
            Err(_) => Vec::new(),
        }
    }

    /// Restores plugin state via `NVE_IFACE_DEV` if exported.
    pub fn restore_state(&mut self, state: &[u8]) -> bool {
        let Some(api) = self.dev else {
            return false;
        };
        catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.restore_state)(self.handle, NveBufferSlice::from_slice(state));
        }))
        .is_ok()
    }
}

impl Drop for NvePluginSlot {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
                (self.surface.destroy)(self.handle);
            }));
            self.handle = std::ptr::null_mut();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            drop(self._lib.take());
        }
        if let Some(path) = self.temp_copy.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Safe, typed registry hosting isolated `nve_get_interface` plugin slots with transactional
/// zero-downtime hot-swapping.
pub struct NvePluginHost {
    next_plugin_id: PluginId,
    next_surface_id: SurfaceId,
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
        Self::with_surface_base(DEFAULT_PLUGIN_SURFACE_BASE)
    }

    /// Creates a plugin host whose isolated surface IDs start at `base_surface_id`.
    pub fn with_surface_base(base_surface_id: SurfaceId) -> Self {
        Self {
            next_plugin_id: 1,
            next_surface_id: base_surface_id.max(1),
            slots: HashMap::new(),
            name_index: HashMap::new(),
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

    fn peek_ids(&self) -> (PluginId, SurfaceId) {
        (self.next_plugin_id, self.next_surface_id)
    }

    fn commit_ids(&mut self) {
        self.next_plugin_id = self.next_plugin_id.saturating_add(1);
        self.next_surface_id = self.next_surface_id.saturating_add(1);
    }

    /// Registers an in-process `nve_get_interface` plugin with ABI capability validation and a
    /// freshly allocated isolated `SurfaceId`.
    pub fn register_in_process(
        &mut self,
        name: impl Into<String>,
        get_interface: NveGetInterfaceFn,
        req: PluginRequirement,
    ) -> Result<PluginId, PluginError> {
        let name = name.into();
        self.validate_name(&name)?;
        let (plugin_id, surface_id) = self.peek_ids();

        let slot = NvePluginSlot::from_in_process(
            plugin_id,
            name.clone(),
            surface_id,
            1,
            get_interface,
            req,
            None,
        )?;
        self.commit_ids();
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
        let (plugin_id, surface_id) = self.peek_ids();

        let slot = NvePluginSlot::from_dylib(
            plugin_id,
            name.clone(),
            surface_id,
            1,
            dylib_path,
            req,
            None,
        )?;
        self.commit_ids();
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

    /// Unloads and destroys a mounted plugin slot cleanly.
    pub fn unload(&mut self, plugin_id: PluginId) -> Result<PluginMetadata, PluginError> {
        let slot = self
            .slots
            .remove(&plugin_id)
            .ok_or(PluginError::PluginNotFound(plugin_id))?;
        let meta = slot.metadata().clone();
        self.name_index.remove(&meta.name);
        drop(slot);
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
        let mut out: Vec<PluginMetadata> = self.slots.values().map(|s| s.metadata().clone()).collect();
        out.sort_by_key(|m| m.plugin_id);
        out
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;

    use nve_abi::{CommandBufferBuilder, CommandDecoder, ComponentKind, ROOT_NODE_ID};

    use super::*;

    // ---- Test Plugin V1: full capability plugin (SURFACE + LAYOUT + COMPONENT + DEV) ----
    struct MockPluginState {
        surface_id: SurfaceId,
        version_tag: &'static str,
        counter: u32,
        frame_buf: Vec<u8>,
        state_buf: Vec<u8>,
        out_events: Vec<u8>,
    }

    unsafe extern "C" fn p1_create(surface_id: u32) -> NveAppHandle {
        Box::into_raw(Box::new(MockPluginState {
            surface_id,
            version_tag: "v1",
            counter: 1,
            frame_buf: Vec::new(),
            state_buf: Vec::new(),
            out_events: Vec::new(),
        })) as NveAppHandle
    }

    unsafe extern "C" fn p2_create(surface_id: u32) -> NveAppHandle {
        Box::into_raw(Box::new(MockPluginState {
            surface_id,
            version_tag: "v2",
            counter: 1,
            frame_buf: Vec::new(),
            state_buf: Vec::new(),
            out_events: Vec::new(),
        })) as NveAppHandle
    }

    unsafe extern "C" fn p_destroy(handle: NveAppHandle) {
        if !handle.is_null() {
            unsafe { drop(Box::from_raw(handle as *mut MockPluginState)) };
        }
    }

    unsafe extern "C" fn p_render(handle: NveAppHandle, frame_id: u64) -> NveBufferSlice {
        let st = unsafe { &mut *(handle as *mut MockPluginState) };
        let mut b = CommandBufferBuilder::new(st.surface_id, frame_id);
        b.create_node(2, ComponentKind::Text, None);
        b.set_text(2, &format!("{}:count={}", st.version_tag, st.counter));
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        st.frame_buf = b.into_bytes();
        NveBufferSlice::from_slice(&st.frame_buf)
    }

    unsafe extern "C" fn p_dispatch(handle: NveAppHandle, _events: NveBufferSlice) -> u32 {
        let st = unsafe { &mut *(handle as *mut MockPluginState) };
        st.counter += 1;
        1
    }

    unsafe extern "C" fn p_measure(
        _handle: NveAppHandle,
        c: NveSizeConstraints,
    ) -> NveIntrinsicSize {
        NveIntrinsicSize {
            width: c.max_w.min(240.0),
            height: c.max_h.min(80.0),
        }
    }

    unsafe extern "C" fn p_hit_test(_: NveAppHandle, _: f32, _: f32, _: u8) -> u8 {
        NveGestureDisposition::Claim as u8
    }

    unsafe extern "C" fn p_set_props(handle: NveAppHandle, props: NveBufferSlice) -> u32 {
        let st = unsafe { &mut *(handle as *mut MockPluginState) };
        let raw = unsafe { props.as_slice() };
        if raw.len() == 4 {
            st.counter = u32::from_le_bytes(raw.try_into().unwrap());
            return 1;
        }
        0
    }

    unsafe extern "C" fn p_drain_events(handle: NveAppHandle) -> NveBufferSlice {
        let st = unsafe { &mut *(handle as *mut MockPluginState) };
        st.out_events = st.counter.to_le_bytes().to_vec();
        NveBufferSlice::from_slice(&st.out_events)
    }

    unsafe extern "C" fn p_hot_patch(_: NveAppHandle, _: NveBufferSlice) -> u32 {
        1
    }

    unsafe extern "C" fn p_snapshot(handle: NveAppHandle) -> NveBufferSlice {
        let st = unsafe { &mut *(handle as *mut MockPluginState) };
        st.state_buf = st.counter.to_le_bytes().to_vec();
        NveBufferSlice::from_slice(&st.state_buf)
    }

    unsafe extern "C" fn p_restore(handle: NveAppHandle, state: NveBufferSlice) {
        let st = unsafe { &mut *(handle as *mut MockPluginState) };
        let raw = unsafe { state.as_slice() };
        if raw.len() == 4 {
            st.counter = u32::from_le_bytes(raw.try_into().unwrap());
        }
    }

    static P1_SURFACE: NveSurfaceApi = NveSurfaceApi {
        create: p1_create,
        destroy: p_destroy,
        render: p_render,
        dispatch_events: p_dispatch,
        dispatch_events_direct: None,
    };
    static P2_SURFACE: NveSurfaceApi = NveSurfaceApi {
        create: p2_create,
        destroy: p_destroy,
        render: p_render,
        dispatch_events: p_dispatch,
        dispatch_events_direct: None,
    };
    static P_LAYOUT: NveLayoutApi = NveLayoutApi {
        measure: p_measure,
        hit_test: p_hit_test,
    };
    static P_COMPONENT: NveComponentApi = NveComponentApi {
        set_props: p_set_props,
        drain_emitted_events: p_drain_events,
        set_props_direct: None,
    };
    static P_DEV: NveDevApi = NveDevApi {
        apply_hot_patch: p_hot_patch,
        snapshot_state: p_snapshot,
        restore_state: p_restore,
    };

    unsafe extern "C" fn plugin_v1_get_interface(id: u32) -> *const c_void {
        match id {
            NVE_IFACE_SURFACE => &P1_SURFACE as *const _ as *const c_void,
            NVE_IFACE_LAYOUT => &P_LAYOUT as *const _ as *const c_void,
            NVE_IFACE_COMPONENT => &P_COMPONENT as *const _ as *const c_void,
            NVE_IFACE_DEV => &P_DEV as *const _ as *const c_void,
            _ => std::ptr::null(),
        }
    }

    unsafe extern "C" fn plugin_v2_get_interface(id: u32) -> *const c_void {
        match id {
            NVE_IFACE_SURFACE => &P2_SURFACE as *const _ as *const c_void,
            NVE_IFACE_LAYOUT => &P_LAYOUT as *const _ as *const c_void,
            NVE_IFACE_COMPONENT => &P_COMPONENT as *const _ as *const c_void,
            NVE_IFACE_DEV => &P_DEV as *const _ as *const c_void,
            _ => std::ptr::null(),
        }
    }

    unsafe extern "C" fn surface_only_get_interface(id: u32) -> *const c_void {
        match id {
            NVE_IFACE_SURFACE => &P1_SURFACE as *const _ as *const c_void,
            _ => std::ptr::null(),
        }
    }

    unsafe extern "C" fn broken_missing_surface_get_interface(_id: u32) -> *const c_void {
        std::ptr::null()
    }

    unsafe extern "C" fn null_handle_create(_surface_id: u32) -> NveAppHandle {
        std::ptr::null_mut()
    }

    static BROKEN_NULL_HANDLE_SURFACE: NveSurfaceApi = NveSurfaceApi {
        create: null_handle_create,
        destroy: p_destroy,
        render: p_render,
        dispatch_events: p_dispatch,
        dispatch_events_direct: None,
    };

    unsafe extern "C" fn broken_null_handle_get_interface(id: u32) -> *const c_void {
        match id {
            NVE_IFACE_SURFACE => &BROKEN_NULL_HANDLE_SURFACE as *const _ as *const c_void,
            _ => std::ptr::null(),
        }
    }

    #[test]
    fn capability_checks_isolated_surfaces_and_transactional_hotswap() {
        let mut host = NvePluginHost::with_surface_base(20_000);

        // 1. Reject plugin missing NVE_IFACE_SURFACE.
        assert_eq!(
            host.register_in_process(
                "bad-no-surface",
                broken_missing_surface_get_interface,
                PluginRequirement::SURFACE_ONLY
            ),
            Err(PluginError::MissingSurfaceInterface)
        );

        // 2. Reject surface-only plugin when caller requires NVE_IFACE_COMPONENT.
        assert_eq!(
            host.register_in_process(
                "needs-component",
                surface_only_get_interface,
                PluginRequirement::EMBEDDED_COMPONENT
            ),
            Err(PluginError::MissingRequiredInterface {
                interface_name: "NVE_IFACE_COMPONENT",
                interface_id: NVE_IFACE_COMPONENT,
            })
        );

        // 3. Surface-only plugin mounts cleanly under SURFACE_ONLY requirement.
        let p_min = host
            .register_in_process(
                "minimal-widget",
                surface_only_get_interface,
                PluginRequirement::SURFACE_ONLY,
            )
            .unwrap();
        let min_slot = host.get(p_min).unwrap();
        assert_eq!(min_slot.surface_id(), 20_000);
        assert!(min_slot.capabilities().surface);
        assert!(!min_slot.capabilities().component);
        assert!(!min_slot.capabilities().layout);

        // 4. Full-featured plugin gets a distinct isolated SurfaceId (20_001).
        let p_full = host
            .register_in_process(
                "studio-inspector-plugin",
                plugin_v1_get_interface,
                PluginRequirement::EMBEDDED_COMPONENT,
            )
            .unwrap();
        assert_eq!(host.active_count(), 2);

        let slot = host.get_mut(p_full).unwrap();
        assert_eq!(slot.surface_id(), 20_001);
        assert!(slot.capabilities().component && slot.capabilities().dev && slot.capabilities().layout);
        assert_eq!(
            slot.measure(NveSizeConstraints::loose(400.0, 200.0)),
            Some(NveIntrinsicSize {
                width: 240.0,
                height: 80.0
            })
        );
        assert_eq!(slot.hit_test(10.0, 10.0, 0), NveGestureDisposition::Claim);

        // Mutate plugin state via props + events before hot-swap.
        assert!(slot.set_props(&42u32.to_le_bytes()).unwrap());
        assert!(slot.dispatch_events(&[]).unwrap()); // counter -> 43
        let f1 = slot.render(1).unwrap().to_vec();
        let (hdr1, cmds1) = CommandDecoder::new(&f1).unwrap().decode_all().unwrap();
        assert_eq!(hdr1.surface_id, 20_001);
        assert!(format!("{cmds1:?}").contains("v1:count=43"));

        // 5. Failed hot-swap (broken plugin returning null handle or missing surface)
        // must NOT crash or destroy v1!
        assert_eq!(
            host.hot_swap_in_process(
                p_full,
                broken_null_handle_get_interface,
                PluginRequirement::SURFACE_ONLY
            ),
            Err(PluginError::NullHandleReturned)
        );
        assert_eq!(
            host.hot_swap_in_process(
                p_full,
                broken_missing_surface_get_interface,
                PluginRequirement::SURFACE_ONLY
            ),
            Err(PluginError::MissingSurfaceInterface)
        );
        // Original v1 slot is still live at generation 1 with counter=43!
        let still_v1 = host.get_mut(p_full).unwrap();
        assert_eq!(still_v1.generation(), 1);
        let f_after_fail = still_v1.render(2).unwrap().to_vec();
        let (_, cmds_fail) = CommandDecoder::new(&f_after_fail).unwrap().decode_all().unwrap();
        assert!(format!("{cmds_fail:?}").contains("v1:count=43"));

        // 6. Valid hot-swap v1 -> v2 preserves isolated surface_id (20_001) and counter=43!
        let gen2 = host
            .hot_swap_in_process(
                p_full,
                plugin_v2_get_interface,
                PluginRequirement::EMBEDDED_COMPONENT,
            )
            .unwrap();
        assert_eq!(gen2, 2);

        let v2_slot = host.get_mut(p_full).unwrap();
        assert_eq!(v2_slot.surface_id(), 20_001);
        assert_eq!(v2_slot.generation(), 2);
        let f2 = v2_slot.render(3).unwrap().to_vec();
        let (hdr2, cmds2) = CommandDecoder::new(&f2).unwrap().decode_all().unwrap();
        assert_eq!(hdr2.surface_id, 20_001);
        assert!(format!("{cmds2:?}").contains("v2:count=43"));

        // 7. Duplicate name rejected; unload cleans up cleanly.
        assert!(matches!(
            host.register_in_process(
                "studio-inspector-plugin",
                plugin_v1_get_interface,
                PluginRequirement::SURFACE_ONLY
            ),
            Err(PluginError::DuplicatePluginName(_))
        ));
        let unloaded = host.unload(p_full).unwrap();
        assert_eq!(unloaded.generation, 2);
        assert_eq!(host.active_count(), 1);
        assert!(host.find_by_name("studio-inspector-plugin").is_none());
    }
}
