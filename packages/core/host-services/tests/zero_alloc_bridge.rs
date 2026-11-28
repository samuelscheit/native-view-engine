//! `CountingAlloc` zero-allocation synchronous C-ABI bridge guard + canonical `NveHostEventBatch`
//! ingestion, exactly-once `Pending` resolution across unbind/reload/drop, and drop-ordering tests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{
    encode_single_push_into, NveHostEventBatchBuilder, NveHostEventBatchRef, NveHostStatus,
    SERVICE_CLIPBOARD, SERVICE_SYSTEM_ENV,
};
use nve_host_services::{
    CompletionSink, FfiHostServices, HandlerReply, HostCallbackBridge, HostPushEvent,
    HostServiceHandler, HostServiceProvider, HostServiceRegistry, InvokeResult, ResponseBuf,
    ServiceError,
};
use nve_interop::NveChannelBus;

struct CountingAlloc;

thread_local! {
    static TRACK_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static THREAD_ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// SAFETY: Delegates directly to `System` while counting heap allocations on the tracked thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK_THREAD.try_with(|flag| {
            if flag.get() {
                let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get() + 1));
            }
        });
        // SAFETY: Forwarded unchanged to the global `System` allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` came from `System::alloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_thread_allocs<F: FnOnce()>(f: F) -> usize {
    THREAD_ALLOCS.with(|c| c.set(0));
    TRACK_THREAD.with(|flag| flag.set(true));
    f();
    TRACK_THREAD.with(|flag| flag.set(false));
    THREAD_ALLOCS.with(|c| c.get())
}

struct InlineSyncHandler;

impl HostServiceHandler for InlineSyncHandler {
    fn service_id(&self) -> u32 {
        SERVICE_CLIPBOARD
    }

    fn handle(
        &self,
        method_id: u32,
        _request_id: u64,
        request: &[u8],
        _done: &CompletionSink,
    ) -> Result<HandlerReply, ServiceError> {
        match method_id {
            1 => Ok(HandlerReply::Ready(ResponseBuf::from_slice_or_vec(request))),
            _ => Err(ServiceError::Unsupported),
        }
    }
}

struct AsyncPendingHandler {
    pending_ids: Mutex<Vec<u64>>,
}

impl AsyncPendingHandler {
    fn new() -> Self {
        Self {
            pending_ids: Mutex::new(Vec::new()),
        }
    }
}

impl HostServiceHandler for AsyncPendingHandler {
    fn service_id(&self) -> u32 {
        SERVICE_CLIPBOARD
    }

    fn handle(
        &self,
        method_id: u32,
        request_id: u64,
        _request: &[u8],
        _done: &CompletionSink,
    ) -> Result<HandlerReply, ServiceError> {
        match method_id {
            1 => {
                self.pending_ids
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(request_id);
                Ok(HandlerReply::Pending)
            }
            _ => Err(ServiceError::Unsupported),
        }
    }
}

#[test]
fn sync_host_callback_bridge_and_stack_batch_encode_are_zero_alloc() {
    let reg = HostServiceRegistry::new().with_handler(InlineSyncHandler);
    let bridge = HostCallbackBridge::new(reg);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());

    let req = b"zero-alloc-c-abi-payload";
    let allocs = count_thread_allocs(|| {
        for _ in 0..32 {
            let resp = ffi
                .invoke_sync(SERVICE_CLIPBOARD, 1, req)
                .expect("sync bridge invoke");
            assert!(resp.is_inline());
            assert_eq!(resp.as_slice(), req);
        }
        let mut stack = [0u8; 56];
        let theme = [1u8; 12];
        for _ in 0..32 {
            let encoded = encode_single_push_into(&mut stack, SERVICE_SYSTEM_ENV, 1, &theme)
                .expect("stack encode");
            let parsed = NveHostEventBatchRef::parse(encoded).expect("zero-alloc parse");
            assert_eq!(parsed.count(), 1);
            let rec = parsed.iter().next().expect("record");
            assert_eq!(rec.payload, &theme);
        }
    });
    assert_eq!(
        allocs,
        0,
        "sync C-ABI bridge round-trip and stack batch encode/parse must perform 0 heap allocs"
    );
}

