//! Audio playback & recording capability (`SERVICE_AUDIO = 0x0022`) with direct
//! zero-serialization in-process dispatch ([`AudioBackend`]), direct typed playback/level
//! meter events ([`AudioEvent`] over [`TypedEventBus`]), [`MemoryAudio`], and [`NativeAudio`].

pub mod handler;

use std::sync::Arc;

use nve_host_services::{
    resolve_typed, BinReader, BinWriter, HostServiceProvider, ServiceError, TypedEventBus,
    TypedEventSubscription,
};

pub use handler::{dispatch_audio_wire, AudioBackend, MemoryAudio, MockAudioTrack, NativeAudio};

/// Built-in service ID for audio playback and microphone recording.
pub const SERVICE_AUDIO: u32 = 0x0022;

pub const METHOD_LOAD_SOURCE: u32 = 1;
pub const METHOD_UNLOAD: u32 = 2;
pub const METHOD_PLAY: u32 = 3;
pub const METHOD_PAUSE: u32 = 4;
pub const METHOD_STOP: u32 = 5;
pub const METHOD_SEEK_MS: u32 = 6;
pub const METHOD_SET_VOLUME: u32 = 7;
pub const METHOD_SET_LOOPING: u32 = 8;
pub const METHOD_START_RECORDING: u32 = 9;
pub const METHOD_STOP_RECORDING: u32 = 10;

/// Host push event ID emitted on playback state/position transitions.
pub const EVENT_PLAYBACK_STATE: u32 = 1;
/// Host push event ID emitted for audio playback/recording level meters.
pub const EVENT_LEVEL_METER: u32 = 2;

/// Opaque handle identifying a loaded audio playback stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioHandle(pub u64);

/// Input source for [`AudioClient::load_source`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioSource {
    Uri(String),
    Bytes(Vec<u8>),
}

/// Audio codec/container format for recording and clip metadata.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioEncoding {
    Pcm16 = 0,
    Aac = 1,
    Opus = 2,
    Wav = 3,
    Flac = 4,
}

impl AudioEncoding {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Pcm16),
            1 => Some(Self::Aac),
            2 => Some(Self::Opus),
            3 => Some(Self::Wav),
            4 => Some(Self::Flac),
            _ => None,
        }
    }
}

/// Configuration for [`AudioClient::start_recording`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRecordConfig {
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub encoding: AudioEncoding,
    pub bit_rate_bps: Option<u32>,
    pub output_path: Option<String>,
    pub max_duration_ms: Option<u64>,
}

impl Default for AudioRecordConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: 48_000,
            channels: 1,
            encoding: AudioEncoding::Pcm16,
            bit_rate_bps: None,
            output_path: None,
            max_duration_ms: None,
        }
    }
}

/// Metadata returned by [`AudioClient::stop_recording`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioClipMeta {
    pub path: Option<String>,
    pub encoding: AudioEncoding,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub duration_ms: u64,
    pub byte_len: u64,
}

/// Playback lifecycle state carried by [`AudioPlaybackEvent`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioPlaybackState {
    Idle = 0,
    Playing = 1,
    Paused = 2,
    Stopped = 3,
    Completed = 4,
}

impl AudioPlaybackState {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Idle),
            1 => Some(Self::Playing),
            2 => Some(Self::Paused),
            3 => Some(Self::Stopped),
            4 => Some(Self::Completed),
            _ => None,
        }
    }
}

/// Binary playback status event pushed by the host audio engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioPlaybackEvent {
    pub handle: AudioHandle,
    pub state: AudioPlaybackState,
    pub position_ms: u64,
    pub duration_ms: u64,
}

/// Binary RMS/peak meter sample pushed during playback or microphone recording.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioLevelEvent {
    pub handle: Option<AudioHandle>,
    pub rms_db: f32,
    pub peak_db: f32,
    pub timestamp_ms: u64,
}

/// Typed push events emitted under [`SERVICE_AUDIO`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioEvent {
    Playback(AudioPlaybackEvent),
    Level(AudioLevelEvent),
}

fn validate_handle(handle: AudioHandle) -> Result<(), ServiceError> {
    if handle.0 == 0 {
        return Err(ServiceError::InvalidArg(
            "audio handle must be non-zero".into(),
        ));
    }
    Ok(())
}

fn validate_volume(volume: f32) -> Result<(), ServiceError> {
    if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
        // Static message: avoids linking `core::fmt` float formatting into wasm bundles.
        return Err(ServiceError::InvalidArg(
            "audio volume must be finite and in 0.0..=1.0".to_owned(),
        ));
    }
    Ok(())
}

