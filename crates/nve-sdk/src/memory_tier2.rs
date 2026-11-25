//! Deterministic in-memory mock state and binary dispatch for Tier-2/3 capabilities
//! inside [`crate::MemoryHostServices`].

use std::collections::{BTreeMap, BTreeSet};

use crate::audio::{
    self, AudioClipMeta, AudioHandle, AudioPlaybackState, AudioRecordConfig, AudioSource,
    SERVICE_AUDIO,
};
use crate::camera::{
    self, CameraDevice, CameraFacing, CameraPreviewConfig, CameraSessionId, CapturedMedia,
    MediaFormat, SERVICE_CAMERA,
};
use crate::error::SdkError;
use crate::fonts::{self, FontRegistry, SERVICE_FONTS};
use crate::native::storage::KvStoreManager;
use crate::native::tier2::dispatch_storage_kv;
use crate::provider::PermissionStatus;
use crate::sensors::{
    self, GeoFix, LocationWatchConfig, LocationWatchId, SensorKind, SensorSubscriptionId,
    SERVICE_LOCATION, SERVICE_SENSORS,
};
use crate::share_web::{
    self, InAppBrowserOptions, ShareResult, WebAuthResult, SERVICE_SHARE, SERVICE_WEB_AUTH,
};
use crate::storage::SERVICE_STORAGE;

/// Snapshot of a loaded mock audio track inside [`crate::MemoryHostServices`].
#[derive(Debug, Clone, PartialEq)]
pub struct MockAudioTrack {
    pub handle: AudioHandle,
    pub source: AudioSource,
    pub state: AudioPlaybackState,
    pub position_ms: u64,
    pub volume: f32,
    pub looping: bool,
}

/// Recorded share-sheet invocation for test assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedShare {
    Text {
        text: String,
        subject: Option<String>,
    },
    Url {
        url: String,
        title: Option<String>,
    },
    Files(Vec<String>),
}

/// Recorded in-app browser launch for test assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedBrowserLaunch {
    pub url: String,
    pub options: InAppBrowserOptions,
}

pub(crate) struct MemoryTier2State {
    pub kv: KvStoreManager,
    pub fonts: FontRegistry,
    pub mock_font_files: BTreeMap<String, Vec<u8>>,
    next_audio_id: u64,
    pub audio_tracks: BTreeMap<AudioHandle, MockAudioTrack>,
    pub active_recording: Option<AudioRecordConfig>,
    pub recording_clip_meta: Option<AudioClipMeta>,
    pub camera_permission: PermissionStatus,
    pub camera_devices: Vec<CameraDevice>,
    pub captured_media: CapturedMedia,
    next_camera_session_id: u64,
    pub camera_previews: BTreeMap<CameraSessionId, CameraPreviewConfig>,
    pub location_permission: PermissionStatus,
    pub current_fix: GeoFix,
    next_location_watch_id: u64,
    pub location_watches: BTreeMap<LocationWatchId, LocationWatchConfig>,
    pub available_sensors: BTreeSet<SensorKind>,
    next_sensor_sub_id: u64,
    pub sensor_subscriptions: BTreeMap<SensorSubscriptionId, (SensorKind, u16)>,
    pub share_result: ShareResult,
    pub recorded_shares: Vec<RecordedShare>,
    pub opened_browsers: Vec<RecordedBrowserLaunch>,
    pub web_auth_result: Option<WebAuthResult>,
    pub web_auth_requests: Vec<(String, String)>,
}

