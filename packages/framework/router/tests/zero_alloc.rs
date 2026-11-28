//! Zero-allocation heap-guard suite for `nve-router` hot paths.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;
use std::borrow::Cow;
use std::hint::black_box;
use std::sync::Mutex;

use nve_router::{
    decode_nav_packet_ref, encode_nav_packet_into, is_unreserved, percent_decode, percent_encode,
    percent_encode_into, split_uri, NavPacketRef, NavigationAction, Nvn1DecodeError, RawQueryIter,
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

    let (zeroed, zeroed_allocs) = measure_allocs(|| black_box(vec![0u8; 64]));
    drop(zeroed);
    assert!(
        zeroed_allocs >= 1,
        "positive probe failed: zeroed Vec allocation must increment CountingAlloc (got {zeroed_allocs})"
    );
}

#[test]
fn nvn1_encode_into_and_decode_ref_are_zero_alloc_in_steady_state() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let pkt = NavPacketRef {
        action: NavigationAction::Push,
        can_go_back: true,
        can_go_forward: false,
        is_modal: true,
        stack_depth: 4,
        active_tab_index: 2,
        uri: "/issues/42?tab=comments&p=3#top",
        tab_id: "issues",
    };

    let mut wire = Vec::with_capacity(128);
    // Warm buffer once.
    encode_nav_packet_into(pkt, &mut wire).unwrap();

    let ((decoded_depth, decoded_uri_len, decoded_tab_len, bad_err), allocs) = measure_allocs(|| {
        wire.clear();
        encode_nav_packet_into(black_box(pkt), &mut wire).expect("encode_nav_packet_into");
        let rt = decode_nav_packet_ref(black_box(&wire)).expect("decode_nav_packet_ref");
        assert_eq!(rt, pkt);

        let bad_err = matches!(
            decode_nav_packet_ref(black_box(b"NOPE0123456789012345")),
            Err(Nvn1DecodeError::InvalidMagic(_))
        );
        (rt.stack_depth, rt.uri.len(), rt.tab_id.len(), bad_err)
    });

    assert_eq!(
        allocs, 0,
        "Warm encode_nav_packet_into and decode_nav_packet_ref must perform 0 heap allocs, did {allocs}"
    );
    assert_eq!(decoded_depth, 4);
    assert_eq!(decoded_uri_len, pkt.uri.len());
    assert_eq!(decoded_tab_len, pkt.tab_id.len());
    assert!(bad_err);
}

#[test]
fn split_uri_raw_query_iter_and_percent_codecs_are_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let uri = "/issues/42?tab=comments&&flag&p=3&tag=rust#section-1";
    let unreserved = "alpha-09._~Z";
    let needs_escape = "café 100% /a?b#c&d=e";

    let mut enc_buf = String::with_capacity(128);
    // Warm reusable percent-encoding buffer once.
    percent_encode_into(needs_escape, &mut enc_buf);

    let ((pair_count, enc_borrowed, dec_borrowed, enc_into_len, unres_ok), allocs) =
        measure_allocs(|| {
            let parts = split_uri(black_box(uri));
            assert_eq!(parts.path, "/issues/42");
            assert_eq!(parts.raw_query, Some("tab=comments&&flag&p=3&tag=rust"));
            assert_eq!(parts.raw_fragment, Some("section-1"));

            let mut pair_count = 0usize;
            for (k, v) in RawQueryIter::new(black_box("?tab=comments&&flag&p=3&tag=rust#frag")) {
                black_box((k, v));
                pair_count += 1;
            }

            let enc_borrowed = matches!(
                percent_encode(black_box(unreserved)),
                Cow::Borrowed(s) if s == unreserved
            );
            let dec_borrowed = matches!(
                percent_decode(black_box(unreserved)),
                Ok(Cow::Borrowed(s)) if s == unreserved
            );

            enc_buf.clear();
            percent_encode_into(black_box(needs_escape), &mut enc_buf);
            let unres_ok = is_unreserved(black_box(b'~')) && !is_unreserved(black_box(b'/'));

            (
                pair_count,
                enc_borrowed,
                dec_borrowed,
                enc_buf.len(),
                unres_ok,
            )
        });

    assert_eq!(
        allocs, 0,
        "split_uri, RawQueryIter, Cow::Borrowed percent_encode/decode, and warm percent_encode_into must perform 0 heap allocs, did {allocs}"
    );
    assert_eq!(pair_count, 4);
    assert!(enc_borrowed);
    assert!(dec_borrowed);
    assert_eq!(enc_buf, "caf%C3%A9%20100%25%20%2Fa%3Fb%23c%26d%3De");
    assert_eq!(enc_into_len, enc_buf.len());
    assert!(unres_ok);
}
