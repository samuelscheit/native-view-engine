//! Zero-allocation `CountingAlloc` and scheme-allowlist security tests for `nve-linking-web`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_host_services::ServiceError;
use nve_linking::LinkingBackend;
use nve_linking_web::{is_allowed_web_url_scheme, validate_web_url, WebLinking};

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Delegates directly to `System` while incrementing an atomic allocation counter.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
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

    let before = ALLOC_COUNT.load(Ordering::SeqCst);
    for u in allowed {
        assert!(is_allowed_web_url_scheme(u), "must allow {u}");
        assert!(validate_web_url(u).is_ok(), "must validate {u}");
    }
    for u in denied {
        assert!(!is_allowed_web_url_scheme(u), "must reject {u}");
    }
    let after = ALLOC_COUNT.load(Ordering::SeqCst);
    assert_eq!(after - before, 0, "allowlist validation must not allocate");

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
