//! Zero-allocation heap-guard suite for `nve-hash` (`xxh64`, `const_id`, `Xxh64Hasher`, `Key64`).

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::hash::Hasher;
use std::alloc::System;
use std::hint::black_box;
use std::sync::Mutex;

use nve_hash::{Key64, Xxh64Hasher, const_id, xxh64};

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
fn counting_alloc_positive_probe_and_thread_isolation() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (nonzero, allocs) = measure_allocs(|| black_box(vec![1u8; 64]));
    drop(nonzero);
    assert!(
        allocs >= 1,
        "positive probe failed: non-zero Vec allocation must increment CountingAlloc (got {allocs})"
    );

    let (zeroed, zeroed_allocs) = measure_allocs(|| black_box(vec![0u8; 64]));
    drop(zeroed);
    assert!(
        zeroed_allocs >= 1,
        "positive probe failed: zeroed Vec allocation must increment CountingAlloc (got {zeroed_allocs})"
    );

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        for _ in 0..32 {
            let v = black_box(vec![1u8; 64]);
            drop(v);
        }
        done_tx.send(()).unwrap();
    });
    ready_rx.recv().unwrap();
    let start = thread_alloc_count();
    while done_rx.try_recv().is_err() {
        std::thread::yield_now();
    }
    let end = thread_alloc_count();
    worker.join().unwrap();
    assert_eq!(
        end.wrapping_sub(start),
        0,
        "sibling thread allocations must not pollute this thread's counter"
    );
}

#[test]
fn xxh64_const_id_and_key64_are_zero_alloc_across_input_sizes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let short = b"nve.id";
    let border = [0x3Cu8; 31];
    let exact_stripe = [0x7Eu8; 32];
    let medium = [0xA5u8; 256];
    let long = vec![0x5Au8; 8192];

    let short_str = "nve.ui";
    let medium_str = "dev.native_view_engine.components.layout.flex_container.v2";
    let long_str = "nve.schema.field.".repeat(64);

    let ((h_empty, h_short, h_border, h_stripe, h_med, h_long, id_s, id_m, id_l, k_s, k_m, k_l), allocs) =
        measure_allocs(|| {
            let h_empty = xxh64(black_box(b""), 0);
            let h_short = xxh64(black_box(short), 0x9E37_79B1);
            let h_border = xxh64(black_box(&border), 0);
            let h_stripe = xxh64(black_box(&exact_stripe), 0);
            let h_med = xxh64(black_box(&medium), 42);
            let h_long = xxh64(black_box(&long), 0xDEAD_BEEF);

            let id_s = const_id(black_box(short_str));
            let id_m = const_id(black_box(medium_str));
            let id_l = const_id(black_box(&long_str));

            let k_s = Key64::of(black_box(short_str));
            let k_m = Key64::of(black_box(medium_str));
            let k_l = Key64::of(black_box(&long_str));
            let roundtrip = Key64::from_halves(k_l.lo(), k_l.hi());
            assert_eq!(roundtrip, k_l);

            (
                h_empty, h_short, h_border, h_stripe, h_med, h_long, id_s, id_m, id_l, k_s, k_m,
                k_l,
            )
        });

    assert_eq!(allocs, 0, "xxh64/const_id/Key64 must perform 0 heap allocs, did {allocs}");
    assert_eq!(h_empty, 0xEF46_DB37_51D8_E999);
    assert_ne!(h_short, h_border);
    assert_ne!(h_stripe, h_med);
    assert_ne!(h_med, h_long);
    assert_eq!(k_s.0, id_s);
    assert_eq!(k_m.0, id_m);
    assert_eq!(k_l.0, id_l);
}

#[test]
fn xxh64_streaming_multi_chunk_updates_are_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let payload: Vec<u8> = (0..4099usize).map(|i| (i.wrapping_mul(37) & 0xFF) as u8).collect();
    let expected = xxh64(&payload, 0x1234_5678);

    let ((stream_digest, std_hasher_digest, typed_digest), allocs) = measure_allocs(|| {
        // Irregular chunk boundaries exercising partial buffer fills, exact stripes, and tail.
        let mut h = Xxh64Hasher::new(0x1234_5678);
        let chunk_sizes = [1usize, 3, 7, 13, 31, 32, 33, 64, 127, 256];
        let mut offset = 0usize;
        let mut idx = 0usize;
        while offset < payload.len() {
            let take = chunk_sizes[idx % chunk_sizes.len()].min(payload.len() - offset);
            h.update(black_box(&payload[offset..offset + take]));
            let _mid = black_box(h.digest());
            offset += take;
            idx += 1;
        }
        let stream_digest = h.digest();

        // `core::hash::Hasher` trait path.
        let mut h2 = Xxh64Hasher::new(0x1234_5678);
        for chunk in black_box(&payload).chunks(19) {
            h2.write(chunk);
        }
        let std_hasher_digest = h2.finish();

        // Scalar helper methods (`update_u64`, `update_u32`, `update_f32`).
        let mut h3 = Xxh64Hasher::default();
        h3.update_u64(black_box(0xCAFE_BABE_1234_5678));
        h3.update_u32(black_box(42));
        h3.update_f32(black_box(-0.0));
        h3.update_f32(black_box(f32::NAN));
        h3.update_f32(black_box(12.5));
        let typed_digest = h3.digest();

        (stream_digest, std_hasher_digest, typed_digest)
    });

    assert_eq!(
        allocs, 0,
        "Xxh64Hasher streaming multi-chunk updates must perform 0 heap allocs, did {allocs}"
    );
    assert_eq!(stream_digest, expected);
    assert_eq!(std_hasher_digest, expected);
    assert_ne!(typed_digest, 0);
}
