//! Deterministic in-memory mock state and binary dispatch for Tier-2/3 capabilities
//! inside [`crate::MemoryHostServices`].

#[cfg(any(
    feature = "fonts",
    feature = "audio",
    feature = "camera",
    feature = "location",
    feature = "sensors",
))]
use std::collections::BTreeMap;
#[cfg(feature = "sensors")]
use std::collections::BTreeSet;

#[cfg(feature = "audio")]
use crate::audio::{
    self, AudioClipMeta, AudioHandle, AudioPlaybackState, AudioRecordConfig, AudioSource,
    SERVICE_AUDIO,
};
#[cfg(feature = "camera")]
use crate::camera::{
    self, CameraDevice, CameraFacing, CameraPreviewConfig, CameraSessionId, CapturedMedia,
    MediaFormat, SERVICE_CAMERA,
};
use crate::error::SdkError;
#[cfg(feature = "fonts")]
use crate::fonts::{self, FontRegistry, SERVICE_FONTS};
#[cfg(feature = "location")]
use crate::location::{
    self, GeoFix, LocationWatchConfig, LocationWatchId, SERVICE_LOCATION,
};
#[cfg(feature = "storage")]
use crate::native::storage::KvStoreManager;
#[cfg(feature = "storage")]
use crate::native::tier2::dispatch_storage_kv;
#[cfg(any(feature = "camera", feature = "location"))]
use crate::provider::PermissionStatus;
#[cfg(feature = "sensors")]
use crate::sensors::{self, SensorKind, SensorSubscriptionId, SERVICE_SENSORS};
#[cfg(feature = "share")]
use crate::share_web::{self, ShareResult, SERVICE_SHARE};
#[cfg(feature = "web-auth")]
use crate::share_web::{self as web_auth_mod, InAppBrowserOptions, WebAuthResult, SERVICE_WEB_AUTH};
#[cfg(feature = "storage")]
use crate::storage::SERVICE_STORAGE;

/// Snapshot of a loaded mock audio track inside [`crate::MemoryHostServices`].
#[cfg(feature = "audio")]
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
#[cfg(feature = "share")]
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
#[cfg(feature = "web-auth")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedBrowserLaunch {
    pub url: String,
    pub options: InAppBrowserOptions,
}

pub(crate) struct MemoryTier2State {
    #[cfg(feature = "storage")]
    pub kv: KvStoreManager,
    #[cfg(feature = "fonts")]
    pub fonts: FontRegistry,
    #[cfg(feature = "fonts")]
    pub mock_font_files: BTreeMap<String, Vec<u8>>,
    #[cfg(feature = "audio")]
    next_audio_id: u64,
    #[cfg(feature = "audio")]
    pub audio_tracks: BTreeMap<AudioHandle, MockAudioTrack>,
    #[cfg(feature = "audio")]
    pub active_recording: Option<AudioRecordConfig>,
    #[cfg(feature = "audio")]
    pub recording_clip_meta: Option<AudioClipMeta>,
    #[cfg(feature = "camera")]
    pub camera_permission: PermissionStatus,
    #[cfg(feature = "camera")]
    pub camera_devices: Vec<CameraDevice>,
    #[cfg(feature = "camera")]
    pub captured_media: CapturedMedia,
    #[cfg(feature = "camera")]
    next_camera_session_id: u64,
    #[cfg(feature = "camera")]
    pub camera_previews: BTreeMap<CameraSessionId, CameraPreviewConfig>,
    #[cfg(feature = "location")]
    pub location_permission: PermissionStatus,
    #[cfg(feature = "location")]
    pub current_fix: GeoFix,
    #[cfg(feature = "location")]
    next_location_watch_id: u64,
    #[cfg(feature = "location")]
    pub location_watches: BTreeMap<LocationWatchId, LocationWatchConfig>,
    #[cfg(feature = "sensors")]
    pub available_sensors: BTreeSet<SensorKind>,
    #[cfg(feature = "sensors")]
    next_sensor_sub_id: u64,
    #[cfg(feature = "sensors")]
    pub sensor_subscriptions: BTreeMap<SensorSubscriptionId, (SensorKind, u16)>,
    #[cfg(feature = "share")]
    pub share_result: ShareResult,
    #[cfg(feature = "share")]
    pub recorded_shares: Vec<RecordedShare>,
    #[cfg(feature = "web-auth")]
    pub opened_browsers: Vec<RecordedBrowserLaunch>,
    #[cfg(feature = "web-auth")]
    pub web_auth_result: Option<WebAuthResult>,
    #[cfg(feature = "web-auth")]
    pub web_auth_requests: Vec<(String, String)>,
}

