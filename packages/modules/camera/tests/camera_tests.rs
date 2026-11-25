#![cfg(feature = "spec")]

use nve_camera::{
    CAMERA_FRAME_DESCRIPTOR_SIZE, CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION, CAMERA_PREVIEW_KIND,
    CameraCodecError, CameraFacing, CameraFrameDescriptor, CameraPixelFormat, CameraPreview,
    CameraPreviewCommand, CameraPreviewProps, MAX_CAMERA_FPS,
    NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC, StabilizationMode, TorchMode,
};

#[test]
fn golden_byte_vectors_match_cpp_apple_and_kotlin_codecs() {
    assert_eq!(CameraPreview::KIND, "nve.camera.preview");
    assert_eq!(CAMERA_PREVIEW_KIND, "nve.camera.preview");

    // Golden NVNP bytes for default CameraPreviewProps
    let expected_default_props: &[u8] = &[
        b'N', b'V', b'N', b'P', 1, 10, 0,
        1, 0, 0,
        2, 0, 2, 1, 0, 0, 0,
        3, 0, 2, 0, 0, 0, 0,
        4, 0, 5, 0x00, 0x00, 0x80, 0x3F,
        5, 0, 2, 30, 0, 0, 0,
        6, 0, 1, 0,
        7, 0, 2, 0, 0, 0, 0,
        8, 0, 1, 0,
        9, 0, 1, 0,
        10, 0, 1, 1,
    ];
    let encoded_props = CameraPreviewProps::default().to_prop_bytes().unwrap();
    assert_eq!(encoded_props, expected_default_props);
    assert_eq!(
        CameraPreviewProps::from_prop_bytes(expected_default_props).unwrap(),
        CameraPreviewProps::default()
    );

    // Golden NVNC bytes for FocusPoint { x: 0.25, y: 0.75 }
    let expected_focus: &[u8] = &[
        b'N', b'V', b'N', b'C', 1, 4, 0, 2,
        5, 0x00, 0x00, 0x80, 0x3E,
        5, 0x00, 0x00, 0x40, 0x3F,
    ];
    let encoded_focus = CameraPreviewCommand::FocusPoint { x: 0.25, y: 0.75 }
        .encode_command()
        .unwrap();
    assert_eq!(encoded_focus, expected_focus);
    assert_eq!(
        CameraPreviewCommand::from_command_bytes(expected_focus).unwrap(),
        CameraPreviewCommand::FocusPoint { x: 0.25, y: 0.75 }
    );

    // Golden NVCF 40-byte frame descriptor
    let expected_nvcf: [u8; 40] = [
        b'N', b'V', b'C', b'F', 1, 3, 0x0E, 0x01,
        0x80, 0x07, 0x00, 0x00,
        0x38, 0x04, 0x00, 0x00,
        0x80, 0x07, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
        0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
    ];
    let desc = CameraFrameDescriptor {
        width: 1920,
        height: 1080,
        stride_bytes: 1920,
        pixel_format: CameraPixelFormat::Yuv420_888,
        rotation_deg: 270,
        timestamp_ns: 0x0102_0304_0506_0708,
        native_handle_id: 0x1122_3344_5566_7788,
    };
    assert_eq!(desc.encode().unwrap(), expected_nvcf);
    assert_eq!(CameraFrameDescriptor::decode(&expected_nvcf).unwrap(), desc);
}

