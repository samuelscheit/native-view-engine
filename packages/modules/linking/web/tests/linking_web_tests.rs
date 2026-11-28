//! Zero-allocation thread-local `CountingAlloc` and wire/scheme-allowlist security tests for
//! `nve-linking-web`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_abi::SERVICE_LINKING;
use nve_host_services::{CompletionSink, HandlerReply, HostServiceHandler, ServiceError};
use nve_linking::{
    decode_can_open_response, decode_initial_url_response, encode_url_request, LinkingBackend,
    METHOD_CAN_OPEN_URL, METHOD_INITIAL_URL, METHOD_OPEN_URL,
};
use nve_linking_web::{is_allowed_web_url_scheme, validate_web_url, WebLinking};

struct CountingAlloc;

thread_local! {
    static TRACK_THREAD: Cell<bool> = const { Cell::new(false) };
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
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

#[test]
fn counting_alloc_positive_probe_detects_heap_allocations() {
    let allocs = count_thread_allocs(|| {
        let v = std::hint::black_box(vec![0x55u8; 64]);
        assert_eq!(v.len(), 64);
    });
    assert!(
        allocs >= 1,
        "positive CountingAlloc probe must observe at least 1 heap allocation"
    );
}

#[test]
fn web_linking_allowlist_is_zero_alloc_and_rejects_dangerous_schemes() {
    let allowed = [
        "https://nve.dev/docs",
        "HTTP://example.com",
        "mailto:team@nve.dev",
        "tel:+15551234567",
    ];
    let denied = [
        "javascript:alert(1)",
        "JAVASCRIPT:void(0)",
        "data:text/html,<script>alert(1)</script>",
        "file:///etc/passwd",
        "vbscript:msgbox(1)",
        "blob:https://example.com/123",
    ];

    let allocs = count_thread_allocs(|| {
        for u in allowed {
            assert!(is_allowed_web_url_scheme(u), "must allow {u}");
            assert!(validate_web_url(u).is_ok(), "must validate {u}");
        }
        for u in denied {
            assert!(!is_allowed_web_url_scheme(u), "must reject {u}");
        }
    });
    assert_eq!(allocs, 0, "allowlist validation must perform 0 heap allocations");

    let linking = WebLinking::new();
    for u in allowed {
        assert!(linking.can_open_url(u).unwrap());
    }
    for u in denied {
        assert!(!linking.can_open_url(u).unwrap());
        assert!(matches!(
            linking.open_url(u),
            Err(ServiceError::InvalidArg(_))
        ));
    }
}

#[test]
fn web_linking_host_service_handler_wire_dispatch_validates_all_methods() {
    let handler = WebLinking::new();
    let sink = CompletionSink::new();
    assert_eq!(handler.service_id(), SERVICE_LINKING);

    // METHOD_CAN_OPEN_URL -> allowed true, dangerous false.
    let ok_req = encode_url_request("https://nve.dev/docs").expect("encode allowed");
    let HandlerReply::Ready(ok_resp) = handler
        .handle(METHOD_CAN_OPEN_URL, 1, &ok_req, &sink)
        .expect("can_open_url allowed")
    else {
        panic!("expected Ready");
    };
    assert!(decode_can_open_response(ok_resp.as_slice()).unwrap());

    let bad_req = encode_url_request("javascript:alert(1)").expect("encode scheme");
    let HandlerReply::Ready(bad_resp) = handler
        .handle(METHOD_CAN_OPEN_URL, 2, &bad_req, &sink)
        .expect("can_open_url denied")
    else {
        panic!("expected Ready");
    };
    assert!(!decode_can_open_response(bad_resp.as_slice()).unwrap());

    // METHOD_OPEN_URL -> dangerous scheme rejected with InvalidArg before platform call;
    // valid scheme returns Unsupported on non-wasm32 hosts.
    assert!(matches!(
        handler.handle(METHOD_OPEN_URL, 3, &bad_req, &sink),
        Err(ServiceError::InvalidArg(_))
    ));
    assert_eq!(
        handler.handle(METHOD_OPEN_URL, 4, &ok_req, &sink),
        Err(ServiceError::Unsupported)
    );

    // METHOD_INITIAL_URL -> None on non-wasm32 hosts.
    let HandlerReply::Ready(init_resp) = handler
        .handle(METHOD_INITIAL_URL, 5, &[], &sink)
        .expect("initial_url")
    else {
        panic!("expected Ready");
    };
    assert_eq!(decode_initial_url_response(init_resp.as_slice()).unwrap(), None);

    // Unknown method -> Unsupported.
    assert_eq!(
        handler.handle(999, 6, &[], &sink),
        Err(ServiceError::Unsupported)
    );
}
