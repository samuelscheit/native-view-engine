use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use nve_abi::{
    InterfaceId, NveComponentApi, NveDevApi, NveGetInterfaceFn, NveLayoutApi, NveSurfaceApi,
    SurfaceId, NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};

use crate::error::InteropError;

/// Unique handle identifying a mounted plugin inside [`super::NvePluginHost`].
pub type PluginId = u32;

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

/// Errors produced by [`super::NvePluginHost`] and [`super::NvePluginSlot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    MissingSurfaceInterface,
    MissingRequiredInterface {
        interface_name: &'static str,
        interface_id: InterfaceId,
    },
    NullHandleReturned,
    SurfaceSlotsExhausted,
    SurfaceNotLive(SurfaceId),
    SurfaceAllocationFailed(String),
    PluginIdsExhausted,
    GenerationExhausted(PluginId),
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
                "plugin missing required interface {interface_name} ({})",
                interface_id.raw()
            ),
            Self::NullHandleReturned => {
                f.write_str("plugin NveSurfaceApi::create returned a null handle")
            }
            Self::SurfaceSlotsExhausted => {
                f.write_str("plugin host exhausted all generational surface slots")
            }
            Self::SurfaceNotLive(id) => {
                write!(f, "plugin surface {id} is no longer live in SurfaceOrchestrator")
            }
            Self::SurfaceAllocationFailed(msg) => {
                write!(f, "plugin surface operation failed: {msg}")
            }
            Self::PluginIdsExhausted => f.write_str("plugin host exhausted 32-bit plugin IDs"),
            Self::GenerationExhausted(id) => {
                write!(f, "plugin {id} exhausted 64-bit hot-swap generations")
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
pub(super) struct ResolvedTables {
    pub(super) surface: NveSurfaceApi,
    pub(super) layout: Option<NveLayoutApi>,
    pub(super) component: Option<NveComponentApi>,
    pub(super) dev: Option<NveDevApi>,
    pub(super) capabilities: PluginCapabilities,
}

pub(super) fn resolve_and_validate_tables(
    get_iface: NveGetInterfaceFn,
    req: PluginRequirement,
) -> Result<ResolvedTables, PluginError> {
    let surface_ptr = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `get_iface` is a valid NVE C-ABI interface resolver returning a static vtable or null.
        unsafe { get_iface(NVE_IFACE_SURFACE) as *const NveSurfaceApi }
    }))
    .map_err(|_| PluginError::PluginPanicked("nve_get_interface(NVE_IFACE_SURFACE)".into()))?;

    if surface_ptr.is_null() {
        return Err(PluginError::MissingSurfaceInterface);
    }
    // SAFETY: `surface_ptr` is non-null and points to a valid static `NveSurfaceApi` table.
    let surface = unsafe { *surface_ptr };

    let layout = {
        let p = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: `get_iface` is a valid NVE C-ABI interface resolver.
            unsafe { get_iface(NVE_IFACE_LAYOUT) as *const NveLayoutApi }
        }))
        .map_err(|_| PluginError::PluginPanicked("nve_get_interface(NVE_IFACE_LAYOUT)".into()))?;
        // SAFETY: `p` is non-null and points to a valid static `NveLayoutApi` table.
        (!p.is_null()).then(|| unsafe { *p })
    };
    if req.require_layout && layout.is_none() {
        return Err(PluginError::MissingRequiredInterface {
            interface_name: "NVE_IFACE_LAYOUT",
            interface_id: NVE_IFACE_LAYOUT,
        });
    }

    let component = {
        let p = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: `get_iface` is a valid NVE C-ABI interface resolver.
            unsafe { get_iface(NVE_IFACE_COMPONENT) as *const NveComponentApi }
        }))
        .map_err(|_| {
            PluginError::PluginPanicked("nve_get_interface(NVE_IFACE_COMPONENT)".into())
        })?;
        // SAFETY: `p` is non-null and points to a valid static `NveComponentApi` table.
        (!p.is_null()).then(|| unsafe { *p })
    };
    if req.require_component && component.is_none() {
        return Err(PluginError::MissingRequiredInterface {
            interface_name: "NVE_IFACE_COMPONENT",
            interface_id: NVE_IFACE_COMPONENT,
        });
    }

    let dev = {
        let p = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: `get_iface` is a valid NVE C-ABI interface resolver.
            unsafe { get_iface(NVE_IFACE_DEV) as *const NveDevApi }
        }))
        .map_err(|_| PluginError::PluginPanicked("nve_get_interface(NVE_IFACE_DEV)".into()))?;
        // SAFETY: `p` is non-null and points to a valid static `NveDevApi` table.
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
        direct_events: true,
        direct_props: component.is_some(),
    };

    Ok(ResolvedTables {
        surface,
        layout,
        component,
        dev,
        capabilities,
    })
}
