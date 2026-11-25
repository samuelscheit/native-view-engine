//! Geolocation capability (`SERVICE_LOCATION = 0x0024`) with zero-JSON binary codecs.

use crate::codec::{BinReader, BinWriter};
use crate::error::SdkError;
pub use crate::provider::PermissionStatus;
use crate::provider::{HostPushEvent, HostServiceProvider};

/// Built-in service ID for geolocation fixes and continuous position watches.
pub const SERVICE_LOCATION: u32 = 0x0024;

pub const METHOD_LOC_REQUEST_PERMISSION: u32 = 1;
pub const METHOD_LOC_CURRENT_POSITION: u32 = 2;
pub const METHOD_LOC_WATCH_POSITION: u32 = 3;
pub const METHOD_LOC_STOP_WATCH: u32 = 4;

/// Host push event ID emitted when a watched location receives a new [`GeoFix`].
pub const EVENT_LOCATION_UPDATE: u32 = 1;

/// Desired accuracy tier for geolocation requests.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocationAccuracy {
    Low = 0,
    Balanced = 1,
    High = 2,
    BestForNavigation = 3,
}

impl LocationAccuracy {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Low),
            1 => Some(Self::Balanced),
            2 => Some(Self::High),
            3 => Some(Self::BestForNavigation),
            _ => None,
        }
    }
}

/// Geographic coordinate fix returned by [`LocationClient`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoFix {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_m: Option<f64>,
    pub accuracy_m: f32,
    pub speed_mps: Option<f32>,
    pub heading_deg: Option<f32>,
    pub timestamp_ms: u64,
}

/// Configuration for [`LocationClient::watch_position`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocationWatchConfig {
    pub accuracy: LocationAccuracy,
    pub min_interval_ms: u32,
    pub min_distance_m: f32,
}

impl Default for LocationWatchConfig {
    fn default() -> Self {
        Self {
            accuracy: LocationAccuracy::Balanced,
            min_interval_ms: 1000,
            min_distance_m: 0.0,
        }
    }
}

/// Opaque identifier for an active geolocation watch subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocationWatchId(pub u64);

/// Push event delivered when a watched geolocation produces a new fix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocationUpdateEvent {
    pub watch_id: LocationWatchId,
    pub fix: GeoFix,
}

fn validate_geo_fix(fix: &GeoFix) -> Result<(), SdkError> {
    if !fix.latitude.is_finite() || !(-90.0..=90.0).contains(&fix.latitude) {
        return Err(SdkError::InvalidArg("latitude must be finite in -90..=90".into()));
    }
    if !fix.longitude.is_finite() || !(-180.0..=180.0).contains(&fix.longitude) {
        return Err(SdkError::InvalidArg("longitude must be finite in -180..=180".into()));
    }
    if !fix.accuracy_m.is_finite() || fix.accuracy_m < 0.0 {
        return Err(SdkError::InvalidArg("accuracy_m must be finite and >= 0".into()));
    }
    if let Some(alt) = fix.altitude_m
        && !alt.is_finite()
    {
        return Err(SdkError::InvalidArg("altitude_m must be finite".into()));
    }
    if let Some(spd) = fix.speed_mps
        && (!spd.is_finite() || spd < 0.0)
    {
        return Err(SdkError::InvalidArg("speed_mps must be finite and >= 0".into()));
    }
    if let Some(hdg) = fix.heading_deg
        && (!hdg.is_finite() || !(0.0..=360.0).contains(&hdg))
    {
        return Err(SdkError::InvalidArg(
            "heading_deg must be finite in 0..=360".into(),
        ));
    }
    Ok(())
}

pub fn encode_accuracy_request(accuracy: LocationAccuracy) -> Vec<u8> {
    vec![accuracy as u8]
}

pub fn decode_accuracy_request(bytes: &[u8]) -> Result<LocationAccuracy, SdkError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    LocationAccuracy::from_u8(raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid LocationAccuracy {raw}")))
}

pub fn encode_geo_fix(fix: &GeoFix) -> Result<Vec<u8>, SdkError> {
    validate_geo_fix(fix)?;
    let mut w = BinWriter::with_capacity(48);
    w.write_f64(fix.latitude);
    w.write_f64(fix.longitude);
    w.write_opt_f64(fix.altitude_m);
    w.write_f32(fix.accuracy_m);
    w.write_opt_f32(fix.speed_mps);
    w.write_opt_f32(fix.heading_deg);
    w.write_u64(fix.timestamp_ms);
    Ok(w.into_bytes())
}

pub fn decode_geo_fix(bytes: &[u8]) -> Result<GeoFix, SdkError> {
    let mut r = BinReader::new(bytes);
    let fix = GeoFix {
        latitude: r.read_f64()?,
        longitude: r.read_f64()?,
        altitude_m: r.read_opt_f64()?,
        accuracy_m: r.read_f32()?,
        speed_mps: r.read_opt_f32()?,
        heading_deg: r.read_opt_f32()?,
        timestamp_ms: r.read_u64()?,
    };
    r.finish()?;
    validate_geo_fix(&fix).map_err(|e| SdkError::Codec(e.to_string()))?;
    Ok(fix)
}

