use nve_camera::{
    CameraCodecError, CameraFrameDescriptor, CameraFrameWireLayout, CameraPixelFormat,
    CAMERA_FRAME_DESCRIPTOR_SIZE, CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION,
};

#[test]
fn camera_frame_descriptor_roundtrip_and_validation() {
    let desc = CameraFrameDescriptor {
        width: 1920,
        height: 1080,
        stride_bytes: 1920 * 4,
        pixel_format: CameraPixelFormat::Rgba8888,
        rotation_deg: 90,
        timestamp_ns: 123_456_789,
        native_handle_id: 0xDEAD_BEEF_CAFE_BABEu64,
    };
    let wire = desc.to_wire_layout().unwrap();
    assert_eq!(wire.magic, CAMERA_FRAME_MAGIC);
    assert_eq!(wire.version, CAMERA_FRAME_VERSION);
    assert_eq!(wire.to_descriptor().unwrap(), desc);

    let bytes = desc.encode().unwrap();
    assert_eq!(bytes.len(), CAMERA_FRAME_DESCRIPTOR_SIZE);
    assert_eq!(CameraFrameDescriptor::decode(&bytes).unwrap(), desc);

    // Reject bad rotation, stride, truncation, and trailing bytes.
    let bad_rot = CameraFrameDescriptor {
        rotation_deg: 45,
        ..desc
    };
    assert!(matches!(
        bad_rot.validate(),
        Err(CameraCodecError::InvalidEnum("CameraRotationDeg", 45))
    ));
    let bad_stride = CameraFrameDescriptor {
        stride_bytes: 100,
        ..desc
    };
    assert!(matches!(
        bad_stride.validate(),
        Err(CameraCodecError::TypeMismatch { .. })
    ));
    assert_eq!(
        CameraFrameDescriptor::decode(&bytes[..10]),
        Err(CameraCodecError::Truncated)
    );
    let mut with_extra = bytes.to_vec();
    with_extra.push(0);
    assert_eq!(
        CameraFrameDescriptor::decode(&with_extra),
        Err(CameraCodecError::TrailingBytes(1))
    );
    let _ = CameraFrameWireLayout::from_descriptor(&desc).unwrap();
}
