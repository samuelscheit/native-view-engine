//! Zero-allocation `CountingAlloc` guard + soundness/edge-case tests for `nve_abi::sink`
//! (`SurfaceToken`, `NveBytesSink`, `with_bytes_sink`, `try_with_bytes_sink`,
//! `with_bytes_sink_once`, `AsyncBytesDoneGuard`, `into_async_bytes_done`).
//!
//! Intentionally compiled WITHOUT `#[cfg(feature = "wire")]` so every test also passes under
//! `cargo test -p nve-abi --no-default-features`.

use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::id::SurfaceId;
use nve_abi::sink::{
    into_async_bytes_done, invoke_async_bytes_done, invoke_bytes_sink, sink_bytes,
    try_with_bytes_sink, try_with_bytes_sink_once, with_bytes_sink, with_bytes_sink_once,
    AsyncBytesDoneGuard, BytesSinkError, NveBytesSink, SurfaceToken,
};

struct CountingAlloc;

thread_local! {
    static TRACK_THREAD: Cell<bool> = const { Cell::new(false) };
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: Delegates directly to `System`, counting allocations only on the tracked thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK_THREAD.try_with(|flag| {
            if flag.get() {
                let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
            }
        });
        // SAFETY: Preconditions upheld by caller and forwarded to `System::alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Preconditions upheld by caller and forwarded to `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = TRACK_THREAD.try_with(|flag| {
            if flag.get() {
                let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
            }
        });
        // SAFETY: Preconditions upheld by caller and forwarded to `System::realloc`.
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

#[test]
fn with_bytes_sink_zero_alloc_guard_with_positive_probe() {
    // Positive control probe: prove CountingAlloc detects real heap allocations on this thread.
    let (probe_allocs, probe_len) = count_thread_allocs(|| {
        let v = std::hint::black_box(vec![0x5Au8; 64]);
        v.len()
    });
    assert_eq!(probe_len, 64);
    assert!(
        probe_allocs >= 1,
        "positive control probe must observe >= 1 heap allocation"
    );

    // Background allocation noise proves thread-local counting never false-triggers.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_bg = Arc::clone(&stop);
    let bg = std::thread::spawn(move || {
        while !stop_bg.load(Ordering::Relaxed) {
            let v: Vec<u8> = Vec::with_capacity(64);
            std::hint::black_box(v);
        }
    });

    let payload = b"direct-ffi-zero-alloc-bytes";
    let (allocs, checksum) = count_thread_allocs(|| {
        let mut sum = 0usize;
        for _ in 0..256 {
            let res = with_bytes_sink(
                |bytes| {
                    sum = sum.wrapping_add(bytes.len());
                    if let Some(&first) = bytes.first() {
                        sum = sum.wrapping_add(usize::from(first));
                    }
                },
                |sink, ctx| {
                    // SAFETY: Valid synchronous sink/ctx pair for this call.
                    unsafe {
                        assert!(invoke_bytes_sink(Some(sink), ctx, payload));
                        assert!(invoke_bytes_sink(Some(sink), ctx, &[]));
                    }
                    1usize
                },
            );
            assert_eq!(res, Ok(1));

            let once = with_bytes_sink_once(
                |bytes| {
                    sum = sum.wrapping_add(bytes.len());
                },
                |sink, ctx| {
                    // SAFETY: Valid synchronous sink/ctx pair for this call.
                    unsafe { invoke_bytes_sink(Some(sink), ctx, payload) }
                },
            );
            assert_eq!(once, Ok(true));

            // Corrupt slice rejection also performs 0 allocations.
            let corrupt = with_bytes_sink(
                |_| unreachable!("corrupt slice must not invoke callback"),
                |sink, ctx| {
                    // SAFETY: Null pointer with non-zero len is rejected before creating a slice.
                    unsafe { sink(ctx, std::ptr::null(), 128) };
                },
            );
            assert_eq!(corrupt, Err(BytesSinkError::CorruptSlice { len: 128 }));
        }
        sum
    });

    stop.store(true, Ordering::Relaxed);
    let _ = bg.join();

    assert_eq!(allocs, 0, "with_bytes_sink fast path must perform 0 heap allocations");
    assert_eq!(checksum, 256 * (payload.len() + usize::from(payload[0]) + payload.len()));
}