fn validate_record_config(cfg: &AudioRecordConfig) -> Result<(), ServiceError> {
    if !(8_000..=192_000).contains(&cfg.sample_rate_hz) {
        return Err(ServiceError::InvalidArg(format!(
            "sample_rate_hz {} must be in 8000..=192000",
            cfg.sample_rate_hz
        )));
    }
    if !(1..=8).contains(&cfg.channels) {
        return Err(ServiceError::InvalidArg(format!(
            "channels {} must be in 1..=8",
            cfg.channels
        )));
    }
    if let Some(br) = cfg.bit_rate_bps
        && br == 0
    {
        return Err(ServiceError::InvalidArg(
            "bit_rate_bps must be non-zero".into(),
        ));
    }
    if let Some(path) = &cfg.output_path
        && path.trim().is_empty()
    {
        return Err(ServiceError::InvalidArg(
            "output_path must not be empty when specified".into(),
        ));
    }
    Ok(())
}

pub fn encode_load_source_request(source: &AudioSource) -> Result<Vec<u8>, ServiceError> {
    let mut w = BinWriter::new();
    match source {
        AudioSource::Uri(uri) => {
            if uri.trim().is_empty() {
                return Err(ServiceError::InvalidArg(
                    "audio URI must not be empty".into(),
                ));
            }
            w.write_u8(1);
            w.write_str(uri)?;
        }
        AudioSource::Bytes(data) => {
            if data.is_empty() {
                return Err(ServiceError::InvalidArg(
                    "audio bytes must not be empty".into(),
                ));
            }
            w.write_u8(2);
            w.write_bytes(data)?;
        }
    }
    Ok(w.into_bytes())
}

pub fn decode_load_source_request(bytes: &[u8]) -> Result<AudioSource, ServiceError> {
    let mut r = BinReader::new(bytes);
    let source = match r.read_u8()? {
        1 => {
            let uri = r.read_string()?;
            if uri.trim().is_empty() {
                return Err(ServiceError::InvalidArg(
                    "audio URI must not be empty".into(),
                ));
            }
            AudioSource::Uri(uri)
        }
        2 => {
            let data = r.read_bytes()?.to_vec();
            if data.is_empty() {
                return Err(ServiceError::InvalidArg(
                    "audio bytes must not be empty".into(),
                ));
            }
            AudioSource::Bytes(data)
        }
        other => {
            return Err(ServiceError::Codec(format!(
                "invalid AudioSource tag {other}"
            )));
        }
    };
    r.finish()?;
    Ok(source)
}

pub fn encode_handle(handle: AudioHandle) -> Result<Vec<u8>, ServiceError> {
    validate_handle(handle)?;
    let mut w = BinWriter::with_capacity(8);
    w.write_u64(handle.0);
    Ok(w.into_bytes())
}

pub fn decode_handle(bytes: &[u8]) -> Result<AudioHandle, ServiceError> {
    let mut r = BinReader::new(bytes);
    let handle = AudioHandle(r.read_u64()?);
    r.finish()?;
    validate_handle(handle)?;
    Ok(handle)
}

pub fn encode_seek_request(
    handle: AudioHandle,
    position_ms: u64,
) -> Result<Vec<u8>, ServiceError> {
    validate_handle(handle)?;
    let mut w = BinWriter::with_capacity(16);
    w.write_u64(handle.0);
    w.write_u64(position_ms);
    Ok(w.into_bytes())
}

pub fn decode_seek_request(bytes: &[u8]) -> Result<(AudioHandle, u64), ServiceError> {
    let mut r = BinReader::new(bytes);
    let handle = AudioHandle(r.read_u64()?);
    let position_ms = r.read_u64()?;
    r.finish()?;
    validate_handle(handle)?;
    Ok((handle, position_ms))
}

pub fn encode_set_volume_request(
    handle: AudioHandle,
    volume: f32,
) -> Result<Vec<u8>, ServiceError> {
    validate_handle(handle)?;
    validate_volume(volume)?;
    let mut w = BinWriter::with_capacity(12);
    w.write_u64(handle.0);
    w.write_f32(volume);
    Ok(w.into_bytes())
}

pub fn decode_set_volume_request(bytes: &[u8]) -> Result<(AudioHandle, f32), ServiceError> {
    let mut r = BinReader::new(bytes);
    let handle = AudioHandle(r.read_u64()?);
    let volume = r.read_f32()?;
    r.finish()?;
    validate_handle(handle)?;
    validate_volume(volume)?;
    Ok((handle, volume))
}

pub fn encode_set_looping_request(
    handle: AudioHandle,
    looping: bool,
) -> Result<Vec<u8>, ServiceError> {
    validate_handle(handle)?;
    let mut w = BinWriter::with_capacity(9);
    w.write_u64(handle.0);
    w.write_bool(looping);
    Ok(w.into_bytes())
}

