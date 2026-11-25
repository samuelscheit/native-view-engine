//! Deep unit, edge-case, and deterministic fuzz tests for [`CameraPreview`] (`"nve.camera.preview"`),
//! its zero-JSON `NVNP`/`NVNC` codecs, `NativeViewHandle<CameraPreview>` `Command::DispatchCommand`
//! framing, and 40-byte zero-copy [`CameraFrameDescriptor`] (`NVCF`) binary codec.

use nve_abi::{Command, CommandBufferBuilder, CommandDecoder};
use nve_interop::native_view::{
    CameraFrameDescriptor, CameraPixelFormat, CameraPreview, CameraPreviewCommand,
    CameraPreviewFacing, CameraPreviewProps, DecodeNativeCommand, DecodeNativeProps,
    EncodeNativeCommand, EncodeNativeProps, NativeViewCodecError, NativeViewHandle,
    NativeViewRegistry, NativeViewSpec, StabilizationMode, TorchMode, CAMERA_FRAME_DESCRIPTOR_SIZE,
    CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION, CAMERA_PREVIEW_KIND, MAX_CAMERA_FPS,
    NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC,
};

#[test]
fn camera_preview_props_zero_json_roundtrip_and_enum_alignment() {
    assert_eq!(CameraPreview::KIND, "nve.camera.preview");
    assert_eq!(CAMERA_PREVIEW_KIND, "nve.camera.preview");

    // Verify discriminant alignment with nve-sdk::camera (Back=1, Front=2, External=3; Flash/Torch 0..=2)
    assert_eq!(CameraPreviewFacing::Back as u8, 1);
    assert_eq!(CameraPreviewFacing::Front as u8, 2);
    assert_eq!(CameraPreviewFacing::External as u8, 3);
    assert_eq!(TorchMode::Off as u8, 0);
    assert_eq!(TorchMode::On as u8, 1);
    assert_eq!(TorchMode::Auto as u8, 2);
    assert_eq!(StabilizationMode::Off as u8, 0);
    assert_eq!(StabilizationMode::Standard as u8, 1);
    assert_eq!(StabilizationMode::Cinematic as u8, 2);

    // Default props round-trip with device_id = None
    let defaults = CameraPreviewProps::default();
    let default_bytes = defaults.to_prop_bytes().unwrap();
    assert!(default_bytes.starts_with(&NATIVE_PROPS_MAGIC));
    assert_eq!(
        CameraPreviewProps::from_prop_bytes(&default_bytes).unwrap(),
        defaults
    );

    // Full customized props across all enums & ultrawide zoom (0.5x)
    let custom = CameraPreviewProps::new()
        .with_device_id("com.apple.avfoundation.avcapturedevice.built-in_video:0")
        .with_facing(CameraPreviewFacing::Front)
        .with_torch(TorchMode::Auto)
        .with_zoom(0.5)
        .with_fps(120)
        .with_hdr(true)
        .with_stabilization(StabilizationMode::Cinematic)
        .with_frame_processor(true)
        .with_audio(true)
        .with_active(false);
    let custom_bytes = custom.to_prop_bytes().unwrap();
    assert!(custom_bytes.starts_with(&NATIVE_PROPS_MAGIC));
    assert_eq!(
        CameraPreviewProps::from_prop_bytes(&custom_bytes).unwrap(),
        custom
    );

    // External camera + max FPS boundary
    let ext = CameraPreviewProps::new()
        .with_device_id("usb-uvc-4k")
        .with_facing(CameraPreviewFacing::External)
        .with_torch(TorchMode::On)
        .with_zoom(8.25)
        .with_fps(MAX_CAMERA_FPS)
        .with_stabilization(StabilizationMode::Standard);
    assert_eq!(
        CameraPreviewProps::from_prop_bytes(&ext.to_prop_bytes().unwrap()).unwrap(),
        ext
    );

    // Registry registration
    let reg = NativeViewRegistry::new().with_spec::<CameraPreview>();
    assert!(reg.supports_spec::<CameraPreview>());
    assert!(reg.supports("nve.camera.preview"));
    assert!(reg.supports_props::<CameraPreview>(&custom));
}

