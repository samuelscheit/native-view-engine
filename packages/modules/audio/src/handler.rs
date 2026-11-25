//! Direct typed [`AudioBackend`] plus [`MemoryAudio`] and headless-safe [`NativeAudio`] handlers.

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::{
    decode_handle, decode_load_source_request, decode_record_config, decode_seek_request,
    decode_set_looping_request, decode_set_volume_request, encode_clip_meta, encode_handle,
    AudioClipMeta, AudioHandle, AudioPlaybackState, AudioRecordConfig, AudioSource,
    METHOD_LOAD_SOURCE, METHOD_PAUSE, METHOD_PLAY, METHOD_SEEK_MS, METHOD_SET_LOOPING,
    METHOD_SET_VOLUME, METHOD_START_RECORDING, METHOD_STOP, METHOD_STOP_RECORDING, METHOD_UNLOAD,
    SERVICE_AUDIO,
};

/// Direct typed in-process backend for [`SERVICE_AUDIO`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait AudioBackend: Send + Sync {
    fn load_source(&self, source: &AudioSource) -> Result<AudioHandle, ServiceError>;
    fn unload(&self, handle: AudioHandle) -> Result<(), ServiceError>;
    fn play(&self, handle: AudioHandle) -> Result<(), ServiceError>;
    fn pause(&self, handle: AudioHandle) -> Result<(), ServiceError>;
    fn stop(&self, handle: AudioHandle) -> Result<(), ServiceError>;
    fn seek_ms(&self, handle: AudioHandle, position_ms: u64) -> Result<(), ServiceError>;
    fn set_volume(&self, handle: AudioHandle, volume: f32) -> Result<(), ServiceError>;
    fn set_looping(&self, handle: AudioHandle, looping: bool) -> Result<(), ServiceError>;
    fn start_recording(&self, config: &AudioRecordConfig) -> Result<(), ServiceError>;
    fn stop_recording(&self) -> Result<AudioClipMeta, ServiceError>;
}

/// Bridges an [`AudioBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_audio_wire<B: AudioBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_LOAD_SOURCE => {
            let source = decode_load_source_request(request)?;
            encode_handle(backend.load_source(&source)?)
        }
        METHOD_UNLOAD => {
            backend.unload(decode_handle(request)?)?;
            Ok(Vec::new())
        }
        METHOD_PLAY => {
            backend.play(decode_handle(request)?)?;
            Ok(Vec::new())
        }
        METHOD_PAUSE => {
            backend.pause(decode_handle(request)?)?;
            Ok(Vec::new())
        }
        METHOD_STOP => {
            backend.stop(decode_handle(request)?)?;
            Ok(Vec::new())
        }
        METHOD_SEEK_MS => {
            let (handle, pos) = decode_seek_request(request)?;
            backend.seek_ms(handle, pos)?;
            Ok(Vec::new())
        }
        METHOD_SET_VOLUME => {
            let (handle, volume) = decode_set_volume_request(request)?;
            backend.set_volume(handle, volume)?;
            Ok(Vec::new())
        }
        METHOD_SET_LOOPING => {
            let (handle, looping) = decode_set_looping_request(request)?;
            backend.set_looping(handle, looping)?;
            Ok(Vec::new())
        }
        METHOD_START_RECORDING => {
            let cfg = decode_record_config(request)?;
            backend.start_recording(&cfg)?;
            Ok(Vec::new())
        }
        METHOD_STOP_RECORDING => encode_clip_meta(&backend.stop_recording()?),
        _ => Err(ServiceError::Unsupported),
    }
}

/// Snapshot of a loaded mock audio track inside [`MemoryAudio`].
#[derive(Debug, Clone, PartialEq)]
pub struct MockAudioTrack {
    pub handle: AudioHandle,
    pub source: AudioSource,
    pub state: AudioPlaybackState,
    pub position_ms: u64,
    pub volume: f32,
    pub looping: bool,
}