impl MemoryTier2State {
    pub fn new() -> Self {
        let mut available_sensors = BTreeSet::new();
        available_sensors.insert(SensorKind::Accelerometer);
        available_sensors.insert(SensorKind::Gyroscope);
        available_sensors.insert(SensorKind::Magnetometer);
        available_sensors.insert(SensorKind::Barometer);
        available_sensors.insert(SensorKind::AmbientLight);

        Self {
            kv: KvStoreManager::in_memory(),
            fonts: FontRegistry::new(),
            mock_font_files: BTreeMap::new(),
            next_audio_id: 1,
            audio_tracks: BTreeMap::new(),
            active_recording: None,
            recording_clip_meta: None,
            camera_permission: PermissionStatus::Authorized,
            camera_devices: vec![
                CameraDevice {
                    id: "cam-back-0".into(),
                    name: "Back Wide Camera".into(),
                    facing: CameraFacing::Back,
                    has_flash: true,
                    max_width: 4032,
                    max_height: 3024,
                },
                CameraDevice {
                    id: "cam-front-1".into(),
                    name: "Front FaceTime Camera".into(),
                    facing: CameraFacing::Front,
                    has_flash: false,
                    max_width: 1920,
                    max_height: 1080,
                },
            ],
            captured_media: CapturedMedia {
                format: MediaFormat::Jpeg,
                width: 1280,
                height: 720,
                bytes: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0xFF, 0xD9],
                file_path: Some("/mem/tmp/capture.jpg".into()),
                timestamp_ms: 1_700_000_000_000,
            },
            next_camera_session_id: 1,
            camera_previews: BTreeMap::new(),
            location_permission: PermissionStatus::Authorized,
            current_fix: GeoFix {
                latitude: 37.7749,
                longitude: -122.4194,
                altitude_m: Some(15.5),
                accuracy_m: 4.2,
                speed_mps: Some(0.0),
                heading_deg: Some(90.0),
                timestamp_ms: 1_700_000_000_000,
            },
            next_location_watch_id: 1,
            location_watches: BTreeMap::new(),
            available_sensors,
            next_sensor_sub_id: 1,
            sensor_subscriptions: BTreeMap::new(),
            share_result: ShareResult::Completed,
            recorded_shares: Vec::new(),
            opened_browsers: Vec::new(),
            web_auth_result: None,
            web_auth_requests: Vec::new(),
        }
    }

    pub fn dispatch(
        &mut self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<Vec<u8>, SdkError> {
        match service_id {
            SERVICE_STORAGE => dispatch_storage_kv(&mut self.kv, method_id, request),
            SERVICE_FONTS => self.dispatch_fonts(method_id, request),
            SERVICE_AUDIO => self.dispatch_audio(method_id, request),
            SERVICE_CAMERA => self.dispatch_camera(method_id, request),
            SERVICE_LOCATION => self.dispatch_location(method_id, request),
            SERVICE_SENSORS => self.dispatch_sensors(method_id, request),
            SERVICE_SHARE => self.dispatch_share(method_id, request),
            SERVICE_WEB_AUTH => self.dispatch_web_auth(method_id, request),
            _ => Err(SdkError::Unsupported),
        }
    }

    fn dispatch_fonts(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            fonts::METHOD_REGISTER_BYTES => {
                let (family, weight, italic, bytes) =
                    fonts::decode_register_bytes_request(request)?;
                let handle = self
                    .fonts
                    .register_bytes(family, weight, italic, &bytes, None)?;
                Ok(fonts::encode_handle(handle))
            }
            fonts::METHOD_REGISTER_FILE => {
                let (family, weight, italic, path) = fonts::decode_register_file_request(request)?;
                let bytes = self
                    .mock_font_files
                    .get(&path)
                    .ok_or(SdkError::NotFound)?
                    .clone();
                let handle = self
                    .fonts
                    .register_bytes(family, weight, italic, &bytes, Some(path))?;
                Ok(fonts::encode_handle(handle))
            }
            fonts::METHOD_UNREGISTER => {
                let handle = fonts::decode_handle(request)?;
                Ok(fonts::encode_unregister_response(
                    self.fonts.unregister(handle),
                ))
            }
            fonts::METHOD_LIST_REGISTERED => {
                fonts::encode_list_registered_response(&self.fonts.list())
            }
            _ => Err(SdkError::Unsupported),
        }
    }

    fn dispatch_audio(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            audio::METHOD_LOAD_SOURCE => {
                let source = audio::decode_load_source_request(request)?;
                let handle = AudioHandle(self.next_audio_id);
                self.next_audio_id = self.next_audio_id.saturating_add(1);
                self.audio_tracks.insert(
                    handle,
                    MockAudioTrack {
                        handle,
                        source,
                        state: AudioPlaybackState::Idle,
                        position_ms: 0,
                        volume: 1.0,
                        looping: false,
                    },
                );
                audio::encode_handle(handle)
            }
            audio::METHOD_UNLOAD => {
                let handle = audio::decode_handle(request)?;
                self.audio_tracks.remove(&handle).ok_or(SdkError::NotFound)?;
                Ok(Vec::new())
            }
            audio::METHOD_PLAY => {
                let handle = audio::decode_handle(request)?;
                let track = self.audio_tracks.get_mut(&handle).ok_or(SdkError::NotFound)?;
                track.state = AudioPlaybackState::Playing;
                Ok(Vec::new())
            }
            audio::METHOD_PAUSE => {
                let handle = audio::decode_handle(request)?;
                let track = self.audio_tracks.get_mut(&handle).ok_or(SdkError::NotFound)?;
                track.state = AudioPlaybackState::Paused;
                Ok(Vec::new())
            }
            audio::METHOD_STOP => {
                let handle = audio::decode_handle(request)?;
                let track = self.audio_tracks.get_mut(&handle).ok_or(SdkError::NotFound)?;
                track.state = AudioPlaybackState::Stopped;
                track.position_ms = 0;
                Ok(Vec::new())
            }
            audio::METHOD_SEEK_MS => {
                let (handle, pos) = audio::decode_seek_request(request)?;
                let track = self.audio_tracks.get_mut(&handle).ok_or(SdkError::NotFound)?;
                track.position_ms = pos;
                Ok(Vec::new())
            }
            audio::METHOD_SET_VOLUME => {
                let (handle, volume) = audio::decode_set_volume_request(request)?;
                let track = self.audio_tracks.get_mut(&handle).ok_or(SdkError::NotFound)?;
                track.volume = volume;
                Ok(Vec::new())
            }
            audio::METHOD_SET_LOOPING => {
                let (handle, looping) = audio::decode_set_looping_request(request)?;
                let track = self.audio_tracks.get_mut(&handle).ok_or(SdkError::NotFound)?;
                track.looping = looping;
                Ok(Vec::new())
            }
            audio::METHOD_START_RECORDING => {
                let cfg = audio::decode_record_config(request)?;
                if self.active_recording.is_some() {
                    return Err(SdkError::InvalidArg("audio recording already active".into()));
                }
                self.active_recording = Some(cfg);
                Ok(Vec::new())
            }
            audio::METHOD_STOP_RECORDING => {
                let cfg = self.active_recording.take().ok_or(SdkError::NotFound)?;
                let meta = self.recording_clip_meta.clone().unwrap_or(AudioClipMeta {
                    path: cfg.output_path.or_else(|| Some("/mem/tmp/recording.wav".into())),
                    encoding: cfg.encoding,
                    sample_rate_hz: cfg.sample_rate_hz,
                    channels: cfg.channels,
                    duration_ms: 1500,
                    byte_len: u64::from(cfg.sample_rate_hz) * u64::from(cfg.channels) * 3,
                });
                audio::encode_clip_meta(&meta)
            }
            _ => Err(SdkError::Unsupported),
        }
    }

    fn dispatch_camera(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            camera::METHOD_LIST_DEVICES => camera::encode_devices_response(&self.camera_devices),
            camera::METHOD_REQUEST_PERMISSION => Ok(self.camera_permission.encode()),
            camera::METHOD_CAPTURE_PHOTO => {
                let opts = camera::decode_capture_options(request)?;
                if matches!(
                    self.camera_permission,
                    PermissionStatus::Denied | PermissionStatus::Restricted
                ) {
                    return Err(SdkError::PermissionDenied);
                }
                if let Some(dev_id) = &opts.device_id
                    && !self.camera_devices.iter().any(|d| &d.id == dev_id)
                {
                    return Err(SdkError::NotFound);
                }
                let mut media = self.captured_media.clone();
                media.format = opts.format;
                if let Some(w) = opts.max_width {
                    media.width = media.width.min(w);
                }
                if let Some(h) = opts.max_height {
                    media.height = media.height.min(h);
                }
                camera::encode_captured_media(&media)
            }
            camera::METHOD_START_PREVIEW => {
                let cfg = camera::decode_preview_config(request)?;
                if matches!(
                    self.camera_permission,
                    PermissionStatus::Denied | PermissionStatus::Restricted
                ) {
                    return Err(SdkError::PermissionDenied);
                }
                if let Some(dev_id) = &cfg.device_id
                    && !self.camera_devices.iter().any(|d| &d.id == dev_id)
                {
                    return Err(SdkError::NotFound);
                }
                let session = CameraSessionId(self.next_camera_session_id);
                self.next_camera_session_id = self.next_camera_session_id.saturating_add(1);
                self.camera_previews.insert(session, cfg);
                camera::encode_session_id(session)
            }
            camera::METHOD_STOP_PREVIEW => {
                let session = camera::decode_session_id(request)?;
                self.camera_previews
                    .remove(&session)
                    .ok_or(SdkError::NotFound)?;
                Ok(Vec::new())
            }
            _ => Err(SdkError::Unsupported),
        }
    }

    fn dispatch_location(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            sensors::METHOD_LOC_REQUEST_PERMISSION => Ok(self.location_permission.encode()),
            sensors::METHOD_LOC_CURRENT_POSITION => {
                let _acc = sensors::decode_accuracy_request(request)?;
                if matches!(
                    self.location_permission,
                    PermissionStatus::Denied | PermissionStatus::Restricted
                ) {
                    return Err(SdkError::PermissionDenied);
                }
                sensors::encode_geo_fix(&self.current_fix)
            }
            sensors::METHOD_LOC_WATCH_POSITION => {
                let cfg = sensors::decode_watch_config(request)?;
                if matches!(
                    self.location_permission,
                    PermissionStatus::Denied | PermissionStatus::Restricted
                ) {
                    return Err(SdkError::PermissionDenied);
                }
                let id = LocationWatchId(self.next_location_watch_id);
                self.next_location_watch_id = self.next_location_watch_id.saturating_add(1);
                self.location_watches.insert(id, cfg);
                sensors::encode_watch_id(id)
            }
            sensors::METHOD_LOC_STOP_WATCH => {
                let id = sensors::decode_watch_id(request)?;
                self.location_watches
                    .remove(&id)
                    .ok_or(SdkError::NotFound)?;
                Ok(Vec::new())
            }
            _ => Err(SdkError::Unsupported),
        }
    }

    fn dispatch_sensors(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            sensors::METHOD_SENSOR_IS_AVAILABLE => {
                let kind = sensors::decode_sensor_kind(request)?;
                Ok(sensors::encode_is_available_response(
                    self.available_sensors.contains(&kind),
                ))
            }
            sensors::METHOD_SENSOR_SUBSCRIBE => {
                let (kind, rate_hz) = sensors::decode_sensor_subscribe_request(request)?;
                if !self.available_sensors.contains(&kind) {
                    return Err(SdkError::Unsupported);
                }
                let id = SensorSubscriptionId(self.next_sensor_sub_id);
                self.next_sensor_sub_id = self.next_sensor_sub_id.saturating_add(1);
                self.sensor_subscriptions.insert(id, (kind, rate_hz));
                sensors::encode_sensor_subscription_id(id)
            }
            sensors::METHOD_SENSOR_UNSUBSCRIBE => {
                let id = sensors::decode_sensor_subscription_id(request)?;
                self.sensor_subscriptions
                    .remove(&id)
                    .ok_or(SdkError::NotFound)?;
                Ok(Vec::new())
            }
            _ => Err(SdkError::Unsupported),
        }
    }

    fn dispatch_share(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            share_web::METHOD_SHARE_TEXT => {
                let (text, subject) = share_web::decode_share_text_request(request)?;
                self.recorded_shares
                    .push(RecordedShare::Text { text, subject });
                Ok(share_web::encode_share_result(self.share_result))
            }
            share_web::METHOD_SHARE_URL => {
                let (url, title) = share_web::decode_share_url_request(request)?;
                self.recorded_shares.push(RecordedShare::Url { url, title });
                Ok(share_web::encode_share_result(self.share_result))
            }
            share_web::METHOD_SHARE_FILES => {
                let paths = share_web::decode_share_files_request(request)?;
                self.recorded_shares.push(RecordedShare::Files(paths));
                Ok(share_web::encode_share_result(self.share_result))
            }
            _ => Err(SdkError::Unsupported),
        }
    }

    fn dispatch_web_auth(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            share_web::METHOD_OPEN_IN_APP_BROWSER => {
                let (url, options) = share_web::decode_open_browser_request(request)?;
                self.opened_browsers
                    .push(RecordedBrowserLaunch { url, options });
                Ok(Vec::new())
            }
            share_web::METHOD_AUTHENTICATE => {
                let (auth_url, callback_scheme) = share_web::decode_authenticate_request(request)?;
                self.web_auth_requests
                    .push((auth_url, callback_scheme.clone()));
                let result = self.web_auth_result.clone().unwrap_or_else(|| {
                    WebAuthResult::CallbackUri(format!(
                        "{callback_scheme}://callback?code=mock_auth_code"
                    ))
                });
                share_web::encode_web_auth_result(&result)
            }
            _ => Err(SdkError::Unsupported),
        }
    }
}