#[test]
fn camera_preview_props_reject_nan_inf_out_of_range_and_corrupt_wire() {
    // Reject NaN / Inf / <= 0 zoom on encode
    for bad_zoom in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.5] {
        let props = CameraPreviewProps::default().with_zoom(bad_zoom);
        assert!(
            props.to_prop_bytes().is_err(),
            "zoom={bad_zoom} must be rejected on encode"
        );
    }

    // Reject 0 and > 240 FPS on encode
    for bad_fps in [0u16, 241, u16::MAX] {
        let props = CameraPreviewProps::default().with_fps(bad_fps);
        assert_eq!(
            props.to_prop_bytes().unwrap_err(),
            NativeViewCodecError::InvalidEnum("CameraFps", u32::from(bad_fps))
        );
    }

    // Mutate valid wire bytes to inject NaN zoom -> must fail on decode
    let mut bytes = CameraPreviewProps::default().to_prop_bytes().unwrap();
    let one_bits = 1.0f32.to_le_bytes();
    let zoom_pos = bytes
        .windows(4)
        .position(|w| w == one_bits)
        .expect("locate 1.0f32 zoom bytes");
    bytes[zoom_pos..zoom_pos + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(matches!(
        CameraPreviewProps::from_prop_bytes(&bytes),
        Err(NativeViewCodecError::TypeMismatch { .. })
    ));

    // Mutate to negative infinity zoom -> must fail on decode
    bytes[zoom_pos..zoom_pos + 4].copy_from_slice(&f32::NEG_INFINITY.to_le_bytes());
    assert!(matches!(
        CameraPreviewProps::from_prop_bytes(&bytes),
        Err(NativeViewCodecError::TypeMismatch { .. })
    ));

    // Every strict prefix of valid props buffer must fail without panicking
    let valid = CameraPreviewProps::default()
        .with_device_id("back-wide-0")
        .to_prop_bytes()
        .unwrap();
    for cut in 0..valid.len() {
        assert!(
            CameraPreviewProps::from_prop_bytes(&valid[..cut]).is_err(),
            "truncated props len={cut} should fail"
        );
    }

    // Trailing byte must fail
    let mut trailing = valid.clone();
    trailing.push(0x00);
    assert_eq!(
        CameraPreviewProps::from_prop_bytes(&trailing).unwrap_err(),
        NativeViewCodecError::TrailingBytes(1)
    );
}