struct MemoryAudioState {
    next_id: u64,
    tracks: BTreeMap<AudioHandle, MockAudioTrack>,
    active_recording: Option<AudioRecordConfig>,
    recording_clip_meta: Option<AudioClipMeta>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_AUDIO`].
pub struct MemoryAudio {
    state: Mutex<MemoryAudioState>,
}

impl Default for MemoryAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryAudio {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryAudioState {
                next_id: 1,
                tracks: BTreeMap::new(),
                active_recording: None,
                recording_clip_meta: None,
            }),
        }
    }

    pub fn track(&self, handle: AudioHandle) -> Option<MockAudioTrack> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tracks
            .get(&handle)
            .cloned()
    }

    pub fn set_recording_clip_meta(&self, meta: AudioClipMeta) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recording_clip_meta = Some(meta);
    }
}

impl AudioBackend for MemoryAudio {
    fn load_source(&self, source: &AudioSource) -> Result<AudioHandle, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let handle = AudioHandle(st.next_id);
        st.next_id = st.next_id.saturating_add(1);
        st.tracks.insert(
            handle,
            MockAudioTrack {
                handle,
                source: source.clone(),
                state: AudioPlaybackState::Idle,
                position_ms: 0,
                volume: 1.0,
                looping: false,
            },
        );
        Ok(handle)
    }

    fn unload(&self, handle: AudioHandle) -> Result<(), ServiceError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tracks
            .remove(&handle)
            .ok_or(ServiceError::NotFound)?;
        Ok(())
    }

    fn play(&self, handle: AudioHandle) -> Result<(), ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let track = st.tracks.get_mut(&handle).ok_or(ServiceError::NotFound)?;
        track.state = AudioPlaybackState::Playing;
        Ok(())
    }

    fn pause(&self, handle: AudioHandle) -> Result<(), ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let track = st.tracks.get_mut(&handle).ok_or(ServiceError::NotFound)?;
        track.state = AudioPlaybackState::Paused;
        Ok(())
    }

    fn stop(&self, handle: AudioHandle) -> Result<(), ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let track = st.tracks.get_mut(&handle).ok_or(ServiceError::NotFound)?;
        track.state = AudioPlaybackState::Stopped;
        track.position_ms = 0;
        Ok(())
    }

    fn seek_ms(&self, handle: AudioHandle, position_ms: u64) -> Result<(), ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let track = st.tracks.get_mut(&handle).ok_or(ServiceError::NotFound)?;
        track.position_ms = position_ms;
        Ok(())
    }

    fn set_volume(&self, handle: AudioHandle, volume: f32) -> Result<(), ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let track = st.tracks.get_mut(&handle).ok_or(ServiceError::NotFound)?;
        track.volume = volume;
        Ok(())
    }

    fn set_looping(&self, handle: AudioHandle, looping: bool) -> Result<(), ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let track = st.tracks.get_mut(&handle).ok_or(ServiceError::NotFound)?;
        track.looping = looping;
        Ok(())
    }

    fn start_recording(&self, config: &AudioRecordConfig) -> Result<(), ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.active_recording.is_some() {
            return Err(ServiceError::InvalidArg(
                "audio recording already active".into(),
            ));
        }
        st.active_recording = Some(config.clone());
        Ok(())
    }

    fn stop_recording(&self) -> Result<AudioClipMeta, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = st.active_recording.take().ok_or(ServiceError::NotFound)?;
        Ok(st.recording_clip_meta.clone().unwrap_or(AudioClipMeta {
            path: cfg
                .output_path
                .or_else(|| Some("/mem/tmp/recording.wav".into())),
            encoding: cfg.encoding,
            sample_rate_hz: cfg.sample_rate_hz,
            channels: cfg.channels,
            duration_ms: 1500,
            byte_len: u64::from(cfg.sample_rate_hz) * u64::from(cfg.channels) * 3,
        }))
    }
}

impl HostServiceHandler for MemoryAudio {
    fn service_id(&self) -> u32 {
        SERVICE_AUDIO
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_audio_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn AudioBackend>(self))
    }
}

/// Safe non-modal native [`HostServiceHandler`] for [`SERVICE_AUDIO`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeAudio;

impl NativeAudio {
    pub const fn new() -> Self {
        Self
    }
}

impl AudioBackend for NativeAudio {
    fn load_source(&self, _source: &AudioSource) -> Result<AudioHandle, ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn unload(&self, _handle: AudioHandle) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn play(&self, _handle: AudioHandle) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn pause(&self, _handle: AudioHandle) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn stop(&self, _handle: AudioHandle) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn seek_ms(&self, _handle: AudioHandle, _position_ms: u64) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn set_volume(&self, _handle: AudioHandle, _volume: f32) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn set_looping(&self, _handle: AudioHandle, _looping: bool) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn start_recording(&self, _config: &AudioRecordConfig) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn stop_recording(&self) -> Result<AudioClipMeta, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

impl HostServiceHandler for NativeAudio {
    fn service_id(&self) -> u32 {
        SERVICE_AUDIO
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_audio_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn AudioBackend>(self))
    }
}