pub fn decode_set_looping_request(bytes: &[u8]) -> Result<(AudioHandle, bool), ServiceError> {
    let mut r = BinReader::new(bytes);
    let handle = AudioHandle(r.read_u64()?);
    let looping = r.read_bool()?;
    r.finish()?;
    validate_handle(handle)?;
    Ok((handle, looping))
}

pub fn encode_record_config(cfg: &AudioRecordConfig) -> Result<Vec<u8>, ServiceError> {
    validate_record_config(cfg)?;
    let mut w = BinWriter::with_capacity(32);
    w.write_u32(cfg.sample_rate_hz);
    w.write_u8(cfg.channels);
    w.write_u8(cfg.encoding as u8);
    w.write_opt_u32(cfg.bit_rate_bps);
    w.write_opt_str(cfg.output_path.as_deref())?;
    w.write_opt_u64(cfg.max_duration_ms);
    Ok(w.into_bytes())
}

pub fn decode_record_config(bytes: &[u8]) -> Result<AudioRecordConfig, ServiceError> {
    let mut r = BinReader::new(bytes);
    let sample_rate_hz = r.read_u32()?;
    let channels = r.read_u8()?;
    let enc_raw = r.read_u8()?;
    let encoding = AudioEncoding::from_u8(enc_raw)
        .ok_or_else(|| ServiceError::Codec(format!("invalid AudioEncoding {enc_raw}")))?;
    let bit_rate_bps = r.read_opt_u32()?;
    let output_path = r.read_opt_string()?;
    let max_duration_ms = r.read_opt_u64()?;
    r.finish()?;
    let cfg = AudioRecordConfig {
        sample_rate_hz,
        channels,
        encoding,
        bit_rate_bps,
        output_path,
        max_duration_ms,
    };
    validate_record_config(&cfg)?;
    Ok(cfg)
}

pub fn encode_clip_meta(meta: &AudioClipMeta) -> Result<Vec<u8>, ServiceError> {
    let mut w = BinWriter::with_capacity(32);
    w.write_opt_str(meta.path.as_deref())?;
    w.write_u8(meta.encoding as u8);
    w.write_u32(meta.sample_rate_hz);
    w.write_u8(meta.channels);
    w.write_u64(meta.duration_ms);
    w.write_u64(meta.byte_len);
    Ok(w.into_bytes())
}

pub fn decode_clip_meta(bytes: &[u8]) -> Result<AudioClipMeta, ServiceError> {
    let mut r = BinReader::new(bytes);
    let path = r.read_opt_string()?;
    let enc_raw = r.read_u8()?;
    let encoding = AudioEncoding::from_u8(enc_raw)
        .ok_or_else(|| ServiceError::Codec(format!("invalid AudioEncoding {enc_raw}")))?;
    let sample_rate_hz = r.read_u32()?;
    let channels = r.read_u8()?;
    let duration_ms = r.read_u64()?;
    let byte_len = r.read_u64()?;
    r.finish()?;
    Ok(AudioClipMeta {
        path,
        encoding,
        sample_rate_hz,
        channels,
        duration_ms,
        byte_len,
    })
}

/// Validates an [`AudioEvent`] (non-zero handle and finite dB readings).
pub fn validate_audio_event(ev: &AudioEvent) -> Result<(), ServiceError> {
    match ev {
        AudioEvent::Playback(p) => validate_handle(p.handle),
        AudioEvent::Level(l) => {
            if !l.rms_db.is_finite() || !l.peak_db.is_finite() {
                return Err(ServiceError::InvalidArg(
                    "audio level rms_db and peak_db must be finite".into(),
                ));
            }
            if let Some(h) = l.handle {
                validate_handle(h)?;
            }
            Ok(())
        }
    }
}

/// Validates and publishes a typed [`AudioEvent`] directly onto `bus` with zero serialization.
pub fn emit_audio_event(bus: &TypedEventBus, ev: AudioEvent) -> Result<usize, ServiceError> {
    validate_audio_event(&ev)?;
    Ok(bus.publish(ev))
}

fn validate_source(source: &AudioSource) -> Result<(), ServiceError> {
    match source {
        AudioSource::Uri(uri) if uri.trim().is_empty() => Err(ServiceError::InvalidArg(
            "audio URI must not be empty".into(),
        )),
        AudioSource::Bytes(data) if data.is_empty() => Err(ServiceError::InvalidArg(
            "audio bytes must not be empty".into(),
        )),
        _ => Ok(()),
    }
}

