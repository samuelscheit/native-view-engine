//! Zero-allocation heap-guard suite for `nve-wire` hot paths.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;
use std::hint::black_box;
use std::sync::Mutex;

#[cfg(feature = "client")]
use nve_wire::{is_valid_pair_host, WireResyncState};
use nve_wire::{
    verify_bearer_token, NveWireFrame, NveWireFrameRef, NveWireKind, DEFAULT_MAX_WIRE_FRAME_BYTES,
};

struct CountingAlloc;

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
}

#[inline]
fn thread_alloc_count() -> usize {
    ALLOC_COUNT.with(Cell::get)
}

#[inline]
fn measure_allocs<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = thread_alloc_count();
    let out = f();
    let after = thread_alloc_count();
    (out, after.wrapping_sub(before))
}

// SAFETY: Every allocation method forwards unchanged arguments to `System`, upholding `GlobalAlloc`.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn counting_alloc_positive_probe() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (nonzero, allocs) = measure_allocs(|| black_box(vec![1u8; 64]));
    drop(nonzero);
    assert!(
        allocs >= 1,
        "positive probe failed: Vec allocation must increment CountingAlloc (got {allocs})"
    );
}

#[test]
fn bearer_token_verification_is_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let expected = "nve-secret-token-0123456789abcdef0123456789abcdef";
    let wrong_same_len = "nve-secret-token-0123456789abcdef0123456789abcde0";
    let wrong_short = "nve-secret-token";

    let (results, allocs) = measure_allocs(|| {
        let a = verify_bearer_token(black_box(expected), black_box(expected));
        let b = verify_bearer_token(black_box(expected), black_box(wrong_same_len));
        let c = verify_bearer_token(black_box(expected), black_box(wrong_short));
        let d = verify_bearer_token(black_box(""), black_box(""));
        (a, b, c, d)
    });

    assert_eq!(results, (true, false, false, false));
    assert_eq!(
        allocs, 0,
        "verify_bearer_token hot path must allocate 0 bytes (got {allocs})"
    );
}

#[cfg(feature = "client")]
#[test]
fn pair_host_validation_and_resync_state_are_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (ok, allocs) = measure_allocs(|| {
        let valid_dns = is_valid_pair_host(black_box("dev-host.local"));
        let valid_v4 = is_valid_pair_host(black_box("127.0.0.1"));
        let valid_v6 = is_valid_pair_host(black_box("[::1]"));
        let bad_space = is_valid_pair_host(black_box("bad host"));
        let bad_dot = is_valid_pair_host(black_box(".leading-dot"));
        let bad_v6 = is_valid_pair_host(black_box("[not-ipv6]"));

        let mut resync = WireResyncState::new();
        let mut sends = 0u32;
        resync.on_session_started(1, false, false);
        assert!(resync.should_apply(false, false));

        // Failed send must allow a retry; successful send deduplicates within epoch.
        resync.on_overflowed(1, || false);
        assert!(resync.is_waiting_for_keyframe());
        assert!(!resync.should_apply(false, false));
        assert!(resync.should_apply(true, false));

        resync.on_decode_error(1, || {
            sends += 1;
            true
        });
        resync.on_decode_error(1, || {
            sends += 1;
            true
        });
        resync.mark_keyframe_applied();
        assert!(!resync.is_waiting_for_keyframe());

        resync.on_session_started(2, true, true);
        assert!(resync.is_waiting_for_keyframe());
        resync.mark_keyframe_applied();

        valid_dns && valid_v4 && valid_v6 && !bad_space && !bad_dot && !bad_v6 && sends == 1
    });

    assert!(ok);
    assert_eq!(
        allocs, 0,
        "is_valid_pair_host and WireResyncState hot paths must allocate 0 bytes (got {allocs})"
    );
}

#[test]
fn wire_frame_xxh64_roundtrip_into_reused_buffer_is_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let frame = NveWireFrame::new(NveWireKind::CommandDelta, 42, vec![0xA5u8; 1024]);
    let mut buf = Vec::with_capacity(2048);

    // Warm up capacity once.
    frame.encode_into(&mut buf).expect("warmup encode");

    let (payload_len, allocs) = measure_allocs(|| {
        let mut total = 0usize;
        for _ in 0..16 {
            black_box(&frame)
                .encode_into(&mut buf)
                .expect("zero-alloc encode_into");
            let decoded = NveWireFrameRef::decode(&buf, DEFAULT_MAX_WIRE_FRAME_BYTES)
                .expect("zero-alloc decode");
            total += decoded.payload.len();
        }
        black_box(total)
    });

    assert_eq!(payload_len, 16 * 1024);
    assert_eq!(
        allocs, 0,
        "NveWireFrame::encode_into + NveWireFrameRef::decode hot path must allocate 0 bytes (got {allocs})"
    );
}
