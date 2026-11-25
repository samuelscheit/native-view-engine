#![cfg(feature = "interop")]

use nve_abi::{Command, CommandBufferBuilder, CommandDecoder};
use nve_camera::{
    CameraFacing, CameraPreview, CameraPreviewCommand, CameraPreviewProps, StabilizationMode,
    TorchMode, CAMERA_PREVIEW_KIND, MAX_CAMERA_FPS,
};
use nve_interop::native_view::{
    DecodeNativeCommand, DecodeNativeProps, EncodeNativeCommand, EncodeNativeProps,
    NativeCommandDecoder, NativePropDecoder, NativePropEncoder, NativeViewCodecError,
    NativeViewHandle, NativeViewRegistry, NativeViewSpec, NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC,
};

#[test]
fn camera_preview_registry_and_spec_contract() {
    assert_eq!(<CameraPreview as NativeViewSpec>::KIND, CAMERA_PREVIEW_KIND);
    assert_eq!(<CameraPreview as NativeViewSpec>::KIND, "nve.camera.preview");

    let mut reg = NativeViewRegistry::new();
    assert!(!reg.supports_spec::<CameraPreview>());
    assert!(!reg.supports(CAMERA_PREVIEW_KIND));

    let reg_with = NativeViewRegistry::new().with_spec::<CameraPreview>();
    assert!(reg_with.supports_spec::<CameraPreview>());
    assert!(reg_with.supports(CAMERA_PREVIEW_KIND));
    assert!(reg_with.supports_props::<CameraPreview>(&CameraPreviewProps::default()));

    reg.register_spec::<CameraPreview>();
    assert!(reg.supports_spec::<CameraPreview>());
    assert!(reg.unregister(CAMERA_PREVIEW_KIND));
    assert!(!reg.supports_spec::<CameraPreview>());
}

#[test]
fn camera_preview_props_and_commands_roundtrip_via_interop_traits() {
    let props = CameraPreviewProps::new()
        .with_device_id("back-telephoto")
        .with_facing(CameraFacing::Front)
        .with_torch(TorchMode::Auto)
        .with_zoom(2.25)
        .with_fps(MAX_CAMERA_FPS)
        .with_hdr(true)
        .with_stabilization(StabilizationMode::Cinematic)
        .with_frame_processor(true)
        .with_audio(true)
        .with_active(true);

    // 1. Direct trait bytes path matches canonical `CameraPreviewProps::to_prop_bytes` byte-for-byte.
    let via_trait = EncodeNativeProps::to_prop_bytes(&props).unwrap();
    let canonical = props.to_prop_bytes().unwrap();
    assert!(via_trait.starts_with(&NATIVE_PROPS_MAGIC));
    assert_eq!(via_trait, canonical);
    assert_eq!(
        <CameraPreviewProps as DecodeNativeProps>::from_prop_bytes(&via_trait).unwrap(),
        props
    );

    // 2. Encoder / decoder streaming methods round-trip identically.
    let mut enc = NativePropEncoder::new();
    props.encode_props(&mut enc).unwrap();
    let streamed = enc.finish();
    assert_eq!(streamed, canonical);
    let dec = NativePropDecoder::decode(&streamed).unwrap();
    assert_eq!(CameraPreviewProps::decode_props(&dec).unwrap(), props);

    // 3. Invalid prop values map cleanly onto `NativeViewCodecError`.
    let bad_facing = CameraPreviewProps::default().with_facing(CameraFacing::Unknown);
    assert_eq!(
        EncodeNativeProps::to_prop_bytes(&bad_facing).unwrap_err(),
        NativeViewCodecError::InvalidEnum("CameraFacing", 0)
    );
    let bad_zoom = CameraPreviewProps::default().with_zoom(f32::NAN);
    assert!(matches!(
        EncodeNativeProps::to_prop_bytes(&bad_zoom),
        Err(NativeViewCodecError::TypeMismatch { .. })
    ));
    let bad_fps = CameraPreviewProps::default().with_fps(MAX_CAMERA_FPS + 1);
    assert_eq!(
        EncodeNativeProps::to_prop_bytes(&bad_fps).unwrap_err(),
        NativeViewCodecError::InvalidEnum("CameraFps", u32::from(MAX_CAMERA_FPS + 1))
    );

    // 4. All 7 imperative commands round-trip through both `from_command_bytes` and `decode_command`.
    let commands = [
        (
            "switch_facing",
            CameraPreviewCommand::SwitchFacing(CameraFacing::External),
        ),
        ("set_torch", CameraPreviewCommand::SetTorch(TorchMode::On)),
        ("set_zoom", CameraPreviewCommand::SetZoom(3.5)),
        (
            "focus_point",
            CameraPreviewCommand::FocusPoint { x: 0.25, y: 0.75 },
        ),
        (
            "capture_photo",
            CameraPreviewCommand::CapturePhoto {
                quality: 92,
                flash: 2,
            },
        ),
        (
            "start_recording",
            CameraPreviewCommand::StartRecording {
                output_path: "/tmp/camera.mp4".into(),
            },
        ),
        ("stop_recording", CameraPreviewCommand::StopRecording),
    ];
    for (expected_name, cmd) in &commands {
        assert_eq!(EncodeNativeCommand::command_name(cmd), *expected_name);
        let bytes = EncodeNativeCommand::encode_command(cmd).unwrap();
        assert!(bytes.starts_with(&NATIVE_CMD_MAGIC));
        assert_eq!(bytes, cmd.encode_command().unwrap());
        assert_eq!(
            <CameraPreviewCommand as DecodeNativeCommand>::from_command_bytes(&bytes).unwrap(),
            *cmd
        );
        let cmd_dec = NativeCommandDecoder::decode(&bytes).unwrap();
        assert_eq!(CameraPreviewCommand::decode_command(&cmd_dec).unwrap(), *cmd);
    }

    // 5. Invalid commands & corrupt buffers map onto `NativeViewCodecError`.
    assert_eq!(
        EncodeNativeCommand::encode_command(&CameraPreviewCommand::SwitchFacing(
            CameraFacing::Unknown
        ))
        .unwrap_err(),
        NativeViewCodecError::InvalidEnum("CameraFacing", 0)
    );
    assert_eq!(
        EncodeNativeCommand::encode_command(&CameraPreviewCommand::CapturePhoto {
            quality: 101,
            flash: 0
        })
        .unwrap_err(),
        NativeViewCodecError::InvalidEnum("PhotoQuality", 101)
    );
    assert_eq!(
        <CameraPreviewCommand as DecodeNativeCommand>::from_command_bytes(&[
            b'N', b'V', b'N', b'C', 1, 99, 0, 0
        ])
        .unwrap_err(),
        NativeViewCodecError::UnknownCommandOpcode(99)
    );
    assert_eq!(
        <CameraPreviewCommand as DecodeNativeCommand>::from_command_bytes(&[]).unwrap_err(),
        NativeViewCodecError::Truncated
    );
}