#[test]
fn ingest_batch_routes_response_push_and_channel_with_exactly_once_semantics() {
    let async_handler = Arc::new(AsyncPendingHandler::new());
    let reg = Arc::new(HostServiceRegistry::new().with_handler_arc(async_handler.clone()));
    let bridge = HostCallbackBridge::new(Arc::clone(&reg));
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());

    let channel_bus = Arc::new(Mutex::new(NveChannelBus::new()));
    let chan_id = {
        let mut bus = channel_bus.lock().unwrap();
        bus.ensure_channel("host.test").unwrap()
    };
    let received_chan = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let rx_chan = Arc::clone(&received_chan);
    channel_bus
        .lock()
        .unwrap()
        .subscribe_by_id(chan_id, None, false, move |msg| {
            rx_chan.lock().unwrap().push(msg.as_bytes().to_vec());
        })
        .unwrap();
    ffi.attach_channel_bus(Arc::clone(&channel_bus));

    // Fire an async Pending call over C ABI bridge; callback must fire EXACTLY ONCE.
    let cb_fires = Arc::new(AtomicUsize::new(0));
    let cb_payload = Arc::new(Mutex::new(None));
    let fires_c = Arc::clone(&cb_fires);
    let payload_c = Arc::clone(&cb_payload);
    let req_id = ffi
        .invoke_with_callback(SERVICE_CLIPBOARD, 1, &[], move |res| {
            fires_c.fetch_add(1, Ordering::SeqCst);
            *payload_c.lock().unwrap() = Some(res);
        })
        .expect("invoke_with_callback ok")
        .expect("must return Pending req_id");
    assert_eq!(reg.completion_sink().in_flight_count(), 1);
    assert_eq!(ffi.pending_request_count(), 1);

    // Complete request twice on host side: second completion MUST be rejected (at-most-once).
    assert!(reg.completion_sink().complete_ok(req_id, b"clipboard-async".to_vec()));
    assert!(!reg.completion_sink().complete_ok(req_id, b"duplicate-ignored".to_vec()));
    reg.completion_sink()
        .push_event(SERVICE_SYSTEM_ENV, 1, vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    reg.completion_sink()
        .push_channel(chan_id, true, b"chan-bytes".to_vec());

    let batch = reg.drain_host_event_batch().expect("non-empty batch");
    let stats = ffi.ingest_batch(&batch).expect("ingest valid batch");
    assert_eq!(stats.responses, 1);
    assert_eq!(stats.pushes, 1);
    assert_eq!(stats.channels, 1);
    assert_eq!(cb_fires.load(Ordering::SeqCst), 1);
    assert_eq!(
        *cb_payload.lock().unwrap(),
        Some(Ok(b"clipboard-async".to_vec()))
    );
    assert_eq!(ffi.pending_request_count(), 0);

    // Re-ingesting a forged duplicate Response for the same req_id MUST NOT double-fire.
    let mut dup_builder = NveHostEventBatchBuilder::new();
    dup_builder
        .push_response(SERVICE_CLIPBOARD, 1, req_id, NveHostStatus::Ok, b"forged")
        .unwrap();
    let _ = ffi.ingest_batch(&dup_builder.into_bytes()).unwrap();
    assert_eq!(cb_fires.load(Ordering::SeqCst), 1);
    assert_eq!(ffi.take_async_response(req_id), None);

    // Push event landed on TypedEventBus as HostPushEvent.
    let pushes = ffi.typed_events().drain_typed::<HostPushEvent>();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes[0].service_id, SERVICE_SYSTEM_ENV);
    assert_eq!(pushes[0].event_id, 1);

    // Channel message landed on NveChannelBus.
    assert_eq!(
        received_chan.lock().unwrap().as_slice(),
        &[b"chan-bytes".to_vec()]
    );
}

