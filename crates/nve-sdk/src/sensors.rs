//! Geolocation (`SERVICE_LOCATION = 0x0024`) and device motion/environment sensors
//! (`SERVICE_SENSORS = 0x0025`) with zero-copy fixed-layout binary `SensorSample` decoding.

use crate::codec::{BinReader, BinWriter, MAX_SEQ_ITEMS};
use crate::error::SdkError;
pub use crate::location::*;
use crate::provider::{HostPushEvent, HostServiceProvider};

/// Built-in service ID for hardware inertial & environmental sensor streams.
pub const SERVICE_SENSORS: u32 = 0x0025;

pub const METHOD_SENSOR_IS_AVAILABLE: u32 = 1;
pub const METHOD_SENSOR_SUBSCRIBE: u32 = 2;
pub const METHOD_SENSOR_UNSUBSCRIBE: u32 = 3;

/// Host push event ID carrying a single fixed-layout 20-byte [`SensorSample`].
pub const EVENT_SENSOR_SAMPLE: u32 = 1;
/// Host push event ID carrying a packed batch of zero-copy 20-byte [`SensorSample`] records.
pub const EVENT_SENSOR_BATCH: u32 = 2;

/// Fixed wire size in bytes of one little-endian [`SensorSample`] (`3 * f32 + u64`).
pub const SENSOR_SAMPLE_WIRE_LEN: usize = 20;

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
///
/// On the wire each sample occupies exactly [`SENSOR_SAMPLE_WIRE_LEN`] (20) little-endian bytes:
/// `[x: f32][y: f32][z: f32][timestamp_ns: u64]`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorSample {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub timestamp_ns: u64,
}

// Lock #[repr(C)] ABI layout at compile time.
const _: () = {
    assert!(std::mem::size_of::<SensorSample>() == 24);
    assert!(std::mem::offset_of!(SensorSample, x) == 0);
    assert!(std::mem::offset_of!(SensorSample, y) == 4);
    assert!(std::mem::offset_of!(SensorSample, z) == 8);
    assert!(std::mem::offset_of!(SensorSample, timestamp_ns) == 16);
};

impl SensorSample {
    /// Encodes this sample into a fixed 20-byte little-endian array without heap allocation.
    pub fn encode(&self) -> Result<[u8; SENSOR_SAMPLE_WIRE_LEN], SdkError> {
        if !self.x.is_finite() || !self.y.is_finite() || !self.z.is_finite() {
            return Err(SdkError::InvalidArg(
                "SensorSample axes x/y/z must be finite".into(),
            ));
        }
        let mut out = [0u8; SENSOR_SAMPLE_WIRE_LEN];
        out[0..4].copy_from_slice(&self.x.to_le_bytes());
        out[4..8].copy_from_slice(&self.y.to_le_bytes());
        out[8..12].copy_from_slice(&self.z.to_le_bytes());
        out[12..20].copy_from_slice(&self.timestamp_ns.to_le_bytes());
        Ok(out)
    }

    /// Decodes exactly 20 little-endian bytes into a validated [`SensorSample`] with zero allocation.
    pub fn decode(bytes: &[u8]) -> Result<Self, SdkError> {
        if bytes.len() != SENSOR_SAMPLE_WIRE_LEN {
            return Err(SdkError::Codec(format!(
                "invalid SensorSample byte length {}: expected {SENSOR_SAMPLE_WIRE_LEN}",
                bytes.len()
            )));
        }
        let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err(SdkError::Codec(
                "SensorSample contains non-finite float axis".into(),
            ));
        }
        let timestamp_ns = u64::from_le_bytes([
            bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19],
        ]);
        Ok(Self {
            x,
            y,
            z,
            timestamp_ns,
        })
    }
}

/// Zero-copy borrowed view over packed 20-byte [`SensorSample`] records.
#[derive(Debug, Clone, Copy)]
pub struct SensorSampleSlice<'a> {
    raw: &'a [u8],
}