impl MemoryTier2State {
    pub fn new() -> Self {
        #[cfg(feature = "sensors")]
        let available_sensors = {
            let mut s = BTreeSet::new();
            s.insert(SensorKind::Accelerometer);
            s.insert(SensorKind::Gyroscope);
            s.insert(SensorKind::Magnetometer);
            s.insert(SensorKind::Barometer);
            s.insert(SensorKind::AmbientLight);
            s
        };

        Self {
            #[cfg(feature = "storage")]
            kv: KvStoreManager::in_memory(),
            #[cfg(feature = "fonts")]
            fonts: FontRegistry::new(),
            #[cfg(feature = "fonts")]
            mock_font_files: BTreeMap::new(),
            #[cfg(feature = "audio")]
            next_audio_id: 1,
            #[cfg(feature = "audio")]
            audio_tracks: BTreeMap::new(),
            #[cfg(feature = "audio")]
            active_recording: None,
            #[cfg(feature = "audio")]
            recording_clip_meta: None,
            #[cfg(feature = "camera")]
            camera_permission: PermissionStatus::Authorized,
            #[cfg(feature = "camera")]
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
            #[cfg(feature = "camera")]
            captured_media: CapturedMedia {
                format: MediaFormat::Jpeg,
                width: 1280,
                height: 720,
                bytes: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0xFF, 0xD9],
                file_path: Some("/mem/tmp/capture.jpg".into()),
                timestamp_ms: 1_700_000_000_000,
            },
            #[cfg(feature = "camera")]
            next_camera_session_id: 1,
            #[cfg(feature = "camera")]
            camera_previews: BTreeMap::new(),
            #[cfg(feature = "location")]
            location_permission: PermissionStatus::Authorized,
            #[cfg(feature = "location")]
            current_fix: GeoFix {
                latitude: 37.7749,
                longitude: -122.4194,
                altitude_m: Some(15.5),
                accuracy_m: 4.2,
                speed_mps: Some(0.0),
                heading_deg: Some(90.0),
                timestamp_ms: 1_700_000_000_000,
            },
            #[cfg(feature = "location")]
            next_location_watch_id: 1,
            #[cfg(feature = "location")]
            location_watches: BTreeMap::new(),
            #[cfg(feature = "sensors")]
            available_sensors,
            #[cfg(feature = "sensors")]
            next_sensor_sub_id: 1,
            #[cfg(feature = "sensors")]
            sensor_subscriptions: BTreeMap::new(),
            #[cfg(feature = "share")]
            share_result: ShareResult::Completed,
            #[cfg(feature = "share")]
            recorded_shares: Vec::new(),
            #[cfg(feature = "web-auth")]
            opened_browsers: Vec::new(),
            #[cfg(feature = "web-auth")]
            web_auth_result: None,
            #[cfg(feature = "web-auth")]
            web_auth_requests: Vec::new(),
        }
    }

    pub const fn handles_service(service_id: u32) -> bool {
        match service_id {
            #[cfg(feature = "storage")]
            SERVICE_STORAGE => true,
            #[cfg(feature = "fonts")]
            SERVICE_FONTS => true,
            #[cfg(feature = "audio")]
            SERVICE_AUDIO => true,
            #[cfg(feature = "camera")]
            SERVICE_CAMERA => true,
            #[cfg(feature = "location")]
            SERVICE_LOCATION => true,
            #[cfg(feature = "sensors")]
            SERVICE_SENSORS => true,
            #[cfg(feature = "share")]
            SERVICE_SHARE => true,
            #[cfg(feature = "web-auth")]
            SERVICE_WEB_AUTH => true,
            _ => false,
        }
    }

    pub fn dispatch(
        &mut self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<Vec<u8>, SdkError> {
        match service_id {
            #[cfg(feature = "storage")]
            SERVICE_STORAGE => dispatch_storage_kv(&mut self.kv, method_id, request),
            #[cfg(feature = "fonts")]
            SERVICE_FONTS => self.dispatch_fonts(method_id, request),
            #[cfg(feature = "audio")]
            SERVICE_AUDIO => self.dispatch_audio(method_id, request),
            #[cfg(feature = "camera")]
            SERVICE_CAMERA => self.dispatch_camera(method_id, request),
            #[cfg(feature = "location")]
            SERVICE_LOCATION => self.dispatch_location(method_id, request),
            #[cfg(feature = "sensors")]
            SERVICE_SENSORS => self.dispatch_sensors(method_id, request),
            #[cfg(feature = "share")]
            SERVICE_SHARE => self.dispatch_share(method_id, request),
            #[cfg(feature = "web-auth")]
            SERVICE_WEB_AUTH => self.dispatch_web_auth(method_id, request),
            _ => Err(SdkError::Unsupported),
        }
    }

    #[cfg(feature = "fonts")]
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

    #[cfg(feature = "audio")]
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

    #[cfg(feature = "camera")]
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

    #[cfg(feature = "location")]
    fn dispatch_location(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            location::METHOD_LOC_REQUEST_PERMISSION => Ok(self.location_permission.encode()),
            location::METHOD_LOC_CURRENT_POSITION => {
                let _acc = location::decode_accuracy_request(request)?;
                if matches!(
                    self.location_permission,
                    PermissionStatus::Denied | PermissionStatus::Restricted
                ) {
                    return Err(SdkError::PermissionDenied);
                }
                location::encode_geo_fix(&self.current_fix)
            }
            location::METHOD_LOC_WATCH_POSITION => {
                let cfg = location::decode_watch_config(request)?;
                if matches!(
                    self.location_permission,
                    PermissionStatus::Denied | PermissionStatus::Restricted
                ) {
                    return Err(SdkError::PermissionDenied);
                }
                let id = LocationWatchId(self.next_location_watch_id);
                self.next_location_watch_id = self.next_location_watch_id.saturating_add(1);
                self.location_watches.insert(id, cfg);
                location::encode_watch_id(id)
            }
            location::METHOD_LOC_STOP_WATCH => {
                let id = location::decode_watch_id(request)?;
                self.location_watches
                    .remove(&id)
                    .ok_or(SdkError::NotFound)?;
                Ok(Vec::new())
            }
            _ => Err(SdkError::Unsupported),
        }
    }

    #[cfg(feature = "sensors")]
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

    #[cfg(feature = "share")]
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

    #[cfg(feature = "web-auth")]
    fn dispatch_web_auth(&mut self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
        match method_id {
            web_auth_mod::METHOD_OPEN_IN_APP_BROWSER => {
                let (url, options) = web_auth_mod::decode_open_browser_request(request)?;
                self.opened_browsers
                    .push(RecordedBrowserLaunch { url, options });
                Ok(Vec::new())
            }
            web_auth_mod::METHOD_AUTHENTICATE => {
                let (auth_url, callback_scheme) = web_auth_mod::decode_authenticate_request(request)?;
                self.web_auth_requests
                    .push((auth_url, callback_scheme.clone()));
                let result = self.web_auth_result.clone().unwrap_or_else(|| {
                    WebAuthResult::CallbackUri(format!(
                        "{callback_scheme}://callback?code=mock_auth_code"
                    ))
                });
                web_auth_mod::encode_web_auth_result(&result)
            }
            _ => Err(SdkError::Unsupported),
        }
    }
}

