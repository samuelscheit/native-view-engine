#![cfg(feature = "host-service")]

use std::sync::Arc;

use nve_camera::host_service::{
    decode_capture_options, decode_captured_media, decode_devices_response, decode_preview_config,
};
use nve_camera::{
    CameraCaptureOptions, CameraClient, CameraFacing, CameraPreviewConfig, CapturedMedia,
    FlashMode, MediaFormat, MemoryCamera, NativeCamera, PermissionStatus,
};
use nve_host_services::{
    FfiHostServices, HostCallbackBridge, MemoryHostServices, NativeHostServices, ServiceError,
};

#[test]
fn camera_host_service_devices_capture_preview_permission_native_and_fuzz() {
    let cam_mem = Arc::new(MemoryCamera::new());
    let mem = MemoryHostServices::new().with_handler_arc(cam_mem.clone());
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let client = CameraClient::new(&ffi);

    let devices = client.list_devices().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].facing, CameraFacing::Back);
    assert_eq!(devices[1].facing, CameraFacing::Front);
    assert_eq!(
        client.request_permission().unwrap(),
        PermissionStatus::Authorized
    );

    let photo: CapturedMedia = client
        .capture_photo(&CameraCaptureOptions {
            device_id: Some("cam-back-0".into()),
            flash: FlashMode::On,
            format: MediaFormat::Png,
            quality: 95,
            max_width: Some(640),
            max_height: Some(480),
        })
        .unwrap();
    assert_eq!(photo.format, MediaFormat::Png);
    assert_eq!(photo.width, 640);
    assert_eq!(photo.height, 480);

    let preview = client
        .start_preview(&CameraPreviewConfig {
            device_id: Some("cam-front-1".into()),
            width: 1280,
            height: 720,
            target_fps: 60,
            mirror: true,
        })
        .unwrap();
    assert_eq!(cam_mem.active_previews(), vec![preview]);
    client.stop_preview(preview).unwrap();
    assert_eq!(client.stop_preview(preview), Err(ServiceError::NotFound));

    assert!(matches!(
        client.capture_photo(&CameraCaptureOptions {
            quality: 101,
            ..CameraCaptureOptions::default()
        }),
        Err(ServiceError::InvalidArg(_))
    ));
    cam_mem.set_permission(PermissionStatus::Denied);
    assert_eq!(
        client.capture_photo(&CameraCaptureOptions::default()),
        Err(ServiceError::PermissionDenied)
    );

    // Invalid enum byte rejection
    assert!(decode_captured_media(&[99, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());

    // Headless native safety
    let native = NativeHostServices::new().with_handler(NativeCamera::new());
    let native_client = CameraClient::new(&native);
    assert_eq!(native_client.list_devices().unwrap(), Vec::new());
    assert_eq!(
        native_client.request_permission(),
        Err(ServiceError::Unsupported)
    );

    let mut seed: u64 = 0xCA4E_4A00_5EED_0015;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_devices_response(&buf);
        let _ = decode_capture_options(&buf);
        let _ = decode_captured_media(&buf);
        let _ = decode_preview_config(&buf);
    }
}
