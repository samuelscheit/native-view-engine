//! Direct typed [`SensorsBackend`] plus [`MemorySensors`] and headless-safe [`NativeSensors`].

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::{
    decode_sensor_kind, decode_sensor_subscribe_request, decode_sensor_subscription_id,
    encode_is_available_response, encode_sensor_subscription_id, SensorKind, SensorSubscriptionId,
    METHOD_SENSOR_IS_AVAILABLE, METHOD_SENSOR_SUBSCRIBE, METHOD_SENSOR_UNSUBSCRIBE,
    SERVICE_SENSORS,
};

/// Direct typed in-process backend for [`SERVICE_SENSORS`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait SensorsBackend: Send + Sync {
    fn is_available(&self, kind: SensorKind) -> Result<bool, ServiceError>;
    fn subscribe(&self, kind: SensorKind, rate_hz: u16) -> Result<SensorSubscriptionId, ServiceError>;
    fn unsubscribe(&self, id: SensorSubscriptionId) -> Result<(), ServiceError>;
}

/// Bridges a [`SensorsBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_sensors_wire<B: SensorsBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_SENSOR_IS_AVAILABLE => {
            let kind = decode_sensor_kind(request)?;
            Ok(encode_is_available_response(backend.is_available(kind)?))
        }
        METHOD_SENSOR_SUBSCRIBE => {
            let (kind, rate_hz) = decode_sensor_subscribe_request(request)?;
            encode_sensor_subscription_id(backend.subscribe(kind, rate_hz)?)
        }
        METHOD_SENSOR_UNSUBSCRIBE => {
            let id = decode_sensor_subscription_id(request)?;
            backend.unsubscribe(id)?;
            Ok(Vec::new())
        }
        _ => Err(ServiceError::Unsupported),
    }
}

struct MemorySensorsState {
    available: BTreeSet<SensorKind>,
    next_sub_id: u64,
    subscriptions: BTreeMap<SensorSubscriptionId, (SensorKind, u16)>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_SENSORS`].
pub struct MemorySensors {
    state: Mutex<MemorySensorsState>,
}

impl Default for MemorySensors {
    fn default() -> Self {
        Self::new()
    }
}

impl MemorySensors {
    pub fn new() -> Self {
        let mut available = BTreeSet::new();
        available.insert(SensorKind::Accelerometer);
        available.insert(SensorKind::Gyroscope);
        available.insert(SensorKind::Magnetometer);
        available.insert(SensorKind::Barometer);
        available.insert(SensorKind::AmbientLight);
        Self {
            state: Mutex::new(MemorySensorsState {
                available,
                next_sub_id: 1,
                subscriptions: BTreeMap::new(),
            }),
        }
    }

    pub fn set_available(&self, kind: SensorKind, available: bool) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if available {
            st.available.insert(kind);
        } else {
            st.available.remove(&kind);
        }
    }

    pub fn active_subscriptions(&self) -> Vec<(SensorSubscriptionId, SensorKind, u16)> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscriptions
            .iter()
            .map(|(&id, &(k, r))| (id, k, r))
            .collect()
    }
}

impl SensorsBackend for MemorySensors {
    fn is_available(&self, kind: SensorKind) -> Result<bool, ServiceError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .available
            .contains(&kind))
    }

    fn subscribe(&self, kind: SensorKind, rate_hz: u16) -> Result<SensorSubscriptionId, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !st.available.contains(&kind) {
            return Err(ServiceError::Unsupported);
        }
        let id = SensorSubscriptionId(st.next_sub_id);
        st.next_sub_id = st.next_sub_id.saturating_add(1);
        st.subscriptions.insert(id, (kind, rate_hz));
        Ok(id)
    }

    fn unsubscribe(&self, id: SensorSubscriptionId) -> Result<(), ServiceError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscriptions
            .remove(&id)
            .ok_or(ServiceError::NotFound)?;
        Ok(())
    }
}

impl HostServiceHandler for MemorySensors {
    fn service_id(&self) -> u32 {
        SERVICE_SENSORS
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_sensors_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn SensorsBackend>(self))
    }
}

/// Safe non-modal native [`HostServiceHandler`] for [`SERVICE_SENSORS`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeSensors;

impl NativeSensors {
    pub const fn new() -> Self {
        Self
    }
}

impl SensorsBackend for NativeSensors {
    fn is_available(&self, _kind: SensorKind) -> Result<bool, ServiceError> {
        Ok(false)
    }

    fn subscribe(
        &self,
        _kind: SensorKind,
        _rate_hz: u16,
    ) -> Result<SensorSubscriptionId, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn unsubscribe(&self, _id: SensorSubscriptionId) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

impl HostServiceHandler for NativeSensors {
    fn service_id(&self) -> u32 {
        SERVICE_SENSORS
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_sensors_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn SensorsBackend>(self))
    }
}