impl<'a> SensorSampleSlice<'a> {
    /// Validates alignment to 20-byte boundaries and finite floats across all packed samples.
    pub fn from_bytes(raw: &'a [u8]) -> Result<Self, SdkError> {
        if !raw.len().is_multiple_of(SENSOR_SAMPLE_WIRE_LEN) {
            return Err(SdkError::Codec(format!(
                "sensor sample buffer length {} is not a multiple of {SENSOR_SAMPLE_WIRE_LEN}",
                raw.len()
            )));
        }
        let count = raw.len() / SENSOR_SAMPLE_WIRE_LEN;
        if count > MAX_SEQ_ITEMS {
            return Err(SdkError::Codec(format!(
                "sensor sample batch count {count} exceeds limit {MAX_SEQ_ITEMS}"
            )));
        }
        for i in 0..count {
            let off = i * SENSOR_SAMPLE_WIRE_LEN;
            let _ = SensorSample::decode(&raw[off..off + SENSOR_SAMPLE_WIRE_LEN])?;
        }
        Ok(Self { raw })
    }

    pub fn len(&self) -> usize {
        self.raw.len() / SENSOR_SAMPLE_WIRE_LEN
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<SensorSample> {
        let off = index.checked_mul(SENSOR_SAMPLE_WIRE_LEN)?;
        let slice = self.raw.get(off..off + SENSOR_SAMPLE_WIRE_LEN)?;
        SensorSample::decode(slice).ok()
    }

    pub fn iter(&self) -> impl Iterator<Item = SensorSample> + 'a {
        self.raw
            .as_chunks::<SENSOR_SAMPLE_WIRE_LEN>()
            .0
            .iter()
            .map(|chunk| SensorSample::decode(chunk).expect("validated on construction"))
    }
}

/// Zero-copy decoded sensor batch push event.
#[derive(Debug, Clone, Copy)]
pub struct SensorBatchEventRef<'a> {
    pub subscription_id: SensorSubscriptionId,
    pub kind: SensorKind,
    pub samples: SensorSampleSlice<'a>,
}

/// Owned decoded sensor push event.
#[derive(Debug, Clone, PartialEq)]
pub struct SensorBatchEvent {
    pub subscription_id: SensorSubscriptionId,
    pub kind: SensorKind,
    pub samples: Vec<SensorSample>,
}

pub fn encode_sensor_kind(kind: SensorKind) -> Vec<u8> {
    vec![kind as u8]
}

pub fn decode_sensor_kind(bytes: &[u8]) -> Result<SensorKind, SdkError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    SensorKind::from_u8(raw).ok_or_else(|| SdkError::Codec(format!("invalid SensorKind {raw}")))
}

pub fn encode_is_available_response(available: bool) -> Vec<u8> {
    vec![u8::from(available)]
}

pub fn decode_is_available_response(bytes: &[u8]) -> Result<bool, SdkError> {
    let mut r = BinReader::new(bytes);
    let available = r.read_bool()?;
    r.finish()?;
    Ok(available)
}

pub fn encode_sensor_subscribe_request(kind: SensorKind, rate_hz: u16) -> Result<Vec<u8>, SdkError> {
    if rate_hz == 0 || rate_hz > MAX_SENSOR_RATE_HZ {
        return Err(SdkError::InvalidArg(format!(
            "sensor rate_hz {rate_hz} must be in 1..={MAX_SENSOR_RATE_HZ}"
        )));
    }
    let mut w = BinWriter::with_capacity(3);
    w.write_u8(kind as u8);
    w.write_u16(rate_hz);
    Ok(w.into_bytes())
}

