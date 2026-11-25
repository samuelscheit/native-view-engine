//! Tier-2/3 native host backend dispatch for `NativeHostServices`.
//!
//! - `SERVICE_STORAGE`: zero-dep crash-safe `NVK1` persistence engine (`native::storage`).
//! - `SERVICE_FONTS`: in-process font registry with binary magic validation & deduplication;
//!   disk file loading is gated behind `#[cfg(not(target_arch = "wasm32"))]`.
//! - Hardware/UI services (`AUDIO`, `CAMERA`, `LOCATION`, `SENSORS`, `SHARE`, `WEB_AUTH`):
//!   strictly validate every binary request and return safe non-modal defaults or
//!   `SdkError::Unsupported` on headless/unbundled native hosts without spawning OS modals.

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use crate::audio::{self, SERVICE_AUDIO};
use crate::camera::{self, SERVICE_CAMERA};
use crate::error::SdkError;
use crate::fonts::{self, FontRegistry, SERVICE_FONTS};
use crate::native::storage::KvStoreManager;
use crate::sensors::{self, SERVICE_LOCATION, SERVICE_SENSORS};
use crate::share_web::{self, SERVICE_SHARE, SERVICE_WEB_AUTH};
use crate::storage::{self, SERVICE_STORAGE};

pub struct NativeTier2State {
    pub kv: KvStoreManager,
    pub fonts: FontRegistry,
}