#[test]
fn camera_preview_commands_and_handle_dispatch_command_framing() {
    let all_commands = vec![
        CameraPreviewCommand::SwitchFacing(CameraPreviewFacing::Front),
        CameraPreviewCommand::SwitchFacing(CameraPreviewFacing::Back),
        CameraPreviewCommand::SwitchFacing(CameraPreviewFacing::External),
        CameraPreviewCommand::SetTorch(TorchMode::Off),
        CameraPreviewCommand::SetTorch(TorchMode::On),
        CameraPreviewCommand::SetTorch(TorchMode::Auto),
        CameraPreviewCommand::SetZoom(2.5),
        CameraPreviewCommand::FocusPoint { x: 0.25, y: 0.75 },
        CameraPreviewCommand::CapturePhoto {
            quality: 92,
            flash: 2,
        },
        CameraPreviewCommand::StartRecording {
            output_path: "/data/user/0/dev.nve/cache/clip_01.mp4".into(),
        },
        CameraPreviewCommand::StopRecording,
    ];

    let expected_names = [
        "switch_facing",
        "switch_facing",
        "switch_facing",
        "set_torch",
        "set_torch",
        "set_torch",
        "set_zoom",
        "focus_point",
        "capture_photo",
        "start_recording",
        "stop_recording",
    ];

    // Unbound handle fails cleanly
    let handle = NativeViewHandle::<CameraPreview>::new();
    assert_eq!(
        handle.dispatch(CameraPreviewCommand::StopRecording),
        Err(NativeViewCodecError::UnboundHandle)
    );
    assert_eq!(
        handle.encode_command_frame(1, CameraPreviewCommand::SetZoom(2.0)),
        Err(NativeViewCodecError::UnboundHandle)
    );

    // Bind and dispatch every command into a single nve-abi CommandBuffer frame
    handle.bind(11, 77);
    for cmd in &all_commands {
        handle.dispatch(cmd.clone()).unwrap();
    }
    assert_eq!(handle.pending_count(), all_commands.len());

    let frame = handle.take_command_frame(42).unwrap();
    let (hdr, abi_cmds) = CommandDecoder::new(&frame).unwrap().decode_all().unwrap();
    assert_eq!(hdr.surface_id, 11);
    assert_eq!(hdr.frame_id, 42);
    assert_eq!(abi_cmds.len(), all_commands.len() + 1); // +1 Commit

    let mut decoded_cmds = Vec::new();
    for (idx, abi_cmd) in abi_cmds[..all_commands.len()].iter().enumerate() {
        match abi_cmd {
            Command::DispatchCommand {
                node_id,
                command_name,
                payload,
            } => {
                assert_eq!(*node_id, 77);
                assert_eq!(command_name, expected_names[idx]);
                assert!(payload.starts_with(&NATIVE_CMD_MAGIC));
                decoded_cmds.push(CameraPreviewCommand::from_command_bytes(payload).unwrap());
            }
            other => panic!("expected DispatchCommand, got {other:?}"),
        }
    }
    assert_eq!(decoded_cmds, all_commands);

    // Flush directly into an existing CommandBufferBuilder + encode_command_frame
    let single = handle
        .encode_command_frame(
            43,
            CameraPreviewCommand::FocusPoint { x: 0.5, y: 0.5 },
        )
        .unwrap();
    let (_, single_cmds) = CommandDecoder::new(&single).unwrap().decode_all().unwrap();
    assert!(matches!(
        &single_cmds[0],
        Command::DispatchCommand { command_name, .. } if command_name == "focus_point"
    ));

    handle
        .dispatch(CameraPreviewCommand::CapturePhoto {
            quality: 100,
            flash: 0,
        })
        .unwrap();
    let mut builder = CommandBufferBuilder::new(11, 44);
    assert_eq!(handle.flush_into_builder(&mut builder), 1);
    builder.commit(false);
    let (_, flushed) = CommandDecoder::new(builder.finish())
        .unwrap()
        .decode_all()
        .unwrap();
    assert_eq!(flushed.len(), 2);
}

