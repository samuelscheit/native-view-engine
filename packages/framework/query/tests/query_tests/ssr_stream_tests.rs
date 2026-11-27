//! Slice 1c `NVEQ` v2 codec, raw error capture & sanitization, `DehydrateCursor` delta dedup,
//! monotonic fill-only hydration, `finish_ssr_stream` recovery, `wait_for_settled`, and zero-alloc
//! hydrated error read guards.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Waker};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use nve_query::{
    DehydrateCursor, DehydrateError, ManualClock, QueryClient, QueryClientConfig, QueryMeta,
    QueryMode, QueryOptions, Shared,
};
use nve_rpc::{
    BoxFuture, CborCodec, Procedure, RawCall, RawReply, ReplyKind, RpcCodec, RpcError, RpcStatus,
    Tag64, Transport, TransportError,
};

use crate::{block_on, DemoErr, GetItem, Item};

#[derive(Clone)]
pub(crate) struct ErrorMatrixTransport;

impl Transport for ErrorMatrixTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        Box::pin(async move {
            let id: u64 = CborCodec::default().decode_bounded(&req.body).expect("decode RPC args");
            match id {
                1 => {
                    let mut body = Vec::new();
                    CborCodec::default()
                        .encode_into(
                            &Item {
                                id: 1,
                                title: "Ok Item".into(),
                                closed: false,
                            },
                            &mut body,
                        )
                        .expect("encode CBOR");
                    Ok(RawReply {
                        http_status: 200,
                        kind: ReplyKind::Ok,
                        invalidated_tags: Vec::new(),
                        invalidate_all: false,
                        body: Bytes::from(body),
                    })
                }
                2 => {
                    let mut body = Vec::new();
                    CborCodec::default()
                        .encode_into(&DemoErr::Rejected, &mut body)
                        .expect("encode CBOR");
                    Ok(RawReply {
                        http_status: 403,
                        kind: ReplyKind::Domain,
                        invalidated_tags: Vec::new(),
                        invalidate_all: false,
                        body: Bytes::from(body),
                    })
                }
                3 => {
                    // 500 Internal with sensitive server details MUST be clamped to "internal".
                    let json = br#"{"code":"internal","message":"postgres://admin:secret@10.0.0.5/db panic"}"#;
                    Ok(RawReply {
                        http_status: 500,
                        kind: ReplyKind::Protocol,
                        invalidated_tags: Vec::new(),
                        invalidate_all: false,
                        body: Bytes::from_static(json),
                    })
                }
                4 => {
                    // 404 NotFound with >256-byte multi-byte UTF-8 message MUST truncate cleanly.
                    let long_msg = "é".repeat(200); // 400 bytes
                    let json = format!(r#"{{"code":"not_found","message":"{long_msg}"}}"#);
                    Ok(RawReply {
                        http_status: 404,
                        kind: ReplyKind::Protocol,
                        invalidated_tags: Vec::new(),
                        invalidate_all: false,
                        body: Bytes::from(json),
                    })
                }
                5 => Err(TransportError::timeout(
                    "upstream 10.1.2.3:5432 timed out after 30s",
                )),
                6 => Err(TransportError::network("dial tcp 192.168.1.9:443 refused")),
                _ => Err(TransportError::cancelled("client aborted")),
            }
        })
    }
}

pub(crate) struct WakeCount(pub(crate) AtomicUsize);
impl std::task::Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn nveq_v2_round_trips_all_four_kinds_sanitizes_errors_and_respects_grace() {
    assert_eq!(core::mem::size_of::<QueryMeta>(), 64);
    assert_eq!(core::mem::align_of::<QueryMeta>(), 64);

    let ssr_clock = Shared::new(ManualClock::new(10_000));
    let ssr = QueryClient::with_clock(
        ErrorMatrixTransport,
        CborCodec::default(),
        ssr_clock.clone(),
        QueryClientConfig {
            mode: QueryMode::Ssr,
            ..QueryClientConfig::default()
        },
    );

    let h_ok = ssr.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    let h_dom = ssr.subscribe::<GetItem>(&2, QueryOptions::default()).unwrap();
    let h_500 = ssr.subscribe::<GetItem>(&3, QueryOptions::default()).unwrap();
    let h_404 = ssr.subscribe::<GetItem>(&4, QueryOptions::default()).unwrap();
    let h_tmo = ssr.subscribe::<GetItem>(&5, QueryOptions::default()).unwrap();
    let h_net = ssr.subscribe::<GetItem>(&6, QueryOptions::default()).unwrap();
    let h_pending = ssr.subscribe::<GetItem>(&99, QueryOptions::default()).unwrap();