#[test]
fn surface_token_layout_canonical_null_invariant_wrap_and_packing() {
    assert_eq!(size_of::<SurfaceToken>(), 8);
    assert_eq!(align_of::<SurfaceToken>(), 4);
    assert_eq!(offset_of!(SurfaceToken, surface_id), 0);
    assert_eq!(offset_of!(SurfaceToken, reload_gen), 4);

    assert!(SurfaceToken::NULL.is_null());
    assert_eq!(SurfaceToken::NULL.reload_gen, 0);
    assert!(!SurfaceToken::NULL.is_live_for(SurfaceToken::NULL));
    assert_eq!(SurfaceToken::NULL.next_reload(), SurfaceToken::NULL);

    // Constructing with null SurfaceId canonicalizes reload_gen to 0.
    assert_eq!(SurfaceToken::new(SurfaceId::NULL, 42), SurfaceToken::NULL);

    // FFI-injected dirty null struct (`SurfaceId::NULL` with non-zero reload_gen) canonicalizes cleanly.
    let dirty_null = SurfaceToken {
        surface_id: SurfaceId::NULL,
        reload_gen: 77,
    };
    assert!(dirty_null.is_null());
    assert_eq!(dirty_null.canonicalize(), SurfaceToken::NULL);
    assert_eq!(dirty_null.next_reload(), SurfaceToken::NULL);
    assert_eq!(dirty_null.to_u64(), 0);

    // Bogus index-0 non-zero generation in raw u64 canonicalizes to SurfaceToken::NULL (`reload_gen == 0`).
    let bogus_raw = (0x7Fu64 << 24) | (9u64 << 32);
    let canon = SurfaceToken::from_u64(bogus_raw);
    assert!(canon.is_null());
    assert_eq!(canon, SurfaceToken::NULL);
    assert_eq!(canon.surface_id, SurfaceId::NULL);
    assert_eq!(canon.reload_gen, 0);

    // Live token reload progression + u32::MAX -> 0 wrap.
    let s1 = SurfaceId::new(1, 3);
    let tok_v1 = SurfaceToken::new(s1, 1);
    let tok_v2 = tok_v1.next_reload();
    assert_eq!(tok_v2.surface_id, s1);
    assert_eq!(tok_v2.reload_gen, 2);
    assert!(tok_v1.is_live_for(tok_v1));
    assert!(!tok_v1.is_live_for(tok_v2));
    assert_eq!(SurfaceToken::from_u64(tok_v2.to_u64()), tok_v2);

    let tok_max = SurfaceToken::new(s1, u32::MAX);
    let tok_wrapped = tok_max.next_reload();
    assert_eq!(tok_wrapped.surface_id, s1);
    assert_eq!(tok_wrapped.reload_gen, 0);
    assert!(!tok_wrapped.is_null());
    assert!(!tok_max.is_live_for(tok_wrapped));
    assert!(tok_wrapped.is_live_for(tok_wrapped));
}

