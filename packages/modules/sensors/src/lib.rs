//! Hardware inertial & environmental sensors capability (`SERVICE_SENSORS = 0x0025`) with
//! direct zero-serialization in-process dispatch ([`SensorsBackend`]), fixed-layout
//! `#[repr(C)]` [`SensorSample`] events over [`TypedEventBus`], [`MemorySensors`], and
//! [`NativeSensors`].

pub mod handler;

use std::sync::Arc;

use nve_host_services::{
    resolve_typed, BinReader, BinWriter, HostServiceProvider, ServiceError, TypedEventBus,
    TypedEventSubscription, MAX_SEQ_ITEMS,
};

pub use handler::{dispatch_sensors_wire, MemorySensors, NativeSensors, SensorsBackend};

/// Built-in service ID for hardware inertial & environmental sensor streams.
pub const SERVICE_SENSORS: u32 = 0x0025;

pub const METHOD_SENSOR_IS_AVAILABLE: u32 = 1;
pub const METHOD_SENSOR_SUBSCRIBE: u32 = 2;
pub const METHOD_SENSOR_UNSUBSCRIBE: u32 = 3;

/// Maximum supported sensor sampling frequency (Hz).
pub const MAX_SENSOR_RATE_HZ: u16 = 1000;

/// Supported hardware sensor families.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SensorKind {
    Accelerometer = 0,
    Gyroscope = 1,
    Magnetometer = 2,
    Barometer = 3,
    AmbientLight = 4,
}

impl SensorKind {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Accelerometer),
            1 => Some(Self::Gyroscope),
            2 => Some(Self::Magnetometer),
            3 => Some(Self::Barometer),
            4 => Some(Self::AmbientLight),
            _ => None,
        }
    }
}

/// Opaque identifier for an active hardware sensor subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SensorSubscriptionId(pub u64);

/// Fixed-layout hardware sensor sample (`f32 x, y, z, u64 timestamp_ns`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorSample {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub timestamp_ns: u64,
}

const _: () = {
    assert!(std::mem::size_of::<SensorSample>() == 24);
    assert!(std::mem::offset_of!(SensorSample, x) == 0);
    assert!(std::mem::offset_of!(SensorSample, y) == 4);
    assert!(std::mem::offset_of!(SensorSample, z) == 8);
    assert!(std::mem::offset_of!(SensorSample, timestamp_ns) == 16);
};

impl SensorSample {
    /// Validates that all three sample axes are finite numbers.
    pub fn validate(&self) -> Result<(), ServiceError> {
        if !self.x.is_finite() || !self.y.is_finite() || !self.z.is_finite() {
            return Err(ServiceError::InvalidArg(
                "SensorSample axes x/y/z must be finite".into(),
            ));
        }
        Ok(())
    }
}

/// Direct typed sensor batch event delivered over [`TypedEventBus`] with zero serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct SensorBatchEvent {
    pub subscription_id: SensorSubscriptionId,
    pub kind: SensorKind,
    pub samples: Vec<SensorSample>,
}

impl SensorBatchEvent {
    pub fn new(
        subscription_id: SensorSubscriptionId,
        kind: SensorKind,
        samples: Vec<SensorSample>,
    ) -> Result<Self, ServiceError> {
        if subscription_id.0 == 0 {
            return Err(ServiceError::InvalidArg(
                "sensor subscription id must be non-zero".into(),
            ));
        }
        if samples.len() > MAX_SEQ_ITEMS {
            return Err(ServiceError::InvalidArg(
                "sensor sample batch exceeds MAX_SEQ_ITEMS".into(),
            ));
        }
        for sample in &samples {
            sample.validate()?;
        }
        Ok(Self {
            subscription_id,
            kind,
            samples,
        })
    }
}

/// Validates and publishes a typed [`SensorBatchEvent`] directly onto `bus` with zero serialization.
pub fn emit_sensor_batch(
    bus: &TypedEventBus,
    batch: SensorBatchEvent,
) -> Result<usize, ServiceError> {
    if batch.subscription_id.0 == 0 {
        return Err(ServiceError::InvalidArg(
            "sensor subscription id must be non-zero".into(),
        ));
    }
    if batch.samples.len() > MAX_SEQ_ITEMS {
        return Err(ServiceError::InvalidArg(
            "sensor sample batch exceeds MAX_SEQ_ITEMS".into(),
        ));
    }
    for sample in &batch.samples {
        sample.validate()?;
    }
    Ok(bus.publish(batch))
}

pub fn encode_sensor_kind(kind: SensorKind) -> Vec<u8> {
    vec![kind as u8]
}

pub fn decode_sensor_kind(bytes: &[u8]) -> Result<SensorKind, ServiceError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    SensorKind::from_u8(raw)
        .ok_or_else(|| ServiceError::Codec(format!("invalid SensorKind {raw}")))
}

pub fn encode_is_available_response(available: bool) -> Vec<u8> {
    vec![u8::from(available)]
}

pub fn decode_is_available_response(bytes: &[u8]) -> Result<bool, ServiceError> {
    let mut r = BinReader::new(bytes);
    let available = r.read_bool()?;
    r.finish()?;
    Ok(available)
}