#[test]
fn props_commands_and_frames_roundtrip_and_reject_invalid_values() {
    let custom = CameraPreviewProps::new()
        .with_device_id("back-ultra-wide")
        .with_facing(CameraFacing::External)
        .with_torch(TorchMode::Auto)
        .with_zoom(0.5)
        .with_fps(MAX_CAMERA_FPS)
        .with_hdr(true)
        .with_stabilization(StabilizationMode::Cinematic)
        .with_frame_processor(true)
        .with_audio(true)
        .with_active(false);
    let raw_props = custom.to_prop_bytes().unwrap();
    assert_eq!(CameraPreviewProps::from_prop_bytes(&raw_props).unwrap(), custom);

    for bad_zoom in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -2.0] {
        assert!(CameraPreviewProps::default().with_zoom(bad_zoom).to_prop_bytes().is_err());
        assert!(CameraPreviewCommand::SetZoom(bad_zoom).encode_command().is_err());
    }
    for bad_fps in [0u16, 241, u16::MAX] {
        assert_eq!(
            CameraPreviewProps::default().with_fps(bad_fps).to_prop_bytes().unwrap_err(),
            CameraCodecError::InvalidEnum("CameraFps", u32::from(bad_fps))
        );
    }

    let commands = [
        CameraPreviewCommand::SwitchFacing(CameraFacing::Front),
        CameraPreviewCommand::SetTorch(TorchMode::On),
        CameraPreviewCommand::SetZoom(3.5),
        CameraPreviewCommand::FocusPoint { x: 0.0, y: 1.0 },
        CameraPreviewCommand::CapturePhoto { quality: 100, flash: 2 },
        CameraPreviewCommand::StartRecording { output_path: "/tmp/clip.mp4".into() },
        CameraPreviewCommand::StopRecording,
    ];
    for cmd in &commands {
        let encoded = cmd.encode_command().unwrap();
        assert!(encoded.starts_with(&NATIVE_CMD_MAGIC));
        assert_eq!(CameraPreviewCommand::from_command_bytes(&encoded).unwrap(), *cmd);
        assert!(!cmd.command_name().is_empty());
    }

    assert_eq!(
        CameraPreviewCommand::CapturePhoto { quality: 101, flash: 0 }.encode_command().unwrap_err(),
        CameraCodecError::InvalidEnum("PhotoQuality", 101)
    );
    assert_eq!(
        CameraPreviewCommand::CapturePhoto { quality: 80, flash: 3 }.encode_command().unwrap_err(),
        CameraCodecError::InvalidEnum("FlashMode", 3)
    );
    assert!(CameraPreviewCommand::FocusPoint { x: -0.01, y: 0.5 }.encode_command().is_err());
    assert!(CameraPreviewCommand::FocusPoint { x: 0.5, y: 1.01 }.encode_command().is_err());

    // NVCF truncation, trailing bytes, reserved, bad rotation, overflow
    let valid_nvcf = CameraFrameDescriptor {
        width: 1280,
        height: 720,
        stride_bytes: 1280 * 4,
        pixel_format: CameraPixelFormat::Rgba8888,
        rotation_deg: 90,
        timestamp_ns: 42,
        native_handle_id: 99,
    }
    .encode()
    .unwrap();
    for cut in 0..CAMERA_FRAME_DESCRIPTOR_SIZE {
        assert_eq!(
            CameraFrameDescriptor::decode(&valid_nvcf[..cut]).unwrap_err(),
            CameraCodecError::Truncated
        );
    }
    let mut trailing = valid_nvcf.to_vec();
    trailing.push(0xFF);
    assert_eq!(
        CameraFrameDescriptor::decode(&trailing).unwrap_err(),
        CameraCodecError::TrailingBytes(1)
    );
    let mut bad_res = valid_nvcf;
    bad_res[20] = 1;
    assert_eq!(
        CameraFrameDescriptor::decode(&bad_res).unwrap_err(),
        CameraCodecError::InvalidEnum("CameraFrameReserved", 1)
    );
    let overflow = CameraFrameDescriptor {
        width: u32::MAX,
        height: 1,
        stride_bytes: u32::MAX,
        pixel_format: CameraPixelFormat::Bgra8888,
        rotation_deg: 0,
        timestamp_ns: 0,
        native_handle_id: 0,
    };
    assert_eq!(overflow.encode().unwrap_err(), CameraCodecError::PayloadTooLarge);
}

#[test]
fn deterministic_fuzz_never_panics() {
    let mut state: u64 = 0x5EED_CAFE_1234_9876;
    let mut next_u8 = || -> u8 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as u8
    };
    let mut buf = Vec::with_capacity(96);
    for case in 0..2000 {
        let len = (case * 7) % 88;
        buf.clear();
        for _ in 0..len {
            buf.push(next_u8());
        }
        if len >= 8 {
            match case % 4 {
                0 => { buf[0..4].copy_from_slice(&NATIVE_PROPS_MAGIC); buf[4] = 1; }
                1 => { buf[0..4].copy_from_slice(&NATIVE_CMD_MAGIC); buf[4] = 1; }
                2 => { buf[0..4].copy_from_slice(&CAMERA_FRAME_MAGIC); buf[4] = CAMERA_FRAME_VERSION; }
                _ => {}
            }
        }
        let _ = CameraPreviewProps::from_prop_bytes(&buf);
        let _ = CameraPreviewCommand::from_command_bytes(&buf);
        let _ = CameraFrameDescriptor::decode(&buf);
    }
}
