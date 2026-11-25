use super::*;

#[test]
fn host_callbacks_abi_layout_matches_c_header() {
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(std::mem::size_of::<NveHostCallbacks>(), 24);
        assert_eq!(std::mem::offset_of!(NveHostCallbacks, host_ctx), 0);
        assert_eq!(std::mem::offset_of!(NveHostCallbacks, invoke), 8);
        assert_eq!(std::mem::offset_of!(NveHostCallbacks, channel_publish), 16);
    }
}

#[test]
fn host_status_roundtrips_all_discriminants() {
    for status in [
        NveHostStatus::Ok,
        NveHostStatus::Pending,
        NveHostStatus::BufferTooSmall,
        NveHostStatus::Unsupported,
        NveHostStatus::PermissionDenied,
        NveHostStatus::InvalidArg,
        NveHostStatus::NotFound,
        NveHostStatus::Cancelled,
        NveHostStatus::IoError,
        NveHostStatus::InternalError,
    ] {
        assert_eq!(NveHostStatus::from_i32(status.as_i32()), status);
    }
    assert_eq!(NveHostStatus::from_i32(-999), NveHostStatus::InternalError);
}
