//! Direct typed [`LocationBackend`] plus [`MemoryLocation`] and headless-safe [`NativeLocation`].

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nve_host_services::{pack_typed, HostServiceHandler, PermissionStatus, ServiceError};

use crate::{
    decode_accuracy_request, decode_watch_config, decode_watch_id, encode_geo_fix, encode_watch_id,
    GeoFix, LocationAccuracy, LocationWatchConfig, LocationWatchId, METHOD_LOC_CURRENT_POSITION,
    METHOD_LOC_REQUEST_PERMISSION, METHOD_LOC_STOP_WATCH, METHOD_LOC_WATCH_POSITION,
    SERVICE_LOCATION,
};

/// Direct typed in-process backend for [`SERVICE_LOCATION`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait LocationBackend: Send + Sync {
    fn request_permission(&self) -> Result<PermissionStatus, ServiceError>;
    fn current_position(&self, accuracy: LocationAccuracy) -> Result<GeoFix, ServiceError>;
    fn watch_position(&self, config: &LocationWatchConfig) -> Result<LocationWatchId, ServiceError>;
    fn stop_watch(&self, watch_id: LocationWatchId) -> Result<(), ServiceError>;
}

/// Bridges a [`LocationBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_location_wire<B: LocationBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_LOC_REQUEST_PERMISSION => Ok(backend.request_permission()?.encode()),
        METHOD_LOC_CURRENT_POSITION => {
            let acc = decode_accuracy_request(request)?;
            encode_geo_fix(&backend.current_position(acc)?)
        }
        METHOD_LOC_WATCH_POSITION => {
            let cfg = decode_watch_config(request)?;
            encode_watch_id(backend.watch_position(&cfg)?)
        }
        METHOD_LOC_STOP_WATCH => {
            let id = decode_watch_id(request)?;
            backend.stop_watch(id)?;
            Ok(Vec::new())
        }
        _ => Err(ServiceError::Unsupported),
    }
}

struct MemoryLocationState {
    permission: PermissionStatus,
    current_fix: GeoFix,
    next_watch_id: u64,
    watches: BTreeMap<LocationWatchId, LocationWatchConfig>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_LOCATION`].
pub struct MemoryLocation {
    state: Mutex<MemoryLocationState>,
}

impl Default for MemoryLocation {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryLocation {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryLocationState {
                permission: PermissionStatus::Authorized,
                current_fix: GeoFix {
                    latitude: 37.7749,
                    longitude: -122.4194,
                    altitude_m: Some(15.5),
                    accuracy_m: 4.2,
                    speed_mps: Some(0.0),
                    heading_deg: Some(90.0),
                    timestamp_ms: 1_700_000_000_000,
                },
                next_watch_id: 1,
                watches: BTreeMap::new(),
            }),
        }
    }

    pub fn set_permission(&self, status: PermissionStatus) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .permission = status;
    }

    pub fn set_current_fix(&self, fix: GeoFix) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current_fix = fix;
    }

    pub fn active_watches(&self) -> Vec<LocationWatchId> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .watches
            .keys()
            .copied()
            .collect()
    }
}

impl LocationBackend for MemoryLocation {
    fn request_permission(&self) -> Result<PermissionStatus, ServiceError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .permission)
    }

    fn current_position(&self, _accuracy: LocationAccuracy) -> Result<GeoFix, ServiceError> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(
            st.permission,
            PermissionStatus::Denied | PermissionStatus::Restricted
        ) {
            return Err(ServiceError::PermissionDenied);
        }
        Ok(st.current_fix)
    }

    fn watch_position(&self, config: &LocationWatchConfig) -> Result<LocationWatchId, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(
            st.permission,
            PermissionStatus::Denied | PermissionStatus::Restricted
        ) {
            return Err(ServiceError::PermissionDenied);
        }
        let id = LocationWatchId(st.next_watch_id);
        st.next_watch_id = st.next_watch_id.saturating_add(1);
        st.watches.insert(id, *config);
        Ok(id)
    }

    fn stop_watch(&self, watch_id: LocationWatchId) -> Result<(), ServiceError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .watches
            .remove(&watch_id)
            .ok_or(ServiceError::NotFound)?;
        Ok(())
    }
}

impl HostServiceHandler for MemoryLocation {
    fn service_id(&self) -> u32 {
        SERVICE_LOCATION
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_location_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn LocationBackend>(self))
    }
}

/// Safe non-modal native [`HostServiceHandler`] for [`SERVICE_LOCATION`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeLocation;

impl NativeLocation {
    pub const fn new() -> Self {
        Self
    }
}

impl LocationBackend for NativeLocation {
    fn request_permission(&self) -> Result<PermissionStatus, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn current_position(&self, _accuracy: LocationAccuracy) -> Result<GeoFix, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn watch_position(&self, _config: &LocationWatchConfig) -> Result<LocationWatchId, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn stop_watch(&self, _watch_id: LocationWatchId) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

impl HostServiceHandler for NativeLocation {
    fn service_id(&self) -> u32 {
        SERVICE_LOCATION
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_location_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn LocationBackend>(self))
    }
}