impl NativeTier2State {
    pub fn in_memory() -> Self {
        Self {
            kv: KvStoreManager::in_memory(),
            fonts: FontRegistry::new(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_storage_root(root: PathBuf) -> Self {
        Self {
            kv: KvStoreManager::with_root_dir(root),
            fonts: FontRegistry::new(),
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
            SERVICE_AUDIO => dispatch_audio_headless(method_id, request),
            SERVICE_CAMERA => dispatch_camera_headless(method_id, request),
            SERVICE_LOCATION => dispatch_location_headless(method_id, request),
            SERVICE_SENSORS => dispatch_sensors_headless(method_id, request),
            SERVICE_SHARE => dispatch_share_headless(method_id, request),
            SERVICE_WEB_AUTH => dispatch_web_auth_headless(method_id, request),
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
                self.register_font_file(family, weight, italic, path)
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

    #[cfg(not(target_arch = "wasm32"))]
    fn register_font_file(
        &mut self,
        family: String,
        weight: u16,
        italic: bool,
        path: String,
    ) -> Result<Vec<u8>, SdkError> {
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SdkError::NotFound
            } else {
                SdkError::IoError(e.to_string())
            }
        })?;
        let handle = self
            .fonts
            .register_bytes(family, weight, italic, &bytes, Some(path))?;
        Ok(fonts::encode_handle(handle))
    }

    #[cfg(target_arch = "wasm32")]
    fn register_font_file(
        &mut self,
        _family: String,
        _weight: u16,
        _italic: bool,
        _path: String,
    ) -> Result<Vec<u8>, SdkError> {
        Err(SdkError::Unsupported)
    }
}

pub(crate) fn dispatch_storage_kv(
    kv: &mut KvStoreManager,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, SdkError> {
    match method_id {
        storage::METHOD_OPEN => {
            let ns = storage::decode_open_request(request)?;
            let info = kv.open(&ns)?;
            Ok(storage::encode_open_response(info))
        }
        storage::METHOD_GET => {
            let (ns, key) = storage::decode_key_request(request)?;
            let val = kv.get_or_open(&ns)?.get(&key)?;
            storage::encode_get_response(val.as_deref())
        }
        storage::METHOD_PUT => {
            let (ns, key, value) = storage::decode_put_request(request)?;
            kv.get_or_open(&ns)?.put(&key, &value)?;
            Ok(Vec::new())
        }
        storage::METHOD_DELETE => {
            let (ns, key) = storage::decode_key_request(request)?;
            let deleted = kv.get_or_open(&ns)?.delete(&key)?;
            Ok(storage::encode_delete_response(deleted))
        }
        storage::METHOD_SCAN_PREFIX => {
            let (ns, prefix, limit) = storage::decode_scan_prefix_request(request)?;
            let entries = kv.get_or_open(&ns)?.scan_prefix(&prefix, limit);
            storage::encode_scan_prefix_response(&entries)
        }
        storage::METHOD_COMMIT_BATCH => {
            let (ns, ops) = storage::decode_commit_batch_request(request)?;
            kv.get_or_open(&ns)?.commit_batch(&ops)?;
            Ok(Vec::new())
        }
        storage::METHOD_COMPACT => {
            let ns = storage::decode_open_request(request)?;
            let stats = kv.get_or_open(&ns)?.compact()?;
            Ok(storage::encode_compact_response(stats))
        }
        _ => Err(SdkError::Unsupported),
    }
}

fn dispatch_audio_headless(method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
    match method_id {
        audio::METHOD_LOAD_SOURCE => {
            let _ = audio::decode_load_source_request(request)?;
            Err(SdkError::Unsupported)
        }
        audio::METHOD_UNLOAD
        | audio::METHOD_PLAY
        | audio::METHOD_PAUSE
        | audio::METHOD_STOP => {
            let _ = audio::decode_handle(request)?;
            Err(SdkError::Unsupported)
        }
        audio::METHOD_SEEK_MS => {
            let _ = audio::decode_seek_request(request)?;
            Err(SdkError::Unsupported)
        }
        audio::METHOD_SET_VOLUME => {
            let _ = audio::decode_set_volume_request(request)?;
            Err(SdkError::Unsupported)
        }
        audio::METHOD_SET_LOOPING => {
            let _ = audio::decode_set_looping_request(request)?;
            Err(SdkError::Unsupported)
        }
        audio::METHOD_START_RECORDING => {
            let _ = audio::decode_record_config(request)?;
            Err(SdkError::Unsupported)
        }
        audio::METHOD_STOP_RECORDING => Err(SdkError::Unsupported),
        _ => Err(SdkError::Unsupported),
    }
}

fn dispatch_camera_headless(method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
    match method_id {
        camera::METHOD_LIST_DEVICES => camera::encode_devices_response(&[]),
        camera::METHOD_REQUEST_PERMISSION => Err(SdkError::Unsupported),
        camera::METHOD_CAPTURE_PHOTO => {
            let _ = camera::decode_capture_options(request)?;
            Err(SdkError::Unsupported)
        }
        camera::METHOD_START_PREVIEW => {
            let _ = camera::decode_preview_config(request)?;
            Err(SdkError::Unsupported)
        }
        camera::METHOD_STOP_PREVIEW => {
            let _ = camera::decode_session_id(request)?;
            Err(SdkError::Unsupported)
        }
        _ => Err(SdkError::Unsupported),
    }
}

fn dispatch_location_headless(method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
    match method_id {
        sensors::METHOD_LOC_REQUEST_PERMISSION => Err(SdkError::Unsupported),
        sensors::METHOD_LOC_CURRENT_POSITION => {
            let _ = sensors::decode_accuracy_request(request)?;
            Err(SdkError::Unsupported)
        }
        sensors::METHOD_LOC_WATCH_POSITION => {
            let _ = sensors::decode_watch_config(request)?;
            Err(SdkError::Unsupported)
        }
        sensors::METHOD_LOC_STOP_WATCH => {
            let _ = sensors::decode_watch_id(request)?;
            Err(SdkError::Unsupported)
        }
        _ => Err(SdkError::Unsupported),
    }
}

fn dispatch_sensors_headless(method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
    match method_id {
        sensors::METHOD_SENSOR_IS_AVAILABLE => {
            let _ = sensors::decode_sensor_kind(request)?;
            Ok(sensors::encode_is_available_response(false))
        }
        sensors::METHOD_SENSOR_SUBSCRIBE => {
            let _ = sensors::decode_sensor_subscribe_request(request)?;
            Err(SdkError::Unsupported)
        }
        sensors::METHOD_SENSOR_UNSUBSCRIBE => {
            let _ = sensors::decode_sensor_subscription_id(request)?;
            Err(SdkError::Unsupported)
        }
        _ => Err(SdkError::Unsupported),
    }
}

fn dispatch_share_headless(method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
    match method_id {
        share_web::METHOD_SHARE_TEXT => {
            let _ = share_web::decode_share_text_request(request)?;
            Err(SdkError::Unsupported)
        }
        share_web::METHOD_SHARE_URL => {
            let _ = share_web::decode_share_url_request(request)?;
            Err(SdkError::Unsupported)
        }
        share_web::METHOD_SHARE_FILES => {
            let _ = share_web::decode_share_files_request(request)?;
            Err(SdkError::Unsupported)
        }
        _ => Err(SdkError::Unsupported),
    }
}

fn dispatch_web_auth_headless(method_id: u32, request: &[u8]) -> Result<Vec<u8>, SdkError> {
    match method_id {
        share_web::METHOD_OPEN_IN_APP_BROWSER => {
            let _ = share_web::decode_open_browser_request(request)?;
            Err(SdkError::Unsupported)
        }
        share_web::METHOD_AUTHENTICATE => {
            let _ = share_web::decode_authenticate_request(request)?;
            Err(SdkError::Unsupported)
        }
        _ => Err(SdkError::Unsupported),
    }
}
