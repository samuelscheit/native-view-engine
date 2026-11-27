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

mod manifest;
mod reload;
mod sandbox;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod panic_tests;

pub use manifest::{
    PluginCapabilities, PluginError, PluginId, PluginMetadata, PluginRequirement, PluginSourceKind,
};
pub use reload::NvePluginHost;
pub use sandbox::NvePluginSlot;