    // Plain QueryMode::Ssr never auto-fetches.
    assert!(!ssr.needs_fetch(h_ok));
    assert!(!ssr.needs_fetch(h_pending));

    assert!(block_on(ssr.fetch_handle::<GetItem>(h_ok)).is_ok());
    assert!(matches!(
        block_on(ssr.fetch_handle::<GetItem>(h_dom)).unwrap_err().as_ref(),
        RpcError::Domain {
            status: RpcStatus::PermissionDenied,
            error: DemoErr::Rejected
        }
    ));
    assert!(block_on(ssr.fetch_handle::<GetItem>(h_500)).is_err());
    assert!(block_on(ssr.fetch_handle::<GetItem>(h_404)).is_err());
    assert!(block_on(ssr.fetch_handle::<GetItem>(h_tmo)).is_err());
    assert!(block_on(ssr.fetch_handle::<GetItem>(h_net)).is_err());

    let settled_blob = ssr.dehydrate_bytes().unwrap();
    let pending_blob = ssr
        .dehydrate_initial_stream_chunk(&[h_ok, h_dom, h_pending], &mut DehydrateCursor::new())
        .unwrap();

    // Hydrate into a fresh Client with 5s grace window.
    let cli_clock = Shared::new(ManualClock::new(20_000));
    let cli = QueryClient::with_clock(
        ErrorMatrixTransport,
        CborCodec::default(),
        cli_clock.clone(),
        QueryClientConfig {
            hydration_grace_ms: 5_000,
            ..QueryClientConfig::default()
        },
    );

    assert_eq!(cli.hydrate_bytes(&settled_blob).unwrap(), 6);
    assert_eq!(cli.hydrate_bytes(&pending_blob).unwrap(), 1);

    let c_ok = cli.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    let c_dom = cli.subscribe::<GetItem>(&2, QueryOptions::default()).unwrap();
    let c_500 = cli.subscribe::<GetItem>(&3, QueryOptions::default()).unwrap();
    let c_404 = cli.subscribe::<GetItem>(&4, QueryOptions::default()).unwrap();
    let c_tmo = cli.subscribe::<GetItem>(&5, QueryOptions::default()).unwrap();
    let c_net = cli.subscribe::<GetItem>(&6, QueryOptions::default()).unwrap();
    let c_pending = cli.subscribe::<GetItem>(&99, QueryOptions::default()).unwrap();

    // 1. Ok entry decoded lazily & fresh during grace.
    let snap_ok = cli.read::<GetItem>(c_ok);
    assert_eq!(snap_ok.data.as_ref().unwrap().title, "Ok Item");
    assert!(!snap_ok.is_stale && !cli.needs_fetch(c_ok));

    // 2. DomainErr decoded lazily & NOT stale on Frame 0 (hydration grace applies to errors!).
    let snap_dom = cli.read::<GetItem>(c_dom);
    assert!(matches!(
        snap_dom.error.as_deref(),
        Some(RpcError::Domain {
            status: RpcStatus::PermissionDenied,
            error: DemoErr::Rejected
        })
    ));
    assert!(!snap_dom.is_stale && !cli.needs_fetch(c_dom));

    // 3. 500 Internal message strictly clamped to "internal" (zero secret leakage!).
    let snap_500 = cli.read::<GetItem>(c_500);
    assert_eq!(
        snap_500.error.as_deref(),
        Some(&RpcError::Protocol {
            status: RpcStatus::Internal,
            message: "internal".to_owned(),
        })
    );
    assert!(!snap_500.is_stale && !cli.needs_fetch(c_500));

    // 4. 404 message preserved and capped at <= 256 valid UTF-8 bytes.
    let snap_404 = cli.read::<GetItem>(c_404);
    match snap_404.error.as_deref() {
        Some(RpcError::Protocol {
            status: RpcStatus::NotFound,
            message,
        }) => {
            assert!(message.len() <= 256);
            assert_eq!(message.len(), 256); // 128 x 2-byte 'é'
        }
        other => panic!("expected capped 404 Protocol error, got {other:?}"),
    }

    // 5. Transport errors sanitized strictly to canonical status.code_str().
    assert_eq!(
        cli.read::<GetItem>(c_tmo).error.as_deref(),
        Some(&RpcError::Protocol {
            status: RpcStatus::RequestTimeout,
            message: "deadline_exceeded".to_owned(),
        })
    );
    assert_eq!(
        cli.read::<GetItem>(c_net).error.as_deref(),
        Some(&RpcError::Protocol {
            status: RpcStatus::Unavailable,
            message: "unavailable".to_owned(),
        })
    );

