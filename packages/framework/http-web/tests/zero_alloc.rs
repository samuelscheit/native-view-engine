//! Zero-allocation heap-guard suite for `nve-http-web` hot paths.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;
use std::hint::black_box;
use std::sync::Mutex;

use nve_http_web::{BrowserClock, CredentialsMode, FetchConfig};
use nve_query::Clock;

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

    let (zeroed, zeroed_allocs) = measure_allocs(|| black_box(vec![0u8; 64]));
    drop(zeroed);
    assert!(
        zeroed_allocs >= 1,
        "positive probe failed: zeroed Vec allocation must increment CountingAlloc (got {zeroed_allocs})"
    );
}

#[test]
fn fetch_config_build_url_into_is_zero_alloc_in_steady_state() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let same_origin = FetchConfig::same_origin();
    let trailing_slash = FetchConfig::with_base_url("https://api.example.com/")
        .with_rpc_prefix("/api/v2/rpc/");
    let empty_prefix = FetchConfig::same_origin().with_rpc_prefix("");

    let expected_same = same_origin.build_url("issues.v1.Issues/List");
    let expected_leading = trailing_slash.build_url("/issues.v1.Issues/Get");
    let expected_empty = empty_prefix.build_url("health.v1.Health/Ping");
    let expected_multi = trailing_slash.build_url("users.v1.Users/Profile");

    let mut buf = String::with_capacity(128);
    // Warm the reusable URL buffer once.
    trailing_slash.build_url_into("/issues.v1.Issues/Get", &mut buf);

    let ((len_a, len_b, len_c, len_d), allocs) = measure_allocs(|| {
        same_origin.build_url_into(black_box("issues.v1.Issues/List"), &mut buf);
        assert_eq!(buf, expected_same);
        let len_a = buf.len();

        trailing_slash.build_url_into(black_box("/issues.v1.Issues/Get"), &mut buf);
        assert_eq!(buf, expected_leading);
        let len_b = buf.len();

        empty_prefix.build_url_into(black_box("health.v1.Health/Ping"), &mut buf);
        assert_eq!(buf, expected_empty);
        let len_c = buf.len();

        trailing_slash.build_url_into(black_box("users.v1.Users/Profile"), &mut buf);
        assert_eq!(buf, expected_multi);
        let len_d = buf.len();

        (len_a, len_b, len_c, len_d)
    });

    assert_eq!(
        allocs, 0,
        "Warm FetchConfig::build_url_into must perform 0 heap allocs, did {allocs}"
    );
    assert_eq!(len_a, expected_same.len());
    assert_eq!(len_b, expected_leading.len());
    assert_eq!(len_c, expected_empty.len());
    assert_eq!(len_d, expected_multi.len());
}

#[test]
fn browser_clock_and_credentials_mode_are_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let clock = BrowserClock::new();
    let _warm = clock.now_ms();

    let ((t0, t1, s0, s1, s2), allocs) = measure_allocs(|| {
        let t0 = clock.now_ms();
        let t1 = clock.now_ms();
        let s0 = black_box(CredentialsMode::SameOrigin).as_js_str();
        let s1 = black_box(CredentialsMode::Include).as_js_str();
        let s2 = black_box(CredentialsMode::Omit).as_js_str();
        (t0, t1, s0, s1, s2)
    });

    assert_eq!(
        allocs, 0,
        "BrowserClock::now_ms and CredentialsMode::as_js_str must perform 0 heap allocs, did {allocs}"
    );
    assert!(t1 >= t0);
    assert_eq!(s0, "same-origin");
    assert_eq!(s1, "include");
    assert_eq!(s2, "omit");
}