#[test]
fn camera_preview_commands_reject_invalid_args_nan_and_truncation() {
    // NaN / Inf / non-positive zoom
    for bad in [f32::NAN, f32::INFINITY, 0.0, -0.5] {
        assert!(CameraPreviewCommand::SetZoom(bad).encode_command().is_err());
    }

    // Out-of-bounds or NaN focus points
    for (x, y) in [
        (f32::NAN, 0.5),
        (0.5, f32::INFINITY),
        (-0.01, 0.5),
        (0.5, 1.01),
    ] {
        assert!(CameraPreviewCommand::FocusPoint { x, y }
            .encode_command()
            .is_err());
    }

    // Out-of-range quality or flash
    assert_eq!(
        CameraPreviewCommand::CapturePhoto {
            quality: 101,
            flash: 0,
        }
        .encode_command()
        .unwrap_err(),
        NativeViewCodecError::InvalidEnum("PhotoQuality", 101)
    );
    assert_eq!(
        CameraPreviewCommand::CapturePhoto {
            quality: 85,
            flash: 3,
        }
        .encode_command()
        .unwrap_err(),
        NativeViewCodecError::InvalidEnum("FlashMode", 3)
    );

    // Inject NaN into valid FocusPoint NVNC wire payload -> rejected on decode
    let mut fp_wire = CameraPreviewCommand::FocusPoint { x: 0.5, y: 0.25 }
        .encode_command()
        .unwrap();
    // Header (8) + F32 tag (1) = offset 9 for x
    fp_wire[9..13].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(CameraPreviewCommand::from_command_bytes(&fp_wire).is_err());

    // Truncation at every byte prefix
    let rec_wire = CameraPreviewCommand::StartRecording {
        output_path: "/tmp/out.mp4".into(),
    }
    .encode_command()
    .unwrap();
    for cut in 0..rec_wire.len() {
        assert!(CameraPreviewCommand::from_command_bytes(&rec_wire[..cut]).is_err());
    }

    // Unknown opcode
    let unknown_op = [b'N', b'V', b'N', b'C', 1, 99, 0, 0];
    assert_eq!(
        CameraPreviewCommand::from_command_bytes(&unknown_op).unwrap_err(),
        NativeViewCodecError::UnknownCommandOpcode(99)
    );
}

#[test]
fn camera_frame_descriptor_nvcf_exact_40_byte_layout_and_roundtrip() {
    assert_eq!(CAMERA_FRAME_DESCRIPTOR_SIZE, 40);

    let formats = [
        (CameraPixelFormat::Rgba8888, 1920 * 4),
        (CameraPixelFormat::Bgra8888, 1920 * 4 + 64),
        (CameraPixelFormat::Nv12, 1920),
        (CameraPixelFormat::Yuv420_888, 2048),
    ];

    for (idx, &(pixel_format, stride_bytes)) in formats.iter().enumerate() {
        let rotation_deg = (idx as u16) * 90;
        let desc = CameraFrameDescriptor {
            width: 1920,
            height: 1080,
            stride_bytes,
            pixel_format,
            rotation_deg,
            timestamp_ns: 0x0102_0304_0506_0708 + idx as u64,
            native_handle_id: 0xDEAD_BEEF_CAFE_BABE ^ (idx as u64),
        };
        let raw = desc.encode().unwrap();
        assert_eq!(raw.len(), 40);
        assert_eq!(&raw[0..4], &CAMERA_FRAME_MAGIC);
        assert_eq!(raw[4], CAMERA_FRAME_VERSION);
        assert_eq!(raw[5], pixel_format as u8);
        assert_eq!(u16::from_le_bytes([raw[6], raw[7]]), rotation_deg);
        assert_eq!(u32::from_le_bytes(raw[8..12].try_into().unwrap()), 1920);
        assert_eq!(u32::from_le_bytes(raw[12..16].try_into().unwrap()), 1080);
        assert_eq!(
            u32::from_le_bytes(raw[16..20].try_into().unwrap()),
            stride_bytes
        );
        assert_eq!(u32::from_le_bytes(raw[20..24].try_into().unwrap()), 0);
        assert_eq!(
            u64::from_le_bytes(raw[24..32].try_into().unwrap()),
            desc.timestamp_ns
        );
        assert_eq!(
            u64::from_le_bytes(raw[32..40].try_into().unwrap()),
            desc.native_handle_id
        );

        assert_eq!(CameraFrameDescriptor::decode(&raw).unwrap(), desc);
        assert_eq!(
            CameraFrameDescriptor::decode(&desc.to_bytes().unwrap()).unwrap(),
            desc
        );
    }
}