    // 6. StreamPending entry marks IS_FETCHING | STREAM_PENDING without bumping completed_seq.
    let snap_pend = cli.read::<GetItem>(c_pending);
    assert!(snap_pend.is_fetching && snap_pend.is_stream_pending);
    assert_eq!(snap_pend.completed_seq, 0);
    assert!(!cli.needs_fetch(c_pending));

    // 7. Zero heap allocations on cached reads for BOTH hydrated Ok and hydrated Errors!
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (_, ok_allocs) = crate::count_allocs(|| cli.read::<GetItem>(c_ok));
        let (_, dom_allocs) = crate::count_allocs(|| cli.read::<GetItem>(c_dom));
        let (_, proto_allocs) = crate::count_allocs(|| cli.read::<GetItem>(c_500));
        assert_eq!(ok_allocs, 0, "cached hydrated Ok read must allocate 0 times");
        assert_eq!(dom_allocs, 0, "cached hydrated DomainErr read must allocate 0 times");
        assert_eq!(
            proto_allocs, 0,
            "cached hydrated ProtocolErr read must allocate 0 times"
        );
    }

    // 8. After hydration_grace_ms elapses, hydrated errors expire cleanly too.
    cli_clock.advance(5_001);
    assert!(cli.read::<GetItem>(c_dom).is_stale);
    assert!(cli.needs_fetch(c_dom));
    assert!(cli.needs_fetch(c_500));
}

#[test]
fn nveq_v2_rejects_reserved_flags_invalid_status_oversize_msg_and_bad_utf8() {
    let cli = QueryClient::new(ErrorMatrixTransport);

    // Helper: build 1-entry v2 header + prefix with custom flags + payload tail.
    let make_entry = |flags: u8, tail: &[u8]| -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"NVEQ");
        buf.push(2); // version 2
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&GetItem::ID.raw().to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // age_ms
        buf.push(flags);
        buf.extend_from_slice(&0u16.to_le_bytes()); // tag_count
        buf.extend_from_slice(&0u32.to_le_bytes()); // args_len
        buf.extend_from_slice(tail);
        buf
    };

    // Reserved bit 3 set (0x08) -> InvalidFlags.
    assert_eq!(
        cli.hydrate_bytes(&make_entry(0x08, &0u32.to_le_bytes())),
        Err(DehydrateError::InvalidFlags(0x08))
    );
    // Reserved bit 7 set (0x80) -> InvalidFlags.
    assert_eq!(
        cli.hydrate_bytes(&make_entry(0x80, &[])),
        Err(DehydrateError::InvalidFlags(0x80))
    );

    // DomainErr (kind=1 -> flags=0b010) with status=0 (Ok) -> InvalidStatus(0).
    let mut dom_bad_status = vec![0u8];
    dom_bad_status.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        cli.hydrate_bytes(&make_entry(0b0000_0010, &dom_bad_status)),
        Err(DehydrateError::InvalidStatus(0))
    );

    // ProtocolErr (kind=2 -> flags=0b100) with out-of-range status=99 -> InvalidStatus(99).
    let mut proto_bad_status = vec![99u8];
    proto_bad_status.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        cli.hydrate_bytes(&make_entry(0b0000_0100, &proto_bad_status)),
        Err(DehydrateError::InvalidStatus(99))
    );

    // ProtocolErr with msg_len = 257 (> 256 cap) -> TooLarge(257).
    let mut proto_too_big = vec![RpcStatus::NotFound as u8];
    proto_too_big.extend_from_slice(&257u32.to_le_bytes());
    proto_too_big.extend_from_slice(&[b'a'; 257]);
    assert_eq!(
        cli.hydrate_bytes(&make_entry(0b0000_0100, &proto_too_big)),
        Err(DehydrateError::TooLarge(257))
    );

    // ProtocolErr with invalid UTF-8 bytes -> InvalidUtf8.
    let mut proto_bad_utf8 = vec![RpcStatus::NotFound as u8];
    proto_bad_utf8.extend_from_slice(&2u32.to_le_bytes());
    proto_bad_utf8.extend_from_slice(&[0xFF, 0xFE]);
    assert_eq!(
        cli.hydrate_bytes(&make_entry(0b0000_0100, &proto_bad_utf8)),
        Err(DehydrateError::InvalidUtf8)
    );
}

