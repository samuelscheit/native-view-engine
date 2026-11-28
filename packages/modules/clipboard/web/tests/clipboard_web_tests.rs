//! Zero-allocation `CountingAlloc` and shadow-buffer lifecycle tests for `nve-clipboard-web`.

use std::alloc::{GlobalAlloc, Layout, System};

use nve_clipboard::{
    decode_has_text_response, decode_read_text_response, encode_write_text_request, METHOD_CLEAR,
    METHOD_HAS_TEXT, METHOD_READ_TEXT, METHOD_WRITE_TEXT,
};
use nve_clipboard_web::WebClipboard;
use nve_host_services::{CompletionSink, HandlerReply, HostServiceHandler};

struct CountingAlloc;

thread_local! {
    static TRACK_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static THREAD_ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// SAFETY: Delegates directly to `System` while counting allocations on the tracked test thread.
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

fn count_thread_allocs<F: FnOnce() -> R, R>(f: F) -> (R, usize) {
    THREAD_ALLOCS.with(|c| c.set(0));
    TRACK_THREAD.with(|flag| flag.set(true));
    let out = f();
    TRACK_THREAD.with(|flag| flag.set(false));
    (out, THREAD_ALLOCS.with(|c| c.get()))
}

#[test]
fn web_clipboard_has_text_is_zero_alloc_and_shadow_roundtrips() {
    let clip = WebClipboard::headless();
    let sink = CompletionSink::new();

    let req = encode_write_text_request("hello-web").expect("encode write");
    assert_eq!(
        clip.handle(METHOD_WRITE_TEXT, 1, &req, &sink).unwrap(),
        HandlerReply::empty()
    );

    let (reply, allocs) = count_thread_allocs(|| clip.handle(METHOD_HAS_TEXT, 2, &[], &sink).unwrap());
    assert_eq!(allocs, 0, "METHOD_HAS_TEXT must do zero heap allocs");

    let HandlerReply::Ready(buf) = reply else {
        panic!("expected Ready");
    };
    assert!(buf.is_inline());
    assert!(decode_has_text_response(&buf).unwrap());

    let HandlerReply::Ready(read_buf) = clip.handle(METHOD_READ_TEXT, 3, &[], &sink).unwrap()
    else {
        panic!("expected Ready");
    };
    assert_eq!(
        decode_read_text_response(&read_buf).unwrap().as_deref(),
        Some("hello-web")
    );

    assert_eq!(
        clip.handle(METHOD_CLEAR, 4, &[], &sink).unwrap(),
        HandlerReply::empty()
    );
    assert!(!clip.has_shadow_text());
    assert_eq!(WebClipboard::debug_slab_counts().pending_reads, 0);
}