#[test]
fn with_bytes_sink_surfaces_corrupt_slices_reentrancy_duplicates_and_panics() {
    // 1. Happy path round-trip (non-empty, empty, and None sink).
    let mut captured = Vec::new();
    let delivered = with_bytes_sink(
        |slice| captured.push(slice.to_vec()),
        |sink, ctx| {
            // SAFETY: Valid synchronous sink/ctx pair for this call.
            unsafe {
                let a = invoke_bytes_sink(Some(sink), ctx, b"hello-direct-ffi");
                let b = invoke_bytes_sink(Some(sink), ctx, &[]);
                let c = invoke_bytes_sink(None, ctx, b"ignored");
                // Null user_ctx is ignored safely without touching state.
                sink(std::ptr::null_mut(), b"x".as_ptr(), 1);
                (a, b, c)
            }
        },
    );
    assert_eq!(delivered, Ok((true, true, false)));
    assert_eq!(captured, vec![b"hello-direct-ffi".to_vec(), Vec::new()]);

    // 2. Corrupt slice (NULL, 128) MUST return Err(BytesSinkError::CorruptSlice), never silent Ok,
    // and MUST short-circuit subsequent valid calls so no side effects run after a violation.
    let mut seen = false;
    let res_null = with_bytes_sink(
        |_| seen = true,
        |sink, ctx| {
            // SAFETY: Rejected before reading pointer; second call is short-circuited after violation.
            unsafe {
                sink(ctx, std::ptr::null(), 128);
                invoke_bytes_sink(Some(sink), ctx, b"after-violation-must-not-run");
            }
            "would-be-ok"
        },
    );
    assert_eq!(res_null, Err(BytesSinkError::CorruptSlice { len: 128 }));
    assert!(!seen, "callback must not run after a prior sink violation");

    // 3. Corrupt slice (non-null, isize::MAX + 1) MUST return Err(BytesSinkError::CorruptSlice).
    let huge_len = isize::MAX as usize + 1;
    let res_huge = try_with_bytes_sink::<_, (), BytesSinkError>(
        |_| panic!("corrupt huge slice must not invoke callback"),
        |sink, ctx| {
            // SAFETY: Rejected before reading pointer.
            unsafe { sink(ctx, b"x".as_ptr(), huge_len) };
            Ok(())
        },
    );
    assert_eq!(res_huge, Err(BytesSinkError::CorruptSlice { len: huge_len }));

    // 4. Re-entrant invocation MUST NOT cause &mut aliasing UB and MUST return Err(Reentrant).
    let reentrant_sink: Cell<Option<(NveBytesSink, *mut c_void)>> = Cell::new(None);
    let mut depth_seen = 0usize;
    let res_reentrant = with_bytes_sink(
        |slice| {
            depth_seen += 1;
            assert_eq!(slice, b"outer");
            if let Some((sink, ctx)) = reentrant_sink.get() {
                // SAFETY: Intentionally re-entering `sink` to test the re-entrancy guard.
                unsafe {
                    invoke_bytes_sink(Some(sink), ctx, b"inner-reentrant");
                }
            }
        },
        |sink, ctx| {
            reentrant_sink.set(Some((sink, ctx)));
            // SAFETY: Valid synchronous sink/ctx pair for this call.
            unsafe { invoke_bytes_sink(Some(sink), ctx, b"outer") }
        },
    );
    assert_eq!(res_reentrant, Err(BytesSinkError::Reentrant));
    assert_eq!(depth_seen, 1, "re-entrant inner call must never enter closure");

    // 5. Single-shot sink rejects duplicate invocations explicitly.
    let mut once_val = None;
    let res_dup = try_with_bytes_sink_once::<_, (), BytesSinkError>(
        |slice| {
            once_val = Some(slice.to_vec());
        },
        |sink, ctx| {
            // SAFETY: Valid synchronous sink/ctx pair for this call.
            unsafe {
                invoke_bytes_sink(Some(sink), ctx, b"first");
                invoke_bytes_sink(Some(sink), ctx, b"second");
            }
            Ok(())
        },
    );
    assert_eq!(res_dup, Err(BytesSinkError::DuplicateCall));
    assert_eq!(once_val.as_deref(), Some(b"first".as_slice()));

    // 6. Panic inside sink callback is caught across extern "C" and resumed after FFI returns.
    let panic_res = std::panic::catch_unwind(|| {
        let _ = with_bytes_sink(
            |_slice| panic!("sink callback panic"),
            |sink, ctx| {
                // SAFETY: Valid sink from `with_bytes_sink`.
                unsafe {
                    invoke_bytes_sink(Some(sink), ctx, b"boom");
                    invoke_bytes_sink(Some(sink), ctx, b"second");
                }
            },
        );
    });
    assert!(panic_res.is_err());

    // Error Display smoke coverage.
    assert!(BytesSinkError::CorruptSlice { len: 7 }.to_string().contains("7"));
    assert!(BytesSinkError::Reentrant.to_string().contains("re-entrant"));
    assert!(BytesSinkError::DuplicateCall.to_string().contains("duplicate"));
}