impl crate::MemoryHostServices {
    pub fn add_mock_font_file(&self, path: impl Into<String>, bytes: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .tier2
            .mock_font_files
            .insert(path.into(), bytes);
    }

    pub fn audio_track(&self, handle: AudioHandle) -> Option<MockAudioTrack> {
        self.state
            .lock()
            .unwrap()
            .tier2
            .audio_tracks
            .get(&handle)
            .cloned()
    }

    pub fn set_recording_clip_meta(&self, meta: AudioClipMeta) {
        self.state.lock().unwrap().tier2.recording_clip_meta = Some(meta);
    }

    pub fn set_camera_permission(&self, status: PermissionStatus) {
        self.state.lock().unwrap().tier2.camera_permission = status;
    }

    pub fn set_camera_devices(&self, devices: Vec<CameraDevice>) {
        self.state.lock().unwrap().tier2.camera_devices = devices;
    }

    pub fn set_captured_media(&self, media: CapturedMedia) {
        self.state.lock().unwrap().tier2.captured_media = media;
    }

    pub fn active_camera_previews(&self) -> Vec<CameraSessionId> {
        self.state
            .lock()
            .unwrap()
            .tier2
            .camera_previews
            .keys()
            .copied()
            .collect()
    }

    pub fn set_location_permission(&self, status: PermissionStatus) {
        self.state.lock().unwrap().tier2.location_permission = status;
    }

