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
fn nvh1_batch_roundtrip_all_three_record_kinds() {
    let mut batch = NveHostEventBatch::new();
    batch.push_async_response(42, NveHostStatus::Ok, b"hello-world".to_vec());
    batch.push_async_response(99, NveHostStatus::Cancelled, Vec::new());
    batch.push_host_event(SERVICE_LINKING, 1, b"nve://open/doc/7".to_vec());
    batch.push_channel_message(1, NVE_CHANNEL_FLAG_RETAIN, vec![1, 2, 3, 4]);

    let bytes = batch.encode();
    assert_eq!(&bytes[..4], b"NVH1");
    let decoded = NveHostEventBatch::decode(&bytes).expect("decode NVH1");
    assert_eq!(decoded, batch);
}

#[test]
fn nvh1_decoder_rejects_corrupt_truncated_and_oversized_batches() {
    assert!(matches!(
        NveHostEventBatch::decode(&[]),
        Err(HostBatchError::TruncatedHeader { actual: 0 })
    ));
    assert!(matches!(
        NveHostEventBatch::decode(b"BAD!01234567"),
        Err(HostBatchError::InvalidMagic(_))
    ));

    // Bad version
    let mut bad_ver = NveHostEventBatch::new().encode();
    bad_ver[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert!(matches!(
        NveHostEventBatch::decode(&bad_ver),
        Err(HostBatchError::UnsupportedVersion(2))
    ));

    // Crafted record_count = u32::MAX (anti-OOM check)
    let mut crafted = NveHostEventBatch::new().encode();
    crafted[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        NveHostEventBatch::decode(&crafted),
        Err(HostBatchError::RecordCountOverflow { .. })
    ));

    // Every prefix truncation of a valid multi-record batch fails cleanly without panicking.
    let mut full = NveHostEventBatch::new();
    full.push_async_response(7, NveHostStatus::Ok, vec![0xAA; 12]);
    full.push_host_event(SERVICE_NOTIFICATIONS, 1, vec![0xBB; 5]);
    full.push_channel_message(2, 0, vec![0xCC; 8]);
    let encoded = full.encode();
    for len in 0..encoded.len() {
        assert!(
            NveHostEventBatch::decode(&encoded[..len]).is_err(),
            "prefix length {len} must fail"
        );
    }

    // Trailing bytes rejected
    let mut with_trailing = encoded;
    with_trailing.push(0);
    assert!(matches!(
        NveHostEventBatch::decode(&with_trailing),
        Err(HostBatchError::TrailingBytes { remaining: 1 })
    ));
}