#[test]
fn async_bytes_done_guard_enforces_once_only_panic_safety_and_corrupt_slice_surfacing() {
    // 1. Happy path + duplicate invocation ignored cleanly (zero UAF, exact once-only delivery).
    let calls = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(Mutex::new(None));
    let calls_c = Arc::clone(&calls);
    let recv_c = Arc::clone(&received);
    let guard = into_async_bytes_done(move |req_id, status, payload| {
        calls_c.fetch_add(1, Ordering::SeqCst);
        *recv_c.lock().unwrap() = Some((req_id, status, payload.map(|b| b.to_vec())));
    });
    let (done_fn, ctx) = guard.as_raw();
    assert!(!guard.is_completed());
    // SAFETY: `guard` stays alive across both invocations.
    unsafe {
        assert!(invoke_async_bytes_done(Some(done_fn), ctx, 42, 0, b"async-ok"));
        assert!(invoke_async_bytes_done(Some(done_fn), ctx, 42, 0, b"duplicate-ignored"));
        assert!(!invoke_async_bytes_done(None, ctx, 43, 0, b"nope"));
        done_fn(std::ptr::null_mut(), 44, 0, b"x".as_ptr(), 1);
    }
    assert!(guard.is_completed());
    assert!(!guard.panicked());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *received.lock().unwrap(),
        Some((42, 0, Ok(b"async-ok".to_vec())))
    );

    // 2. Corrupt slice with status == 0 MUST surface Err(BytesSinkError::CorruptSlice), never Ok(&[]).
    let corrupt_seen = Arc::new(Mutex::new(None));
    let corrupt_c = Arc::clone(&corrupt_seen);
    let corrupt_guard = AsyncBytesDoneGuard::new(move |req_id, status, payload| {
        *corrupt_c.lock().unwrap() = Some((req_id, status, payload.map(|b| b.to_vec())));
    });
    let (c_fn, c_ctx) = corrupt_guard.as_raw();
    // SAFETY: `corrupt_guard` stays alive; corrupt `(NULL, 64)` is rejected before reading.
    unsafe {
        c_fn(c_ctx, 99, 0, std::ptr::null(), 64);
    }
    assert!(corrupt_guard.is_completed());
    assert_eq!(
        *corrupt_seen.lock().unwrap(),
        Some((99, 0, Err(BytesSinkError::CorruptSlice { len: 64 })))
    );

    // 3. Clean error status with (NULL, 0) delivers Ok(&[]).
    let err_seen = Arc::new(Mutex::new(None));
    let err_c = Arc::clone(&err_seen);
    let err_guard = AsyncBytesDoneGuard::new(move |req_id, status, payload| {
        *err_c.lock().unwrap() = Some((req_id, status, payload.map(|b| b.to_vec())));
    });
    let (e_fn, e_ctx) = err_guard.as_raw();
    // SAFETY: `err_guard` stays alive; `invoke_async_bytes_done` normalizes `&[]` to `(NULL, 0)`.
    unsafe {
        assert!(invoke_async_bytes_done(Some(e_fn), e_ctx, 100, -6, &[]));
    }
    assert_eq!(*err_seen.lock().unwrap(), Some((100, -6, Ok(Vec::new()))));

    // 4. Panic inside async completion closure is caught inside extern "C" (never aborts/UB).
    let mut panic_guard = AsyncBytesDoneGuard::new(move |_req_id, _status, _payload| {
        panic!("async completion panic");
    });
    let (p_fn, p_ctx) = panic_guard.as_raw();
    // SAFETY: `panic_guard` stays alive across both calls.
    unsafe {
        invoke_async_bytes_done(Some(p_fn), p_ctx, 7, 0, b"boom");
        invoke_async_bytes_done(Some(p_fn), p_ctx, 7, 0, b"dup-after-panic");
    }
    assert!(panic_guard.is_completed());
    assert!(panic_guard.panicked());
    assert!(panic_guard.take_panic().is_some());
    assert!(panic_guard.take_panic().is_none());

    // 5. Cancel prevents subsequent foreign completion from invoking the closure.
    let cancelled_calls = Arc::new(AtomicUsize::new(0));
    let cc = Arc::clone(&cancelled_calls);
    let cancel_guard = AsyncBytesDoneGuard::new(move |_, _, _| {
        cc.fetch_add(1, Ordering::SeqCst);
    });
    let (x_fn, x_ctx) = cancel_guard.as_raw();
    assert!(cancel_guard.cancel());
    assert!(!cancel_guard.cancel());
    // SAFETY: `cancel_guard` stays alive during this late completion.
    unsafe {
        invoke_async_bytes_done(Some(x_fn), x_ctx, 8, 0, b"late");
    }
    assert_eq!(cancelled_calls.load(Ordering::SeqCst), 0);

    // 6. Direct `sink_bytes` validation.
    // SAFETY: Valid slice / null-with-zero / rejected corrupt pairs.
    unsafe {
        assert_eq!(sink_bytes(std::ptr::null(), 0), Some(&[][..]));
        assert_eq!(sink_bytes(std::ptr::null(), 1), None);
        assert_eq!(sink_bytes(b"a".as_ptr(), isize::MAX as usize + 1), None);
        assert_eq!(sink_bytes(b"abc".as_ptr(), 3), Some(&b"abc"[..]));
    }
}

