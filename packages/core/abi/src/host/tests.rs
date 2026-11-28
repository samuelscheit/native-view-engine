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

#[test]
fn host_event_batch_roundtrips_response_push_channel_with_8_byte_alignment() {
    let mut b = NveHostEventBatchBuilder::new();
    assert!(b.is_empty());
    b.push_response(SERVICE_CLIPBOARD, 1, 42, NveHostStatus::Ok, b"hello")
        .expect("push response");
    b.push_push(SERVICE_SYSTEM_ENV, 1, &[1u8; 12])
        .expect("push event");
    b.push_channel(99, true, b"").expect("push channel");
    b.push_response(SERVICE_LINKING, 2, 77, NveHostStatus::Cancelled, b"")
        .expect("push cancelled response");

    let bytes = b.into_bytes();
    assert_eq!(bytes.len() % NVE_HOST_BATCH_ALIGN, 0);

    let parsed = NveHostEventBatchRef::parse(&bytes).expect("parse valid batch");
    assert_eq!(parsed.count(), 4);
    let recs: Vec<NveHostEventRecord<'_>> = parsed.iter().collect();
    assert_eq!(recs.len(), 4);

    assert_eq!(recs[0].kind, NveHostEventKind::Response);
    assert_eq!(recs[0].service_id, SERVICE_CLIPBOARD);
    assert_eq!(recs[0].method_or_event_or_channel, 1);
    assert_eq!(recs[0].request_id, 42);
    assert_eq!(recs[0].status, NveHostStatus::Ok);
    assert_eq!(recs[0].payload, b"hello");

    assert_eq!(recs[1].kind, NveHostEventKind::Push);
    assert_eq!(recs[1].service_id, SERVICE_SYSTEM_ENV);
    assert_eq!(recs[1].method_or_event_or_channel, 1);
    assert_eq!(recs[1].payload, &[1u8; 12]);

    assert_eq!(recs[2].kind, NveHostEventKind::Channel);
    assert_eq!(recs[2].service_id, SERVICE_CHANNEL);
    assert_eq!(recs[2].method_or_event_or_channel, 99);
    assert!(recs[2].is_retained());
    assert!(recs[2].payload.is_empty());

    assert_eq!(recs[3].kind, NveHostEventKind::Response);
    assert_eq!(recs[3].status, NveHostStatus::Cancelled);
    assert_eq!(recs[3].request_id, 77);
}

#[test]
fn host_event_batch_stack_single_push_and_malformed_fuzz_rejection() {
    let payload = [0x11u8; 12];
    let mut stack = [0u8; 56];
    let encoded = encode_single_push_into(&mut stack, SERVICE_SYSTEM_ENV, 1, &payload)
        .expect("encode 12-byte push into 56-byte stack buffer");
    assert_eq!(encoded.len(), 56);
    let parsed = NveHostEventBatchRef::parse(encoded).expect("parse stack batch");
    assert_eq!(parsed.count(), 1);
    let rec = parsed.iter().next().expect("one record");
    assert_eq!(rec.kind, NveHostEventKind::Push);
    assert_eq!(rec.service_id, SERVICE_SYSTEM_ENV);
    assert_eq!(rec.method_or_event_or_channel, 1);
    assert_eq!(rec.payload, &payload);

    // Reject too-short, bad magic, bad version, trailing bytes, invalid kind, truncated payloads.
    assert_eq!(
        NveHostEventBatchRef::parse(&[]),
        Err(NveHostBatchError::TooShort)
    );
    assert_eq!(
        NveHostEventBatchRef::parse(&[0u8; 12]),
        Err(NveHostBatchError::BadMagic)
    );
    let mut bad_ver = stack;
    bad_ver[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        NveHostEventBatchRef::parse(&bad_ver),
        Err(NveHostBatchError::UnsupportedVersion(2))
    );
    let mut bad_kind = stack;
    bad_kind[8] = 9;
    assert_eq!(
        NveHostEventBatchRef::parse(&bad_kind),
        Err(NveHostBatchError::InvalidKind { index: 0, raw: 9 })
    );
    let mut with_trailing = stack.to_vec();
    with_trailing.push(0);
    assert_eq!(
        NveHostEventBatchRef::parse(&with_trailing),
        Err(NveHostBatchError::TrailingBytes(1))
    );
    for trunc in 0..stack.len() {
        assert!(
            NveHostEventBatchRef::parse(&stack[..trunc]).is_err(),
            "truncated len {trunc} must be rejected"
        );
    }

    // Deterministic fuzz sweep over pseudo-random mutated headers/payloads: must never panic.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut fuzz_buf = [0u8; 128];
    for case in 0..512u32 {
        let len = (case as usize) % fuzz_buf.len();
        for b in &mut fuzz_buf[..len] {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 56) as u8;
        }
        if case % 3 == 0 && len >= 8 {
            fuzz_buf[0..4].copy_from_slice(&NVE_HOST_BATCH_MAGIC);
            fuzz_buf[4..6].copy_from_slice(&NVE_HOST_BATCH_VERSION.to_le_bytes());
        }
        if let Ok(valid) = NveHostEventBatchRef::parse(&fuzz_buf[..len]) {
            assert_eq!(valid.iter().count(), usize::from(valid.count()));
        }
    }
}