impl crate::MemoryHostServices {
    #[cfg(feature = "fonts")]
    pub fn add_mock_font_file(&self, path: impl Into<String>, bytes: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .tier2
            .mock_font_files
            .insert(path.into(), bytes);
    }

    #[cfg(feature = "audio")]
    pub fn audio_track(&self, handle: AudioHandle) -> Option<MockAudioTrack> {
        self.state
            .lock()
            .unwrap()
            .tier2
            .audio_tracks
            .get(&handle)
            .cloned()
    }

    #[cfg(feature = "audio")]
    pub fn set_recording_clip_meta(&self, meta: AudioClipMeta) {
        self.state.lock().unwrap().tier2.recording_clip_meta = Some(meta);
    }

    #[cfg(feature = "camera")]
    pub fn set_camera_permission(&self, status: PermissionStatus) {
        self.state.lock().unwrap().tier2.camera_permission = status;
    }

    #[cfg(feature = "camera")]
    pub fn set_camera_devices(&self, devices: Vec<CameraDevice>) {
        self.state.lock().unwrap().tier2.camera_devices = devices;
    }

    #[cfg(feature = "camera")]
    pub fn set_captured_media(&self, media: CapturedMedia) {
        self.state.lock().unwrap().tier2.captured_media = media;
    }

    #[cfg(feature = "camera")]
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

    #[cfg(feature = "location")]
    pub fn set_location_permission(&self, status: PermissionStatus) {
        self.state.lock().unwrap().tier2.location_permission = status;
    }

    #[cfg(feature = "location")]
    pub fn set_current_fix(&self, fix: GeoFix) {
        self.state.lock().unwrap().tier2.current_fix = fix;
    }

    #[cfg(feature = "location")]
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

    #[cfg(feature = "sensors")]
    pub fn set_sensor_available(&self, kind: SensorKind, available: bool) {
        let mut st = self.state.lock().unwrap();
        if available {
            st.tier2.available_sensors.insert(kind);
        } else {
            st.tier2.available_sensors.remove(&kind);
        }
    }

    #[cfg(feature = "sensors")]
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

    #[cfg(feature = "share")]
    pub fn set_share_result(&self, result: ShareResult) {
        self.state.lock().unwrap().tier2.share_result = result;
    }

    #[cfg(feature = "share")]
    pub fn recorded_shares(&self) -> Vec<RecordedShare> {
        self.state.lock().unwrap().tier2.recorded_shares.clone()
    }

    #[cfg(feature = "web-auth")]
    pub fn opened_browsers(&self) -> Vec<RecordedBrowserLaunch> {
        self.state.lock().unwrap().tier2.opened_browsers.clone()
    }

    #[cfg(feature = "web-auth")]
    pub fn set_web_auth_result(&self, result: Option<WebAuthResult>) {
        self.state.lock().unwrap().tier2.web_auth_result = result;
    }

    #[cfg(feature = "web-auth")]
    pub fn web_auth_requests(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().tier2.web_auth_requests.clone()
    }
}