#[test]
fn async_bytes_done_guard_drop_during_running_and_concurrent_cancel_are_sound() {
    use core::any::Any;
    use std::sync::Barrier;

    // 1. Synchronous self-drop INSIDE `on_done` (normal return and panic path).
    for should_panic in [false, true] {
        let holder: Arc<Mutex<Option<Box<dyn Any + Send>>>> = Arc::new(Mutex::new(None));
        let holder_c = Arc::clone(&holder);
        let dropped_inside = Arc::new(AtomicBool::new(false));
        let dropped_c = Arc::clone(&dropped_inside);
        let guard = AsyncBytesDoneGuard::new(move |req_id, status, payload| {
            assert_eq!(req_id, 501);
            assert_eq!(status, 0);
            assert_eq!(payload, Ok(&b"sync-self-drop"[..]));
            let taken = holder_c.lock().unwrap().take();
            assert!(taken.is_some());
            drop(taken);
            dropped_c.store(true, Ordering::SeqCst);
            if should_panic {
                panic!("panic after synchronous self-drop");
            }
        });
        let (done_fn, ctx) = guard.as_raw();
        *holder.lock().unwrap() = Some(Box::new(guard));
        // SAFETY: `guard` is live when `done_fn` enters `PHASE_RUNNING`, and self-drop hands off
        // deallocation cleanly to the trampoline.
        unsafe {
            invoke_async_bytes_done(Some(done_fn), ctx, 501, 0, b"sync-self-drop");
        }
        assert!(dropped_inside.load(Ordering::SeqCst));
        assert!(holder.lock().unwrap().is_none());
    }

    // 2. Cross-thread drop WHILE `on_done` is actively running (deterministic via Barriers),
    // covering both normal return and panic paths.
    for should_panic in [false, true] {
        let entered = Arc::new(Barrier::new(2));
        let dropped = Arc::new(Barrier::new(2));
        let finished = Arc::new(AtomicBool::new(false));

        let entered_c = Arc::clone(&entered);
        let dropped_c = Arc::clone(&dropped);
        let finished_c = Arc::clone(&finished);

        let guard = AsyncBytesDoneGuard::new(move |req_id, status, payload| {
            assert_eq!(req_id, 777);
            assert_eq!(status, 0);
            assert_eq!(payload, Ok(&b"cross-thread-drop"[..]));
            // Signal owner thread to drop `guard` right now while we are in `PHASE_RUNNING`.
            entered_c.wait();
            // Wait until owner thread has definitely finished `drop(guard)`.
            dropped_c.wait();
            finished_c.store(true, Ordering::SeqCst);
            if should_panic {
                panic!("panic after cross-thread guard drop");
            }
        });
        let (done_fn, ctx) = guard.as_raw();
        let raw_ctx = ctx as usize;

        let worker = std::thread::spawn(move || {
            // SAFETY: `guard` stays live until after `entered.wait()` inside `PHASE_RUNNING`.
            unsafe {
                invoke_async_bytes_done(
                    Some(done_fn),
                    raw_ctx as *mut c_void,
                    777,
                    0,
                    b"cross-thread-drop",
                );
            }
        });

        entered.wait();
        drop(guard);
        dropped.wait();
        worker.join().unwrap();
        assert!(finished.load(Ordering::SeqCst));
    }

    // 3. Stress concurrent `cancel()` vs `done_fn` (plus duplicate `done_fn`): exactly one of
    // {cancel succeeds with 0 invocations, or done_fn runs once with cancel returning false}.
    for round in 0..256u64 {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = Arc::clone(&calls);
        let guard = Arc::new(AsyncBytesDoneGuard::new(move |req_id, status, payload| {
            assert_eq!(req_id, round);
            assert_eq!(status, 0);
            assert_eq!(payload, Ok(&b"race"[..]));
            calls_c.fetch_add(1, Ordering::SeqCst);
        }));
        let (done_fn, ctx) = guard.as_raw();
        let raw_ctx = ctx as usize;
        let start = Arc::new(Barrier::new(3));

        let b1 = Arc::clone(&start);
        let t_done1 = std::thread::spawn(move || {
            b1.wait();
            // SAFETY: `guard` outlives all spawned threads via `join()`.
            unsafe {
                invoke_async_bytes_done(Some(done_fn), raw_ctx as *mut c_void, round, 0, b"race");
            }
        });

        let b2 = Arc::clone(&start);
        let t_done2 = std::thread::spawn(move || {
            b2.wait();
            // SAFETY: `guard` outlives all spawned threads via `join()`.
            unsafe {
                invoke_async_bytes_done(Some(done_fn), raw_ctx as *mut c_void, round, 0, b"race");
            }
        });

        start.wait();
        let cancelled = guard.cancel();
        t_done1.join().unwrap();
        t_done2.join().unwrap();

        let count = calls.load(Ordering::SeqCst);
        if cancelled {
            assert_eq!(count, 0, "cancelled guard must never invoke callback");
            assert!(!guard.is_completed());
        } else {
            assert_eq!(count, 1, "uncancelled guard must invoke callback exactly once");
            assert!(guard.is_completed());
        }
    }
}