#[test]
fn dehydrate_cursor_deduplicates_shared_handles_and_re_emits_new_revisions() {
    let ssr = QueryClient::ssr(ErrorMatrixTransport);
    let h1 = ssr.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    let h2 = ssr.subscribe::<GetItem>(&2, QueryOptions::default()).unwrap();
    let h3 = ssr.subscribe::<GetItem>(&3, QueryOptions::default()).unwrap();
    let h_unsettled = ssr
        .subscribe::<GetItem>(&99, QueryOptions::default())
        .unwrap();

    assert!(block_on(ssr.fetch_handle::<GetItem>(h1)).is_ok());
    assert!(block_on(ssr.fetch_handle::<GetItem>(h2)).is_err());
    assert!(block_on(ssr.fetch_handle::<GetItem>(h3)).is_err());

    let mut cursor = DehydrateCursor::new();
    // Boundary 1 owns [h1, h2, h1(dup), h_unsettled] -> emits ONLY settled unique [h1, h2].
    let chunk1 = ssr
        .dehydrate_handles_delta(&[h1, h2, h1, h_unsettled], &mut cursor)
        .unwrap();
    assert_eq!(cursor.len(), 2);

    // Boundary 2 shares h1 and owns h3 -> emits ONLY h3 (zero duplicate h1!).
    let chunk2 = ssr.dehydrate_handles_delta(&[h1, h3], &mut cursor).unwrap();
    assert_eq!(cursor.len(), 3);

    // Re-calling with already-emitted handles emits 0 entries (9-byte empty NVEQ v2 header).
    let empty_chunk = ssr.dehydrate_handles_delta(&[h1, h2, h3], &mut cursor).unwrap();
    assert_eq!(empty_chunk.len(), 9);

    let cli = QueryClient::new(ErrorMatrixTransport);
    assert_eq!(cli.hydrate_stream_chunk_bytes(&chunk1).unwrap(), 2);
    assert_eq!(cli.hydrate_stream_chunk_bytes(&chunk2).unwrap(), 1);
    assert_eq!(cli.hydrate_stream_chunk_bytes(&empty_chunk).unwrap(), 0);

    // Updating h1 advances completed_seq so the same cursor emits the new revision.
    ssr.set_query_data::<GetItem>(
        &1,
        Item {
            id: 1,
            title: "Revised".into(),
            closed: true,
        },
    )
    .unwrap();
    let revised_chunk = ssr.dehydrate_handles_delta(&[h1], &mut cursor).unwrap();
    assert!(revised_chunk.len() > 9);
    let fresh_cli = QueryClient::new(ErrorMatrixTransport);
    assert_eq!(fresh_cli.hydrate_bytes(&revised_chunk).unwrap(), 1);
    let ch1 = fresh_cli
        .subscribe::<GetItem>(&1, QueryOptions::default())
        .unwrap();
    assert_eq!(fresh_cli.read::<GetItem>(ch1).data.unwrap().title, "Revised");
}