#[test]
fn pending_requests_resolve_as_cancelled_exactly_once_on_unbind_reload_and_drop() {
    let reg = Arc::new(HostServiceRegistry::new().with_handler(AsyncPendingHandler::new()));
    let bridge = HostCallbackBridge::new(Arc::clone(&reg));
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());

    // 1. Unbind cancels both callback-based and polled Pending requests cleanly.
    let cb_res = Arc::new(Mutex::new(Vec::<Result<Vec<u8>, ServiceError>>::new()));
    let cb_clone = Arc::clone(&cb_res);
    let _req_cb = ffi
        .invoke_with_callback(SERVICE_CLIPBOARD, 1, &[], move |res| {
            cb_clone.lock().unwrap().push(res);
        })
        .unwrap()
        .expect("pending cb");
    let InvokeResult::Pending {
        request_id: req_poll,
    } = ffi.invoke_raw(SERVICE_CLIPBOARD, 1, &[]).unwrap()
    else {
        panic!("expected Pending");
    };
    assert_eq!(ffi.pending_request_count(), 2);

    // Host-side cancel flushes Cancelled batch for all in-flight entries.
    assert_eq!(reg.cancel_pending(), 2);
    assert_eq!(reg.completion_sink().in_flight_count(), 0);
    let cancel_batch = reg.drain_host_event_batch().expect("cancel batch");
    let _ = ffi.ingest_batch(&cancel_batch).unwrap();

    assert_eq!(
        cb_res.lock().unwrap().as_slice(),
        &[Err(ServiceError::Cancelled)]
    );
    assert_eq!(
        ffi.take_async_response(req_poll),
        Some(Err(ServiceError::Cancelled))
    );
    assert_eq!(ffi.pending_request_count(), 0);

    // 2. Unbind directly on FfiHostServices (hot-reload / teardown without host cancel batch)
    // still resolves pending callbacks as Cancelled exactly once.
    let unbind_res = Arc::new(Mutex::new(Vec::<Result<Vec<u8>, ServiceError>>::new()));
    let u_clone = Arc::clone(&unbind_res);
    let _ = ffi
        .invoke_with_callback(SERVICE_CLIPBOARD, 1, &[], move |res| {
            u_clone.lock().unwrap().push(res);
        })
        .unwrap()
        .expect("pending before unbind");
    ffi.bind(None);
    assert_eq!(
        unbind_res.lock().unwrap().as_slice(),
        &[Err(ServiceError::Cancelled)]
    );
    assert_eq!(ffi.pending_request_count(), 0);

    // 3. Dropping FfiHostServices while a request is Pending cancels its callback cleanly.
    let drop_res = Arc::new(Mutex::new(Vec::<Result<Vec<u8>, ServiceError>>::new()));
    let d_clone = Arc::clone(&drop_res);
    {
        let temp_ffi = FfiHostServices::with_callbacks(bridge.callbacks());
        let _ = temp_ffi
            .invoke_with_callback(SERVICE_CLIPBOARD, 1, &[], move |res| {
                d_clone.lock().unwrap().push(res);
            })
            .unwrap()
            .expect("pending before drop");
    }
    assert_eq!(
        drop_res.lock().unwrap().as_slice(),
        &[Err(ServiceError::Cancelled)]
    );
}

