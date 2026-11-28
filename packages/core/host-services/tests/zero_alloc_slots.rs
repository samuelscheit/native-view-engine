//! Per-thread `CountingAlloc` zero-allocation guards for lock-free [`ServiceSlots`] +
//! [`ServiceHandle`] and [`nve_codec::ByteTransport`] + [`nve_codec::with_stack_writer`],
//! plus snapshot isolation, refcount/drop accounting, and concurrency tests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use nve_abi::SERVICE_CLIPBOARD;
use nve_codec::{
    call_sync_decode, try_with_stack_writer, with_stack_writer, ByteReader, ByteTransport,
    TransportError,
};
use nve_host_services::{
    CompletionSink, FfiHostServices, HandlerReply, HostCallbackBridge, HostServiceHandler,
    HostServiceRegistry, ResponseBuf, ServiceError, ServiceHandle, ServiceSlot, ServiceSlotId,
    ServiceSlots, SLOT_CLIPBOARD, SLOT_HAPTICS,
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

    // ByteTransport::call_async delivers sync-ready and unbound-error replies cleanly.
    let (tx, rx) = std::sync::mpsc::channel();
    transport.call_async(
        SERVICE_CLIPBOARD,
        1,
        b"async-echo",
        Box::new(move |res| {
            let _ = tx.send(res.map(|b| b.to_vec()));
        }),
    );
    assert_eq!(rx.recv().unwrap(), Ok(b"async-echo".to_vec()));

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
        assert_eq!(base.len(), 2);
        assert_eq!(base.slot_capacity(), 11);
        assert_eq!(base.get::<dyn TestBackend>().unwrap().compute(5), 15);
        assert_eq!(base.get_at::<u32>(ServiceSlotId::from_const::<10>()), Some(&42));
        // Wrong type at an occupied slot MUST return None (no UB or mis-cast).
        assert!(base.get_at::<u64>(ServiceSlotId::from_const::<10>()).is_none());
        assert!(base.get_at::<dyn OtherBackend>(SLOT_CLIPBOARD).is_none());

        // Per-surface / test override leaves `base` untouched.
        let overridden = base.with_override(Arc::clone(&mock_backend));
        assert_eq!(base.get::<dyn TestBackend>().unwrap().compute(5), 15);
        assert_eq!(overridden.get::<dyn TestBackend>().unwrap().compute(5), 500);

        // Removing a slot leaves sibling snapshots intact.
        let stripped = overridden.without_slot(SLOT_CLIPBOARD);
        assert!(stripped.get::<dyn TestBackend>().is_none());
        assert_eq!(overridden.get::<dyn TestBackend>().unwrap().compute(5), 500);

        // Multi-threaded concurrent lock-free reads.
        let mut threads = Vec::new();
        for _ in 0..4 {
            let snap = overridden.clone();
            threads.push(std::thread::spawn(move || {
                for i in 0..500u64 {
                    assert_eq!(snap.get::<dyn TestBackend>().unwrap().compute(i), i * 100);
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