#[test]
fn camera_frame_descriptor_nvcf_rejects_corrupt_truncated_and_invalid_fields() {
    let valid = CameraFrameDescriptor {
        width: 1280,
        height: 720,
        stride_bytes: 1280 * 4,
        pixel_format: CameraPixelFormat::Rgba8888,
        rotation_deg: 90,
        timestamp_ns: 99_000_000,
        native_handle_id: 0x1122_3344_5566_7788,
    }
    .encode()
    .unwrap();

    // Every truncated length 0..40 fails with Truncated
    for cut in 0..CAMERA_FRAME_DESCRIPTOR_SIZE {
        assert_eq!(
            CameraFrameDescriptor::decode(&valid[..cut]).unwrap_err(),
            NativeViewCodecError::Truncated
        );
    }

    // Trailing byte fails with TrailingBytes(1)
    let mut with_extra = valid.to_vec();
    with_extra.push(0xAA);
    assert_eq!(
        CameraFrameDescriptor::decode(&with_extra).unwrap_err(),
        NativeViewCodecError::TrailingBytes(1)
    );

    // Bad magic
    let mut bad_magic = valid;
    bad_magic[0..4].copy_from_slice(b"NOPE");
    assert_eq!(
        CameraFrameDescriptor::decode(&bad_magic).unwrap_err(),
        NativeViewCodecError::InvalidMagic(*b"NOPE")
    );

    // Unsupported version
    let mut bad_ver = valid;
    bad_ver[4] = 2;
    assert_eq!(
        CameraFrameDescriptor::decode(&bad_ver).unwrap_err(),
        NativeViewCodecError::UnsupportedVersion(2)
    );

    // Invalid pixel format
    let mut bad_fmt = valid;
    bad_fmt[5] = 9;
    assert_eq!(
        CameraFrameDescriptor::decode(&bad_fmt).unwrap_err(),
        NativeViewCodecError::InvalidEnum("CameraPixelFormat", 9)
    );

    // Invalid rotation (e.g. 45 deg)
    let mut bad_rot = valid;
    bad_rot[6..8].copy_from_slice(&45u16.to_le_bytes());
    assert_eq!(
        CameraFrameDescriptor::decode(&bad_rot).unwrap_err(),
        NativeViewCodecError::InvalidEnum("CameraRotationDeg", 45)
    );

    // Zero width / zero height
    let mut zero_w = valid;
    zero_w[8..12].copy_from_slice(&0u32.to_le_bytes());
    assert!(CameraFrameDescriptor::decode(&zero_w).is_err());

    let mut zero_h = valid;
    zero_h[12..16].copy_from_slice(&0u32.to_le_bytes());
    assert!(CameraFrameDescriptor::decode(&zero_h).is_err());

    // Stride smaller than width * 4 for RGBA8888
    let mut bad_stride = valid;
    bad_stride[16..20].copy_from_slice(&(1280u32 * 4 - 1).to_le_bytes());
    assert!(CameraFrameDescriptor::decode(&bad_stride).is_err());

    // Non-zero reserved field in v1
    let mut bad_reserved = valid;
    bad_reserved[20..24].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        CameraFrameDescriptor::decode(&bad_reserved).unwrap_err(),
        NativeViewCodecError::InvalidEnum("CameraFrameReserved", 1)
    );

    // Width * 4 overflow for RGBA8888
    let overflow_desc = CameraFrameDescriptor {
        width: u32::MAX,
        height: 1,
        stride_bytes: u32::MAX,
        pixel_format: CameraPixelFormat::Rgba8888,
        rotation_deg: 0,
        timestamp_ns: 0,
        native_handle_id: 1,
    };
    assert_eq!(
        overflow_desc.encode().unwrap_err(),
        NativeViewCodecError::PayloadTooLarge
    );
}

