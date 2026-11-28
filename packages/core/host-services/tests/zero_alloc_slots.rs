//! Per-thread `CountingAlloc` zero-allocation guards for lock-free [`ServiceSlots`] +
//! [`ServiceHandle`] and [`nve_codec::ByteTransport`] + [`nve_codec::with_stack_writer`],
//! plus snapshot isolation, refcount/drop accounting, and concurrency tests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{
    NveBufferSlice, NveHostCallbacks, NveHostEventBatchBuilder, NveHostStatus, SERVICE_CLIPBOARD,
};
use nve_codec::{
    call_sync_decode, try_with_stack_writer, with_stack_writer, ByteReader, ByteTransport,
    TransportDoneFn, TransportError,
};
use nve_host_services::{
    CompletionSink, FfiHostServices, HandlerReply, HostCallbackBridge, HostServiceHandler,
    HostServiceRegistry, ResponseBuf, ServiceError, ServiceHandle, ServiceSlot, ServiceSlotId,
    ServiceSlots, SLOT_CLIPBOARD, SLOT_FILESYSTEM, SLOT_HAPTICS, SLOT_USER_BASE,
};

struct CountingAlloc;

thread_local! {
    static TRACK_THREAD: Cell<bool> = const { Cell::new(false) };
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: Delegates directly to `System` while counting heap allocations only on the tracked thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK_THREAD.try_with(|flag| {
            if flag.get() {
                let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
            }
        });
        // SAFETY: Forwarded unchanged to `System::alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forwarded unchanged to `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = TRACK_THREAD.try_with(|flag| {
            if flag.get() {
                let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
            }
        });
        // SAFETY: Forwarded unchanged to `System::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_thread_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    THREAD_ALLOCS.with(|c| c.set(0));
    TRACK_THREAD.with(|flag| flag.set(true));
    let out = f();
    TRACK_THREAD.with(|flag| flag.set(false));
    (THREAD_ALLOCS.with(Cell::get), out)
}

trait TestBackend: Send + Sync {
    fn compute(&self, x: u64) -> u64;
}

impl ServiceSlot for dyn TestBackend {
    const SLOT: ServiceSlotId = SLOT_CLIPBOARD;
}

trait OtherBackend: Send + Sync {
    fn tag(&self) -> u32;
}

impl ServiceSlot for dyn OtherBackend {
    const SLOT: ServiceSlotId = SLOT_HAPTICS;
}

struct MultiplierBackend {
    factor: u64,
    dropped: Option<Arc<AtomicUsize>>,
}