pub fn encode_watch_config(cfg: &LocationWatchConfig) -> Result<Vec<u8>, SdkError> {
    if !cfg.min_distance_m.is_finite() || cfg.min_distance_m < 0.0 {
        return Err(SdkError::InvalidArg(
            "min_distance_m must be finite and >= 0".into(),
        ));
    }
    let mut w = BinWriter::with_capacity(9);
    w.write_u8(cfg.accuracy as u8);
    w.write_u32(cfg.min_interval_ms);
    w.write_f32(cfg.min_distance_m);
    Ok(w.into_bytes())
}

pub fn decode_watch_config(bytes: &[u8]) -> Result<LocationWatchConfig, SdkError> {
    let mut r = BinReader::new(bytes);
    let acc_raw = r.read_u8()?;
    let accuracy = LocationAccuracy::from_u8(acc_raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid LocationAccuracy {acc_raw}")))?;
    let min_interval_ms = r.read_u32()?;
    let min_distance_m = r.read_f32()?;
    r.finish()?;
    if !min_distance_m.is_finite() || min_distance_m < 0.0 {
        return Err(SdkError::InvalidArg(
            "min_distance_m must be finite and >= 0".into(),
        ));
    }
    Ok(LocationWatchConfig {
        accuracy,
        min_interval_ms,
        min_distance_m,
    })
}

pub fn encode_watch_id(id: LocationWatchId) -> Result<Vec<u8>, SdkError> {
    if id.0 == 0 {
        return Err(SdkError::InvalidArg("location watch id must be non-zero".into()));
    }
    let mut w = BinWriter::with_capacity(8);
    w.write_u64(id.0);
    Ok(w.into_bytes())
}

pub fn decode_watch_id(bytes: &[u8]) -> Result<LocationWatchId, SdkError> {
    let mut r = BinReader::new(bytes);
    let id = r.read_u64()?;
    r.finish()?;
    if id == 0 {
        return Err(SdkError::InvalidArg("location watch id must be non-zero".into()));
    }
    Ok(LocationWatchId(id))
}

pub fn encode_location_update_event(ev: &LocationUpdateEvent) -> Result<Vec<u8>, SdkError> {
    if ev.watch_id.0 == 0 {
        return Err(SdkError::InvalidArg("location watch id must be non-zero".into()));
    }
    let fix_bytes = encode_geo_fix(&ev.fix)?;
    let mut w = BinWriter::with_capacity(8 + fix_bytes.len());
    w.write_u64(ev.watch_id.0);
    w.write_raw(&fix_bytes);
    Ok(w.into_bytes())
}

pub fn decode_location_update_event(bytes: &[u8]) -> Result<LocationUpdateEvent, SdkError> {
    if bytes.len() < 8 {
        return Err(SdkError::Codec("location update event truncated".into()));
    }
    let id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    if id == 0 {
        return Err(SdkError::Codec("location watch id must be non-zero".into()));
    }
    Ok(LocationUpdateEvent {
        watch_id: LocationWatchId(id),
        fix: decode_geo_fix(&bytes[8..])?,
    })
}

/// Typed ergonomic client for geolocation fixes and continuous position watches.
#[derive(Debug, Clone, Copy)]
pub struct LocationClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> LocationClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn request_permission(&self) -> Result<PermissionStatus, SdkError> {
        let resp = self
            .provider
            .invoke_sync(SERVICE_LOCATION, METHOD_LOC_REQUEST_PERMISSION, &[])?;
        PermissionStatus::decode(&resp)
    }

    pub fn current_position(&self, accuracy: LocationAccuracy) -> Result<GeoFix, SdkError> {
        let req = encode_accuracy_request(accuracy);
        let resp =
            self.provider
                .invoke_sync(SERVICE_LOCATION, METHOD_LOC_CURRENT_POSITION, &req)?;
        decode_geo_fix(&resp)
    }

    pub fn watch_position(
        &self,
        config: &LocationWatchConfig,
    ) -> Result<LocationWatchId, SdkError> {
        let req = encode_watch_config(config)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_LOCATION, METHOD_LOC_WATCH_POSITION, &req)?;
        decode_watch_id(&resp)
    }

    pub fn stop_watch(&self, watch_id: LocationWatchId) -> Result<(), SdkError> {
        let req = encode_watch_id(watch_id)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_LOCATION, METHOD_LOC_STOP_WATCH, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec(
                "unexpected non-empty location::stop_watch response".into(),
            ));
        }
        Ok(())
    }

    pub fn decode_push_event(
        event: &HostPushEvent,
    ) -> Result<Option<LocationUpdateEvent>, SdkError> {
        if event.service_id != SERVICE_LOCATION {
            return Ok(None);
        }
        match event.event_id {
            EVENT_LOCATION_UPDATE => Ok(Some(decode_location_update_event(&event.payload)?)),
            other => Err(SdkError::Codec(format!("unknown location event_id {other}"))),
        }
    }
}