#[test]
fn deterministic_fuzz_never_panics_on_props_commands_or_nvcf_decoders() {
    let mut state: u64 = 0x5EED_CAFE_1234_9876;
    let mut next_byte = || -> u8 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as u8
    };

    let mut buf = Vec::with_capacity(96);
    for case in 0..2000 {
        let len = (case * 7) % 88;
        buf.clear();
        for _ in 0..len {
            buf.push(next_byte());
        }
        // Periodically stamp valid headers so fuzzing reaches deep field parsing
        if len >= 8 {
            match case % 4 {
                0 => {
                    buf[0..4].copy_from_slice(&NATIVE_PROPS_MAGIC);
                    buf[4] = 1;
                }
                1 => {
                    buf[0..4].copy_from_slice(&NATIVE_CMD_MAGIC);
                    buf[4] = 1;
                }
                2 => {
                    buf[0..4].copy_from_slice(&CAMERA_FRAME_MAGIC);
                    buf[4] = CAMERA_FRAME_VERSION;
                }
                _ => {}
            }
        }
        let _ = CameraPreviewProps::from_prop_bytes(&buf);
        let _ = CameraPreviewCommand::from_command_bytes(&buf);
        let _ = CameraFrameDescriptor::decode(&buf);
    }
}

#[test]
fn cross_language_golden_byte_vectors_match_kotlin_codec() {
    // Golden NVNP vector for default CameraPreviewProps
    let default_props_bytes = CameraPreviewProps::default().to_prop_bytes().unwrap();
    let expected_default_props: &[u8] = &[
        b'N', b'V', b'N', b'P', 1, 10, 0, // header: version=1, 10 fields
        1, 0, 0, // field 1: device_id = Null
        2, 0, 2, 1, 0, 0, 0, // field 2: facing = U32(1 = Back)
        3, 0, 2, 0, 0, 0, 0, // field 3: torch = U32(0 = Off)
        4, 0, 5, 0x00, 0x00, 0x80, 0x3F, // field 4: zoom = F32(1.0)
        5, 0, 2, 30, 0, 0, 0, // field 5: fps = U32(30)
        6, 0, 1, 0, // field 6: hdr = Bool(false)
        7, 0, 2, 0, 0, 0, 0, // field 7: stabilization = U32(0 = Off)
        8, 0, 1, 0, // field 8: enable_frame_processor = Bool(false)
        9, 0, 1, 0, // field 9: audio_enabled = Bool(false)
        10, 0, 1, 1, // field 10: active = Bool(true)
    ];
    assert_eq!(default_props_bytes, expected_default_props);

    // Golden NVNC vector for FocusPoint { x: 0.25, y: 0.75 }
    let focus_bytes = CameraPreviewCommand::FocusPoint { x: 0.25, y: 0.75 }
        .encode_command()
        .unwrap();
    let expected_focus: &[u8] = &[
        b'N', b'V', b'N', b'C', 1, 4, 0, 2, // header: version=1, op=4, args=2
        5, 0x00, 0x00, 0x80, 0x3E, // arg 0: F32(0.25)
        5, 0x00, 0x00, 0x40, 0x3F, // arg 1: F32(0.75)
    ];
    assert_eq!(focus_bytes, expected_focus);

    // Golden NVCF 40-byte vector
    let nvcf = CameraFrameDescriptor {
        width: 1920,
        height: 1080,
        stride_bytes: 1920,
        pixel_format: CameraPixelFormat::Yuv420_888,
        rotation_deg: 270,
        timestamp_ns: 0x0102_0304_0506_0708,
        native_handle_id: 0x1122_3344_5566_7788,
    }
    .encode()
    .unwrap();
    let expected_nvcf: [u8; 40] = [
        b'N', b'V', b'C', b'F', 1, 3, 0x0E, 0x01, // magic, ver=1, Yuv420_888=3, rot=270
        0x80, 0x07, 0x00, 0x00, // width=1920
        0x38, 0x04, 0x00, 0x00, // height=1080
        0x80, 0x07, 0x00, 0x00, // stride_bytes=1920
        0x00, 0x00, 0x00, 0x00, // reserved=0
        0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // timestamp_ns
        0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, // native_handle_id
    ];
    assert_eq!(nvcf, expected_nvcf);
}