pub fn encode_sensor_subscribe_request(
    kind: SensorKind,
    rate_hz: u16,
) -> Result<Vec<u8>, ServiceError> {
    if rate_hz == 0 || rate_hz > MAX_SENSOR_RATE_HZ {
        return Err(ServiceError::InvalidArg(format!(
            "sensor rate_hz {rate_hz} must be in 1..={MAX_SENSOR_RATE_HZ}"
        )));
    }
    let mut w = BinWriter::with_capacity(3);
    w.write_u8(kind as u8);
    w.write_u16(rate_hz);
    Ok(w.into_bytes())
}

pub fn decode_sensor_subscribe_request(bytes: &[u8]) -> Result<(SensorKind, u16), ServiceError> {
    let mut r = BinReader::new(bytes);
    let kind_raw = r.read_u8()?;
    let kind = SensorKind::from_u8(kind_raw)
        .ok_or_else(|| ServiceError::Codec(format!("invalid SensorKind {kind_raw}")))?;
    let rate_hz = r.read_u16()?;
    r.finish()?;
    if rate_hz == 0 || rate_hz > MAX_SENSOR_RATE_HZ {
        return Err(ServiceError::InvalidArg(format!(
            "sensor rate_hz {rate_hz} must be in 1..={MAX_SENSOR_RATE_HZ}"
        )));
    }
    Ok((kind, rate_hz))
}

pub fn encode_sensor_subscription_id(id: SensorSubscriptionId) -> Result<Vec<u8>, ServiceError> {
    if id.0 == 0 {
        return Err(ServiceError::InvalidArg(
            "sensor subscription id must be non-zero".into(),
        ));
    }
    let mut w = BinWriter::with_capacity(8);
    w.write_u64(id.0);
    Ok(w.into_bytes())
}

pub fn decode_sensor_subscription_id(bytes: &[u8]) -> Result<SensorSubscriptionId, ServiceError> {
    let mut r = BinReader::new(bytes);
    let id = r.read_u64()?;
    r.finish()?;
    if id == 0 {
        return Err(ServiceError::InvalidArg(
            "sensor subscription id must be non-zero".into(),
        ));
    }
    Ok(SensorSubscriptionId(id))
}

/// Typed ergonomic client for hardware inertial and environmental sensors.
#[derive(Debug, Clone, Copy)]
pub struct SensorClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> SensorClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn is_available(&self, kind: SensorKind) -> Result<bool, ServiceError> {
        if let Some(backend) = resolve_typed::<dyn SensorsBackend>(
            self.provider,
            SERVICE_SENSORS,
            METHOD_SENSOR_IS_AVAILABLE,
        ) {
            return backend.is_available(kind);
        }
        let req = encode_sensor_kind(kind);
        let resp =
            self.provider
                .invoke_sync(SERVICE_SENSORS, METHOD_SENSOR_IS_AVAILABLE, &req)?;
        decode_is_available_response(&resp)
    }

    pub fn subscribe(
        &self,
        kind: SensorKind,
        rate_hz: u16,
    ) -> Result<SensorSubscriptionId, ServiceError> {
        if rate_hz == 0 || rate_hz > MAX_SENSOR_RATE_HZ {
            return Err(ServiceError::InvalidArg(format!(
                "sensor rate_hz {rate_hz} must be in 1..={MAX_SENSOR_RATE_HZ}"
            )));
        }
        if let Some(backend) = resolve_typed::<dyn SensorsBackend>(
            self.provider,
            SERVICE_SENSORS,
            METHOD_SENSOR_SUBSCRIBE,
        ) {
            return backend.subscribe(kind, rate_hz);
        }
        let req = encode_sensor_subscribe_request(kind, rate_hz)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SENSORS, METHOD_SENSOR_SUBSCRIBE, &req)?;
        decode_sensor_subscription_id(&resp)
    }

    pub fn unsubscribe(&self, id: SensorSubscriptionId) -> Result<(), ServiceError> {
        if id.0 == 0 {
            return Err(ServiceError::InvalidArg(
                "sensor subscription id must be non-zero".into(),
            ));
        }
        if let Some(backend) = resolve_typed::<dyn SensorsBackend>(
            self.provider,
            SERVICE_SENSORS,
            METHOD_SENSOR_UNSUBSCRIBE,
        ) {
            return backend.unsubscribe(id);
        }
        let req = encode_sensor_subscription_id(id)?;
        let resp =
            self.provider
                .invoke_sync(SERVICE_SENSORS, METHOD_SENSOR_UNSUBSCRIBE, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty sensors::unsubscribe response".into(),
            ));
        }
        Ok(())
    }

    /// Subscribes directly to typed [`SensorBatchEvent`]s on the provider's [`TypedEventBus`].
    pub fn subscribe_events<F>(&self, handler: F) -> Result<TypedEventSubscription, ServiceError>
    where
        F: Fn(&SensorBatchEvent) + Send + Sync + 'static,
    {
        let bus = self
            .provider
            .typed_event_bus()
            .ok_or(ServiceError::Unsupported)?;
        Ok(bus.subscribe::<SensorBatchEvent, F>(false, handler))
    }

    /// Drains queued typed [`SensorBatchEvent`]s directly from the provider's [`TypedEventBus`].
    pub fn drain_events(&self) -> Vec<Arc<SensorBatchEvent>> {
        self.provider
            .typed_event_bus()
            .map(|bus| bus.drain_typed::<SensorBatchEvent>())
            .unwrap_or_default()
    }
}