#[test]
fn camera_preview_handle_and_command_buffer_dispatch_framing() {
    let handle = NativeViewHandle::<CameraPreview>::new();
    assert!(!handle.is_bound());
    assert_eq!(
        handle.dispatch(CameraPreviewCommand::StopRecording),
        Err(NativeViewCodecError::UnboundHandle)
    );
    assert_eq!(
        handle.encode_command_frame(1, CameraPreviewCommand::SetTorch(TorchMode::On)),
        Err(NativeViewCodecError::UnboundHandle)
    );

    handle.bind(5, 77);
    assert!(handle.is_bound());
    assert_eq!(handle.surface_id(), Some(5));
    assert_eq!(handle.node_id(), Some(77));

    // Single-command frame via `encode_command_frame`.
    let single_frame = handle
        .encode_command_frame(10, CameraPreviewCommand::SetZoom(2.5))
        .unwrap();
    let (hdr, cmds) = CommandDecoder::new(&single_frame)
        .unwrap()
        .decode_all()
        .unwrap();
    assert_eq!(hdr.surface_id, 5);
    assert_eq!(hdr.frame_id, 10);
    assert_eq!(cmds.len(), 2);
    match &cmds[0] {
        Command::DispatchCommand {
            node_id,
            command_name,
            payload,
        } => {
            assert_eq!(*node_id, 77);
            assert_eq!(command_name, "set_zoom");
            assert_eq!(
                <CameraPreviewCommand as DecodeNativeCommand>::from_command_bytes(payload)
                    .unwrap(),
                CameraPreviewCommand::SetZoom(2.5)
            );
        }
        other => panic!("expected DispatchCommand, got {other:?}"),
    }

    // Batched commands flushed directly into `CommandBufferBuilder`.
    handle
        .dispatch(CameraPreviewCommand::SwitchFacing(CameraFacing::Front))
        .unwrap();
    handle
        .dispatch(CameraPreviewCommand::FocusPoint { x: 0.5, y: 0.5 })
        .unwrap();
    handle
        .dispatch(CameraPreviewCommand::StartRecording {
            output_path: "/tmp/rec.mov".into(),
        })
        .unwrap();
    handle
        .dispatch(CameraPreviewCommand::StopRecording)
        .unwrap();
    assert_eq!(handle.pending_count(), 4);

    let mut builder = CommandBufferBuilder::new(5, 11);
    assert_eq!(handle.flush_into_builder(&mut builder), 4);
    assert_eq!(handle.pending_count(), 0);
    builder.commit(false);

    let (batch_hdr, batch_cmds) = CommandDecoder::new(builder.finish())
        .unwrap()
        .decode_all()
        .unwrap();
    assert_eq!(batch_hdr.surface_id, 5);
    assert_eq!(batch_hdr.frame_id, 11);

    let decoded: Vec<(&str, CameraPreviewCommand)> = batch_cmds
        .iter()
        .filter_map(|cmd| match cmd {
            Command::DispatchCommand {
                node_id,
                command_name,
                payload,
            } => {
                assert_eq!(*node_id, 77);
                Some((
                    command_name.as_str(),
                    <CameraPreviewCommand as DecodeNativeCommand>::from_command_bytes(payload)
                        .unwrap(),
                ))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        decoded,
        vec![
            (
                "switch_facing",
                CameraPreviewCommand::SwitchFacing(CameraFacing::Front)
            ),
            (
                "focus_point",
                CameraPreviewCommand::FocusPoint { x: 0.5, y: 0.5 }
            ),
            (
                "start_recording",
                CameraPreviewCommand::StartRecording {
                    output_path: "/tmp/rec.mov".into()
                }
            ),
            ("stop_recording", CameraPreviewCommand::StopRecording),
        ]
    );
}
