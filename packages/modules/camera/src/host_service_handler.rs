//! Direct typed [`CameraBackend`] plus [`MemoryCamera`] and headless-safe [`NativeCamera`].

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nve_host_services::{pack_typed, HostServiceHandler, PermissionStatus, ServiceError};

use crate::host_service::{
    decode_capture_options, decode_preview_config, decode_session_id, encode_captured_media,
    encode_devices_response, encode_session_id, CameraCaptureOptions, CameraDevice,
    CameraPreviewConfig, CameraSessionId, CapturedMedia, MediaFormat, METHOD_CAPTURE_PHOTO,
    METHOD_LIST_DEVICES, METHOD_REQUEST_PERMISSION, METHOD_START_PREVIEW, METHOD_STOP_PREVIEW,
    SERVICE_CAMERA,
};
use crate::types::CameraFacing;

/// Direct typed in-process backend for [`SERVICE_CAMERA`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait CameraBackend: Send + Sync {
    fn list_devices(&self) -> Result<Vec<CameraDevice>, ServiceError>;
    fn request_permission(&self) -> Result<PermissionStatus, ServiceError>;
    fn capture_photo(&self, options: &CameraCaptureOptions) -> Result<CapturedMedia, ServiceError>;
    fn start_preview(&self, config: &CameraPreviewConfig) -> Result<CameraSessionId, ServiceError>;
    fn stop_preview(&self, session: CameraSessionId) -> Result<(), ServiceError>;
}

/// Bridges a [`CameraBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_camera_wire<B: CameraBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_LIST_DEVICES => encode_devices_response(&backend.list_devices()?),
        METHOD_REQUEST_PERMISSION => Ok(backend.request_permission()?.encode()),
        METHOD_CAPTURE_PHOTO => {
            let opts = decode_capture_options(request)?;
            encode_captured_media(&backend.capture_photo(&opts)?)
        }
        METHOD_START_PREVIEW => {
            let cfg = decode_preview_config(request)?;
            encode_session_id(backend.start_preview(&cfg)?)
        }
        METHOD_STOP_PREVIEW => {
            let session = decode_session_id(request)?;
            backend.stop_preview(session)?;
            Ok(Vec::new())
        }
        _ => Err(ServiceError::Unsupported),
    }
}

struct MemoryCameraState {
    permission: PermissionStatus,
    devices: Vec<CameraDevice>,
    captured_media: CapturedMedia,
    next_session_id: u64,
    previews: BTreeMap<CameraSessionId, CameraPreviewConfig>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_CAMERA`].
pub struct MemoryCamera {
    state: Mutex<MemoryCameraState>,
}

impl Default for MemoryCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryCamera {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryCameraState {
                permission: PermissionStatus::Authorized,
                devices: vec![
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
                next_session_id: 1,
                previews: BTreeMap::new(),
            }),
        }
    }

    pub fn set_permission(&self, status: PermissionStatus) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .permission = status;
    }

    pub fn set_devices(&self, devices: Vec<CameraDevice>) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).devices = devices;
    }

    pub fn set_captured_media(&self, media: CapturedMedia) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .captured_media = media;
    }

    pub fn active_previews(&self) -> Vec<CameraSessionId> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .previews
            .keys()
            .copied()
            .collect()
    }
}

impl CameraBackend for MemoryCamera {
    fn list_devices(&self) -> Result<Vec<CameraDevice>, ServiceError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .devices
            .clone())
    }

    fn request_permission(&self) -> Result<PermissionStatus, ServiceError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .permission)
    }

    fn capture_photo(&self, options: &CameraCaptureOptions) -> Result<CapturedMedia, ServiceError> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(
            st.permission,
            PermissionStatus::Denied | PermissionStatus::Restricted
        ) {
            return Err(ServiceError::PermissionDenied);
        }
        if let Some(dev_id) = &options.device_id
            && !st.devices.iter().any(|d| &d.id == dev_id)
        {
            return Err(ServiceError::NotFound);
        }
        let mut media = st.captured_media.clone();
        media.format = options.format;
        if let Some(w) = options.max_width {
            media.width = media.width.min(w);
        }
        if let Some(h) = options.max_height {
            media.height = media.height.min(h);
        }
        Ok(media)
    }

    fn start_preview(&self, config: &CameraPreviewConfig) -> Result<CameraSessionId, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(
            st.permission,
            PermissionStatus::Denied | PermissionStatus::Restricted
        ) {
            return Err(ServiceError::PermissionDenied);
        }
        if let Some(dev_id) = &config.device_id
            && !st.devices.iter().any(|d| &d.id == dev_id)
        {
            return Err(ServiceError::NotFound);
        }
        let session = CameraSessionId(st.next_session_id);
        st.next_session_id = st.next_session_id.saturating_add(1);
        st.previews.insert(session, config.clone());
        Ok(session)
    }

    fn stop_preview(&self, session: CameraSessionId) -> Result<(), ServiceError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .previews
            .remove(&session)
            .ok_or(ServiceError::NotFound)?;
        Ok(())
    }
}

impl HostServiceHandler for MemoryCamera {
    fn service_id(&self) -> u32 {
        SERVICE_CAMERA
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_camera_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn CameraBackend>(self))
    }
}

/// Safe non-modal native [`HostServiceHandler`] for [`SERVICE_CAMERA`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeCamera;

impl NativeCamera {
    pub const fn new() -> Self {
        Self
    }
}

impl CameraBackend for NativeCamera {
    fn list_devices(&self) -> Result<Vec<CameraDevice>, ServiceError> {
        Ok(Vec::new())
    }

    fn request_permission(&self) -> Result<PermissionStatus, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn capture_photo(
        &self,
        _options: &CameraCaptureOptions,
    ) -> Result<CapturedMedia, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn start_preview(
        &self,
        _config: &CameraPreviewConfig,
    ) -> Result<CameraSessionId, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn stop_preview(&self, _session: CameraSessionId) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

impl HostServiceHandler for NativeCamera {
    fn service_id(&self) -> u32 {
        SERVICE_CAMERA
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_camera_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn CameraBackend>(self))
    }
}