#[test]
fn duplicate_request_id_rejected_and_distinct_ffi_clients_do_not_collide() {
    let calls = Arc::new(AtomicUsize::new(0));
    struct CountingPending(Arc<AtomicUsize>);
    impl HostServiceHandler for CountingPending {
        fn service_id(&self) -> u32 {
            SERVICE_CLIPBOARD
        }
        fn handle(
            &self,
            _method_id: u32,
            _request_id: u64,
            _request: &[u8],
            _done: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(HandlerReply::Pending)
        }
    }

    let reg = Arc::new(HostServiceRegistry::new().with_handler(CountingPending(Arc::clone(&calls))));
    // First Pending call with id=42 succeeds; duplicate id=42 is rejected WITHOUT invoking handler.
    assert_eq!(
        reg.dispatch_with_id(SERVICE_CLIPBOARD, 1, 42, &[]).unwrap(),
        HandlerReply::Pending
    );
    assert!(matches!(
        reg.dispatch_with_id(SERVICE_CLIPBOARD, 1, 42, &[]),
        Err(ServiceError::InvalidArg(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(reg.completion_sink().in_flight_count(), 1);
    assert!(reg.completion_sink().complete_ok(42, b"first-only".to_vec()));
    assert!(!reg.completion_sink().complete_ok(42, b"second".to_vec()));
    assert_eq!(
        reg.take_async_response(42),
        Some(Ok(b"first-only".to_vec()))
    );

    // Two independent FfiHostServices sharing one HostServiceRegistry allocate distinct IDs.
    let bridge = HostCallbackBridge::new(Arc::clone(&reg));
    let ffi_a = FfiHostServices::with_callbacks(bridge.callbacks());
    let ffi_b = FfiHostServices::with_callbacks(bridge.callbacks());
    let InvokeResult::Pending { request_id: id_a } =
        ffi_a.invoke_raw(SERVICE_CLIPBOARD, 1, &[]).unwrap()
    else {
        panic!("expected Pending A");
    };
    let InvokeResult::Pending { request_id: id_b } =
        ffi_b.invoke_raw(SERVICE_CLIPBOARD, 1, &[]).unwrap()
    else {
        panic!("expected Pending B");
    };
    assert_ne!(id_a, id_b, "separate FfiHostServices clients must not collide on request_id");
    assert_eq!(reg.completion_sink().in_flight_count(), 2);
}

#[test]
fn oversized_records_never_stall_queue_and_partial_drain_rearms_wake() {
    let sink = CompletionSink::new();
    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_c = Arc::clone(&wakes);
    sink.set_on_ready(Some(Arc::new(move || {
        wakes_c.fetch_add(1, Ordering::SeqCst);
    })));

    let oversized_len = nve_abi::NVE_HOST_BATCH_MAX_PAYLOAD + 1;
    assert!(sink.register_in_flight(1, SERVICE_CLIPBOARD, 1));
    assert!(sink.register_in_flight(2, SERVICE_CLIPBOARD, 1));
    assert!(sink.register_in_flight(3, SERVICE_CLIPBOARD, 1));

    // Sequence: good -> oversized response -> oversized push -> oversized channel -> good.
    assert!(sink.complete_ok(1, b"good-1".to_vec()));
    assert!(sink.complete_ok(2, vec![0xAA; oversized_len]));
    sink.push_event(SERVICE_SYSTEM_ENV, 1, vec![0xBB; oversized_len]);
    sink.push_channel(9, false, vec![0xCC; oversized_len]);
    assert!(sink.complete_ok(3, b"good-3".to_vec()));

    assert_eq!(sink.dropped_oversized_count(), 3);
    let bytes = sink.drain_batch().expect("non-empty drained batch");
    assert_eq!(sink.queued_count(), 0, "queue must be completely empty after drain");

    let parsed = NveHostEventBatchRef::parse(&bytes).expect("valid batch");
    let recs: Vec<_> = parsed.iter().collect();
    assert_eq!(recs.len(), 3);
    assert_eq!((recs[0].request_id, recs[0].status, recs[0].payload), (1, NveHostStatus::Ok, b"good-1".as_slice()));
    assert_eq!((recs[1].request_id, recs[1].status, recs[1].payload), (2, NveHostStatus::InternalError, b"".as_slice()));
    assert_eq!((recs[2].request_id, recs[2].status, recs[2].payload), (3, NveHostStatus::Ok, b"good-3".as_slice()));

    // Verify u16::MAX cap stops cleanly without dropping records and re-arms on_ready!
    let large_sink = CompletionSink::with_max_queued_records(usize::from(u16::MAX) + 4);
    let mut prefilled = NveHostEventBatchBuilder::with_capacity(64);
    for i in 0..(usize::from(u16::MAX) + 2) {
        large_sink.push_event(SERVICE_SYSTEM_ENV, i as u32, Vec::new());
    }
    let rearm_wakes = Arc::new(AtomicUsize::new(0));
    let rw = Arc::clone(&rearm_wakes);
    large_sink.set_on_ready(Some(Arc::new(move || {
        rw.fetch_add(1, Ordering::SeqCst);
    })));
    let first = large_sink.drain_batch_into(&mut prefilled).expect("drain max");
    assert_eq!(first, usize::from(u16::MAX));
    assert_eq!(large_sink.queued_count(), 2);
    assert_eq!(rearm_wakes.load(Ordering::SeqCst), 1, "remaining records must re-fire on_ready");
    prefilled.clear();
    let second = large_sink.drain_batch_into(&mut prefilled).expect("drain rest");
    assert_eq!(second, 2);
    assert_eq!(large_sink.queued_count(), 0);
}

#[test]
fn panicking_handler_across_c_abi_returns_internal_error_and_cleans_in_flight() {
    struct PanickingHandler;
    impl HostServiceHandler for PanickingHandler {
        fn service_id(&self) -> u32 {
            SERVICE_CLIPBOARD
        }
        fn handle(
            &self,
            _method_id: u32,
            _request_id: u64,
            _request: &[u8],
            _done: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            panic!("simulated handler panic across extern C");
        }
    }

    let reg = Arc::new(HostServiceRegistry::new().with_handler(PanickingHandler));
    let bridge = HostCallbackBridge::new(Arc::clone(&reg));
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());

    let res = ffi.invoke_sync(SERVICE_CLIPBOARD, 1, b"boom");
    assert!(
        matches!(res, Err(ServiceError::InternalError(_))),
        "panic across extern C must convert to InternalError, got {res:?}"
    );
    assert_eq!(
        reg.completion_sink().in_flight_count(),
        0,
        "InFlightGuard must clean up in_flight slot when handler panics"
    );
}

#[test]
fn drain_batch_into_reused_builder_performs_zero_heap_allocs() {
    let sink = CompletionSink::new();
    let mut builder = NveHostEventBatchBuilder::with_capacity(4096);
    // Warm up queue capacity & builder once.
    for i in 0..8u64 {
        assert!(sink.register_in_flight(i + 1, SERVICE_CLIPBOARD, 1));
        assert!(sink.complete_ok(i + 1, Vec::new()));
    }
    assert_eq!(sink.drain_batch_into(&mut builder).unwrap(), 8);

    // Steady-state empty-payload complete + drain_batch_into into pre-sized builder = 0 heap allocs.
    let allocs = count_thread_allocs(|| {
        assert!(sink.register_in_flight(100, SERVICE_CLIPBOARD, 1));
        assert!(sink.complete_ok(100, Vec::new()));
        builder.clear();
        let written = sink.drain_batch_into(&mut builder).expect("drain");
        assert_eq!(written, 1);
    });
    assert_eq!(
        allocs, 0,
        "steady-state single-in-flight complete + drain_batch_into into reused builder must do 0 allocs"
    );
}

#[test]
fn sync_roundtrip_stays_zero_alloc_with_pending_requests_live_and_oversized_retry_runs_once() {
    struct MixedHandler {
        oversized_calls: Arc<AtomicUsize>,
        seen_pending_id: Arc<Mutex<Option<u64>>>,
    }
    impl HostServiceHandler for MixedHandler {
        fn service_id(&self) -> u32 {
            SERVICE_CLIPBOARD
        }
        fn handle(
            &self,
            method_id: u32,
            request_id: u64,
            request: &[u8],
            _done: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            match method_id {
                1 => Ok(HandlerReply::Ready(ResponseBuf::from_slice_or_vec(request))),
                2 => {
                    *self.seen_pending_id.lock().unwrap() = Some(request_id);
                    Ok(HandlerReply::Pending)
                }
                3 => {
                    self.oversized_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(HandlerReply::ready(vec![0x5Au8; 5 * 1024]))
                }
                _ => Err(ServiceError::Unsupported),
            }
        }
    }

    let oversized_calls = Arc::new(AtomicUsize::new(0));
    let seen_pending_id = Arc::new(Mutex::new(None));
    let reg = Arc::new(HostServiceRegistry::new().with_handler(MixedHandler {
        oversized_calls: Arc::clone(&oversized_calls),
        seen_pending_id: Arc::clone(&seen_pending_id),
    }));
    let bridge = HostCallbackBridge::new(Arc::clone(&reg));
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());

    // 1. Caller-chosen request_id passes through bridge intact (Pending.request_id == caller_id).
    let res = ffi
        .invoke_raw_with_id(SERVICE_CLIPBOARD, 2, 42_007, &[])
        .expect("pending with explicit caller_id");
    assert_eq!(res, InvokeResult::Pending { request_id: 42_007 });
    assert_eq!(*seen_pending_id.lock().unwrap(), Some(42_007));

    // Add a second long-lived Pending request; sync calls MUST STILL perform 0 heap allocs.
    let res2 = ffi
        .invoke_raw_with_id(SERVICE_CLIPBOARD, 2, 42_008, &[])
        .expect("second pending");
    assert_eq!(res2, InvokeResult::Pending { request_id: 42_008 });
    assert_eq!(reg.completion_sink().in_flight_count(), 2);
    assert_eq!(ffi.pending_request_count(), 2);

    let req = b"sync-while-pending-live";
    let allocs = count_thread_allocs(|| {
        for _ in 0..16 {
            let out = ffi.invoke_sync(SERVICE_CLIPBOARD, 1, req).expect("sync ok");
            assert!(out.is_inline());
            assert_eq!(out.as_slice(), req);
        }
    });
    assert_eq!(
        allocs, 0,
        "sync C-ABI invoke must stay 0-alloc even while multiple Pending requests are live"
    );

    // 2. Oversized (> 4 KiB) BufferTooSmall retry MUST execute the handler EXACTLY ONCE.
    let big = ffi
        .invoke_sync(SERVICE_CLIPBOARD, 3, &[])
        .expect("5 KiB oversized sync response");
    assert_eq!(big.len(), 5 * 1024);
    assert!(big.as_slice().iter().all(|&b| b == 0x5A));
    assert_eq!(
        oversized_calls.load(Ordering::SeqCst),
        1,
        "oversized BufferTooSmall retry must serve cached response without re-invoking handler"
    );

    // 3. Polled pending requests are counted and cleaned on cancel_pending (zero slot leaks).
    assert_eq!(ffi.cancel_pending(), 2);
    assert_eq!(ffi.pending_request_count(), 0);
    assert_eq!(ffi.total_request_slot_count(), 0);
}

#[test]
fn completion_sink_caps_depth_and_byte_budget_and_request_table_rejects_dup_and_sync_race() {
    // 1. CompletionSink depth cap prefers evicting Push over Response.
    let sink = CompletionSink::with_max_queued_records(4);
    assert!(sink.register_in_flight(10, SERVICE_CLIPBOARD, 1));
    assert!(sink.complete_ok(10, b"resp-10".to_vec()));
    for i in 0..10u32 {
        sink.push_event(SERVICE_SYSTEM_ENV, i, vec![i as u8; 8]);
    }
    assert_eq!(sink.queued_count(), 4);
    assert_eq!(sink.dropped_overflow_count(), 7);
    let batch = sink.drain_batch().expect("bounded batch");
    let parsed = NveHostEventBatchRef::parse(&batch).unwrap();
    let recs: Vec<_> = parsed.iter().collect();
    assert_eq!(recs.len(), 4);
    assert_eq!(
        recs[0].request_id, 10,
        "overflow eviction must preserve Response over older Push records"
    );

    // 2. Byte-budgeted drain stops at budget, re-arms on_ready, and drains single >budget record.
    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_c = Arc::clone(&wakes);
    let sink2 = CompletionSink::new();
    sink2.push_event(SERVICE_SYSTEM_ENV, 1, vec![1u8; 300]);
    sink2.push_event(SERVICE_SYSTEM_ENV, 2, vec![2u8; 100]);
    sink2.push_event(SERVICE_SYSTEM_ENV, 3, vec![3u8; 100]);
    sink2.set_on_ready(Some(Arc::new(move || {
        wakes_c.fetch_add(1, Ordering::SeqCst);
    })));
    let mut b = NveHostEventBatchBuilder::with_capacity(512);
    // Budget 200B: first record is 336B (> budget), must still drain 1 record when written == 0!
    assert_eq!(sink2.drain_batch_into_with_budget(&mut b, 200).unwrap(), 1);
    assert_eq!(sink2.queued_count(), 2);
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    b.clear();
    // Next budget 200B fits record 2 (136B) but stops before record 3 (272B > 200B).
    assert_eq!(sink2.drain_batch_into_with_budget(&mut b, 200).unwrap(), 1);
    assert_eq!(sink2.queued_count(), 1);
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    b.clear();
    assert_eq!(sink2.drain_batch_into_with_budget(&mut b, 200).unwrap(), 1);
    assert_eq!(sink2.queued_count(), 0);

    // 3. RequestTable duplicate rejection + interleaved bind(None) during sync invoke leaves 0 slots.
    let mut tbl = nve_host_services::RequestTable::new();
    assert!(tbl.register_pending(7, None));
    assert!(!tbl.register_pending(7, None));
    let (cancelled, cbs) = tbl.cancel_all_pending();
    assert_eq!(cancelled, 1);
    assert!(cbs.is_empty());
    assert_eq!(tbl.total_slot_count(), 0);

    let ffi_race = Arc::new(FfiHostServices::new());
    struct UnbindOnInvoke(Arc<FfiHostServices>);
    impl HostServiceProvider for UnbindOnInvoke {
        fn invoke_raw_with_id(
            &self,
            _: u32,
            _: u32,
            _: u64,
            _: &[u8],
        ) -> Result<InvokeResult, ServiceError> {
            self.0.bind(None);
            Ok(InvokeResult::Ready(ResponseBuf::from_slice_or_vec(b"ok")))
        }
        fn channel_publish(&self, _: u32, _: bool, _: &[u8]) -> Result<(), ServiceError> {
            Ok(())
        }
        fn poll_host_events(&self) -> Vec<nve_host_services::HostEvent> {
            Vec::new()
        }
    }
    let race_bridge = HostCallbackBridge::new(UnbindOnInvoke(Arc::clone(&ffi_race)));
    ffi_race.bind(Some(race_bridge.callbacks()));
    let out = ffi_race.invoke_sync(SERVICE_CLIPBOARD, 1, &[]).unwrap();
    assert_eq!(out.as_slice(), b"ok");
    assert_eq!(
        ffi_race.total_request_slot_count(),
        0,
        "racing bind(None) during sync invoke_raw must not strand Completed(Cancelled) zombie slot"
    );
}
