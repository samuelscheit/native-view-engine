#![cfg(not(target_arch = "wasm32"))]
//! Zero-allocation heap-guard suite for `nve-storage` read & validation hot paths.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;
use std::hint::black_box;
use std::sync::Mutex;

use nve_storage::{validate_key, validate_namespace, validate_value, KvStore};

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
fn get_ref_and_validators_are_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut store = KvStore::in_memory();
    for i in 0..256u32 {
        let k = format!("session:{i:04}");
        let v = format!("payload-{i:04}");
        store.put(k.as_bytes(), v.as_bytes()).expect("seed put");
    }

    let hit_key = b"session:0128";
    let miss_key = b"session:9999";
    let payload = [0x5Au8; 64];

    let (bytes_read, allocs) = measure_allocs(|| {
        validate_namespace(black_box("app_state.v1")).expect("valid namespace");
        validate_key(black_box(hit_key)).expect("valid key");
        validate_value(black_box(&payload)).expect("valid value");

        let hit = store
            .get_ref(black_box(hit_key))
            .expect("hit lookup")
            .expect("hit value");
        let miss = store.get_ref(black_box(miss_key)).expect("miss lookup");
        assert!(miss.is_none());

        let info = store.open_info();
        let log_len = store.memory_log_bytes().map_or(0, <[u8]>::len);
        black_box(hit.len() + info.key_count as usize + log_len)
    });

    assert!(bytes_read > 256);
    assert_eq!(
        allocs, 0,
        "KvStore::get_ref and valid-path validators must allocate 0 bytes (got {allocs})"
    );
}