pub fn decode_sensor_subscribe_request(bytes: &[u8]) -> Result<(SensorKind, u16), SdkError> {
    let mut r = BinReader::new(bytes);
    let kind_raw = r.read_u8()?;
    let kind = SensorKind::from_u8(kind_raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid SensorKind {kind_raw}")))?;
    let rate_hz = r.read_u16()?;
    r.finish()?;
    if rate_hz == 0 || rate_hz > MAX_SENSOR_RATE_HZ {
        return Err(SdkError::InvalidArg(format!(
            "sensor rate_hz {rate_hz} must be in 1..={MAX_SENSOR_RATE_HZ}"
        )));
    }
    Ok((kind, rate_hz))
}

pub fn encode_sensor_subscription_id(id: SensorSubscriptionId) -> Result<Vec<u8>, SdkError> {
    if id.0 == 0 {
        return Err(SdkError::InvalidArg(
            "sensor subscription id must be non-zero".into(),
        ));
    }
    let mut w = BinWriter::with_capacity(8);
    w.write_u64(id.0);
    Ok(w.into_bytes())
}

pub fn decode_sensor_subscription_id(bytes: &[u8]) -> Result<SensorSubscriptionId, SdkError> {
    let mut r = BinReader::new(bytes);
    let id = r.read_u64()?;
    r.finish()?;
    if id == 0 {
        return Err(SdkError::InvalidArg(
            "sensor subscription id must be non-zero".into(),
        ));
    }
    Ok(SensorSubscriptionId(id))
}

/// Encodes a packed sensor batch push payload (`sub_id: u64, kind: u8, count: u32, samples: [20B]`).
pub fn encode_sensor_batch_event(
    subscription_id: SensorSubscriptionId,
    kind: SensorKind,
    samples: &[SensorSample],
) -> Result<Vec<u8>, SdkError> {
    if subscription_id.0 == 0 {
        return Err(SdkError::InvalidArg(
            "sensor subscription id must be non-zero".into(),
        ));
    }
    if samples.len() > MAX_SEQ_ITEMS {
        return Err(SdkError::InvalidArg(
            "sensor sample batch exceeds MAX_SEQ_ITEMS".into(),
        ));
    }
    let mut w = BinWriter::with_capacity(13 + samples.len() * SENSOR_SAMPLE_WIRE_LEN);
    w.write_u64(subscription_id.0);
    w.write_u8(kind as u8);
    w.write_u32(samples.len() as u32);
    for sample in samples {
        w.write_raw(&sample.encode()?);
    }
    Ok(w.into_bytes())
}

/// Zero-copy decoder for packed binary sensor batch events.
pub fn decode_sensor_batch_event_ref(bytes: &[u8]) -> Result<SensorBatchEventRef<'_>, SdkError> {
    let mut r = BinReader::new(bytes);
    let sub_id = r.read_u64()?;
    if sub_id == 0 {
        return Err(SdkError::Codec(
            "sensor subscription id must be non-zero".into(),
        ));
    }
    let kind_raw = r.read_u8()?;
    let kind = SensorKind::from_u8(kind_raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid SensorKind {kind_raw}")))?;
    let count = r.read_seq_len(SENSOR_SAMPLE_WIRE_LEN)?;
    let byte_len = count
        .checked_mul(SENSOR_SAMPLE_WIRE_LEN)
        .ok_or_else(|| SdkError::Codec("sensor sample byte length overflow".into()))?;
    let raw = r.read_exact(byte_len)?;
    r.finish()?;
    let samples = SensorSampleSlice::from_bytes(raw)?;
    Ok(SensorBatchEventRef {
        subscription_id: SensorSubscriptionId(sub_id),
        kind,
        samples,
    })
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

    pub fn is_available(&self, kind: SensorKind) -> Result<bool, SdkError> {
        let req = encode_sensor_kind(kind);
        let resp = self
            .provider
            .invoke_sync(SERVICE_SENSORS, METHOD_SENSOR_IS_AVAILABLE, &req)?;
        decode_is_available_response(&resp)
    }

    pub fn subscribe(
        &self,
        kind: SensorKind,
        rate_hz: u16,
    ) -> Result<SensorSubscriptionId, SdkError> {
        let req = encode_sensor_subscribe_request(kind, rate_hz)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SENSORS, METHOD_SENSOR_SUBSCRIBE, &req)?;
        decode_sensor_subscription_id(&resp)
    }

    pub fn unsubscribe(&self, id: SensorSubscriptionId) -> Result<(), SdkError> {
        let req = encode_sensor_subscription_id(id)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SENSORS, METHOD_SENSOR_UNSUBSCRIBE, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec(
                "unexpected non-empty sensors::unsubscribe response".into(),
            ));
        }
        Ok(())
    }

    /// Zero-copy decode of a [`HostPushEvent`] carrying packed [`SensorSample`] data.
    pub fn decode_push_event_ref(
        event: &HostPushEvent,
    ) -> Result<Option<SensorBatchEventRef<'_>>, SdkError> {
        if event.service_id != SERVICE_SENSORS {
            return Ok(None);
        }
        match event.event_id {
            EVENT_SENSOR_SAMPLE | EVENT_SENSOR_BATCH => {
                Ok(Some(decode_sensor_batch_event_ref(&event.payload)?))
            }
            other => Err(SdkError::Codec(format!("unknown sensors event_id {other}"))),
        }
    }

    /// Owned convenience wrapper over [`Self::decode_push_event_ref`].
    pub fn decode_push_event(event: &HostPushEvent) -> Result<Option<SensorBatchEvent>, SdkError> {
        Ok(Self::decode_push_event_ref(event)?.map(|ev| SensorBatchEvent {
            subscription_id: ev.subscription_id,
            kind: ev.kind,
            samples: ev.samples.iter().collect(),
        }))
    }
}