    pub fn set_current_fix(&self, fix: GeoFix) {
        self.state.lock().unwrap().tier2.current_fix = fix;
    }

    pub fn active_location_watches(&self) -> Vec<LocationWatchId> {
        self.state
            .lock()
            .unwrap()
            .tier2
            .location_watches
            .keys()
            .copied()
            .collect()
    }

    pub fn set_sensor_available(&self, kind: SensorKind, available: bool) {
        let mut st = self.state.lock().unwrap();
        if available {
            st.tier2.available_sensors.insert(kind);
        } else {
            st.tier2.available_sensors.remove(&kind);
        }
    }

    pub fn active_sensor_subscriptions(&self) -> Vec<(SensorSubscriptionId, SensorKind, u16)> {
        self.state
            .lock()
            .unwrap()
            .tier2
            .sensor_subscriptions
            .iter()
            .map(|(&id, &(k, r))| (id, k, r))
            .collect()
    }

    pub fn set_share_result(&self, result: ShareResult) {
        self.state.lock().unwrap().tier2.share_result = result;
    }

    pub fn recorded_shares(&self) -> Vec<RecordedShare> {
        self.state.lock().unwrap().tier2.recorded_shares.clone()
    }

    pub fn opened_browsers(&self) -> Vec<RecordedBrowserLaunch> {
        self.state.lock().unwrap().tier2.opened_browsers.clone()
    }

    pub fn set_web_auth_result(&self, result: Option<WebAuthResult>) {
        self.state.lock().unwrap().tier2.web_auth_result = result;
    }

    pub fn web_auth_requests(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().tier2.web_auth_requests.clone()
    }
}