#[test]
fn fill_only_hydration_monotonicity_wait_for_settled_and_finish_ssr_stream_work() {
    let ssr = QueryClient::ssr(ErrorMatrixTransport);
    let h1 = ssr.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    let h2 = ssr.subscribe::<GetItem>(&2, QueryOptions::default()).unwrap();
    let h3 = ssr.subscribe::<GetItem>(&3, QueryOptions::default()).unwrap();

    // Boot chunk marks h1, h2, h3 StreamPending.
    let pending_blob = ssr.dehydrate_initial_stream_chunk(&[h1, h2, h3], &mut DehydrateCursor::new()).unwrap();
    // Server settles h1 (Ok) and h2 (DomainErr).
    assert!(block_on(ssr.fetch_handle::<GetItem>(h1)).is_ok());
    assert!(block_on(ssr.fetch_handle::<GetItem>(h2)).is_err());
    let mut cursor = DehydrateCursor::new();
    let chunk_h1 = ssr.dehydrate_handles_delta(&[h1], &mut cursor).unwrap();
    let chunk_h2 = ssr.dehydrate_handles_delta(&[h2], &mut cursor).unwrap();

    let cli = QueryClient::new(ErrorMatrixTransport);
    assert_eq!(cli.hydrate_bytes(&pending_blob).unwrap(), 3);

    let c1 = cli.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    let c2 = cli.subscribe::<GetItem>(&2, QueryOptions::default()).unwrap();
    let c3 = cli.subscribe::<GetItem>(&3, QueryOptions::default()).unwrap();

    // 1. `wait_for_settled` suspends cleanly on StreamPending and wakes when chunk arrives!
    let wake_c1 = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker_c1 = Waker::from(Arc::clone(&wake_c1));
    let mut fut_c1 = Box::pin(cli.wait_for_settled(c1));
    assert!(Pin::as_mut(&mut fut_c1)
        .poll(&mut Context::from_waker(&waker_c1))
        .is_pending());

    assert_eq!(cli.hydrate_stream_chunk_bytes(&chunk_h1).unwrap(), 1);
    assert_eq!(wake_c1.0.load(Ordering::SeqCst), 1);
    assert!(Pin::as_mut(&mut fut_c1)
        .poll(&mut Context::from_waker(&waker_c1))
        .is_ready());
    let snap1 = cli.read::<GetItem>(c1);
    assert_eq!(snap1.data.as_ref().unwrap().title, "Ok Item");
    assert!(!snap1.is_fetching && !snap1.is_stream_pending && snap1.completed_seq == 1);

    // 2. Fill-only guard: replaying or overwriting a committed slot is skipped (0 applied)!
    ssr.set_query_data::<GetItem>(
        &1,
        Item {
            id: 1,
            title: "Stale Overwrite".into(),
            closed: true,
        },
    )
    .unwrap();
    let overwrite_chunk = ssr.dehydrate_handles_delta(&[h1], &mut cursor).unwrap();
    assert_eq!(cli.hydrate_stream_chunk_bytes(&overwrite_chunk).unwrap(), 0);
    assert_eq!(cli.read::<GetItem>(c1).data.unwrap().title, "Ok Item");

    // 3. Local tag invalidation on a StreamPending slot clears FLAG_STREAM_PENDING and blocks
    //    late SSR chunks!
    let _ = cli.invalidate_tags(&[Tag64::keyed("item", 2)]);
    let snap2_inv = cli.read::<GetItem>(c2);
    assert!(!snap2_inv.is_stream_pending && !snap2_inv.is_fetching && snap2_inv.is_invalidated);
    assert_eq!(cli.hydrate_stream_chunk_bytes(&chunk_h2).unwrap(), 0);
    assert!(cli.read::<GetItem>(c2).error.is_none());

    // 4. `finish_ssr_stream()` wakes remaining StreamPending waiters without advancing completed_seq!
    let wake_c3 = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker_c3 = Waker::from(Arc::clone(&wake_c3));
    let mut fut_c3 = Box::pin(cli.wait_for_settled(c3));
    assert!(Pin::as_mut(&mut fut_c3)
        .poll(&mut Context::from_waker(&waker_c3))
        .is_pending());

    assert_eq!(cli.finish_ssr_stream(), 1);
    assert_eq!(wake_c3.0.load(Ordering::SeqCst), 1);
    assert!(Pin::as_mut(&mut fut_c3)
        .poll(&mut Context::from_waker(&waker_c3))
        .is_ready());
    let snap3 = cli.read::<GetItem>(c3);
    assert!(!snap3.is_stream_pending && !snap3.is_fetching);
    assert_eq!(snap3.completed_seq, 0);
    assert!(cli.needs_fetch(c3));

    // 5. After `invalidate_all()` (`wildcard_epoch > 0`), `hydrate_stream_chunk_bytes` refuses
    //    absent slots AND inactive StreamPending slots reject late chunks & clear flags on mount.
    let cli2 = QueryClient::new(ErrorMatrixTransport);
    assert_eq!(cli2.hydrate_bytes(&pending_blob).unwrap(), 3); // all 3 inactive!
    let _ = cli2.invalidate_all();
    assert_eq!(cli2.hydrate_stream_chunk_bytes(&chunk_h1).unwrap(), 0);
    let h4 = ssr.subscribe::<GetItem>(&4, QueryOptions::default()).unwrap();
    assert!(block_on(ssr.fetch_handle::<GetItem>(h4)).is_err());
    let chunk_h4 = ssr.dehydrate_handles_delta(&[h4], &mut cursor).unwrap();
    assert_eq!(
        cli2.hydrate_stream_chunk_bytes(&chunk_h4).unwrap(),
        0,
        "absent slot must not hydrate after invalidate_all()"
    );
    let mounted_after_wild = cli2
        .subscribe::<GetItem>(&1, QueryOptions::default())
        .unwrap();
    let snap_wild = cli2.read::<GetItem>(mounted_after_wild);
    assert!(
        !snap_wild.is_stream_pending && !snap_wild.is_fetching && cli2.needs_fetch(mounted_after_wild),
        "inactive StreamPending slot invalidated by invalidate_all must clear flags on subscribe"
    );
}