impl Drop for MultiplierBackend {
    fn drop(&mut self) {
        if let Some(counter) = &self.dropped {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl TestBackend for MultiplierBackend {
    fn compute(&self, x: u64) -> u64 {
        x.wrapping_mul(self.factor)
    }
}

struct StaticOther(u32);
impl OtherBackend for StaticOther {
    fn tag(&self) -> u32 {
        self.0
    }
}

#[test]
fn positive_control_probe_detects_allocations() {
    let (allocs, len) = count_thread_allocs(|| {
        let v = std::hint::black_box(vec![0xCDu8; 64]);
        v.len()
    });
    assert_eq!(len, 64);
    assert!(
        allocs >= 1,
        "positive CountingAlloc control probe must observe >= 1 allocation"
    );
}

#[test]
fn service_slots_get_and_handle_perform_zero_allocs_and_zero_refcount_bumps() {
    // Background allocation noise proves thread-local CountingAlloc never false-triggers.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_bg = Arc::clone(&stop);
    let bg = std::thread::spawn(move || {
        while !stop_bg.load(Ordering::Relaxed) {
            let v: Vec<u8> = Vec::with_capacity(64);
            std::hint::black_box(v);
        }
    });

    let backend: Arc<dyn TestBackend> = Arc::new(MultiplierBackend {
        factor: 7,
        dropped: None,
    });
    let slots = ServiceSlots::builder()
        .set(Arc::clone(&backend))
        .set::<dyn OtherBackend>(Arc::new(StaticOther(99)))
        .build();
    let cached: ServiceHandle<dyn TestBackend> = slots
        .handle::<dyn TestBackend>()
        .expect("cached handle must resolve");

    let count_before = Arc::strong_count(&backend);

    let (allocs, total) = count_thread_allocs(|| {
        let mut acc = 0u64;
        for i in 0..1024u64 {
            let svc = slots
                .get::<dyn TestBackend>()
                .expect("dense slot must resolve");
            acc = acc.wrapping_add(svc.compute(i));
            acc = acc.wrapping_add(cached.compute(i));
            assert_eq!(
                slots
                    .get::<dyn OtherBackend>()
                    .expect("other slot")
                    .tag(),
                99
            );
        }
        acc
    });

    let count_after = Arc::strong_count(&backend);
    stop.store(true, Ordering::Relaxed);
    let _ = bg.join();

    assert_eq!(count_before, count_after, "get() must not alter Arc strong count");
    assert_eq!(
        allocs, 0,
        "ServiceSlots::get::<dyn TestBackend>() + call must perform 0 heap allocations"
    );
    assert_eq!(total, (0..1024u64).map(|i| i * 14).sum::<u64>());
}

#[test]
fn byte_transport_call_sync_with_stack_writer_fast_path_performs_zero_allocs() {
    struct InlineEchoHandler;
    impl HostServiceHandler for InlineEchoHandler {
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

    let reg = HostServiceRegistry::new().with_handler(InlineEchoHandler);
    let bridge = HostCallbackBridge::new(reg);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let transport: &dyn ByteTransport = &ffi;

    let (allocs, acc) = count_thread_allocs(|| {
        let mut sum = 0u64;
        for i in 0..64u64 {
            let decoded: u64 = try_with_stack_writer::<512, _, TransportError>(
                |w| {
                    w.write_u64(i)?;
                    w.write_str("slot.fast.path")?;
                    w.write_bytes(b"inline-bytes")?;
                    w.write_bool(true)
                },
                |req| {
                    call_sync_decode(transport, SERVICE_CLIPBOARD, 1, req, |reply| {
                        let mut r = ByteReader::new(reply);
                        let n = r.read_u64()?;
                        let label = r.read_str(64)?;
                        let payload = r.read_bytes(64)?;
                        let ok = r.read_bool()?;
                        r.finish()?;
                        assert_eq!(label, "slot.fast.path");
                        assert_eq!(payload, b"inline-bytes");
                        assert!(ok);
                        Ok(n)
                    })
                },
            )
            .expect("sync ByteTransport call");
            sum += decoded;
        }
        sum
    });

    assert_eq!(acc, (0..64u64).sum::<u64>());
    assert_eq!(
        allocs, 0,
        "ByteTransport::call_sync + with_stack_writer::<512> fast path must perform 0 allocations"
    );

    // Overflowing stack capacity spills cleanly and succeeds.
    let (spill_allocs, len) = count_thread_allocs(|| {
        with_stack_writer::<16, _>(
            |w| w.write_str("payload-exceeding-16-bytes-triggers-heap-spill"),
            |bytes| bytes.len(),
        )
        .expect("spill encode")
    });
    assert!(len > 16);
    assert!(spill_allocs >= 1, "overflow path must spill to heap Vec<u8>");

    // ByteTransport::call_async sync-ready inline reply allocates 0 bytes inside call_async
    // (no double-boxing of TransportDoneFn and no `ResponseBuf::Inline::into_vec` copy).
    let sync_out = Arc::new(Mutex::new(None));
    let sync_out_c = Arc::clone(&sync_out);
    let mut preboxed_len = 0usize;
    let preboxed: TransportDoneFn = Box::new(move |res| {
        if let Ok(bytes) = res {
            preboxed_len = bytes.len();
        }
        let _ = preboxed_len;
        *sync_out_c.lock().unwrap() = Some(res.map(|b| b.to_vec()));
    });
    // Measure with a non-allocating callback first to prove 0 allocs inside call_async itself.
    let async_hit = Arc::new(AtomicUsize::new(0));
    let async_hit_c = Arc::clone(&async_hit);
    let zero_alloc_cb: TransportDoneFn = Box::new(move |res| {
        if let Ok(bytes) = res {
            async_hit_c.store(bytes.len(), Ordering::SeqCst);
        }
    });
    let (async_inline_allocs, ()) = count_thread_allocs(|| {
        transport.call_async(SERVICE_CLIPBOARD, 1, b"async-inline-zero-alloc", zero_alloc_cb);
    });
    assert_eq!(async_hit.load(Ordering::SeqCst), b"async-inline-zero-alloc".len());
    assert_eq!(
        async_inline_allocs, 0,
        "ByteTransport::call_async on inline ready reply must perform 0 extra allocations"
    );

    transport.call_async(SERVICE_CLIPBOARD, 1, b"async-echo", preboxed);
    assert_eq!(*sync_out.lock().unwrap(), Some(Ok(b"async-echo".to_vec())));

    let unbound = FfiHostServices::new();
    let (tx2, rx2) = std::sync::mpsc::channel();
    unbound.call_async(
        SERVICE_CLIPBOARD,
        1,
        &[],
        Box::new(move |res| {
            let _ = tx2.send(res.map(|b| b.to_vec()));
        }),
    );
    assert_eq!(rx2.recv().unwrap(), Err(TransportError::Unsupported));
}

#[test]
fn ffi_byte_transport_pending_cancel_unbind_errors_oversized_retry_and_zero_leak() {
    struct MultiModeHandler {
        pending_ids: Mutex<Vec<u64>>,
    }

    impl HostServiceHandler for MultiModeHandler {
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
                1 => {
                    // Pending without mutating heap when request is empty (for 0-alloc call_sync test).
                    if !request.is_empty() {
                        self.pending_ids.lock().unwrap().push(request_id);
                    }
                    Ok(HandlerReply::Pending)
                }
                2 => Err(ServiceError::PermissionDenied),
                3 => {
                    // > 4 KiB reply triggering BufferTooSmall + exact-sized heap retry.
                    Ok(HandlerReply::Ready(ResponseBuf::Heap(vec![0x5Au8; 12_345])))
                }
                _ => Err(ServiceError::Unsupported),
            }
        }
    }

    let handler = Arc::new(MultiModeHandler {
        pending_ids: Mutex::new(Vec::new()),
    });
    let reg = HostServiceRegistry::new()
        .with_handler_arc(Arc::clone(&handler) as Arc<dyn HostServiceHandler>);
    let bridge = HostCallbackBridge::new(reg);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let transport: &dyn ByteTransport = &ffi;

    // 1. call_sync receiving Pending > 8 times (32 calls) MUST NOT leak RequestTable slots and
    // MUST perform 0 client-side heap allocations (measured against a raw C callback so host-side
    // CompletionSink tracking does not conflate client allocation counts).
    unsafe extern "C" fn always_pending_invoke(
        _host_ctx: *mut core::ffi::c_void,
        _service_id: u32,
        _method_id: u32,
        _request_id: u64,
        _request: NveBufferSlice,
        _out_buf: *mut u8,
        _out_cap: usize,
        _out_len: *mut usize,
    ) -> i32 {
        NveHostStatus::Pending.as_i32()
    }
    let raw_pending_ffi = FfiHostServices::with_callbacks(NveHostCallbacks {
        host_ctx: core::ptr::null_mut(),
        invoke: Some(always_pending_invoke),
        channel_publish: None,
    });
    let raw_pending_transport: &dyn ByteTransport = &raw_pending_ffi;
    let (pending_sync_allocs, ()) = count_thread_allocs(|| {
        for _ in 0..32 {
            let res = raw_pending_transport.call_sync(SERVICE_CLIPBOARD, 1, &[], &mut |_| {
                panic!("Pending must not invoke on_reply");
            });
            assert_eq!(
                res,
                Err(TransportError::Status(NveHostStatus::Pending.as_i32()))
            );
        }
    });
    assert_eq!(
        pending_sync_allocs, 0,
        "FfiHostServices::call_sync Pending path must allocate 0 bytes across >8 calls"
    );
    assert_eq!(
        raw_pending_ffi.total_request_slot_count(),
        0,
        "call_sync Pending must remove its RequestTable slot without leaking"
    );
    assert_eq!(raw_pending_ffi.pending_request_count(), 0);

    // Also verify zero RequestTable slot leak through HostCallbackBridge + HostServiceRegistry.
    for _ in 0..16 {
        let res = transport.call_sync(SERVICE_CLIPBOARD, 1, &[], &mut |_| {
            panic!("Pending must not invoke on_reply");
        });
        assert_eq!(
            res,
            Err(TransportError::Status(NveHostStatus::Pending.as_i32()))
        );
    }
    assert_eq!(ffi.total_request_slot_count(), 0);
    assert_eq!(ffi.pending_request_count(), 0);

    // 2. call_async with HandlerReply::Pending resolved via resolve_async_response.
    let (tx1, rx1) = std::sync::mpsc::channel();
    transport.call_async(
        SERVICE_CLIPBOARD,
        1,
        b"track",
        Box::new(move |res| {
            let _ = tx1.send(res.map(|b| b.to_vec()));
        }),
    );
    let req1 = handler.pending_ids.lock().unwrap().pop().expect("req1");
    assert_eq!(ffi.pending_request_count(), 1);
    ffi.resolve_async_response(req1, NveHostStatus::Ok, b"async-resolved".to_vec());
    assert_eq!(rx1.recv().unwrap(), Ok(b"async-resolved".to_vec()));
    assert_eq!(ffi.total_request_slot_count(), 0);

    // 3. call_async with HandlerReply::Pending resolved via ingest_batch.
    let (tx2, rx2) = std::sync::mpsc::channel();
    transport.call_async(
        SERVICE_CLIPBOARD,
        1,
        b"track",
        Box::new(move |res| {
            let _ = tx2.send(res.map(|b| b.to_vec()));
        }),
    );
    let req2 = handler.pending_ids.lock().unwrap().pop().expect("req2");
    let mut batch = NveHostEventBatchBuilder::new();
    batch
        .push_response(
            SERVICE_CLIPBOARD,
            1,
            req2,
            NveHostStatus::Ok,
            b"batch-resolved",
        )
        .unwrap();
    let stats = ffi.ingest_batch(batch.as_bytes()).unwrap();
    assert_eq!(stats.responses, 1);
    assert_eq!(rx2.recv().unwrap(), Ok(b"batch-resolved".to_vec()));
    assert_eq!(ffi.total_request_slot_count(), 0);

    // 4. call_async + cancel_all_pending() -> TransportError::Cancelled.
    let (tx3, rx3) = std::sync::mpsc::channel();
    transport.call_async(
        SERVICE_CLIPBOARD,
        1,
        b"track",
        Box::new(move |res| {
            let _ = tx3.send(res.map(|b| b.to_vec()));
        }),
    );
    assert_eq!(ffi.cancel_all_pending(), 1);
    assert_eq!(rx3.recv().unwrap(), Err(TransportError::Cancelled));
    assert_eq!(ffi.total_request_slot_count(), 0);

    // 5. call_async with bound handler returning Err(PermissionDenied).
    let (tx4, rx4) = std::sync::mpsc::channel();
    transport.call_async(
        SERVICE_CLIPBOARD,
        2,
        &[],
        Box::new(move |res| {
            let _ = tx4.send(res.map(|b| b.to_vec()));
        }),
    );
    assert_eq!(rx4.recv().unwrap(), Err(TransportError::PermissionDenied));
    assert_eq!(ffi.total_request_slot_count(), 0);

    // 6. > 4 KiB BufferTooSmall retry over both ByteTransport::call_sync and call_async.
    let sync_big = call_sync_decode(transport, SERVICE_CLIPBOARD, 3, &[], |reply| {
        Ok(reply.to_vec())
    })
    .unwrap();
    assert_eq!(sync_big.len(), 12_345);
    assert!(sync_big.iter().all(|&b| b == 0x5A));

    let (tx5, rx5) = std::sync::mpsc::channel();
    transport.call_async(
        SERVICE_CLIPBOARD,
        3,
        &[],
        Box::new(move |res| {
            let _ = tx5.send(res.map(|b| b.to_vec()));
        }),
    );
    let async_big = rx5.recv().unwrap().unwrap();
    assert_eq!(async_big.len(), 12_345);
    assert!(async_big.iter().all(|&b| b == 0x5A));

    // 7. call_async + bind(None) -> TransportError::Cancelled.
    let (tx6, rx6) = std::sync::mpsc::channel();
    transport.call_async(
        SERVICE_CLIPBOARD,
        1,
        b"track",
        Box::new(move |res| {
            let _ = tx6.send(res.map(|b| b.to_vec()));
        }),
    );
    ffi.bind(None);
    assert_eq!(rx6.recv().unwrap(), Err(TransportError::Cancelled));
    assert_eq!(ffi.total_request_slot_count(), 0);
}

#[test]
fn service_slots_snapshot_overrides_type_mismatch_drop_accounting_and_threads() {
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let base_backend: Arc<dyn TestBackend> = Arc::new(MultiplierBackend {
            factor: 3,
            dropped: Some(Arc::clone(&drops)),
        });
        let mock_backend: Arc<dyn TestBackend> = Arc::new(MultiplierBackend {
            factor: 100,
            dropped: Some(Arc::clone(&drops)),
        });

        let empty = ServiceSlots::empty();
        assert!(empty.is_empty());
        assert_eq!(empty.slot_capacity(), 0);
        assert!(empty.get::<dyn TestBackend>().is_none());

        let base = ServiceSlots::builder()
            .set(Arc::clone(&base_backend))
            .set_value_at(ServiceSlotId::from_const::<10>(), 42u32)
            .build();
        assert!(!base.is_empty());
        assert_eq!(base.len(), 2);
        assert_eq!(base.slot_capacity(), 11);
        assert!(base.contains(SLOT_CLIPBOARD));
        assert!(!base.contains(SLOT_FILESYSTEM));
        assert!(base.contains(ServiceSlotId::from_const::<10>()));
        assert!(!base.contains(ServiceSlotId::from_const::<99>()));
        assert_eq!(base.get::<dyn TestBackend>().unwrap().compute(5), 15);
        assert_eq!(base.get_arc::<dyn TestBackend>().unwrap().compute(5), 15);
        assert_eq!(
            *base
                .handle_at::<u32>(ServiceSlotId::from_const::<10>())
                .unwrap(),
            42
        );
        assert_eq!(base.get_at::<u32>(ServiceSlotId::from_const::<10>()), Some(&42));
        // Wrong type at an occupied slot MUST return None (no UB or mis-cast).
        assert!(base.get_at::<u64>(ServiceSlotId::from_const::<10>()).is_none());
        assert!(base.get_at::<dyn OtherBackend>(SLOT_CLIPBOARD).is_none());

        // Single-pass with_override_at: extends capacity, leaves `base` untouched, and does NOT
        // bump/drop the replaced slot's Arc.
        let zst_slot = ServiceSlotId::from_const::<SLOT_USER_BASE>();
        let dst_slot = ServiceSlotId::from_const::<{ SLOT_USER_BASE + 1 }>();
        let zst_arc: Arc<()> = Arc::new(());
        let slice_arc: Arc<[u8]> = Arc::from(&b"fat-slice-payload"[..]);
        let with_special = base
            .with_override_at(zst_slot, Arc::clone(&zst_arc))
            .with_override_at(dst_slot, Arc::clone(&slice_arc));
        assert_eq!(with_special.get_at::<()>(zst_slot), Some(&()));
        assert_eq!(
            with_special.get_at::<[u8]>(dst_slot),
            Some(&b"fat-slice-payload"[..])
        );
        assert_eq!(
            with_special.get_arc_at::<[u8]>(dst_slot).as_deref(),
            Some(&b"fat-slice-payload"[..])
        );

        let base_strong_before = Arc::strong_count(&base_backend);
        let overridden = base.with_override(Arc::clone(&mock_backend));
        assert_eq!(Arc::strong_count(&base_backend), base_strong_before);
        assert_eq!(base.get::<dyn TestBackend>().unwrap().compute(5), 15);
        assert_eq!(overridden.get::<dyn TestBackend>().unwrap().compute(5), 500);

        // Removing a slot leaves sibling snapshots intact, and removing all slots trims to EMPTY.
        let stripped = overridden.without_slot(SLOT_CLIPBOARD);
        assert!(stripped.get::<dyn TestBackend>().is_none());
        assert!(!stripped.contains(SLOT_CLIPBOARD));
        assert_eq!(overridden.get::<dyn TestBackend>().unwrap().compute(5), 500);

        // Removing an already-empty slot is a cheap clone; removing final slot trims back to EMPTY.
        let same_cap = stripped.without_slot(SLOT_FILESYSTEM);
        assert_eq!(same_cap.slot_capacity(), stripped.slot_capacity());
        let trimmed_empty = stripped.without_slot(ServiceSlotId::from_const::<10>());
        assert!(trimmed_empty.is_empty());
        assert_eq!(trimmed_empty.len(), 0);
        assert_eq!(trimmed_empty.slot_capacity(), 0);

        // Multi-threaded concurrent lock-free reads, overrides, and drops.
        let mut threads = Vec::new();
        for tid in 0..4u64 {
            let snap = overridden.clone();
            let mock_c = Arc::clone(&mock_backend);
            threads.push(std::thread::spawn(move || {
                for i in 0..250u64 {
                    assert_eq!(snap.get::<dyn TestBackend>().unwrap().compute(i), i * 100);
                    if i % 50 == 0 {
                        let local = snap
                            .with_override_at(
                                ServiceSlotId::from_const::<{ SLOT_USER_BASE + 2 }>(),
                                Arc::new(tid + i),
                            )
                            .with_override(Arc::clone(&mock_c));
                        assert_eq!(
                            local.get_at::<u64>(ServiceSlotId::from_const::<{ SLOT_USER_BASE + 2 }>()),
                            Some(&(tid + i))
                        );
                        let cleared =
                            local.without_slot(ServiceSlotId::from_const::<{ SLOT_USER_BASE + 2 }>());
                        assert!(!cleared.contains(ServiceSlotId::from_const::<{ SLOT_USER_BASE + 2 }>()));
                    }
                }
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
    }
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "every single-Arc service stored in ServiceSlots must drop exactly once with zero leaks"
    );

    // Dynamic user slot allocator produces distinct valid slots >= SLOT_USER_BASE.
    let s1 = ServiceSlotId::alloc_user_slot().expect("user slot 1");
    let s2 = ServiceSlotId::alloc_user_slot().expect("user slot 2");
    assert_ne!(s1, s2);
    assert!(s1.raw() >= nve_host_services::SLOT_USER_BASE);
    assert!(ServiceSlotId::try_new(nve_host_services::MAX_SERVICE_SLOTS as u16).is_none());
}