/// Typed ergonomic client for native audio playback and microphone recording.
#[derive(Debug, Clone, Copy)]
pub struct AudioClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> AudioClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn load_source(&self, source: &AudioSource) -> Result<AudioHandle, ServiceError> {
        validate_source(source)?;
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_LOAD_SOURCE)
        {
            return backend.load_source(source);
        }
        let req = encode_load_source_request(source)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_AUDIO, METHOD_LOAD_SOURCE, &req)?;
        decode_handle(&resp)
    }

    pub fn unload(&self, handle: AudioHandle) -> Result<(), ServiceError> {
        validate_handle(handle)?;
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_UNLOAD)
        {
            return backend.unload(handle);
        }
        self.invoke_void(METHOD_UNLOAD, &encode_handle(handle)?, "audio::unload")
    }

    pub fn play(&self, handle: AudioHandle) -> Result<(), ServiceError> {
        validate_handle(handle)?;
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_PLAY)
        {
            return backend.play(handle);
        }
        self.invoke_void(METHOD_PLAY, &encode_handle(handle)?, "audio::play")
    }

    pub fn pause(&self, handle: AudioHandle) -> Result<(), ServiceError> {
        validate_handle(handle)?;
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_PAUSE)
        {
            return backend.pause(handle);
        }
        self.invoke_void(METHOD_PAUSE, &encode_handle(handle)?, "audio::pause")
    }

    pub fn stop(&self, handle: AudioHandle) -> Result<(), ServiceError> {
        validate_handle(handle)?;
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_STOP)
        {
            return backend.stop(handle);
        }
        self.invoke_void(METHOD_STOP, &encode_handle(handle)?, "audio::stop")
    }

    pub fn seek_ms(&self, handle: AudioHandle, position_ms: u64) -> Result<(), ServiceError> {
        validate_handle(handle)?;
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_SEEK_MS)
        {
            return backend.seek_ms(handle, position_ms);
        }
        let req = encode_seek_request(handle, position_ms)?;
        self.invoke_void(METHOD_SEEK_MS, &req, "audio::seek_ms")
    }

    pub fn set_volume(&self, handle: AudioHandle, volume: f32) -> Result<(), ServiceError> {
        validate_handle(handle)?;
        validate_volume(volume)?;
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_SET_VOLUME)
        {
            return backend.set_volume(handle, volume);
        }
        let req = encode_set_volume_request(handle, volume)?;
        self.invoke_void(METHOD_SET_VOLUME, &req, "audio::set_volume")
    }

    pub fn set_looping(&self, handle: AudioHandle, looping: bool) -> Result<(), ServiceError> {
        validate_handle(handle)?;
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_SET_LOOPING)
        {
            return backend.set_looping(handle, looping);
        }
        let req = encode_set_looping_request(handle, looping)?;
        self.invoke_void(METHOD_SET_LOOPING, &req, "audio::set_looping")
    }

    pub fn start_recording(&self, config: &AudioRecordConfig) -> Result<(), ServiceError> {
        validate_record_config(config)?;
        if let Some(backend) = resolve_typed::<dyn AudioBackend>(
            self.provider,
            SERVICE_AUDIO,
            METHOD_START_RECORDING,
        ) {
            return backend.start_recording(config);
        }
        let req = encode_record_config(config)?;
        self.invoke_void(METHOD_START_RECORDING, &req, "audio::start_recording")
    }

    pub fn stop_recording(&self) -> Result<AudioClipMeta, ServiceError> {
        if let Some(backend) =
            resolve_typed::<dyn AudioBackend>(self.provider, SERVICE_AUDIO, METHOD_STOP_RECORDING)
        {
            return backend.stop_recording();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_AUDIO, METHOD_STOP_RECORDING, &[])?;
        decode_clip_meta(&resp)
    }

    /// Subscribes directly to typed [`AudioEvent`]s on the provider's [`TypedEventBus`].
    pub fn subscribe_events<F>(&self, handler: F) -> Result<TypedEventSubscription, ServiceError>
    where
        F: Fn(&AudioEvent) + Send + Sync + 'static,
    {
        let bus = self
            .provider
            .typed_event_bus()
            .ok_or(ServiceError::Unsupported)?;
        Ok(bus.subscribe::<AudioEvent, F>(false, handler))
    }

    /// Drains queued typed [`AudioEvent`]s directly from the provider's [`TypedEventBus`].
    pub fn drain_events(&self) -> Vec<Arc<AudioEvent>> {
        self.provider
            .typed_event_bus()
            .map(|bus| bus.drain_typed::<AudioEvent>())
            .unwrap_or_default()
    }

    fn invoke_void(&self, method_id: u32, req: &[u8], label: &str) -> Result<(), ServiceError> {
        let resp = self.provider.invoke_sync(SERVICE_AUDIO, method_id, req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(format!(
                "unexpected non-empty {label} response"
            )));
        }
        Ok(())
    }
}
